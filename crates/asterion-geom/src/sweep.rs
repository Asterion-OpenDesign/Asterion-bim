//! Sweeps: a planar profile carried along a plan path.
//!
//! What it is for: wall sweeps and reveals, mouldings, skirtings, slab edges,
//! handrails along a curved run — every "profile following a line" element.
//!
//! # Frames
//!
//! * The **path** is an open or closed chain of plan curves (segments and arcs)
//!   at a fixed `elevation`.
//! * The **profile** is a `Region` in the path's local section plane with
//!   coordinates `(u, v)`: `u` is the horizontal distance to the **left** of
//!   travel (the kernel's usual left-positive convention, as in
//!   `Curve::offset`), `v` is height above the path's elevation.
//!
//! # Why every quantity is exact: one linear function
//!
//! A profile point at lateral offset `u` travels along the path *offset by
//! `u`*, and the length of that offset path is **linear in `u`**:
//!
//! * a segment contributes its length, minus `u * tan(phi/2)` at each mitred
//!   end where the path turns by `phi` (left turns shorten the left side);
//! * an arc of signed sweep `theta` and radius `R` contributes
//!   `|theta| * R - theta * u` (the left side of a counter-clockwise arc is
//!   the inside, so it is shorter).
//!
//! So `L(u) = L0 + k * u`, and integrating over the profile gives the
//! **generalised Pappus theorem**:
//!
//! * `volume  = A * (L0 + k * u_area)` with `u_area` the profile's area
//!   centroid;
//! * `lateral = P * L0 + k * M` with `M = integral of u ds` over the profile
//!   boundary (holes included);
//! * end caps add `2A` for an open path, nothing for a closed one.
//!
//! For a single arc this *is* Pappus (`A * |theta| * R_centroid`); for a mitred
//! polyline it is the classical "truncated prism = section x centroidal
//! length". No tessellation anywhere.
//!
//! # The degenerate case: a profile that crosses the axis
//!
//! Every path segment sweeps each profile point a length `L_i(u)`. Validity
//! requires `L_i(u) >= 0` for every `u` the profile occupies (it is linear, so
//! checking the profile's extreme `u` is enough). On an arc,
//! `L_i(u) < 0` means the point lies **beyond the arc's centre** — the profile
//! crosses the axis of revolution. There the solid self-intersects and
//! **no single closed form is right**: signed Pappus subtracts the far part,
//! absolute Pappus double-counts the overlap, and the true volume depends on
//! the sweep angle (for `theta < pi` the two parts are disjoint, for a full
//! turn they overlap). `sweep::tests::axis_crossing_has_no_single_closed_form`
//! pins three different numbers for one such case.
//!
//! ASTERION therefore **rejects** it with `GEOM_SWEEP_SELF_INTERSECTS`,
//! reporting the segment and the most negative swept length, instead of
//! returning a wrong volume. A profile that only *touches* the axis
//! (`L_i = 0` at one edge, within tolerance) is accepted: that is a cone-like
//! apex, not an overlap, and Pappus holds. The same rule rejects a mitred
//! corner so sharp that a segment's inner side would be swept a negative
//! length.
//!
//! Rejecting is the conservative choice and is reversible once a measurement says otherwise.

//!
//! # Sloped paths: what "vertical profile" means
//!
//! A ramp, a stair handrail or the edge of a sloped slab follows its plan path
//! while rising at a uniform **grade** `g` (rise over run, measured along the
//! path's centreline: `z(s) = elevation + g * s`). There are two different
//! solids that both deserve the name "profile swept up a slope", and they do
//! **not** have the same volume:
//!
//! * [`SlopeConvention::VerticalProfile`] — the section plane stays
//!   **vertical** (it contains the horizontal normal of the path and the true
//!   vertical). The solid is the flat sweep **sheared** vertically by `g * s`.
//!   A shear preserves volume, so `V = A * (L0 + k * u_area)` exactly as on
//!   flat ground, whatever the grade and on arcs too (a helical ramp has the
//!   volume of its flat curved band). Inclined skins get longer by
//!   `sqrt(1 + g^2 cos^2)`, vertical skins do not.
//! * [`SlopeConvention::PerpendicularProfile`] — the section plane is
//!   **perpendicular to the 3D path tangent** (tilted back by `atan g`). On a
//!   straight run the solid is a right prism of length `L * sqrt(1 + g^2)`:
//!   `V = A * L * sqrt(1 + g^2)` and lateral `= P * L * sqrt(1 + g^2)`.
//!
//! On a 1:12 ramp the two volumes differ by a factor `sqrt(1 + 1/144)`, i.e.
//! 0.35 %, well above the 1e-4 the comparison harness resolves, so the choice
//! is measurable and must be measured.
//!
//! **Unmeasured for sweeps.** The convention is a required argument with no
//! default. Round 11 measured the analogous question on **sloped floors** and
//! the reference uses the **perpendicular** thickness there
//! (`slab::ThicknessConvention::Perpendicular`, OBSERVED), which falsifies the
//! earlier "consistency with `Slab`" argument for `VerticalProfile`. If
//! consistency is the guide, it now points to `PerpendicularProfile`; but a
//! sweep is a different host and only a sweep observation can settle it.
//! `sweep::tests::the_two_slope_conventions_give_different_volumes` records
//! both numbers.
//!
//! Scope, stated rather than approximated: with a non-zero grade the path must
//! be tangent-continuous (a mitred corner would assign two heights to the same
//! plan point on the mitre plane — real handrails use a gooseneck there), the
//! perpendicular convention is supported on straight paths only (a helical
//! tube needs a Frenet-frame correction), and the vertical convention's skin
//! swept by an *arc* of the profile is an elliptic integral, so it is reported
//! as `GEOM_SWEEP_NO_CLOSED_FORM` instead of being tessellated.

