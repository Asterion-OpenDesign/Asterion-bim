//! Sketch-loop validation and classification for slabs (floors, ceilings, roofs).
//!
//! Every rule here is an specification rule (written specification only):
//!
//! * Loops are tested as flat curves in their plane. Loops are walked in order; each loop is tested against ITSELF, then
//!   against every EARLIER loop; the first violating pair of edges stops the search. A pair is reported as "loops
//!   intersect" when the two edges are not parallel and as "lines overlap" when they are parallel; a contact at a shared
//!   endpoint counts as an intersection like a crossing; an open loop with open loops disallowed is reported only when no
//!   pair was found. With no violation, the orientation rule: a loop must be counter-clockwise exactly when the number of
//!   OTHER loops that contain it is even (outer boundaries and islands inside holes), clockwise otherwise (holes); a loop
//!   whose orientation differs is flipped.
//! * The walk order is the input order. Inside the OUTER iteration is the edges of the EARLIER loop `Lj` and the INNER the
//!   edges of the later loop `Li`, both in chain order; the pair is (edge of the later loop, edge of the earlier loop); the
//!   first pair found wins. In the outer iteration is `e_i` in chain order and the inner the edges after it; an adjacent
//!   pair that meets only at the shared end point is not a violation.
//! * Closed loops are handed to the extrusion builder; null / empty loops are dropped.
//! * The vertices of `C` are classified in loop order against `O` (0 outside, 1 inside, 2 on the boundary; the first point
//!   NOT on the boundary decides, boundary points are skipped); when every point is on the boundary the midpoints of the
//!   edges are tried; no decision is Err (not counted). The classifier: outside the envelope grown by the vertex epsilon is
//!   outside at once, a distance to an edge `<=` the epsilon is the boundary state, then the exact even-odd parity
//!   (half-open ordinate rule, strict abscissa, NO tolerance in the crossing test) gives inside or outside. See
//!   `asterion_geom::loop_point`.
//!
//! UNKNOWN (not specified, listed so nobody fills them with a guess):
//!
//! * `asterion_geom::loop_point` documents the choice taken.
//! * the sketch option `DisallowNestedLoops` is not modelled: the slab sketch options were not specified.

//! ARC EDGES (a stored loop is a list of lines and arcs). The rules above are applied to arcs as follows:
//!
//! * the pair test (step 4) asks for "any overlap range or any intersection point" between two edges; for an arc the engine
//!   answers it analytically (circle-line and circle-circle intersections, a coincident circle as an overlap range), with a
//!   contact at a shared end point of two adjacent edges excluded exactly as for lines. that numeric bound is not modelled
//!   (not specified: the numerics), so two arcs closer than it but not touching are accepted here.
//! * the code: when the END tangents of the two curves are parallel or anti-parallel, else - for an arc, its tangent at its
//!   end point.
//! * How samples an arc edge is not specified (classifies the loop's vertices, then edge midpoints).

use asterion_geom::loop_point::{loop_encloses_loop, Containment};
use asterion_geom::{Curve, Tolerances, Vec2};

pub const VERTEX_EPS_M: f64 = Tolerances::REFERENCE_COMPATIBLE.vertex;

/// (double, 3-argument): parallel (same or opposite direction) when both squared lengths are at least 1e-18 and `cos^2 >=
/// 0.9999969538258021` (0.1 degree).
pub const PARALLEL_COS2: f64 = 0.9999969538258021;

/// One loop of a sketch. `closed == false` is an open chain (its last point does not return).
#[derive(Clone, Debug, PartialEq)]
pub struct SketchLoop {
    pub points: Vec<Vec2>,
    pub closed: bool,
    /// Per edge, `Some(p)`: the edge is the arc from its start through `p` to its end. Empty =
    /// straight edges.
    pub arcs: Vec<Option<Vec2>>,
}

impl SketchLoop {
    pub fn closed(points: Vec<Vec2>) -> Self {
        SketchLoop { points, closed: true, arcs: Vec::new() }
    }

    /// An open chain of straight edges.
    pub fn open(points: Vec<Vec2>) -> Self {
        SketchLoop { points, closed: false, arcs: Vec::new() }
    }

    /// A closed loop with arc edges (`arcs` index-aligned with the edges).
    pub fn closed_with_arcs(points: Vec<Vec2>, arcs: Vec<Option<Vec2>>) -> Self {
        SketchLoop { points, closed: true, arcs }
    }

    fn has_arcs(&self) -> bool {
        self.arcs.iter().any(|a| a.is_some())
    }

    /// Edges in order; an open chain has `n - 1`, a closed loop `n`.
    fn edges(&self) -> Vec<(Vec2, Vec2)> {
        let n = self.points.len();
        if n < 2 {
            return Vec::new();
        }
        let m = if self.closed { n } else { n - 1 };
        (0..m).map(|i| (self.points[i], self.points[(i + 1) % n])).collect()
    }

