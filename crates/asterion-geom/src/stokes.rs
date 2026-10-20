//! Volume and surface area of solids bounded by planes and circular cylinders, from first principles.
//!
//! Everything in this module follows from three results of ordinary calculus. No numerical constant of any other
//! application is used; every tolerance below is chosen here, with the reasoning next to it.
//!
//! * **Volume by the divergence theorem.** For a closed solid with boundary `S` and any fixed point `O`, the field `F = (x
//!   - O) / 3` has divergence 1, so with `n` the outward unit normal. A planar face at signed distance `h` from `O`
//!   (measured along `n`) contributes `h * A / 3` (a cone of apex `O`); a cylindrical face of radius `R` contributes `(1/3)
//!   * integral of R * (R - c . e(u)) du dv`, with `u` the angle, `v` the axial coordinate, `e(u)` the radial unit vector
//!   and `c` the position of `O` in the plane of the circle. The sum over the faces does not depend on `O`.
//! * **Areas and moments by Green's theorem.** For a plane region bounded by a closed curve traversed counter-clockwise, `A
//!   = (1/2) closed-integral (x dy - y dx)` and. The surface integrals above reduce the same way to line integrals over the
//!   loops of the face in its `(u, v)` space.
//! * **Gauss-Legendre quadrature.** An `n`-point rule integrates polynomials of degree `2n - 1` exactly and smooth
//!   functions with an error that falls faster than any power of the panel width. [`integrate`] compares an 8-point and a
//!   16-point rule on every panel and bisects a panel until the two agree.
//!
//! # Two ways in
//!
//! * **Exact entry points** ([`arc_band_volume`], [`arc_band_surface_area`] and their slanted and cut variants,
//!   [`extruded_loops_metrics`], [`cap_area`]) take the true geometry (circular arcs and line segments) and return the
//!   closed-form quantities, evaluated with [`integrate`] on the exact parametrisation. Their accuracy is that of
//!   double precision: the relative error is a few times `1e-15` for the geometry the engine meets.
//! * **Sampled faces** ([`Face`], [`UvLoop`], [`cone_volume`], [`face_area`]) describe a face by the sample points of its
//!   loops in `(u, v)`. Each edge is a piecewise cubic Hermite curve through its samples (tangents from the three-point
//!   Bessel formula on the chord-length parameter, one-sided at the ends), so a polyline edge is exact and an edge
//!   sampled from an arc has an area error that falls like the fourth power of the sample spacing.
//!
//! # Tolerances (all chosen here)
//!
//! * [`QUADRATURE_REL_TOL`] `= 1e-13`, about 450 machine epsilons. The rule used for the answer (16 points) is far more
//!   accurate than the 8-point rule it is compared with, so the true error of an accepted panel is much smaller than
//!   the difference that was tested. A floor proportional to the largest integrand value seen keeps an integral that
//!   cancels to zero from demanding an impossible relative accuracy.
//! * [`TessTols::display_default`]: a chord sag of 1 mm and a step of at most 5 degrees. They govern the sample points
//!   used to draw an arc and to take its box; no quantity of this module depends on them.
//!
//! Lengths are metres and angles radians, except that the short-curve checks of [`crate::tolerance`] are stated in feet and are
//! applied after conversion with [`FEET_PER_METRE`].

use crate::vec::Vec2;
use std::sync::OnceLock;

/// Feet per metre (exact by definition of the international foot).
pub const FEET_PER_METRE: f64 = 1.0 / 0.3048;

/// Relative accuracy asked of the adaptive quadrature (see the module documentation).
pub const QUADRATURE_REL_TOL: f64 = 1.0e-13;

/// Failures of the integration operations. ASTERION's own vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StokesError {
    /// Tessellation input outside the domain: a non-positive sag, a negative span or radius, or a value that is not finite.
    InvalidTessellationInput,
    /// The requested sampling would need an unreasonable number of points.
    TessellationUnreachable,
    /// The quadrature did not reach its tolerance (a non-finite integrand or one that is not integrable).
    IntegrationDidNotConverge,
    /// An edge with fewer than two samples, a face without a loop, or input that is not finite.
    DegenerateFace,
    /// The inner arc of a band would have a negative radius.
    NegativeOffsetRadius,
    /// The width of a band is smaller than 1e-9 ft: its end caps have no extent.
    DegenerateCap,
    /// A curve of the profile (an arc or an end cap) is shorter than the short-curve tolerance.
    CurveTooShort,
}

impl std::fmt::Display for StokesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            StokesError::InvalidTessellationInput => "invalid tessellation input",
            StokesError::TessellationUnreachable => "tessellation tolerance cannot be achieved",
            StokesError::IntegrationDidNotConverge => "integration does not converge",
            StokesError::DegenerateFace => "degenerate face",
            StokesError::NegativeOffsetRadius => "offset radius is negative",
            StokesError::DegenerateCap => "end cap is degenerate",
            StokesError::CurveTooShort => "a curve of the thickened profile is shorter than the short-curve tolerance",
        };
        f.write_str(s)
    }
}

impl std::error::Error for StokesError {}

// ---------------------------------------------------------------------------------------------
// Display sampling of arcs

/// How finely an arc is sampled for drawing and for its box. Metres and radians.
///
/// The sample points of an inscribed polygon with `n` equal steps of angle `d` deviate from the arc by the sagitta
/// `R (1 - cos(d / 2))`; the step is the largest one for which that stays within `sag`, and never more than `max_step`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TessTols {
    /// Largest distance between the arc and its chord, metres.
    pub sag: f64,
    /// Largest angle between two consecutive samples, radians.
    pub max_step: f64,
}

impl TessTols {
    /// 1 mm of sag and 5 degrees: finer than a pixel at the scales a plan is drawn at, and small enough that the box of
    /// the samples differs from the analytic box of an arc by well under a millimetre.
    pub fn display_default() -> Self {
        TessTols { sag: 1.0e-3, max_step: 5.0_f64.to_radians() }
    }
}

/// Number of sample points (both ends included, at least 3) of an arc of angular extent `span` and radius `radius`.
pub fn arc_sample_count(span: f64, radius: f64, tols: &TessTols) -> Result<usize, StokesError> {
    const MAX_SEGMENTS: f64 = 1.0e6;
    if !(tols.sag > 0.0) || !(tols.max_step > 0.0) || !(span >= 0.0) || !(radius >= 0.0) || !span.is_finite() || !radius.is_finite() {
        return Err(StokesError::InvalidTessellationInput);
    }
    // largest step with R (1 - cos(d/2)) <= sag; once the sag reaches the radius any step up to pi is within it
    let ratio = tols.sag / radius.max(f64::MIN_POSITIVE);
    let sag_step = if ratio >= 1.0 { std::f64::consts::PI } else { 2.0 * (1.0 - ratio).acos() };
    let step = sag_step.min(tols.max_step);
    let segments = (span / step - 1.0e-12).ceil().max(2.0);
    if !(segments <= MAX_SEGMENTS) {
        return Err(StokesError::TessellationUnreachable);
    }
    Ok(segments as usize + 1)
}

// ---------------------------------------------------------------------------------------------
// Adaptive Gauss-Legendre quadrature

struct Rule {
    x: Vec<f64>,
    w: Vec<f64>,
}

/// The `n`-point Gauss-Legendre rule on `[-1, 1]`: nodes are the roots of the Legendre polynomial `P_n`, found by Newton
/// iteration from the classical starting values `cos(pi (i + 3/4) / (n + 1/2))`; the weights are `2 / ((1 - x^2) P_n'(x)^2)`.
fn legendre_rule(n: usize) -> Rule {
    let nf = n as f64;
    let mut x = vec![0.0; n];
    let mut w = vec![0.0; n];
    for i in 0..(n + 1) / 2 {
        let mut z = (std::f64::consts::PI * (i as f64 + 0.75) / (nf + 0.5)).cos();
        let mut dp = 1.0;
        for _ in 0..100 {
            let (mut p_prev, mut p) = (1.0, z);
            for k in 2..=n {
                let kf = k as f64;
                let next = ((2.0 * kf - 1.0) * z * p - (kf - 1.0) * p_prev) / kf;
                p_prev = p;
                p = next;
            }
            dp = nf * (z * p - p_prev) / (z * z - 1.0);
            let dz = p / dp;
            z -= dz;
            if dz.abs() < 1.0e-16 {
                break;
            }
        }
        let weight = 2.0 / ((1.0 - z * z) * dp * dp);
        x[i] = -z;
        x[n - 1 - i] = z;
        w[i] = weight;
        w[n - 1 - i] = weight;
    }
    Rule { x, w }
}

fn rules() -> &'static (Rule, Rule) {
    static RULES: OnceLock<(Rule, Rule)> = OnceLock::new();
    RULES.get_or_init(|| (legendre_rule(8), legendre_rule(16)))
}

fn apply_rule(rule: &Rule, f: &mut impl FnMut(f64) -> f64, lo: f64, hi: f64, scale: &mut f64) -> f64 {
    let (mid, half) = (0.5 * (lo + hi), 0.5 * (hi - lo));
    let mut sum = 0.0;
    for (x, w) in rule.x.iter().zip(&rule.w) {
        let v = f(mid + half * x);
        *scale = scale.max(v.abs());
        sum += w * v;
    }
    sum * half
}

/// Adaptive Gauss-Legendre integral of `f` over `[a, b]` to the relative accuracy `rel_tol` (the first four panels are
/// always evaluated, so a function that is symmetric about the middle cannot fool the error estimate).
pub fn integrate(f: impl FnMut(f64) -> f64, a: f64, b: f64, rel_tol: f64) -> Result<f64, StokesError> {
    integrate_core(f, a, b, rel_tol, 4, 0.0)
}