use crate::band::offset_band;
use crate::curve::Curve;
use crate::error::{GeomError, GeomResult};
use crate::prism::Aabb;
use crate::region::Region;
use crate::tolerance::Tolerances;
use crate::vec::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// Swept length of one path segment as a linear function of `u`:
/// `base + slope * u`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SegmentLength {
    pub base: f64,
    pub slope: f64,
}

impl SegmentLength {
    pub fn at(&self, u: f64) -> f64 {
        self.base + self.slope * u
    }
}

/// How the profile is oriented when the path rises. See the module docs: the
/// two conventions give different volumes and the reference's choice is not
/// measured yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlopeConvention {
    /// Section plane stays vertical; the solid is the flat sweep sheared by
    /// the grade. Unmeasured for sweeps (falsified for sloped floors).
    VerticalProfile,
    /// Section plane perpendicular to the 3D path tangent. Unmeasured for sweeps; the convention sloped floors use.
    PerpendicularProfile,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sweep {
    profile: Region,
    path: Vec<Curve>,
    elevation: f64,
    /// Rise over run along the centreline; 0 for a level sweep.
    grade: f64,
    convention: SlopeConvention,
}

impl Sweep {
    /// Validate and build. Fails if the path is disconnected, has a
    /// non-tangent corner involving an arc, or if any part of the profile
    /// would be swept a negative length (self-intersection).
    pub fn new(profile: Region, path: Vec<Curve>, elevation: f64, tol: &Tolerances) -> GeomResult<Self> {
        // On level ground the two slope conventions coincide.
        Sweep::new_sloped(profile, path, elevation, 0.0, SlopeConvention::VerticalProfile, tol)
    }

    /// A sweep that rises at a uniform `grade` along its centreline. The
    /// convention is required: the two give different solids (module docs).
    pub fn new_sloped(
        profile: Region,
        path: Vec<Curve>,
        elevation: f64,
        grade: f64,
        convention: SlopeConvention,
        tol: &Tolerances,
    ) -> GeomResult<Self> {
        crate::band::validate_path(&path, tol)?;
        if grade != 0.0 {
            // A mitre would give two heights to one plan point.
            for i in 0..path.len().saturating_sub(1) {
                let t_in = path[i].tangent_at(1.0).unwrap_or(Vec2::new(1.0, 0.0));
                let t_out = path[i + 1].tangent_at(0.0).unwrap_or(Vec2::new(1.0, 0.0));
                let phi = t_in.cross(t_out).atan2(t_in.dot(t_out));
                if phi.abs() > tol.angle {
                    return Err(GeomError::SweepUnsupportedCorner { index: i, angle: phi });
                }
            }
            if convention == SlopeConvention::PerpendicularProfile {
                for (i, c) in path.iter().enumerate() {
                    if c.is_arc() {
                        return Err(GeomError::SweepConventionUnsupported { segment: i });
                    }
                }
            }
        }
        let sweep = Sweep { profile, path, elevation, grade, convention };
        let lengths = sweep.segment_lengths(tol)?;
        let pb = sweep.profile.aabb();
        for (i, l) in lengths.iter().enumerate() {
            let worst = l.at(pb.min.x).min(l.at(pb.max.x));
            if worst < -tol.vertex {
                return Err(GeomError::SweepSelfIntersects { segment: i, min_length: worst });
            }
        }
        Ok(sweep)
    }

    pub fn profile(&self) -> &Region {
        &self.profile
    }
    pub fn path(&self) -> &[Curve] {
        &self.path
    }
    pub fn elevation(&self) -> f64 {
        self.elevation
    }
    pub fn grade(&self) -> f64 {
        self.grade
    }
    pub fn convention(&self) -> SlopeConvention {
        self.convention
    }
    /// Elevation of the centreline at the end of the path.
    pub fn end_elevation(&self) -> f64 {
        self.elevation + self.grade * self.path_length()
    }

    /// A path whose end meets its start is closed: no end caps, and the seam
    /// is a corner like any other. A rising path never closes (a full turn of
    /// a helix ends one storey up), so it always keeps its caps.
    pub fn is_closed(&self, tol: &Tolerances) -> bool {
        self.grade == 0.0
            && self.path.len() > 1
            && self.path.last().unwrap().end().distance_to(self.path[0].start()) <= tol.vertex
    }

    /// Length of the path itself (`u = 0`).
    pub fn path_length(&self) -> f64 {
        self.path.iter().map(|c| c.length()).sum()
    }

    /// Per-segment swept length `L_i(u)`, including the mitre corrections of
    /// both ends of each segment.
    pub fn segment_lengths(&self, tol: &Tolerances) -> GeomResult<Vec<SegmentLength>> {
        let n = self.path.len();
        let mut out: Vec<SegmentLength> = self
            .path
            .iter()
            .map(|c| match c {
                Curve::Line(l) => SegmentLength { base: l.length(), slope: 0.0 },
                Curve::Arc(a) => SegmentLength {
                    base: a.sweep.abs() * a.radius,
                    slope: -a.sweep,
                },
            })
            .collect();
        let closed = self.is_closed(tol);
        let corners = if closed { n } else { n.saturating_sub(1) };
        for i in 0..corners {
            let j = (i + 1) % n;
            let (a, b) = (&self.path[i], &self.path[j]);
            let t_in = a.tangent_at(1.0).unwrap_or(Vec2::new(1.0, 0.0));
            let t_out = b.tangent_at(0.0).unwrap_or(Vec2::new(1.0, 0.0));
            let phi = t_in.cross(t_out).atan2(t_in.dot(t_out));
            if phi.abs() <= tol.angle {
                continue; // tangent-continuous: no correction
            }
            match (a, b) {
                (Curve::Line(_), Curve::Line(_)) => {
                    // A left turn shortens the left side of both segments by
                    // u * tan(phi/2); a right turn lengthens it.
                    let t = (phi / 2.0).tan();
                    out[i].slope -= t;
                    out[j].slope -= t;
                }
                _ => return Err(GeomError::SweepUnsupportedCorner { index: i, angle: phi }),
            }
        }
        Ok(out)
    }

