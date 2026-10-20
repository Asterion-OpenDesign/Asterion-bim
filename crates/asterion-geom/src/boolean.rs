//! Arc-aware 2D boolean operations on planar regions.
//!
//! This is the capability the measured residuals need:
//!
//! * **Openings** — (MEASURED): a host is reduced by *"one union of clipped rectangles, intersected with the host's extent
//!   in both axes. Never sum opening areas.
//! * **Corner joins** — : the butt wall keeps its full bounding box because its footprint is **notched**, not axially
//!   trimmed. A notch is `difference`.
//!
//! # Algorithm
//!
//! A boundary-representation boolean on an arrangement of directed curves:
//!
//! 1. intersect every curve of A with every curve of B, keeping the *same*
//!    intersection point for both sides so the split endpoints agree;
//! 2. split both boundaries at those points into directed **pieces**;
//! 3. classify each piece's midpoint against the other operand as `Inside`,
//!    `Outside` or `OnBoundary { same_direction }`;
//! 4. select pieces per operation (table in [`select`]);
//! 5. stitch the survivors into closed loops;
//! 6. sort loops into outer shells and holes and rebuild `Region`s.
//!
//! Metrics of the result are then the kernel's ordinary **exact** formulas on
//! the surviving arcs and segments — a boolean never turns an arc into a
//! polyline, so `Area` and `Perimeter` stay closed-form.
//!
//! # Determinism
//!
//! Pieces are generated and visited in a fixed order (A's loops in order, then
//! B's), ties while stitching are broken by the straightest continuation and
//! then by index, and the output regions are sorted by (descending area, then
//! bounding-box corner). No hash containers, no threads: the same input gives
//! bit-identical output.
//!
//! # Known limitations (documented, not hidden)
//!
//! * Self-intersecting input loops are out of scope; `Polycurve` does not
//!   validate simplicity beyond its turning number.
//! * Three or more pieces meeting at one point (regions touching at a single
//!   vertex) are resolved by the straightest-continuation tie-break, which is a
//!   choice, not a theorem.
//! * A failure to close returns `GEOM_BOOLEAN_DID_NOT_CLOSE` rather than
//!   silently dropping geometry.

use crate::curve::Curve;
use crate::error::{GeomError, GeomResult};
use crate::intersect::intersect_curves;
use crate::region::{Polycurve, Region};
use crate::tolerance::Tolerances;
use crate::vec::Vec2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    Union,
    Intersection,
    /// `a - b`.
    Difference,
}

impl BoolOp {
    /// The reference product's `BooleanType` value of the operation - 0 union, 1 difference (first minus second), 2
    /// intersection.
    pub fn reference_code(self) -> u8 {
        match self {
            BoolOp::Union => 0,
            BoolOp::Difference => 1,
            BoolOp::Intersection => 2,
        }
    }

    /// The operation of a `BooleanType` value, `None` for a value above 2.
    pub fn from_reference_code(code: u8) -> Option<BoolOp> {
        match code {
            0 => Some(BoolOp::Union),
            1 => Some(BoolOp::Difference),
            2 => Some(BoolOp::Intersection),
            _ => None,
        }
    }

    /// The inversion of the result from the inversion flags of the operands (`a` first, `b` second): union `a OR b`,
    /// difference `a AND NOT b`, intersection `a AND b`.
    pub fn result_inverted(self, a: bool, b: bool) -> bool {
        match self {
            BoolOp::Union => a || b,
            BoolOp::Difference => a && !b,
            BoolOp::Intersection => a && b,
        }
    }
}

/// Where a boundary piece sits relative to the other operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Inside,
    Outside,
    OnBoundary { same_direction: bool },
}

#[derive(Clone, Copy, Debug)]
struct Piece {
    curve: Curve,
    from_a: bool,
}

/// Boolean of two regions. The result can be empty (no overlap for an
/// intersection), one region, or several (a union of disjoint parts, or a
/// difference that splits the operand in two).
pub fn boolean(a: &Region, b: &Region, op: BoolOp, tol: &Tolerances) -> GeomResult<Vec<Region>> {
    let a_curves: Vec<Curve> = a.loops().flat_map(|l| l.curves().to_vec()).collect();
    let b_curves: Vec<Curve> = b.loops().flat_map(|l| l.curves().to_vec()).collect();

    // 1 + 2: split both boundaries at the shared intersection points.
    let mut a_cuts: Vec<Vec<Vec2>> = vec![Vec::new(); a_curves.len()];
    let mut b_cuts: Vec<Vec<Vec2>> = vec![Vec::new(); b_curves.len()];
    for (i, ca) in a_curves.iter().enumerate() {
        for (j, cb) in b_curves.iter().enumerate() {
            for p in intersect_curves(ca, cb, tol) {
                a_cuts[i].push(p);
                b_cuts[j].push(p);
            }
        }
    }

    let mut pieces: Vec<Piece> = Vec::new();
    for (i, c) in a_curves.iter().enumerate() {
        for piece in split_curve(c, &a_cuts[i], tol)? {
            pieces.push(Piece { curve: piece, from_a: true });
        }
    }
    for (j, c) in b_curves.iter().enumerate() {
        for piece in split_curve(c, &b_cuts[j], tol)? {
            pieces.push(Piece { curve: piece, from_a: false });
        }
    }

    // 3 + 4: classify and select.
    let mut kept: Vec<Curve> = Vec::new();
    for piece in &pieces {
        let other = if piece.from_a { b } else { a };
        let side = classify(&piece.curve, other, tol);
        if let Some(curve) = select(piece, side, op) {
            kept.push(curve);
        }
    }
    if kept.is_empty() {
        return Ok(Vec::new());
    }

    // 5 + 6.
    let loops = stitch(kept, tol)?;
    assemble(loops, tol)
}

