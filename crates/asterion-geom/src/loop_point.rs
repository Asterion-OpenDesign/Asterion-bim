//! Point classification against a closed polygon loop, as the reference's loop operations do it.
//!
//! * An open or empty loop answers 0; a point outside the loop envelope grown by `tol` answers 0 at once; every edge of the
//!   chain is visited in order and a distance from the point to the edge `<= tol` answers 2. The polygons here have
//!   straight two-point edges, so it never arises. The tolerance is therefore used ONLY for the on-boundary state and the
//!   envelope; the inside state is exact.
//!
//! # What is NOT specified (documented UNKNOWN, never filled with a guess)
//!
//! * Here: the distance is the unsigned Euclidean one; a zero-length segment is skipped by [`pnt_on_polygon`]; the grown
//!   box is closed (a point exactly on it is not rejected by the envelope test, then the distance test decides).
//! * the `rect` argument and the sag-triangle branch (curved edges with interior samples).

use crate::vec::Vec2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointState {
    Outside = 0,
    Inside = 1,
    OnBoundary = 2,
}

/// Even-odd parity of the segments of the closed polygon `poly` crossed by the ray from `q` toward `-u`. Exact comparisons,
/// no tolerance.
pub fn point_inside_polygon(q: Vec2, poly: &[Vec2]) -> bool {
    let n = poly.len();
    if n < 2 {
        return false;
    }
    let mut odd = false;
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        // half-open ordinate range: (q.v <= A.v) == (B.v < q.v)
        if (q.y <= a.y) == (b.y < q.y) {
            // the two ordinates differ here, so the division is defined
            let x = a.x + (q.y - a.y) * (a.x - b.x) / (a.y - b.y);
            if x < q.x {
                odd = !odd;
            }
        }
    }
    odd
}

/// The distance and the parameter `(t, d)` of the projection of `q` onto the INFINITE line through `a` with direction
/// `dir`: `t` in units of `dir` from `a`, `d` the unsigned distance. `None` for a zero direction (UNKNOWN: the reference
/// operation's answer 1).
fn project_point_to_line(q: Vec2, a: Vec2, dir: Vec2) -> Option<(f64, f64)> {
    let l2 = dir.dot(dir);
    if !(l2 > 0.0) {
        return None;
    }
    let w = q - a;
    let t = w.dot(dir) / l2;
    let d = w.cross(dir).abs() / l2.sqrt();
    Some((t, d))
}

/// True as soon as one segment `(A, B)` passes the four non-strict box conditions `min(A.u,B.u) <= q.u + tol`,
/// `max(A.u,B.u) >= q.u - tol`, `min(A.v,B.v) <= q.v + tol`, `max(A.v,B.v) >= q.v - tol` AND the projection of `q` onto its
/// line gives a distance `d <= tol` with the segment parameter in `[-tol, 1 + tol]`.
pub fn point_on_polygon(q: Vec2, poly: &[Vec2], tol: f64) -> bool {
    let n = poly.len();
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        if a.x.min(b.x) <= q.x + tol
            && a.x.max(b.x) >= q.x - tol
            && a.y.min(b.y) <= q.y + tol
            && a.y.max(b.y) >= q.y - tol
        {
            if let Some((t, d)) = project_point_to_line(q, a, b - a) {
                if d <= tol && -tol <= t && t <= 1.0 + tol {
                    return true;
                }
            }
        }
    }
    false
}

/// Distance from `q` to the bounded segment `a`-`b` (nearest point of the segment).
fn distance_to_segment(q: Vec2, a: Vec2, b: Vec2) -> f64 {
    let d = b - a;
    let l2 = d.dot(d);
    if !(l2 > 0.0) {
        return (q - a).length();
    }
    let t = ((q - a).dot(d) / l2).clamp(0.0, 1.0);
    (q - Vec2::new(a.x + d.x * t, a.y + d.y * t)).length()
}