    /// `L(u) = L0 + k u` for the whole path: `(L0, k)`.
    pub fn swept_length_coefficients(&self, tol: &Tolerances) -> GeomResult<(f64, f64)> {
        let segs = self.segment_lengths(tol)?;
        Ok((
            segs.iter().map(|s| s.base).sum(),
            segs.iter().map(|s| s.slope).sum(),
        ))
    }

    /// Length swept by a profile point at lateral offset `u`.
    pub fn swept_length_at(&self, u: f64, tol: &Tolerances) -> GeomResult<f64> {
        let (l0, k) = self.swept_length_coefficients(tol)?;
        Ok(l0 + k * u)
    }

    /// Exact volume.
    ///
    /// * level, or `VerticalProfile` at any grade: `A * (L0 + k * u_centroid)`
    ///   (a vertical shear preserves volume);
    /// * `PerpendicularProfile` on a straight path: `A * L * sqrt(1 + g^2)`.
    pub fn volume(&self, tol: &Tolerances) -> GeomResult<f64> {
        if self.grade != 0.0 && self.convention == SlopeConvention::PerpendicularProfile {
            let stretch = (1.0 + self.grade * self.grade).sqrt();
            return Ok(self.profile.area() * self.path_length() * stretch);
        }
        let (l0, k) = self.swept_length_coefficients(tol)?;
        let a = self.profile.area();
        let u_c = self.profile.centroid().map(|c| c.x).unwrap_or(0.0);
        Ok(a * (l0 + k * u_c))
    }

    /// Exact lateral area: `P * L0 + k * integral(u ds)` over every profile
    /// boundary curve (holes included — a hollow profile sweeps an inner skin).
    pub fn lateral_area(&self, tol: &Tolerances) -> GeomResult<f64> {
        if self.grade != 0.0 {
            return match self.convention {
                SlopeConvention::PerpendicularProfile => {
                    let stretch = (1.0 + self.grade * self.grade).sqrt();
                    let perimeter: f64 = self.profile.loops().map(|l| l.perimeter()).sum();
                    Ok(perimeter * self.path_length() * stretch)
                }
                SlopeConvention::VerticalProfile => self.sloped_vertical_lateral_area(),
            };
        }
        let (l0, k) = self.swept_length_coefficients(tol)?;
        let mut perimeter = 0.0;
        let mut moment = 0.0;
        for lp in self.profile.loops() {
            for c in lp.curves() {
                perimeter += c.length();
                moment += c.first_moment_x();
            }
        }
        Ok(perimeter * l0 + k * moment)
    }

    /// End caps: two copies of the profile for an open path, none when closed.
    pub fn cap_area(&self, tol: &Tolerances) -> f64 {
        if self.is_closed(tol) {
            0.0
        } else {
            2.0 * self.profile.area()
        }
    }

    pub fn surface_area(&self, tol: &Tolerances) -> GeomResult<f64> {
        Ok(self.lateral_area(tol)? + self.cap_area(tol))
    }

    /// Exact bounding box. The plan projection of the swept solid is the band
    /// of the path between the profile's extreme offsets, so its box is the
    /// band's box; heights come from the profile's `v` range.
    pub fn aabb(&self, tol: &Tolerances) -> GeomResult<Aabb> {
        if self.grade != 0.0 && self.convention == SlopeConvention::PerpendicularProfile {
            return Ok(self.perpendicular_aabb());
        }
        let pb = self.profile.aabb();
        let (u_min, u_max) = (pb.min.x, pb.max.x);
        // Vertical shear: the plan projection is unchanged, heights gain the
        // rise of the centreline over the whole path.
        let rise = self.grade * self.path_length();
        // offset_band takes distances to the left and to the right.
        let band = offset_band(&self.path, u_max, -u_min, tol)?;
        let b = band.aabb();
        Ok(Aabb {
            min: Vec3::new(b.min.x, b.min.y, self.elevation + pb.min.y + rise.min(0.0)),
            max: Vec3::new(b.max.x, b.max.y, self.elevation + pb.max.y + rise.max(0.0)),
        })
    }

    /// Lateral skin of a `VerticalProfile` sweep on a sloped path.
    ///
    /// Parametrise the solid by centreline length `s`, lateral offset `u` and
    /// height `v`: a point is `(P(s, u), elevation + g s + v)`. For a profile
    /// edge with unit direction `(c_u, c_v)` the area element is
    /// `sqrt(lambda(u)^2 + g^2 c_u^2) dw ds`, where `lambda(u)` is how much the
    /// offset curve at `u` is stretched relative to the centreline (1 on a
    /// segment, `(R - sigma u) / R` on an arc). Integrated:
    ///
    /// * segment: `L * len_edge * sqrt(1 + g^2 c_u^2)`;
    /// * arc, vertical edge (`c_u = 0`): `|theta| * rho * len_edge` with
    ///   `rho = R - sigma u` — an ordinary cylinder strip;
    /// * arc, inclined edge: with `C = g R c_u` and `rho` linear along the
    ///   edge, `|theta| / |c_u| * [F(rho)]` where
    ///   `F(rho) = rho/2 sqrt(rho^2 + C^2) + C^2/2 asinh(rho / C)` — a helicoid
    ///   strip, elementary.
    ///
    /// An *arc* in the profile turns `c_u` into a function of the angle and
    /// the integral becomes elliptic, so it is reported, not approximated.
    fn sloped_vertical_lateral_area(&self) -> GeomResult<f64> {
        let g = self.grade;
        let mut total = 0.0;
        let mut index = 0usize;
        for lp in self.profile.loops() {
            for pc in lp.curves() {
                let edge = match pc {
                    Curve::Line(l) => *l,
                    Curve::Arc(_) => return Err(GeomError::SweepNoClosedForm { profile_curve: index }),
                };
                index += 1;
                let len = edge.length();
                if len == 0.0 {
                    continue;
                }
                let c_u = (edge.end.x - edge.start.x) / len;
                for seg in &self.path {
                    total += match seg {
                        Curve::Line(l) => l.length() * len * (1.0 + g * g * c_u * c_u).sqrt(),
                        Curve::Arc(a) => {
                            let sigma = a.sweep.signum();
                            let r = a.radius;
                            let rho0 = r - sigma * edge.start.x;
                            let rho1 = r - sigma * edge.end.x;
                            if c_u.abs() < 1e-15 {
                                a.sweep.abs() * rho0.abs() * len
                            } else {
                                let c = g * r * c_u;
                                let f = |rho: f64| {
                                    if c == 0.0 {
                                        rho * rho.abs() / 2.0
                                    } else {
                                        rho / 2.0 * (rho * rho + c * c).sqrt()
                                            + c * c / 2.0 * (rho / c.abs()).asinh()
                                    }
                                };
                                a.sweep.abs() / c_u.abs() * (f(rho1) - f(rho0)).abs()
                            }
                        }
                    };
                }
            }
        }
        Ok(total)
    }