    /// Edges as curves; an arc whose three points have no circle is taken as its chord (the
    /// creating operations refuse such an arc before validation).
    fn curves(&self) -> Vec<Curve> {
        self.edges()
            .into_iter()
            .enumerate()
            .map(|(i, (a, b))| match self.arcs.get(i).copied().flatten() {
                Some(t) => asterion_geom::Arc::from_three_points(a, t, b, crate::model::ARC_COLLINEAR_TOL_M)
                    .map(Curve::Arc)
                    .unwrap_or(Curve::line(a, b)),
                None => Curve::line(a, b),
            })
            .collect()
    }

    /// The polygon the containment and orientation tests run on: the vertices, every arc replaced by
    /// its stored samples.
    fn sample_polygon(&self) -> Vec<Vec2> {
        if !self.has_arcs() {
            return self.points.clone();
        }
        crate::model::SlabLoop { points: self.points.clone(), arcs: self.arcs.clone() }.outline()
    }
}

/// Tolerance of the analytic touch test when an arc takes part (metres): numerical noise of the
/// circle intersections, the analogue of the 1e-12 of the exact segment test.
const ARC_TOUCH_TOL_M: f64 = 1e-9;

fn arc_tols() -> Tolerances {
    Tolerances { vertex: ARC_TOUCH_TOL_M, ..Tolerances::REFERENCE_COMPATIBLE }
}

/// Two curves cross, touch or overlap (any overlap range or any intersection point, a contact at an end point included).
/// Two lines use the exact segment test.
fn curves_touch(a: &Curve, b: &Curve) -> bool {
    match (a, b) {
        (Curve::Line(l0), Curve::Line(l1)) => segments_touch((l0.start, l0.end), (l1.start, l1.end)),
        _ => !asterion_geom::intersect_curves(a, b, &arc_tols()).is_empty(),
    }
}

/// Two ADJACENT curves of a loop (sharing the end points in `shared`) touch somewhere else, or overlap beyond a shared
/// point (an adjacent pair that meets only at the shared end point is not a violation).
fn adjacent_curves_violate(a: &Curve, b: &Curve, shared: &[Vec2]) -> bool {
    let tol = arc_tols();
    if let (Curve::Arc(c0), Curve::Arc(c1)) = (a, b) {
        if c0.centre.distance_to(c1.centre) <= tol.vertex && (c0.radius - c1.radius).abs() <= tol.vertex {
            // the same circle: an overlap range longer than a point is a fold-back (the middle of
            // one arc lies on the other), as is a shared span ending away from the shared points
            if asterion_geom::arc_contains_point(c1, c0.point_at(0.5), &tol)
                || asterion_geom::arc_contains_point(c0, c1.point_at(0.5), &tol)
            {
                return true;
            }
            let pts = asterion_geom::intersect::coincident_arc_overlap_points(c0, c1, &tol);
            return pts.iter().any(|p| !shared.iter().any(|s| s.distance_to(*p) <= tol.vertex));
        }
    }
    asterion_geom::intersect_curves(a, b, &tol)
        .iter()
        .any(|p| !shared.iter().any(|s| s.distance_to(*p) <= tol.vertex))
}

/// On the END tangents of two curves.
fn end_tangents_parallel(a: &Curve, b: &Curve) -> bool {
    match (a.tangent_at(1.0), b.tangent_at(1.0)) {
        (Some(ta), Some(tb)) => segments_parallel((Vec2::new(0.0, 0.0), ta), (Vec2::new(0.0, 0.0), tb)),
        _ => false,
    }
}

/// For a loop with arc edges: the same walk as [`self_pair`], the pair test on curves.
fn self_pair_curves(c: &[Curve], closed: bool) -> Option<(usize, usize)> {
    let n = c.len();
    for i in 0..n {
        for j in (i + 1)..n {
            let adjacent_next = j == i + 1;
            let adjacent_wrap = closed && i == 0 && j == n - 1 && n >= 2;
            if adjacent_next || adjacent_wrap {
                let mut shared = Vec::with_capacity(2);
                if adjacent_next {
                    shared.push(c[i].end());
                }
                if adjacent_wrap {
                    shared.push(c[i].start());
                }
                if adjacent_curves_violate(&c[i], &c[j], &shared) {
                    return Some((i, j));
                }
            } else if curves_touch(&c[i], &c[j]) {
                return Some((i, j));
            }
        }
    }
    None
}

/// On curves: outer iteration the EARLIER loop, inner the later one.
fn loop_pair_curves(a: &[Curve], b: &[Curve]) -> Option<(usize, usize)> {
    for (ib, eb) in b.iter().enumerate() {
        for (ia, ea) in a.iter().enumerate() {
            if curves_touch(ea, eb) {
                return Some((ia, ib));
            }
        }
    }
    None
}

/// One edge of a sketch: the loop (index in the input array) and the edge index in chain order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeRef {
    pub loop_index: usize,
    pub edge: usize,
}

/// What the validation found, mapped to the failure the sketch posts.
///
/// `pair` holds the two edges in the order the reference stores their ids: (edge of the later loop, edge of the earlier
/// loop) for two loops, (earlier edge, later edge) for a self hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchFault {
    /// The sketch has no loops (an empty set, or only empty loops).
    NoLoops,
    /// The end tangents of the pair are not parallel (loops cross, or a vertex touches).
    LoopsIntersect { a: usize, b: usize, pair: [EdgeRef; 2] },
    /// The end tangents of the pair are parallel or anti-parallel (shared or collinear edges).
    LinesOverlap { a: usize, b: usize, pair: [EdgeRef; 2] },
    /// An open loop and no violating pair.
    OpenLoop { loop_index: usize },
}