/// The adaptive rule behind [`integrate`], starting from `panels` equal panels. `noise` is the absolute size below which a
/// disagreement between the two rules is rounding error rather than quadrature error: a caller whose integrand is a difference
/// of large terms (`x dy - y dx` along a line through the origin is zero, but each term is not) passes about `1e-14` times the
/// size of those terms times the width of the interval.
fn integrate_core(mut f: impl FnMut(f64) -> f64, a: f64, b: f64, rel_tol: f64, panels: usize, noise: f64) -> Result<f64, StokesError> {
    const MAX_DEPTH: u32 = 40;
    const MAX_PANELS: usize = 100_000;
    const FLOOR: f64 = 1.0e-14;
    if !a.is_finite() || !b.is_finite() {
        return Err(StokesError::IntegrationDidNotConverge);
    }
    if a == b {
        return Ok(0.0);
    }
    let (r8, r16) = rules();
    let n0 = panels.max(1);
    let mut scale = 0.0_f64;
    let mut work: Vec<(f64, f64, u32, f64, f64)> = Vec::with_capacity(n0 + 64);
    let mut total = 0.0;
    for k in (0..n0).rev() {
        let (lo, hi) = (a + (b - a) * k as f64 / n0 as f64, a + (b - a) * (k + 1) as f64 / n0 as f64);
        let (i8, i16) = (apply_rule(r8, &mut f, lo, hi, &mut scale), apply_rule(r16, &mut f, lo, hi, &mut scale));
        total += i16;
        work.push((lo, hi, 0, i8, i16));
    }
    if !total.is_finite() || !scale.is_finite() {
        return Err(StokesError::IntegrationDidNotConverge);
    }
    let width = (b - a).abs();
    let budget = (rel_tol * total.abs()).max(FLOOR * scale * width).max(noise);
    let mut result = 0.0;
    let mut visited = 0usize;
    while let Some((lo, hi, depth, i8, i16)) = work.pop() {
        visited += 1;
        let allowed = budget * (hi - lo).abs() / width;
        if (i16 - i8).abs() <= allowed {
            result += i16;
            continue;
        }
        if depth >= MAX_DEPTH || visited > MAX_PANELS {
            return Err(StokesError::IntegrationDidNotConverge);
        }
        let mid = 0.5 * (lo + hi);
        for (l, h) in [(mid, hi), (lo, mid)] {
            let (j8, j16) = (apply_rule(r8, &mut f, l, h, &mut scale), apply_rule(r16, &mut f, l, h, &mut scale));
            if !j16.is_finite() {
                return Err(StokesError::IntegrationDidNotConverge);
            }
            work.push((l, h, depth + 1, j8, j16));
        }
    }
    Ok(result)
}

// ---------------------------------------------------------------------------------------------
// Sampled faces: cubic Hermite edges in (u, v)

/// A piecewise cubic Hermite curve through sample points. The tangent at a sample is the three-point Bessel estimate on
/// the chord-length parameter (exact for quadratics on any spacing), one-sided at the two ends; with two samples the
/// curve is the straight segment.
struct EdgeSpline<'a> {
    pts: &'a [[f64; 2]],
    /// Tangent at each sample as a velocity with respect to chord length.
    tan: Vec<[f64; 2]>,
    /// Chord length of each segment.
    len: Vec<f64>,
}

impl<'a> EdgeSpline<'a> {
    fn new(pts: &'a [[f64; 2]]) -> Self {
        let m = pts.len();
        let len: Vec<f64> = (0..m - 1).map(|i| (pts[i + 1][0] - pts[i][0]).hypot(pts[i + 1][1] - pts[i][1])).collect();
        let secant = |i: usize| -> [f64; 2] {
            if len[i] > 0.0 {
                [(pts[i + 1][0] - pts[i][0]) / len[i], (pts[i + 1][1] - pts[i][1]) / len[i]]
            } else {
                [0.0, 0.0]
            }
        };
        let mut tan = vec![[0.0; 2]; m];
        if m == 2 {
            tan[0] = secant(0);
            tan[1] = secant(0);
        } else {
            for k in 1..m - 1 {
                let (d0, d1) = (secant(k - 1), secant(k));
                let s = len[k - 1] + len[k];
                if s > 0.0 {
                    for c in 0..2 {
                        tan[k][c] = (len[k] * d0[c] + len[k - 1] * d1[c]) / s;
                    }
                }
            }
            // one-sided three-point estimates at the ends
            let (d0, d1) = (secant(0), secant(1));
            let s = len[0] + len[1];
            for c in 0..2 {
                tan[0][c] = if s > 0.0 { d0[c] + (d0[c] - d1[c]) * len[0] / s } else { 0.0 };
            }
            let (e0, e1) = (secant(m - 2), secant(m - 3));
            let s = len[m - 2] + len[m - 3];
            for c in 0..2 {
                tan[m - 1][c] = if s > 0.0 { e0[c] + (e0[c] - e1[c]) * len[m - 2] / s } else { 0.0 };
            }
        }
        // The speed of the curve with respect to chord length is close to 1, so the tangent is taken as a unit vector: the
        // plain average of two secants is shorter than 1 by a factor cos(phi/2) and would make a sampled circle too flat.
        for v in tan.iter_mut() {
            let n = v[0].hypot(v[1]);
            if n > 0.0 {
                *v = [v[0] / n, v[1] / n];
            }
        }
        EdgeSpline { pts, tan, len }
    }

    /// Position and derivative with respect to the local parameter `tau` in `[0, 1]` of segment `i`.
    fn eval(&self, i: usize, tau: f64) -> ([f64; 2], [f64; 2]) {
        let (t2, t3) = (tau * tau, tau * tau * tau);
        let (h00, h10, h01, h11) = (2.0 * t3 - 3.0 * t2 + 1.0, t3 - 2.0 * t2 + tau, -2.0 * t3 + 3.0 * t2, t3 - t2);
        let (d00, d10, d01, d11) = (6.0 * t2 - 6.0 * tau, 3.0 * t2 - 4.0 * tau + 1.0, -6.0 * t2 + 6.0 * tau, 3.0 * t2 - 2.0 * tau);
        let h = self.len[i];
        let (mut p, mut d) = ([0.0; 2], [0.0; 2]);
        for c in 0..2 {
            let (p0, p1) = (self.pts[i][c], self.pts[i + 1][c]);
            let (m0, m1) = (self.tan[i][c] * h, self.tan[i + 1][c] * h);
            p[c] = h00 * p0 + h10 * m0 + h01 * p1 + h11 * m1;
            d[c] = d00 * p0 + d10 * m0 + d01 * p1 + d11 * m1;
        }
        (p, d)
    }

    /// Line integral of the 1-form `omega(u, v, du, dv)` along the edge, from its first to its last sample. `noise` is the
    /// rounding floor of one segment's integral (see [`integrate_core`]).
    fn line_integral(&self, noise: f64, mut omega: impl FnMut(f64, f64, f64, f64) -> f64) -> Result<f64, StokesError> {
        let mut total = 0.0;
        for i in 0..self.len.len() {
            if self.len[i] == 0.0 {
                continue;
            }
            total += integrate_core(
                |tau| {
                    let (p, d) = self.eval(i, tau);
                    omega(p[0], p[1], d[0], d[1])
                },
                0.0,
                1.0,
                QUADRATURE_REL_TOL,
                1,
                noise,
            )?;
        }
        Ok(total)
    }
}

/// The density of the cone-volume integral over the `(u, v)` region of a face (see [`Face`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OneForm {
    /// A plane. `d` is the signed distance of the apex `O` from the plane along the parametric normal `n = dp/du x dp/dv`,
    /// `d = (O - P0) . n` for any point `P0` of the plane. The cone density is `-d / 3` per unit of `(u, v)` area: positive
    /// when the apex lies behind the plane as seen from the normal, which is the case for an outward normal and an apex
    /// inside the solid.
    Plane { d: f64 },
    /// A cylinder of the given radius with `u` the angle and `v` the axial coordinate; `c` is the apex `O` in the plane of the
    /// circle, relative to the axis (its components along the directions `u = 0` and `u = pi / 2`). The cone density is
    /// `R (R - c . e(u)) / 3`.
    Cylinder { radius: f64, c: [f64; 2] },
}

impl OneForm {
    /// Density `k(u)` such that the cone volume of a patch with outward parametric normal is the integral of `k` over its
    /// `(u, v)` region.
    fn density(&self, u: f64) -> f64 {
        match *self {
            OneForm::Plane { d } => -d / 3.0,
            OneForm::Cylinder { radius, c } => radius * (radius - (c[0] * u.cos() + c[1] * u.sin())) / 3.0,
        }
    }
}

/// One boundary loop of a face: its edges as lists of `(u, v)` samples in traversal order (the last sample of an edge is
/// the first of the next).
#[derive(Clone, Debug, PartialEq)]
pub struct UvLoop {
    pub edges: Vec<Vec<[f64; 2]>>,
    /// An inner boundary (a hole of the face).
    pub is_hole: bool,
}

/// A face of a solid for the cone-volume sum.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub form: OneForm,
    /// `+1` when the parametric normal of `form` points out of the solid, `-1` when inward.
    pub outward: f64,
    pub loops: Vec<UvLoop>,
}

/// `+1` when the sample polygon of the loop is counter-clockwise in `(u, v)`, `-1` otherwise (shoelace sum).
fn loop_orientation(l: &UvLoop) -> f64 {
    let mut pts: Vec<[f64; 2]> = Vec::new();
    for e in &l.edges {
        for (k, p) in e.iter().enumerate() {
            if k > 0 || pts.is_empty() {
                pts.push(*p);
            }
        }
    }
    let mut twice_area = 0.0;
    for i in 0..pts.len() {
        let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
        twice_area += p[0] * q[1] - q[0] * p[1];
    }
    if twice_area < 0.0 {
        -1.0
    } else {
        1.0
    }
}

