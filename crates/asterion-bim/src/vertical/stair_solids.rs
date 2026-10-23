//! The solids of a stairs: the treads and risers of its straight runs, the tread and supports of its landings and its stringers.
//!
//! Every rule below is one of the specification rules of the stair-body reading, written in the code that follows it:
//!
//! * Spec STAIR-RISER-PROFILE-01: a riser is the parallelogram `P0 = (a, d)`, `P1 = (a + c, d)`, `P2 = (b + c, e)`, `P3 =
//!   (b, e)` in the plane of the tread's front loop (x along travel from the riser line, y up from the tread's top), with
//!   the slope term `s = (nosing length - nosing profile width) / R`, `c = rt sqrt(1 + s^2)`, `d = -t`, `e = -(R +
//!   t_prev)`, `a = s t`, `b = n + s t_prev`.
//! * Tread `i` has its top at `H(i) = (i + b) R + z0`, is `t` thick, spans the plan between riser line `i` and riser line
//!   `i + 1` extended at the REAR by the nosing length `n` (not for the last tread of a run that ends with a tread), and
//!   its front and rear faces lean by `s`; tread and riser are SEPARATE bodies.
//! * Riser `i` hangs under tread `i` (`H(i) - (R + t_prev)` to `H(i) - t`, `t_prev` the thickness of tread `i - 1`, 0 for
//!   the first); the rear riser of a run that ends with a riser is the profile of the last tread moved by `(-n, +R)`; a run
//!   that begins and ends with a riser has `N` treads and `N + 1` risers: `2N + 1` bodies of 6 faces.
//! * Spec STAIR-LANDING-01: a landing has one tread, its boundary loop extruded by the landing's thickness, its top at its
//!   base elevation.
//! * A stringer is the rectangle `width x depth` (depth perpendicular to the slope) along the boundary line through the
//!   points `(s_i, H(i))` of the riser lines, moved up by the cell's height shift, cut by vertical planes at the first and
//!   last riser line and by the horizontal plane of the stairs' bottom; a landing has four supports, mitre joined, each a
//!   vertical band of the total depth.
//!
//! * Whether tread and riser fuse (the run type's two joint fields are not modelled): they stay separate, which is the
//!   row's default.
//! * A stairs whose type has no run type (every stairs made through the low-level C ABI, every imported one) gets the
//!   template's assembled run type and support type.
//! * The plane that cuts the stringer's underside is the stairs' bottom elevation (the row finds it at z = 0 in every
//!   corpus stairs and has not specified the plane).
//! * : winder, sketched and spiral runs, monolithic runs, runs that begin or end with a tread joined to a landing, open
//!   stringers, carriages and middle supports have no solid (their rows are unspecified), and are named as gaps by
//!   [`crate::element_geometry`].
//! * A landing's outline is its boundary loop; the offsets of the neighbouring runs (zero when they begin or end with a
//!   riser, the case of every corpus landing) are not applied.
//!
//! Nothing is stored: every function is a pure function of the model.

use crate::model::BimModel;
use crate::polysolid::{Part, PartRole, PolySolid, ProfileFrame};
use crate::stairs::{Stairs, StairsLanding, StairsRun};
use crate::stairs_scene::{Justification, RunStyle};
use asterion_core::ElementId;
use asterion_geom::Vec2;

/// The dimensions of a stairs support type (the type fields, metres).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SupportDims {
    /// Lateral width.
    pub width: f64,
    /// Total depth below the path.
    pub total_depth: f64,
    /// Structural depth on a run.
    pub structural_depth_run: f64,
    /// Structural depth on a landing.
    pub structural_depth_landing: f64,
}

/// The template's stringer, "Stringer - 50mm Width" (the values of its type fields in the lab harness dump of rounds 24 and
/// 26, converted from feet: 0.16404 / 1.31234 / 0.49869 / 0.99280). The engine has no support types.
pub const TEMPLATE_SUPPORT: SupportDims =
    SupportDims { width: 0.05, total_depth: 0.4, structural_depth_run: 0.152, structural_depth_landing: 0.302_606_273_040_956_66 };

/// The thicknesses a run is built with, metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunParams {
    pub tread_thickness: f64,
    pub riser_thickness: f64,
    pub nosing_length: f64,
    pub has_treads: bool,
    pub has_risers: bool,
    pub riser_slanted: bool,
    pub monolithic: bool,
}

/// The run type of a stairs: its own run type when the stairs type has one, else the template's assembled run type, with
/// the stairs type's tread thickness when it states one.
pub fn run_params(m: &BimModel, stairs: &Stairs) -> RunParams {
    if let Some(r) = m.stairs_scene.types.get(&stairs.type_id).and_then(|e| m.stairs_scene.run_types.get(&e.run_type)) {
        return RunParams {
            tread_thickness: r.tread_thickness,
            riser_thickness: r.riser_thickness,
            nosing_length: r.nosing_length,
            has_treads: r.has_treads,
            has_risers: r.has_risers,
            riser_slanted: r.riser_slanted,
            monolithic: r.is_monolithic,
        };
    }
    let (tread, riser, nosing) = crate::stairs_scene::template_run_dims();
    let tread = m.stairs.types.get(&stairs.type_id).and_then(|t| t.params.tread_thickness).filter(|t| *t > 0.0).unwrap_or(tread);
    RunParams { tread_thickness: tread, riser_thickness: riser, nosing_length: nosing, has_treads: true, has_risers: true, riser_slanted: true, monolithic: false }
}

/// Whether the stairs' type carries a stringer on a side (`left`, `right`); a stairs without a scene type has none.
fn has_support(m: &BimModel, stairs: &Stairs, left: bool) -> bool {
    m.stairs_scene.types.get(&stairs.type_id).map_or(false, |e| if left { e.left_support.is_some() } else { e.right_support.is_some() })
}

/// The lateral extent of a run's body from its location line, `(u0, u1)` with `u` positive to the left of travel: the line
/// sits on the left boundary for a left-justified run, in the middle for a centred one and on the right boundary for a
/// right-justified one (the location path is the base path offset by -W/2 for Left, 0 for Center, +W/2 for Right, positive
/// to the right of travel).
pub fn lateral_extent(width: f64, j: Justification) -> (f64, f64) {
    match j {
        Justification::Center => (-width / 2.0, width / 2.0),
        Justification::Left => (-width, 0.0),
        Justification::Right => (0.0, width),
    }
}

/// One body of a run in the run's plane: `(s, z)` with `s` along the run from the first riser line.
#[derive(Clone, Debug, PartialEq)]
pub struct StepBody {
    pub riser: bool,
    pub profile: [[f64; 2]; 4],
}

/// The tread and riser bodies of a run, in its plane (Spec STAIR-RISER-PROFILE-01; see the module doc). `r` is the riser
/// height `R`, `t_depth` the tread depth `T`, `base_z` the run's bottom elevation `z0`.
pub fn run_step_bodies(p: &RunParams, r: f64, t_depth: f64, base_z: f64, treads: usize, begin_with_riser: bool, end_with_riser: bool) -> Vec<StepBody> {
    let b = usize::from(begin_with_riser);
    let t = if p.has_treads { p.tread_thickness } else { 0.0 };
    let n = p.nosing_length.max(0.0);
    // the slope term of a slanted riser (nosing profile width 0: the template has no nosing profile); 0 for a straight riser
    let s = if p.riser_slanted { n / r } else { 0.0 };
    let c = p.riser_thickness * (1.0 + s * s).sqrt();
    let h = |i: usize| (i + b) as f64 * r + base_z;
    let mut out = Vec::new();
    if p.has_treads && t > 1e-9 {
        for i in 0..treads {
            let sf = i as f64 * t_depth;
            // the rear offset is the nosing length, except for the last tread of a run that ends with a tread
            let depth = t_depth + if i + 1 == treads && !end_with_riser { 0.0 } else { n };
            let top = h(i);
            out.push(StepBody { riser: false, profile: [[sf, top], [sf + depth, top], [sf + depth + s * t, top - t], [sf + s * t, top - t]] });
        }
    }
    // built only with risers on, a riser thickness above 1e-9 and a tread thinner than the riser height by 1e-9
    if p.has_risers && p.riser_thickness > 1e-9 && t <= r - 1e-9 {
        for j in 0..=treads {
            let (top_h, t_prev) = if j < treads {
                if !(b == 1 || j >= 1) {
                    continue;
                }
                // the first tread's riser reaches the run's base: no tread below it (`min(t, -Extend Below Riser Base)` = 0 by default)
                (h(j), if j == 0 { 0.0 } else { t })
            } else {
                if !end_with_riser || treads == 0 {
                    continue;
                }
                // the rear riser: the last tread's profile moved by (-n, +R)
                (h(treads - 1) + r, t)
            };
            let x0 = j as f64 * t_depth;
            let (a, bb) = if p.riser_slanted { (s * t, n + s * t_prev) } else { (0.0, 0.0) };
            let (d, e) = (-t, -(r + t_prev));
            out.push(StepBody { riser: true, profile: [[x0 + a, top_h + d], [x0 + a + c, top_h + d], [x0 + bb + c, top_h + e], [x0 + bb, top_h + e]] });
        }
    }
    out
}