impl SketchFault {
    /// The reported pair of edges, when the fault is a pair.
    pub fn pair(&self) -> Option<[EdgeRef; 2]> {
        match self {
            SketchFault::LoopsIntersect { pair, .. } | SketchFault::LinesOverlap { pair, .. } => Some(*pair),
            _ => None,
        }
    }
}

fn orient(p: Vec2, q: Vec2, r: Vec2) -> f64 {
    (q - p).cross(r - p)
}

fn on_seg(p: Vec2, q: Vec2, r: Vec2) -> bool {
    r.x >= p.x.min(q.x) - 1e-12 && r.x <= p.x.max(q.x) + 1e-12 && r.y >= p.y.min(q.y) - 1e-12 && r.y <= p.y.max(q.y) + 1e-12
}

/// Two segments cross, touch (an endpoint on the other) or overlap (any overlap range or any intersection point, a contact
/// at an endpoint included).
pub fn segments_touch(a: (Vec2, Vec2), b: (Vec2, Vec2)) -> bool {
    let (p1, p2) = a;
    let (q1, q2) = b;
    let (o1, o2, o3, o4) = (orient(p1, p2, q1), orient(p1, p2, q2), orient(q1, q2, p1), orient(q1, q2, p2));
    if o1 * o2 < 0.0 && o3 * o4 < 0.0 {
        return true;
    }
    (o1.abs() < 1e-12 && on_seg(p1, p2, q1))
        || (o2.abs() < 1e-12 && on_seg(p1, p2, q2))
        || (o3.abs() < 1e-12 && on_seg(q1, q2, p1))
        || (o4.abs() < 1e-12 && on_seg(q1, q2, p2))
}

/// Not parallel when either squared length is below 1e-18; else parallel exactly when `(a.b)^2 / (|a|^2 |b|^2) >=
/// 0.9999969538258021` (either direction).
pub fn segments_parallel(a: (Vec2, Vec2), b: (Vec2, Vec2)) -> bool {
    let (da, db) = (a.1 - a.0, b.1 - b.0);
    let (la2, lb2) = (da.dot(da), db.dot(db));
    if la2 < 1e-18 || lb2 < 1e-18 {
        return false;
    }
    let d = da.dot(db);
    d * d / (la2 * lb2) >= PARALLEL_COS2
}

/// Two edges that are ADJACENT in a loop (they share the end point `s`) overlap beyond that point: collinear and both
/// running away from `s` on the same side. An adjacent pair that meets only at the shared end point is not a violation, any
/// other contact is.
fn adjacent_edges_overlap(earlier: (Vec2, Vec2), later: (Vec2, Vec2)) -> bool {
    // earlier ends where later starts (the shared end point).
    let s = earlier.1;
    let (d1, d2) = (earlier.0 - s, later.1 - s);
    let (l1, l2) = (d1.dot(d1), d2.dot(d2));
    if l1 < 1e-18 || l2 < 1e-18 {
        return false;
    }
    d1.cross(d2).abs() < 1e-12 && d1.dot(d2) > 0.0
}

/// The first violating pair ends the search, `e_i` being the earlier edge.
///
/// It is not modelled here.
fn self_pair(edges: &[(Vec2, Vec2)], closed: bool) -> Option<(usize, usize)> {
    let n = edges.len();
    for i in 0..n {
        for j in (i + 1)..n {
            let adjacent_next = j == i + 1;
            let adjacent_wrap = closed && i == 0 && j == n - 1 && n >= 3;
            if adjacent_next {
                if adjacent_edges_overlap(edges[i], edges[j]) {
                    return Some((i, j));
                }
            } else if adjacent_wrap {
                // edge n-1 ends where edge 0 starts
                if adjacent_edges_overlap(edges[j], edges[i]) {
                    return Some((i, j));
                }
            } else if segments_touch(edges[i], edges[j]) {
                return Some((i, j));
            }
        }
    }
    None
}

/// A closed polygon crosses or touches itself.
pub fn loop_self_intersects(p: &[Vec2]) -> bool {
    let l = SketchLoop::closed(p.to_vec());
    self_pair(&l.edges(), true).is_some()
}

/// The OUTER iteration walks the edges of `B` in chain order, the INNER iteration the edges of `A` in chain order; the
/// first pair `(a, b)` for which the edges touch (any overlap range or any intersection point, a contact at an end point
/// included) is reported. Returns `(edge index in A, edge index in B)`.
fn loop_pair(a_edges: &[(Vec2, Vec2)], b_edges: &[(Vec2, Vec2)]) -> Option<(usize, usize)> {
    for (ib, eb) in b_edges.iter().enumerate() {
        for (ia, ea) in a_edges.iter().enumerate() {
            if segments_touch(*ea, *eb) {
                return Some((ia, ib));
            }
        }
    }
    None
}

