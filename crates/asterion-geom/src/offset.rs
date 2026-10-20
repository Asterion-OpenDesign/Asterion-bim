//! Offsetting a whole region: grow it or shrink it by a distance, with corner
//! joins and **self-overlap removal**.
//!
//! This is the operation behind every "inset/outset a boundary" job in a BIM
//! model: the per-layer bands of a compound structure, a clearance envelope
//! around a solid, shrinking a slab boundary, and the faces of a wall derived
//! from its location line when the boundary is a closed loop rather than an
//! open path (for an open path use [`crate::band`]).
//!
//! # Sign convention
//!
//! A `Region` is canonical: the outer loop runs counter-clockwise and holes run
//! clockwise, so the material is always on the **left** of travel. Therefore
//!
//! * `distance > 0` **grows** the region (boundary moves right of travel:
//!   outer loop outwards, holes inwards);
//! * `distance < 0` **shrinks** it.
//!
//! The same number applies to every loop, so growing a pierced slab by `d`
//! enlarges its outline by `d` and closes its openings by `d`, which is what
//! "offset the boundary" means physically.
//!
//! # Why self-overlap removal is the hard part
//!
//! A naive offset of each curve is wrong as soon as `|distance|` exceeds the
//! local feature size: the raw offset loop crosses itself and encloses lobes
//! that are not part of the true offset. Shrinking a 0.2 m thick wall footprint
//! by 0.2 m is the everyday case — the two long faces pass through each other.
//!
//! The fix is a *global* resolution rather than per-corner cleverness, which is
//! what the literature calls the distance criterion: every point of a true
//! offset boundary lies at distance exactly `|distance|` from the source
//! boundary (corners excepted, which are farther). So the raw loop is split at
//! all of its self-intersections and each piece is kept only if
//!
//! 1. its midpoint is at distance `>= |distance| - slack` from the **source**
//!    region's boundary, and
//! 2. it is on the correct side (outside the source when growing, inside when
//!    shrinking).
//!
//! Pieces that the offset "passed over" fail (1); inverted lobes fail (2).
//! The survivors are stitched with the same machinery the booleans use. If
//! nothing survives, the region has been consumed and the result is empty —
//! that is the correct answer, not an error.

use crate::boolean::{assemble, closest_boundary, split_curve, stitch_partial};
use crate::curve::{Arc, Curve};
use crate::error::{GeomError, GeomResult};
use crate::intersect::{intersect_curves, intersect_curves_unbounded};
use crate::region::{Polycurve, Region};
use crate::tolerance::Tolerances;
use crate::vec::Vec2;

/// How to close the gap that opens at a corner when a boundary is offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JoinStyle {
    /// Extend both offset curves to the intersection of their supports. Falls
    /// back to a bevel when the mitre would reach farther than
    /// `limit * |distance|` from the original corner (the standard mitre limit,
    /// without which a near-tangent corner produces an arbitrarily long spike).
    Mitre { limit: f64 },
    /// Close the gap with a straight segment.
    Bevel,
    /// Close the gap with a circular arc of radius `|distance|` centred on the
    /// original corner — the true offset of a corner point.
    Round,
}

impl Default for JoinStyle {
    fn default() -> Self {
        JoinStyle::Mitre { limit: 4.0 }
    }
}

/// One piece of a raw offset boundary.
///
/// `joint` marks a corner connector (a bevel chord or a round fillet). Those
/// are **exempt from the distance criterion**: a bevel chord is deliberately
/// closer to the source than `|distance|` — being closer is what a chamfer is —
/// so testing it like a boundary piece would delete it and leave the loop open.
/// Joints are still split at self-intersections and still have to pass the side
/// test, which is what removes them inside a self-overlap.
#[derive(Clone, Copy, Debug)]
struct OffsetPiece {
    curve: Curve,
    joint: bool,
}

impl OffsetPiece {
    fn boundary(curve: Curve) -> Self {
        OffsetPiece { curve, joint: false }
    }
    fn joint(curve: Curve) -> Self {
        OffsetPiece { curve, joint: true }
    }
}