/// Integral of a 1-form over the loops of a face, each loop first brought to the orientation that makes an outer boundary
/// count positively (counter-clockwise) and a hole negatively (clockwise). `weight` bounds the size of the form's coefficients
/// and is used only to set the rounding floor of the quadrature.
fn face_loops_integral(loops: &[UvLoop], weight: f64, mut omega: impl FnMut(f64, f64, f64, f64) -> f64) -> Result<f64, StokesError> {
    if loops.is_empty() {
        return Err(StokesError::DegenerateFace);
    }
    let mut reach = 0.0_f64;
    for l in loops {
        for e in &l.edges {
            if e.len() < 2 || e.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
                return Err(StokesError::DegenerateFace);
            }
            for p in e {
                reach = reach.max(p[0].abs()).max(p[1].abs());
            }
        }
    }
    let noise = 1.0e-14 * weight * reach * reach;
    let mut total = 0.0;
    for l in loops {
        let mut sum = 0.0;
        for e in &l.edges {
            sum += EdgeSpline::new(e).line_integral(noise, &mut omega)?;
        }
        let sigma = loop_orientation(l);
        total += if l.is_hole { -sigma * sum } else { sigma * sum };
    }
    Ok(total)
}

/// Contribution of one face to the cone volume: `outward` times the integral of the cone density over the face. By Green's
/// theorem the integral over the region is the line integral of `-k(u) v du` around its loops.
pub fn face_volume(face: &Face) -> Result<f64, StokesError> {
    let form = face.form;
    let weight = match form {
        OneForm::Plane { d } => d.abs() / 3.0,
        OneForm::Cylinder { radius, c } => radius * (radius + c[0].abs() + c[1].abs()) / 3.0,
    };
    let integral = face_loops_integral(&face.loops, weight, |u, v, du, _dv| -form.density(u) * v * du)?;
    Ok(face.outward * integral)
}

/// The cone-volume sum over the faces of a closed solid (divergence theorem).
pub fn cone_volume(faces: &[Face]) -> Result<f64, StokesError> {
    let mut v = 0.0;
    for f in faces {
        v += face_volume(f)?;
    }
    Ok(v)
}

/// Area element of a surface in terms of `(u, v)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AreaForm {
    /// Plane parametrised by arc length in both directions: area is the `(u, v)` area.
    Plane,
    /// Cylinder with `u` the angle: area is `R` times the `(u, v)` area.
    Cylinder { radius: f64 },
}

impl AreaForm {
    fn scale(&self) -> f64 {
        match *self {
            AreaForm::Plane => 1.0,
            AreaForm::Cylinder { radius } => radius,
        }
    }
}

/// A face for the area sum: its area element and its loops (outer boundaries and holes alike).
#[derive(Clone, Debug, PartialEq)]
pub struct AreaFace {
    pub form: AreaForm,
    pub loops: Vec<UvLoop>,
}

/// Area of a face: the scale of its area element times `(1/2) closed-integral (u dv - v du)` over its loops (Green's theorem),
/// outer boundaries counting positively and holes negatively. A result that is negative only by rounding is returned as zero.
pub fn face_area(face: &AreaFace) -> Result<f64, StokesError> {
    let mut magnitude = 0.0;
    let total = face_loops_integral(&face.loops, 1.0, |u, v, du, dv| {
        let term = 0.5 * (u * dv - v * du);
        magnitude += term.abs();
        term
    })?;
    let a = face.form.scale() * total;
    let noise = 1.0e-12 * face.form.scale() * magnitude.max(1.0);
    if a < 0.0 && a >= -noise {
        return Ok(0.0);
    }
    Ok(a)
}

/// The area of a closed solid: the sum of its face areas.
pub fn solid_surface_area(faces: &[AreaFace]) -> Result<f64, StokesError> {
    let mut a = 0.0;
    for f in faces {
        a += face_area(f)?;
    }
    Ok(a)
}

// ---------------------------------------------------------------------------------------------
// Plan curves and exact boundary integrals

/// A plan curve for the exact boundary integrals.
#[derive(Clone, Copy, Debug)]
enum PlanCurve {
    Line { a: Vec2, b: Vec2 },
    Arc { c: Vec2, r: f64, a0: f64, sweep: f64 },
}

impl PlanCurve {
    fn from_curve(c: &crate::curve::Curve) -> PlanCurve {
        Self::from_curve_shifted(c, Vec2::new(0.0, 0.0))
    }

    /// The curve with the point `o` moved to the origin (the boundary integrals lose digits far from the origin).
    fn from_curve_shifted(c: &crate::curve::Curve, o: Vec2) -> PlanCurve {
        match c {
            crate::curve::Curve::Line(l) => PlanCurve::Line { a: l.start - o, b: l.end - o },
            crate::curve::Curve::Arc(a) => PlanCurve::Arc { c: a.centre - o, r: a.radius, a0: a.start_angle, sweep: a.sweep },
        }
    }

    fn length(&self) -> f64 {
        match *self {
            PlanCurve::Line { a, b } => (b - a).length(),
            PlanCurve::Arc { r, sweep, .. } => r * sweep.abs(),
        }
    }

    /// Largest distance of a point of the curve from the origin.
    fn reach(&self) -> f64 {
        match *self {
            PlanCurve::Line { a, b } => a.length().max(b.length()),
            PlanCurve::Arc { c, r, .. } => c.length() + r,
        }
    }

    /// Integral over the curve of `f(p, dp/dt)` for `t` in `[0, 1]`. `order` is the power of the distance from the origin that
    /// the terms of `f` carry (1 for `x dy`, 2 for `x^2 dy`, 0 for a plain length integral); it sets the rounding floor.
    fn integral(&self, order: i32, mut f: impl FnMut(Vec2, Vec2) -> f64) -> Result<f64, StokesError> {
        let noise = 1.0e-14 * self.reach().powi(order) * self.length();
        match *self {
            PlanCurve::Line { a, b } => {
                let d = b - a;
                integrate_core(|t| f(a + d * t, d), 0.0, 1.0, QUADRATURE_REL_TOL, 1, noise)
            }
            PlanCurve::Arc { c, r, a0, sweep } => {
                let panels = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
                integrate_core(
                    |t| {
                        let (s, co) = (a0 + sweep * t).sin_cos();
                        f(Vec2::new(c.x + r * co, c.y + r * s), Vec2::new(-r * sweep * s, r * sweep * co))
                    },
                    0.0,
                    1.0,
                    QUADRATURE_REL_TOL,
                    panels,
                    noise,
                )
            }
        }
    }

    /// Integral over the curve of `f(p) ds` with `s` the arc length; `order` as for [`PlanCurve::integral`] (the power of the
    /// distance from the origin in `f`).
    fn length_integral(&self, order: i32, mut f: impl FnMut(Vec2) -> f64) -> Result<f64, StokesError> {
        self.integral(order, |p, dp| f(p) * dp.length())
    }
}

fn chain_moments(chain: &[PlanCurve]) -> Result<(f64, f64, f64), StokesError> {
    let (mut a, mut mx, mut my) = (0.0, 0.0, 0.0);
    for c in chain {
        a += c.integral(1, |p, dp| 0.5 * (p.x * dp.y - p.y * dp.x))?;
        mx += c.integral(2, |p, dp| 0.5 * p.x * p.x * dp.y)?;
        my += c.integral(2, |p, dp| -0.5 * p.y * p.y * dp.x)?;
    }
    Ok((a, mx, my))
}

// ---------------------------------------------------------------------------------------------
// The annular-sector (arc wall) solid

/// An arc wall body: the band between two concentric arcs, extruded between two heights. Lengths are METRES.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcBand {
    pub centre: Vec2,
    /// Angle of the start of the location arc, radians.
    pub start_angle: f64,
    /// Signed sweep, radians (positive = counter-clockwise).
    pub sweep: f64,
    pub inner_radius: f64,
    pub outer_radius: f64,
    pub z0: f64,
    pub z1: f64,
}

impl ArcBand {
    /// The band of a wall of the given location arc and offset radii.
    pub fn wall(centre: Vec2, start_angle: f64, sweep: f64, inner_radius: f64, outer_radius: f64, z0: f64, z1: f64) -> Self {
        ArcBand { centre, start_angle, sweep, inner_radius, outer_radius, z0, z1 }
    }
}

/// The topology `(faces, edges, vertices)` of the solid extruded from a closed loop of `n` curves: two end faces and one
/// side face per curve, one edge per curve on each end face plus one vertical edge per curve, `2n` vertices (Euler's
/// relation `V - E + F = 2` gives `2n - 3n + n + 2 = 2`).
pub const fn extruded_loop_topology(n: usize) -> (usize, usize, usize) {
    (n + 2, 3 * n, 2 * n)
}

/// The number of curves of the closed profile of a wall: two arcs and two end caps.
pub const WALL_LOOP_CURVES: usize = 4;

/// Runs `f`. Kept for callers that evaluate a clipped body: the quantities of this module are exact and do not depend on
/// the frame in which a cap is described, so nothing changes inside the call.
pub fn with_clip_cap_world_frame<R>(f: impl FnOnce() -> R) -> R {
    f()
}

/// The angular ranges of the OUTER and the INNER arc of a band whose ends are not radial (an end cut by a plane that is not
/// radial): each arc runs from its first to its second angle along the sweep, and the end caps join the end points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EndRanges {
    pub outer: (f64, f64),
    pub inner: (f64, f64),
}

/// Plan geometry of a validated band.
struct BandPlan {
    area: f64,
    outer_len: f64,
    inner_len: f64,
    caps: [f64; 2],
}

fn at(c: Vec2, r: f64, a: f64) -> Vec2 {
    Vec2::new(c.x + r * a.cos(), c.y + r * a.sin())
}

/// Area of the region between two circles of one centre bounded by the planes through `(ro, outer.0)`-`(ri, inner.0)` and
/// `(ro, outer.1)`-`(ri, inner.1)`.
fn sector_between(ri: f64, ro: f64, outer: (f64, f64), inner: (f64, f64)) -> Result<f64, StokesError> {
    // the area does not depend on where the centre is: work about it, where the integrals are best conditioned
    let c = Vec2::new(0.0, 0.0);
    let arc = |r: f64, from: f64, to: f64| PlanCurve::Arc { c, r, a0: from, sweep: to - from };
    let chain = [
        arc(ro, outer.0, outer.1),
        PlanCurve::Line { a: at(c, ro, outer.1), b: at(c, ri, inner.1) },
        arc(ri, inner.1, inner.0),
        PlanCurve::Line { a: at(c, ri, inner.0), b: at(c, ro, outer.0) },
    ];
    Ok(chain_moments(&chain)?.0.abs())
}

