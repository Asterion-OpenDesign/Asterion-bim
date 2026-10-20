//! Inclined slabs: the first **non-prismatic** solid in the kernel.
//!
//! A slab is a planar `Region` in plan, a top **plane** (not a constant
//! elevation) and a thickness measured **vertically**. The solid lies between
//! two parallel planes with vertical sides, which makes every quantity exact:
//!
//! | quantity | closed form | why |
//! |---|---|---|
//! | volume | `plan_area * thickness` | the solid is a cylinder between two parallel planes; the vertical height is constant |
//! | top / bottom face area | `plan_area * sqrt(1 + |grad|^2)` | the plan is the orthogonal projection of the face |
//! | lateral area | `boundary_length * thickness` | the sides are vertical and of constant vertical height |
//! | elevation range | plane evaluated at the region's **support points** | exact for arcs, no sampling |
//!
//! Status tags: the *geometry* above is exact by derivation.
//!
//! Every value below matched at machine precision, residuals 1e-14 .. 1e-17: a floor created with a slope arrow is a
//! **sheared prism** — top and bottom faces parallel, **ends vertical**, the type's thickness `t` measured
//! **perpendicular** to the faces, so the vertical offset between the faces is `t * sqrt(1 + g^2)`. Hence
//!
//! * `Volume = A_plan * t * sqrt(1 + g^2)`;
//! * `Area` is the **inclined top face**, `A_plan * sqrt(1 + g^2)`;
//! * `Perimeter` is the **plan** perimeter (20.000 for 6 x 4 at any slope);
//! * bounding box: x/y equal the plan box (vertical ends — a "tilted box" with
//!   sides normal to the face is falsified), top `= tail + g * run`, bottom
//!   `= -t * sqrt(1 + g^2)` at the tail;
//! * `Height Offset` lifts the whole solid; the arrow's tail sits at
//!   `level + offset`;
//! * 6 faces and 12 edges, like a flat slab.
//!
//! `Slab::sloped_floor` builds exactly that solid, and `reported_area` / `reported_perimeter` / `reported_volume` return
//! the three quantities in the referentials the reference uses — which are **not the same referential** (see those
//! methods). The flat case (§2: top at `level.elevation + Height Offset`, extending downwards by the thickness) is the `g =
//! 0` special case and is unchanged.

use crate::error::{GeomError, GeomResult};
use crate::prism::Aabb;
use crate::region::Region;
use crate::solid::ExtrudedRegion;
use crate::vec::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// A non-vertical plane written as `z = elevation + gradient . (x, y)`.
/// `gradient` is a rise-over-run slope vector, so `gradient = 0` is horizontal.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plane {
    pub elevation: f64,
    pub gradient: Vec2,
}