/// Selection table. For a piece of **A** classified against B, and for a piece
/// of **B** classified against A:
///
/// | side | union | intersection | difference (`a - b`) |
/// |---|---|---|---|
/// | A inside B | drop | keep | drop |
/// | A outside B | keep | drop | keep |
/// | A on B, same direction | keep | keep | drop |
/// | A on B, opposite direction | drop | drop | keep |
/// | B inside A | drop | keep | keep **reversed** |
/// | B outside A | keep | drop | drop |
/// | B on A (either direction) | drop (A's copy is kept) | drop | drop |
fn select(piece: &Piece, side: Side, op: BoolOp) -> Option<Curve> {
    let c = piece.curve;
    match (piece.from_a, op, side) {
        // --- pieces of A ---
        (true, BoolOp::Union, Side::Outside) => Some(c),
        (true, BoolOp::Union, Side::OnBoundary { same_direction: true }) => Some(c),
        (true, BoolOp::Intersection, Side::Inside) => Some(c),
        (true, BoolOp::Intersection, Side::OnBoundary { same_direction: true }) => Some(c),
        (true, BoolOp::Difference, Side::Outside) => Some(c),
        (true, BoolOp::Difference, Side::OnBoundary { same_direction: false }) => Some(c),
        // --- pieces of B ---
        (false, BoolOp::Union, Side::Outside) => Some(c),
        (false, BoolOp::Intersection, Side::Inside) => Some(c),
        (false, BoolOp::Difference, Side::Inside) => Some(c.reversed()),
        _ => None,
    }
}

/// Classify a piece by its midpoint and tangent.
fn classify(piece: &Curve, other: &Region, tol: &Tolerances) -> Side {
    let m = piece.midpoint();
    if let Some((distance, other_tangent)) = closest_boundary(other, m) {
        if distance <= tol.vertex {
            let t = piece.tangent_at(0.5).unwrap_or(Vec2::new(1.0, 0.0));
            return Side::OnBoundary { same_direction: t.dot(other_tangent) > 0.0 };
        }
    }
    if other.contains_point(m) {
        Side::Inside
    } else {
        Side::Outside
    }
}

/// Distance from `p` to a region's boundary and the boundary's tangent there.
pub(crate) fn closest_boundary(region: &Region, p: Vec2) -> Option<(f64, Vec2)> {
    let mut best: Option<(f64, Vec2)> = None;
    for lp in region.loops() {
        for c in lp.curves() {
            let (q, t) = c.closest_point(p);
            let d = p.distance_to(q);
            if best.is_none() || d < best.unwrap().0 {
                let tangent = c.tangent_at(t).unwrap_or(Vec2::new(1.0, 0.0));
                best = Some((d, tangent));
            }
        }
    }
    best
}

/// Split a curve at a set of points lying on it, dropping duplicates and
/// points within tolerance of the ends.
pub(crate) fn split_curve(c: &Curve, points: &[Vec2], tol: &Tolerances) -> GeomResult<Vec<Curve>> {
    if points.is_empty() {
        return Ok(vec![*c]);
    }
    let length = c.length();
    if length <= 0.0 {
        return Ok(vec![*c]);
    }
    let param_slack = (tol.vertex / length).min(0.49);
    let mut keyed: Vec<(f64, Vec2)> = Vec::new();
    for p in points {
        let t = c.parameter_at_point(*p);
        if t <= param_slack || t >= 1.0 - param_slack {
            continue;
        }
        if keyed.iter().any(|(u, _)| (u - t).abs() <= param_slack) {
            continue;
        }
        keyed.push((t, *p));
    }
    if keyed.is_empty() {
        return Ok(vec![*c]);
    }
    keyed.sort_by(|x, y| x.0.total_cmp(&y.0));

    let mut out: Vec<Curve> = Vec::with_capacity(keyed.len() + 1);
    let mut start = c.start();
    for (_, p) in &keyed {
        out.push(c.with_start(start)?.with_end(*p)?);
        start = *p;
    }
    out.push(c.with_start(start)?.with_end(c.end())?);
    // Drop anything that the split made degenerate.
    out.retain(|piece| piece.length() > tol.vertex);
    if out.is_empty() {
        out.push(*c);
    }
    Ok(out)
}

/// Stitch directed pieces into closed loops, failing if anything is left over.
pub(crate) fn stitch(pieces: Vec<Curve>, tol: &Tolerances) -> GeomResult<Vec<Polycurve>> {
    let (loops, leftover) = stitch_partial(pieces, tol);
    if leftover > 0 {
        return Err(GeomError::BooleanDidNotClose { open_pieces: leftover });
    }
    Ok(loops)
}

/// Stitch directed pieces into closed loops, returning the loops that closed
/// and how many pieces closed nothing.
///
/// Booleans treat a leftover as a failure, because a boolean of two valid
/// regions has no reason to leave one. Offsetting does not: when an offset
/// collapses a feature (shrinking a 0.200 m wall footprint by 0.100 m), the
/// pieces around the collapsed part connect to nothing by construction, and the
/// correct answer is the loops that did close — possibly none.
pub(crate) fn stitch_partial(pieces: Vec<Curve>, tol: &Tolerances) -> (Vec<Polycurve>, usize) {
    let n = pieces.len();
    let mut used = vec![false; n];
    let mut loops: Vec<Polycurve> = Vec::new();
    let mut leftover = 0usize;
    // The pieces whose start lies within `tol.vertex` of a point are found through a hash grid of the
    // starts (cell = four vertex tolerances, the 3 x 3 cells around the point), instead of a scan of
    // every piece for every link of every chain (a quadratic stitch per boolean, cubic over a union of
    // a thousand walls). The candidates are visited in ascending index, so the pick (the smallest
    // turn, the lowest index on a tie) is exactly the one the scan made.
    let cell = (tol.vertex * 4.0).max(1e-9);
    let key = |p: Vec2| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
    let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> = std::collections::HashMap::with_capacity(n);
    for (i, piece) in pieces.iter().enumerate() {
        grid.entry(key(piece.start())).or_default().push(i);
    }
    let mut near: Vec<usize> = Vec::new();

    for seed in 0..n {
        if used[seed] {
            continue;
        }
        let mut chain: Vec<Curve> = vec![pieces[seed]];
        used[seed] = true;
        loop {
            let tail = *chain.last().unwrap();
            if chain.len() > 1 && tail.end().distance_to(chain[0].start()) <= tol.vertex {
                break;
            }
            let incoming = tail.tangent_at(1.0).unwrap_or(Vec2::new(1.0, 0.0));
            let mut pick: Option<(usize, f64)> = None;
            near.clear();
            let (kx, ky) = key(tail.end());
            for dx in -1..=1 {
                for dy in -1..=1 {
                    if let Some(v) = grid.get(&(kx + dx, ky + dy)) {
                        near.extend(v.iter().copied());
                    }
                }
            }
            near.sort_unstable();
            for &i in &near {
                let candidate = &pieces[i];
                if used[i] || candidate.start().distance_to(tail.end()) > tol.vertex {
                    continue;
                }
                let out = candidate.tangent_at(0.0).unwrap_or(Vec2::new(1.0, 0.0));
                // Straightest continuation first; deterministic by index after.
                let turn = out.cross(incoming).atan2(out.dot(incoming)).abs();
                if pick.is_none() || turn < pick.unwrap().1 {
                    pick = Some((i, turn));
                }
            }
            match pick {
                Some((i, _)) => {
                    used[i] = true;
                    chain.push(pieces[i]);
                }
                None => break,
            }
        }
        if chain.last().unwrap().end().distance_to(chain[0].start()) <= tol.vertex {
            match Polycurve::closed(chain, tol) {
                Ok(lp) => loops.push(lp),
                // A sliver thinner than ShortCurveTolerance is not
                // representable geometry: drop it instead of failing. This is
                // the one place the kernel loses area, and it is bounded by
                // the tolerance, so it is documented rather than hidden.
                Err(GeomError::CurveTooShort { .. }) => {}
                Err(_) => leftover += 1,
            }
        } else {
            leftover += chain.len();
        }
    }
    (loops, leftover)
}

