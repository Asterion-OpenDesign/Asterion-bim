//! Generalised extrusion: a planar `Region` (arcs allowed, holes allowed) swept
//! vertically between two elevations.
//!
//! # Topology convention (compatibility-critical)
//!
//! `ExtrudedRegion` generalises that with `C` = total number of boundary curves
//! across **all** loops (outer plus holes), since each boundary curve
//! contributes exactly one lateral face and one plan corner:
//!
//! | quantity | formula | polygonal single loop |
//! |---|---|---|
//! | vertices | `2C` | `2N` |
//! | faces | `C + 2` | `N + 2` |
//! | edges | `3C` | `3N` |
//!
//! For a single polygonal loop `C == N`, so the measured convention is
//! reproduced **exactly** and the floor parity cannot regress. The extension to
//! arcs (one cylindrical lateral face per arc) and to holes is **ASSUMED**: the
//! reference's counts for curved or pierced slabs have not been measured yet.
//! `asterion-compat` will pin them when the scenarios exist.

use crate::curve::Aabb2;
use crate::error::{GeomError, GeomResult};
use crate::prism::{Aabb, Prism};
use crate::region::Region;
use crate::tolerance::Tolerances;
use crate::vec::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtrudedRegion {
    region: Region,
    z0: f64,
    z1: f64,
}

impl ExtrudedRegion {
    pub fn new(region: Region, z0: f64, z1: f64) -> GeomResult<Self> {
        if !(z1 > z0) {
            return Err(GeomError::NonPositiveExtrusion { z0, z1 });
        }
        Ok(ExtrudedRegion { region, z0, z1 })
    }

    /// Lift the v0.1 `Prism` (single polygonal loop) into the general model,
    /// so both code paths produce the same numbers.
    pub fn from_prism(p: &Prism, tol: &Tolerances) -> GeomResult<Self> {
        let region = Region::from_polygon(&p.footprint, tol)?;
        ExtrudedRegion::new(region, p.z0, p.z1)
    }

    pub fn region(&self) -> &Region {
        &self.region
    }
    pub fn z0(&self) -> f64 {
        self.z0
    }
    pub fn z1(&self) -> f64 {
        self.z1
    }
    pub fn height(&self) -> f64 {
        self.z1 - self.z0
    }

    /// Net plan area (outer minus holes). Exact.
    pub fn plan_area(&self) -> f64 {
        self.region.area()
    }
    /// Outer boundary perimeter — the quantity the reference reports as a slab's `Perimeter` for a single-loop boundary.
    pub fn plan_perimeter(&self) -> f64 {
        self.region.outer_perimeter()
    }
    pub fn volume(&self) -> f64 {
        self.plan_area() * self.height()
    }
    /// Lateral (side) area: every loop generates faces, holes included.
    pub fn lateral_area(&self) -> f64 {
        self.region.total_boundary_length() * self.height()
    }
    pub fn surface_area(&self) -> f64 {
        2.0 * self.plan_area() + self.lateral_area()
    }

    /// Total boundary curves across all loops (`C` above).
    pub fn boundary_curve_count(&self) -> usize {
        self.region.curve_count()
    }
    pub fn face_count(&self) -> usize {
        self.boundary_curve_count() + 2
    }
    /// Face count under the reference's convention: lateral faces that share a
    /// supporting plane are reported as **one** face.
    ///
    /// **OBSERVED** (Section 1 rule 9 plus the round-9 counts): a free-floating opening adds +4 faces, but an opening flush
    /// with the host's edge adds only **+3**, because the edge it interrupts is still a single plane. `face_count` (one
    /// face per boundary curve) returns 10 for that solid; this returns 9, the measured value. Plain prisms are unaffected:
    /// 4 distinct planes + 2 = 6.
    pub fn face_count_merged(&self, tol: &Tolerances) -> usize {
        self.region.distinct_support_count(tol) + 2
    }

    pub fn edge_count(&self) -> usize {
        self.boundary_curve_count() * 3
    }
    pub fn vertex_count(&self) -> usize {
        self.boundary_curve_count() * 2
    }