/// The number of treads of a run from its riser count (Spec S-01f: the risers, less one when it ends with a riser, plus one
/// when it does not begin with one).
pub fn tread_count(risers: i32, begin_with_riser: bool, end_with_riser: bool) -> usize {
    (risers - i32::from(end_with_riser) + i32::from(!begin_with_riser)).max(0) as usize
}

fn gap(why: impl Into<String>) -> String {
    why.into()
}

/// What a run is laid out as: the stairs, its type's thicknesses, the number of treads, the distance between two riser lines and the lateral extent.
struct RunLayout<'a> {
    run: &'a StairsRun,
    stairs: &'a Stairs,
    params: RunParams,
    treads: usize,
    /// The distance between consecutive riser lines `T`: the stairs' tread depth for a straight run, the sketch's for a sketched one.
    t_depth: f64,
    begin_with_riser: bool,
    end_with_riser: bool,
    dir: Vec2,
    extent: (f64, f64),
}

fn run_layout<'a>(m: &'a BimModel, run_id: ElementId) -> Result<RunLayout<'a>, String> {
    let run = m.stairs.runs.get(&run_id).ok_or_else(|| gap("no such run"))?;
    let stairs = m.stairs.stairs.get(&run.stairs).ok_or_else(|| gap("the run's stairs is missing"))?;
    let ext = m.stairs_scene.runs.get(&run_id);
    let style = ext.map_or(RunStyle::Straight, |e| e.style);
    let params = run_params(m, stairs);
    if params.monolithic || !params.has_treads && !params.has_risers {
        return Err(gap("a monolithic run has no solid: its structural body is not specified (OQ-STAIR-MONOLITHIC-SOLID)"));
    }
    if !(stairs.actual_riser_height > 1e-9 && stairs.actual_tread_depth > 1e-9) {
        return Err(gap("the stairs has no positive riser height and tread depth"));
    }
    let dir = (run.end - run.start).normalized().ok_or_else(|| gap("the run's location line has no direction"))?;
    let (treads, t_depth, begin_with_riser, end_with_riser, extent) = match style {
        RunStyle::Straight => {
            let state = crate::stairs::stairs_state(m, run.stairs).map_err(|f| gap(f.message))?;
            let rs = state.runs.iter().find(|r| r.id == run_id).ok_or_else(|| gap("the run has no derived state"))?;
            let treads = tread_count(rs.risers, run.begin_with_riser, run.end_with_riser);
            let extent = lateral_extent(run.width, ext.map_or(Justification::Center, |e| e.justification));
            (treads, stairs.actual_tread_depth, run.begin_with_riser, run.end_with_riser, extent)
        }
        // A sketched run with straight, parallel boundaries follows the straight-run rules tread by tread, with T the
        // distance between consecutive riser lines. The engine stores the number of riser lines and the path, not the
        // lines: they are taken evenly spaced along the path from its first point to its last (ENGINE CHOICE), the first
        // and the last being riser lines
        RunStyle::Sketched => {
            let (e, n) = (ext.ok_or_else(|| gap("a sketched run without its sketch"))?, ext.and_then(|e| e.fixed_risers).unwrap_or(0));
            if n < 2 || e.boundary.len() != 2 {
                return Err(gap("a sketched run needs two boundary lines and at least two riser lines (OQ-STAIR-RUN-STYLES)"));
            }
            let across = dir.perp_ccw();
            let mut us = Vec::new();
            for (a, b) in &e.boundary {
                let along = (*b - *a).normalized().ok_or_else(|| gap("a sketched run boundary has no length"))?;
                if along.cross(dir).abs() > 1e-6 {
                    return Err(gap("a sketched run whose boundaries are not parallel to its path has no solid (OQ-STAIR-RUN-STYLES)"));
                }
                us.push((*a - run.start).dot(across));
            }
            let extent = (us[0].min(us[1]), us[0].max(us[1]));
            if !(extent.1 - extent.0 > 1e-9) {
                return Err(gap("a sketched run's boundaries coincide"));
            }
            let length = run.start.distance_to(run.end);
            ((n as usize) - 1, length / (n as f64 - 1.0), true, true, extent)
        }
        RunStyle::Spiral => {
            return Err(gap("a spiral run has no solid: its tread outline and its helix stringers are not exact in the reading"));
        }
    };
    if treads == 0 {
        return Err(gap("the run has no tread"));
    }
    if treads > 2000 {
        return Err(gap("the run has more than 2000 treads: no solid is built for it"));
    }
    if !(t_depth > 1e-9) {
        return Err(gap("the run has no tread depth"));
    }
    Ok(RunLayout { run, stairs, params, treads, t_depth, begin_with_riser, end_with_riser, dir, extent })
}

/// The solids of one straight run: its treads and risers, separate bodies. `Err` names why the run has none.
pub fn run_geometry(m: &BimModel, run_id: ElementId) -> Result<Vec<Part>, String> {
    let l = run_layout(m, run_id)?;
    let bodies = run_step_bodies(&l.params, l.stairs.actual_riser_height, l.t_depth, l.run.bottom_elevation, l.treads, l.begin_with_riser, l.end_with_riser);
    let frame = ProfileFrame { origin: l.run.start, dir: l.dir };
    let parts: Vec<Part> = bodies
        .iter()
        .filter_map(|b| PolySolid::from_profile(&frame, &b.profile, l.extent.0, l.extent.1))
        .map(|solid| Part::new(PartRole::Step, solid))
        .collect();
    if parts.is_empty() {
        return Err(gap("the run's treads and risers collapse to no volume"));
    }
    Ok(parts)
}

/// The thickness of a landing's tread (Spec STAIR-LANDING-01: the landing type's value, or the run type's when the landing
/// type follows it).
fn landing_thickness(m: &BimModel, stairs: &Stairs) -> f64 {
    match m.stairs_scene.types.get(&stairs.type_id) {
        Some(e) if e.landing_monolithic => e.landing_thickness,
        // a non-monolithic landing is as thick as the run type's tread (Spec STAIR-LANDING-01; OBSERVED)
        _ => run_params(m, stairs).tread_thickness,
    }
}

/// The solid of a landing: the prism of its boundary loop below the landing's base elevation (Spec STAIR-LANDING-01).
pub fn landing_geometry(m: &BimModel, id: ElementId) -> Result<Vec<Part>, String> {
    let l: &StairsLanding = m.stairs.landings.get(&id).ok_or_else(|| gap("no such landing"))?;
    let stairs = m.stairs.stairs.get(&l.stairs).ok_or_else(|| gap("the landing's stairs is missing"))?;
    let boundary = m.stairs_scene.landings.get(&id).map(|e| e.boundary.as_slice()).unwrap_or(&[]);
    if boundary.len() < 3 {
        return Err(gap("the landing has no boundary sketch loop (a landing made without one has no outline: OQ-STAIR-LANDING-BOUNDARY)"));
    }
    let th = landing_thickness(m, stairs);
    if !(th > 1e-9) {
        return Err(gap("the landing type has no thickness"));
    }
    let solid = PolySolid::from_prism(boundary, l.base_elevation - th, l.base_elevation).ok_or_else(|| gap("the landing's boundary has no area"))?;
    Ok(vec![Part::new(PartRole::Landing, solid)])
}

/// The height by which the support path of a straight run is moved up, `((D - (sin(theta) ro + Ds)) - cos(theta) (R + t)) /
/// cos(theta)` with `theta = atan(R / T)`, `ro = s t + n + sqrt(1 + s^2) rt` the rear offset of tread 0 and `s = n / R`.
pub fn run_support_shift(d: &SupportDims, p: &RunParams, r: f64, t_depth: f64) -> f64 {
    let t = if p.has_treads { p.tread_thickness } else { 0.0 };
    let theta = (r / t_depth).atan();
    let s = if p.riser_slanted { p.nosing_length / r } else { 0.0 };
    let ro = s * t + p.nosing_length + (1.0 + s * s).sqrt() * p.riser_thickness;
    ((d.total_depth - (theta.sin() * ro + d.structural_depth_run)) - theta.cos() * (r + t)) / theta.cos()
}