/// Sort loops into outer shells (counter-clockwise) and holes (clockwise), and
/// assign each hole to the smallest shell that contains it.
pub(crate) fn assemble(loops: Vec<Polycurve>, tol: &Tolerances) -> GeomResult<Vec<Region>> {
    let min_area = tol.vertex * tol.vertex;
    let mut shells: Vec<Polycurve> = Vec::new();
    let mut holes: Vec<Polycurve> = Vec::new();
    for lp in loops {
        if lp.area() <= min_area {
            continue;
        }
        // Normalise: a boolean split a boundary at every crossing, so adjacent
        // pieces of one original curve must be merged back before the counts
        // mean anything.
        let lp = lp.simplified(tol).unwrap_or(lp);
        if lp.is_counter_clockwise() {
            shells.push(lp);
        } else {
            holes.push(lp);
        }
    }
    if shells.is_empty() {
        return Ok(Vec::new());
    }
    let mut assigned: Vec<Vec<Polycurve>> = vec![Vec::new(); shells.len()];
    for hole in holes {
        let probe = hole.curves()[0].midpoint();
        let mut best: Option<usize> = None;
        for (i, shell) in shells.iter().enumerate() {
            if !shell.contains_point(probe) {
                continue;
            }
            if best.is_none() || shell.area() < shells[best.unwrap()].area() {
                best = Some(i);
            }
        }
        if let Some(i) = best {
            assigned[i].push(hole);
        }
    }
    let mut out: Vec<Region> = Vec::with_capacity(shells.len());
    for (shell, hs) in shells.into_iter().zip(assigned) {
        out.push(Region::new(shell, hs, tol)?);
    }
    // Deterministic order, independent of how the arrangement was traversed.
    out.sort_by(|x, y| {
        y.area()
            .total_cmp(&x.area())
            .then(x.aabb().min.x.total_cmp(&y.aabb().min.x))
            .then(x.aabb().min.y.total_cmp(&y.aabb().min.y))
    });
    Ok(out)
}

/// Union of many regions, folded pairwise. Disjoint parts stay separate.
pub fn union_all(regions: &[Region], tol: &Tolerances) -> GeomResult<Vec<Region>> {
    // Two regions whose boxes, grown by twice the vertex tolerance, do not overlap cannot touch:
    // their union is the two of them unchanged, which is what the boolean answers (two regions), so
    // the pair is kept apart without running it. Same result, without the quadratic booleans of a
    // plan full of separate walls. Each region's box is computed once (a region's box is not free).
    let pad = 2.0 * tol.vertex + 1e-9;
    let apart = |a: &crate::curve::Aabb2, b: &crate::curve::Aabb2| {
        a.max.x + pad < b.min.x || b.max.x + pad < a.min.x || a.max.y + pad < b.min.y || b.max.y + pad < a.min.y
    };
    let mut acc: Vec<(Region, crate::curve::Aabb2)> = Vec::new();
    for r in regions {
        let mut current = r.clone();
        let mut current_box = current.aabb();
        let mut rest: Vec<(Region, crate::curve::Aabb2)> = Vec::new();
        for (existing, existing_box) in acc.drain(..) {
            if apart(&current_box, &existing_box) {
                rest.push((existing, existing_box));
                continue;
            }
            let merged = boolean(&current, &existing, BoolOp::Union, tol)?;
            if merged.len() == 1 {
                current = merged.into_iter().next().expect("checked length");
                current_box = current.aabb();
            } else {
                rest.push((existing, existing_box));
            }
        }
        rest.push((current, current_box));
        acc = rest;
    }
    let mut acc: Vec<Region> = acc.into_iter().map(|(r, _)| r).collect();
    // Deterministic order.
    acc.sort_by(|x, y| {
        y.area()
            .total_cmp(&x.area())
            .then(x.aabb().min.x.total_cmp(&y.aabb().min.x))
            .then(x.aabb().min.y.total_cmp(&y.aabb().min.y))
    });
    Ok(acc)
}

/// Total area of a set of regions (the quantity a host's cut removes).
pub fn total_area(regions: &[Region]) -> f64 {
    regions.iter().map(|r| r.area()).sum()
}