fn band_plan(band: &ArcBand, ends: Option<&EndRanges>) -> Result<BandPlan, StokesError> {
    let (c, ri, ro) = (band.centre, band.inner_radius, band.outer_radius);
    let a1 = band.start_angle + band.sweep;
    if ![c.x, c.y, ri, ro, band.sweep, band.start_angle, band.z0, band.z1].iter().all(|v| v.is_finite()) {
        return Err(StokesError::DegenerateFace);
    }
    if ri < 0.0 {
        return Err(StokesError::NegativeOffsetRadius);
    }
    if !((ro - ri) * FEET_PER_METRE >= 1.0e-9) {
        return Err(StokesError::DegenerateCap);
    }
    let (outer, inner) = match ends {
        Some(e) => (e.outer, e.inner),
        None => ((band.start_angle, a1), (band.start_angle, a1)),
    };
    let (span_o, span_i) = ((outer.1 - outer.0).abs(), (inner.1 - inner.0).abs());
    let cap = |ai: f64, ao: f64| (at(c, ro, ao) - at(c, ri, ai)).length();
    let caps = [cap(inner.0, outer.0), cap(inner.1, outer.1)];
    let short = |len_m: f64| crate::tolerance::is_too_short_ft(len_m * FEET_PER_METRE);
    let arc_len = |span: f64, r: f64| crate::tolerance::arc_curve_length_ft(span, r * FEET_PER_METRE) / FEET_PER_METRE;
    let (outer_len, inner_len) = (arc_len(span_o, ro), arc_len(span_i, ri));
    if short(inner_len) || short(outer_len) || short(caps[0]) || short(caps[1]) {
        return Err(StokesError::CurveTooShort);
    }
    let area = sector_between(ri, ro, outer, inner)?;
    Ok(BandPlan { area, outer_len, inner_len, caps })
}

fn band_volume_of(plan: &BandPlan, band: &ArcBand) -> f64 {
    plan.area * (band.z1 - band.z0)
}

fn band_surface_of(plan: &BandPlan, band: &ArcBand) -> f64 {
    let h = band.z1 - band.z0;
    2.0 * plan.area + (plan.outer_len + plan.inner_len + plan.caps[0] + plan.caps[1]) * h
}

/// Volume of an [`ArcBand`], metres in and cubic metres out: the plan area (Green's theorem on the two arcs and the two end
/// caps) times the height. `tols` is not used: nothing here is sampled.
pub fn arc_band_volume(band: &ArcBand, _tols: &TessTols) -> Result<f64, StokesError> {
    Ok(band_volume_of(&band_plan(band, None)?, band))
}

/// [`arc_band_volume`] for a band whose ends are planes through the given angles on each circle.
pub fn arc_band_volume_slanted(band: &ArcBand, ends: &EndRanges, _tols: &TessTols) -> Result<f64, StokesError> {
    Ok(band_volume_of(&band_plan(band, Some(ends))?, band))
}

/// Surface area of an [`ArcBand`]: two plan faces, the two cylindrical faces `R * span * height` and the two end caps.
pub fn arc_band_surface_area(band: &ArcBand, _tols: &TessTols) -> Result<f64, StokesError> {
    Ok(band_surface_of(&band_plan(band, None)?, band))
}

/// [`arc_band_surface_area`] for a band with slanted ends.
pub fn arc_band_surface_area_slanted(band: &ArcBand, ends: &EndRanges, _tols: &TessTols) -> Result<f64, StokesError> {
    Ok(band_surface_of(&band_plan(band, Some(ends))?, band))
}

/// A rectangular cut through an arc band by a straight prism (the cutter of a door or window on an arc wall): the absolute
/// angles, about the centre, of the two planes of the cutter on the OUTER circle and on the INNER circle (`.0` the plane nearer
/// the start of the sweep), and the height range of the cut.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandCut {
    pub outer: (f64, f64),
    pub inner: (f64, f64),
    pub z_lo: f64,
    pub z_hi: f64,
}

/// Volume and surface area of an [`ArcBand`] with ONE [`BandCut`], in metres.
///
/// Three shapes are recognised from the heights: a NOTCH (the cut starts at the base and ends below the top), a HOLE (the cut
/// lies wholly inside the height of the band) and a SLOT (the cut spans the full height and splits the band in two). With
/// `F` the plan area of the cutter between the two circles, `ch` the height removed, `c_o` and `c_i` the lengths of the
/// removed strips of the outer and inner cylinders and `j` the two cutter planes (jambs):
///
/// * volume removed: `F * ch`;
/// * hole: surface changes by `-(c_o + c_i) ch + j ch + 2 F` (a head and a sill appear);
/// * notch: `-(c_o + c_i) ch + j ch` (the new head `F` replaces the bottom area `F` that was removed);
///
/// Any other arrangement is [`StokesError::DegenerateFace`].
pub fn arc_band_cut_metrics(band: &ArcBand, cut: &BandCut, _tols: &TessTols) -> Result<(f64, f64), StokesError> {
    let (c, ri, ro) = (band.centre, band.inner_radius, band.outer_radius);
    let eps = 1.0e-9;
    let (z0, z1) = (band.z0, band.z1);
    let slot = cut.z_lo <= z0 + eps && cut.z_hi >= z1 - eps;
    let notch = (cut.z_lo <= z0 + eps && cut.z_hi < z1 - eps) || slot;
    let hole = cut.z_lo > z0 + eps && cut.z_hi < z1 - eps;
    if !(notch || hole) || ri <= 0.0 {
        return Err(StokesError::DegenerateFace);
    }
    let plan = band_plan(band, None)?;
    let h = z1 - z0;
    let ch = cut.z_hi.min(z1) - cut.z_lo.max(z0);
    let footprint = sector_between(ri, ro, cut.outer, cut.inner)?;
    let strips = ro * (cut.outer.1 - cut.outer.0).abs() + ri * (cut.inner.1 - cut.inner.0).abs();
    let jambs = (at(c, ro, cut.outer.0) - at(c, ri, cut.inner.0)).length() + (at(c, ro, cut.outer.1) - at(c, ri, cut.inner.1)).length();
    let volume = band_volume_of(&plan, band) - footprint * ch;
    let base = band_surface_of(&plan, band);
    let surface = if slot {
        base - strips * h + jambs * h - 2.0 * footprint
    } else if hole {
        base - strips * ch + jambs * ch + 2.0 * footprint
    } else {
        base - strips * ch + jambs * ch
    };
    Ok((volume, surface))
}

// ---------------------------------------------------------------------------------------------
// Curve loops extruded between two planes (slab bodies)

/// A cap plane `z = c0 + g . p`, METRES (`g` is the rise per metre along x and y).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapPlane {
    pub c0: f64,
    pub g: Vec2,
}

impl CapPlane {
    /// A horizontal plane at `z`.
    pub fn flat(z: f64) -> Self {
        CapPlane { c0: z, g: Vec2::new(0.0, 0.0) }
    }

    /// Elevation of the plane above plan point `p`.
    pub fn at(&self, p: Vec2) -> f64 {
        self.c0 + self.g.x * p.x + self.g.y * p.y
    }

    /// The same plane lifted by `dz`.
    pub fn lifted(&self, dz: f64) -> Self {
        CapPlane { c0: self.c0 + dz, g: self.g }
    }

    /// Cosine of the angle between the plane's normal and the vertical: `1 / sqrt(1 + |g|^2)`.
    pub fn vertical_cosine(&self) -> f64 {
        1.0 / (1.0 + self.g.dot(self.g)).sqrt()
    }
}

/// One closed loop of plan curves (METRES) bounding a slab region, oriented with the MATERIAL ON ITS LEFT (material loops
/// counter-clockwise, holes clockwise).
#[derive(Clone, Debug, PartialEq)]
pub struct CapLoop {
    pub curves: Vec<crate::curve::Curve>,
    pub is_hole: bool,
}

/// Quantities of a body bounded by two planes and vertical side faces, METRES.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExtrudedLoopsMetrics {
    pub top_area: f64,
    pub bottom_area: f64,
    pub volume: f64,
    pub surface_area: f64,
}

/// The point the cone volumes are measured from. The exact quantities of this module do not depend on it (the cone sum is
/// independent of the apex); both values give the same result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferencePoint {
    UvEnvelope,
    WorldBox,
}

/// The sample points of an arc, METRES, uniform in the angle, both ends included ([`arc_sample_count`]). They are for
/// drawing; no quantity is computed from them.
pub fn arc_samples_m(arc: &crate::curve::Arc, tols: &TessTols) -> Result<Vec<Vec2>, StokesError> {
    let n = arc_sample_count(arc.sweep.abs(), arc.radius, tols)?;
    Ok((0..n).map(|i| arc.point_at(i as f64 / (n - 1) as f64)).collect())
}

/// The plan box of the sample points of a curve, METRES (a line: its two ends; an arc: its drawing samples, so an extreme
/// that falls between two samples is not reached). `None` when the sampling rule refuses the arc.
pub fn sampled_curve_box(c: &crate::curve::Curve, tols: &TessTols) -> Option<(Vec2, Vec2)> {
    let pts: Vec<Vec2> = match c {
        crate::curve::Curve::Line(l) => vec![l.start, l.end],
        crate::curve::Curve::Arc(a) => arc_samples_m(a, tols).ok()?,
    };
    let mut lo = Vec2::new(f64::INFINITY, f64::INFINITY);
    let mut hi = Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in pts {
        lo = Vec2::new(lo.x.min(p.x), lo.y.min(p.y));
        hi = Vec2::new(hi.x.max(p.x), hi.y.max(p.y));
    }
    Some((lo, hi))
}