/// The profile `(s, z)` of a stringer along a run: the band `depth` below the shifted path (measured square to the slope),
/// cut by the vertical planes at `s = 0` and `s = length` and by the plane `z >= floor`.
pub fn support_profile(top0: f64, slope: f64, length: f64, depth: f64, floor: f64) -> Vec<[f64; 2]> {
    let vertical = depth * (1.0 + slope * slope).sqrt();
    let top = |s: f64| top0 + slope * s;
    let bottom = |s: f64| top(s) - vertical;
    let quad = [[0.0, bottom(0.0)], [length, bottom(length)], [length, top(length)], [0.0, top0]];
    let mut out: Vec<[f64; 2]> = Vec::new();
    for k in 0..4 {
        let (a, c) = (quad[k], quad[(k + 1) % 4]);
        let (ina, inc) = (a[1] >= floor, c[1] >= floor);
        if ina {
            out.push(a);
        }
        if ina != inc {
            let t = (floor - a[1]) / (c[1] - a[1]);
            out.push([a[0] + (c[0] - a[0]) * t, floor]);
        }
    }
    out
}

/// The stringers of a stairs: one on each side of each straight run whose stairs type has a stringer on that side, and four mitre-joined
/// supports round each landing.
pub fn support_geometry(m: &BimModel, stairs_id: ElementId) -> Vec<Part> {
    let Some(stairs) = m.stairs.stairs.get(&stairs_id) else { return Vec::new() };
    if !(stairs.actual_riser_height > 1e-9 && stairs.actual_tread_depth > 1e-9) {
        return Vec::new();
    }
    let d = TEMPLATE_SUPPORT;
    let floor = crate::stairs::stairs_state(m, stairs_id).map(|s| s.bottom_elevation).unwrap_or(f64::NEG_INFINITY);
    let mut out = Vec::new();
    for run in m.stairs.runs_of(stairs_id) {
        let Ok(l) = run_layout(m, run.id) else { continue };
        let (r, t_depth) = (stairs.actual_riser_height, l.t_depth);
        // the path: through the points (s_i, H(i)) of the riser lines (slope R / T, through the first and the last point), moved up
        let top0 = run.bottom_elevation + f64::from(u8::from(l.begin_with_riser)) * r + run_support_shift(&d, &l.params, r, t_depth);
        let profile = support_profile(top0, r / t_depth, l.treads as f64 * t_depth, d.total_depth, floor);
        let frame = ProfileFrame { origin: run.start, dir: l.dir };
        let (u0, u1) = l.extent;
        for (left, (a, c)) in [(true, (u1, u1 + d.width)), (false, (u0 - d.width, u0))] {
            if !has_support(m, stairs, left) {
                continue;
            }
            if let Some(solid) = PolySolid::from_profile(&frame, &profile, a, c) {
                out.push(Part::new(PartRole::Support, solid));
            }
        }
    }
    for l in m.stairs.landings.values().filter(|l| l.stairs == stairs_id) {
        out.extend(landing_support_geometry(m, stairs, l));
    }
    out
}

/// The supports round a landing: one per boundary curve, outside it, mitre joined at the corners, each a vertical band as
/// deep as the support type, its top `(D - Dl) - t` above the landing's top.
fn landing_support_geometry(m: &BimModel, stairs: &Stairs, l: &StairsLanding) -> Vec<Part> {
    let boundary = m.stairs_scene.landings.get(&l.id).map(|e| e.boundary.as_slice()).unwrap_or(&[]);
    if boundary.len() < 3 || !(has_support(m, stairs, true) || has_support(m, stairs, false)) {
        return Vec::new();
    }
    let d = TEMPLATE_SUPPORT;
    let mut v: Vec<Vec2> = boundary.to_vec();
    let area: f64 = (0..v.len()).map(|i| v[i].cross(v[(i + 1) % v.len()])).sum::<f64>() * 0.5;
    if area < 0.0 {
        v.reverse();
    }
    let n = v.len();
    // the outer loop: each vertex pushed out along the bisector of its two sides (a mitre)
    let side = |i: usize| (v[(i + 1) % n] - v[i]).normalized().map(|dir| Vec2::new(dir.y, -dir.x));
    let mut outer: Vec<Vec2> = Vec::with_capacity(n);
    for i in 0..n {
        let (Some(n0), Some(n1)) = (side((i + n - 1) % n), side(i)) else { return Vec::new() };
        let k = 1.0 + n0.dot(n1);
        if k < 1e-6 {
            return Vec::new();
        }
        outer.push(v[i] + (n0 + n1) * (d.width / k));
    }
    let top = l.base_elevation + (d.total_depth - d.structural_depth_landing) - landing_thickness(m, stairs);
    let mut out = Vec::new();
    for i in 0..n {
        let j = (i + 1) % n;
        if let Some(solid) = PolySolid::from_prism(&[v[i], v[j], outer[j], outer[i]], top - d.total_depth, top) {
            out.push(Part::new(PartRole::Support, solid));
        }
    }
    out
}

/// Every solid of a stairs and the element that owns it: its runs' steps, its landings' treads and (under the stairs' own id) its stringers.
pub fn stairs_parts(m: &BimModel, stairs_id: ElementId) -> Vec<(ElementId, Vec<Part>)> {
    let mut out = Vec::new();
    for r in m.stairs.runs_of(stairs_id) {
        if let Ok(p) = run_geometry(m, r.id) {
            out.push((r.id, p));
        }
    }
    for l in m.stairs.landings.values().filter(|l| l.stairs == stairs_id) {
        if let Ok(p) = landing_geometry(m, l.id) {
            out.push((l.id, p));
        }
    }
    let supports = support_geometry(m, stairs_id);
    if !supports.is_empty() {
        out.push((stairs_id, supports));
    }
    out
}

// ------------------------------------------------------------------------------------------------ railings

/// A straight piece of a railing's sweep path in 3D: plan ends and the elevation of the path at each. `tread` is
/// `Some((tread depth, riser height))` on a path that follows a stairs: a baluster then stands on the top of the tread
/// under it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathSeg {
    pub a: Vec2,
    pub b: Vec2,
    pub za: f64,
    pub zb: f64,
    pub tread: Option<(f64, f64)>,
}

impl PathSeg {
    fn length(&self) -> f64 {
        self.a.distance_to(self.b)
    }
    /// Vertical rise over plan length.
    fn grade(&self) -> f64 {
        (self.zb - self.za) / self.length().max(1e-12)
    }
}

/// The section of a rail, a top rail or a baluster.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Section {
    /// A round tube or rod.
    Round { radius: f64 },
    /// A rectangle: across the path and tall.
    Rect { width: f64, height: f64 },
}

impl Section {
    /// Vertical extent of the section seen on a path of the given grade (a section square to the slope is taller by `sqrt(1 + g^2)` in a
    /// vertical plane).
    fn vertical(self, grade: f64) -> f64 {
        let k = (1.0 + grade * grade).sqrt();
        match self {
            Section::Round { radius } => 2.0 * radius * k,
            Section::Rect { height, .. } => height * k,
        }
    }
    fn lateral(self) -> f64 {
        match self {
            Section::Round { radius } => 2.0 * radius,
            Section::Rect { width, .. } => width,
        }
    }
    /// The ring `(across, up)` of the section about the centre height `c`, clockwise seen along the path with `across` to the left (the
    /// order [`sweep_ring`] needs): a rectangle, or a 16-gon whose vertices lie on the section (so its box is the section's).
    fn ring(self, c: f64, grade: f64) -> Vec<[f64; 2]> {
        let k = (1.0 + grade * grade).sqrt();
        match self {
            Section::Round { radius } => (0..16)
                .map(|i| {
                    let t = -(i as f64) * std::f64::consts::TAU / 16.0;
                    [radius * t.cos(), c + radius * k * t.sin()]
                })
                .collect(),
            Section::Rect { width, height } => {
                let (w, h) = (width / 2.0, height * k / 2.0);
                vec![[w, c - h], [-w, c - h], [-w, c + h], [w, c + h]]
            }
        }
    }
    /// The area of the section square to the path.
    fn area(self) -> f64 {
        match self {
            Section::Round { radius } => std::f64::consts::PI * radius * radius,
            Section::Rect { width, height } => width * height,
        }
    }
}