/// Loop order is the input order and edge order inside a loop is the curve order. The walk takes the loops `L0, L1, ...`.
/// [`SketchFault::LinesOverlap`], else [`SketchFault::LoopsIntersect`]; the choice is symmetric in the pair, what decides
/// it is which pair is found first. An open loop is reported only when no pair was found.
///
/// UNKNOWN: the envelope pre-test (loops with a computed envelope whose envelopes do not intersect are skipped by the
/// reference); skipping only removes pairs that cannot touch, so it is not modelled. Curves other than straight segments
/// (arcs, splines) are not sketch loops here, so not specified does not arise.
pub fn first_fault(loops: &[SketchLoop]) -> Option<SketchFault> {
    let loops: Vec<(usize, &SketchLoop)> = loops.iter().enumerate().filter(|(_, l)| !l.points.is_empty()).collect();
    if loops.is_empty() {
        return Some(SketchFault::NoLoops);
    }
    if loops.iter().any(|(_, l)| l.has_arcs()) {
        return first_fault_curves(&loops);
    }
    let edges: Vec<Vec<(Vec2, Vec2)>> = loops.iter().map(|(_, l)| l.edges()).collect();
    let fault = |pair: [EdgeRef; 2], edges_pair: ((Vec2, Vec2), (Vec2, Vec2))| -> SketchFault {
        let (a, b) = (pair[0].loop_index.min(pair[1].loop_index), pair[0].loop_index.max(pair[1].loop_index));
        if segments_parallel(edges_pair.0, edges_pair.1) {
            SketchFault::LinesOverlap { a, b, pair }
        } else {
            SketchFault::LoopsIntersect { a, b, pair }
        }
    };
    for (k, (idx, l)) in loops.iter().enumerate() {
        if let Some((i, j)) = self_pair(&edges[k], l.closed) {
            return Some(fault(
                [EdgeRef { loop_index: *idx, edge: i }, EdgeRef { loop_index: *idx, edge: j }],
                (edges[k][i], edges[k][j]),
            ));
        }
        for (kk, (idx0, _)) in loops.iter().enumerate().take(k) {
            if let Some((ia, ib)) = loop_pair(&edges[k], &edges[kk]) {
                // ids stored as (edge of the LATER loop, edge of the EARLIER loop)
                return Some(fault(
                    [EdgeRef { loop_index: *idx, edge: ia }, EdgeRef { loop_index: *idx0, edge: ib }],
                    (edges[k][ia], edges[kk][ib]),
                ));
            }
        }
    }
    loops
        .iter()
        .find(|(_, l)| !l.closed)
        .map(|(idx, _)| SketchFault::OpenLoop { loop_index: *idx })
}

/// [`first_fault`] for a sketch with arc edges: the same walk, the same codes, the pair test on curves (module docs, "ARC
/// EDGES").
fn first_fault_curves(loops: &[(usize, &SketchLoop)]) -> Option<SketchFault> {
    let curves: Vec<Vec<Curve>> = loops.iter().map(|(_, l)| l.curves()).collect();
    let fault = |pair: [EdgeRef; 2], c: (&Curve, &Curve)| -> SketchFault {
        let (a, b) = (pair[0].loop_index.min(pair[1].loop_index), pair[0].loop_index.max(pair[1].loop_index));
        if end_tangents_parallel(c.0, c.1) {
            SketchFault::LinesOverlap { a, b, pair }
        } else {
            SketchFault::LoopsIntersect { a, b, pair }
        }
    };
    for (k, (idx, l)) in loops.iter().enumerate() {
        if let Some((i, j)) = self_pair_curves(&curves[k], l.closed) {
            return Some(fault(
                [EdgeRef { loop_index: *idx, edge: i }, EdgeRef { loop_index: *idx, edge: j }],
                (&curves[k][i], &curves[k][j]),
            ));
        }
        for (kk, (idx0, _)) in loops.iter().enumerate().take(k) {
            if let Some((ia, ib)) = loop_pair_curves(&curves[k], &curves[kk]) {
                return Some(fault(
                    [EdgeRef { loop_index: *idx, edge: ia }, EdgeRef { loop_index: *idx0, edge: ib }],
                    (&curves[k][ia], &curves[kk][ib]),
                ));
            }
        }
    }
    loops
        .iter()
        .find(|(_, l)| !l.closed)
        .map(|(idx, _)| SketchFault::OpenLoop { loop_index: *idx })
}

/// Per-loop outcome of the orientation step (step 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoopRole {
    /// Number of OTHER loops that contain this loop.
    pub depth: usize,
    /// Even depth: outer boundary or island (material); odd: hole.
    pub material: bool,
    pub should_be_ccw: bool,
    pub is_ccw: bool,
    /// The reference product flips the loop's orientation to `should_be_ccw`.
    pub flipped: bool,
}

