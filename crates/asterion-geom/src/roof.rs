//! Footprint roofs: a plan footprint whose edges may each "define slope".
//!
//! Architecture first (founder, 2026-10-13): this is the geometry behind the
//! reference's footprint roof — single-pitch, gable and hip roofs over a
//! convex footprint — computed exactly.
//!
//! # Model
//!
//! Every edge that defines slope carries a **plane** through the edge's line at
//! the eave elevation, rising into the footprint at that edge's slope (a
//! rise/run ratio — the reference's API stores slopes as ratios, measured on
//! floors in round 11). The roof's reference surface is the **lower envelope**
//! of those planes: at each plan point the lowest one wins. The plan region
//! where plane `i` wins is **facet `i`**.
//!
//! On a **convex** footprint every facet is the footprint clipped by the
//! half-planes `plane_i(p) <= plane_j(p)` — each a *linear* inequality, so the
//! facets are convex polygons obtained by exact line clipping, no
//! tessellation. For equal slopes this lower envelope is the straight-skeleton
//! roof (hip roofs, gables). On a non-convex footprint an edge's infinite plane
//! can undercut regions it should not reach, and the true straight skeleton is
//! needed; that case is **rejected** (`GEOM_ROOF_FOOTPRINT_NOT_CONVEX`) rather
//! than answered wrongly.
//!
//! Edges that do not define slope contribute no plane: a gable end. With no
//! sloped edge at all the roof is flat, one facet at the eave elevation.
//!
//! # Quantities, and what is measured versus assumed
//!
//! Each facet `i` has plan area `A_i` and slope `g_i`; its face area is
//! `A_i * sqrt(1 + g_i^2)`. The roof's totals are sums over facets. Two
//! referential questions are **open until the round-10 roof re-measurement
//! lands**, so both answers are exposed under explicit names rather than one
//! ambiguous "area":
//!
//! * area: `plan_area` versus `inclined_area`;
//! * volume: `volume_vertical_thickness` (`A_plan * t`) versus
//!   `volume_perpendicular_thickness` (`sum A_i t sqrt(1 + g_i^2)`).
//!
//! `Area` is the **inclined** area summed over facets — exact to 1e-14 **including unequal slopes**, which validates the
//! lower-envelope facet partition; with **equal** slopes `Volume = Area * t` (perpendicular thickness, like floors) and the
//! top is `ridge + t sqrt(1 + g^2)`: single pitch 26.832816 / 8.049845 / 2.335410, gable 26.832816 / 8.049845 / 1.335410,
//! slope 0.25 written through the instance parameter 24.738634 / 7.421590 / 1.309233. The flat control reproduces 24.0 /
//! 7.2 with its top at 0.3.
//!
//! **The join between facets of different slopes** is resolved in `asterion_bim::roof` (Spec R-01b2, R-04b; OBSERVED): the
//! vertical separation of the shells is `t / cos(a_min)` of the chain of consecutive slope-defining edges, the top is the
//! envelope of the planes lifted by it and the steeper facets' undersides sit lower with a flat floor at the base. This
//! module (the exact equal-slope facet model) still answers `GEOM_ROOF_UNEQUAL_SLOPE_JOIN` for unequal slopes and is not
//! what the engine's regeneration calls.

use crate::error::{GeomError, GeomResult};
use crate::region::{polygon_signed_area, Region};
use crate::slab::Plane;
use crate::tolerance::Tolerances;
use crate::vec::{Vec2, Vec3};

/// One footprint edge's roof data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoofEdge {
    pub defines_slope: bool,
    /// Rise over run, perpendicular to the edge, into the footprint.
    pub slope: f64,
}

impl RoofEdge {
    pub const FLAT: RoofEdge = RoofEdge { defines_slope: false, slope: 0.0 };
    pub fn sloped(slope: f64) -> Self {
        RoofEdge { defines_slope: true, slope }
    }
}

/// A plan region of the roof governed by one plane.
#[derive(Clone, Debug, PartialEq)]
pub struct RoofFacet {
    /// Index of the footprint edge whose plane governs this facet; `None` for
    /// the single facet of a flat roof.
    pub edge: Option<usize>,
    pub plane: Plane,
    pub region: Region,
}