/// The numbers of a type - rail heights, sections, the pattern's entries - are data of the family that the reading could
/// not reach, so they are ENGINE CHOICES. For the template type "900mm Pipe" they are the dimensions the reading found in
/// the corpus dumps: six round rails of 30 mm, a top rail of 40 mm whose top is 0.9 m over the path, balusters of 25 mm.
#[derive(Clone, Debug, PartialEq)]
pub struct RailingDef {
    /// Height of the top surface of the top rail above the path, measured vertically.
    pub top_height: f64,
    pub top_rail: Section,
    /// The rails below the top rail: the height of the centre above the path (measured vertically) and the section.
    pub rails: Vec<(f64, Section)>,
    /// The section of a baluster; `None` = no balusters.
    pub baluster: Option<Section>,
    /// Baluster pattern: the spaces of its entries (one baluster per entry).
    pub baluster_spaces: Vec<f64>,
    /// Justification code of the pattern (Spec RAIL-BALUSTER-01: 0 start, 1 end, 2 centre, 3 spread).
    pub justification: u8,
    /// A baluster at each end and each corner of the path (Spec RAIL-BALUSTER-01: the start and end corner balusters).
    pub corner_balusters: bool,
    /// A glass or solid panel along the path: thickness, bottom and top heights above the path.
    pub panel: Option<(f64, f64, f64)>,
}

/// The railing type of a name. See [`RailingDef`].
pub fn railing_def(name: &str) -> RailingDef {
    // "900mm ..." names the height of the railing (OBSERVED naming of the template types)
    let top_height = name
        .split_whitespace()
        .next()
        .and_then(|w| w.strip_suffix("mm"))
        .and_then(|n| n.parse::<f64>().ok())
        .map_or(0.9, |mm| mm / 1000.0);
    if name.contains("Pipe") {
        // OBSERVED and the corpus dumps: six round rails of 30 mm whose centres are 0.135 + 0.125 k above the path, a round
        // top rail of 40 mm, round balusters of 25 mm every 1 m
        return RailingDef {
            top_height,
            top_rail: Section::Round { radius: 0.02 },
            rails: (0..6).map(|k| (0.135 + 0.125 * k as f64, Section::Round { radius: 0.015 })).collect(),
            baluster: Some(Section::Round { radius: 0.0125 }),
            baluster_spaces: vec![1.0],
            justification: 3,
            corner_balusters: true,
            panel: None,
        };
    }
    if name.contains("Glass") {
        return RailingDef {
            top_height,
            top_rail: Section::Rect { width: 0.05, height: 0.04 },
            rails: Vec::new(),
            baluster: None,
            baluster_spaces: Vec::new(),
            justification: 3,
            corner_balusters: false,
            panel: Some((0.012, 0.1, top_height - 0.04)),
        };
    }
    RailingDef {
        top_height,
        top_rail: Section::Rect { width: 0.05, height: 0.04 },
        rails: Vec::new(),
        baluster: Some(Section::Rect { width: 0.02, height: 0.0 }),
        baluster_spaces: vec![0.12],
        justification: 3,
        corner_balusters: true,
        panel: None,
    }
}

/// Spec RAIL-BALUSTER-01: the stations (distances from the start of one path sequence of length `s`) of the balusters of a
/// pattern whose leading value is `lead` and whose entries have the spaces `spaces` (one baluster per entry), by
/// justification code.
///
/// The pattern length is `L = lead + sum(spaces)`; none when `L <= 1e-9`. The number of instances is `n = floor((s + 1e-9) / L)`,
/// except for code 3 (spread), where it is `round(s / L)` and the pattern is scaled to the pitch `s / n` (`n = 0` leaves the used length 0).
/// The start offset is 0 (code 0), `s - U` (code 1), `(s - U) / 2` or `(s - L) / 2` when `n = 0` (code 2), 0 (code 3), with `U = n L` (`U = s`
/// when spread). Instance `k` starts at `offset + k pitch`; the baluster of entry `i` stands at the running sum of the spaces of entries
/// `0..=i` inside it. (The balusters at the very start and end of a path belong to the corner balusters, which the caller adds.)
pub fn baluster_stations(s: f64, lead: f64, spaces: &[f64], justification: u8) -> Vec<f64> {
    let l = lead + spaces.iter().sum::<f64>();
    if !(l > 1e-9) || !(s > 0.0) {
        return Vec::new();
    }
    let spread = justification == 3;
    let (n, pitch, used, scale) = if spread {
        let n = (s / l + 0.5).floor();
        if n < 1.0 {
            return Vec::new();
        }
        (n as usize, s / n, s, s / n / l)
    } else {
        let n = ((s + 1e-9) / l).floor();
        (n as usize, l, n * l, 1.0)
    };
    let offset = match justification {
        0 => 0.0,
        1 => s - used,
        2 if n == 0 => (s - l) / 2.0,
        2 => (s - used) / 2.0,
        _ => 0.0,
    };
    let mut out = Vec::new();
    for k in 0..n {
        let mut run = 0.0;
        for sp in spaces {
            run += sp * scale;
            out.push(offset + k as f64 * pitch + run);
        }
    }
    out
}

/// The path segments of a stairs railing: the railing's sub-element keys name, in the engine's own convention, a run or
/// landing and a side (`2 x element id + side`; the shell states a run id alone, the side then being the railing's path
/// index parity); a railing with no key (imported) takes the components in order, two paths per run (left, right) then one
/// per landing.
pub fn stairs_railing_path(m: &BimModel, railing: ElementId) -> Result<Vec<PathSeg>, String> {
    let r = m.stairs.railings.get(&railing).ok_or_else(|| gap("no such railing"))?;
    let stairs = m.stairs.stairs.get(&r.stairs).ok_or_else(|| gap("the railing's stairs is missing"))?;
    if !(stairs.actual_riser_height > 1e-9 && stairs.actual_tread_depth > 1e-9) {
        return Err(gap("the stairs has no positive riser height and tread depth"));
    }
    let runs: Vec<&StairsRun> = m.stairs.runs_of(r.stairs);
    let landings: Vec<&StairsLanding> = m.stairs.landings.values().filter(|l| l.stairs == r.stairs).collect();
    // (component id, side or None for a landing)
    let mut targets: Vec<(ElementId, Option<bool>)> = Vec::new();
    let side_of_index = |i: usize| i % 2 == 0; // true = left
    if r.subelement_keys.is_empty() {
        let p = r.path_index;
        if p < 2 * runs.len() {
            targets.push((runs[p / 2].id, Some(side_of_index(p))));
        } else if p - 2 * runs.len() < landings.len() {
            targets.push((landings[p - 2 * runs.len()].id, None));
        }
    } else {
        for k in &r.subelement_keys {
            let id2 = ElementId(k.0 / 2);
            if runs.iter().any(|x| x.id == id2) {
                targets.push((id2, Some(k.0 % 2 == 0)));
            } else if landings.iter().any(|x| x.id == id2) {
                targets.push((id2, None));
            } else if runs.iter().any(|x| x.id.0 == k.0) {
                targets.push((ElementId(k.0), Some(side_of_index(r.path_index))));
            } else if landings.iter().any(|x| x.id.0 == k.0) {
                targets.push((ElementId(k.0), None));
            }
        }
    }
    if targets.is_empty() {
        return Err(gap("the railing's path keys name no run or landing of its stairs (OQ-RAILING-PATH-KEYS)"));
    }
    let mut segs: Vec<PathSeg> = Vec::new();
    for (id, side) in targets {
        match side {
            Some(left) => {
                let l = run_layout(m, id)?;
                let run = l.run;
                let (u0, u1) = l.extent;
                let off = l.dir.perp_ccw() * if left { u1 } else { u0 };
                // The path runs through the points (s_i, H(i)) of the riser lines: the nosing line, from the first to the
                // last riser line
                let (r_h, t_d) = (l.stairs.actual_riser_height, l.t_depth);
                let length = l.treads as f64 * t_d;
                let slope = r_h / t_d;
                let top0 = run.bottom_elevation + f64::from(u8::from(l.begin_with_riser)) * r_h;
                let end = run.start + l.dir * length;
                segs.push(PathSeg { a: run.start + off, b: end + off, za: top0, zb: top0 + slope * length, tread: Some((t_d, r_h)) });
            }
            None => {
                let l = landings.iter().find(|x| x.id == id).expect("listed above");
                let boundary = m.stairs_scene.landings.get(&id).map(|e| e.boundary.as_slice()).unwrap_or(&[]);
                if boundary.len() < 3 {
                    return Err(gap("the landing has no boundary loop to put a railing along"));
                }
                // the edges of the boundary that do not meet a run's end are open sides (ASSUMED)
                let ends: Vec<(Vec2, f64)> = runs.iter().flat_map(|x| [(x.start, x.width), (x.end, x.width)]).collect();
                let n = boundary.len();
                for i in 0..n {
                    let (a, b) = (boundary[i], boundary[(i + 1) % n]);
                    let mid = Vec2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
                    if !ends.iter().any(|(p, w)| p.distance_to(mid) < 0.5 * w + 0.05) {
                        segs.push(PathSeg { a, b, za: l.base_elevation, zb: l.base_elevation, tread: None });
                    }
                }
            }
        }
    }
    segs.retain(|s| s.length() > 1e-6);
    if segs.is_empty() {
        return Err(gap("the railing's path has no length"));
    }
    Ok(segs)
}