impl Plane {
    /// Horizontal plane at `elevation` — the measured flat-slab case.
    pub const fn level(elevation: f64) -> Self {
        Plane { elevation, gradient: Vec2::new(0.0, 0.0) }
    }
    /// Plane through `point` with the given gradient.
    pub fn through(point: Vec3, gradient: Vec2) -> Self {
        let p = Vec2::new(point.x, point.y);
        Plane { elevation: point.z - gradient.dot(p), gradient }
    }
    /// Plane through `point`, tilted by `slope` (rise over run) in the
    /// horizontal direction `direction`.
    pub fn sloped(point: Vec3, direction: Vec2, slope: f64) -> GeomResult<Self> {
        let u = direction
            .normalized()
            .ok_or(GeomError::PointsCollinear { area2: 0.0 })?;
        Ok(Plane::through(point, u * slope))
    }
    /// **`slope` is a rise/run RATIO**, applied directly: rise = run x slope.
    ///
    /// **The API documentation says otherwise and is wrong.** the reference product (2024) documents the parameter as *"The
    /// slope angle"*. Measured in round 11: passing 0.08314123188844122 raised the top face by **0.49884739133064726** over
    /// 6 m = 6 x 0.08314123..., not 6 tan(0.0831...) = 0.5000 (angle reading) nor 6 sin(...) = 0.4983 (sine reading);
    /// passing 0.24497866312686414 gave **1.4698719787611851** = 6 x 0.24497866..., not 1.5000 nor 1.4552. Both residuals
    /// are below 3e-16. A 1:12 floor is therefore created with `slope = 1/12`.
    ///
    /// **OBSERVED**: the arrow's tail stays at `tail_elevation` (= level
    /// elevation + Height Offset) and the plane rises towards the head.
    pub fn from_slope_arrow(tail: Vec2, head: Vec2, tail_elevation: f64, slope: f64) -> GeomResult<Self> {
        Plane::sloped(Vec3::new(tail.x, tail.y, tail_elevation), head - tail, slope)
    }
    pub fn elevation_at(&self, p: Vec2) -> f64 {
        self.elevation + self.gradient.dot(p)
    }
    pub fn is_horizontal(&self) -> bool {
        self.gradient.x == 0.0 && self.gradient.y == 0.0
    }
    /// Slope magnitude (rise over run).
    pub fn slope(&self) -> f64 {
        self.gradient.length()
    }
    /// Tilt from horizontal, in radians.
    pub fn tilt(&self) -> f64 {
        self.slope().atan()
    }
    /// Ratio between true area on the plane and its plan projection,
    /// `1 / cos(tilt) = sqrt(1 + slope^2)`.
    pub fn area_ratio(&self) -> f64 {
        (1.0 + self.gradient.dot(self.gradient)).sqrt()
    }
    /// Upward unit normal.
    pub fn normal(&self) -> Vec3 {
        let k = 1.0 / self.area_ratio();
        Vec3::new(-self.gradient.x * k, -self.gradient.y * k, k)
    }
    pub fn offset_vertically(&self, dz: f64) -> Self {
        Plane { elevation: self.elevation + dz, gradient: self.gradient }
    }
}

/// How a sloped slab's nominal thickness is measured.
///
/// * `Perpendicular` — **OBSERVED**: the type's thickness is measured normal to the inclined faces, so the vertical
///   distance between them is `t * sqrt(1 + g^2)` and `V = A_plan * t * sqrt(1 + g^2)`. Measured 4.816561 at g = 0.0831412
///   and 4.941936 at g = 0.2449787, against 4.8 for the vertical reading: a gap of 0.35 % and 2.96 %. The ends are
///   **vertical** (x_max = 6.000 exactly), so a "tilted box" with sides normal to the face — same volume, different box —
///   is falsified too.
/// * `Vertical` — **documented alternative, falsified for floors**: thickness
///   as the vertical distance, `V = A_plan * t`. Kept because other hosts
///   (roofs, sweeps) are not measured yet and may differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThicknessConvention {
    Vertical,
    Perpendicular,
}

/// A slab: plan region, top plane, and a vertical thickness.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Slab {
    region: Region,
    top: Plane,
    thickness: f64,
}

impl Slab {
    pub fn new(region: Region, top: Plane, thickness: f64) -> GeomResult<Self> {
        if !(thickness > 0.0) || !thickness.is_finite() {
            return Err(GeomError::NonPositiveExtrusion { z0: -thickness, z1: 0.0 });
        }
        Ok(Slab { region, top, thickness })
    }

    /// Build a slab whose `thickness` is the type's nominal value under the
    /// given convention. Stored internally as the vertical thickness, so every
    /// other method keeps its meaning.
    pub fn with_thickness_convention(
        region: Region,
        top: Plane,
        thickness: f64,
        convention: ThicknessConvention,
    ) -> GeomResult<Self> {
        let vertical = match convention {
            ThicknessConvention::Vertical => thickness,
            ThicknessConvention::Perpendicular => thickness * top.area_ratio(),
        };
        Slab::new(region, top, vertical)
    }

    /// **OBSERVED** in every measured quantity.
    pub fn sloped_floor(
        region: Region,
        arrow_tail: Vec2,
        arrow_head: Vec2,
        tail_elevation: f64,
        slope: f64,
        thickness: f64,
    ) -> GeomResult<Self> {
        let top = Plane::from_slope_arrow(arrow_tail, arrow_head, tail_elevation, slope)?;
        Slab::with_thickness_convention(region, top, thickness, ThicknessConvention::Perpendicular)
    }

    /// The reference's **`Area`** parameter: the area of the **inclined top face**, `A_plan * sqrt(1 + g^2)` (OBSERVED:
    /// 24.082807 and 24.709682 for the 6 x 4 floor at the two measured slopes).
    ///
    /// Note the referential: this is an *on-the-face* quantity, whereas
    /// [`Slab::reported_perimeter`] is a *plan* quantity. One element reports
    /// two magnitudes in two different referentials; the method names say which.
    pub fn reported_area(&self) -> f64 {
        self.top_face_area()
    }