    /// Bounding box of a `PerpendicularProfile` sweep on a straight path: the
    /// solid is a right prism along the 3D direction, so its box is the box of
    /// the two end sections, and each section's extreme along a world axis is
    /// a support point of the profile (exact for arcs in the profile too).
    fn perpendicular_aabb(&self) -> Aabb {
        let start = self.path[0].start();
        let end = self.path.last().unwrap().end();
        let plan = end - start;
        let l = plan.length();
        let t = plan.normalized().unwrap_or(Vec2::new(1.0, 0.0));
        let n = t.perp_ccw();
        let alpha = self.grade.atan();
        let (sa, ca) = alpha.sin_cos();
        // Profile axes in world space: u -> (n, 0); v -> (-t sin a, cos a).
        let u_axis = Vec3::new(n.x, n.y, 0.0);
        let v_axis = Vec3::new(-t.x * sa, -t.y * sa, ca);
        let outer = self.profile.outer();
        let extreme = |cu: f64, cv: f64| -> (f64, f64) {
            let hi = outer.support_point(Vec2::new(cu, cv));
            let lo = outer.support_point(Vec2::new(-cu, -cv));
            (cu * lo.x + cv * lo.y, cu * hi.x + cv * hi.y)
        };
        let origins = [
            Vec3::new(start.x, start.y, self.elevation),
            Vec3::new(end.x, end.y, self.elevation + self.grade * l),
        ];
        let axis = |o: &Vec3, k: usize| match k {
            0 => o.x,
            1 => o.y,
            _ => o.z,
        };
        let comp = |v: &Vec3, k: usize| match k {
            0 => v.x,
            1 => v.y,
            _ => v.z,
        };
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for k in 0..3 {
            let (smin, smax) = extreme(comp(&u_axis, k), comp(&v_axis, k));
            for o in &origins {
                lo[k] = lo[k].min(axis(o, k) + smin);
                hi[k] = hi[k].max(axis(o, k) + smax);
            }
        }
        Aabb { min: Vec3::new(lo[0], lo[1], lo[2]), max: Vec3::new(hi[0], hi[1], hi[2]) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::band::offset_band;
    use crate::curve::Arc;
    use crate::region::Polycurve;
    use crate::solid::ExtrudedRegion;
    use std::f64::consts::{PI, TAU};

    fn tol() -> Tolerances {
        Tolerances::default()
    }
    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-12, "{a} != {b}");
    }
    fn near_eps(a: f64, b: f64, eps: f64) {
        assert!((a - b).abs() < eps, "{a} != {b}");
    }
    /// Profile rectangle in the section plane: u in [u0, u1], v in [v0, v1].
    fn profile(u0: f64, v0: f64, u1: f64, v1: f64) -> Region {
        Region::from_loop(
            Polycurve::rectangle(Vec2::new(u0, v0), Vec2::new(u1, v1), &tol()).unwrap(),
        )
    }
    fn arc_path(r: f64, sweep: f64) -> Vec<Curve> {
        vec![Curve::Arc(Arc::new(Vec2::new(0.0, 0.0), r, 0.0, sweep).unwrap())]
    }

    // ------------------------------------------------------------ straight

    #[test]
    fn straight_sweep_is_section_times_length() {
        // A 0.200 x 0.100 skirting along a 5 m wall.
        let s = Sweep::new(
            profile(0.0, 0.0, 0.2, 0.1),
            vec![Curve::line(Vec2::new(0.0, 0.0), Vec2::new(5.0, 0.0))],
            0.0,
            &tol(),
        )
        .unwrap();
        near(s.volume(&tol()).unwrap(), 0.02 * 5.0);
        near(s.lateral_area(&tol()).unwrap(), 0.6 * 5.0);
        near(s.cap_area(&tol()), 0.04);
        near(s.surface_area(&tol()).unwrap(), 3.04);
        let b = s.aabb(&tol()).unwrap();
        near(b.min.y, 0.0);
        near(b.max.y, 0.2);
        near(b.max.z, 0.1);
    }

    // ----------------------------------------------------------------- arcs

    /// Two independent code paths must agree: a rectangle swept along an arc
    /// IS the curved-wall body (annular sector band extruded), whose numbers
    /// were already pinned in `band.rs` / `solid.rs`.
    #[test]
    fn arc_sweep_of_a_rectangle_equals_the_curved_band_extrusion() {
        let t = tol();
        let (r, theta, half, h) = (5.0, PI / 2.0, 0.1, 0.3);
        let path = arc_path(r, theta);
        let s = Sweep::new(profile(-half, 0.0, half, h), path.clone(), 0.0, &t).unwrap();
        let band = offset_band(&path, half, half, &t).unwrap();
        let body = ExtrudedRegion::new(band, 0.0, h).unwrap();
        near_eps(s.volume(&t).unwrap(), body.volume(), 1e-12);
        near_eps(s.surface_area(&t).unwrap(), body.surface_area(), 1e-12);
        // And both are Pappus: A * theta * R_centroid with R_centroid = R.
        near_eps(s.volume(&t).unwrap(), (2.0 * half * h) * theta * r, 1e-12);
    }