/// The solid of a section swept along a chain of path points `(plan point, elevation)`. `ring` lists `(across, up)` points
/// of the section about the path, clockwise seen along the path with `across` to its left; the section stays in vertical
/// planes (a corner is mitred: the chain must be flat there, since a corner of a sloped path has no single height on its
/// mitre - the caller splits such a chain).
pub fn sweep_ring(points: &[(Vec2, f64)], ring: &[[f64; 2]]) -> Option<PolySolid> {
    let n = points.len();
    let k = ring.len();
    if n < 2 || k < 3 {
        return None;
    }
    let dirs: Vec<Vec2> = (0..n - 1).map(|i| (points[i + 1].0 - points[i].0).normalized()).collect::<Option<_>>()?;
    let mut vertices: Vec<[f64; 3]> = Vec::with_capacity(k * n);
    for i in 0..n {
        // the across direction at this frame, scaled so that the offset lines of the two segments meet (a mitre)
        let across = if i == 0 {
            dirs[0].perp_ccw()
        } else if i == n - 1 {
            dirs[n - 2].perp_ccw()
        } else {
            let (n0, n1) = (dirs[i - 1].perp_ccw(), dirs[i].perp_ccw());
            let c = 1.0 + n0.dot(n1);
            if c < 1e-6 {
                return None;
            }
            (n0 + n1) * (1.0 / c)
        };
        let (p, z) = points[i];
        for q in ring {
            let at = p + across * q[0];
            vertices.push([at.x, at.y, z + q[1]]);
        }
    }
    let last = k * (n - 1);
    let mut faces: Vec<Vec<usize>> = vec![(0..k).collect(), (0..k).rev().map(|j| last + j).collect()];
    for i in 0..n - 1 {
        let (a, b) = (k * i, k * (i + 1));
        for j in 0..k {
            let j2 = (j + 1) % k;
            faces.push(vec![a + j, b + j, b + j2, a + j2]);
        }
    }
    Some(PolySolid::new(vertices, faces))
}

/// The consecutive segments of a path that one swept solid can follow: connected end to start and flat (a sloped segment is its own chain).
fn chains(path: &[PathSeg]) -> Vec<Vec<PathSeg>> {
    let mut out: Vec<Vec<PathSeg>> = Vec::new();
    for s in path {
        let flat = (s.za - s.zb).abs() < 1e-9;
        let joins = out.last().and_then(|c| c.last()).map_or(false, |l| {
            flat && (l.za - l.zb).abs() < 1e-9 && l.b.distance_to(s.a) < 1e-6 && (l.za - s.za).abs() < 1e-9
        });
        if joins {
            out.last_mut().expect("checked").push(*s);
        } else {
            out.push(vec![*s]);
        }
    }
    out
}

fn chain_points(c: &[PathSeg]) -> Vec<(Vec2, f64)> {
    let mut pts: Vec<(Vec2, f64)> = vec![(c[0].a, c[0].za)];
    pts.extend(c.iter().map(|s| (s.b, s.zb)));
    pts
}

/// A member of a railing swept along a chain: a rail at the vertical height `c` above the path, the top rail with its top
/// surface `top` above it.
fn swept_member(chain: &[PathSeg], section: Section, centre: f64, role: PartRole) -> Option<Part> {
    let grade = chain[0].grade();
    let solid = sweep_ring(&chain_points(chain), &section.ring(centre, grade))?;
    let part = Part::new(role, solid);
    Some(match section {
        Section::Round { radius } => {
            let centre_line: f64 = chain.iter().map(|s| (s.length().powi(2) + (s.zb - s.za).powi(2)).sqrt()).sum();
            let caps = std::f64::consts::PI * radius * radius * ((1.0 + chain[0].grade().powi(2)).sqrt() + (1.0 + chain[chain.len() - 1].grade().powi(2)).sqrt());
            part.with_exact(section.area() * centre_line, 2.0 * std::f64::consts::PI * radius * centre_line + caps)
        }
        Section::Rect { .. } => part,
    })
}

/// A baluster standing at station `s` of `seg`: from its base up to `top` (the height of the top of its axis above the
/// path). On a stairs it stands on the tread under it. A round baluster takes `pi r^2 h` and `2 pi r h + pi r^2 (1 + sqrt(1
/// + g^2))` (a flat base, a top cap parallel to the top rail).
fn baluster_piece(seg: &PathSeg, s: f64, section: Section, top: f64) -> Option<Part> {
    let dir = (seg.b - seg.a).normalized()?;
    let g = seg.grade();
    let path_z = seg.za + g * s;
    let base = match seg.tread {
        Some((t_depth, riser)) => seg.za + riser * (s / t_depth + 1e-9).floor(),
        None => path_z,
    };
    let axis_top = path_z + top;
    let h = axis_top - base;
    if !(h > 1e-9) {
        return None;
    }
    let (half, depth) = match section {
        Section::Round { radius } => (radius, radius),
        Section::Rect { width, .. } => (width / 2.0, width / 2.0),
    };
    let profile = [[s - half, base], [s + half, base], [s + half, axis_top], [s - half, axis_top]];
    let frame = ProfileFrame { origin: seg.a, dir };
    let solid = PolySolid::from_profile(&frame, &profile, -depth, depth)?;
    let part = Part::new(PartRole::Baluster, solid);
    Some(match section {
        Section::Round { radius } => {
            let pi = std::f64::consts::PI;
            part.with_exact(pi * radius * radius * h, 2.0 * pi * radius * h + pi * radius * radius * (1.0 + (1.0 + g * g).sqrt()))
        }
        Section::Rect { .. } => part,
    })
}

/// The solids of a railing of type `def` along `path` (segments in order).
pub fn railing_parts(def: &RailingDef, path: &[PathSeg]) -> Vec<Part> {
    let mut out = Vec::new();
    for c in chains(path) {
        let grade = c[0].grade();
        // the top rail: its top surface `top_height` above the path, measured vertically
        let top_centre = def.top_height - def.top_rail.vertical(grade) / 2.0;
        out.extend(swept_member(&c, def.top_rail, top_centre, PartRole::TopRail));
        for (centre, section) in &def.rails {
            out.extend(swept_member(&c, *section, *centre, PartRole::Rail));
        }
        if let Some((th, lo, hi)) = def.panel {
            let ring = Section::Rect { width: th, height: hi - lo }.ring((lo + hi) / 2.0, grade);
            out.extend(sweep_ring(&chain_points(&c), &ring).map(|solid| Part::new(PartRole::Panel, solid)));
        }
    }
    // Spec RAIL-BALUSTER-01: a baluster pattern per path sequence, broken at corners (ASSUMED: every corner breaks)
    if let Some(section) = def.baluster {
        let n = path.len();
        let size = section.lateral();
        let mut placed: Vec<Vec2> = Vec::new();
        let mut put = |seg: &PathSeg, st: f64, out: &mut Vec<Part>| {
            let dir = (seg.b - seg.a).normalized().unwrap_or(Vec2::new(1.0, 0.0));
            let at = seg.a + dir * st;
            if placed.iter().any(|p| p.distance_to(at) < size * 0.5) {
                return;
            }
            placed.push(at);
            // the top of the axis: the top rail's top surface less its vertical thickness, plus the tilt of the top cap
            let g = seg.grade();
            let radius = if let Section::Round { radius } = section { radius } else { 0.0 };
            let top = def.top_height - def.top_rail.vertical(g) + radius * g;
            out.extend(baluster_piece(seg, st, section, top));
        };
        for (i, seg) in path.iter().enumerate() {
            let len = seg.length();
            if def.corner_balusters {
                // the very first and last baluster stand inside the path (their edge on its end); a corner baluster is centred on the corner
                let (s0, s1) = (if i == 0 { size / 2.0 } else { 0.0 }, if i + 1 == n { len - size / 2.0 } else { len });
                put(seg, s0, &mut out);
                put(seg, s1, &mut out);
            }
            for st in baluster_stations(len, 0.0, &def.baluster_spaces, def.justification) {
                if st > size * 0.5 && st < len - size * 0.5 {
                    put(seg, st, &mut out);
                }
            }
        }
    }
    out
}