/// Classify every closed loop by the parity of its nesting depth. Open loops answer `should_be_ccw = true` (the reference
/// product's rule, with a debug warning) and are never flipped.
///
/// Only meaningful when [`first_fault`] found nothing: loops that touch are refused before
/// containment is asked.
pub fn classify(loops: &[SketchLoop]) -> Vec<LoopRole> {
    // the stored-sample polygon of each loop (its vertices when every edge is straight)
    let polys: Vec<Vec<Vec2>> = loops.iter().map(|l| l.sample_polygon()).collect();
    loops
        .iter()
        .enumerate()
        .map(|(i, l)| {
            if !l.closed || polys[i].len() < 3 {
                return LoopRole { depth: 0, material: true, should_be_ccw: true, is_ccw: true, flipped: false };
            }
            // An open loop classifies every point outside, never contains.
            let tol = VERTEX_EPS_M;
            let depth = loops
                .iter()
                .enumerate()
                .filter(|(j, o)| {
                    *j != i
                        && o.closed
                        && polys[*j].len() >= 3
                        && loop_encloses_loop(&polys[*j], &polys[i], tol) == Containment::Contains
                })
                .count();
            let should_be_ccw = depth % 2 == 0;
            let is_ccw = asterion_geom::polygon_signed_area(&polys[i]) > 0.0;
            LoopRole { depth, material: should_be_ccw, should_be_ccw, is_ccw, flipped: should_be_ccw != is_ccw }
        })
        .collect()
}