    #[test]
    fn off_centre_profile_on_an_arc_uses_the_centroid_radius() {
        // CCW arc: left of travel is the centre, so u in [0.5, 1.0] sits at
        // radius 4.5 .. 4.0 from the centre; centroid radius 4.25.
        let t = tol();
        let (r, theta, h) = (5.0, PI / 3.0, 0.4);
        let path = arc_path(r, theta);
        let s = Sweep::new(profile(0.5, 0.0, 1.0, h), path.clone(), 0.0, &t).unwrap();
        let a = 0.5 * h;
        near_eps(s.volume(&t).unwrap(), a * theta * 4.25, 1e-12);
        let band = offset_band(&path, 1.0, -0.5, &t).unwrap();
        let body = ExtrudedRegion::new(band, 0.0, h).unwrap();
        near_eps(s.volume(&t).unwrap(), body.volume(), 1e-12);
        // Compare whole surfaces: the sweep's lateral skin includes the top
        // and bottom (swept by the profile's horizontal edges) and its caps are
        // the band body's radial end faces, so only the totals correspond.
        near_eps(s.surface_area(&t).unwrap(), body.surface_area(), 1e-12);
    }

    #[test]
    fn clockwise_arc_puts_the_left_side_outside() {
        // Same arc, travelled clockwise: u > 0 is now the OUTER side.
        let t = tol();
        let path = vec![Curve::Arc(Arc::new(Vec2::new(0.0, 0.0), 5.0, PI / 3.0, -PI / 3.0).unwrap())];
        let s = Sweep::new(profile(0.5, 0.0, 1.0, 0.4), path, 0.0, &t).unwrap();
        near_eps(s.volume(&t).unwrap(), 0.2 * (PI / 3.0) * 5.75, 1e-12);
    }

    #[test]
    fn torus_from_a_circular_profile_on_a_closed_circular_path() {
        // Closed path (two half arcs), circular profile radius a centred on
        // the path: V = 2 pi^2 R a^2, S = 4 pi^2 R a, no end caps.
        let t = tol();
        let (r, a) = (3.0, 1.0);
        let path = vec![
            Curve::Arc(Arc::new(Vec2::new(0.0, 0.0), r, 0.0, PI).unwrap()),
            Curve::Arc(Arc::new(Vec2::new(0.0, 0.0), r, PI, PI).unwrap()),
        ];
        let circle = Region::from_loop(Polycurve::circle(Vec2::new(0.0, 1.5), a, &t).unwrap());
        let s = Sweep::new(circle, path, 0.0, &t).unwrap();
        assert!(s.is_closed(&t));
        near_eps(s.volume(&t).unwrap(), 2.0 * PI * PI * r * a * a, 1e-10);
        near_eps(s.lateral_area(&t).unwrap(), 4.0 * PI * PI * r * a, 1e-10);
        near(s.cap_area(&t), 0.0);
        let b = s.aabb(&t).unwrap();
        near_eps(b.max.x, r + a, 1e-12);
        near_eps(b.min.z, 0.5, 1e-12);
        near_eps(b.max.z, 2.5, 1e-12);
    }

    // ---------------------------------------------------- mitred polylines