/// Offset a region. Positive grows, negative shrinks. The result can be empty
/// (consumed), one region, or several (a shrink that splits it).
pub fn offset_region(
    region: &Region,
    distance: f64,
    style: JoinStyle,
    tol: &Tolerances,
) -> GeomResult<Vec<Region>> {
    if distance == 0.0 {
        return Ok(vec![region.clone()]);
    }
    // Material is on the left of travel, so growing moves the boundary right.
    let side = -distance;

    let mut raw: Vec<OffsetPiece> = Vec::new();
    for lp in region.loops() {
        raw.extend(offset_closed_loop(lp, side, style, tol)?);
    }
    if raw.is_empty() {
        return Ok(Vec::new());
    }

    // Global self-intersection resolution.
    let mut cuts: Vec<Vec<Vec2>> = vec![Vec::new(); raw.len()];
    for i in 0..raw.len() {
        for j in (i + 1)..raw.len() {
            for p in intersect_curves(&raw[i].curve, &raw[j].curve, tol) {
                cuts[i].push(p);
                cuts[j].push(p);
            }
        }
    }
    let mut pieces: Vec<OffsetPiece> = Vec::new();
    for (i, piece) in raw.iter().enumerate() {
        for part in split_curve(&piece.curve, &cuts[i], tol)? {
            pieces.push(OffsetPiece { curve: part, joint: piece.joint });
        }
    }

    // Distance criterion + side test.
    let want = distance.abs();
    let slack = (tol.vertex * 8.0).max(want * 1e-9);
    let growing = distance > 0.0;
    let kept: Vec<Curve> = pieces
        .into_iter()
        .filter(|piece| {
            let m = piece.curve.midpoint();
            let Some((d, _)) = closest_boundary(region, m) else {
                return false;
            };
            if !piece.joint && d < want - slack {
                return false;
            }
            region.contains_point(m) != growing
        })
        .map(|piece| piece.curve)
        .collect();
    if kept.is_empty() {
        return Ok(Vec::new());
    }

    // A collapsed feature leaves pieces that close nothing; the loops that did
    // close are the answer (see `stitch_partial`).
    let (loops, _orphans) = stitch_partial(kept, tol);
    if loops.is_empty() {
        return Ok(Vec::new());
    }
    assemble(loops, tol)
}

/// Offset one closed loop, curve by curve, closing each corner per `style`.
/// `side` is in the curve convention (positive = left of travel).
///
/// Two passes, because a connector must be built from final endpoints: first
/// every corner that resolves to a *point* is applied (a real crossing, or a
/// mitre), then the remaining corners get their connector. Corner `i` only ever
/// writes `curve[i].end` and `curve[i+1].start`, so the passes cannot fight.
fn offset_closed_loop(
    lp: &Polycurve,
    side: f64,
    style: JoinStyle,
    tol: &Tolerances,
) -> GeomResult<Vec<OffsetPiece>> {
    let source = lp.curves();
    let mut work: Vec<Curve> = Vec::with_capacity(source.len());
    // `pivots[i]` is the original corner at the end of `work[i]`.
    let mut pivots: Vec<Vec2> = Vec::with_capacity(source.len());
    for c in source {
        match c.offset(side) {
            Ok(o) => {
                work.push(o);
                pivots.push(c.end());
            }
            // An arc whose radius the offset consumes contributes nothing; the
            // neighbouring corners close the gap it leaves.
            Err(GeomError::OffsetCollapsesCurve { .. }) => {}
            Err(e) => return Err(e),
        }
    }
    let m = work.len();
    if m <= 1 {
        return Ok(work.into_iter().map(OffsetPiece::boundary).collect());
    }

    #[derive(Clone, Copy)]
    enum Action {
        Continuous,
        Connector,
    }
    let mut actions = vec![Action::Continuous; m];

    // Pass 1: trims and mitres.
    for i in 0..m {
        let j = (i + 1) % m;
        let (prev, next) = (work[i], work[j]);
        let gap = prev.end().distance_to(next.start());
        if gap <= tol.vertex {
            continue;
        }
        let target = prev.end().lerp(next.start(), 0.5);
        // Do the offsets actually cross? Then the crossing is a real vertex.
        if let Some(p) = nearest(&intersect_curves(&prev, &next, tol), target) {
            work[i] = prev.with_end(p)?;
            work[j] = next.with_start(p)?;
            continue;
        }
        if let JoinStyle::Mitre { limit } = style {
            let hits = intersect_curves_unbounded(&prev, &next, tol);
            let pick = nearest(&hits, target)
                .filter(|p| p.distance_to(pivots[i]) <= limit.max(1.0) * side.abs());
            if let Some(p) = pick {
                work[i] = prev.with_end(p)?;
                work[j] = next.with_start(p)?;
                continue;
            }
        }
        actions[i] = Action::Connector;
    }

    // Pass 2: build connectors from the final endpoints and splice.
    let mut out: Vec<OffsetPiece> = Vec::with_capacity(m * 2);
    for i in 0..m {
        out.push(OffsetPiece::boundary(work[i]));
        if let Action::Connector = actions[i] {
            let j = (i + 1) % m;
            let (a, b) = (work[i].end(), work[j].start());
            let connector = match style {
                JoinStyle::Round => round_joint(pivots[i], work[i], work[j], side.abs())
                    .unwrap_or_else(|| Curve::line(a, b)),
                _ => Curve::line(a, b),
            };
            if connector.length() > tol.vertex {
                out.push(OffsetPiece::joint(connector));
            }
        }
    }
    Ok(out)
}