/// Plan area and first moments of the loops, material positive and holes negative, whatever the stored direction of each loop.
fn loops_moments(loops: &[CapLoop]) -> Result<(f64, f64, f64), StokesError> {
    let origin = match loops.iter().find_map(|l| l.curves.first()) {
        Some(c) => c.start(),
        None => return Err(StokesError::DegenerateFace),
    };
    let (mut a, mut mx, mut my) = (0.0, 0.0, 0.0);
    for l in loops {
        if l.curves.is_empty() {
            return Err(StokesError::DegenerateFace);
        }
        let chain: Vec<PlanCurve> = l.curves.iter().map(|c| PlanCurve::from_curve_shifted(c, origin)).collect();
        if chain.iter().any(|c| !(c.length() > 0.0) || !c.length().is_finite()) {
            return Err(StokesError::DegenerateFace);
        }
        let (la, lmx, lmy) = chain_moments(&chain)?;
        let s = if la < 0.0 { -1.0 } else { 1.0 } * if l.is_hole { -1.0 } else { 1.0 };
        a += s * la;
        mx += s * lmx;
        my += s * lmy;
    }
    Ok((a, mx + origin.x * a, my + origin.y * a))
}

fn plane_is_finite(p: &CapPlane) -> bool {
    p.c0.is_finite() && p.g.x.is_finite() && p.g.y.is_finite()
}

/// Quantities of the body bounded by `loops` between the parallel or inclined planes `bottom` and `top` (METRES in and out):
///
/// * cap areas: `A * sqrt(1 + |g|^2)` for each plane, `A` the plan area (Green's theorem, holes subtracted);
/// * volume: the integral over the plan region of `top(p) - bottom(p)`, which is `A * dc + dg . (first moments)`;
/// * surface area: the two caps plus, for every curve of every loop, the vertical strip `integral of (top - bottom) ds`.
pub fn extruded_loops_metrics(loops: &[CapLoop], bottom: CapPlane, top: CapPlane, tols: &TessTols) -> Result<ExtrudedLoopsMetrics, StokesError> {
    extruded_loops_metrics_ref(loops, bottom, top, tols, ReferencePoint::UvEnvelope)
}

/// [`extruded_loops_metrics`] with an explicit reference point (which cannot change the result).
pub fn extruded_loops_metrics_ref(
    loops: &[CapLoop],
    bottom: CapPlane,
    top: CapPlane,
    _tols: &TessTols,
    _reference: ReferencePoint,
) -> Result<ExtrudedLoopsMetrics, StokesError> {
    if !plane_is_finite(&bottom) || !plane_is_finite(&top) {
        return Err(StokesError::DegenerateFace);
    }
    let (a, mx, my) = loops_moments(loops)?;
    let dc = top.c0 - bottom.c0;
    let dg = top.g - bottom.g;
    let volume = dc * a + dg.x * mx + dg.y * my;
    let top_area = a * (1.0 + top.g.dot(top.g)).sqrt();
    let bottom_area = a * (1.0 + bottom.g.dot(bottom.g)).sqrt();
    let mut strips = 0.0;
    for l in loops {
        for c in &l.curves {
            let curve = PlanCurve::from_curve(c);
            strips += curve.length_integral(0, |p| dc + dg.dot(p))?;
        }
    }
    Ok(ExtrudedLoopsMetrics { top_area, bottom_area, volume, surface_area: top_area + bottom_area + strips })
}