    #[test]
    fn mitred_corner_sweep_equals_the_mitred_band() {
        // Moulding round a right-angle corner: path (0,0)->(4,0)->(4,3).
        let t = tol();
        let path = vec![
            Curve::line(Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0)),
            Curve::line(Vec2::new(4.0, 0.0), Vec2::new(4.0, 3.0)),
        ];
        let h = 0.15;
        // Centred profile: the corner corrections cancel, V = A * 7.
        let centred = Sweep::new(profile(-0.2, 0.0, 0.2, h), path.clone(), 0.0, &t).unwrap();
        near_eps(centred.volume(&t).unwrap(), 0.4 * h * 7.0, 1e-12);
        // Off-centre on the inner (left) side: L(u) = 7 - 2 tan(45) u, so the
        // centroid at u = 0.2 travels 6.6.
        let inner = Sweep::new(profile(0.0, 0.0, 0.4, h), path.clone(), 0.0, &t).unwrap();
        near_eps(inner.swept_length_at(0.2, &t).unwrap(), 6.6, 1e-12);
        near_eps(inner.volume(&t).unwrap(), 0.4 * h * 6.6, 1e-12);
        // Cross-check against the independent band code path.
        let band = offset_band(&path, 0.4, 0.0, &t).unwrap();
        let body = ExtrudedRegion::new(band, 0.0, h).unwrap();
        near_eps(inner.volume(&t).unwrap(), body.volume(), 1e-12);
        near_eps(inner.surface_area(&t).unwrap(), body.surface_area(), 1e-12);
    }

    #[test]
    fn tangent_line_into_arc_needs_no_correction() {
        let t = tol();
        let path = vec![
            Curve::line(Vec2::new(-3.0, 0.0), Vec2::new(0.0, 0.0)),
            Curve::Arc(Arc::from_center_angles(Vec2::new(0.0, 2.0), 2.0, -PI / 2.0, 0.0, true).unwrap()),
        ];
        let s = Sweep::new(profile(-0.1, 0.0, 0.1, 1.0), path.clone(), 0.0, &t).unwrap();
        let (l0, k) = s.swept_length_coefficients(&t).unwrap();
        near_eps(l0, 3.0 + PI, 1e-12);
        near_eps(k, -PI / 2.0, 1e-12);
        let band = offset_band(&path, 0.1, 0.1, &t).unwrap();
        near_eps(s.volume(&t).unwrap(), band.area() * 1.0, 1e-12);
    }

    #[test]
    fn non_tangent_corner_with_an_arc_is_rejected() {
        let t = tol();
        let kinked = vec![
            Curve::line(Vec2::new(1.0, -3.0), Vec2::new(1.0, -1.0)),
            Curve::line(Vec2::new(1.0, -1.0), Vec2::new(1.0, 0.0)),
            Curve::Arc(Arc::new(Vec2::new(0.0, 0.0), 1.0, 0.0, PI / 2.0).unwrap()),
        ];
        // line -> line is collinear (fine); line (+Y) -> arc start tangent
        // (+Y at angle 0, CCW) is also tangent, so this one is accepted:
        assert!(Sweep::new(profile(-0.1, 0.0, 0.1, 0.1), kinked, 0.0, &t).is_ok());
        // A genuine kink into an arc: approach (1,0) along +X.
        let bad = vec![
            Curve::line(Vec2::new(-1.0, 0.0), Vec2::new(1.0, 0.0)),
            Curve::Arc(Arc::new(Vec2::new(0.0, 0.0), 1.0, 0.0, PI / 2.0).unwrap()),
        ];
        let e = Sweep::new(profile(-0.1, 0.0, 0.1, 0.1), bad, 0.0, &t).unwrap_err();
        assert_eq!(e.code(), crate::error::codes::SWEEP_UNSUPPORTED_CORNER);
    }

    // ------------------------------------- the axis-crossing degenerate case

    #[test]
    fn a_profile_touching_the_axis_is_accepted_and_pappus_holds() {
        // CCW arc R = 1; u in [0, 1] reaches radius 0 exactly: a cone-like
        // apex, no overlap. V = A * theta * R_centroid with R_centroid = 0.5.
        let t = tol();
        let theta = PI / 2.0;
        let s = Sweep::new(profile(0.0, 0.0, 1.0, 2.0), arc_path(1.0, theta), 0.0, &t).unwrap();
        near_eps(s.volume(&t).unwrap(), 2.0 * theta * 0.5, 1e-12);
    }

    #[test]
    fn a_profile_crossing_the_axis_is_rejected_with_the_offending_length() {
        // CCW arc R = 1; u in [0.5, 1.5] spans radius 0.5 .. -0.5: half the
        // profile lies beyond the centre.
        let t = tol();
        let theta = PI / 2.0;
        let e = Sweep::new(profile(0.5, 0.0, 1.5, 1.0), arc_path(1.0, theta), 0.0, &t).unwrap_err();
        assert_eq!(e.code(), crate::error::codes::SWEEP_SELF_INTERSECTS);
        match e {
            GeomError::SweepSelfIntersects { segment, min_length } => {
                assert_eq!(segment, 0);
                // The far edge (u = 1.5) is at radius -0.5: swept length
                // theta * (R - u) = (pi/2) * (-0.5).
                near_eps(min_length, theta * (1.0 - 1.5), 1e-12);
            }
            other => panic!("{other:?}"),
        }
    }

    /// Why ASTERION rejects instead of picking a formula. Full revolution of a
    /// rectangle u in [0, 2] (height h) about a CCW circle of radius 1: the
    /// part at u in [1, 2] lies beyond the axis (radius 0 .. -1).
    ///
    /// * signed Pappus  `2 pi * A * R_centroid` = 2 pi * 2h * 0   = **0**
    /// * absolute Pappus = 2 pi * h * 1 = **2 pi h**
    /// * true solid: points at radius rho and -rho trace the same circle, so
    ///   the swept set is the full cylinder of radius 1 = **pi h**
    ///
    /// Three formulas, three answers, and only a boolean union of the two
    /// halves gets the third — whose value moreover changes with the sweep
    /// angle (for theta < pi the halves do not overlap at all). No closed form
    /// is honest here, so the constructor refuses.
    #[test]
    fn axis_crossing_has_no_single_closed_form() {
        let h = 1.0_f64;
        let r = 1.0_f64;
        let (u0, u1) = (0.0_f64, 2.0_f64);
        let area = (u1 - u0) * h;
        // Radius of a profile point on a CCW arc: R - u.
        let r_centroid = r - (u0 + u1) / 2.0;
        let signed = TAU * area * r_centroid;
        // integral of |1 - u| du over [0, 2] = 1.
        let absolute = TAU * h * 1.0;
        let true_volume = PI * r * r * h;
        near(signed, 0.0);
        near(absolute, 2.0 * PI * h);
        near(true_volume, PI * h);
        assert!(signed != true_volume && absolute != true_volume);
        // And the kernel declines to choose.
        let path = vec![
            Curve::Arc(Arc::new(Vec2::new(0.0, 0.0), r, 0.0, PI).unwrap()),
            Curve::Arc(Arc::new(Vec2::new(0.0, 0.0), r, PI, PI).unwrap()),
        ];
        let e = Sweep::new(profile(u0, 0.0, u1, h), path, 0.0, &tol()).unwrap_err();
        assert_eq!(e.code(), crate::error::codes::SWEEP_SELF_INTERSECTS);
    }

    #[test]
    fn a_mitre_too_sharp_for_the_profile_is_rejected_by_the_same_rule() {
        // A 170-degree hairpin: the inner side of each 1 m leg would be swept
        // a negative length by a profile 0.5 m wide.
        let t = tol();
        let back = Vec2::new(1.0 - 10.0_f64.to_radians().cos(), 10.0_f64.to_radians().sin());
        let path = vec![
            Curve::line(Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0)),
            Curve::line(Vec2::new(1.0, 0.0), back),
        ];
        let e = Sweep::new(profile(0.0, 0.0, 0.5, 0.1), path, 0.0, &t).unwrap_err();
        assert_eq!(e.code(), crate::error::codes::SWEEP_SELF_INTERSECTS);
    }

    #[test]
    fn first_moment_matches_hand_values() {
        // Segment from (1,0) to (3,0): len 2, mean x 2 => 4.
        near(Curve::line(Vec2::new(1.0, 0.0), Vec2::new(3.0, 0.0)).first_moment_x(), 4.0);
        // Full circle of radius 2 centred at x = 5: 2 pi r cx = 20 pi.
        let full = Polycurve::circle(Vec2::new(5.0, 0.0), 2.0, &tol()).unwrap();
        let m: f64 = full.curves().iter().map(|c| c.first_moment_x()).sum();
        near_eps(m, 20.0 * PI, 1e-12);
        // Reversing a curve does not change integral x ds (ds is unsigned).
        let a = Arc::new(Vec2::new(1.0, 1.0), 2.0, 0.3, 1.1).unwrap();
        near_eps(Curve::Arc(a).first_moment_x(), Curve::Arc(a.reversed()).first_moment_x(), 1e-12);
    }

    // ------------------------------------------------------------ sloped

    /// The question the module docs raise, answered with numbers: on a 1:12
    /// ramp 6 m long, 1.2 m wide and 0.2 m thick, the two conventions give
    ///
    /// * vertical profile (ASSUMED working convention): `A * L` = 1.44 m3;
    /// * perpendicular profile (the alternative): `A * L * sqrt(1 + 1/144)`
    ///   = 1.444997... m3,
    ///
    /// a 0.35 % difference the harness can resolve. Both exist; the first
    /// sloped-sweep observation decides which the reference uses.
    #[test]
    fn the_two_slope_conventions_give_different_volumes() {
        let t = tol();
        let g = 1.0 / 12.0;
        let path = vec![Curve::line(Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0))];
        let section = profile(-0.6, -0.2, 0.6, 0.0);
        let area = 1.2 * 0.2;
        let vertical = Sweep::new_sloped(
            section.clone(), path.clone(), 0.0, g, SlopeConvention::VerticalProfile, &t,
        )
        .unwrap();
        let perpendicular = Sweep::new_sloped(
            section, path, 0.0, g, SlopeConvention::PerpendicularProfile, &t,
        )
        .unwrap();
        let v_vert = vertical.volume(&t).unwrap();
        let v_perp = perpendicular.volume(&t).unwrap();
        near(v_vert, area * 6.0);
        near(v_perp, area * 6.0 * (1.0 + g * g).sqrt());
        near(v_perp / v_vert, (1.0 + 1.0 / 144.0_f64).sqrt());
        // Measurable: the relative gap is far above the harness's 1e-4.
        assert!((v_perp - v_vert) / v_vert > 3e-3);
        // Both rise 0.5 m over the run.
        near(vertical.end_elevation(), 0.5);
        near(perpendicular.end_elevation(), 0.5);
    }

    #[test]
    fn vertical_profile_ramp_skin_has_inclined_top_and_vertical_sides() {
        // 6 m run at 1:12, profile w = 1.2 wide, t = 0.2 thick. Top and bottom
        // are inclined planes of true area w * L * sqrt(1 + g^2); the sides
        // are vertical parallelograms of area L * t.
        let t = tol();
        let g = 1.0 / 12.0;
        let s = Sweep::new_sloped(
            profile(0.0, 0.0, 1.2, 0.2),
            vec![Curve::line(Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0))],
            0.0,
            g,
            SlopeConvention::VerticalProfile,
            &t,
        )
        .unwrap();
        let expected = 6.0 * (2.0 * 1.2 * (1.0 + g * g).sqrt() + 2.0 * 0.2);
        near_eps(s.lateral_area(&t).unwrap(), expected, 1e-12);
        near(s.cap_area(&t), 2.0 * 1.2 * 0.2);
        let b = s.aabb(&t).unwrap();
        near(b.min.z, 0.0);
        near(b.max.z, 0.2 + 0.5);
    }

    #[test]
    fn a_helical_ramp_has_the_volume_of_its_flat_curved_band() {
        // Quarter-turn ramp, centreline radius 5, 2 m wide, 0.2 m thick,
        // rising 1:12. A vertical shear preserves volume.
        let t = tol();
        let path = arc_path(5.0, PI / 2.0);
        let s = Sweep::new_sloped(
            profile(-1.0, -0.2, 1.0, 0.0),
            path.clone(),
            0.0,
            1.0 / 12.0,
            SlopeConvention::VerticalProfile,
            &t,
        )
        .unwrap();
        let band = offset_band(&path, 1.0, 1.0, &t).unwrap();
        near_eps(s.volume(&t).unwrap(), band.area() * 0.2, 1e-12);
        near_eps(s.volume(&t).unwrap(), 0.5 * (PI / 2.0) * (36.0 - 16.0) * 0.2, 1e-12);
        // A rising path never closes, so a full turn still has caps.
        assert!(!s.is_closed(&t));
    }

    /// The helicoid strip's closed form against an independent numerical
    /// integration (Simpson, 4000 intervals). The numeric value is a TEST
    /// ORACLE only — the kernel itself never integrates numerically.
    #[test]
    fn helicoid_skin_matches_an_independent_numeric_integral() {
        let t = tol();
        let (r, theta, g) = (5.0_f64, PI / 2.0, 0.25_f64);
        // Profile: a single slanted top edge would need a closed profile, so
        // use a triangle with one vertical and two inclined edges.
        let tri = Region::from_loop(
            Polycurve::polygon(
                &[Vec2::new(-1.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(-1.0, 0.5)],
                &t,
            )
            .unwrap(),
        );
        let s = Sweep::new_sloped(
            tri.clone(), arc_path(r, theta), 0.0, g, SlopeConvention::VerticalProfile, &t,
        )
        .unwrap();
        let closed = s.lateral_area(&t).unwrap();

        // Numeric oracle: integrate sqrt(lambda^2 + g^2 c_u^2) over every edge.
        let mut numeric = 0.0;
        for c in tri.outer().curves() {
            let (a, b) = (c.start(), c.end());
            let len = a.distance_to(b);
            let c_u = (b.x - a.x) / len;
            let n = 4000;
            let h = len / n as f64;
            let f = |w: f64| {
                let u = a.x + c_u * w;
                let lam = (r - u) / r; // CCW arc: sigma = +1
                (lam * lam + g * g * c_u * c_u).sqrt()
            };
            let mut acc = f(0.0) + f(len);
            for k in 1..n {
                let w = k as f64 * h;
                acc += if k % 2 == 1 { 4.0 * f(w) } else { 2.0 * f(w) };
            }
            numeric += acc * h / 3.0 * theta * r;
        }
        near_eps(closed, numeric, 1e-9);
    }

    #[test]
    fn sloped_sweeps_state_their_limits_instead_of_approximating() {
        let t = tol();
        // An arc in the PROFILE on a sloped path: the skin is elliptic.
        let round = Region::from_loop(Polycurve::circle(Vec2::new(0.0, 0.5), 0.05, &t).unwrap());
        let rail = Sweep::new_sloped(
            round.clone(),
            vec![Curve::line(Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0))],
            0.9,
            0.6,
            SlopeConvention::VerticalProfile,
            &t,
        )
        .unwrap();
        // The volume is still exact (shear) ...
        near_eps(rail.volume(&t).unwrap(), PI * 0.05 * 0.05 * 4.0, 1e-12);
        // ... the skin is reported, not tessellated.
        let e = rail.lateral_area(&t).unwrap_err();
        assert_eq!(e.code(), crate::error::codes::SWEEP_NO_CLOSED_FORM);
        // The perpendicular convention handles that round rail exactly.
        let perp = Sweep::new_sloped(
            round,
            vec![Curve::line(Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0))],
            0.9,
            0.6,
            SlopeConvention::PerpendicularProfile,
            &t,
        )
        .unwrap();
        near_eps(
            perp.lateral_area(&t).unwrap(),
            2.0 * PI * 0.05 * 4.0 * (1.0 + 0.36_f64).sqrt(),
            1e-12,
        );

        // A mitred corner on a rising path is refused.
        let kinked = vec![
            Curve::line(Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0)),
            Curve::line(Vec2::new(4.0, 0.0), Vec2::new(4.0, 3.0)),
        ];
        let e = Sweep::new_sloped(
            profile(-0.1, 0.0, 0.1, 0.1), kinked, 0.0, 0.1, SlopeConvention::VerticalProfile, &t,
        )
        .unwrap_err();
        assert_eq!(e.code(), crate::error::codes::SWEEP_UNSUPPORTED_CORNER);

        // The perpendicular convention on an arc is refused.
        let e = Sweep::new_sloped(
            profile(-0.1, 0.0, 0.1, 0.1),
            arc_path(5.0, PI / 2.0),
            0.0,
            0.1,
            SlopeConvention::PerpendicularProfile,
            &t,
        )
        .unwrap_err();
        assert_eq!(e.code(), crate::error::codes::SWEEP_CONVENTION_UNSUPPORTED);
    }

    #[test]
    fn perpendicular_profile_bbox_tilts_the_section_back() {
        // A 0.2 x 0.2 square section, path along +X rising at 45 degrees.
        // The section is tilted back by 45 degrees, so its top corner leans
        // towards -X by 0.2 sin 45 and rises only 0.2 cos 45.
        let t = tol();
        let s = Sweep::new_sloped(
            profile(-0.1, 0.0, 0.1, 0.2),
            vec![Curve::line(Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0))],
            0.0,
            1.0,
            SlopeConvention::PerpendicularProfile,
            &t,
        )
        .unwrap();
        let b = s.aabb(&t).unwrap();
        let k = (0.5_f64).sqrt();
        near_eps(b.min.x, -0.2 * k, 1e-12);
        near_eps(b.max.x, 1.0, 1e-12);
        near_eps(b.min.z, 0.0, 1e-12);
        near_eps(b.max.z, 1.0 + 0.2 * k, 1e-12);
        near_eps(b.min.y, -0.1, 1e-12);
        near_eps(b.max.y, 0.1, 1e-12);
        near_eps(s.volume(&t).unwrap(), 0.04 * (2.0_f64).sqrt(), 1e-12);
    }

    #[test]
    fn a_level_sweep_is_unchanged_by_either_convention() {
        let t = tol();
        let path = vec![Curve::line(Vec2::new(0.0, 0.0), Vec2::new(5.0, 0.0))];
        let a = Sweep::new_sloped(
            profile(0.0, 0.0, 0.2, 0.1), path.clone(), 0.0, 0.0, SlopeConvention::VerticalProfile, &t,
        )
        .unwrap();
        let b = Sweep::new_sloped(
            profile(0.0, 0.0, 0.2, 0.1), path, 0.0, 0.0, SlopeConvention::PerpendicularProfile, &t,
        )
        .unwrap();
        near(a.volume(&t).unwrap(), b.volume(&t).unwrap());
        near(a.surface_area(&t).unwrap(), b.surface_area(&t).unwrap());
    }

    #[test]
    fn sweep_metrics_are_bit_reproducible() {
        let t = tol();
        let make = || {
            Sweep::new(
                profile(-0.07, 0.0, 0.13, 0.21),
                vec![
                    Curve::line(Vec2::new(0.0, 0.0), Vec2::new(3.3, 0.0)),
                    Curve::line(Vec2::new(3.3, 0.0), Vec2::new(3.3, 2.7)),
                ],
                1.25,
                &t,
            )
            .unwrap()
        };
        let first = make().volume(&t).unwrap().to_bits();
        for _ in 0..8 {
            assert_eq!(make().volume(&t).unwrap().to_bits(), first);
        }
    }
}