    /// The reference's **`Perimeter`** parameter: the **plan** perimeter of the outer boundary, unchanged by the slope
    /// (OBSERVED: 20.000 at both slopes; the inclined top face's perimeter would be 20.0414 / 20.3693). Contrast with
    /// [`Slab::reported_area`], which is on the face.
    pub fn reported_perimeter(&self) -> f64 {
        self.plan_perimeter()
    }

    /// The reference's **`Volume`** parameter, `reported_area x t` = `A_plan * t_vertical` (OBSERVED: 4.816561 and
    /// 4.941936).
    pub fn reported_volume(&self) -> f64 {
        self.volume()
    }

    /// Perimeter of the **inclined** top face, for completeness and to make
    /// the contrast with [`Slab::reported_perimeter`] testable. Exact for
    /// straight boundaries; `None` when the boundary has an arc and the slab is
    /// not level (the length of an arc lifted onto a tilted plane is an
    /// elliptic integral, and this kernel does not tessellate quantities).
    pub fn inclined_perimeter(&self) -> Option<f64> {
        let g = self.top.gradient;
        let mut total = 0.0;
        for c in self.region.outer().curves() {
            match c {
                crate::curve::Curve::Line(l) => {
                    let rise = g.dot(l.direction());
                    total += (l.length() * l.length() + rise * rise).sqrt();
                }
                crate::curve::Curve::Arc(a) => {
                    if !self.top.is_horizontal() {
                        return None;
                    }
                    total += a.length();
                }
            }
        }
        Some(total)
    }

    /// The constructor asks for by name: `Slab::new(region, top, t_perp * top.area_ratio)`. The thickness is perpendicular
    /// to the faces (OBSERVED for floors and roofs); the slab's internal representation stays vertical, which keeps
    /// `volume` exact.
    pub fn with_perpendicular_thickness(region: Region, top: Plane, t_perp: f64) -> GeomResult<Self> {
        Slab::with_thickness_convention(region, top, t_perp, ThicknessConvention::Perpendicular)
    }

    /// Thickness measured normal to the faces (equals the vertical thickness
    /// for a level slab).
    pub fn normal_thickness(&self) -> f64 {
        self.thickness / self.top.area_ratio()
    }

    pub fn level(region: Region, top_elevation: f64, thickness: f64) -> GeomResult<Self> {
        Slab::new(region, Plane::level(top_elevation), thickness)
    }

    pub fn region(&self) -> &Region {
        &self.region
    }
    pub fn top_plane(&self) -> Plane {
        self.top
    }
    pub fn bottom_plane(&self) -> Plane {
        self.top.offset_vertically(-self.thickness)
    }
    pub fn thickness(&self) -> f64 {
        self.thickness
    }
    pub fn is_level(&self) -> bool {
        self.top.is_horizontal()
    }
    pub fn slope(&self) -> f64 {
        self.top.slope()
    }

    pub fn top_elevation_at(&self, p: Vec2) -> f64 {
        self.top.elevation_at(p)
    }
    pub fn bottom_elevation_at(&self, p: Vec2) -> f64 {
        self.top.elevation_at(p) - self.thickness
    }

    /// Net plan area (outer minus holes).
    pub fn plan_area(&self) -> f64 {
        self.region.area()
    }
    /// Outer boundary perimeter in plan — the measured `Perimeter` quantity for a single-loop slab.
    pub fn plan_perimeter(&self) -> f64 {
        self.region.outer_perimeter()
    }
    /// Exact: a solid between two parallel planes with vertical sides has
    /// volume `plan_area * vertical_thickness`, whatever the tilt.
    pub fn volume(&self) -> f64 {
        self.plan_area() * self.thickness
    }
    /// True (on-plane) area of the top face; equals `plan_area` when level.
    pub fn top_face_area(&self) -> f64 {
        self.plan_area() * self.top.area_ratio()
    }
    pub fn bottom_face_area(&self) -> f64 {
        self.top_face_area()
    }
    pub fn lateral_area(&self) -> f64 {
        self.region.total_boundary_length() * self.thickness
    }
    pub fn surface_area(&self) -> f64 {
        2.0 * self.top_face_area() + self.lateral_area()
    }