/// Number of loops with `should_be_ccw` (the count output of the reference product's check).
pub fn should_be_ccw_count(roles: &[LoopRole]) -> usize {
    roles.iter().filter(|r| r.should_be_ccw).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sq(x: f64, y: f64, s: f64) -> SketchLoop {
        SketchLoop::closed(vec![Vec2::new(x, y), Vec2::new(x + s, y), Vec2::new(x + s, y + s), Vec2::new(x, y + s)])
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> SketchLoop {
        SketchLoop::closed(vec![Vec2::new(x, y), Vec2::new(x + w, y), Vec2::new(x + w, y + h), Vec2::new(x, y + h)])
    }

    #[test]
    fn an_empty_sketch_has_no_loops_0x11() {
        assert_eq!(first_fault(&[]), Some(SketchFault::NoLoops));
        let empty = SketchLoop::closed(vec![]);
        assert_eq!(first_fault(&[empty]), Some(SketchFault::NoLoops));
    }

    #[test]
    fn disjoint_nested_loops_are_valid() {
        let l = [rect(0.0, 0.0, 6.0, 4.0), sq(2.0, 1.0, 1.0)];
        assert_eq!(first_fault(&l), None);
    }

    /// The two classes of fault found between a pair of edges.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum FaultClass {
        /// The loops cross, or touch at a vertex.
        Crossing,
        /// The loops share a segment, or two collinear edges meet.
        Overlap,
    }

    /// `(class, earlier loop, later loop)` of a pair fault.
    fn coded(f: Option<SketchFault>) -> Option<(FaultClass, usize, usize)> {
        match f {
            Some(SketchFault::LoopsIntersect { a, b, .. }) => Some((FaultClass::Crossing, a, b)),
            Some(SketchFault::LinesOverlap { a, b, .. }) => Some((FaultClass::Overlap, a, b)),
            _ => None,
        }
    }

    fn er(loop_index: usize, edge: usize) -> EdgeRef {
        EdgeRef { loop_index, edge }
    }

    #[test]
    fn crossing_loops_are_0xe() {
        let l = [rect(0.0, 0.0, 4.0, 4.0), rect(3.0, 1.0, 3.0, 1.0)];
        assert_eq!(coded(first_fault(&l)), Some((FaultClass::Crossing, 0, 1)));
    }

    /// The pair is the first edge `b` of the EARLIER loop (chain order) that touches any edge of the later loop, with the
    /// first edge `a` of the later loop that touches `b`. A hole drawn starting along the shared segment pairs the shared
    /// edge with itself; the same hole drawn starting at the vertex that touches the outer edge pairs two perpendicular
    /// edges.
    #[test]
    fn a_hole_sharing_an_edge_is_0xf_or_0xe_by_the_pair_found_first() {
        let along = SketchLoop::closed(vec![Vec2::new(0.0, 1.0), Vec2::new(0.0, 2.0), Vec2::new(1.0, 2.0), Vec2::new(1.0, 1.0)]);
        let f = first_fault(&[rect(0.0, 0.0, 6.0, 4.0), along]);
        assert_eq!(coded(f), Some((FaultClass::Overlap, 0, 1)));
        // (edge of the later loop, edge of the earlier loop): the hole's first edge with the left edge
        assert_eq!(f.unwrap().pair(), Some([er(1, 0), er(0, 3)]));
        let f = first_fault(&[rect(0.0, 0.0, 6.0, 4.0), sq(0.0, 1.0, 1.0)]);
        assert_eq!(coded(f), Some((FaultClass::Crossing, 0, 1)));
        assert_eq!(f.unwrap().pair(), Some([er(1, 0), er(0, 3)]));
    }

    /// The OUTER iteration is the earlier loop `B`, the INNER the later loop `A`.
    #[test]
    fn the_outer_iteration_is_the_earlier_loop() {
        let outer = rect(0.0, 0.0, 6.0, 4.0);
        let hole = SketchLoop::closed(vec![
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(-1.0, 0.0),
            Vec2::new(3.0, 0.0),
            Vec2::new(3.0, 1.0),
        ]);
        let f = first_fault(&[outer, hole]);
        assert_eq!(f.unwrap().pair(), Some([er(1, 2), er(0, 0)]));
        assert_eq!(coded(f), Some((FaultClass::Overlap, 0, 1)));
    }

    /// A contact at a single vertex is an intersection point, reported like a crossing.
    #[test]
    fn a_vertex_contact_between_non_parallel_edges_is_an_intersection_0xe() {
        // the hole's corner (3, 0) lies on the bottom edge of the outer loop, hole edges diagonal
        let hole = SketchLoop::closed(vec![Vec2::new(3.0, 0.0), Vec2::new(4.0, 1.0), Vec2::new(2.0, 1.0)]);
        let l = [rect(0.0, 0.0, 6.0, 4.0), hole];
        assert_eq!(coded(first_fault(&l)), Some((FaultClass::Crossing, 0, 1)));
    }

    #[test]
    fn collinear_edges_meeting_at_a_vertex_are_0xf() {
        let a = SketchLoop::closed(vec![Vec2::new(0.0, 0.0), Vec2::new(2.0, 0.0), Vec2::new(2.0, 2.0), Vec2::new(0.0, 2.0)]);
        let b = SketchLoop::closed(vec![Vec2::new(2.0, 0.0), Vec2::new(4.0, 0.0), Vec2::new(4.0, -2.0), Vec2::new(2.0, -2.0)]);
        let f = first_fault(&[a, b]);
        // the first earlier edge touching b is a.e0 (bottom), met by b.e0 (collinear, at (2,0))
        assert_eq!(f.unwrap().pair(), Some([er(1, 0), er(0, 0)]));
        assert_eq!(coded(f), Some((FaultClass::Overlap, 0, 1)));
    }

    /// Two squares touching only at a corner: the edges meeting there are perpendicular.
    #[test]
    fn two_squares_touching_at_a_corner_intersect() {
        let l = [sq(0.0, 0.0, 2.0), sq(2.0, 2.0, 2.0)];
        assert_eq!(coded(first_fault(&l)), Some((FaultClass::Crossing, 0, 1)));
    }

    #[test]
    fn a_self_crossing_loop_is_a_pair_in_the_same_loop() {
        let bow = SketchLoop::closed(vec![Vec2::new(0.0, 0.0), Vec2::new(2.0, 2.0), Vec2::new(2.0, 0.0), Vec2::new(0.0, 2.0)]);
        let f = first_fault(&[bow]);
        assert_eq!(coded(f), Some((FaultClass::Crossing, 0, 0)));
        // outer iteration e_i, inner the edges after it: e0 = (0,0)-(2,2) crosses e2 = (2,0)-(0,2)
        assert_eq!(f.unwrap().pair(), Some([er(0, 0), er(0, 2)]));
    }

    /// An adjacent pair that meets only at the shared end point is not a violation (a polygon's corners), but a spike that
    /// folds back over itself is.
    #[test]
    fn adjacent_edges_violate_only_when_they_overlap_beyond_the_shared_point() {
        assert_eq!(first_fault(&[rect(0.0, 0.0, 4.0, 4.0)]), None);
        let spike = SketchLoop::closed(vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(4.0, 0.0),
            Vec2::new(6.0, 0.0),
            Vec2::new(5.0, 0.0),
            Vec2::new(4.0, 3.0),
            Vec2::new(0.0, 3.0),
        ]);
        // e1 = (4,0)-(6,0) and e2 = (6,0)-(5,0) fold back: overlap beyond the shared point (6,0)
        let f = first_fault(&[spike]);
        assert_eq!(f.unwrap().pair(), Some([er(0, 1), er(0, 2)]));
        assert_eq!(coded(f), Some((FaultClass::Overlap, 0, 0)));
    }

    /// The order: each loop is tested against itself and then against EARLIER loops, so a bow-tie
    /// listed after a crossing pair is reached only after the pair; a bow-tie listed FIRST is
    /// reported before the pair.
    #[test]
    fn the_first_violation_in_loop_order_wins() {
        let bow = SketchLoop::closed(vec![Vec2::new(10.0, 0.0), Vec2::new(12.0, 2.0), Vec2::new(12.0, 0.0), Vec2::new(10.0, 2.0)]);
        let l = [rect(0.0, 0.0, 4.0, 4.0), rect(3.0, 1.0, 3.0, 1.0), bow.clone()];
        assert_eq!(coded(first_fault(&l)), Some((FaultClass::Crossing, 0, 1)));
        let l2 = [bow, rect(0.0, 0.0, 4.0, 4.0), rect(3.0, 1.0, 3.0, 1.0)];
        assert_eq!(coded(first_fault(&l2)), Some((FaultClass::Crossing, 0, 0)));
    }

    /// For loop `Li` the self test comes BEFORE the test against the earlier loops, and the earlier loops are tried in
    /// ascending order.
    #[test]
    fn self_then_earlier_loops_ascending() {
        let outer = rect(0.0, 0.0, 20.0, 20.0);
        let a = sq(1.0, 1.0, 2.0); // inside outer, disjoint
        // L2 crosses L1 (index 1) and L0 is not touched: pair is with loop 1
        let l2 = rect(2.0, 2.0, 4.0, 1.0);
        let f = first_fault(&[outer.clone(), a.clone(), l2.clone()]);
        assert_eq!(coded(f), Some((FaultClass::Crossing, 1, 2)));
        // L2 touches both: loop 0 is tried first
        let l2b = SketchLoop::closed(vec![
            Vec2::new(2.0, 2.0),
            Vec2::new(-1.0, 2.0),
            Vec2::new(-1.0, 3.0),
            Vec2::new(2.0, 3.0),
        ]);
        let f = first_fault(&[outer, a, l2b]);
        assert_eq!(coded(f).map(|(_, a, b)| (a, b)), Some((0, 2)));
    }

    /// Step 5: an open loop is reported only when no pair violates.
    #[test]
    fn an_open_loop_is_0x14_only_without_a_violating_pair() {
        let open = SketchLoop::open(vec![Vec2::new(1.0, 1.0), Vec2::new(2.0, 1.0), Vec2::new(2.0, 2.0)]);
        assert_eq!(first_fault(&[rect(0.0, 0.0, 6.0, 4.0), open.clone()]), Some(SketchFault::OpenLoop { loop_index: 1 }));
        let crossing = SketchLoop::open(vec![Vec2::new(-1.0, 1.0), Vec2::new(7.0, 1.0)]);
        assert_eq!(coded(first_fault(&[rect(0.0, 0.0, 6.0, 4.0), crossing])), Some((FaultClass::Crossing, 0, 1)));
    }

    /// Step 6: even depth CCW, odd depth CW; islands inside holes are material again.
    #[test]
    fn nesting_parity_decides_material_and_required_orientation() {
        let outer = rect(0.0, 0.0, 10.0, 10.0); // depth 0, CCW
        let hole = rect(2.0, 2.0, 6.0, 6.0); // depth 1
        let island = rect(4.0, 4.0, 2.0, 2.0); // depth 2
        let outside = rect(20.0, 0.0, 2.0, 2.0); // depth 0 (a second island)
        let l = [outer, hole, island, outside];
        assert_eq!(first_fault(&l), None);
        let roles = classify(&l);
        assert_eq!(roles.iter().map(|r| r.depth).collect::<Vec<_>>(), vec![0, 1, 2, 0]);
        assert_eq!(roles.iter().map(|r| r.material).collect::<Vec<_>>(), vec![true, false, true, true]);
        // all four were drawn counter-clockwise: only the hole must be flipped
        assert_eq!(roles.iter().map(|r| r.flipped).collect::<Vec<_>>(), vec![false, true, false, false]);
        assert_eq!(should_be_ccw_count(&roles), 3);
    }

    fn tri(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> SketchLoop {
        SketchLoop::closed(vec![Vec2::new(a.0, a.1), Vec2::new(b.0, b.1), Vec2::new(c.0, c.1)])
    }

    fn depths(l: &[SketchLoop]) -> Vec<usize> {
        classify(l).iter().map(|r| r.depth).collect()
    }

    /// A vertex within the vertex epsilon of the other loop's edge is ON THE BOUNDARY and is skipped; the first vertex that
    /// is not decides. A strict first-vertex test would call this triangle outside (its first vertex is 0.05 mm below the
    /// outer bottom edge) and give depth 0.
    #[test]
    fn a_first_vertex_within_the_epsilon_of_the_edge_is_skipped_not_decisive() {
        assert!(VERTEX_EPS_M > 0.0001 && VERTEX_EPS_M < 0.0002); // 0.0005233832795 ft = 1.595e-4 m
        let outer = rect(0.0, 0.0, 6.0, 4.0);
        let inner = tri((3.0, -0.00005), (4.0, 1.0), (2.0, 1.0));
        assert_eq!(depths(&[outer.clone(), inner]), vec![0, 1]);
        // beyond the epsilon the first vertex is plainly outside and decides: not contained
        let outside = tri((3.0, -0.001), (4.0, 1.0), (2.0, 1.0));
        assert_eq!(depths(&[outer, outside]), vec![0, 0]);
    }

    /// Every vertex on the other loop's boundary -> the midpoints decide; a loop that coincides with the other is undecided
    /// and counts for neither.
    #[test]
    fn a_shared_boundary_is_undecided_and_midpoints_are_the_second_pass() {
        let a = rect(0.0, 0.0, 4.0, 4.0);
        assert_eq!(depths(&[a.clone(), a.clone()]), vec![0, 0]);
        // triangle with its three vertices on the square's boundary: the midpoint (2,2) is inside
        let t = tri((0.0, 0.0), (4.0, 0.0), (4.0, 4.0));
        assert_eq!(depths(&[a, t]), vec![0, 1]);
    }

    /// The half-open ordinate rule and the strict abscissa. A hole whose vertex has exactly the ordinate of a vertex of the
    /// outer loop is counted once, and a vertex exactly on the other loop's vertical edge is the boundary state, so the
    /// next vertex decides.
    #[test]
    fn ties_in_the_ordinate_and_on_an_edge_follow_the_parity_rule() {
        let diamond = SketchLoop::closed(vec![
            Vec2::new(0.0, -4.0),
            Vec2::new(4.0, 0.0),
            Vec2::new(0.0, 4.0),
            Vec2::new(-4.0, 0.0),
        ]);
        // vertices at ordinate 0, the ordinate of the diamond's side vertices
        let inner = tri((1.0, 0.0), (2.0, 1.0), (0.0, 1.5));
        assert_eq!(depths(&[diamond.clone(), inner]), vec![0, 1]);
        // a point on the ordinate of a vertex but beyond it is outside (two crossings)
        let beyond = tri((5.0, 0.0), (6.0, 1.0), (5.0, 1.0));
        assert_eq!(depths(&[diamond, beyond]), vec![0, 0]);
        // first vertex exactly on the left edge of the outer square: boundary, skipped; second inside
        let outer = rect(0.0, 0.0, 6.0, 4.0);
        let touching = tri((0.0, 2.0), (1.0, 1.0), (1.0, 3.0));
        assert_eq!(depths(&[outer.clone(), touching]), vec![0, 1]);
        // ... and decided outside when the second vertex is outside
        let leaving = tri((0.0, 2.0), (-1.0, 1.0), (-1.0, 3.0));
        assert_eq!(depths(&[outer, leaving]), vec![0, 0]);
    }

    #[test]
    fn a_clockwise_outer_loop_is_flipped() {
        let mut cw = rect(0.0, 0.0, 4.0, 4.0);
        cw.points.reverse();
        let roles = classify(&[cw]);
        assert!(roles[0].flipped && roles[0].should_be_ccw && !roles[0].is_ccw);
    }

    fn arc_loop(points: &[(f64, f64)], arcs: &[Option<(f64, f64)>]) -> SketchLoop {
        SketchLoop::closed_with_arcs(
            points.iter().map(|p| Vec2::new(p.0, p.1)).collect(),
            arcs.iter().map(|a| a.map(|p| Vec2::new(p.0, p.1))).collect(),
        )
    }

    /// ARC EDGES: a disc of two half circles, a D shape (an arc and its chord) and a rectangle with an arched side are
    /// valid; adjacent curves meeting only at their shared end points are not a violation.
    #[test]
    fn valid_loops_with_arcs() {
        let disc = arc_loop(&[(-1.0, 0.0), (1.0, 0.0)], &[Some((0.0, -1.0)), Some((0.0, 1.0))]);
        assert_eq!(first_fault(&[disc]), None);
        let d = arc_loop(&[(-1.0, 0.0), (1.0, 0.0)], &[None, Some((0.0, 1.0))]);
        assert_eq!(first_fault(&[d]), None);
        let arched = arc_loop(&[(0.0, 0.0), (6.0, 0.0), (6.0, 4.0), (0.0, 4.0)], &[None, None, Some((3.0, 5.0)), None]);
        assert_eq!(first_fault(&[arched.clone()]), None);
        // a tangent join: the arc of the D shape continues the line smoothly at (1, 0)
        let tangent = arc_loop(&[(0.0, 0.0), (1.0, 0.0), (1.0, 2.0), (0.0, 2.0)], &[None, Some((2.0, 1.0)), None, None]);
        assert_eq!(first_fault(&[tangent]), None);
    }

    /// The code is decided by the END tangents.
    #[test]
    fn arc_faults_follow_the_pair_rule() {
        // the second half circle retraces the first: same circle, overlap range
        let fold = arc_loop(&[(-1.0, 0.0), (1.0, 0.0)], &[Some((0.0, 1.0)), Some((0.0, 1.0))]);
        assert_eq!(coded(first_fault(&[fold])), Some((FaultClass::Overlap, 0, 0)));
        // a square crossing the arc of the arched rectangle
        let arched = arc_loop(&[(0.0, 0.0), (6.0, 0.0), (6.0, 4.0), (0.0, 4.0)], &[None, None, Some((3.0, 5.0)), None]);
        let f = first_fault(&[arched, sq(2.5, 4.5, 1.0)]);
        assert_eq!(coded(f), Some((FaultClass::Crossing, 0, 1)));
        assert_eq!(f.unwrap().pair().unwrap()[1], er(0, 2), "the earlier edge is the arc");
    }

    /// ARC EDGES: containment and orientation run on the stored-sample polygons: a round hole inside
    /// a rectangle is a hole (depth 1), a rectangle inside a disc is a hole, and the disc drawn
    /// clockwise is flipped.
    #[test]
    fn arcs_take_part_in_the_nesting_parity() {
        let disc_cw = arc_loop(&[(-3.0, 0.0), (3.0, 0.0)], &[Some((0.0, 3.0)), Some((0.0, -3.0))]);
        let roles = classify(&[disc_cw.clone(), sq(-1.0, -1.0, 2.0)]);
        assert_eq!((roles[0].depth, roles[1].depth), (0, 1));
        assert!(roles[0].flipped, "a clockwise outer disc is flipped");
        let hole = arc_loop(&[(2.0, 2.0), (4.0, 2.0)], &[Some((3.0, 1.0)), Some((3.0, 3.0))]);
        let roles = classify(&[rect(0.0, 0.0, 6.0, 4.0), hole]);
        assert_eq!((roles[1].depth, roles[1].material), (1, false));
    }
}