/// Arc of radius `radius` centred on the original corner, from the previous
/// offset's end to the next offset's start, turning the way the boundary turns.
fn round_joint(pivot: Vec2, prev: Curve, next: Curve, radius: f64) -> Option<Curve> {
    let a = prev.end();
    let b = next.start();
    if (a.distance_to(pivot) - radius).abs() > radius * 1e-6
        || (b.distance_to(pivot) - radius).abs() > radius * 1e-6
    {
        return None;
    }
    let t_in = prev.tangent_at(1.0)?;
    let t_out = next.tangent_at(0.0)?;
    let ccw = t_in.cross(t_out) > 0.0;
    Arc::from_center_angles(pivot, radius, (a - pivot).angle(), (b - pivot).angle(), ccw)
        .ok()
        .map(Curve::Arc)
}

fn nearest(points: &[Vec2], target: Vec2) -> Option<Vec2> {
    points
        .iter()
        .copied()
        .min_by(|x, y| x.distance_to(target).total_cmp(&y.distance_to(target)))
}

impl Region {
    /// Offset the whole boundary. Positive grows, negative shrinks.
    pub fn offset(
        &self,
        distance: f64,
        style: JoinStyle,
        tol: &Tolerances,
    ) -> GeomResult<Vec<Region>> {
        offset_region(self, distance, style, tol)
    }
    /// Grow by `distance` with the default mitre join.
    pub fn grown(&self, distance: f64, tol: &Tolerances) -> GeomResult<Vec<Region>> {
        offset_region(self, distance, JoinStyle::default(), tol)
    }
    /// Shrink by `distance` with the default mitre join. Returns an empty vector
    /// when the region is consumed.
    pub fn shrunk(&self, distance: f64, tol: &Tolerances) -> GeomResult<Vec<Region>> {
        offset_region(self, -distance, JoinStyle::default(), tol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boolean::total_area;
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
    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Region {
        Region::from_loop(
            Polycurve::rectangle(Vec2::new(x0, y0), Vec2::new(x1, y1), &tol()).unwrap(),
        )
    }
    fn disc(cx: f64, cy: f64, r: f64) -> Region {
        Region::from_loop(Polycurve::circle(Vec2::new(cx, cy), r, &tol()).unwrap())
    }

    // ------------------------------------------------------------- the basics

    #[test]
    fn growing_a_square_with_a_mitre_gives_the_bigger_square() {
        let s = rect(0.0, 0.0, 4.0, 4.0);
        let g = s.grown(0.5, &tol()).unwrap();
        assert_eq!(g.len(), 1, "{g:?}");
        near(g[0].area(), 25.0);
        near(g[0].outer_perimeter(), 20.0);
        assert_eq!(g[0].curve_count(), 4);
        let b = g[0].aabb();
        near(b.min.x, -0.5);
        near(b.max.y, 4.5);
    }

    #[test]
    fn shrinking_a_square_gives_the_smaller_square() {
        let s = rect(0.0, 0.0, 4.0, 4.0);
        let r = s.shrunk(0.5, &tol()).unwrap();
        assert_eq!(r.len(), 1, "{r:?}");
        near(r[0].area(), 9.0);
        near(r[0].outer_perimeter(), 12.0);
        let b = r[0].aabb();
        near(b.min.x, 0.5);
        near(b.max.x, 3.5);
    }

    #[test]
    fn round_and_bevel_joins_give_their_closed_form_areas() {
        let s = rect(0.0, 0.0, 4.0, 4.0);
        let d = 0.5;
        // Round: square + perimeter band + a full disc of radius d at the corners.
        let round = s.offset(d, JoinStyle::Round, &tol()).unwrap();
        assert_eq!(round.len(), 1, "{round:?}");
        near_eps(round[0].area(), 16.0 + 16.0 * d + PI * d * d, 1e-12);
        assert_eq!(round[0].outer().arc_count(), 4);
        near_eps(round[0].outer_perimeter(), 16.0 + 2.0 * PI * d, 1e-12);
        // Bevel: square + band + four corner triangles (d^2 / 2 each).
        let bevel = s.offset(d, JoinStyle::Bevel, &tol()).unwrap();
        near_eps(bevel[0].area(), 16.0 + 16.0 * d + 4.0 * d * d / 2.0, 1e-12);
        assert_eq!(bevel[0].curve_count(), 8);
    }

    #[test]
    fn growing_and_shrinking_a_disc_is_concentric() {
        let c = disc(1.0, -2.0, 3.0);
        let g = c.grown(0.75, &tol()).unwrap();
        near_eps(total_area(&g), PI * 3.75 * 3.75, 1e-12);
        let s = c.shrunk(0.75, &tol()).unwrap();
        near_eps(total_area(&s), PI * 2.25 * 2.25, 1e-12);
    }

    #[test]
    fn an_arc_boundary_keeps_its_arcs_under_offset() {
        // Stadium: straight 4, radius 1. Grown by 0.5 the caps become r = 1.5.
        let right = Arc::from_center_angles(Vec2::new(2.0, 0.0), 1.0, -PI / 2.0, PI / 2.0, true).unwrap();
        let left = Arc::from_center_angles(Vec2::new(-2.0, 0.0), 1.0, PI / 2.0, 3.0 * PI / 2.0, true).unwrap();
        let lp = Polycurve::closed(
            vec![
                Curve::line(Vec2::new(-2.0, -1.0), Vec2::new(2.0, -1.0)),
                Curve::Arc(right),
                Curve::line(Vec2::new(2.0, 1.0), Vec2::new(-2.0, 1.0)),
                Curve::Arc(left),
            ],
            &tol(),
        )
        .unwrap();
        let s = Region::from_loop(lp);
        near(s.area(), 8.0 + PI);
        let g = s.grown(0.5, &tol()).unwrap();
        assert_eq!(g.len(), 1, "{g:?}");
        // Straight length 4, radius 1.5 => 4 * 3 + pi * 1.5^2
        near_eps(g[0].area(), 4.0 * 3.0 + PI * 2.25, 1e-12);
        assert_eq!(g[0].outer().arc_count(), 2);
    }

    // --------------------------------------------- the degenerate inward case

    #[test]
    fn shrinking_a_thin_wall_footprint_past_its_half_width_consumes_it() {
        // The everyday case: a 4.000 x 0.200 wall footprint. Shrinking by more
        // than 0.100 makes the two long faces cross; the self-overlap must be
        // removed, not returned as a bowtie.
        let w = rect(0.0, 0.0, 4.0, 0.2);
        let t = tol();
        near(w.shrunk(0.05, &t).unwrap()[0].area(), 3.9 * 0.1);
        // Exactly at the half width and beyond: nothing survives.
        assert!(w.shrunk(0.1, &t).unwrap().is_empty());
        assert!(w.shrunk(0.2, &t).unwrap().is_empty());
        assert!(w.shrunk(5.0, &t).unwrap().is_empty());
    }

    #[test]
    fn shrinking_a_square_past_its_inradius_consumes_it() {
        let s = rect(0.0, 0.0, 4.0, 4.0);
        let t = tol();
        near(s.shrunk(1.5, &t).unwrap()[0].area(), 1.0);
        assert!(s.shrunk(2.0, &t).unwrap().is_empty());
        assert!(s.shrunk(2.5, &t).unwrap().is_empty());
    }

    #[test]
    fn shrinking_a_disc_past_its_radius_consumes_it() {
        let c = disc(0.0, 0.0, 1.0);
        assert!(c.shrunk(1.0, &tol()).unwrap().is_empty());
        assert!(c.shrunk(1.5, &tol()).unwrap().is_empty());
    }

    #[test]
    fn shrinking_a_dumbbell_splits_it_into_two_regions() {
        // Two 2x2 pads joined by a 0.2 m wide neck: shrinking by 0.15 severs
        // the neck and must return two separate regions, not one bowtie.
        use crate::boolean::union_all;
        let a = rect(0.0, 0.0, 2.0, 2.0);
        let neck = rect(2.0, 0.9, 5.0, 1.1);
        let b = rect(5.0, 0.0, 7.0, 2.0);
        let joined = union_all(&[a, neck, b], &tol()).unwrap();
        assert_eq!(joined.len(), 1);
        let s = joined[0].shrunk(0.15, &tol()).unwrap();
        assert_eq!(s.len(), 2, "{s:?}");
        near_eps(total_area(&s), 2.0 * (1.7 * 1.7), 1e-9);
    }

    #[test]
    fn a_concave_corner_does_not_leave_a_spike() {
        // L-shape grown outwards: the concave corner's offsets cross and must
        // be trimmed to the crossing, not extended into a spike.
        let l = Region::from_polygon(
            &[
                Vec2::new(0.0, 0.0),
                Vec2::new(4.0, 0.0),
                Vec2::new(4.0, 1.0),
                Vec2::new(1.0, 1.0),
                Vec2::new(1.0, 3.0),
                Vec2::new(0.0, 3.0),
            ],
            &tol(),
        )
        .unwrap();
        near(l.area(), 4.0 + 2.0);
        let g = l.grown(0.25, &tol()).unwrap();
        assert_eq!(g.len(), 1, "{g:?}");
        // Grown area = A + perimeter * d + corner terms. A *mitred* right-angle
        // corner adds a full d^2 (a bevel would add d^2/2), and a concave right
        // angle removes one, because the two offsets cross and the overlap is
        // counted once. Five convex and one concave => (5 - 1) * d^2.
        let d: f64 = 0.25;
        let perimeter = l.outer_perimeter();
        near(perimeter, 14.0);
        near_eps(g[0].area(), 6.0 + perimeter * d + 4.0 * d * d, 1e-9);
        assert!(g[0].aabb().min.x <= -d + 1e-12);
    }

    #[test]
    fn shrinking_an_l_shape_past_its_thin_leg_consumes_only_that_leg() {
        let l = Region::from_polygon(
            &[
                Vec2::new(0.0, 0.0),
                Vec2::new(4.0, 0.0),
                Vec2::new(4.0, 2.0),
                Vec2::new(1.0, 2.0),
                Vec2::new(1.0, 0.2),
                Vec2::new(0.0, 0.2),
            ],
            &tol(),
        )
        .unwrap();
        // The left leg is only 0.2 thick; shrinking by 0.15 must keep just the
        // thick part and may not leave an inverted lobe behind.
        let s = l.shrunk(0.15, &tol()).unwrap();
        assert!(s.len() >= 1, "{s:?}");
        let area: f64 = total_area(&s);
        assert!(area > 0.0 && area < l.area(), "area {area}");
        // Every surviving boundary point is at least 0.15 from the source.
        for r in &s {
            for lp in r.loops() {
                for c in lp.curves() {
                    let (d, _) = closest_boundary(&l, c.midpoint()).unwrap();
                    assert!(d >= 0.15 - 1e-6, "piece at {d} from the source");
                }
            }
        }
    }

    // ------------------------------------------------------------- with holes

    #[test]
    fn offsetting_an_annulus_moves_both_loops_the_same_way() {
        let outer = Polycurve::circle(Vec2::new(0.0, 0.0), 5.0, &tol()).unwrap();
        let inner = Polycurve::circle(Vec2::new(0.0, 0.0), 2.0, &tol()).unwrap();
        let ring = Region::new(outer, vec![inner], &tol()).unwrap();
        near_eps(ring.area(), PI * 21.0, 1e-12);
        // Growing: outer 6, hole 1.
        let g = ring.grown(1.0, &tol()).unwrap();
        assert_eq!(g.len(), 1, "{g:?}");
        near_eps(g[0].area(), PI * (36.0 - 1.0), 1e-9);
        assert_eq!(g[0].holes().len(), 1);
        // Shrinking: outer 4, hole 3.
        let s = ring.shrunk(1.0, &tol()).unwrap();
        assert_eq!(s.len(), 1, "{s:?}");
        near_eps(s[0].area(), PI * (16.0 - 9.0), 1e-9);
        // Shrinking past the ring's own thickness consumes it.
        assert!(ring.shrunk(1.6, &tol()).unwrap().is_empty());
    }

    #[test]
    fn growing_a_slab_closes_its_opening() {
        let outer = Polycurve::rectangle(Vec2::new(0.0, 0.0), Vec2::new(6.0, 4.0), &tol()).unwrap();
        let hole = Polycurve::rectangle(Vec2::new(2.0, 1.0), Vec2::new(3.0, 2.0), &tol()).unwrap();
        let slab = Region::new(outer, vec![hole], &tol()).unwrap();
        near(slab.area(), 24.0 - 1.0);
        let g = slab.grown(0.25, &tol()).unwrap();
        assert_eq!(g.len(), 1, "{g:?}");
        // Outer grows to 6.5 x 4.5 with mitred corners; the 1 x 1 hole shrinks
        // to 0.5 x 0.5.
        near_eps(g[0].area(), 6.5 * 4.5 - 0.25, 1e-9);
        // Growing by half the hole's size closes it completely.
        let closed = slab.grown(0.5, &tol()).unwrap();
        assert_eq!(closed[0].holes().len(), 0, "the opening must be gone");
        near_eps(closed[0].area(), 7.0 * 5.0, 1e-9);
    }

    // --------------------------------- compound structure at a corner (ready)

    /// The real compound structure measured: `AST-ASYM-2625` = finish1 0.0200, thermal_air 0.0800, structure 0.1500,
    /// finish2 0.0125, total **0.2625** (exterior shell = the first 2 layers, interior shell = the last 1).
    ///
    /// This test builds the four **per-layer bands** of the butt wall with the real widths, checks they partition the wall
    /// body exactly, and then computes the joined footprint as `body_layer.difference(union of per-layer penetrations)` —
    /// the "fingers".
    ///
    /// The penetration depths are the **only** unknown: they are a single
    /// `[f64; 4]` here. When the research lane measures them, the numbers go in
    /// that array and nothing else changes. Three depth sets are exercised:
    /// all-zero (no join), full-width (the axial trim the engine does today),
    /// and alternating (a real finger pattern), with the measured invariants
    /// asserted for each.
    #[test]
    fn compound_structure_corner_is_ready_for_measured_penetration_depths() {
        let t = tol();
        let layers = [0.02_f64, 0.08, 0.15, 0.0125];
        let total: f64 = layers.iter().sum();
        near(total, 0.2625);
        let half = total / 2.0;
        let height = 4.0_f64;

        // Butt wall centreline (5,0) -> (5,4). Layer k occupies the band
        // between the cumulative offsets measured from the exterior face.
        let axis = [Curve::line(Vec2::new(5.0, 0.0), Vec2::new(5.0, height))];
        let mut bands: Vec<Region> = Vec::new();
        let mut cumulative = 0.0;
        for w in layers {
            // Offsets are measured from the centreline, left positive.
            let left = half - cumulative;
            let right = -(half - cumulative - w);
            let band = crate::band::offset_band(&axis, left, right, &t).unwrap();
            near(band.area(), w * height);
            bands.push(band);
            cumulative += w;
        }
        near(cumulative, total);

        // The bands partition the body: areas add up and they do not overlap.
        let body = crate::band::offset_band(&axis, half, half, &t).unwrap();
        near(body.area(), total * height);
        near(
            bands.iter().map(|b| b.area()).sum::<f64>(),
            body.area(),
        );
        for i in 0..bands.len() {
            for j in (i + 1)..bands.len() {
                let overlap = bands[i].intersection(&bands[j], &t).unwrap();
                assert!(
                    crate::boolean::total_area(&overlap) <= 1e-12,
                    "layers {i} and {j} overlap"
                );
            }
        }

        // The through wall penetrates each layer to its own depth, measured
        // from y = 0 (the corner).
        let joined_area = |depths: [f64; 4]| -> f64 {
            let mut fingers: Vec<Region> = Vec::new();
            let mut x = 5.0 - half;
            for (w, d) in layers.iter().zip(depths.iter()) {
                if *d > 0.0 {
                    fingers.push(rect(x, 0.0, x + w, *d));
                }
                x += w;
            }
            if fingers.is_empty() {
                return body.area();
            }
            let penetration = crate::boolean::union_all(&fingers, &t).unwrap();
            let mut area = body.area();
            for p in &penetration {
                let cut = body.difference(p, &t).unwrap();
                area = crate::boolean::total_area(&cut);
            }
            area
        };

        // (1) No penetration: the joined body is the unjoined body.
        near(joined_area([0.0; 4]), total * height);

        // (2) Every layer penetrating by h_q: the axial trim the engine does
        //     today, i.e. the measured area loss width * h_q.
        near(joined_area([half; 4]), total * height - total * half);
        near(total * half, 0.2625 * 0.13125);

        // (3) A real finger pattern: only the two middle layers penetrate.
        //     The loss is the sum of layer_width * depth, and the bounding box
        //     keeps its full extent, which is the measured property.
        let depths = [0.0, half, half, 0.0];
        let expected_loss: f64 = layers
            .iter()
            .zip(depths.iter())
            .map(|(w, d)| w * d)
            .sum();
        near(joined_area(depths), total * height - expected_loss);
        near(expected_loss, (0.08 + 0.15) * half);

        let mut fingers: Vec<Region> = Vec::new();
        let mut x = 5.0 - half;
        for (w, d) in layers.iter().zip(depths.iter()) {
            if *d > 0.0 {
                fingers.push(rect(x, 0.0, x + w, *d));
            }
            x += w;
        }
        let penetration = crate::boolean::union_all(&fingers, &t).unwrap();
        let notched = body.difference(&penetration[0], &t).unwrap();
        assert_eq!(notched.len(), 1);
        near(notched[0].aabb().min.y, 0.0);
        near(notched[0].aabb().min.x, 5.0 - half);
        near(notched[0].aabb().max.x, 5.0 + half);
        assert_eq!(notched[0].curve_count(), 8);
    }

    #[test]
    fn offsets_are_bit_reproducible() {
        let s = rect(0.0, 0.0, 3.0, 1.0);
        let first = s.grown(0.37, &tol()).unwrap();
        let key: Vec<u64> = first.iter().map(|r| r.area().to_bits()).collect();
        for _ in 0..8 {
            let again = s.grown(0.37, &tol()).unwrap();
            let k: Vec<u64> = again.iter().map(|r| r.area().to_bits()).collect();
            assert_eq!(k, key);
        }
    }

    #[test]
    fn zero_offset_is_the_identity() {
        let s = rect(0.0, 0.0, 2.0, 3.0);
        let z = s.offset(0.0, JoinStyle::default(), &tol()).unwrap();
        assert_eq!(z.len(), 1);
        near(z[0].area(), 6.0);
    }
}