impl Region {
    pub fn union(&self, other: &Region, tol: &Tolerances) -> GeomResult<Vec<Region>> {
        boolean(self, other, BoolOp::Union, tol)
    }
    pub fn intersection(&self, other: &Region, tol: &Tolerances) -> GeomResult<Vec<Region>> {
        boolean(self, other, BoolOp::Intersection, tol)
    }
    pub fn difference(&self, other: &Region, tol: &Tolerances) -> GeomResult<Vec<Region>> {
        boolean(self, other, BoolOp::Difference, tol)
    }
    /// `self` clipped to `bounds` — the reference's rule for an opening that runs past its host (rules 4 and 5).
    pub fn clipped_to(&self, bounds: &Region, tol: &Tolerances) -> GeomResult<Vec<Region>> {
        boolean(self, bounds, BoolOp::Intersection, tol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::Arc;
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

    // ---------------------------------------------------------------- basics

    #[test]
    fn union_of_two_overlapping_rectangles() {
        let a = rect(0.0, 0.0, 2.0, 2.0);
        let b = rect(1.0, 1.0, 3.0, 3.0);
        let u = a.union(&b, &tol()).unwrap();
        assert_eq!(u.len(), 1);
        near(u[0].area(), 4.0 + 4.0 - 1.0);
        // L-shaped outline: 8 edges, no holes.
        assert_eq!(u[0].curve_count(), 8);
        assert_eq!(u[0].holes().len(), 0);
        near(u[0].outer_perimeter(), 12.0);
    }

    #[test]
    fn intersection_of_two_overlapping_rectangles() {
        let a = rect(0.0, 0.0, 2.0, 2.0);
        let b = rect(1.0, 1.0, 3.0, 3.0);
        let i = a.intersection(&b, &tol()).unwrap();
        assert_eq!(i.len(), 1);
        near(i[0].area(), 1.0);
        near(i[0].outer_perimeter(), 4.0);
    }

    #[test]
    fn difference_cuts_a_notch_and_keeps_the_full_bounding_box() {
        // This is the corner-join shape: the butt wall is NOT axially trimmed, it is notched, so its bbox is unchanged
        // while its area drops.
        let a = rect(0.0, 0.0, 4.0, 1.0);
        let b = rect(3.0, 0.5, 5.0, 2.0);
        let d = a.difference(&b, &tol()).unwrap();
        assert_eq!(d.len(), 1);
        near(d[0].area(), 4.0 - 0.5);
        let bb = d[0].aabb();
        near(bb.min.x, 0.0);
        near(bb.min.y, 0.0);
        near(bb.max.x, 4.0);
        near(bb.max.y, 1.0);
        assert_eq!(d[0].curve_count(), 6);
    }

    #[test]
    fn difference_that_splits_the_operand_returns_two_regions() {
        let a = rect(0.0, 0.0, 4.0, 1.0);
        let b = rect(1.5, -1.0, 2.5, 2.0);
        let d = a.difference(&b, &tol()).unwrap();
        assert_eq!(d.len(), 2, "{d:?}");
        near(total_area(&d), 1.5 + 1.5);
        near(d[0].area(), 1.5);
        near(d[1].area(), 1.5);
        // Deterministic order: equal areas, so sorted by bbox min x.
        near(d[0].aabb().min.x, 0.0);
        near(d[1].aabb().min.x, 2.5);
    }

    #[test]
    fn difference_of_an_enclosed_hole_makes_a_region_with_a_hole() {
        let a = rect(0.0, 0.0, 4.0, 4.0);
        let b = rect(1.0, 1.0, 2.0, 2.0);
        let d = a.difference(&b, &tol()).unwrap();
        assert_eq!(d.len(), 1);
        near(d[0].area(), 15.0);
        assert_eq!(d[0].holes().len(), 1);
        near(d[0].hole_perimeter(), 4.0);
        assert!(!d[0].contains_point(Vec2::new(1.5, 1.5)));
        assert!(d[0].contains_point(Vec2::new(3.0, 3.0)));
    }

    #[test]
    fn disjoint_operands_behave() {
        let a = rect(0.0, 0.0, 1.0, 1.0);
        let b = rect(5.0, 5.0, 6.0, 6.0);
        let u = a.union(&b, &tol()).unwrap();
        assert_eq!(u.len(), 2);
        near(total_area(&u), 2.0);
        assert!(a.intersection(&b, &tol()).unwrap().is_empty());
        let d = a.difference(&b, &tol()).unwrap();
        assert_eq!(d.len(), 1);
        near(d[0].area(), 1.0);
    }

    #[test]
    fn fully_contained_operand_behaves() {
        let big = rect(0.0, 0.0, 10.0, 10.0);
        let small = rect(2.0, 2.0, 3.0, 3.0);
        near(total_area(&big.union(&small, &tol()).unwrap()), 100.0);
        near(total_area(&small.union(&big, &tol()).unwrap()), 100.0);
        near(total_area(&big.intersection(&small, &tol()).unwrap()), 1.0);
        near(total_area(&small.difference(&big, &tol()).unwrap()), 0.0);
        let hole = big.difference(&small, &tol()).unwrap();
        assert_eq!(hole.len(), 1);
        near(hole[0].area(), 99.0);
        assert_eq!(hole[0].holes().len(), 1);
    }

    // ------------------------------------------------- coincident boundaries

    #[test]
    fn a_rectangle_inside_another_sharing_three_edges_unions_cleanly() {
        // The case the opening sequence hits: a clipped opening that lands
        // inside an existing cut and shares part of its boundary.
        let band = rect(0.0, 0.5, 6.0, 1.5);
        let tip = rect(5.5, 0.5, 6.0, 1.5);
        let u = band.union(&tip, &tol()).unwrap();
        assert_eq!(u.len(), 1, "{u:?}");
        near(u[0].area(), 6.0);
        near(u[0].outer_perimeter(), 14.0);
    }

    #[test]
    fn two_rectangles_sharing_one_full_edge_union_into_one() {
        let left = rect(0.0, 0.0, 1.0, 1.0);
        let right = rect(1.0, 0.0, 2.0, 1.0);
        let u = left.union(&right, &tol()).unwrap();
        assert_eq!(u.len(), 1, "{u:?}");
        near(u[0].area(), 2.0);
        near(u[0].outer_perimeter(), 6.0);
    }

    #[test]
    fn difference_along_a_shared_edge_keeps_the_other_half() {
        let whole = rect(0.0, 0.0, 2.0, 1.0);
        let half = rect(0.0, 0.0, 1.0, 1.0);
        let d = whole.difference(&half, &tol()).unwrap();
        assert_eq!(d.len(), 1, "{d:?}");
        near(d[0].area(), 1.0);
        near(d[0].aabb().min.x, 1.0);
    }

    // ------------------------------------------------------------ with arcs

    #[test]
    fn union_of_two_unit_discs_at_distance_one_is_exact() {
        // Classic lens: |A u B| = 2*pi - lens, lens = 2*r^2*(theta - sin theta)/2
        // with half-angle pi/3 => lens = 2*pi/3 - sqrt(3)/2.
        let a = disc(0.0, 0.0, 1.0);
        let b = disc(1.0, 0.0, 1.0);
        let lens = 2.0 * (PI / 3.0) - (3.0_f64).sqrt() / 2.0;
        let u = a.union(&b, &tol()).unwrap();
        assert_eq!(u.len(), 1, "{u:?}");
        near_eps(u[0].area(), 2.0 * PI - lens, 1e-12);
        let i = a.intersection(&b, &tol()).unwrap();
        assert_eq!(i.len(), 1);
        near_eps(i[0].area(), lens, 1e-12);
        let d = a.difference(&b, &tol()).unwrap();
        assert_eq!(d.len(), 1);
        near_eps(d[0].area(), PI - lens, 1e-12);
        // Still arcs, never polylines. The arrangement splits each source disc
        // (two half arcs) into four lens pieces, and `simplified` merges the
        // co-circular ones back, so the lens is exactly two arcs - one per
        // circle - with the exact perimeter 2 * (2 pi / 3).
        assert_eq!(i[0].outer().curve_count(), 2);
        assert_eq!(i[0].outer().arc_count(), 2);
        near_eps(i[0].outer_perimeter(), 2.0 * (2.0 * PI / 3.0), 1e-12);
    }

    #[test]
    fn a_disc_punched_out_of_a_square_is_a_region_with_an_arc_hole() {
        let sq = rect(0.0, 0.0, 10.0, 10.0);
        let hole = disc(5.0, 5.0, 2.0);
        let d = sq.difference(&hole, &tol()).unwrap();
        assert_eq!(d.len(), 1);
        near_eps(d[0].area(), 100.0 - 4.0 * PI, 1e-12);
        assert_eq!(d[0].holes().len(), 1);
        near_eps(d[0].hole_perimeter(), 4.0 * PI, 1e-12);
        assert_eq!(d[0].holes()[0].arc_count(), 2);
    }

    #[test]
    fn half_disc_cut_from_a_square_mixes_lines_and_arcs() {
        // Disc centred on the square's right edge: the intersection is the
        // half disc, area pi r^2 / 2, perimeter pi r + 2r.
        let sq = rect(0.0, 0.0, 4.0, 4.0);
        let d = disc(4.0, 2.0, 1.0);
        let i = sq.intersection(&d, &tol()).unwrap();
        assert_eq!(i.len(), 1, "{i:?}");
        near_eps(i[0].area(), PI / 2.0, 1e-12);
        near_eps(i[0].outer_perimeter(), PI + 2.0, 1e-12);
        // One arc: the two half-arc pieces of the source disc are merged back.
        assert_eq!(i[0].outer().arc_count(), 1);
        assert_eq!(i[0].outer().curve_count(), 2);
    }

    #[test]
    fn a_curved_band_minus_a_rectangle_keeps_the_arc_faces() {
        use crate::band::offset_band;
        let arc = Arc::from_center_angles(Vec2::new(0.0, 0.0), 5.0, 0.0, PI / 2.0, true).unwrap();
        let band = offset_band(&[Curve::Arc(arc)], 0.15, 0.15, &tol()).unwrap();
        let full = band.area();
        // Cut a notch out of the band's start, 0.4 m along X.
        let notch = rect(4.8, -1.0, 5.2, 0.4);
        let d = band.difference(&notch, &tol()).unwrap();
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(d[0].area() < full && d[0].area() > full * 0.5);
        assert!(d[0].outer().arc_count() >= 2, "arc faces must survive the cut");
    }

    // ------------------------------- (MEASURED round 9)

    #[test]
    fn measured_opening_sequence_of_the_specification() {
        let host = rect(0.0, 0.0, 6.0, 3.0);
        let gross = 18.0;
        near(host.area(), gross);

        let clip = |x0: f64, z0: f64, x1: f64, z1: f64| -> Vec<Region> {
            rect(x0, z0, x1, z1).clipped_to(&host, &tol()).unwrap()
        };
        let face_area = |cuts: &[Region]| -> f64 {
            gross - total_area(&union_all(cuts, &tol()).unwrap())
        };

        // window 1.000-2.200 x 0.900-2.100 => 16.560
        let window = clip(1.0, 0.9, 2.2, 2.1);
        near(face_area(&window), 16.560);

        // + door 3.000-3.900 x 0.000-2.100 => 14.670
        let door = clip(3.0, 0.0, 3.9, 2.1);
        let mut cuts: Vec<Region> = Vec::new();
        cuts.extend(window.iter().cloned());
        cuts.extend(door.iter().cloned());
        near(face_area(&cuts), 14.670);

        // delete the window => 16.110 (the cut is not baked in)
        near(face_area(&door), 16.110);

        // overlapping opening 3.500-4.400: only the new 0.500 x 2.100 comes off
        let overlap = clip(3.5, 0.0, 4.4, 2.1);
        let mut cuts: Vec<Region> = door.clone();
        cuts.extend(overlap.iter().cloned());
        near(face_area(&cuts), 15.060);

        // wider than the wall: -1.000-7.000 x 0.500-1.500, clipped to 0..6
        let wide = clip(-1.0, 0.5, 7.0, 1.5);
        assert_eq!(wide.len(), 1);
        near(wide[0].area(), 6.0);
        near(wide[0].aabb().min.x, 0.0);
        near(wide[0].aabb().max.x, 6.0);
        cuts.extend(wide.iter().cloned());
        near(face_area(&cuts), 10.460);

        // hanging off the end: 5.500-6.500 clipped to 5.500-6.000, already cut
        let hanging = clip(5.5, 0.5, 6.5, 1.5);
        assert_eq!(hanging.len(), 1);
        near(hanging[0].area(), 0.5);
        cuts.extend(hanging.iter().cloned());
        near(face_area(&cuts), 10.460);

        // above the wall top: 4.600-5.200 x 2.800-3.400 clipped to z <= 3.000
        let above = clip(4.6, 2.8, 5.2, 3.4);
        assert_eq!(above.len(), 1);
        near(above[0].area(), 0.12);
        cuts.extend(above.iter().cloned());
        near(face_area(&cuts), 10.340);

        // 2 mm opening inside an existing cut removes nothing
        let tiny = clip(5.0, 1.0, 5.002, 1.002);
        assert_eq!(tiny.len(), 1);
        cuts.extend(tiny.iter().cloned());
        near(face_area(&cuts), 10.340);

        // Volume = Area x width still holds (rule 1), width 0.200
        near(10.340 * 0.2, 2.068);
    }

    #[test]
    fn an_opening_entirely_outside_the_host_clips_to_nothing() {
        let host = rect(0.0, 0.0, 6.0, 3.0);
        let away = rect(7.0, 0.5, 8.0, 1.5);
        assert!(away.clipped_to(&host, &tol()).unwrap().is_empty());
    }

    #[test]
    fn union_never_double_counts_however_the_openings_are_ordered() {
        let a = rect(3.0, 0.0, 3.9, 2.1);
        let b = rect(3.5, 0.0, 4.4, 2.1);
        let c = rect(0.0, 0.5, 6.0, 1.5);
        let expected = total_area(&union_all(&[a.clone(), b.clone(), c.clone()], &tol()).unwrap());
        for order in [
            [a.clone(), c.clone(), b.clone()],
            [b.clone(), a.clone(), c.clone()],
            [c.clone(), b.clone(), a.clone()],
        ] {
            near(total_area(&union_all(&order, &tol()).unwrap()), expected);
        }
        // And the union is strictly less than the sum of the parts.
        assert!(expected < a.area() + b.area() + c.area());
    }

    // ----------------------------------- corner geometry

    /// The measured corner of scenario: wall type width 0.02 + 0.08 + 0.15 + 0.0125 = **0.2625**, half width 0.13125. J1
    /// runs (0,0)->(5,0) and is the through wall (created first), J2 runs (5,0)->(5,4) and butts.
    ///
    /// **Finding, stated plainly:** subtracting the through wall's body as a
    /// *single* rectangle reproduces the measured **area** loss exactly
    /// (`h_q * cot(45 deg)` of axial length, i.e. `0.2625 * 0.13125` m2) but
    /// NOT the measured bounding box: because the through body spans the butt
    /// wall's full width at the corner, the difference is an axial trim and
    /// `bbox.min.y` comes out at 0.13125, which is exactly the residual still
    /// reported in `report-round1.json`. So this boolean alone does **not**
    /// close those fields - see the next test for what does.
    #[test]
    fn butt_wall_single_rectangle_difference_is_an_axial_trim() {
        let half = 0.13125_f64;
        let width = 2.0 * half;
        let through = rect(0.0, -half, 5.0 + half, half);
        let butt_full = rect(5.0 - half, 0.0, 5.0 + half, 4.0);
        near(butt_full.area(), width * 4.0);

        let trimmed = butt_full.difference(&through, &tol()).unwrap();
        assert_eq!(trimmed.len(), 1, "{trimmed:?}");
        let lost = width * half;
        near(lost, 0.2625 * 0.13125);
        near(trimmed[0].area(), width * 4.0 - lost);
        // Area matches the reference; the bbox does not.
        let bb = trimmed[0].aabb();
        near(bb.min.y, half);
        near(bb.max.y, 4.0);
        assert_eq!(trimmed[0].curve_count(), 4);
    }

    /// What **does** reproduce the reference's notched corner: per-layer penetration.
    ///
    /// When the through wall penetrates each compound-structure layer to its
    /// own depth, the subtracted region no longer spans the butt wall's full
    /// width, so the result is **notched**: `bbox.min.y` stays at 0, which is
    /// the measured value, and the area loss is `sum(layer_width *
    /// penetration_depth)`.
    ///
    /// **The depths themselves are NOT measured yet** - the per-layer
    /// penetration rule is listed as open in §5 - so this test fixes the
    /// *shape behaviour* of the primitive with illustrative depths, not a
    /// parity claim. The geometry kernel is ready; the missing input is an
    /// observation of the depths.
    #[test]
    fn per_layer_fingers_produce_a_notch_that_keeps_the_full_bounding_box() {
        // The measured layer widths of AST-ASYM-2625, exterior face first.
        let layers = [0.02_f64, 0.08, 0.15, 0.0125];
        let total: f64 = layers.iter().sum();
        near(total, 0.2625);
        let half = total / 2.0;
        // Illustrative alternating penetration depths (ASSUMED, not measured):
        // finishes stop at the face, the core penetrates fully.
        let depths = [0.0_f64, half, half, 0.0];

        let butt_full = rect(5.0 - half, 0.0, 5.0 + half, 4.0);
        let mut fingers: Vec<Region> = Vec::new();
        let mut x = 5.0 - half;
        let mut expected_loss = 0.0;
        for (w, d) in layers.iter().zip(depths.iter()) {
            if *d > 0.0 {
                fingers.push(rect(x, 0.0, x + w, *d));
                expected_loss += w * d;
            }
            x += w;
        }
        let penetration = union_all(&fingers, &tol()).unwrap();
        assert_eq!(penetration.len(), 1, "adjacent layer fingers must merge");

        let notched = butt_full.difference(&penetration[0], &tol()).unwrap();
        assert_eq!(notched.len(), 1, "{notched:?}");
        near(notched[0].area(), total * 4.0 - expected_loss);
        // The measured property: the butt wall keeps its full extent.
        let bb = notched[0].aabb();
        near(bb.min.y, 0.0);
        near(bb.min.x, 5.0 - half);
        near(bb.max.x, 5.0 + half);
        near(bb.max.y, 4.0);
        // A notched rectangle has 8 plan corners, so the generalised prism
        // reports 16 vertices / 10 faces / 24 edges instead of 8 / 6 / 12 -
        // the direction the reference's higher counts come from.
        assert_eq!(notched[0].curve_count(), 8);
        assert!(notched[0].contains_point(Vec2::new(5.0 - half + 0.01, 0.01)));
        assert!(!notched[0].contains_point(Vec2::new(5.0, 0.01)));
    }

    /// The mitre wedge of §5: at a non-perpendicular join the reference removes
    /// a wedge, so `Volume < Area * width`. A half-plane cut models the wedge,
    /// and `difference` against it removes exactly the triangle.
    #[test]
    fn angled_join_mitre_wedge_is_a_triangular_difference() {
        let half = 0.1_f64;
        let body = rect(0.0, -half, 4.0, half);
        near(body.area(), 0.8);
        // A 45-degree mitre at the far end: cut the triangle beyond the line
        // x = 4 - (y + half).
        let wedge = Region::from_loop(
            Polycurve::polygon(
                &[
                    Vec2::new(4.0 - 2.0 * half, half),
                    Vec2::new(4.0, half),
                    Vec2::new(4.0, -half),
                ],
                &tol(),
            )
            .unwrap(),
        );
        let d = body.difference(&wedge, &tol()).unwrap();
        assert_eq!(d.len(), 1, "{d:?}");
        let triangle = 0.5 * (2.0 * half) * (2.0 * half);
        near(d[0].area(), 0.8 - triangle);
        // The bbox is unchanged: the wedge is a chamfer, not a trim.
        near(d[0].aabb().max.x, 4.0);
        // The wedge swallows the whole right edge, so the chamfered body is a
        // quadrilateral: (0,-h) (4,-h) (3.8,h) (0,h).
        assert_eq!(d[0].curve_count(), 4);
    }

    // ------------------------------------------------------- adversarial set

    #[test]
    fn nearly_tangent_overlaps_do_not_blow_up() {
        // Two discs whose boundaries come within a few tolerances of tangency,
        // approached from both sides. The area must stay monotone and the
        // operation must not fail.
        let t = tol();
        let r = 1.0;
        let mut previous = f64::INFINITY;
        let mut deltas = [1e-2, 1e-3, 1e-4, t.vertex * 4.0, t.vertex, t.vertex / 4.0];
        deltas.sort_by(|x, y| y.total_cmp(x));
        for delta in deltas {
            let a = disc(0.0, 0.0, r);
            let b = disc(2.0 * r - delta, 0.0, r);
            let i = a.intersection(&b, &t).unwrap();
            let area = total_area(&i);
            assert!(area >= 0.0 && area < 0.01, "delta {delta}: area {area}");
            assert!(area <= previous + 1e-9, "not monotone at delta {delta}");
            previous = area;
            let u = a.union(&b, &t).unwrap();
            // Inclusion-exclusion must hold, but only down to the tolerance:
            // at near tangency the union's cusp pieces fall below
            // ShortCurveTolerance and are dropped (see `stitch`). The residual
            // is bounded by that cusp, not by the shapes' size.
            near_eps(total_area(&u), 2.0 * PI * r * r - area, 1e-5);
        }
    }

    #[test]
    fn separated_and_touching_discs_are_handled() {
        let t = tol();
        // Strictly separated: union keeps two regions, intersection is empty.
        let a = disc(0.0, 0.0, 1.0);
        let far = disc(2.5, 0.0, 1.0);
        assert_eq!(a.union(&far, &t).unwrap().len(), 2);
        assert!(a.intersection(&far, &t).unwrap().is_empty());
        // Externally tangent within tolerance: the intersection has no area.
        let touching = disc(2.0, 0.0, 1.0);
        let i = a.intersection(&touching, &t).unwrap();
        near_eps(total_area(&i), 0.0, 1e-12);
    }

    #[test]
    fn slivers_below_the_short_curve_tolerance_do_not_produce_invalid_loops() {
        // A cut that leaves a sliver thinner than ShortCurveTolerance
        // (0.00078 m): the result must either drop it or stay valid, never
        // return a loop with a degenerate curve.
        let t = tol();
        let a = rect(0.0, 0.0, 1.0, 1.0);
        let b = rect(0.0005, 0.0, 1.0, 1.0);
        let regions = a.difference(&b, &t).unwrap();
        // Either the sliver was dropped as sub-tolerance geometry, or it came
        // back as a valid loop - never as a loop with a degenerate curve.
        for r in &regions {
            for lp in r.loops() {
                for c in lp.curves() {
                    assert!(c.length() >= t.short_curve, "degenerate curve survived");
                }
            }
        }
        // Here the 0.5 mm sliver is below ShortCurveTolerance (0.78 mm), so it
        // is dropped and the difference is empty. The lost area is 5e-4 m2,
        // bounded by (tolerance x extent); this is the kernel's only
        // area-losing path and it is deliberate.
        assert!(regions.is_empty(), "{regions:?}");
    }

    /// Scenario -L`, step `acute-45-from-corner`, measured in round 5: wall type width 0.200 (half 0.100), height 3.000,
    /// `a_h` (0,0)->(5,0) is the through wall, `a_d` (5,0)->(8,3) butts at 135 degrees. The reference reports for `a_d`:
    /// **Volume 2.4607315985291818**, Area 12.603658, 6 faces.
    ///
    /// **Result: one `difference` reproduces that volume exactly.** Taking the butt wall's *untrimmed* band and subtracting
    /// the through wall's extended body gives plan area 0.8202438662 and volume 2.4607315985 - the reference value to every
    /// digit it reports. The axial-trim model reaches only 0.8402438662 (volume 2.5207315985), which is exactly the 0.06 m3
    /// residual still reported for this scenario.
    #[test]
    fn angled_join_difference_reproduces_the_measured_volume_exactly() {
        use crate::band::offset_band;
        let t = tol();
        let h = 0.1_f64;
        let height = 3.0_f64;
        // Through wall, extended at the corner by h * tan(gamma/2), gamma = 135 deg.
        let extension = h * (67.5_f64).to_radians().tan();
        near(extension, 0.24142135623730956);
        let through = rect(0.0, -h, 5.0 + extension, h);

        // Butt wall, untrimmed band along its centreline.
        let axis = [Curve::line(Vec2::new(5.0, 0.0), Vec2::new(8.0, 3.0))];
        let butt_full = offset_band(&axis, h, h, &t).unwrap();
        near(butt_full.area(), 3.0 * (2.0_f64).sqrt() * 2.0 * h);

        let joined = butt_full.difference(&through, &t).unwrap();
        assert_eq!(joined.len(), 1, "{joined:?}");
        // The measured volume, to the last digit the reference printed.
        near_eps(joined[0].area() * height, 2.4607315985291818, 5e-13);
        // And what the axial-trim model gives instead: 0.06 m3 too much.
        let axial_trim = h * (67.5_f64).to_radians().tan().recip();
        near_eps(
            (butt_full.area() - axial_trim * 2.0 * h) * height,
            2.520731598529185,
            5e-13,
        );
        // The gap between the two models is the 0.06 m3 residual (the literal
        // subtraction of the reference's printed digits carries 3e-15 of its
        // own rounding).
        near_eps(2.520731598529185 - 2.4607315985291818, 0.06, 1e-14);
        // The mitre wedge is exactly w^2 / 2 of plan area.
        near_eps(0.06 / height, 0.2 * 0.2 / 2.0, 1e-15);
    }

    /// The other half of the angled-join residual: the **bounding box**.
    ///
    /// The reference reports `a_d` bbox min = (4.9292893218813445,
    /// -0.07071067811865477). Those are exactly the corners of the butt wall's
    /// **untrimmed** band - this test computes them from the scenario geometry
    /// and checks they agree. The joined (differenced) solid's bbox min is
    /// (4.9585786438, 0.1), because the overlap removal eats the whole corner.
    ///
    /// So in the reference, bbox and quantities do **not** come from the same
    /// solid: Area/Volume follow the joined body while the reported bounding
    /// box follows the wall's own unjoined body. The same holds for every
    /// perpendicular case in the failing set (`J2` min y = 0, `thin_V` min y =
    /// 20, `B_H` min x = 25 - all unjoined starts). That is an engine rule, not
    /// a geometry gap: take the bbox from the unjoined body.
    #[test]
    fn the_reference_bbox_is_the_unjoined_body_not_the_joined_one() {
        use crate::band::offset_band;
        let t = tol();
        let h = 0.1_f64;
        let axis = [Curve::line(Vec2::new(5.0, 0.0), Vec2::new(8.0, 3.0))];
        let butt_full = offset_band(&axis, h, h, &t).unwrap();
        let unjoined = butt_full.aabb();
        near_eps(unjoined.min.x, 4.9292893218813445, 1e-12);
        near_eps(unjoined.min.y, -0.07071067811865477, 1e-12);

        let extension = h * (67.5_f64).to_radians().tan();
        let through = rect(0.0, -h, 5.0 + extension, h);
        let joined = butt_full.difference(&through, &t).unwrap();
        let jb = joined[0].aabb();
        near_eps(jb.min.y, 0.1, 1e-12);
        assert!(jb.min.x > unjoined.min.x);
    }

    #[test]
    fn perpendicular_corner_follows_the_same_two_rules() {
        let t = tol();
        let half = 0.13125_f64;
        let width = 2.0 * half;
        let through = rect(0.0, -half, 5.0 + half, half);
        let butt_full = rect(5.0 - half, 0.0, 5.0 + half, 4.0);
        // Unjoined bbox: the measured value of J2's bbox min y.
        near(butt_full.aabb().min.y, 0.0);
        // Joined quantity: the measured area loss.
        let joined = butt_full.difference(&through, &t).unwrap();
        assert_eq!(joined.len(), 1);
        near(joined[0].area(), width * 4.0 - width * half);
        near(width * half, 0.2625 * 0.13125);
    }

    #[test]
    fn boolean_results_are_bit_reproducible() {
        let a = disc(0.37, -1.2, 2.5);
        let b = rect(0.0, -2.0, 4.0, 0.0);
        let first = a.union(&b, &tol()).unwrap();
        let first_key: Vec<u64> = first.iter().map(|r| r.area().to_bits()).collect();
        for _ in 0..8 {
            let again = a.union(&b, &tol()).unwrap();
            let key: Vec<u64> = again.iter().map(|r| r.area().to_bits()).collect();
            assert_eq!(key, first_key);
        }
    }

    #[test]
    fn union_is_commutative_in_area_for_arcs_and_lines() {
        let a = disc(0.0, 0.0, 1.5);
        let b = rect(0.0, 0.0, 2.0, 2.0);
        let ab = total_area(&a.union(&b, &tol()).unwrap());
        let ba = total_area(&b.union(&a, &tol()).unwrap());
        near_eps(ab, ba, 1e-12);
        let iab = total_area(&a.intersection(&b, &tol()).unwrap());
        let iba = total_area(&b.intersection(&a, &tol()).unwrap());
        near_eps(iab, iba, 1e-12);
        // Inclusion-exclusion must hold: |A| + |B| = |A u B| + |A n B|.
        near_eps(a.area() + b.area(), ab + iab, 1e-12);
    }

    /// BooleanType 0 union, 1 difference, 2 intersection (above 2 refused), and the inversion table of the result.
    #[test]
    fn the_reference_boolean_codes() {
        for op in [BoolOp::Union, BoolOp::Difference, BoolOp::Intersection] {
            assert_eq!(BoolOp::from_reference_code(op.reference_code()), Some(op));
        }
        assert_eq!(BoolOp::from_reference_code(3), None);
        assert!(BoolOp::Union.result_inverted(false, true));
        assert!(!BoolOp::Difference.result_inverted(true, true) && BoolOp::Difference.result_inverted(true, false));
        assert!(!BoolOp::Intersection.result_inverted(true, false) && BoolOp::Intersection.result_inverted(true, true));
    }
}