impl RoofFacet {
    pub fn plan_area(&self) -> f64 {
        self.region.area()
    }
    pub fn inclined_area(&self) -> f64 {
        self.plan_area() * self.plane.area_ratio()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FootprintRoof {
    /// Counter-clockwise footprint corners.
    corners: Vec<Vec2>,
    edges: Vec<RoofEdge>,
    eave_elevation: f64,
}

impl FootprintRoof {
    /// `corners` in either orientation (canonicalised to counter-clockwise);
    /// `edges[i]` belongs to the edge from `corners[i]` to `corners[i + 1]`
    /// **of the input order**.
    pub fn new(corners: &[Vec2], edges: &[RoofEdge], eave_elevation: f64, tol: &Tolerances) -> GeomResult<Self> {
        if corners.len() < 3 || edges.len() != corners.len() {
            return Err(GeomError::LoopTooFewCurves { count: corners.len(), minimum: 3 });
        }
        // Validate the loop (closure, short edges) through the usual path.
        crate::region::Polycurve::polygon(corners, tol)?;
        let mut c = corners.to_vec();
        let mut e = edges.to_vec();
        if polygon_signed_area(&c) < 0.0 {
            // Reverse the corners; edge i (c[i] -> c[i+1]) becomes the edge
            // between the reversed neighbours, so remap the edge data too.
            let n = c.len();
            c.reverse();
            let original = e.clone();
            for k in 0..n {
                // reversed edge k runs c'[k] -> c'[k+1] = c[n-1-k] -> c[n-2-k],
                // which is original edge (n - 2 - k) mod n.
                e[k] = original[(2 * n - 2 - k) % n];
            }
        }
        let roof = FootprintRoof { corners: c, edges: e, eave_elevation };
        if roof.edges.iter().any(|x| x.defines_slope) && !roof.is_convex(tol) {
            return Err(GeomError::RoofFootprintNotConvex);
        }
        Ok(roof)
    }

    pub fn eave_elevation(&self) -> f64 {
        self.eave_elevation
    }
    pub fn corners(&self) -> &[Vec2] {
        &self.corners
    }

    fn is_convex(&self, tol: &Tolerances) -> bool {
        let n = self.corners.len();
        (0..n).all(|i| {
            let a = self.corners[i];
            let b = self.corners[(i + 1) % n];
            let c = self.corners[(i + 2) % n];
            (b - a).cross(c - b) >= -tol.vertex * (b - a).length().max(1.0)
        })
    }

    /// The plane of a sloped edge: through the edge's line at the eave,
    /// rising perpendicular to the edge into the footprint.
    pub fn edge_plane(&self, i: usize) -> GeomResult<Plane> {
        let n = self.corners.len();
        let a = self.corners[i];
        let b = self.corners[(i + 1) % n];
        // Counter-clockwise footprint: the inside is to the left of a -> b.
        let inward = (b - a).perp_ccw();
        let slope = if self.edges[i].defines_slope { self.edges[i].slope } else { 0.0 };
        Plane::sloped(Vec3::new(a.x, a.y, self.eave_elevation), inward, slope)
    }

    /// The facets of the lower envelope, in edge order.
    pub fn facets(&self, tol: &Tolerances) -> GeomResult<Vec<RoofFacet>> {
        let sloped: Vec<usize> = (0..self.edges.len()).filter(|&i| self.edges[i].defines_slope).collect();
        if sloped.is_empty() {
            return Ok(vec![RoofFacet {
                edge: None,
                plane: Plane::level(self.eave_elevation),
                region: Region::from_polygon(&self.corners, tol)?,
            }]);
        }
        let planes: Vec<(usize, Plane)> =
            sloped.iter().map(|&i| self.edge_plane(i).map(|p| (i, p))).collect::<GeomResult<_>>()?;
        let mut out: Vec<RoofFacet> = Vec::new();
        for (i, pi) in &planes {
            let mut poly = self.corners.clone();
            for (j, pj) in &planes {
                if i == j {
                    continue;
                }
                // Keep plane_i(p) <= plane_j(p), i.e. a . p <= b with
                // a = grad_i - grad_j and b = elev_j - elev_i.
                let a = pi.gradient - pj.gradient;
                let b = pj.elevation - pi.elevation;
                poly = clip_half_plane(&poly, a, b);
                if poly.len() < 3 {
                    break;
                }
            }
            if poly.len() < 3 || polygon_signed_area(&poly).abs() <= tol.vertex * tol.vertex {
                continue;
            }
            let poly = dedupe(&poly, tol.vertex);
            if poly.len() < 3 {
                continue;
            }
            match Region::from_polygon(&poly, tol) {
                Ok(region) => out.push(RoofFacet { edge: Some(*i), plane: *pi, region }),
                // A sliver facet below ShortCurveTolerance is not geometry.
                Err(GeomError::CurveTooShort { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    /// Height of the reference surface above the eave at plan point `p`
    /// (the lower envelope of the sloped planes; the eave elevation when flat).
    pub fn surface_elevation_at(&self, p: Vec2) -> GeomResult<f64> {
        let sloped: Vec<usize> = (0..self.edges.len()).filter(|&i| self.edges[i].defines_slope).collect();
        if sloped.is_empty() {
            return Ok(self.eave_elevation);
        }
        let mut z = f64::INFINITY;
        for i in sloped {
            z = z.min(self.edge_plane(i)?.elevation_at(p));
        }
        Ok(z)
    }

    /// Highest point of the reference surface: the maximum of the lower
    /// envelope, attained at a facet vertex (each facet is a convex polygon on
    /// one plane, so its maximum is at a corner). Exact.
    pub fn ridge_height(&self, tol: &Tolerances) -> GeomResult<f64> {
        let mut best = f64::NEG_INFINITY;
        for f in self.facets(tol)? {
            for p in f.region.outer().corner_points() {
                best = best.max(f.plane.elevation_at(p));
            }
        }
        Ok(best)
    }

    pub fn plan_area(&self) -> f64 {
        polygon_signed_area(&self.corners).abs()
    }
    pub fn plan_perimeter(&self) -> f64 {
        crate::region::polygon_perimeter(&self.corners)
    }
    /// Sum over facets of the facet's true (on-plane) area.
    pub fn inclined_area(&self, tol: &Tolerances) -> GeomResult<f64> {
        Ok(self.facets(tol)?.iter().map(|f| f.inclined_area()).sum())
    }
    /// `A_plan * t`: thickness measured vertically.
    pub fn volume_vertical_thickness(&self, thickness: f64) -> f64 {
        self.plan_area() * thickness
    }
    /// `Area * t` with the thickness perpendicular to the facets. **OBSERVED**
    /// exact for equal slopes (round 10: 8.049845, 7.421590). For facets of
    /// different slopes the join is not resolved (module docs) and this
    /// returns `GEOM_ROOF_UNEQUAL_SLOPE_JOIN`.
    pub fn volume_perpendicular_thickness(&self, thickness: f64, tol: &Tolerances) -> GeomResult<f64> {
        self.require_equal_slopes()?;
        Ok(self.inclined_area(tol)? * thickness)
    }

    /// Highest point of the roof solid: the bottom ridge plus the vertical
    /// thickness `t sqrt(1 + g^2)`. **OBSERVED** exact for equal slopes
    /// (2.335410 single pitch, 1.335410 gable, 1.309233 at 0.25); a flat roof
    /// tops out at `eave + t` (0.3). Unequal slopes: unresolved, error.
    pub fn top_elevation_max(&self, thickness: f64, tol: &Tolerances) -> GeomResult<f64> {
        let g = self.require_equal_slopes()?;
        Ok(self.ridge_height(tol)? + thickness * (1.0 + g * g).sqrt())
    }

    /// The roof truncated by a horizontal **cut-off** plane at absolute
    /// elevation `cut_z`.
    ///
    /// Base 3.5, slope 0.5 on one edge, 6 x 4:
    ///
    /// * the reference clips the **solid**, not the area: cut at 4.5 gives
    ///   `Area` 13.416407864998723 — the underside up to the cut,
    ///   6 x 2 x sqrt(1.25) — and `Volume` **3.3499223594996184**, whereas
    ///   `Area * t` would be 4.0249. The identity `Volume = Area * t` breaks
    ///   under a cut, so the volume is derived from the solid here;
    /// * with t = 0.5 the same cut gives 4.833203932499363;
    /// * with the cut 0.5 above the base and t = 0.5 (vertical thickness
    ///   0.559 > 0.5) the top face disappears entirely and the roof becomes a
    ///   wedge: `Area` 6.708203932499367, `Volume` 1.5 (5 faces, 9 edges);
    /// * a cut above the whole roof has no effect;
    /// * a cut **at or below** the underside's lowest point is **refused**
    ///   ("Cutoff Level cannot be below bottom of roof", posted at commit) —
    ///   including the case where the cut is exactly level with the base
    ///   (raising L1 to 4.0 put the base at 4.5 = the cut, refused).
    ///
    /// Exact by construction. With `P` the underside plane of a facet and (a clipped polygon's area times `c -
    /// P(centroid)`), the clipped thickness `min(P + h, cut) - P` integrates to `F(cut) - F(cut - h)`, where `h = t sqrt(1
    /// + g^2)` is the vertical thickness. One expression covers the partial cut, the wedge and the no-op.
    ///
    /// Requires equal slopes (the top surface of an unequal-slope join is not
    /// resolved).
    pub fn cut_off(&self, cut_z: f64, thickness: f64, tol: &Tolerances) -> GeomResult<CutOffRoof> {
        if cut_z <= self.eave_elevation + tol.vertex {
            return Err(GeomError::RoofCutOffNotAboveBottom);
        }
        let g = self.require_equal_slopes()?;
        let h = thickness * (1.0 + g * g).sqrt();
        let mut area = 0.0;
        let mut volume = 0.0;
        for f in self.facets(tol)? {
            let poly = f.region.outer().corner_points();
            area += below(&poly, &f.plane, cut_z, tol).0 * f.plane.area_ratio();
            volume += integral_below(&poly, &f.plane, cut_z, tol)
                - integral_below(&poly, &f.plane, cut_z - h, tol);
        }
        let top = self.ridge_height(tol)? + h;
        Ok(CutOffRoof { area, volume, z_min: self.eave_elevation, z_max: top.min(cut_z) })
    }

    /// The common slope of every slope-defining edge (0 for a flat roof), or
    /// the unresolved-join error.
    fn require_equal_slopes(&self) -> GeomResult<f64> {
        let slopes: Vec<f64> =
            self.edges.iter().filter(|e| e.defines_slope).map(|e| e.slope).collect();
        let Some(&first) = slopes.first() else {
            return Ok(0.0);
        };
        if slopes.iter().all(|g| (g - first).abs() <= 1e-12 * first.abs().max(1.0)) {
            Ok(first)
        } else {
            Err(GeomError::RoofUnequalSlopeJoin)
        }
    }
}

/// Quantities of a footprint roof truncated by a horizontal cut-off plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutOffRoof {
    /// The reference's `Area`: the **underside** (inclined) up to the cut.
    pub area: f64,
    /// Volume of the solid clipped by the plane — NOT `area * t`.
    pub volume: f64,
    pub z_min: f64,
    pub z_max: f64,
}

/// Clip a convex polygon to the half-plane `a . p <= b` (Sutherland-Hodgman,
/// one edge). Exact line intersections; corners on the line are kept.
fn clip_half_plane(poly: &[Vec2], a: Vec2, b: f64) -> Vec<Vec2> {
    let n = poly.len();
    if n == 0 {
        return Vec::new();
    }
    let side = |p: Vec2| a.dot(p) - b;
    let mut out: Vec<Vec2> = Vec::with_capacity(n + 1);
    for k in 0..n {
        let cur = poly[k];
        let nxt = poly[(k + 1) % n];
        let (sc, sn) = (side(cur), side(nxt));
        if sc <= 0.0 {
            out.push(cur);
        }
        if (sc < 0.0 && sn > 0.0) || (sc > 0.0 && sn < 0.0) {
            let t = sc / (sc - sn);
            out.push(cur.lerp(nxt, t));
        }
    }
    out
}

/// Plan area and centroid of the part of a convex facet where `plane <= c`.
fn below(poly: &[Vec2], plane: &Plane, c: f64, tol: &Tolerances) -> (f64, Vec2) {
    let clipped = dedupe(&clip_half_plane(poly, plane.gradient, c - plane.elevation), tol.vertex);
    if clipped.len() < 3 {
        return (0.0, Vec2::new(0.0, 0.0));
    }
    let a = polygon_signed_area(&clipped).abs();
    if a <= 0.0 {
        return (0.0, Vec2::new(0.0, 0.0));
    }
    // Polygon centroid (exact for the convex clipped facet).
    let n = clipped.len();
    let mut cx = 0.0;
    let mut cy = 0.0;
    let mut a2 = 0.0;
    for i in 0..n {
        let p = clipped[i];
        let q = clipped[(i + 1) % n];
        let cr = p.cross(q);
        a2 += cr;
        cx += (p.x + q.x) * cr;
        cy += (p.y + q.y) * cr;
    }
    (a, Vec2::new(cx / (3.0 * a2), cy / (3.0 * a2)))
}

/// On a convex facet. The integrand is linear, so it equals the area times its value at the centroid.
fn integral_below(poly: &[Vec2], plane: &Plane, c: f64, tol: &Tolerances) -> f64 {
    let (a, centroid) = below(poly, plane, c, tol);
    if a == 0.0 {
        0.0
    } else {
        a * (c - plane.elevation_at(centroid))
    }
}

fn dedupe(poly: &[Vec2], eps: f64) -> Vec<Vec2> {
    let mut out: Vec<Vec2> = Vec::with_capacity(poly.len());
    for p in poly {
        if out.last().map_or(true, |q: &Vec2| q.distance_to(*p) > eps) {
            out.push(*p);
        }
    }
    while out.len() > 1 && out[0].distance_to(*out.last().unwrap()) <= eps {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tol() -> Tolerances {
        Tolerances::default()
    }
    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-12, "{a} != {b}");
    }
    /// The 6 x 4 footprint. Edge 0 runs (0,0) -> (6,0); edges go counter-clockwise.
    fn footprint() -> Vec<Vec2> {
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(6.0, 0.0),
            Vec2::new(6.0, 4.0),
            Vec2::new(0.0, 4.0),
        ]
    }
    fn roof(edges: [RoofEdge; 4]) -> FootprintRoof {
        FootprintRoof::new(&footprint(), &edges, 0.0, &tol()).unwrap()
    }
    const F: RoofEdge = RoofEdge::FLAT;

    /// Round 10 re-measured, every step with a committed geometry. Equal-slope steps: area, volume and top all reproduced.
    /// Values at 1e-9 (agreement is ~1e-14).
    #[test]
    fn measured_round_10_footprint_roofs() {
        let t = tol();
        let th = 0.3;
        let close = |a: f64, b: f64| assert!((a - b).abs() < 1e-9, "{a} != {b}");
        // (label, edges, Area, Volume, top z)
        let rows: Vec<(&str, [RoofEdge; 4], f64, f64, f64)> = vec![
            ("flat-roof-6x4-t300", [F; 4], 23.999999999999996, 7.199999999999997, 0.3),
            (
                "slope-0.5-edge-0",
                [RoofEdge::sloped(0.5), F, F, F],
                26.83281572999747,
                8.049844718999237,
                2.3354101966249683,
            ),
            (
                "gable-two-opposite-edges",
                [RoofEdge::sloped(0.5), F, RoofEdge::sloped(0.5), F],
                26.832815729997478,
                8.049844718999237,
                1.335410196624968,
            ),
            (
                "slope-written-through-instance-param",
                [RoofEdge::sloped(0.25), F, F, F],
                24.738633753705958,
                7.4215901261117825,
                1.3092329219213241,
            ),
        ];
        for (label, edges, area, volume, top) in rows {
            let r = roof(edges);
            close(r.inclined_area(&t).unwrap(), area);
            close(r.volume_perpendicular_thickness(th, &t).unwrap(), volume);
            close(r.top_elevation_max(th, &t).unwrap(), top);
            let _ = label;
        }
    }

    /// The unequal-slope hip of round 10: edge 0 at 0.25 (written through the
    /// instance parameter), edges 1-3 at 0.5. The AREA is reproduced exactly;
    /// the volume and top are not reproduced by either candidate join, so the
    /// kernel refuses them. The measured values and both candidates are kept
    /// here as data for the probe that will settle the join.
    #[test]
    fn measured_unequal_slope_hip_area_exact_join_unresolved() {
        let t = tol();
        let th = 0.3_f64;
        let r = roof([RoofEdge::sloped(0.25), RoofEdge::sloped(0.5), RoofEdge::sloped(0.5), RoofEdge::sloped(0.5)]);
        // Area: exact.
        assert!((r.inclined_area(&t).unwrap() - 25.74694359414261).abs() < 1e-9);
        // Bottom ridge: plane_0 = 0.25 y meets plane_2 = 0.5 (4 - y) at y = 8/3.
        near(r.ridge_height(&t).unwrap(), 2.0 / 3.0);
        // Volume and top: refused.
        assert_eq!(
            r.volume_perpendicular_thickness(th, &t).unwrap_err().code(),
            crate::error::codes::ROOF_UNEQUAL_SLOPE_JOIN
        );
        assert!(r.top_elevation_max(th, &t).is_err());
        // The data the refusal is based on.
        let measured_volume = 7.719688284366454_f64;
        let measured_top = 0.9758995885879913_f64;
        let per_facet = r.inclined_area(&t).unwrap() * th; // 7.724083
        assert!((per_facet - measured_volume).abs() > 4e-3);
        // Hint: bottom ridge + the SHALLOWER facet's vertical thickness.
        near(2.0 / 3.0 + th * (1.0 + 0.25_f64 * 0.25).sqrt(), measured_top);
    }

    /// Round 10: the measured cut-off steps, exact values from the observation file.
    #[test]
    fn measured_cut_off_round_10() {
        let t = tol();
        let close = |a: f64, b: f64| assert!((a - b).abs() < 1e-9, "{a} != {b}");
        // Roof on L1 (3.0) with base offset 0.5 => eave at 3.5.
        let r = FootprintRoof::new(&footprint(), &[RoofEdge::sloped(0.5), F, F, F], 3.5, &t).unwrap();

        // Uncut (and cut above the roof): the plain sloped roof.
        close(r.inclined_area(&t).unwrap(), 26.83281572999748);
        close(r.top_elevation_max(0.3, &t).unwrap(), 5.835410196624971);
        let above = r.cut_off(6.0, 0.3, &t).unwrap();
        close(above.area, 26.83281572999748);
        close(above.volume, 8.049844718999251);
        close(above.z_max, 5.835410196624971);

        // Cut at 4.5 crossing the roof: Area is the underside up to the cut,
        // Volume is the clipped solid - and NOT Area * t.
        let c = r.cut_off(4.5, 0.3, &t).unwrap();
        close(c.area, 13.416407864998723);
        close(c.volume, 3.3499223594996184);
        close(c.z_min, 3.5);
        close(c.z_max, 4.5);
        assert!((c.area * 0.3 - c.volume).abs() > 0.6, "Area x t must not be the volume");

        // Same cut, type thickened to 0.5.
        close(r.cut_off(4.5, 0.5, &t).unwrap().volume, 4.833203932499363);

        // Cut-off level deleted: the cut becomes 4.0, and with t = 0.5 the
        // top face disappears - a wedge.
        let w = r.cut_off(4.0, 0.5, &t).unwrap();
        close(w.area, 6.708203932499367);
        close(w.volume, 1.4999999999999984);
        close(w.z_max, 4.0);

        // Refusals: below the whole roof, and exactly level with the base.
        assert_eq!(
            r.cut_off(2.0, 0.3, &t).unwrap_err().code(),
            crate::error::codes::ROOF_CUT_OFF_NOT_ABOVE_BOTTOM
        );
        let raised = FootprintRoof::new(&footprint(), &[RoofEdge::sloped(0.5), F, F, F], 4.5, &t).unwrap();
        assert!(raised.cut_off(4.5, 0.3, &t).is_err(), "a cut level with the base is refused");
    }

    #[test]
    fn cut_off_of_a_flat_roof_and_of_a_gable() {
        let t = tol();
        // Flat roof 0.3 thick, cut 0.2 above the base: a 0.2 slab.
        let flat = roof([F; 4]);
        let c = flat.cut_off(0.2, 0.3, &t).unwrap();
        near(c.volume, 24.0 * 0.2);
        near(c.area, 24.0);
        // Gable at 0.5: ridge 1.0. Cut at 0.5: each facet keeps the band up to
        // 1 m from its eave (underside), i.e. half its plan area.
        let gable = roof([RoofEdge::sloped(0.5), F, RoofEdge::sloped(0.5), F]);
        let g = gable.cut_off(0.5, 0.3, &t).unwrap();
        near(g.area, 2.0 * 6.0 * 1.0 * (1.25_f64).sqrt());
        // Volume never exceeds the uncut one and never exceeds Area * t.
        assert!(g.volume < gable.volume_perpendicular_thickness(0.3, &t).unwrap());
        assert!(g.volume < g.area * 0.3);
    }

    #[test]
    fn flat_roof_is_one_level_facet() {
        let r = roof([F; 4]);
        let f = r.facets(&tol()).unwrap();
        assert_eq!(f.len(), 1);
        near(r.plan_area(), 24.0);
        near(r.inclined_area(&tol()).unwrap(), 24.0);
        near(r.volume_vertical_thickness(0.3), 7.2);
        near(r.volume_perpendicular_thickness(0.3, &tol()).unwrap(), 7.2);
        near(r.plan_perimeter(), 20.0);
        near(r.ridge_height(&tol()).unwrap(), 0.0);
    }

    /// The decisive step of the round-10 scenario, `slope-0.5-edge-0`, under
    /// the scenario's own three models plus the one round 11 suggests.
    #[test]
    fn single_pitch_from_edge_0_matches_the_scenario_models() {
        let t = tol();
        let r = roof([RoofEdge::sloped(0.5), F, F, F]);
        let k = (1.25_f64).sqrt();
        assert_eq!(r.facets(&t).unwrap().len(), 1);
        near(r.plan_area(), 24.0);
        near(r.inclined_area(&t).unwrap(), 24.0 * k); // 26.8328
        near(r.volume_vertical_thickness(0.3), 7.2); // models A, B
        near(r.volume_perpendicular_thickness(0.3, &t).unwrap(), 24.0 * k * 0.3); // model C: 8.0498
        near(r.plan_perimeter(), 20.0); // round-11 analogy (model "D")
        // The scenario's B/C perimeter: the two 4 m sides lengthen.
        near(2.0 * 6.0 + 2.0 * 4.0 * k, 20.94427190999916);
        // Rises 0.5 x 4 = 2.0 to the far edge.
        near(r.ridge_height(&t).unwrap(), 2.0);
        near(r.surface_elevation_at(Vec2::new(3.0, 2.0)).unwrap(), 1.0);
    }

    #[test]
    fn gable_from_two_opposite_edges_has_its_ridge_at_mid_span() {
        let t = tol();
        let r = roof([RoofEdge::sloped(0.5), F, RoofEdge::sloped(0.5), F]);
        let facets = r.facets(&t).unwrap();
        assert_eq!(facets.len(), 2);
        for f in &facets {
            near(f.plan_area(), 12.0);
        }
        // Equal slopes: the totals do not depend on the split ...
        near(r.inclined_area(&t).unwrap(), 24.0 * (1.25_f64).sqrt());
        // ... the ridge does: 0.5 x 2 = 1.0, half the single-pitch height.
        near(r.ridge_height(&t).unwrap(), 1.0);
        near(r.surface_elevation_at(Vec2::new(3.0, 2.0)).unwrap(), 1.0);
        near(r.surface_elevation_at(Vec2::new(3.0, 1.0)).unwrap(), 0.5);
        near(r.surface_elevation_at(Vec2::new(3.0, 3.0)).unwrap(), 0.5);
    }

    #[test]
    fn hip_from_all_four_edges_is_the_straight_skeleton() {
        // 6 x 4 rectangle, all edges at 0.5: ridge from x = 2 to x = 4 at
        // y = 2, height 1.0. Facets: two trapezoids on the 6 m edges
        // ((6 + 2) / 2 x 2 = 8 each) and two triangles on the 4 m edges
        // (4 x 2 / 2 = 4 each).
        let t = tol();
        let r = roof([RoofEdge::sloped(0.5); 4]);
        let facets = r.facets(&t).unwrap();
        assert_eq!(facets.len(), 4);
        let mut areas: Vec<f64> = facets.iter().map(|f| f.plan_area()).collect();
        areas.sort_by(|a, b| a.total_cmp(b));
        near(areas[0], 4.0);
        near(areas[1], 4.0);
        near(areas[2], 8.0);
        near(areas[3], 8.0);
        near(areas.iter().sum::<f64>(), 24.0);
        near(r.ridge_height(&t).unwrap(), 1.0);
        near(r.inclined_area(&t).unwrap(), 24.0 * (1.25_f64).sqrt());
        // Ridge points.
        near(r.surface_elevation_at(Vec2::new(2.0, 2.0)).unwrap(), 1.0);
        near(r.surface_elevation_at(Vec2::new(4.0, 2.0)).unwrap(), 1.0);
        // A hip line runs from the corner (0,0) to (2,2).
        near(r.surface_elevation_at(Vec2::new(1.0, 1.0)).unwrap(), 0.5);
    }

    #[test]
    fn unequal_slopes_split_the_footprint_where_the_planes_meet() {
        // Edge 0 at 1.0, edge 2 at 0.5: plane_0 = y, plane_2 = 0.5 (4 - y).
        // They meet where y = 2 - y/2, i.e. y = 4/3; the steeper side owns the
        // narrower facet and the ridge is at 4/3.
        let t = tol();
        let r = roof([RoofEdge::sloped(1.0), F, RoofEdge::sloped(0.5), F]);
        let facets = r.facets(&t).unwrap();
        assert_eq!(facets.len(), 2);
        let f0 = facets.iter().find(|f| f.edge == Some(0)).unwrap();
        let f2 = facets.iter().find(|f| f.edge == Some(2)).unwrap();
        near(f0.plan_area(), 6.0 * 4.0 / 3.0);
        near(f2.plan_area(), 6.0 * 8.0 / 3.0);
        near(r.ridge_height(&t).unwrap(), 4.0 / 3.0);
        // Inclined area is per facet now, not A sqrt(1 + g^2) for one g.
        near(
            r.inclined_area(&t).unwrap(),
            8.0 * (2.0_f64).sqrt() + 16.0 * (1.25_f64).sqrt(),
        );
    }

    #[test]
    fn a_huge_finite_slope_is_a_near_vertical_plane() {
        // The scenario's last measured step: slope 1e6 on edge 0 alone.
        let t = tol();
        let r = roof([RoofEdge::sloped(1.0e6), F, F, F]);
        near(r.plan_area(), 24.0);
        let ridge = r.ridge_height(&t).unwrap();
        near(ridge, 4.0e6);
        // Inclined area grows with sqrt(1 + g^2) ~ g.
        let inclined = r.inclined_area(&t).unwrap();
        assert!((inclined / (24.0 * 1.0e6) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn clockwise_input_is_canonicalised_with_its_edge_data() {
        // Same footprint given clockwise; slope on the edge (0,0) -> (0,4) of
        // the INPUT order, i.e. the x = 0 side. The roof must rise along +x.
        let t = tol();
        let cw = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(0.0, 4.0),
            Vec2::new(6.0, 4.0),
            Vec2::new(6.0, 0.0),
        ];
        let r = FootprintRoof::new(&cw, &[RoofEdge::sloped(0.5), F, F, F], 0.0, &t).unwrap();
        near(r.ridge_height(&t).unwrap(), 3.0); // 0.5 x 6
        near(r.surface_elevation_at(Vec2::new(2.0, 2.0)).unwrap(), 1.0);
    }

    #[test]
    fn a_non_convex_sloped_footprint_is_rejected_not_guessed() {
        let t = tol();
        let l = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(6.0, 0.0),
            Vec2::new(6.0, 2.0),
            Vec2::new(2.0, 2.0),
            Vec2::new(2.0, 5.0),
            Vec2::new(0.0, 5.0),
        ];
        let e = FootprintRoof::new(&l, &[RoofEdge::sloped(0.5); 6], 0.0, &t).unwrap_err();
        assert_eq!(e.code(), crate::error::codes::ROOF_FOOTPRINT_NOT_CONVEX);
        // A flat roof on the same footprint is fine.
        assert!(FootprintRoof::new(&l, &[F; 6], 0.0, &t).is_ok());
    }
}