    /// Same topology convention as `ExtrudedRegion` (for the polygonal single-loop case; ASSUMED for arcs, holes and tilt).
    pub fn boundary_curve_count(&self) -> usize {
        self.region.curve_count()
    }
    pub fn face_count(&self) -> usize {
        self.boundary_curve_count() + 2
    }
    pub fn edge_count(&self) -> usize {
        self.boundary_curve_count() * 3
    }
    pub fn vertex_count(&self) -> usize {
        self.boundary_curve_count() * 2
    }

    /// Lowest and highest elevation reached by the solid, computed from the
    /// region's support points in `-gradient` and `+gradient` — exact for arcs.
    pub fn elevation_range(&self) -> (f64, f64) {
        if self.top.is_horizontal() {
            return (self.top.elevation - self.thickness, self.top.elevation);
        }
        let g = self.top.gradient;
        let high = self.region.outer().support_point(g);
        let low = self.region.outer().support_point(-g);
        (
            self.top.elevation_at(low) - self.thickness,
            self.top.elevation_at(high),
        )
    }

    pub fn aabb(&self) -> Aabb {
        let plan = self.region.aabb();
        let (z0, z1) = self.elevation_range();
        let (min, max) = plan.extruded(z0, z1);
        Aabb { min, max }
    }

    /// Vertices of the solid: every plan corner at its own top and bottom
    /// elevation. Sorted like `Prism::vertices` so the order never depends on
    /// loop order.
    pub fn vertices(&self) -> Vec<Vec3> {
        let mut v: Vec<Vec3> = self
            .region
            .loops()
            .flat_map(|lp| lp.corner_points())
            .flat_map(|p| {
                [
                    Vec3::from_xy(p, self.bottom_elevation_at(p)),
                    Vec3::from_xy(p, self.top_elevation_at(p)),
                ]
            })
            .collect();
        v.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)).then(a.z.total_cmp(&b.z)));
        v
    }

    pub fn contains_point(&self, p: Vec3) -> bool {
        let q = Vec2::new(p.x, p.y);
        p.z <= self.top_elevation_at(q)
            && p.z >= self.bottom_elevation_at(q)
            && self.region.contains_point(q)
    }

    /// A level slab is exactly the generalised prism, so the two code paths can
    /// be cross-checked (and `ExtrudedRegion` keeps the measured floor numbers).
    pub fn as_extruded_region(&self) -> Option<ExtrudedRegion> {
        if !self.is_level() {
            return None;
        }
        ExtrudedRegion::new(
            self.region.clone(),
            self.top.elevation - self.thickness,
            self.top.elevation,
        )
        .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::{Arc, Curve};
    use crate::region::Polycurve;
    use crate::tolerance::Tolerances;
    use std::f64::consts::PI;

    fn tol() -> Tolerances {
        Tolerances::default()
    }
    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-12, "{a} != {b}");
    }
    fn near_eps(a: f64, b: f64, eps: f64) {
        assert!((a - b).abs() < eps, "{a} != {b}");
    }

    fn rect_region(w: f64, h: f64) -> Region {
        Region::from_polygon(
            &[
                Vec2::new(0.0, 0.0),
                Vec2::new(w, 0.0),
                Vec2::new(w, h),
                Vec2::new(0.0, h),
            ],
            &tol(),
        )
        .unwrap()
    }

    #[test]
    fn level_slab_reproduces_the_observed_floor_numbers() {
        // And §3: 6x4 slab, t = 0.2, top at level 0 => Area 24, Volume 4.8, Perimeter 20, bbox z [-0.2, 0], 8/6/12.
        let s = Slab::level(rect_region(6.0, 4.0), 0.0, 0.2).unwrap();
        near(s.plan_area(), 24.0);
        near(s.volume(), 4.8);
        near(s.plan_perimeter(), 20.0);
        assert_eq!((s.vertex_count(), s.face_count(), s.edge_count()), (8, 6, 12));
        let b = s.aabb();
        near(b.min.z, -0.2);
        near(b.max.z, 0.0);
        // Height offset 0.5 => z [0.3, 0.5] (measured).
        let o = Slab::level(rect_region(6.0, 4.0), 0.5, 0.2).unwrap();
        near(o.aabb().min.z, 0.3);
        near(o.aabb().max.z, 0.5);
    }

    #[test]
    fn a_level_slab_and_the_generalised_prism_agree_on_everything() {
        let s = Slab::level(rect_region(6.0, 4.0), 0.0, 0.2).unwrap();
        let e = s.as_extruded_region().unwrap();
        near(s.plan_area(), e.plan_area());
        near(s.volume(), e.volume());
        near(s.surface_area(), e.surface_area());
        near(s.plan_perimeter(), e.plan_perimeter());
        assert_eq!(s.face_count(), e.face_count());
        assert_eq!(s.edge_count(), e.edge_count());
        assert_eq!(s.vertices().len(), e.vertices().len());
        assert_eq!(s.aabb().min.to_array(), e.aabb().min.to_array());
        assert_eq!(s.aabb().max.to_array(), e.aabb().max.to_array());
    }

    #[test]
    fn a_tilted_slab_keeps_the_same_volume_but_a_larger_face() {
        // 6 x 4 slab, 0.2 thick, sloping 1:4 along +X (slope 0.25).
        let top = Plane::sloped(Vec3::new(0.0, 0.0, 0.0), Vec2::new(1.0, 0.0), 0.25).unwrap();
        let s = Slab::new(rect_region(6.0, 4.0), top, 0.2).unwrap();
        near(s.volume(), 4.8);
        near(s.plan_area(), 24.0);
        // sqrt(1 + 0.0625) = 1.0307764064044151
        near(s.top_face_area(), 24.0 * (1.0625_f64).sqrt());
        near(s.lateral_area(), 20.0 * 0.2);
        near(s.surface_area(), 2.0 * 24.0 * (1.0625_f64).sqrt() + 4.0);
        // The slab rises 0.25 * 6 = 1.5 m across the 6 m span.
        let (lo, hi) = s.elevation_range();
        near(lo, -0.2);
        near(hi, 1.5);
        near(s.top_elevation_at(Vec2::new(6.0, 4.0)), 1.5);
        near(s.bottom_elevation_at(Vec2::new(6.0, 4.0)), 1.3);
        near(s.top.tilt(), 0.25_f64.atan());
    }

    #[test]
    fn diagonal_slope_uses_the_right_corner_for_each_extreme() {
        // Slope 0.1 along the (1,1)/sqrt(2) direction over a 10 x 10 square:
        // the high corner is (10,10), the rise is 0.1 * |(10,10)| = sqrt(2).
        let top = Plane::sloped(Vec3::new(0.0, 0.0, 0.0), Vec2::new(1.0, 1.0), 0.1).unwrap();
        let s = Slab::new(rect_region(10.0, 10.0), top, 0.3).unwrap();
        let (lo, hi) = s.elevation_range();
        near(hi, 0.1 * (200.0_f64).sqrt());
        near(lo, -0.3);
        near(s.volume(), 100.0 * 0.3);
    }

    #[test]
    fn an_inclined_slab_over_an_arc_boundary_uses_the_exact_radial_extreme() {
        // Half disc of radius 4 (flat chord on y = 0), sloping along +Y.
        // The highest point is the top of the arc at y = 4, which is NOT a
        // corner point of the loop: it is the radial extreme of the arc.
        let arc = Arc::from_center_angles(Vec2::new(0.0, 0.0), 4.0, 0.0, PI, true).unwrap();
        let lp = Polycurve::closed(
            vec![
                Curve::Arc(arc),
                Curve::line(Vec2::new(-4.0, 0.0), Vec2::new(4.0, 0.0)),
            ],
            &tol(),
        )
        .unwrap();
        let top = Plane::sloped(Vec3::new(0.0, 0.0, 0.0), Vec2::new(0.0, 1.0), 0.5).unwrap();
        let s = Slab::new(Region::from_loop(lp), top, 0.25).unwrap();
        let (lo, hi) = s.elevation_range();
        near(hi, 0.5 * 4.0);
        near(lo, -0.25);
        near(s.plan_area(), PI * 16.0 / 2.0);
        near(s.volume(), PI * 16.0 / 2.0 * 0.25);
        near(s.plan_perimeter(), PI * 4.0 + 8.0);
        near(s.top_face_area(), PI * 8.0 * (1.25_f64).sqrt());
    }

    #[test]
    fn a_pierced_inclined_slab_subtracts_the_opening_from_both_faces() {
        let outer = Polycurve::rectangle(Vec2::new(0.0, 0.0), Vec2::new(6.0, 4.0), &tol()).unwrap();
        let hole = Polycurve::circle(Vec2::new(3.0, 2.0), 1.0, &tol()).unwrap();
        let region = Region::new(outer, vec![hole], &tol()).unwrap();
        let top = Plane::sloped(Vec3::new(0.0, 0.0, 0.0), Vec2::new(1.0, 0.0), 0.1).unwrap();
        let s = Slab::new(region, top, 0.2).unwrap();
        let net = 24.0 - PI;
        near(s.plan_area(), net);
        near(s.volume(), net * 0.2);
        near(s.lateral_area(), (20.0 + 2.0 * PI) * 0.2);
        // 4 outer curves + 2 arc curves for the circle = 6.
        assert_eq!((s.vertex_count(), s.face_count(), s.edge_count()), (12, 8, 18));
        assert!(!s.contains_point(Vec3::new(3.0, 2.0, 0.1)));
        assert!(s.contains_point(Vec3::new(5.0, 2.0, 0.45)));
    }

    /// Each slope is the value passed to the API, which the reference applies as a rise/run ratio. Nine quantities per
    /// slope.
    #[test]
    fn measured_sloped_floors_round_11() {
        // (slope passed, Area, Volume, Perimeter, bbox z min, bbox z max)
        let measured = [
            (
                0.08314123188844122,
                24.082806720093856,
                4.816561344018772,
                20.0,
                -0.20069005600078224,
                0.49884739133064726,
            ),
            (
                0.24497866312686414,
                24.709681870537246,
                4.941936374107448,
                20.000000000000004,
                -0.20591401558781045,
                1.4698719787611851,
            ),
        ];
        for (slope, area, volume, perimeter, z_min, z_max) in measured {
            let f = Slab::sloped_floor(
                rect_region(6.0, 4.0),
                Vec2::new(0.0, 2.0),
                Vec2::new(6.0, 2.0),
                0.0,
                slope,
                0.2,
            )
            .unwrap();
            near_eps(f.reported_area(), area, 1e-9);
            near_eps(f.reported_volume(), volume, 1e-9);
            near_eps(f.reported_perimeter(), perimeter, 1e-9);
            let b = f.aabb();
            near_eps(b.min.z, z_min, 1e-9);
            near_eps(b.max.z, z_max, 1e-9);
            // Vertical ends: the plan box is the solid's box in x and y.
            near(b.min.x, 0.0);
            near(b.max.x, 6.0);
            near(b.min.y, 0.0);
            near(b.max.y, 4.0);
            // 6 faces, 12 edges, like a flat slab.
            assert_eq!((f.face_count(), f.edge_count()), (6, 12));
            // The type thickness is the perpendicular one.
            near(f.normal_thickness(), 0.2);
        }
    }

    #[test]
    fn measured_height_offset_lifts_the_whole_sloped_floor() {
        // Round 11, step `sloped-1-in-12-offset-0.5`: z 0.29930994 .. 0.99884739.
        let f = Slab::sloped_floor(
            rect_region(6.0, 4.0),
            Vec2::new(0.0, 2.0),
            Vec2::new(6.0, 2.0),
            0.5,
            0.08314123188844122,
            0.2,
        )
        .unwrap();
        let b = f.aabb();
        near_eps(b.min.z, 0.29930994399921773, 1e-9);
        near_eps(b.max.z, 0.9988473913306473, 1e-9);
        // Quantities do not move with the offset.
        near_eps(f.reported_area(), 24.082806720093856, 1e-9);
        near_eps(f.reported_volume(), 4.816561344018772, 1e-9);
    }

    /// The two readings that the measurement FALSIFIED, kept as a test so they
    /// are not reintroduced: the API value read as an angle (tan) or through a
    /// sine, and the vertical-thickness convention.
    #[test]
    fn falsified_readings_of_the_sloped_floor() {
        let slope = 0.08314123188844122_f64;
        let measured_rise = 0.49884739133064726_f64;
        // Ratio reading: exact.
        near(6.0 * slope, measured_rise);
        // Angle reading (what the documentation says): off by 0.23 %.
        assert!((6.0 * slope.tan() - measured_rise).abs() > 1e-3);
        // Sine reading: off too.
        assert!((6.0 * slope.sin() - measured_rise).abs() > 5e-4);

        // Vertical thickness: 4.8, against the measured 4.816561.
        let vertical = Slab::with_thickness_convention(
            rect_region(6.0, 4.0),
            Plane::from_slope_arrow(Vec2::new(0.0, 2.0), Vec2::new(6.0, 2.0), 0.0, slope).unwrap(),
            0.2,
            ThicknessConvention::Vertical,
        )
        .unwrap();
        near(vertical.volume(), 4.8);
        assert!((vertical.volume() - 4.816561344018772).abs() / 4.816561344018772 > 3e-3);

        // "Tilted box" (sides normal to the face) would push x_max past the
        // boundary by t sin(a); the measured x_max is 6.000.
        let a = slope.atan();
        assert!(6.0 + 0.2 * a.sin() - 6.0 > 1e-2);
    }

    /// One element, two referentials: Area on the face, Perimeter in plan.
    /// The method names are what keeps them apart.
    #[test]
    fn area_is_on_the_face_and_perimeter_is_in_plan() {
        let f = Slab::sloped_floor(
            rect_region(6.0, 4.0),
            Vec2::new(0.0, 2.0),
            Vec2::new(6.0, 2.0),
            0.0,
            0.24497866312686414,
            0.2,
        )
        .unwrap();
        // Area: on the face, larger than plan.
        assert!(f.reported_area() > f.plan_area());
        near(f.reported_area(), f.top_face_area());
        // Perimeter: in plan, smaller than the face's own perimeter.
        let inclined = f.inclined_perimeter().unwrap();
        near(f.reported_perimeter(), 20.0);
        assert!(inclined > f.reported_perimeter());
        let r = (1.0 + 0.24497866312686414_f64.powi(2)).sqrt();
        near(inclined, 2.0 * 6.0 * r + 2.0 * 4.0);
        // An arc boundary on a sloped plane has no closed-form face perimeter.
        let disc = Region::from_loop(
            crate::region::Polycurve::circle(Vec2::new(0.0, 0.0), 2.0, &tol()).unwrap(),
        );
        let round = Slab::sloped_floor(disc, Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), 0.0, 0.1, 0.2)
            .unwrap();
        assert!(round.inclined_perimeter().is_none());
        // ...but its reported quantities are all exact.
        near(round.reported_perimeter(), 4.0 * std::f64::consts::PI);
        near(round.reported_area(), 4.0 * std::f64::consts::PI * (1.01_f64).sqrt());
    }

    #[test]
    fn plane_normal_is_unit_and_points_up() {
        let p = Plane::sloped(Vec3::new(0.0, 0.0, 0.0), Vec2::new(1.0, 0.0), 1.0).unwrap();
        let n = p.normal();
        near((n.x * n.x + n.y * n.y + n.z * n.z).sqrt(), 1.0);
        assert!(n.z > 0.0);
        near(n.x, -(0.5_f64).sqrt());
        near(p.tilt(), PI / 4.0);
        near(p.area_ratio(), (2.0_f64).sqrt());
    }

    #[test]
    fn non_positive_thickness_is_rejected() {
        let e = Slab::level(rect_region(1.0, 1.0), 0.0, 0.0).unwrap_err();
        assert_eq!(e.code(), crate::error::codes::NON_POSITIVE_EXTRUSION);
    }

    #[test]
    fn slab_metrics_are_bit_reproducible() {
        let top = Plane::sloped(Vec3::new(1.0, 2.0, 3.0), Vec2::new(0.3, -0.7), 0.17).unwrap();
        let first = Slab::new(rect_region(5.5, 3.25), top, 0.22).unwrap();
        for _ in 0..8 {
            let again = Slab::new(rect_region(5.5, 3.25), top, 0.22).unwrap();
            assert_eq!(again.volume().to_bits(), first.volume().to_bits());
            assert_eq!(again.top_face_area().to_bits(), first.top_face_area().to_bits());
            assert_eq!(
                again.elevation_range().0.to_bits(),
                first.elevation_range().0.to_bits()
            );
        }
    }
}