/// The solids of a railing: of a stairs or drawn on a path. `Err` names why it has none.
pub fn railing_geometry(m: &BimModel, id: ElementId) -> Result<Vec<Part>, String> {
    let (railing_type, path) = if let Some(r) = m.stairs.railings.get(&id) {
        (r.railing_type, stairs_railing_path(m, id)?)
    } else if let Some(p) = m.stairs_scene.path_railings.get(&id) {
        let z = m.levels.get(&p.level).map(|l| l.elevation).ok_or_else(|| gap("the railing's level is missing"))?;
        (p.railing_type, p.path.iter().map(|(a, b)| PathSeg { a: *a, b: *b, za: z, zb: z, tread: None }).collect())
    } else {
        return Err(gap("no such railing"));
    };
    let name = m.stairs_scene.railing_types.get(&railing_type).map_or("900mm Pipe", |t| t.name.as_str());
    let def = railing_def(name);
    let mut parts = railing_parts(&def, &path);
    // a stored railing carries the placement of each of its balusters: those, and no laid-out ones
    if let Some(list) = m.stairs_scene.baluster_instances.get(&id) {
        parts.retain(|p| p.role != PartRole::Baluster);
        let section = def.baluster.unwrap_or(Section::Rect { width: 0.02, height: 0.0 });
        let size = section.lateral();
        let height = def.top_height - def.top_rail.vertical(0.0);
        for b in list {
            let frame = ProfileFrame { origin: Vec2::new(b.position[0], b.position[1]), dir: Vec2::new(b.rotation.cos(), b.rotation.sin()) };
            let z = b.position[2];
            let profile = [[-size / 2.0, z], [size / 2.0, z], [size / 2.0, z + height], [-size / 2.0, z + height]];
            parts.extend(PolySolid::from_profile(&frame, &profile, -size / 2.0, size / 2.0).map(|solid| Part::new(PartRole::Baluster, solid)));
        }
    }
    if parts.is_empty() {
        return Err(gap("the railing has no solid"));
    }
    Ok(parts)
}

// ------------------------------------------------------------------------------------------------ closed-form numbers

/// The numbers of a set of solids, all closed-form (divergence theorem over the planar faces, the extremes of the vertices): what the lab
/// harness reads from an element's geometry (the count of solids, their summed volume and surface area, the box).
#[derive(Clone, Debug, PartialEq)]
pub struct SolidSummary {
    pub solid_count: usize,
    pub volume: f64,
    pub surface_area: f64,
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub faces: usize,
    pub edges: usize,
    pub vertices: usize,
}

/// Sums the solids of `parts` (separate solids: nothing is subtracted where two touch). A member with analytic values (a round tube) counts them.
/// `None` without a part.
pub fn summarize(parts: &[Part]) -> Option<SolidSummary> {
    if parts.is_empty() {
        return None;
    }
    let mut out = SolidSummary { solid_count: parts.len(), volume: 0.0, surface_area: 0.0, min: [f64::INFINITY; 3], max: [f64::NEG_INFINITY; 3], faces: 0, edges: 0, vertices: 0 };
    for p in parts {
        out.volume += p.volume();
        out.surface_area += p.area();
        out.faces += p.solid.face_count();
        out.edges += p.solid.edge_count();
        out.vertices += p.solid.vertex_count();
        let (lo, hi) = p.solid.aabb();
        for k in 0..3 {
            out.min[k] = out.min[k].min(lo[k]);
            out.max[k] = out.max[k].max(hi[k]);
        }
    }
    Some(out)
}

/// The numbers of a stairs' solids: its runs, landings and stringers (not its railings, which are elements of their own).
/// The harness reports the `2N + 1` bodies of a run as ONE solid (one multi-lump solid), so a run's steps count once.
pub fn stairs_summary(m: &BimModel, stairs_id: ElementId) -> Option<SolidSummary> {
    let groups = stairs_parts(m, stairs_id);
    let parts: Vec<Part> = groups.iter().flat_map(|(_, p)| p.iter().cloned()).collect();
    let mut x = summarize(&parts)?;
    x.solid_count = groups
        .iter()
        .map(|(_, ps)| ps.iter().filter(|p| p.role != PartRole::Step).count() + usize::from(ps.iter().any(|p| p.role == PartRole::Step)))
        .sum();
    Some(x)
}

/// The bounding box of a stairs element: the box of its solids, extended in height to the stairs' base and top elevations
/// when they lie beyond (OBSERVED in every stairs of rounds 24 and 26: the harness box reaches the stairs' top elevation -
/// 3 m for a stairs between levels 3 m apart whose run stops at 2.12 m or 2.82 m - and goes higher only where the solids
/// do; which operation gives the box of the element is not specified). `None` without a solid.
pub fn stairs_bbox(m: &BimModel, stairs_id: ElementId) -> Option<([f64; 3], [f64; 3])> {
    let x = stairs_summary(m, stairs_id)?;
    let (mut lo, mut hi) = (x.min, x.max);
    if let Ok(st) = crate::stairs::stairs_state(m, stairs_id) {
        lo[2] = lo[2].min(st.bottom_elevation);
        hi[2] = hi[2].max(st.top_elevation);
    }
    Some((lo, hi))
}

/// The numbers of a railing's solids as the harness reads them: the rails (one multi-lump solid) and the balusters (one
/// solid each: solids = balusters + 1 composition); the top rail is an element of its own in the reference product and is
/// not in the railing's solid, but it is in the railing's box (OBSERVED: the box of the railing on a path reaches the top
/// rail's 0.02 m to the side while its solid does not).
pub fn railing_summary(m: &BimModel, id: ElementId) -> Option<SolidSummary> {
    let parts = railing_geometry(m, id).ok()?;
    let body: Vec<Part> = parts.iter().filter(|p| p.role != PartRole::TopRail).cloned().collect();
    let mut x = summarize(&body).or_else(|| summarize(&parts))?;
    x.solid_count = body.iter().filter(|p| p.role != PartRole::Rail).count() + usize::from(body.iter().any(|p| p.role == PartRole::Rail));
    let all = summarize(&parts)?;
    x.min = all.min;
    x.max = all.max;
    Some(x)
}

// ------------------------------------------------------------------------------------------------ plan line work

/// One segment of the plan line work of stairs and railings, with the element that owns it and its weight class (2 cut, 1 projection:
/// the classes of `asterion_view_linework`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanSeg {
    pub element: ElementId,
    pub a: Vec2,
    pub b: Vec2,
    pub class: f32,
}

fn seg_key(a: Vec2, b: Vec2) -> ((i64, i64), (i64, i64)) {
    let q = |p: Vec2| ((p.x * 1e6).round() as i64, (p.y * 1e6).round() as i64);
    let (qa, qb) = (q(a), q(b));
    if qa <= qb { (qa, qb) } else { (qb, qa) }
}