/// Area of ONE planar face bounded by `loops` on `plane` (a slab of zero depth), METRES.
pub fn cap_area(loops: &[CapLoop], plane: CapPlane, _tols: &TessTols) -> Result<f64, StokesError> {
    if !plane_is_finite(&plane) {
        return Err(StokesError::DegenerateFace);
    }
    let (a, _, _) = loops_moments(loops)?;
    Ok(a * (1.0 + plane.g.dot(plane.g)).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::{Arc as CircArc, Curve};
    use std::f64::consts::{FRAC_PI_2, PI};

    fn tols() -> TessTols {
        TessTols::display_default()
    }

    fn rel(a: f64, b: f64) -> f64 {
        (a - b).abs() / b.abs().max(1e-300)
    }

    fn wall_band(centre: Vec2, a0: f64, sweep: f64, ri: f64, ro: f64, h: f64) -> ArcBand {
        ArcBand::wall(centre, a0, sweep, ri, ro, 0.0, h)
    }

    #[test]
    fn the_sample_count_keeps_the_sag_and_the_step_within_their_limits() {
        for &(span, r) in &[(PI, 2.0), (FRAC_PI_2, 3.0), (1.5 * PI, 0.2), (0.3, 50.0), (2.0 * PI, 1.0e-3), (PI, 120.0)] {
            let n = arc_sample_count(span, r, &tols()).unwrap();
            assert!(n >= 3);
            let step = span / (n - 1) as f64;
            assert!(r * (1.0 - (step / 2.0).cos()) <= tols().sag * (1.0 + 1e-9), "sag, span {span} r {r}");
            assert!(step <= tols().max_step * (1.0 + 1e-9), "step, span {span} r {r}");
            // one sample fewer would break a limit
            if n > 3 {
                let coarser = span / (n - 2) as f64;
                assert!(r * (1.0 - (coarser / 2.0).cos()) > tols().sag || coarser > tols().max_step);
            }
        }
    }

    #[test]
    fn the_sample_count_refuses_input_outside_its_domain() {
        assert_eq!(arc_sample_count(1.0, 1.0, &TessTols { sag: 0.0, ..tols() }), Err(StokesError::InvalidTessellationInput));
        assert_eq!(arc_sample_count(-1.0, 1.0, &tols()), Err(StokesError::InvalidTessellationInput));
        assert_eq!(arc_sample_count(1.0, f64::NAN, &tols()), Err(StokesError::InvalidTessellationInput));
        assert_eq!(arc_sample_count(1.0e9, 1.0, &tols()), Err(StokesError::TessellationUnreachable));
        // a tiny arc still gets the minimum of three samples
        assert_eq!(arc_sample_count(0.001, 1.0, &tols()).unwrap(), 3);
    }

    #[test]
    fn a_small_radius_is_limited_by_the_step_angle_not_by_the_sag() {
        let r = 0.01; // sag 1 mm is 10 % of the radius: the sag allows ~51 degrees, the step 5
        let n = arc_sample_count(PI, r, &tols()).unwrap();
        assert_eq!(n, 37); // 180 / 5 = 36 segments
    }

    #[test]
    fn the_legendre_rules_are_exact_for_polynomials_up_to_degree_2n_minus_1() {
        let (r8, r16) = rules();
        for (rule, n) in [(r8, 8usize), (r16, 16usize)] {
            assert!((rule.w.iter().sum::<f64>() - 2.0).abs() < 1e-14);
            for k in 0..2 * n {
                let q: f64 = rule.x.iter().zip(&rule.w).map(|(x, w)| w * x.powi(k as i32)).sum();
                let exact = if k % 2 == 1 { 0.0 } else { 2.0 / (k as f64 + 1.0) };
                assert!((q - exact).abs() < 1e-14, "n {n} degree {k}: {q} vs {exact}");
            }
        }
    }

    #[test]
    fn the_adaptive_quadrature_integrates_smooth_and_kinked_functions() {
        let v = integrate(|x| x.sin(), 0.0, PI, QUADRATURE_REL_TOL).unwrap();
        assert!((v - 2.0).abs() < 1e-13, "{v}");
        let v = integrate(|x| 3.0 * x * x, 0.0, 2.0, QUADRATURE_REL_TOL).unwrap();
        assert!((v - 8.0).abs() < 1e-13, "{v}");
        // a kink at 0.3 is found by bisection: integral of |x - 0.3| over [0, 1] is (0.09 + 0.49) / 2
        let v = integrate(|x| (x - 0.3).abs(), 0.0, 1.0, QUADRATURE_REL_TOL).unwrap();
        assert!((v - 0.29).abs() < 1e-12, "{v}");
        // an integrand that cancels to zero does not demand an impossible relative accuracy
        let v = integrate(|x| x.sin(), -1.0, 1.0, QUADRATURE_REL_TOL).unwrap();
        assert!(v.abs() < 1e-14, "{v}");
        // a reversed interval changes the sign
        let w = integrate(|x| x.cos(), 1.0, 0.0, QUADRATURE_REL_TOL).unwrap();
        assert!((w + 1.0_f64.sin()).abs() < 1e-14);
    }

    #[test]
    fn a_narrow_peak_is_found_by_bisection() {
        // a Gaussian of width 0.01 centred in [0, 1]: the integral is 0.01 sqrt(pi) to far better than the tolerance
        let w = 0.01_f64;
        let v = integrate(|x| (-((x - 0.5) / w).powi(2)).exp(), 0.0, 1.0, QUADRATURE_REL_TOL).unwrap();
        assert!(rel(v, w * PI.sqrt()) < 1e-12, "{v}");
    }

    #[test]
    fn a_non_finite_integrand_is_an_error() {
        assert_eq!(integrate(|_| f64::NAN, 0.0, 1.0, QUADRATURE_REL_TOL), Err(StokesError::IntegrationDidNotConverge));
        assert_eq!(integrate(|x| 1.0 / x, 0.0, 1.0, QUADRATURE_REL_TOL), Err(StokesError::IntegrationDidNotConverge));
    }

    #[test]
    fn the_edge_spline_reproduces_a_line_on_uneven_spacing_and_a_parabola_closely() {
        // collinear samples at any spacing give back the line exactly
        let xs = [0.0, 0.5, 1.7, 2.0, 3.5, 4.0];
        let line: Vec<[f64; 2]> = xs.iter().map(|&x| [x, 0.5 * x + 1.0]).collect();
        let s = EdgeSpline::new(&line);
        for i in 0..line.len() - 1 {
            let (p, d) = s.eval(i, 0.37);
            assert!((p[1] - (0.5 * p[0] + 1.0)).abs() < 1e-13 && (d[1] / d[0] - 0.5).abs() < 1e-13, "segment {i}");
        }
        // samples of a parabola, uneven: the curve stays within 2e-5 of it (spacing about 0.1; the end segments are the least accurate)
        let xs = [0.0, 0.1, 0.22, 0.3, 0.41, 0.5, 0.62, 0.7];
        let par: Vec<[f64; 2]> = xs.iter().map(|&x| [x, 0.3 * x * x + 1.0]).collect();
        let s = EdgeSpline::new(&par);
        for i in 0..par.len() - 1 {
            let (p, _) = s.eval(i, 0.5);
            assert!((p[1] - (0.3 * p[0] * p[0] + 1.0)).abs() < 2e-5, "segment {i}: {}", (p[1] - (0.3 * p[0] * p[0] + 1.0)).abs());
        }
    }

    #[test]
    fn two_samples_are_a_straight_line() {
        let pts = [[1.0, 2.0], [5.0, 10.0]];
        let s = EdgeSpline::new(&pts);
        let (p, d) = s.eval(0, 0.25);
        assert!((p[0] - 2.0).abs() < 1e-14 && (p[1] - 4.0).abs() < 1e-14);
        assert!((d[0] - 4.0).abs() < 1e-14 && (d[1] - 8.0).abs() < 1e-14);
        let w = s.line_integral(0.0, |u, v, du, dv| u * dv - v * du).unwrap();
        assert!((w - (1.0 * 10.0 - 2.0 * 5.0)).abs() < 1e-13);
    }

    fn quad(a: [f64; 2]) -> UvLoop {
        UvLoop {
            edges: vec![
                vec![[0.0, 0.0], [a[0], 0.0]],
                vec![[a[0], 0.0], [a[0], a[1]]],
                vec![[a[0], a[1]], [0.0, a[1]]],
                vec![[0.0, a[1]], [0.0, 0.0]],
            ],
            is_hole: false,
        }
    }

    fn cross3(u: [f64; 3], v: [f64; 3]) -> [f64; 3] {
        [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
    }

    /// A box is a closed solid of six planar faces: the cone-volume sum must equal width x depth x height from any apex.
    #[test]
    fn a_box_is_exact_from_any_apex() {
        let (w, d, h) = (3.0, 2.0, 5.0);
        for o in [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [10.0, -4.0, 7.0]] {
            // (u axis, v axis, a point of the plane, extent in u and v); the parametric normal is u x v
            let specs: [([f64; 3], [f64; 3], [f64; 3], [f64; 2]); 6] = [
                ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 0.0], [d, h]),
                ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [w, 0.0, 0.0], [d, h]),
                ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [h, w]),
                ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, d, 0.0], [h, w]),
                ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.0], [w, d]),
                ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, h], [w, d]),
            ];
            let faces: Vec<Face> = specs
                .iter()
                .enumerate()
                .map(|(i, (u, v, p0, ext))| {
                    let n = cross3(*u, *v);
                    // the apex's signed distance from the plane along the parametric normal; `outward` says whether that
                    // normal points out of the solid
                    let dist = (o[0] - p0[0]) * n[0] + (o[1] - p0[1]) * n[1] + (o[2] - p0[2]) * n[2];
                    Face { form: OneForm::Plane { d: dist }, outward: if i % 2 == 0 { -1.0 } else { 1.0 }, loops: vec![quad(*ext)] }
                })
                .collect();
            let v = cone_volume(&faces).unwrap();
            assert!((v - w * d * h).abs() < 1e-12, "apex {o:?}: {v}");
        }
    }

    /// A cylinder and its two discs: the cone sum from an off-axis apex equals the analytic volume as the sampling is refined.
    #[test]
    fn a_sampled_cylinder_converges_to_the_analytic_volume() {
        let (r, h) = (2.0, 3.0);
        let exact = PI * r * r * h;
        let mut previous = f64::INFINITY;
        for n in [16usize, 32, 64, 128, 256] {
            let ang = |i: usize| 2.0 * PI * i as f64 / n as f64;
            let ring: Vec<[f64; 2]> = (0..=n).map(|i| [r * ang(i).cos(), r * ang(i).sin()]).collect();
            let c = [0.4, -0.7, 1.1]; // apex inside
            // wall: u = angle, v = z, parametric normal radial (outward)
            let wall = Face {
                form: OneForm::Cylinder { radius: r, c: [c[0], c[1]] },
                outward: 1.0,
                loops: vec![UvLoop {
                    edges: vec![
                        (0..=n).map(|i| [ang(i), 0.0]).collect(),
                        vec![[2.0 * PI, 0.0], [2.0 * PI, h]],
                        (0..=n).rev().map(|i| [ang(i), h]).collect(),
                        vec![[0.0, h], [0.0, 0.0]],
                    ],
                    is_hole: false,
                }],
            };
            let disc = |z: f64, normal_up: bool| Face {
                form: OneForm::Plane { d: (c[2] - z) * if normal_up { 1.0 } else { -1.0 } },
                outward: 1.0,
                loops: vec![UvLoop { edges: vec![ring.clone()], is_hole: false }],
            };
            // the top disc has normal +z with u = x, v = y; the bottom disc is parametrised with normal -z (v mirrored)
            let mut bottom = disc(0.0, false);
            for e in &mut bottom.loops[0].edges {
                for p in e.iter_mut() {
                    p[1] = -p[1];
                }
            }
            let v = cone_volume(&[wall, disc(h, true), bottom]).unwrap();
            let err = (v - exact).abs();
            assert!(err < previous, "n {n}: error {err} did not shrink");
            previous = err;
        }
        assert!(previous / exact < 1e-7, "{previous}");
    }

    #[test]
    fn a_plane_face_area_is_the_enclosed_uv_area_whatever_the_stored_direction() {
        let sq = |ccw: bool| {
            let mut pts = vec![[1.0, 2.0], [4.0, 2.0], [4.0, 7.0], [1.0, 7.0]];
            if !ccw {
                pts.reverse();
            }
            UvLoop { edges: (0..4).map(|i| vec![pts[i], pts[(i + 1) % 4]]).collect(), is_hole: false }
        };
        for ccw in [true, false] {
            let f = AreaFace { form: AreaForm::Plane, loops: vec![sq(ccw)] };
            assert!((face_area(&f).unwrap() - 15.0).abs() < 1e-12);
        }
    }

    #[test]
    fn inner_loops_subtract_from_the_outer_boundary() {
        let rect = |x0: f64, y0: f64, x1: f64, y1: f64, hole: bool| {
            let pts = if hole { [[x0, y0], [x0, y1], [x1, y1], [x1, y0]] } else { [[x0, y0], [x1, y0], [x1, y1], [x0, y1]] };
            UvLoop { edges: (0..4).map(|i| vec![pts[i], pts[(i + 1) % 4]]).collect(), is_hole: hole }
        };
        let f = AreaFace { form: AreaForm::Plane, loops: vec![rect(0.0, 0.0, 10.0, 10.0, false), rect(2.0, 2.0, 5.0, 4.0, true)] };
        assert!((face_area(&f).unwrap() - 94.0).abs() < 1e-12);
        let g = AreaFace {
            form: AreaForm::Plane,
            loops: vec![rect(0.0, 0.0, 10.0, 10.0, false), rect(2.0, 2.0, 5.0, 4.0, true), rect(6.0, 6.0, 9.0, 9.0, true)],
        };
        assert!((face_area(&g).unwrap() - 85.0).abs() < 1e-12);
    }

    #[test]
    fn a_cylinder_patch_area_is_radius_times_the_uv_area() {
        let f = AreaFace { form: AreaForm::Cylinder { radius: 2.5 }, loops: vec![quad_at(0.4, 1.0, 1.9, 4.0)] };
        assert!((face_area(&f).unwrap() - 2.5 * 1.5 * 3.0).abs() < 1e-12);
    }

    fn quad_at(x0: f64, y0: f64, x1: f64, y1: f64) -> UvLoop {
        let pts = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        UvLoop { edges: (0..4).map(|i| vec![pts[i], pts[(i + 1) % 4]]).collect(), is_hole: false }
    }

    #[test]
    fn a_face_without_loops_or_with_a_one_point_edge_is_an_error() {
        assert_eq!(face_area(&AreaFace { form: AreaForm::Plane, loops: vec![] }), Err(StokesError::DegenerateFace));
        let bad = UvLoop { edges: vec![vec![[0.0, 0.0]]], is_hole: false };
        assert_eq!(face_area(&AreaFace { form: AreaForm::Plane, loops: vec![bad] }), Err(StokesError::DegenerateFace));
        let nan = quad_at(0.0, 0.0, f64::NAN, 1.0);
        assert_eq!(face_area(&AreaFace { form: AreaForm::Plane, loops: vec![nan] }), Err(StokesError::DegenerateFace));
    }

    #[test]
    fn the_area_of_a_box_is_the_sum_of_its_faces() {
        let (w, d, h) = (3.0, 2.0, 5.0);
        let faces: Vec<AreaFace> =
            [(w, d), (w, d), (w, h), (w, h), (d, h), (d, h)].iter().map(|&(a, b)| AreaFace { form: AreaForm::Plane, loops: vec![quad_at(0.0, 0.0, a, b)] }).collect();
        assert!((solid_surface_area(&faces).unwrap() - 2.0 * (w * d + w * h + d * h)).abs() < 1e-12);
    }

    #[test]
    fn a_circle_sampled_with_halving_spacing_gains_a_factor_of_sixteen() {
        let r = 5.0;
        let mut previous = f64::INFINITY;
        for n in [16usize, 32, 64, 128, 256] {
            let ring: Vec<[f64; 2]> = (0..=n).map(|i| [r * (2.0 * PI * i as f64 / n as f64).cos(), r * (2.0 * PI * i as f64 / n as f64).sin()]).collect();
            let f = AreaFace { form: AreaForm::Plane, loops: vec![UvLoop { edges: vec![ring], is_hole: false }] };
            let err = (face_area(&f).unwrap() - PI * r * r).abs();
            // halving the spacing divides the error by about 16
            assert!(err < previous / 12.0, "n {n}: {err} after {previous}");
            previous = err;
        }
        assert!(previous < 1e-6, "{previous}");
    }

    #[test]
    fn the_band_volume_is_the_annular_sector_prism() {
        for &(r, theta, t, h) in &[(2.0, PI, 0.2, 3.0), (3.0, FRAC_PI_2, 0.3, 2.5), (2.0, 1.5 * PI, 0.2, 3.0), (40.0, 0.25, 0.4, 4.0)] {
            let (ri, ro) = (r - t / 2.0, r + t / 2.0);
            for sign in [1.0, -1.0] {
                let band = wall_band(Vec2::new(10.0, -3.0), 0.7, sign * theta, ri, ro, h);
                let exact = 0.5 * theta * (ro * ro - ri * ri) * h;
                let v = arc_band_volume(&band, &tols()).unwrap();
                assert!(rel(v, exact) < 1e-13, "r {r} theta {theta}: {v} vs {exact}");
            }
        }
    }

    #[test]
    fn the_band_surface_area_is_the_sum_of_its_six_faces() {
        let (ri, ro, h, theta) = (1.9, 2.1, 3.0, PI);
        let band = wall_band(Vec2::new(0.0, 0.0), 0.0, theta, ri, ro, h);
        let analytic = theta * (ro * ro - ri * ri) + theta * ro * h + theta * ri * h + 2.0 * (ro - ri) * h;
        let a = arc_band_surface_area(&band, &tols()).unwrap();
        assert!(rel(a, analytic) < 1e-13, "{a} vs {analytic}");
    }

    #[test]
    fn a_band_far_from_the_origin_and_its_mirror_image_have_the_same_quantities() {
        let a = wall_band(Vec2::new(3.0, -1.0), 0.4, 2.0, 2.9, 3.1, 3.5);
        let near = wall_band(Vec2::new(0.0, 0.0), 0.4, 2.0, 2.9, 3.1, 3.5);
        let far = wall_band(Vec2::new(1.0e4, -2.5e4), 0.4, 2.0, 2.9, 3.1, 3.5);
        let mirror = ArcBand { centre: Vec2::new(3.0, 1.0), start_angle: -0.4, sweep: -2.0, ..a };
        let v = arc_band_volume(&a, &tols()).unwrap();
        for b in [near, far, mirror] {
            assert!(rel(arc_band_volume(&b, &tols()).unwrap(), v) < 1e-10);
            assert!(rel(arc_band_surface_area(&b, &tols()).unwrap(), arc_band_surface_area(&a, &tols()).unwrap()) < 1e-10);
        }
    }

    /// The plan area of a polygon with many vertices on the arcs, by the shoelace formula, as an independent reference.
    fn polygon_area(pts: &[Vec2]) -> f64 {
        let n = pts.len();
        0.5 * (0..n).map(|i| pts[i].cross(pts[(i + 1) % n])).sum::<f64>().abs()
    }

    #[test]
    fn slanted_ends_agree_with_a_fine_polygon_and_with_the_radial_case() {
        let c = Vec2::new(1.0, 2.0);
        let (ri, ro, h) = (2.0, 2.4, 3.0);
        let band = wall_band(c, 0.0, FRAC_PI_2, ri, ro, h);
        let radial = EndRanges { outer: (0.0, FRAC_PI_2), inner: (0.0, FRAC_PI_2) };
        assert!(rel(arc_band_volume_slanted(&band, &radial, &tols()).unwrap(), arc_band_volume(&band, &tols()).unwrap()) < 1e-13);
        assert!(rel(arc_band_surface_area_slanted(&band, &radial, &tols()).unwrap(), arc_band_surface_area(&band, &tols()).unwrap()) < 1e-13);
        let ends = EndRanges { outer: (0.1, 1.45), inner: (0.3, 1.3) };
        let n = 200_000usize;
        let mut poly: Vec<Vec2> = Vec::new();
        for i in 0..=n {
            let a = ends.outer.0 + (ends.outer.1 - ends.outer.0) * i as f64 / n as f64;
            poly.push(Vec2::new(c.x + ro * a.cos(), c.y + ro * a.sin()));
        }
        for i in 0..=n {
            let a = ends.inner.1 + (ends.inner.0 - ends.inner.1) * i as f64 / n as f64;
            poly.push(Vec2::new(c.x + ri * a.cos(), c.y + ri * a.sin()));
        }
        let want = polygon_area(&poly) * h;
        let got = arc_band_volume_slanted(&band, &ends, &tols()).unwrap();
        assert!(rel(got, want) < 1e-9, "{got} vs {want}");
    }

    #[test]
    fn a_degenerate_band_is_an_error_not_a_number() {
        let band = wall_band(Vec2::new(0.0, 0.0), 0.0, 0.0, 1.0, 2.0, 1.0);
        assert_eq!(arc_band_volume(&band, &tols()), Err(StokesError::CurveTooShort));
        assert_eq!(arc_band_surface_area(&band, &tols()), Err(StokesError::CurveTooShort));
        assert_eq!(arc_band_volume(&ArcBand { sweep: f64::NAN, ..band }, &tols()), Err(StokesError::DegenerateFace));
    }

    #[test]
    fn the_profile_checks_answer_in_a_fixed_order() {
        let ft = 0.3048;
        let at = |inner_ft: f64, outer_ft: f64, sweep: f64| arc_band_volume(&ArcBand::wall(Vec2::new(0.0, 0.0), 0.0, sweep, inner_ft * ft, outer_ft * ft, 0.0, 3.0), &tols());
        assert!(at(2.0, 2.2, 1.0).is_ok());
        assert_eq!(at(-0.1, 0.5, 1.0), Err(StokesError::NegativeOffsetRadius));
        assert_eq!(at(0.0, 0.5, 1.0), Err(StokesError::CurveTooShort));
        assert_eq!(at(2.0, 2.0 + 1e-10, 1.0), Err(StokesError::DegenerateCap));
        assert_eq!(at(2.0, 2.002, 1.0), Err(StokesError::CurveTooShort));
        assert!(at(2.0, 2.0026, 1.0).is_ok());
        assert_eq!(at(0.0025, 1.0, 1.0), Err(StokesError::CurveTooShort));
        assert!(at(0.0026, 1.0, 1.0).is_ok());
    }

    #[test]
    fn the_topology_of_an_extruded_loop_satisfies_euler_s_relation() {
        assert_eq!(WALL_LOOP_CURVES, 4);
        assert_eq!(extruded_loop_topology(WALL_LOOP_CURVES), (6, 12, 8));
        for n in 3..12 {
            let (f, e, v) = extruded_loop_topology(n);
            assert_eq!(v as i64 - e as i64 + f as i64, 2);
        }
    }

    fn poly_loop(pts: &[(f64, f64)], is_hole: bool) -> CapLoop {
        let n = pts.len();
        CapLoop { curves: (0..n).map(|i| Curve::line(Vec2::new(pts[i].0, pts[i].1), Vec2::new(pts[(i + 1) % n].0, pts[(i + 1) % n].1))).collect(), is_hole }
    }

    #[test]
    fn a_polygon_prism_with_a_hole_is_exact() {
        let outer = poly_loop(&[(0.0, 0.0), (6.0, 0.0), (6.0, 4.0), (0.0, 4.0)], false);
        let hole = poly_loop(&[(1.0, 1.0), (1.0, 2.0), (2.0, 2.0), (2.0, 1.0)], true);
        let m = extruded_loops_metrics(&[outer, hole], CapPlane::flat(-0.2), CapPlane::flat(0.0), &tols()).unwrap();
        assert!((m.top_area - 23.0).abs() < 1e-12 && (m.bottom_area - 23.0).abs() < 1e-12, "{m:?}");
        assert!((m.volume - 23.0 * 0.2).abs() < 1e-12, "{m:?}");
        assert!((m.surface_area - (2.0 * 23.0 + 24.0 * 0.2)).abs() < 1e-12, "{m:?}");
    }

    #[test]
    fn inclined_caps_give_the_inclined_area_and_the_prism_volume() {
        let outer = poly_loop(&[(0.0, 0.0), (6.0, 0.0), (6.0, 4.0), (0.0, 4.0)], false);
        let g = Vec2::new(0.0, 0.5);
        let vt = 0.3 * 1.25_f64.sqrt();
        let top = CapPlane { c0: 0.0, g };
        let m = extruded_loops_metrics(&[outer], top.lifted(-vt), top, &tols()).unwrap();
        assert!((m.top_area - 24.0 * 1.25_f64.sqrt()).abs() < 1e-12, "{m:?}");
        assert!((m.volume - 24.0 * vt).abs() < 1e-12, "{m:?}");
        // two planes that are not parallel: the volume is the plan area times the mean thickness at the centroid
        let tilted = extruded_loops_metrics(&[poly_loop(&[(0.0, 0.0), (6.0, 0.0), (6.0, 4.0), (0.0, 4.0)], false)], CapPlane::flat(0.0), CapPlane { c0: 1.0, g: Vec2::new(0.1, 0.0) }, &tols()).unwrap();
        assert!((tilted.volume - 24.0 * (1.0 + 0.1 * 3.0)).abs() < 1e-12, "{tilted:?}");
    }

    #[test]
    fn a_slab_with_arcs_matches_the_closed_forms_and_the_band() {
        let (ri, ro, h) = (1.9, 2.1, 3.0);
        let c = Vec2::new(0.0, 0.0);
        let l = CapLoop {
            curves: vec![
                Curve::Arc(CircArc::new(c, ro, 0.0, PI).unwrap()),
                Curve::line(Vec2::new(-ro, 0.0), Vec2::new(-ri, 0.0)),
                Curve::Arc(CircArc::new(c, ri, PI, -PI).unwrap()),
                Curve::line(Vec2::new(ri, 0.0), Vec2::new(ro, 0.0)),
            ],
            is_hole: false,
        };
        let m = extruded_loops_metrics(&[l], CapPlane::flat(0.0), CapPlane::flat(h), &tols()).unwrap();
        let band = wall_band(c, PI, -PI, ri, ro, h);
        assert!(rel(m.volume, arc_band_volume(&band, &tols()).unwrap()) < 1e-12);
        assert!(rel(m.surface_area, arc_band_surface_area(&band, &tols()).unwrap()) < 1e-12);
        assert!(rel(m.top_area, 0.5 * PI * (ro * ro - ri * ri)) < 1e-13);
    }

    #[test]
    fn a_disc_of_two_half_circles_has_the_exact_area_and_volume() {
        let r = 5.0;
        let c = Vec2::new(1.0, 2.0);
        let l = CapLoop { curves: vec![Curve::Arc(CircArc::new(c, r, 0.0, PI).unwrap()), Curve::Arc(CircArc::new(c, r, PI, PI).unwrap())], is_hole: false };
        let a = cap_area(&[l.clone()], CapPlane::flat(0.0), &tols()).unwrap();
        assert!(rel(a, PI * r * r) < 1e-13, "{a}");
        let m = extruded_loops_metrics(&[l], CapPlane::flat(0.0), CapPlane::flat(0.25), &tols()).unwrap();
        assert!(rel(m.volume, PI * r * r * 0.25) < 1e-13, "{m:?}");
        assert!(rel(m.surface_area, 2.0 * PI * r * r + 2.0 * PI * r * 0.25) < 1e-13, "{m:?}");
    }

    #[test]
    fn the_slab_results_do_not_depend_on_the_loop_direction_or_the_reference_point() {
        let fwd = poly_loop(&[(0.0, 0.0), (5.0, 0.0), (5.0, 3.0), (0.0, 3.0)], false);
        let rev = CapLoop { curves: fwd.curves.iter().rev().map(|c| c.reversed()).collect(), is_hole: false };
        let (b, t) = (CapPlane::flat(0.0), CapPlane { c0: 0.5, g: Vec2::new(0.1, 0.2) });
        let a = extruded_loops_metrics_ref(&[fwd], b, t, &tols(), ReferencePoint::UvEnvelope).unwrap();
        let c = extruded_loops_metrics_ref(&[rev], b, t, &tols(), ReferencePoint::WorldBox).unwrap();
        assert_eq!(a, c);
        assert_eq!(extruded_loops_metrics(&[], b, t, &tols()), Err(StokesError::DegenerateFace));
    }

    /// The cut shapes agree with a direct numerical integration of the plan region of the cut band.
    #[test]
    fn a_cut_band_matches_the_closed_forms_of_each_shape() {
        let c = Vec2::new(0.0, 0.0);
        let (ri, ro, h) = (4.9, 5.1, 3.0);
        let band = wall_band(c, 0.0, PI, ri, ro, h);
        let (to, ti) = ((1.2, 1.5), (1.21, 1.52));
        let footprint = sector_between(ri, ro, to, ti).unwrap();
        let (v0, s0) = (arc_band_volume(&band, &tols()).unwrap(), arc_band_surface_area(&band, &tols()).unwrap());
        let jambs = (at(c, ro, to.0) - at(c, ri, ti.0)).length() + (at(c, ro, to.1) - at(c, ri, ti.1)).length();
        let strips = ro * (to.1 - to.0) + ri * (ti.1 - ti.0);
        // hole
        let hole = BandCut { outer: to, inner: ti, z_lo: 0.9, z_hi: 2.1 };
        let (v, s) = arc_band_cut_metrics(&band, &hole, &tols()).unwrap();
        assert!(rel(v, v0 - footprint * 1.2) < 1e-13);
        assert!(rel(s, s0 + (jambs - strips) * 1.2 + 2.0 * footprint) < 1e-13);
        // notch
        let notch = BandCut { z_lo: -0.5, z_hi: 2.1, ..hole };
        let (v, s) = arc_band_cut_metrics(&band, &notch, &tols()).unwrap();
        assert!(rel(v, v0 - footprint * 2.1) < 1e-13);
        assert!(rel(s, s0 + (jambs - strips) * 2.1) < 1e-13);
        let slot = BandCut { z_lo: -0.5, z_hi: 4.0, ..hole };
        let (v, s) = arc_band_cut_metrics(&band, &slot, &tols()).unwrap();
        assert!(rel(v, v0 - footprint * h) < 1e-13);
        assert!(rel(s, s0 + (jambs - strips) * h - 2.0 * footprint) < 1e-13);
    }

    #[test]
    fn a_cut_that_is_neither_a_notch_a_hole_nor_a_slot_is_refused() {
        let band = wall_band(Vec2::new(0.0, 0.0), 0.0, PI, 4.9, 5.1, 3.0);
        // starts above the base and reaches the top: a bite out of the top edge
        let bite = BandCut { outer: (1.2, 1.5), inner: (1.2, 1.5), z_lo: 1.0, z_hi: 3.5 };
        assert_eq!(arc_band_cut_metrics(&band, &bite, &tols()), Err(StokesError::DegenerateFace));
        let zero_inner = ArcBand { inner_radius: 0.0, ..band };
        let hole = BandCut { z_lo: 1.0, z_hi: 2.0, ..bite };
        assert_eq!(arc_band_cut_metrics(&zero_inner, &hole, &tols()), Err(StokesError::DegenerateFace));
    }

    #[test]
    fn the_clip_cap_helper_returns_the_closure_result_unchanged() {
        let band = wall_band(Vec2::new(0.0, 0.0), 0.3, 1.0, 2.9, 3.1, 3.0);
        let plain = arc_band_volume(&band, &tols()).unwrap();
        let inside = with_clip_cap_world_frame(|| with_clip_cap_world_frame(|| arc_band_volume(&band, &tols()).unwrap()));
        assert_eq!(plain, inside);
    }

    #[test]
    fn the_sample_points_of_an_arc_include_both_ends_and_lie_on_the_circle() {
        let arc = CircArc::new(Vec2::new(1.0, 2.0), 3.0, 0.4, -2.0).unwrap();
        let pts = arc_samples_m(&arc, &tols()).unwrap();
        assert!(pts.len() >= 3);
        assert!(pts[0].distance_to(arc.start()) < 1e-12 && pts[pts.len() - 1].distance_to(arc.end()) < 1e-12);
        assert!(pts.iter().all(|p| (p.distance_to(arc.centre) - 3.0).abs() < 1e-12));
    }

    #[test]
    fn the_box_of_the_samples_is_within_the_sag_of_the_analytic_box() {
        let arc = CircArc::new(Vec2::new(0.0, 0.0), 37.5, -1.0, 2.5).unwrap();
        let (lo, hi) = sampled_curve_box(&Curve::Arc(arc), &tols()).unwrap();
        let exact = arc.aabb();
        assert!(hi.y <= exact.max.y + 1e-12 && lo.x >= exact.min.x - 1e-12);
        assert!(exact.max.y - hi.y <= tols().sag * (1.0 + 1e-9), "{}", exact.max.y - hi.y);
        let (ln, hn) = sampled_curve_box(&Curve::line(Vec2::new(0.0, 1.0), Vec2::new(4.0, -2.0)), &tols()).unwrap();
        assert_eq!((ln, hn), (Vec2::new(0.0, -2.0), Vec2::new(4.0, 1.0)));
    }

    #[test]
    fn a_cylinder_form_with_the_apex_on_the_axis_has_a_constant_density() {
        let f = OneForm::Cylinder { radius: 2.0, c: [0.0, 0.0] };
        assert!((f.density(0.3) - 4.0 / 3.0).abs() < 1e-15 && (f.density(2.0) - 4.0 / 3.0).abs() < 1e-15);
        let off = OneForm::Cylinder { radius: 2.0, c: [0.5, 0.0] };
        assert!(off.density(0.0) < off.density(PI));
    }

    #[test]
    fn the_cone_volume_of_a_wedge_of_a_cylinder_matches_the_sector_prism() {
        // a closed solid made of the annular sector faces, built as sampled faces, against the band function
        let (ri, ro, h, theta) = (1.0, 1.5, 2.0, FRAC_PI_2);
        let n = 64usize;
        let ang = |i: usize| theta * i as f64 / n as f64;
        let arc_pts = |r: f64| -> Vec<[f64; 2]> { (0..=n).map(|i| [r * ang(i).cos(), r * ang(i).sin()]).collect() };
        let apex = [0.2, 0.3, 0.7];
        // planar top (u = x, v = y, normal +z) and bottom (normal -z, v mirrored)
        let top_loop: Vec<Vec<[f64; 2]>> = vec![arc_pts(ro), vec![[0.0, ro], [0.0, ri]], arc_pts(ri).into_iter().rev().collect(), vec![[ri, 0.0], [ro, 0.0]]];
        let mirror = |l: &Vec<Vec<[f64; 2]>>| -> Vec<Vec<[f64; 2]>> { l.iter().map(|e| e.iter().map(|p| [p[0], -p[1]]).collect()).collect() };
        let top = Face { form: OneForm::Plane { d: apex[2] - h }, outward: 1.0, loops: vec![UvLoop { edges: top_loop.clone(), is_hole: false }] };
        let bottom = Face { form: OneForm::Plane { d: -apex[2] }, outward: 1.0, loops: vec![UvLoop { edges: mirror(&top_loop), is_hole: false }] };
        let wall = |r: f64, outward: f64| Face {
            form: OneForm::Cylinder { radius: r, c: [apex[0], apex[1]] },
            outward,
            loops: vec![UvLoop {
                edges: vec![
                    (0..=n).map(|i| [ang(i), 0.0]).collect(),
                    vec![[theta, 0.0], [theta, h]],
                    (0..=n).rev().map(|i| [ang(i), h]).collect(),
                    vec![[0.0, h], [0.0, 0.0]],
                ],
                is_hole: false,
            }],
        };
        // radial end caps: plane through the axis; u = radial distance, v = z, normal = r_hat x z
        let cap = |a: f64, outward: f64| {
            let n_uv = [a.sin(), -a.cos(), 0.0];
            let dist = (apex[0] - ri * a.cos()) * n_uv[0] + (apex[1] - ri * a.sin()) * n_uv[1] + apex[2] * n_uv[2];
            Face { form: OneForm::Plane { d: dist }, outward, loops: vec![quad_at(ri, 0.0, ro, h)] }
        };
        let v = cone_volume(&[top, bottom, wall(ro, 1.0), wall(ri, -1.0), cap(0.0, 1.0), cap(theta, -1.0)]).unwrap();
        let exact = 0.5 * theta * (ro * ro - ri * ri) * h;
        assert!(rel(v, exact) < 1e-6, "{v} vs {exact}");
    }
}