/// For a closed loop of straight edges: 0 outside, 1 inside, 2 on the boundary.
pub fn classify_point(q: Vec2, poly: &[Vec2], tol: f64) -> PointState {
    // (a) an empty loop answers 0
    if poly.len() < 2 {
        return PointState::Outside;
    }
    // (b) outside the envelope grown by tol: 0 at once
    let (mut lo, mut hi) = (poly[0], poly[0]);
    for p in poly {
        lo = Vec2::new(lo.x.min(p.x), lo.y.min(p.y));
        hi = Vec2::new(hi.x.max(p.x), hi.y.max(p.y));
    }
    if q.x < lo.x - tol || q.x > hi.x + tol || q.y < lo.y - tol || q.y > hi.y + tol {
        return PointState::Outside;
    }
    // (c) every edge in chain order: distance <= tol is the boundary state
    let n = poly.len();
    for i in 0..n {
        if distance_to_segment(q, poly[i], poly[(i + 1) % n]) <= tol {
            return PointState::OnBoundary;
        }
    }
    // (d) the exact parity decides inside / outside
    if point_inside_polygon(q, poly) {
        PointState::Inside
    } else {
        PointState::Outside
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Containment {
    /// `Err 0`, `*out = true`.
    Contains,
    /// `Err 0`, `*out = false`.
    DoesNotContain,
    /// `*out = false`: coincident or fully shared boundary, undecided.
    Undecided,
}

/// For two closed loops of straight edges.
///
/// Pass 1 walks the edges of `inner` in loop order and, for each, its stored points (start, end: a
/// straight edge has no interior samples); the FIRST point whose classification against `outer` is
/// not on-boundary decides (outside: false, inside: true); on-boundary points are skipped. If every
/// point is on the boundary, pass 2 classifies the MIDPOINT of each edge in order with the same
/// rule; with no decision the answer is [`Containment::Undecided`].
pub fn loop_encloses_loop(outer: &[Vec2], inner: &[Vec2], tol: f64) -> Containment {
    let n = inner.len();
    let decide = |p: Vec2| -> Option<Containment> {
        match classify_point(p, outer, tol) {
            PointState::OnBoundary => None,
            PointState::Inside => Some(Containment::Contains),
            PointState::Outside => Some(Containment::DoesNotContain),
        }
    };
    for i in 0..n {
        let (a, b) = (inner[i], inner[(i + 1) % n]);
        for p in [a, b] {
            if let Some(r) = decide(p) {
                return r;
            }
        }
    }
    for i in 0..n {
        let (a, b) = (inner[i], inner[(i + 1) % n]);
        let mid = Vec2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        if let Some(r) = decide(mid) {
            return r;
        }
    }
    Containment::Undecided
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<Vec2> {
        vec![Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0), Vec2::new(4.0, 4.0), Vec2::new(0.0, 4.0)]
    }

    #[test]
    fn parity_counts_the_crossings_toward_minus_u() {
        let sq = square();
        assert!(point_inside_polygon(Vec2::new(2.0, 2.0), &sq));
        assert!(!point_inside_polygon(Vec2::new(5.0, 2.0), &sq));
        assert!(!point_inside_polygon(Vec2::new(-1.0, 2.0), &sq));
        assert!(!point_inside_polygon(Vec2::new(2.0, 5.0), &sq));
    }

    /// Half-open ordinate range `(q.v <= A.v) == (B.v < q.v)`: a segment is counted when `A.v >= q.v > B.v` or `B.v >= q.v
    /// > A.v`. A ray through the bottom vertices of the square is evaluated with the vertex counted on exactly one of its
    /// two edges.
    #[test]
    fn a_ray_through_a_vertex_counts_it_once_by_the_half_open_rule() {
        let sq = square();
        // q.v == 0, the ordinate of the bottom edge: every segment fails the half-open test (the
        // vertical edges' ordinate range is (0, 4], the horizontal ones have none): no crossing.
        assert!(!point_inside_polygon(Vec2::new(2.0, 0.0), &sq));
        // q.v == 4, the top: the left edge (0,4)-(0,0) is counted (A.v >= q.v > B.v) with x = 0 < 2,
        // the right edge (4,0)-(4,4) is counted (B.v >= q.v > A.v) with x = 4, not below 2: one crossing.
        assert!(point_inside_polygon(Vec2::new(2.0, 4.0), &sq));
        // a diamond with a vertex exactly on the ray: counted once, not twice and not zero times
        let diamond = vec![Vec2::new(0.0, -2.0), Vec2::new(2.0, 0.0), Vec2::new(0.0, 2.0), Vec2::new(-2.0, 0.0)];
        // ray toward -u from (1, 0) crosses the vertex (-2, 0): exactly one crossing -> inside
        assert!(point_inside_polygon(Vec2::new(1.0, 0.0), &diamond));
        // from (3, 0): ray crosses the vertex (2, 0) and (-2, 0): two crossings -> outside
        assert!(!point_inside_polygon(Vec2::new(3.0, 0.0), &diamond));
    }

    /// The abscissa test is STRICT and exact, no tolerance: a point exactly on a vertical edge is not crossed by it (x <
    /// q.u is false), a point one ulp to its right is.
    #[test]
    fn the_crossing_abscissa_is_strict_and_exact() {
        let sq = square();
        // on the right edge x = 4: the left edge x = 0 < 4 counted, the right edge x = 4 < 4 not:
        // one crossing -> odd -> "inside" by the parity alone (the boundary state is another test)
        assert!(point_inside_polygon(Vec2::new(4.0, 2.0), &sq));
        // on the left edge x = 0: neither edge has x < 0 -> zero crossings -> outside by parity
        assert!(!point_inside_polygon(Vec2::new(0.0, 2.0), &sq));
        // no tolerance: 1e-12 inside the left edge is still a crossing of it, 1e-12 outside is not
        assert!(point_inside_polygon(Vec2::new(1e-12, 2.0), &sq));
        assert!(!point_inside_polygon(Vec2::new(-1e-12, 2.0), &sq));
    }

    /// Box prefilter with tol, `d <= tol`, `t` in `[-tol, 1 + tol]`.
    #[test]
    fn on_polygon_uses_tol_in_the_box_the_distance_and_the_parameter_window() {
        let sq = square();
        let tol = 0.01;
        assert!(point_on_polygon(Vec2::new(2.0, 0.0), &sq, tol));
        assert!(point_on_polygon(Vec2::new(2.0, 0.01), &sq, tol)); // d == tol: non-strict
        assert!(!point_on_polygon(Vec2::new(2.0, 0.011), &sq, tol));
        // beyond the end of the bottom edge by less than tol in parameter units (t = 1.0025 of a
        // length-4 segment is 0.01 of length): on the line, accepted by the window
        assert!(point_on_polygon(Vec2::new(4.009, 0.0), &sq, tol));
        // (4.1, 0): the bottom edge's box fails (max u = 4 < 4.1 - tol), so does every other edge's
        assert!(!point_on_polygon(Vec2::new(4.1, 0.0), &sq, tol));
        // the box prefilter is closed: a point exactly tol outside the box passes it
        assert!(point_on_polygon(Vec2::new(-0.01, 2.0), &sq, tol));
    }

    /// 0 outside / 1 inside / 2 boundary; envelope grown by tol returns outside at once; the boundary state is the distance
    /// to the edges `<= tol`; the inside state is exact.
    #[test]
    fn the_classifier_returns_outside_inside_or_boundary() {
        let sq = square();
        let tol = 0.01;
        assert_eq!(classify_point(Vec2::new(2.0, 2.0), &sq, tol), PointState::Inside);
        assert_eq!(classify_point(Vec2::new(5.0, 2.0), &sq, tol), PointState::Outside);
        // on an edge, and within tol of it on either side
        assert_eq!(classify_point(Vec2::new(2.0, 0.0), &sq, tol), PointState::OnBoundary);
        assert_eq!(classify_point(Vec2::new(2.0, -0.01), &sq, tol), PointState::OnBoundary);
        assert_eq!(classify_point(Vec2::new(2.0, 0.01), &sq, tol), PointState::OnBoundary);
        assert_eq!(classify_point(Vec2::new(2.0, 0.0100001), &sq, tol), PointState::Inside);
        assert_eq!(classify_point(Vec2::new(2.0, -0.0100001), &sq, tol), PointState::Outside);
        // at a corner: distance to the nearest vertex <= tol
        assert_eq!(classify_point(Vec2::new(4.005, 4.005), &sq, tol), PointState::OnBoundary);
        // diagonal distance beyond tol: outside by the envelope
        assert_eq!(classify_point(Vec2::new(4.02, 4.02), &sq, tol), PointState::Outside);
        // an empty loop answers outside
        assert_eq!(classify_point(Vec2::new(0.0, 0.0), &[], tol), PointState::Outside);
    }

    /// The first non-boundary point of the inner loop decides; boundary points are skipped.
    #[test]
    fn containment_is_decided_by_the_first_point_that_is_not_on_the_boundary() {
        let outer = square();
        let tol = 1e-4;
        // an inner loop strictly inside
        let inner = vec![Vec2::new(1.0, 1.0), Vec2::new(2.0, 1.0), Vec2::new(2.0, 2.0)];
        assert_eq!(loop_encloses_loop(&outer, &inner, tol), Containment::Contains);
        // a loop outside
        let away = vec![Vec2::new(10.0, 10.0), Vec2::new(11.0, 10.0), Vec2::new(11.0, 11.0)];
        assert_eq!(loop_encloses_loop(&outer, &away, tol), Containment::DoesNotContain);
        // its first vertex on the boundary of the outer loop: skipped, the second one decides
        let touching = vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.5, 2.0)];
        assert_eq!(loop_encloses_loop(&outer, &touching, tol), Containment::Contains);
        let touching_out = vec![Vec2::new(0.0, 0.0), Vec2::new(-1.0, 1.0), Vec2::new(-2.0, 0.0)];
        assert_eq!(loop_encloses_loop(&outer, &touching_out, tol), Containment::DoesNotContain);
    }

    /// Every point on the boundary -> pass 2 classifies edge midpoints; a shared boundary is undecided.
    #[test]
    fn a_fully_shared_boundary_is_undecided_and_midpoints_are_pass_two() {
        let outer = square();
        let tol = 1e-4;
        // the same loop: every vertex and every midpoint is on the boundary
        assert_eq!(loop_encloses_loop(&outer, &outer, tol), Containment::Undecided);
        // an inner triangle whose three vertices are on the outer boundary but whose edge midpoint
        // (2,2) is inside: pass 1 skips all vertices, pass 2 decides on the first midpoint
        let tri = vec![Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0), Vec2::new(4.0, 4.0)];
        // midpoints in order: (2,0) on the boundary -> skipped, (4,2) skipped, (2,2) inside
        assert_eq!(loop_encloses_loop(&outer, &tri, tol), Containment::Contains);
        // reversed roles: the vertex (0,4) is inside the triangle's box but outside the triangle
        assert_eq!(loop_encloses_loop(&tri, &outer, tol), Containment::DoesNotContain);
    }
}