    /// Explicit vertex list (bottom and top copy of each plan corner), sorted
    /// to be independent of loop order — same ordering rule as `Prism`.
    pub fn vertices(&self) -> Vec<Vec3> {
        let mut v: Vec<Vec3> = self
            .region
            .loops()
            .flat_map(|lp| lp.corner_points())
            .flat_map(|p| [Vec3::from_xy(p, self.z0), Vec3::from_xy(p, self.z1)])
            .collect();
        v.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)).then(a.z.total_cmp(&b.z)));
        v
    }

    /// Exact bounding box: arcs contribute their true extremes, not their
    /// endpoints.
    pub fn aabb(&self) -> Aabb {
        let b: Aabb2 = self.region.aabb();
        let (min, max) = b.extruded(self.z0, self.z1);
        Aabb { min, max }
    }

    pub fn contains_point(&self, p: Vec3) -> bool {
        p.z >= self.z0 && p.z <= self.z1 && self.region.contains_point(Vec2::new(p.x, p.y))
    }

    pub fn has_arcs(&self) -> bool {
        self.region.has_arcs()
    }
    pub fn hole_count(&self) -> usize {
        self.region.holes().len()
    }

    /// Display tessellation of the loops at the given sagitta. Quantities never
    /// go through this.
    pub fn tessellated_loops(&self, max_sagitta: f64) -> Vec<Vec<Vec2>> {
        self.region.loops().map(|l| l.tessellate(max_sagitta)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::{Arc, Curve};
    use crate::region::Polycurve;
    use std::f64::consts::{PI, TAU};

    fn tol() -> Tolerances {
        Tolerances::default()
    }
    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-12, "{a} != {b}");
    }

    fn slab_6x4(thickness: f64) -> ExtrudedRegion {
        let r = Region::from_polygon(
            &[
                Vec2::new(0.0, 0.0),
                Vec2::new(6.0, 0.0),
                Vec2::new(6.0, 4.0),
                Vec2::new(0.0, 4.0),
            ],
            &tol(),
        )
        .unwrap();
        ExtrudedRegion::new(r, -thickness, 0.0).unwrap()
    }

    #[test]
    fn observed_floor_numbers_are_reproduced() {
        // 6x4 slab, t = 0.2 at level 0 => Area 24, Volume 4.8, Perimeter 20, bbox z [-0.2, 0], topology 8 vertices / 6
        // faces / 12 edges.
        let s = slab_6x4(0.2);
        near(s.plan_area(), 24.0);
        near(s.volume(), 4.8);
        near(s.plan_perimeter(), 20.0);
        assert_eq!((s.vertex_count(), s.face_count(), s.edge_count()), (8, 6, 12));
        assert_eq!(s.vertices().len(), 8);
        let b = s.aabb();
        near(b.min.z, -0.2);
        near(b.max.z, 0.0);
    }

    #[test]
    fn observed_topology_convention_for_six_and_three_point_boundaries() {
        let l = Region::from_polygon(
            &[
                Vec2::new(0.0, 0.0),
                Vec2::new(8.0, 0.0),
                Vec2::new(8.0, 3.0),
                Vec2::new(4.0, 3.0),
                Vec2::new(4.0, 6.0),
                Vec2::new(0.0, 6.0),
            ],
            &tol(),
        )
        .unwrap();
        let l = ExtrudedRegion::new(l, 0.0, 0.2).unwrap();
        assert_eq!((l.vertex_count(), l.face_count(), l.edge_count()), (12, 8, 18));
        near(l.plan_area(), 36.0);
        near(l.plan_perimeter(), 28.0);

        let t = Region::from_polygon(
            &[Vec2::new(0.0, 0.0), Vec2::new(5.0, 0.0), Vec2::new(0.0, 4.0)],
            &tol(),
        )
        .unwrap();
        let t = ExtrudedRegion::new(t, 0.0, 0.2).unwrap();
        assert_eq!((t.vertex_count(), t.face_count(), t.edge_count()), (6, 5, 9));
        near(t.volume(), 2.0);
    }

    #[test]
    fn prism_and_extruded_region_agree_on_every_metric() {
        let p = Prism::new(
            vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(5.0, 0.0),
                Vec2::new(5.0, 0.2),
                Vec2::new(0.0, 0.2),
            ],
            0.0,
            3.0,
        );
        let e = ExtrudedRegion::from_prism(&p, &tol()).unwrap();
        near(e.plan_area(), p.footprint_area());
        near(e.plan_perimeter(), p.footprint_perimeter());
        near(e.volume(), p.volume());
        near(e.surface_area(), p.surface_area());
        assert_eq!(e.face_count(), p.face_count());
        assert_eq!(e.edge_count(), p.edge_count());
        assert_eq!(e.vertices().len(), p.vertices().len());
        let (a, b) = (e.aabb(), p.aabb());
        assert_eq!(a.min.to_array(), b.min.to_array());
        assert_eq!(a.max.to_array(), b.max.to_array());
    }

    #[test]
    fn slab_with_one_opening_subtracts_volume_and_adds_faces() {
        let outer = Polycurve::rectangle(Vec2::new(0.0, 0.0), Vec2::new(6.0, 4.0), &tol()).unwrap();
        let hole = Polycurve::rectangle(Vec2::new(1.0, 1.0), Vec2::new(3.0, 2.0), &tol()).unwrap();
        let region = Region::new(outer, vec![hole], &tol()).unwrap();
        let s = ExtrudedRegion::new(region, -0.2, 0.0).unwrap();
        near(s.plan_area(), 24.0 - 2.0);
        near(s.volume(), (24.0 - 2.0) * 0.2);
        near(s.plan_perimeter(), 20.0);
        near(s.lateral_area(), (20.0 + 6.0) * 0.2);
        near(s.surface_area(), 2.0 * 22.0 + 26.0 * 0.2);
        // 8 boundary curves => 16 vertices, 10 faces, 24 edges.
        assert_eq!((s.vertex_count(), s.face_count(), s.edge_count()), (16, 10, 24));
        assert_eq!(s.hole_count(), 1);
        assert!(!s.contains_point(Vec3::new(2.0, 1.5, -0.1)));
        assert!(s.contains_point(Vec3::new(5.0, 3.0, -0.1)));
    }

    #[test]
    fn cylinder_from_a_two_arc_circle_matches_closed_form() {
        let c = Polycurve::circle(Vec2::new(0.0, 0.0), 2.0, &tol()).unwrap();
        let s = ExtrudedRegion::new(Region::from_loop(c), 0.0, 3.0).unwrap();
        near(s.plan_area(), PI * 4.0);
        near(s.volume(), PI * 4.0 * 3.0);
        near(s.plan_perimeter(), TAU * 2.0);
        near(s.surface_area(), 2.0 * PI * 4.0 + TAU * 2.0 * 3.0);
        // Two boundary arcs => 4 vertices, 4 faces, 6 edges under the convention.
        assert_eq!((s.vertex_count(), s.face_count(), s.edge_count()), (4, 4, 6));
        let b = s.aabb();
        near(b.min.x, -2.0);
        near(b.max.y, 2.0);
    }

    #[test]
    fn hollow_tube_volume_is_the_annulus_times_height() {
        let outer = Polycurve::circle(Vec2::new(0.0, 0.0), 1.0, &tol()).unwrap();
        let inner = Polycurve::circle(Vec2::new(0.0, 0.0), 0.5, &tol()).unwrap();
        let s = ExtrudedRegion::new(
            Region::new(outer, vec![inner], &tol()).unwrap(),
            0.0,
            10.0,
        )
        .unwrap();
        near(s.plan_area(), PI * 0.75);
        near(s.volume(), PI * 7.5);
        near(s.lateral_area(), (TAU + PI) * 10.0);
        assert_eq!((s.vertex_count(), s.face_count(), s.edge_count()), (8, 6, 12));
    }

    #[test]
    fn curved_wall_body_volume_matches_the_annular_sector() {
        // A 90-degree curved wall: centreline radius 5, width 0.3, height 3.
        // Faces are concentric arcs at r = 4.85 and r = 5.15.
        // Area of the annular sector = (pi/2 / 2) * (5.15^2 - 4.85^2) = 2.25 * pi / 2.
        let inner = Arc::from_center_angles(Vec2::new(0.0, 0.0), 4.85, 0.0, PI / 2.0, true).unwrap();
        let outer = Arc::from_center_angles(Vec2::new(0.0, 0.0), 5.15, PI / 2.0, 0.0, false).unwrap();
        let lp = Polycurve::closed(
            vec![
                Curve::Arc(inner),
                Curve::line(inner.end(), outer.start()),
                Curve::Arc(outer),
                Curve::line(outer.end(), inner.start()),
            ],
            &tol(),
        )
        .unwrap();
        let s = ExtrudedRegion::new(Region::from_loop(lp), 0.0, 3.0).unwrap();
        let expected_area = 0.5 * (PI / 2.0) * (5.15 * 5.15 - 4.85 * 4.85);
        near(s.plan_area(), expected_area);
        near(s.volume(), expected_area * 3.0);
        // Perimeter: both arcs plus the two 0.3 end caps.
        near(s.plan_perimeter(), (PI / 2.0) * (4.85 + 5.15) + 0.6);
        assert_eq!((s.vertex_count(), s.face_count(), s.edge_count()), (8, 6, 12));
    }

    /// Section 1 rule 9 as pure geometry: the host wall face is 6.000 x 3.000 and the solid is that face region extruded by
    /// the wall thickness. A plain wall reports 6 faces.
    #[test]
    fn measured_face_count_convention_for_openings() {
        let t = tol();
        let host = || Polycurve::rectangle(Vec2::new(0.0, 0.0), Vec2::new(6.0, 3.0), &t).unwrap();

        let plain = ExtrudedRegion::new(Region::from_loop(host()), 0.0, 0.2).unwrap();
        assert_eq!(plain.face_count_merged(&t), 6);
        assert_eq!(plain.face_count(), 6);

        let window = Polycurve::rectangle(Vec2::new(1.0, 0.9), Vec2::new(2.2, 2.1), &t).unwrap();
        let pierced =
            ExtrudedRegion::new(Region::new(host(), vec![window], &t).unwrap(), 0.0, 0.2).unwrap();
        assert_eq!(pierced.face_count_merged(&t), 10);
        assert_eq!(pierced.face_count_merged(&t) - plain.face_count_merged(&t), 4);

        let door = Region::from_loop(
            Polycurve::rectangle(Vec2::new(3.0, 0.0), Vec2::new(3.9, 2.1), &t).unwrap(),
        );
        let notched = Region::from_loop(host()).difference(&door, &t).unwrap();
        assert_eq!(notched.len(), 1);
        assert_eq!(notched[0].curve_count(), 8);
        assert_eq!(notched[0].holes().len(), 0, "a flush opening is a notch, not a hole");
        let with_door = ExtrudedRegion::new(notched[0].clone(), 0.0, 0.2).unwrap();
        assert_eq!(with_door.face_count(), 10, "one face per curve overcounts");
        assert_eq!(with_door.face_count_merged(&t), 9, "the measured value");
        assert_eq!(with_door.face_count_merged(&t) - plain.face_count_merged(&t), 3);
        near(with_door.plan_area(), 18.0 - 0.9 * 2.1);
    }

    /// The window head and the door head are coplanar (both at z = 2.100) but separated by wall material, so they are two
    /// faces; the base stays one face because the gap in it is the door itself. Merging every coplanar pair gave 12.
    #[test]
    fn coplanar_heads_separated_by_material_are_two_faces() {
        let t = tol();
        let host = Region::from_loop(
            Polycurve::rectangle(Vec2::new(0.0, 0.0), Vec2::new(6.0, 3.0), &t).unwrap(),
        );
        let window = Region::from_loop(
            Polycurve::rectangle(Vec2::new(1.0, 0.9), Vec2::new(2.2, 2.1), &t).unwrap(),
        );
        let door = Region::from_loop(
            Polycurve::rectangle(Vec2::new(3.0, 0.0), Vec2::new(3.9, 2.1), &t).unwrap(),
        );
        let cut = crate::boolean::union_all(&[window, door], &t).unwrap();
        assert_eq!(cut.len(), 2);
        let mut face = vec![host];
        for c in &cut {
            face = face[0].difference(c, &t).unwrap();
            assert_eq!(face.len(), 1);
        }
        let solid = ExtrudedRegion::new(face[0].clone(), 0.0, 0.2).unwrap();
        near(solid.plan_area(), 14.670);
        assert_eq!(solid.face_count_merged(&t), 13, "the measured value");
        // And the two previous rows still hold with the corrected rule.
        assert_eq!(solid.face_count(), 14, "one face per curve overcounts by one");
    }

    /// | step | area | faces |
    /// |---|---|---|
    /// | no opening | 18.000 | 6 |
    /// | window | 16.560 | 10 |
    /// | + door (touches the base) | 14.670 | 13 |
    /// | delete the window | 16.110 | 9 |
    /// | overlapping opening | 15.060 | 9 |
    /// | wider than the wall | 10.460 | 11 |
    /// | hanging off the end | 10.460 | 11 |
    /// | above the wall top | 10.340 | 14 |
    /// | 2 mm degenerate | 10.340 | 14 |
    #[test]
    fn every_measured_opening_row_area_and_face_count() {
        use crate::boolean::{total_area, union_all};
        use crate::region::distinct_support_count_all;
        let t = tol();
        let r = |x0: f64, z0: f64, x1: f64, z1: f64| {
            Region::from_loop(
                Polycurve::rectangle(Vec2::new(x0, z0), Vec2::new(x1, z1), &t).unwrap(),
            )
        };
        let host = r(0.0, 0.0, 6.0, 3.0);
        // Cut the host by the union of the openings clipped to it.
        let cut = |openings: &[Region]| -> Vec<Region> {
            let mut clipped: Vec<Region> = Vec::new();
            for o in openings {
                clipped.extend(o.clipped_to(&host, &t).unwrap());
            }
            if clipped.is_empty() {
                return vec![host.clone()];
            }
            let holes = union_all(&clipped, &t).unwrap();
            let mut pieces = vec![host.clone()];
            for h in &holes {
                let mut next: Vec<Region> = Vec::new();
                for p in &pieces {
                    next.extend(p.difference(h, &t).unwrap());
                }
                pieces = next;
            }
            pieces
        };
        let faces = |pieces: &[Region]| distinct_support_count_all(pieces, &t) + 2;

        let window = r(1.0, 0.9, 2.2, 2.1);
        let door = r(3.0, 0.0, 3.9, 2.1);
        let overlap = r(3.5, 0.0, 4.4, 2.1);
        let wide = r(-1.0, 0.5, 7.0, 1.5);
        let hanging = r(5.5, 0.5, 6.5, 1.5);
        let above = r(4.6, 2.8, 5.2, 3.4);
        let tiny = r(5.0, 1.0, 5.002, 1.002);

        let rows: Vec<(&str, Vec<Region>, f64, usize)> = vec![
            ("no opening", vec![], 18.000, 6),
            ("window", vec![window.clone()], 16.560, 10),
            ("+ door", vec![window.clone(), door.clone()], 14.670, 13),
            ("delete the window", vec![door.clone()], 16.110, 9),
            ("overlapping", vec![door.clone(), overlap.clone()], 15.060, 9),
            ("wider than the wall", vec![door.clone(), overlap.clone(), wide.clone()], 10.460, 11),
            (
                "hanging off the end",
                vec![door.clone(), overlap.clone(), wide.clone(), hanging.clone()],
                10.460,
                11,
            ),
            (
                "above the wall top",
                vec![door.clone(), overlap.clone(), wide.clone(), hanging.clone(), above.clone()],
                10.340,
                14,
            ),
            (
                "2 mm degenerate",
                vec![door, overlap, wide, hanging, above, tiny],
                10.340,
                14,
            ),
        ];
        for (label, openings, area, measured_faces) in rows {
            let pieces = cut(&openings);
            let a = total_area(&pieces);
            assert!((a - area).abs() < 1e-9, "{label}: area {a} vs {area}");
            assert_eq!(faces(&pieces), measured_faces, "{label}: face count");
        }
    }

    /// Edge counts of the same measured sequence against `3 * C`, where `C` is the curve count of the cut face region after
    /// union and simplification — what an extruded face region has.
    ///
    /// | step | reference edges | 3C |
    /// |---|---|---|
    /// | no opening | 12 | 12 |
    /// | window | 24 | 24 |
    /// | + door | 36 | 36 |
    /// | delete the window | 24 | 24 |
    /// | overlapping | 24 | 24 |
    /// | wider than the wall | 48 | 48 |
    ///
    /// **Geometry explains the first six exactly.** From "hanging off the end"
    /// on, the reference carries a constant **+4** that no geometry change
    /// accounts for: that opening removes no area (10.460 before and after) and
    /// leaves `C` at 16, yet the reference goes 48 -> 52. The +4 then persists
    /// unchanged, and the later increments agree exactly (above the top: +12 in
    /// both; the floating 2 mm opening: +0 in both). The hanging opening is the
    /// only one in the data whose clipped rectangle adds nothing **but lies on
    /// existing boundary** (z = 0.5 and z = 1.5 over x 5.5..6.0, and the wall
    /// end); the 2 mm one floats inside the void and adds nothing. So the +4
    /// looks like an imprint of an opening coincident with existing edges — a
    /// hypothesis with one data point, recorded here as data and left to the
    /// edge-accounting probe, NOT encoded as a kernel rule.
    #[test]
    fn measured_opening_edge_counts_are_three_c_plus_an_unexplained_imprint() {
        use crate::boolean::union_all;
        let t = tol();
        let r = |x0: f64, z0: f64, x1: f64, z1: f64| {
            Region::from_loop(
                Polycurve::rectangle(Vec2::new(x0, z0), Vec2::new(x1, z1), &t).unwrap(),
            )
        };
        let host = r(0.0, 0.0, 6.0, 3.0);
        let cut = |openings: &[Region]| -> Vec<Region> {
            let mut clipped: Vec<Region> = Vec::new();
            for o in openings {
                clipped.extend(o.clipped_to(&host, &t).unwrap());
            }
            if clipped.is_empty() {
                return vec![host.clone()];
            }
            let mut pieces = vec![host.clone()];
            for h in &union_all(&clipped, &t).unwrap() {
                let mut next: Vec<Region> = Vec::new();
                for p in &pieces {
                    next.extend(p.difference(h, &t).unwrap());
                }
                pieces = next;
            }
            pieces
        };
        let three_c = |pieces: &[Region]| 3 * pieces.iter().map(|p| p.curve_count()).sum::<usize>();

        let window = r(1.0, 0.9, 2.2, 2.1);
        let door = r(3.0, 0.0, 3.9, 2.1);
        let overlap = r(3.5, 0.0, 4.4, 2.1);
        let wide = r(-1.0, 0.5, 7.0, 1.5);
        let hanging = r(5.5, 0.5, 6.5, 1.5);
        let above = r(4.6, 2.8, 5.2, 3.4);
        let tiny = r(5.0, 1.0, 5.002, 1.002);
        // (label, openings, reference edges, residual not explained by 3C)
        let rows: Vec<(&str, Vec<Region>, usize, usize)> = vec![
            ("no opening", vec![], 12, 0),
            ("window", vec![window.clone()], 24, 0),
            ("+ door", vec![window, door.clone()], 36, 0),
            ("delete the window", vec![door.clone()], 24, 0),
            ("overlapping", vec![door.clone(), overlap.clone()], 24, 0),
            ("wider than the wall", vec![door.clone(), overlap.clone(), wide.clone()], 48, 0),
            (
                "hanging off the end",
                vec![door.clone(), overlap.clone(), wide.clone(), hanging.clone()],
                52,
                4,
            ),
            (
                "above the wall top",
                vec![door.clone(), overlap.clone(), wide.clone(), hanging.clone(), above.clone()],
                64,
                4,
            ),
            ("2 mm degenerate", vec![door, overlap, wide, hanging, above, tiny], 64, 4),
        ];
        for (label, openings, reference, residual) in rows {
            let pieces = cut(&openings);
            assert_eq!(three_c(&pieces) + residual, reference, "{label}");
        }
    }

    #[test]
    fn distinct_supports_merge_collinear_and_co_circular_boundaries() {
        let t = tol();
        let notched = Region::from_polygon(
            &[
                Vec2::new(0.0, 0.0),
                Vec2::new(3.0, 0.0),
                Vec2::new(3.0, 2.0),
                Vec2::new(4.0, 2.0),
                Vec2::new(4.0, 0.0),
                Vec2::new(6.0, 0.0),
                Vec2::new(6.0, 3.0),
                Vec2::new(0.0, 3.0),
            ],
            &t,
        )
        .unwrap();
        assert_eq!(notched.curve_count(), 8);
        assert_eq!(notched.distinct_support_count(&t), 7);

        let c = Region::from_loop(Polycurve::circle(Vec2::new(0.0, 0.0), 1.0, &t).unwrap());
        assert_eq!(c.curve_count(), 2);
        assert_eq!(c.distinct_support_count(&t), 1);
        let cyl = ExtrudedRegion::new(c, 0.0, 1.0).unwrap();
        assert_eq!(cyl.face_count_merged(&t), 3, "one side plus top and bottom");
    }

    #[test]
    fn non_positive_extrusion_is_rejected() {
        let r = Region::from_polygon(
            &[Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)],
            &tol(),
        )
        .unwrap();
        let e = ExtrudedRegion::new(r, 1.0, 1.0).unwrap_err();
        assert_eq!(e.code(), crate::error::codes::NON_POSITIVE_EXTRUSION);
    }
}