/// The plan line work of the polyhedral parts of one element: what the view's range lets the plan see of them. (an ENGINE
/// CHOICE: how the reference product draws a stairs in plan - tread lines, the break at the cut plane, the up arrow - is
/// not specified): a part the cut plane crosses draws the outline of its section at the cut plane (cut weight) and the
/// outlines of its upward faces below the plane (projection weight); a part wholly below the cut plane and above the bottom
/// of the view range draws the outlines of its upward faces; a part above the cut plane draws nothing. Spec V-02 decides
/// which of these a part is ([`crate::views::classify_plan_condition`]).
pub fn plan_part_segments(owner: ElementId, parts: &[Part], planes: &crate::views::PlanClassifierPlanes) -> Vec<PlanSeg> {
    use crate::views::{classify_plan_condition, PlanCondition};
    let mut out: Vec<PlanSeg> = Vec::new();
    let mut seen: std::collections::BTreeSet<((i64, i64), (i64, i64), u8)> = std::collections::BTreeSet::new();
    let mut push = |a: Vec2, b: Vec2, class: f32, out: &mut Vec<PlanSeg>| {
        if a.distance_to(b) > 1e-9 {
            let (ka, kb) = seg_key(a, b);
            if seen.insert((ka, kb, class as u8)) {
                out.push(PlanSeg { element: owner, a, b, class });
            }
        }
    };
    for p in parts {
        let (lo, hi) = p.solid.aabb();
        let cond = classify_plan_condition(lo[2], hi[2], planes);
        let cut = match cond {
            PlanCondition::Cut => true,
            PlanCondition::BelowCut => false,
            _ => continue,
        };
        // the upward faces below the cut plane
        for (fi, f) in p.solid.faces().iter().enumerate() {
            let Some(n) = p.solid.face_normal(fi) else { continue };
            if n[2] < 0.5 {
                continue;
            }
            let zs: Vec<f64> = f.iter().map(|&i| p.solid.vertices()[i][2]).collect();
            let (zl, zh) = zs.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), z| (l.min(*z), h.max(*z)));
            if matches!(classify_plan_condition(zl, zh, planes), PlanCondition::BelowCut) {
                for k in 0..f.len() {
                    let (a, b) = (p.solid.vertices()[f[k]], p.solid.vertices()[f[(k + 1) % f.len()]]);
                    push(Vec2::new(a[0], a[1]), Vec2::new(b[0], b[1]), 1.0, &mut out);
                }
            }
        }
        if cut {
            if let Some(c) = planes.cut {
                for l in p.solid.slice([0.0, 0.0, c], [0.0, 0.0, 1.0]) {
                    for k in 0..l.len() {
                        let (a, b) = (l[k], l[(k + 1) % l.len()]);
                        push(Vec2::new(a[0], a[1]), Vec2::new(b[0], b[1]), 2.0, &mut out);
                    }
                }
            }
        }
    }
    out
}

/// The range planes of a plan view as the classifier of Spec V-02 takes them; `None` for any other view or an invalid
/// range.
fn plan_planes(m: &BimModel, view: &crate::model::View) -> Option<crate::views::PlanClassifierPlanes> {
    use crate::model::ViewKind;
    if !matches!(view.kind, ViewKind::FloorPlan | ViewKind::CeilingPlan) {
        return None;
    }
    let st = crate::views::view_state_light(m, view).ok()?;
    st.range_valid.then(|| crate::views::PlanClassifierPlanes {
        ceiling: view.kind == ViewKind::CeilingPlan,
        cut: st.cut_z(),
        top: st.top_z(),
        bottom: st.bottom_z(),
        depth: st.depth_z(),
        underlay: None,
    })
}

/// The plan line work of every stairs and railing for a plan view (`view` is the view with its hidden set already extended: see
/// `view_visibility::effective_view`). Each run, landing and railing is drawn under its own id and the stringers under the stairs'.
pub fn plan_linework(m: &BimModel, view: &crate::model::View) -> Vec<PlanSeg> {
    let Some(planes) = plan_planes(m, view) else { return Vec::new() };
    let hidden = &view.hidden_elements;
    let mut out = Vec::new();
    for s in m.stairs.stairs.keys().filter(|s| !hidden.contains(s)) {
        for (owner, parts) in stairs_parts(m, *s) {
            if owner != *s && hidden.contains(&owner) {
                continue;
            }
            out.extend(plan_part_segments(owner, &parts, &planes));
        }
    }
    for r in m.stairs.railings.keys().chain(m.stairs_scene.path_railings.keys()).filter(|r| !hidden.contains(r)) {
        if let Ok(parts) = railing_geometry(m, *r) {
            out.extend(plan_part_segments(*r, &parts, &planes));
        }
    }
    out
}

/// The distance in plan from `p` to the footprint of a run (0 inside it): the rectangle between its location line and its width, by
/// justification. What a pick in a plan measures (the old pick measured the location line only).
pub fn run_footprint_distance(m: &BimModel, run: &StairsRun, p: Vec2) -> f64 {
    let Some(dir) = (run.end - run.start).normalized() else { return run.start.distance_to(p) };
    let length = run.start.distance_to(run.end);
    let (u0, u1) = lateral_extent(run.width, m.stairs_scene.runs.get(&run.id).map_or(Justification::Center, |e| e.justification));
    let d = p - run.start;
    let (s, u) = (d.dot(dir), d.dot(dir.perp_ccw()));
    let ds = (-s).max(s - length).max(0.0);
    let du = (u0 - u).max(u - u1).max(0.0);
    (ds * ds + du * du).sqrt()
}

/// The runs a plan view draws: a run whose steps appear in the view's range (Spec V-02, [`plan_linework`]) and, for a run
/// the engine has no solid for (a spiral or sketched run, a monolithic one), the plan of its base level - the presentation
/// the plan had before the stairs had a body. `view` is the view with its hidden set extended.
pub fn plan_drawn_runs(m: &BimModel, view: &crate::model::View) -> std::collections::BTreeSet<ElementId> {
    use crate::views::{classify_plan_condition, PlanCondition};
    let planes = plan_planes(m, view);
    let mut out = std::collections::BTreeSet::new();
    for r in m.stairs.runs.values() {
        let Some(s) = m.stairs.stairs.get(&r.stairs) else { continue };
        if view.hidden_elements.contains(&s.id) || view.hidden_elements.contains(&r.id) {
            continue;
        }
        match run_layout(m, r.id) {
            // a run with a solid is drawn when its vertical extent reaches the plan's range (cheap: no solid is built)
            Ok(l) => {
                let Some(planes) = &planes else { continue };
                let t = if l.params.has_treads { l.params.tread_thickness } else { 0.0 };
                let top = r.bottom_elevation + (l.treads + usize::from(l.begin_with_riser)) as f64 * s.actual_riser_height;
                if matches!(classify_plan_condition(r.bottom_elevation - t, top, planes), PlanCondition::Cut | PlanCondition::BelowCut) {
                    out.insert(r.id);
                }
            }
            // a run the engine has no solid for keeps the presentation it had before: the plan of its base level
            Err(_) => {
                if view.gen_level.is_none() || view.gen_level == Some(s.base_level) {
                    out.insert(r.id);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterion_geom::{boolean::union_all, Region, Tolerances};

    #[test]
    fn the_treads_of_a_run_count_as_row_s01f_says() {
        // Spec S-01f
        assert_eq!(tread_count(16, true, true), 15);
        assert_eq!(tread_count(16, false, true), 16);
        assert_eq!(tread_count(16, true, false), 16);
        assert_eq!(tread_count(16, false, false), 17);
        assert_eq!(tread_count(0, true, true), 0);
    }

    fn template() -> RunParams {
        let (t, r, n) = crate::stairs_scene::template_run_dims();
        RunParams { tread_thickness: t, riser_thickness: r, nosing_length: n, has_treads: true, has_risers: true, riser_slanted: true, monolithic: false }
    }

    fn area(p: &[[f64; 2]; 4]) -> f64 {
        (0..4).map(|i| p[i][0] * p[(i + 1) % 4][1] - p[(i + 1) % 4][0] * p[i][1]).sum::<f64>().abs() / 2.0
    }

    /// A run that begins and ends with a riser has N treads and N + 1 risers, 2N + 1 bodies; the volume is `W (N t (T + n)
    /// + c ((N + 1) R - t))` and the area `N (2W(T+n) + 2Wt sqrt(1+s^2) + 2(T+n)t)` plus, per riser with `h = R - t` for
    /// the first riser and `R` for the others.
    #[test]
    fn a_run_is_two_n_plus_one_separate_bodies_with_the_rows_volume_and_area() {
        let (r, t_depth, w) = (3.0 / 17.0, 0.275, 1.0);
        let p = template();
        let n_treads = 15usize;
        let bodies = run_step_bodies(&p, r, t_depth, 0.0, n_treads, true, true);
        assert_eq!(bodies.len(), 2 * n_treads + 1);
        assert_eq!(bodies.iter().filter(|b| b.riser).count(), n_treads + 1);
        let (t, n, rt) = (p.tread_thickness, p.nosing_length, p.riser_thickness);
        let s = n / r;
        let sq = (1.0 + s * s).sqrt();
        let c = rt * sq;
        let volume: f64 = bodies.iter().map(|b| area(&b.profile) * w).sum();
        let want_v = w * (n_treads as f64 * t * (t_depth + n) + c * ((n_treads + 1) as f64 * r - t));
        assert!((volume - want_v).abs() < 1e-12, "{volume} vs {want_v}");
        // the closed solids: 6 faces, 12 edges, 8 vertices each, and the area of the row
        let frame = ProfileFrame { origin: Vec2::new(0.0, 0.0), dir: Vec2::new(1.0, 0.0) };
        let solids: Vec<PolySolid> = bodies.iter().map(|b| PolySolid::from_profile(&frame, &b.profile, -w / 2.0, w / 2.0).unwrap()).collect();
        assert!(solids.iter().all(|x| (x.face_count(), x.edge_count(), x.vertex_count()) == (6, 12, 8)));
        let mut want_a = n_treads as f64 * (2.0 * w * (t_depth + n) + 2.0 * w * t * sq + 2.0 * (t_depth + n) * t);
        for k in 0..=n_treads {
            let h = if k == 0 { r - t } else { r };
            want_a += 2.0 * w * h * sq + 2.0 * c * h + 2.0 * c * w;
        }
        let got_a: f64 = solids.iter().map(PolySolid::surface_area).sum();
        assert!((got_a - want_a).abs() < 1e-9, "{got_a} vs {want_a}");
        let got_v: f64 = solids.iter().map(PolySolid::volume).sum();
        assert!((got_v - want_v).abs() < 1e-12);
    }

    /// The 94 distinct points `(s, z)` of one side of the lab harness solid of the template stairs (round 24, stairs `s1`:
    /// a run of 16 risers, 15 treads, 0.275 m tread, 3 / 17 m riser), less the 5 of its stringer. OBSERVED, a check.
    const HARNESS_SIDE: [(f64, f64); 94] = [
        (0.00000, 0.17647), (0.00708, 0.12647), (0.01971, 0.12647), (0.02500, 0.00000),
        (0.03762, 0.00000), (0.27500, 0.35294), (0.28208, 0.30294), (0.29471, 0.30294),
        (0.30000, 0.17647), (0.30708, 0.12647), (0.31971, 0.12647), (0.55000, 0.52941),
        (0.55708, 0.47941), (0.56971, 0.47941), (0.57500, 0.35294), (0.58208, 0.30294),
        (0.59471, 0.30294), (0.82500, 0.70588), (0.83208, 0.65588), (0.84471, 0.65588),
        (0.85000, 0.52941), (0.85708, 0.47941), (0.86971, 0.47941), (1.10000, 0.88235),
        (1.10708, 0.83235), (1.11971, 0.83235), (1.12500, 0.70588), (1.13208, 0.65588),
        (1.14471, 0.65588), (1.37500, 1.05882), (1.38208, 1.00882), (1.39471, 1.00882),
        (1.40000, 0.88235), (1.40708, 0.83235), (1.41971, 0.83235), (1.65000, 1.23529),
        (1.65708, 1.18529), (1.66971, 1.18529), (1.67500, 1.05882), (1.68208, 1.00882),
        (1.69471, 1.00882), (1.92500, 1.41176), (1.93208, 1.36176), (1.94471, 1.36176),
        (1.95000, 1.23529), (1.95708, 1.18529), (1.96971, 1.18529), (2.20000, 1.58824),
        (2.20708, 1.53824), (2.21971, 1.53824), (2.22500, 1.41176), (2.23208, 1.36176),
        (2.24471, 1.36176), (2.47500, 1.76471), (2.48208, 1.71471), (2.49471, 1.71471),
        (2.50000, 1.58824), (2.50708, 1.53824), (2.51971, 1.53824), (2.75000, 1.94118),
        (2.75708, 1.89118), (2.76971, 1.89118), (2.77500, 1.76471), (2.78208, 1.71471),
        (2.79471, 1.71471), (3.02500, 2.11765), (3.03208, 2.06765), (3.04471, 2.06765),
        (3.05000, 1.94118), (3.05708, 1.89118), (3.06971, 1.89118), (3.30000, 2.29412),
        (3.30708, 2.24412), (3.31971, 2.24412), (3.32500, 2.11765), (3.33208, 2.06765),
        (3.34471, 2.06765), (3.57500, 2.47059), (3.58208, 2.42059), (3.59471, 2.42059),
        (3.60000, 2.29412), (3.60708, 2.24412), (3.61971, 2.24412), (3.85000, 2.64706),
        (3.85708, 2.59706), (3.86971, 2.59706), (3.87500, 2.47059), (3.88208, 2.42059),
        (3.89471, 2.42059), (4.13208, 2.77353), (4.14471, 2.77353), (4.15000, 2.64706),
        (4.15708, 2.59706), (4.16971, 2.59706),
    ];

    fn distance_to_boundary(poly: &[[f64; 2]], q: (f64, f64)) -> f64 {
        let n = poly.len();
        (0..n)
            .map(|i| {
                let (a, b) = (poly[i], poly[(i + 1) % n]);
                let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                let l2 = dx * dx + dz * dz;
                let t = if l2 > 0.0 { (((q.0 - a[0]) * dx + (q.1 - a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
                ((a[0] + t * dx - q.0).powi(2) + (a[1] + t * dz - q.1).powi(2)).sqrt()
            })
            .fold(f64::INFINITY, f64::min)
    }

    /// The bodies of the rows, joined, have the outline of the harness solid (its points are the corners of the faces; the harness also lists the
    /// points in the middle of straight edges, so: every engine corner is a harness point and every harness point lies on the outline).
    #[test]
    fn the_template_run_has_the_vertices_the_harness_measured() {
        let h = 3.0 / 17.0;
        let bodies = run_step_bodies(&template(), h, 0.275, 0.0, 15, true, true);
        let tol = Tolerances::default();
        let regions: Vec<Region> = bodies
            .iter()
            .map(|b| Region::from_polygon(&b.profile.iter().map(|q| Vec2::new(q[0], q[1])).collect::<Vec<_>>(), &tol).unwrap())
            .collect();
        let joined = union_all(&regions, &tol).unwrap();
        assert_eq!(joined.len(), 1, "treads and risers touch face to face: one outline");
        let pts: Vec<[f64; 2]> = joined[0].outer().corner_points().iter().map(|q| [q.x, q.y]).collect();
        for p in &pts {
            assert!(HARNESS_SIDE.iter().any(|q| (q.0 - p[0]).abs() < 6e-5 && (q.1 - p[1]).abs() < 6e-5), "the engine corner {p:?} is no harness point");
        }
        for q in HARNESS_SIDE {
            let d = distance_to_boundary(&pts, q);
            assert!(d < 6e-5, "the harness point {q:?} is {d} m from the engine outline");
        }
    }

    /// The height shift of the template's run support and of its landing support, which the lab harness shows as the
    /// stringer top 0.0395106 m above the nosing line and the landing support top 0.0473937 m above the landing's top.
    #[test]
    fn the_support_shifts_are_the_rows_formula() {
        let (r, t_depth) = (3.0 / 17.0, 0.275);
        let shift = run_support_shift(&TEMPLATE_SUPPORT, &template(), r, t_depth);
        assert!((shift - 0.039_510_586_705_758).abs() < 1e-9, "{shift}");
        let landing = (TEMPLATE_SUPPORT.total_depth - TEMPLATE_SUPPORT.structural_depth_landing) - 0.05;
        assert!((landing - 0.047_393_726_959_043).abs() < 1e-9, "{landing}");
        // the fine type of round 24 (riser 0.15, tread 0.3) with a riser 0.03 thick: 0.045399 (s7), with 0.0125: 0.054270 (s3)
        let mut p = template();
        p.riser_thickness = 0.03;
        assert!((run_support_shift(&TEMPLATE_SUPPORT, &p, 0.15, 0.3) - 0.045_399).abs() < 1e-6);
        p.riser_thickness = 0.0125;
        assert!((run_support_shift(&TEMPLATE_SUPPORT, &p, 0.15, 0.3) - 0.054_270).abs() < 1e-6);
    }

    #[test]
    fn a_stringer_is_cut_square_at_the_ends_and_by_the_base_plane() {
        // OBSERVED shape: 5 corners, the bottom meeting z = 0 at x = 0.40407
        let slope = (3.0 / 17.0) / 0.275;
        let shift = run_support_shift(&TEMPLATE_SUPPORT, &template(), 3.0 / 17.0, 0.275);
        let p = support_profile(3.0 / 17.0 + shift, slope, 4.125, 0.4, 0.0);
        assert_eq!(p.len(), 5);
        assert!(p.iter().any(|q| (q[0] - 0.404_07).abs() < 1e-4 && q[1].abs() < 1e-12));
        assert!(p.iter().any(|q| q[0] == 0.0 && (q[1] - 0.215_981).abs() < 1e-5));
        assert!(p.iter().any(|q| q[0] == 4.125 && (q[1] - 2.863_04).abs() < 1e-4));
    }
}
