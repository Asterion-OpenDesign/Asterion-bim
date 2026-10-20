//! Closed curve loops (`Polycurve`) and planar regions with holes (`Region`).
//!
//! # Why exact and not tessellated
//!
//! Introducing arcs must not degrade that, so:
//!
//! * `perimeter` sums `|chord|` for segments and `r * |sweep|` for arcs;
//! * `signed_area` sums Green's theorem terms, i.e. the chord shoelace plus the
//!   circular segment `r^2 (sweep - sin sweep) / 2`.
//!
//! Both are closed-form; no sampling, no chord tolerance, so the result is a
//! deterministic function of the input numbers only.
//!
//! # Orientation convention
//!
//! A `Region` is canonicalised: the outer loop runs counter-clockwise (positive signed area) and every inner loop clockwise
//! (negative). `Region::area` is then `outer - sum(holes)` and is always non-negative for a valid region. **ASSUMED** for
//! arcs and holes: the reference's own face/edge counts for curved or pierced slabs are not yet measured.

use crate::curve::{Aabb2, Arc, Curve, LineSegment};
use crate::error::{GeomError, GeomResult};
use crate::tolerance::Tolerances;
use crate::vec::Vec2;
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

/// Unsigned area of a polygon given by its corner points (shoelace).
pub fn polygon_signed_area(points: &[Vec2]) -> f64 {
    let n = points.len();
    if n < 3 {
        return 0.0;
    }
    let twice: f64 = (0..n).map(|i| points[i].cross(points[(i + 1) % n])).sum();
    twice / 2.0
}

/// Perimeter of the closed polygon through `points`.
pub fn polygon_perimeter(points: &[Vec2]) -> f64 {
    let n = points.len();
    if n < 2 {
        return 0.0;
    }
    (0..n).map(|i| (points[(i + 1) % n] - points[i]).length()).sum()
}

/// Crossing-number point-in-polygon test on the corner points.
pub fn polygon_contains_point(points: &[Vec2], q: Vec2) -> bool {
    let n = points.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        // Half-open rule on y avoids double-counting shared vertices.
        if (a.y > q.y) != (b.y > q.y) {
            let x = a.x + (q.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if q.x < x {
                inside = !inside;
            }
        }
    }
    inside
}

/// Is `q` inside the region bounded by the arc and its own chord (the
/// "circular segment" lune)?
///
/// Unified rule, valid for any sweep: `q` is in the lune iff it is inside the
/// circle **and** on the same side of the chord as the arc's midpoint. For
/// `|sweep| < pi` that is the minor segment; for `|sweep| > pi` the arc's
/// midpoint falls on the centre's side and the rule selects the major segment,
/// which is exactly the region the arc plus chord encloses.
///
/// Points lying **on** the chord are reported as outside (the chord is the
/// lune's own boundary). That is why `Polycurve::contains_point` does *not*
/// build on this predicate: a chord is an artefact, not part of the real
/// boundary, so a point on it (e.g. the centre of a circle built from two half
/// arcs) must still be classified against the true loop. Containment therefore
/// uses `ray_crossings_*` below.
pub fn arc_segment_contains_point(arc: &Arc, q: Vec2) -> bool {
    if q.distance_to(arc.centre) >= arc.radius {
        return false;
    }
    let a = arc.start();
    let b = arc.end();
    let chord = b - a;
    if chord.length() == 0.0 {
        // Full circle: the whole disc is "inside".
        return true;
    }
    let side_q = chord.cross(q - a);
    let side_mid = chord.cross(arc.point_at(0.5) - a);
    side_q * side_mid > 0.0
}

/// Crossings of the ray `y = q.y, x >= q.x` with a straight segment (0 or 1).
///
/// Half-open rule on `y` (`[y_start, y_end)`): a vertex shared by two curves is
/// counted once, so the parity of the total is well defined.
pub fn ray_crossings_segment(a: Vec2, b: Vec2, q: Vec2) -> u32 {
    if (a.y > q.y) == (b.y > q.y) {
        return 0;
    }
    let x = a.x + (q.y - a.y) / (b.y - a.y) * (b.x - a.x);
    u32::from(x > q.x)
}

/// Crossings of the ray `y = q.y, x >= q.x` with a circular arc (0, 1 or 2).
pub fn ray_crossings_arc(arc: &Arc, q: Vec2) -> u32 {
    ray_crossings_arc_chained(arc, q, arc.start().y, arc.end().y)
}

/// Same as [`ray_crossings_arc`], but with the endpoint `y` values supplied by
/// the caller.
///
/// # Why the override exists
///
/// A closed loop shares each vertex between two curves, yet the two sides
/// evaluate it from different parameters: a circle built from the arcs
/// `[0, pi]` and `[pi, 2pi]` has `r * sin(0) = 0` on one side of the seam and
/// `r * sin(2pi) = -1.2e-15 * r` on the other. A ray whose `y` falls between
/// those two values sees a 1e-15 gap in the loop and loses a crossing, which
/// flips the answer. Feeding the **chain-consistent** endpoint (the next
/// curve's start) to both sides removes the inconsistency, which is what
/// `Polycurve::contains_point` does.
///
/// The arc is split at its vertical extremes (`pi/2 + k*pi`) into y-monotone
/// pieces; each piece meets a horizontal line at most once, and that point is
/// found analytically from `sin(theta) = (q.y - cy) / r`. No tessellation. When
/// the monotone piece straddles `q.y` a crossing must exist, so if the analytic
/// angle lands marginally outside the piece (again, 1e-15 effects) it is
/// clamped into it instead of being dropped.
pub fn ray_crossings_arc_chained(arc: &Arc, q: Vec2, start_y: f64, end_y: f64) -> u32 {
    let s = (q.y - arc.centre.y) / arc.radius;
    if s.abs() > 1.0 {
        return 0;
    }
    let base = s.clamp(-1.0, 1.0).asin();
    let pieces = monotone_pieces(arc);
    let last = pieces.len() - 1;
    let mut crossings = 0;
    for (i, (t0, t1)) in pieces.iter().copied().enumerate() {
        let (a0, a1) = (arc.angle_at(t0), arc.angle_at(t1));
        let y0 = if i == 0 { start_y } else { arc.point_at(t0).y };
        let y1 = if i == last { end_y } else { arc.point_at(t1).y };
        if (y0 > q.y) == (y1 > q.y) {
            continue;
        }
        let (lo, hi) = if a0 <= a1 { (a0, a1) } else { (a1, a0) };
        // Candidate angles with the required sine, over the few periods that
        // can overlap the piece; keep the one closest to [lo, hi].
        let mut best = f64::INFINITY;
        let mut hit = lo;
        for k in -2..=2 {
            for cand in [base + TAU * k as f64, PI - base + TAU * k as f64] {
                let d = (lo - cand).max(cand - hi).max(0.0);
                if d < best {
                    best = d;
                    hit = cand.clamp(lo, hi);
                }
            }
        }
        if arc.point_at_angle(hit).x > q.x {
            crossings += 1;
        }
    }
    crossings
}

/// Parameter intervals of the arc's y-monotone pieces, in travel order.
fn monotone_pieces(arc: &Arc) -> Vec<(f64, f64)> {
    let start = arc.start_angle;
    let end = arc.end_angle();
    let mut splits: Vec<f64> = Vec::new();
    if arc.sweep > 0.0 {
        // smallest pi/2 + k*pi strictly greater than start
        let mut k = ((start - PI / 2.0) / PI).floor() + 1.0;
        loop {
            let a = PI / 2.0 + k * PI;
            if a >= end {
                break;
            }
            if a > start {
                splits.push(a);
            }
            k += 1.0;
        }
    } else {
        let mut k = ((start - PI / 2.0) / PI).ceil() - 1.0;
        loop {
            let a = PI / 2.0 + k * PI;
            if a <= end {
                break;
            }
            if a < start {
                splits.push(a);
            }
            k -= 1.0;
        }
    }
    let to_t = |angle: f64| (angle - start) / arc.sweep;
    let mut ts: Vec<f64> = vec![0.0];
    ts.extend(splits.into_iter().map(to_t));
    ts.push(1.0);
    ts.windows(2).map(|w| (w[0], w[1])).collect()
}

/// A closed loop of curves, traversed in order. Invariants (checked at
/// construction): no curve shorter than `Tolerances::short_curve`, consecutive
/// curves meet within `Tolerances::vertex`, and the loop closes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Polycurve {
    curves: Vec<Curve>,
}

impl Polycurve {
    /// Build a validated closed loop.
    pub fn closed(curves: Vec<Curve>, tol: &Tolerances) -> GeomResult<Self> {
        if curves.is_empty() {
            return Err(GeomError::LoopTooFewCurves { count: 0, minimum: 1 });
        }
        for c in &curves {
            let len = c.length();
            if len < tol.short_curve {
                return Err(GeomError::CurveTooShort { length: len, tolerance: tol.short_curve });
            }
        }
        let n = curves.len();
        for i in 0..n {
            let gap = curves[i].end().distance_to(curves[(i + 1) % n].start());
            if gap > tol.vertex {
                return Err(GeomError::LoopNotClosed { gap, tolerance: tol.vertex });
            }
        }
        Ok(Polycurve { curves })
    }

    /// Closed polygon through `points` (at least 3).
    pub fn polygon(points: &[Vec2], tol: &Tolerances) -> GeomResult<Self> {
        if points.len() < 3 {
            return Err(GeomError::LoopTooFewCurves { count: points.len(), minimum: 3 });
        }
        let n = points.len();
        let curves = (0..n)
            .map(|i| Curve::line(points[i], points[(i + 1) % n]))
            .collect();
        Polycurve::closed(curves, tol)
    }

    /// Axis-aligned rectangle, counter-clockwise.
    pub fn rectangle(min: Vec2, max: Vec2, tol: &Tolerances) -> GeomResult<Self> {
        Polycurve::polygon(
            &[min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)],
            tol,
        )
    }

    /// Full circle as two half arcs (the usual sketch representation).
    pub fn circle(centre: Vec2, radius: f64, tol: &Tolerances) -> GeomResult<Self> {
        let upper = Arc::new(centre, radius, 0.0, PI)?;
        let lower = Arc::new(centre, radius, PI, PI)?;
        Polycurve::closed(vec![Curve::Arc(upper), Curve::Arc(lower)], tol)
    }

    pub fn curves(&self) -> &[Curve] {
        &self.curves
    }
    pub fn curve_count(&self) -> usize {
        self.curves.len()
    }
    pub fn arc_count(&self) -> usize {
        self.curves.iter().filter(|c| c.is_arc()).count()
    }
    pub fn has_arcs(&self) -> bool {
        self.arc_count() > 0
    }
    /// The loop's corner points: the start point of every curve. For a polygon these are its vertices, which keeps the
    /// measured "N boundary points <=> N curves" relation.
    pub fn corner_points(&self) -> Vec<Vec2> {
        self.curves.iter().map(|c| c.start()).collect()
    }
    /// Exact perimeter (arc length for arcs, chord length for segments).
    pub fn perimeter(&self) -> f64 {
        self.curves.iter().map(|c| c.length()).sum()
    }
    /// Signed area: positive counter-clockwise. Exact (Green's theorem).
    pub fn signed_area(&self) -> f64 {
        self.curves.iter().map(|c| c.signed_area_term()).sum()
    }
    pub fn area(&self) -> f64 {
        self.signed_area().abs()
    }
    pub fn is_counter_clockwise(&self) -> bool {
        self.signed_area() > 0.0
    }
    pub fn aabb(&self) -> Aabb2 {
        let mut it = self.curves.iter();
        let mut b = it.next().expect("validated loop is non-empty").aabb();
        for c in it {
            b = b.union(c.aabb());
        }
        b
    }
    pub fn reversed(&self) -> Self {
        let curves = self.curves.iter().rev().map(|c| c.reversed()).collect();
        Polycurve { curves }
    }
    /// Make the loop run counter-clockwise (`true`) or clockwise (`false`).
    pub fn oriented(&self, counter_clockwise: bool) -> Self {
        if self.is_counter_clockwise() == counter_clockwise {
            self.clone()
        } else {
            self.reversed()
        }
    }
    /// Exact point containment by ray crossing against the **real** curves.
    ///
    /// The ray `y = q.y, x >= q.x` is intersected analytically with every
    /// segment and every arc (arcs are split into y-monotone pieces first);
    /// an odd number of crossings means inside. This works for loops of 1 or 2
    /// curves (a full circle), for arcs that bulge either way, and for points
    /// that happen to sit on an arc's chord — the chord is not part of the
    /// boundary, so it must not influence the answer.
    ///
    /// Points exactly **on** the boundary are not classified (the usual
    /// ray-casting caveat); callers that care use a tolerance band instead.
    pub fn contains_point(&self, q: Vec2) -> bool {
        let n = self.curves.len();
        let mut crossings: u32 = 0;
        for i in 0..n {
            // Chain-consistent endpoints: the shared vertex has one value.
            let start = self.curves[i].start();
            let end = self.curves[(i + 1) % n].start();
            crossings += match &self.curves[i] {
                Curve::Line(_) => ray_crossings_segment(start, end, q),
                Curve::Arc(a) => ray_crossings_arc_chained(a, q, start.y, end.y),
            };
        }
        crossings % 2 == 1
    }
    /// Farthest point of the loop along `direction` (support function).
    /// Exact for arcs: a radial extreme inside the sweep beats the endpoints.
    /// This is what gives an inclined slab an exact elevation range.
    pub fn support_point(&self, direction: Vec2) -> Vec2 {
        let mut best = self.curves[0].support_point(direction);
        let mut score = best.dot(direction);
        for c in &self.curves[1..] {
            let p = c.support_point(direction);
            let s = p.dot(direction);
            if s > score {
                best = p;
                score = s;
            }
        }
        best
    }
    /// Merge consecutive curves that are really one curve: collinear segments
    /// in the same direction, and arcs on the same circle turning the same way.
    ///
    /// This does not change any metric (area, perimeter and the bounding box are identical); it changes the **topology
    /// counts**, which is why it matters. The reference merges join-face edges the same way (*"the reference product merges
    /// join-face edges"*), so a boolean result has to be normalised before its face/edge/vertex counts can be compared with
    /// a measurement.
    pub fn simplified(&self, tol: &Tolerances) -> GeomResult<Self> {
        let mut curves = self.curves.clone();
        let mut changed = true;
        while changed && curves.len() > 2 {
            changed = false;
            let n = curves.len();
            for i in 0..n {
                let j = (i + 1) % n;
                if let Some(merged) = merge_curves(&curves[i], &curves[j], tol) {
                    curves[i] = merged;
                    curves.remove(j);
                    changed = true;
                    break;
                }
            }
        }
        Polycurve::closed(curves, tol)
    }

    /// Display/hit-test polyline. Never used for quantities.
    pub fn tessellate(&self, max_sagitta: f64) -> Vec<Vec2> {
        let mut out: Vec<Vec2> = Vec::new();
        for c in &self.curves {
            let pts = c.tessellate(max_sagitta);
            let skip = if out.is_empty() { 0 } else { 1 };
            out.extend_from_slice(&pts[skip..]);
        }
        if out.len() > 1 && out[0].distance_to(*out.last().unwrap()) == 0.0 {
            out.pop();
        }
        out
    }
    /// Total turning of the tangent along the loop; `+2*pi` for a simple
    /// counter-clockwise loop, `-2*pi` clockwise. Useful as a sanity check that
    /// an arc loop is simple (no inner self-wraps).
    pub fn total_turning(&self) -> f64 {
        let n = self.curves.len();
        let mut turning = 0.0;
        for c in &self.curves {
            if let Some(arc) = c.as_arc() {
                turning += arc.sweep;
            }
        }
        for i in 0..n {
            let a = &self.curves[i];
            let b = &self.curves[(i + 1) % n];
            if let (Some(t0), Some(t1)) = (a.tangent_at(1.0), b.tangent_at(0.0)) {
                turning += t0.cross(t1).atan2(t0.dot(t1));
            }
        }
        turning
    }
    /// Is the loop plausibly simple? Checks the turning number against
    /// `+-2*pi` within the angle tolerance.
    pub fn is_simple_by_turning(&self, tol: &Tolerances) -> bool {
        (self.total_turning().abs() - TAU).abs() <= tol.angle.max(1e-9)
    }
}

/// A planar region: one outer loop and any number of inner loops (holes).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Region {
    outer: Polycurve,
    holes: Vec<Polycurve>,
}

impl Region {
    /// Canonicalise orientations and validate that every hole lies inside the
    /// outer loop.
    pub fn new(outer: Polycurve, holes: Vec<Polycurve>, _tol: &Tolerances) -> GeomResult<Self> {
        let outer = outer.oriented(true);
        let mut canonical: Vec<Polycurve> = Vec::with_capacity(holes.len());
        for (i, h) in holes.into_iter().enumerate() {
            let h = h.oriented(false);
            // Sample the hole: every corner and every curve midpoint must be
            // inside the outer loop.
            let inside = h
                .curves()
                .iter()
                .all(|c| outer.contains_point(c.start()) && outer.contains_point(c.midpoint()));
            if !inside {
                return Err(GeomError::HoleNotInsideOuterLoop { hole_index: i });
            }
            canonical.push(h);
        }
        Ok(Region { outer, holes: canonical })
    }

    pub fn from_loop(outer: Polycurve) -> Self {
        Region { outer: outer.oriented(true), holes: Vec::new() }
    }

    pub fn from_polygon(points: &[Vec2], tol: &Tolerances) -> GeomResult<Self> {
        Ok(Region::from_loop(Polycurve::polygon(points, tol)?))
    }

    pub fn outer(&self) -> &Polycurve {
        &self.outer
    }
    pub fn holes(&self) -> &[Polycurve] {
        &self.holes
    }
    pub fn loop_count(&self) -> usize {
        1 + self.holes.len()
    }
    pub fn loops(&self) -> impl Iterator<Item = &Polycurve> {
        std::iter::once(&self.outer).chain(self.holes.iter())
    }
    /// Number of lateral faces under the reference's counting convention.
    ///
    /// **Rule (OBSERVED Section 1, faces column):** boundary pieces lying on the same support (line or circle) and running
    /// the same way are **one face when the gap between them is a cut**, and **separate faces when the gap between them is
    /// material**.
    ///
    /// The three measured counts that pin it, on a 6.000 x 3.000 wall face:
    ///
    /// * window only: 6 + 4 = **10** (a hole adds four planes);
    /// * door flush with the base: 6 + 3 = **9**. The door splits the base
    ///   edge into two collinear pieces; the gap between them is the door
    ///   opening itself, i.e. *outside* the material, so they stay one face;
    /// * window + door: **13**, not 12. The window head and the door head both
    ///   sit at z = 2.100 - same plane, same direction - but the wall between
    ///   x = 2.2 and x = 3.0 is *material*, so they are two faces.
    ///
    /// A first version of this method merged every coplanar pair and returned
    /// 12 for the last case; the round-9 persistence test exposed it.
    pub fn distinct_support_count(&self, tol: &Tolerances) -> usize {
        distinct_support_count_all(std::slice::from_ref(self), tol)
    }

    /// Total number of boundary curves over all loops. This is also the number of plan corner points, which is what the
    /// measured topology convention counts.
    pub fn curve_count(&self) -> usize {
        self.loops().map(|l| l.curve_count()).sum()
    }
    pub fn has_arcs(&self) -> bool {
        self.loops().any(|l| l.has_arcs())
    }
    /// Net area: outer minus holes. Exact.
    pub fn area(&self) -> f64 {
        self.outer.area() - self.holes.iter().map(|h| h.area()).sum::<f64>()
    }
    /// Perimeter of the outer loop only.
    ///
    /// Callers that need the full edge length (e.g. lateral surface of an extrusion) must use `total_boundary_length`.
    pub fn outer_perimeter(&self) -> f64 {
        self.outer.perimeter()
    }
    pub fn hole_perimeter(&self) -> f64 {
        self.holes.iter().map(|h| h.perimeter()).sum()
    }
    /// Outer plus every hole: the length that generates lateral faces.
    pub fn total_boundary_length(&self) -> f64 {
        self.outer_perimeter() + self.hole_perimeter()
    }
    pub fn aabb(&self) -> Aabb2 {
        self.outer.aabb()
    }
    pub fn contains_point(&self, q: Vec2) -> bool {
        self.outer.contains_point(q) && !self.holes.iter().any(|h| h.contains_point(q))
    }
    /// Centroid of the net region (area-weighted, exact for polygons; arcs are
    /// handled through their chord polygon plus circular-segment centroids).
    pub fn centroid(&self) -> Option<Vec2> {
        let a = self.area();
        if a <= 0.0 {
            return None;
        }
        let mut acc = Vec2::new(0.0, 0.0);
        for (sign, lp) in std::iter::once((1.0, &self.outer)).chain(self.holes.iter().map(|h| (-1.0, h)))
        {
            let (m, c) = loop_area_moment(lp);
            acc = acc + c * (sign * m);
        }
        Some(acc * (1.0 / a))
    }
    /// Convenience accessor used by extrusions and display code.
    pub fn outer_corner_points(&self) -> Vec<Vec2> {
        self.outer.corner_points()
    }
}

/// The infinite support of a curve: the line a segment lies on, or the circle
/// an arc lies on. Two curves with the same support generate one face under the
/// reference's counting convention.
#[derive(Clone, Copy, Debug)]
enum CurveSupport {
    /// Canonical line: unit normal (sign-normalised) and signed offset.
    Line { normal: Vec2, offset: f64 },
    Circle { centre: Vec2, radius: f64 },
}

impl CurveSupport {
    fn of(c: &Curve) -> Self {
        match c {
            Curve::Line(l) => {
                let n = l
                    .direction()
                    .normalized()
                    .unwrap_or(Vec2::new(1.0, 0.0))
                    .perp_ccw();
                // Sign-normalise so a segment and its reverse share a key.
                let flip = n.x < 0.0 || (n.x == 0.0 && n.y < 0.0);
                let normal = if flip { -n } else { n };
                CurveSupport::Line { normal, offset: normal.dot(l.start) }
            }
            Curve::Arc(a) => CurveSupport::Circle { centre: a.centre, radius: a.radius },
        }
    }
    fn same_as(&self, other: &CurveSupport, tol: &Tolerances) -> bool {
        match (self, other) {
            (
                CurveSupport::Line { normal: n0, offset: d0 },
                CurveSupport::Line { normal: n1, offset: d1 },
            ) => n0.cross(*n1).abs() <= tol.angle.sin() && (d0 - d1).abs() <= tol.vertex,
            (
                CurveSupport::Circle { centre: c0, radius: r0 },
                CurveSupport::Circle { centre: c1, radius: r1 },
            ) => c0.distance_to(*c1) <= tol.vertex && (r0 - r1).abs() <= tol.vertex,
            _ => false,
        }
    }
}

/// Lateral face count of a **set** of regions treated as one solid - the
/// reference's convention for a host whose cut severs it into pieces.
///
/// Section 1 rule 9 says *"the solid stays a single solid throughout"*, including after an opening wider than the wall cuts
/// the face into three disconnected pieces. Counting each piece separately gives 30; the reference reports 11. With the
/// pieces taken together, coplanar boundary pieces whose gap is a cut merge across pieces (the two halves of the base, the
/// left edge above and below the band, ...), and the count is exactly 11. Material, for the gap test, is the union of all
/// pieces.
pub fn distinct_support_count_all(regions: &[Region], tol: &Tolerances) -> usize {
    let curves: Vec<Curve> = regions
        .iter()
        .flat_map(|r| r.loops().flat_map(|l| l.curves().to_vec()).collect::<Vec<_>>())
        .collect();
    let n = curves.len();
    if n == 0 {
        return 0;
    }
    let supports: Vec<CurveSupport> = curves.iter().map(CurveSupport::of).collect();

    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while parent[r] != r {
            r = parent[r];
        }
        let mut c = i;
        while parent[c] != r {
            let next = parent[c];
            parent[c] = r;
            c = next;
        }
        r
    }

    // Group by (same support, same direction), in first-seen order.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for i in 0..n {
        match groups.iter_mut().find(|g| {
            supports[g[0]].same_as(&supports[i], tol) && same_direction(&curves[g[0]], &curves[i])
        }) {
            Some(g) => g.push(i),
            None => groups.push(vec![i]),
        }
    }

    let material = |q: Vec2| regions.iter().any(|r| r.contains_point(q));
    for g in &groups {
        if g.len() < 2 {
            continue;
        }
        // Order the pieces along their common support, then link each piece
        // to the next one when the gap between them is not material.
        let mut ordered: Vec<(f64, f64, usize)> = g
            .iter()
            .map(|&i| {
                let (a, b) = support_interval(&curves[i], &supports[i]);
                (a.min(b), a.max(b), i)
            })
            .collect();
        ordered.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.total_cmp(&y.1)));
        let circular = matches!(supports[g[0]], CurveSupport::Circle { .. });
        let m = ordered.len();
        let pairs = if circular { m } else { m - 1 };
        for k in 0..pairs {
            let (_, end_a, ia) = ordered[k];
            let (start_b, _, ib) = ordered[(k + 1) % m];
            let start_b = if circular && k + 1 == m {
                start_b + std::f64::consts::TAU
            } else {
                start_b
            };
            if gap_is_not_material(&supports[ia], end_a, start_b, &material, tol) {
                let (ra, rb) = (find(&mut parent, ia), find(&mut parent, ib));
                if ra != rb {
                    parent[ra] = rb;
                }
            }
        }
    }

    let mut roots: Vec<usize> = (0..n).map(|i| find(&mut parent, i)).collect();
    roots.sort_unstable();
    roots.dedup();
    roots.len()
}

/// Is the stretch of `support` between parameters `from` and `to` free of
/// material? A touching pair (no gap) counts as free: it is a straight
/// continuation of one face.
fn gap_is_not_material(
    support: &CurveSupport,
    from: f64,
    to: f64,
    material: &dyn Fn(Vec2) -> bool,
    tol: &Tolerances,
) -> bool {
    match *support {
        CurveSupport::Line { normal, offset } => {
            if to - from <= tol.vertex {
                return true;
            }
            let dir = Vec2::new(-normal.y, normal.x);
            let mid = normal * offset + dir * ((from + to) / 2.0);
            !material(mid)
        }
        CurveSupport::Circle { centre, radius } => {
            if (to - from) * radius <= tol.vertex {
                return true;
            }
            let mid = centre + Vec2::from_angle((from + to) / 2.0) * radius;
            !material(mid)
        }
    }
}

/// Do two curves run the same way? Segments: positive dot of directions.
/// Arcs: same sweep sign.
fn same_direction(a: &Curve, b: &Curve) -> bool {
    match (a, b) {
        (Curve::Line(x), Curve::Line(y)) => x.direction().dot(y.direction()) > 0.0,
        (Curve::Arc(x), Curve::Arc(y)) => x.sweep.signum() == y.sweep.signum(),
        _ => false,
    }
}

/// Parameter interval a curve occupies on its support: distance along the
/// canonical line direction, or absolute angle (radians, normalised to start
/// in [0, 2pi)) on the circle.
fn support_interval(c: &Curve, support: &CurveSupport) -> (f64, f64) {
    match (c, support) {
        (Curve::Line(l), CurveSupport::Line { normal, .. }) => {
            let dir = Vec2::new(-normal.y, normal.x);
            (l.start.dot(dir), l.end.dot(dir))
        }
        (Curve::Arc(a), CurveSupport::Circle { .. }) => {
            let (lo, hi) = if a.sweep > 0.0 {
                (a.start_angle, a.end_angle())
            } else {
                (a.end_angle(), a.start_angle)
            };
            let shift = crate::curve::normalize_angle_positive(lo) - lo;
            (lo + shift, hi + shift)
        }
        _ => (0.0, 0.0),
    }
}

/// Merge two consecutive curves into one when they lie on the same support
/// and run the same way. `None` when they genuinely are two curves.
fn merge_curves(a: &Curve, b: &Curve, tol: &Tolerances) -> Option<Curve> {
    if a.end().distance_to(b.start()) > tol.vertex {
        return None;
    }
    match (a, b) {
        (Curve::Line(l0), Curve::Line(l1)) => {
            let (d0, d1) = (l0.direction(), l1.direction());
            let (n0, n1) = (d0.normalized()?, d1.normalized()?);
            if n0.dot(n1) <= 0.0 || n0.cross(n1).abs() > tol.angle.sin() {
                return None;
            }
            Some(Curve::Line(LineSegment::new(l0.start, l1.end)))
        }
        (Curve::Arc(a0), Curve::Arc(a1)) => {
            if a0.centre.distance_to(a1.centre) > tol.vertex
                || (a0.radius - a1.radius).abs() > tol.vertex
                || a0.sweep.signum() != a1.sweep.signum()
            {
                return None;
            }
            let sweep = a0.sweep + a1.sweep;
            if sweep.abs() >= TAU {
                return None;
            }
            Arc::new(a0.centre, a0.radius, a0.start_angle, sweep).ok().map(Curve::Arc)
        }
        _ => None,
    }
}

/// Unsigned area and area-weighted centroid contribution of a loop.
fn loop_area_moment(lp: &Polycurve) -> (f64, Vec2) {
    let corners = lp.corner_points();
    let mut area = polygon_signed_area(&corners);
    let mut moment = polygon_centroid_moment(&corners);
    for c in lp.curves() {
        if let Some(arc) = c.as_arc() {
            let seg_area = arc.circular_segment_area();
            area += seg_area;
            moment = moment + circular_segment_centroid(arc) * seg_area;
        }
    }
    // Normalise sign so the result is an unsigned area with a real centroid.
    if area < 0.0 {
        (-area, moment * (1.0 / area))
    } else if area > 0.0 {
        (area, moment * (1.0 / area))
    } else {
        (0.0, Vec2::new(0.0, 0.0))
    }
}

fn polygon_centroid_moment(points: &[Vec2]) -> Vec2 {
    let n = points.len();
    if n < 3 {
        return Vec2::new(0.0, 0.0);
    }
    let mut acc = Vec2::new(0.0, 0.0);
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        let cross = a.cross(b);
        acc = acc + (a + b) * (cross / 6.0);
    }
    acc
}

/// Centroid of the circular segment between an arc and its chord.
/// Distance from the centre: `4 r sin^3(theta/2) / (3 (theta - sin theta))`
/// along the bisector, with `theta = |sweep|`.
fn circular_segment_centroid(arc: &Arc) -> Vec2 {
    let theta = arc.sweep.abs();
    let denom = theta - theta.sin();
    let bisector = Vec2::from_angle(arc.start_angle + arc.sweep / 2.0);
    if denom.abs() < 1e-15 {
        return arc.centre + bisector * arc.radius;
    }
    let half = theta / 2.0;
    let d = 4.0 * arc.radius * half.sin().powi(3) / (3.0 * denom);
    arc.centre + bisector * d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LineSegment;

    fn tol() -> Tolerances {
        Tolerances::default()
    }
    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-12, "{a} != {b}");
    }
    fn near_eps(a: f64, b: f64, eps: f64) {
        assert!((a - b).abs() < eps, "{a} != {b}");
    }

    #[test]
    fn rectangle_matches_the_polygon_formulas() {
        let r = Polycurve::rectangle(Vec2::new(0.0, 0.0), Vec2::new(6.0, 4.0), &tol()).unwrap();
        near(r.area(), 24.0);
        near(r.perimeter(), 20.0);
        assert!(r.is_counter_clockwise());
        assert_eq!(r.curve_count(), 4);
        assert_eq!(r.corner_points().len(), 4);
    }

    #[test]
    fn floor_specification_shapes_still_match() {
        // Section 3 (OBSERVED): 6x4 => 24.0 / 20.0; L-shape => 36.0 / 28.0; triangle 5x4/2 => 10.0 / 5 + 4 + sqrt(41).
        let l = Polycurve::polygon(
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
        near(l.area(), 36.0);
        near(l.perimeter(), 28.0);
        let t = Polycurve::polygon(
            &[Vec2::new(0.0, 0.0), Vec2::new(5.0, 0.0), Vec2::new(0.0, 4.0)],
            &tol(),
        )
        .unwrap();
        near(t.area(), 10.0);
        near(t.perimeter(), 5.0 + 4.0 + 41.0_f64.sqrt());
    }

    #[test]
    fn half_disc_area_and_perimeter_are_exact() {
        // Chord from (-1,0) to (1,0) plus the upper unit semicircle.
        let arc = Arc::from_center_angles(Vec2::new(0.0, 0.0), 1.0, 0.0, PI, true).unwrap();
        let lp = Polycurve::closed(
            vec![
                Curve::Arc(arc),
                Curve::Line(LineSegment::new(Vec2::new(-1.0, 0.0), Vec2::new(1.0, 0.0))),
            ],
            &tol(),
        )
        .unwrap();
        near(lp.area(), PI / 2.0);
        near(lp.perimeter(), PI + 2.0);
        assert!(lp.is_counter_clockwise());
    }

    #[test]
    fn full_circle_from_two_arcs_is_pi_r_squared() {
        let c = Polycurve::circle(Vec2::new(3.0, -2.0), 2.5, &tol()).unwrap();
        near(c.area(), PI * 2.5 * 2.5);
        near(c.perimeter(), TAU * 2.5);
        assert_eq!(c.curve_count(), 2);
        assert!(c.contains_point(Vec2::new(3.0, -2.0)));
        assert!(!c.contains_point(Vec2::new(3.0 + 2.5001, -2.0)));
    }

    #[test]
    fn quarter_pie_area_matches_hand_value() {
        // Pie slice r = 2, 90 degrees: area = pi r^2 / 4 = pi.
        let arc = Arc::from_center_angles(Vec2::new(0.0, 0.0), 2.0, 0.0, PI / 2.0, true).unwrap();
        let lp = Polycurve::closed(
            vec![
                Curve::Arc(arc),
                Curve::line(Vec2::new(0.0, 2.0), Vec2::new(0.0, 0.0)),
                Curve::line(Vec2::new(0.0, 0.0), Vec2::new(2.0, 0.0)),
            ],
            &tol(),
        )
        .unwrap();
        near(lp.area(), PI);
        near(lp.perimeter(), PI + 4.0);
    }

    #[test]
    fn stadium_shape_mixes_lines_and_half_arcs() {
        // Straight length 4, radius 1: area = 4*2 + pi*1^2, perimeter = 8 + 2*pi.
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
        near(lp.area(), 8.0 + PI);
        near(lp.perimeter(), 8.0 + TAU);
        assert!(lp.is_simple_by_turning(&tol()));
    }

    #[test]
    fn l_shape_with_a_rounded_corner() {
        // Square 4x4 with the top-right corner replaced by a fillet of r = 1.
        // Area = 16 - (1 - pi/4) ; perimeter = 4 + 3 + pi/2 + 3 + 4 ... built below.
        let r = 1.0;
        let centre = Vec2::new(4.0 - r, 4.0 - r);
        let fillet = Arc::from_center_angles(centre, r, 0.0, PI / 2.0, true).unwrap();
        let lp = Polycurve::closed(
            vec![
                Curve::line(Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0)),
                Curve::line(Vec2::new(4.0, 0.0), Vec2::new(4.0, 4.0 - r)),
                Curve::Arc(fillet),
                Curve::line(Vec2::new(4.0 - r, 4.0), Vec2::new(0.0, 4.0)),
                Curve::line(Vec2::new(0.0, 4.0), Vec2::new(0.0, 0.0)),
            ],
            &tol(),
        )
        .unwrap();
        // The removed corner is the square r^2 minus the quarter disc.
        near(lp.area(), 16.0 - (r * r - PI * r * r / 4.0));
        near(lp.perimeter(), 4.0 + (4.0 - r) + PI * r / 2.0 + (4.0 - r) + 4.0);
        assert!(lp.is_simple_by_turning(&tol()));
    }

    #[test]
    fn inward_bulging_arc_subtracts_area() {
        // Same square, but the arc is centred on the corner and runs clockwise,
        // so it bites into the square: the area drops by the quarter disc.
        let r = 1.0;
        let a = Arc::from_center_angles(
            Vec2::new(4.0, 4.0),
            r,
            3.0 * PI / 2.0,
            PI,
            false,
        )
        .unwrap();
        let lp = Polycurve::closed(
            vec![
                Curve::line(Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0)),
                Curve::line(Vec2::new(4.0, 0.0), Vec2::new(4.0, 3.0)),
                Curve::Arc(a),
                Curve::line(Vec2::new(3.0, 4.0), Vec2::new(0.0, 4.0)),
                Curve::line(Vec2::new(0.0, 4.0), Vec2::new(0.0, 0.0)),
            ],
            &tol(),
        )
        .unwrap();
        near(lp.area(), 16.0 - PI * r * r / 4.0);
    }

    #[test]
    fn reversed_loop_has_the_opposite_sign_and_the_same_metrics() {
        let lp = Polycurve::circle(Vec2::new(0.0, 0.0), 3.0, &tol()).unwrap();
        let rv = lp.reversed();
        near(rv.signed_area(), -lp.signed_area());
        near(rv.perimeter(), lp.perimeter());
        assert!(rv.contains_point(Vec2::new(0.0, 0.0)));
    }

    #[test]
    fn annulus_area_is_outer_minus_inner() {
        let outer = Polycurve::circle(Vec2::new(0.0, 0.0), 5.0, &tol()).unwrap();
        let inner = Polycurve::circle(Vec2::new(0.0, 0.0), 2.0, &tol()).unwrap();
        let region = Region::new(outer, vec![inner], &tol()).unwrap();
        near(region.area(), PI * (25.0 - 4.0));
        near(region.outer_perimeter(), TAU * 5.0);
        near(region.hole_perimeter(), TAU * 2.0);
        near(region.total_boundary_length(), TAU * 7.0);
        assert_eq!(region.curve_count(), 4);
        assert_eq!(region.loop_count(), 2);
        // Inside the ring, not in the hole, not outside.
        assert!(region.contains_point(Vec2::new(3.5, 0.0)));
        assert!(!region.contains_point(Vec2::new(1.0, 0.0)));
        assert!(!region.contains_point(Vec2::new(6.0, 0.0)));
    }

    #[test]
    fn slab_with_two_rectangular_openings() {
        let outer = Polycurve::rectangle(Vec2::new(0.0, 0.0), Vec2::new(6.0, 4.0), &tol()).unwrap();
        let h1 = Polycurve::rectangle(Vec2::new(1.0, 1.0), Vec2::new(2.0, 2.0), &tol()).unwrap();
        let h2 = Polycurve::rectangle(Vec2::new(3.0, 1.0), Vec2::new(5.0, 3.0), &tol()).unwrap();
        let region = Region::new(outer, vec![h1, h2], &tol()).unwrap();
        near(region.area(), 24.0 - 1.0 - 4.0);
        near(region.outer_perimeter(), 20.0);
        near(region.hole_perimeter(), 4.0 + 8.0);
        assert_eq!(region.curve_count(), 12);
        // Holes are canonicalised clockwise.
        for h in region.holes() {
            assert!(!h.is_counter_clockwise());
        }
        assert!(region.outer().is_counter_clockwise());
    }

    #[test]
    fn a_hole_outside_the_outer_loop_is_rejected() {
        let outer = Polycurve::rectangle(Vec2::new(0.0, 0.0), Vec2::new(2.0, 2.0), &tol()).unwrap();
        let away = Polycurve::rectangle(Vec2::new(5.0, 5.0), Vec2::new(6.0, 6.0), &tol()).unwrap();
        let e = Region::new(outer, vec![away], &tol()).unwrap_err();
        assert_eq!(e.code(), crate::error::codes::HOLE_NOT_INSIDE_OUTER_LOOP);
    }

    #[test]
    fn open_loop_and_short_curve_are_rejected_with_their_codes() {
        let open = Polycurve::closed(
            vec![
                Curve::line(Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0)),
                Curve::line(Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0)),
            ],
            &tol(),
        )
        .unwrap_err();
        assert_eq!(open.code(), crate::error::codes::LOOP_NOT_CLOSED);

        // 0.5 mm edge is below the observed ShortCurveTolerance (0.78 mm).
        let tiny = Polycurve::polygon(
            &[
                Vec2::new(0.0, 0.0),
                Vec2::new(0.0005, 0.0),
                Vec2::new(0.0005, 0.0005),
                Vec2::new(0.0, 0.0005),
            ],
            &tol(),
        )
        .unwrap_err();
        assert_eq!(tiny.code(), crate::error::codes::CURVE_TOO_SHORT);
    }

    #[test]
    fn two_millimetre_slab_boundary_is_accepted_like_the_reference() {
        // Section 5 (OBSERVED): a 2 mm x 2 mm boundary commits. 2 mm > ShortCurveTolerance 0.78 mm, so the loop is valid.
        let lp = Polycurve::rectangle(Vec2::new(0.0, 0.0), Vec2::new(0.002, 0.002), &tol()).unwrap();
        near_eps(lp.perimeter(), 0.008, 1e-15);
        near_eps(lp.area(), 4e-6, 1e-18);
    }

    #[test]
    fn centroid_of_a_centred_annulus_is_its_centre() {
        let outer = Polycurve::circle(Vec2::new(1.0, 2.0), 4.0, &tol()).unwrap();
        let inner = Polycurve::circle(Vec2::new(1.0, 2.0), 1.0, &tol()).unwrap();
        let region = Region::new(outer, vec![inner], &tol()).unwrap();
        let c = region.centroid().unwrap();
        near_eps(c.x, 1.0, 1e-10);
        near_eps(c.y, 2.0, 1e-10);
    }

    #[test]
    fn centroid_of_a_half_disc_matches_the_textbook_value() {
        // Upper unit half disc: centroid at y = 4 / (3 pi).
        let arc = Arc::from_center_angles(Vec2::new(0.0, 0.0), 1.0, 0.0, PI, true).unwrap();
        let lp = Polycurve::closed(
            vec![
                Curve::Arc(arc),
                Curve::line(Vec2::new(-1.0, 0.0), Vec2::new(1.0, 0.0)),
            ],
            &tol(),
        )
        .unwrap();
        let region = Region::from_loop(lp);
        let c = region.centroid().unwrap();
        near_eps(c.x, 0.0, 1e-12);
        near_eps(c.y, 4.0 / (3.0 * PI), 1e-12);
    }

    /// Regression: a ray that grazes the seam between two arcs used to lose one
    /// crossing, because the shared vertex evaluates to `r * sin(0) = 0` from
    /// one side and `r * sin(2 pi) = -1.2e-15 * r` from the other. Points at
    /// `y = +-1e-16` next to that seam were then reported outside, which in turn
    /// made `Region::new` reject a concentric circular hole.
    #[test]
    fn ray_grazing_a_loop_seam_is_still_counted() {
        let c = Polycurve::circle(Vec2::new(0.0, 0.0), 5.0, &tol()).unwrap();
        for y in [
            0.0,
            4.898_587_196_589_413e-16,
            -4.898_587_196_589_413e-16,
            1.224_646_799_147_353e-16,
            -1.224_646_799_147_353e-16,
        ] {
            for x in [-4.9, -2.0, 0.0, 2.0, 4.9] {
                assert!(c.contains_point(Vec2::new(x, y)), "({x}, {y:e}) should be inside");
            }
            for x in [-5.1, 5.1, 12.0] {
                assert!(!c.contains_point(Vec2::new(x, y)), "({x}, {y:e}) should be outside");
            }
        }
    }

    /// Regression: the hole corners of a concentric circle sit exactly on the
    /// seam latitude, so `Region::new` exercised the case above.
    #[test]
    fn concentric_circular_hole_is_accepted_in_both_orientations() {
        let outer = Polycurve::circle(Vec2::new(0.0, 0.0), 5.0, &tol()).unwrap();
        for inner in [
            Polycurve::circle(Vec2::new(0.0, 0.0), 2.0, &tol()).unwrap(),
            Polycurve::circle(Vec2::new(0.0, 0.0), 2.0, &tol()).unwrap().reversed(),
        ] {
            let r = Region::new(outer.clone(), vec![inner], &tol()).unwrap();
            near(r.area(), PI * 21.0);
        }
    }

    /// The area formula must not depend on which curve the loop starts at: a
    /// rotated traversal is the same region and must give the same number.
    #[test]
    fn area_is_stable_under_rotating_the_loop_start() {
        let right = Arc::from_center_angles(Vec2::new(2.0, 0.0), 1.0, -PI / 2.0, PI / 2.0, true).unwrap();
        let left = Arc::from_center_angles(Vec2::new(-2.0, 0.0), 1.0, PI / 2.0, 3.0 * PI / 2.0, true).unwrap();
        let curves = vec![
            Curve::line(Vec2::new(-2.0, -1.0), Vec2::new(2.0, -1.0)),
            Curve::Arc(right),
            Curve::line(Vec2::new(2.0, 1.0), Vec2::new(-2.0, 1.0)),
            Curve::Arc(left),
        ];
        let reference = Polycurve::closed(curves.clone(), &tol()).unwrap();
        for shift in 1..curves.len() {
            let mut rotated = curves.clone();
            rotated.rotate_left(shift);
            let lp = Polycurve::closed(rotated, &tol()).unwrap();
            assert!(
                (lp.area() - reference.area()).abs() < 1e-15,
                "shift {shift}: {} vs {}",
                lp.area(),
                reference.area()
            );
            assert!((lp.perimeter() - reference.perimeter()).abs() < 1e-15);
        }
    }

    /// Repeated evaluation is bit-identical: nothing is cached, nothing depends
    /// on iteration order of a hash container, no thread pool is involved.
    #[test]
    fn metrics_are_bit_reproducible() {
        let outer = Polycurve::circle(Vec2::new(1.5, -0.25), 4.0, &tol()).unwrap();
        let hole = Polycurve::rectangle(Vec2::new(0.0, -1.0), Vec2::new(1.0, 1.0), &tol()).unwrap();
        let r = Region::new(outer, vec![hole], &tol()).unwrap();
        let first = (r.area().to_bits(), r.total_boundary_length().to_bits());
        for _ in 0..8 {
            let again = Region::new(
                Polycurve::circle(Vec2::new(1.5, -0.25), 4.0, &tol()).unwrap(),
                vec![Polycurve::rectangle(Vec2::new(0.0, -1.0), Vec2::new(1.0, 1.0), &tol()).unwrap()],
                &tol(),
            )
            .unwrap();
            assert_eq!((again.area().to_bits(), again.total_boundary_length().to_bits()), first);
        }
    }

    #[test]
    fn simplified_merges_collinear_segments_without_changing_metrics() {
        // A square whose bottom edge arrives as three collinear pieces.
        let lp = Polycurve::closed(
            vec![
                Curve::line(Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0)),
                Curve::line(Vec2::new(1.0, 0.0), Vec2::new(2.5, 0.0)),
                Curve::line(Vec2::new(2.5, 0.0), Vec2::new(4.0, 0.0)),
                Curve::line(Vec2::new(4.0, 0.0), Vec2::new(4.0, 4.0)),
                Curve::line(Vec2::new(4.0, 4.0), Vec2::new(0.0, 4.0)),
                Curve::line(Vec2::new(0.0, 4.0), Vec2::new(0.0, 0.0)),
            ],
            &tol(),
        )
        .unwrap();
        assert_eq!(lp.curve_count(), 6);
        let s = lp.simplified(&tol()).unwrap();
        assert_eq!(s.curve_count(), 4);
        near(s.area(), lp.area());
        near(s.perimeter(), lp.perimeter());
    }

    #[test]
    fn simplified_merges_co_circular_arcs_and_keeps_a_full_circle_valid() {
        // Four quarter arcs of one circle collapse to the fewest curves that
        // can still describe a closed circle (a single curve cannot be a loop
        // unless it is the full circle, which `merge_curves` refuses to build).
        let c = Vec2::new(1.0, -2.0);
        let q = |k: i32| {
            Curve::Arc(
                Arc::new(c, 3.0, k as f64 * PI / 2.0, PI / 2.0).unwrap(),
            )
        };
        let lp = Polycurve::closed(vec![q(0), q(1), q(2), q(3)], &tol()).unwrap();
        near(lp.area(), PI * 9.0);
        let s = lp.simplified(&tol()).unwrap();
        assert!(s.curve_count() <= 2, "{} curves", s.curve_count());
        near(s.area(), PI * 9.0);
        near(s.perimeter(), TAU * 3.0);
    }

    #[test]
    fn simplified_keeps_a_real_corner_and_a_direction_reversal() {
        let square = Polycurve::rectangle(Vec2::new(0.0, 0.0), Vec2::new(2.0, 2.0), &tol()).unwrap();
        assert_eq!(square.simplified(&tol()).unwrap().curve_count(), 4);
        // Two collinear but opposite segments must not merge into nothing.
        let spike = Polycurve::closed(
            vec![
                Curve::line(Vec2::new(0.0, 0.0), Vec2::new(2.0, 0.0)),
                Curve::line(Vec2::new(2.0, 0.0), Vec2::new(2.0, 1.0)),
                Curve::line(Vec2::new(2.0, 1.0), Vec2::new(0.0, 1.0)),
                Curve::line(Vec2::new(0.0, 1.0), Vec2::new(0.0, 0.0)),
            ],
            &tol(),
        )
        .unwrap();
        assert_eq!(spike.simplified(&tol()).unwrap().curve_count(), 4);
    }

    #[test]
    fn point_containment_handles_a_curved_boundary() {
        // Stadium again: points just inside and just outside the right cap.
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
        assert!(lp.contains_point(Vec2::new(2.9, 0.0)));
        assert!(!lp.contains_point(Vec2::new(3.1, 0.0)));
        assert!(lp.contains_point(Vec2::new(0.0, 0.9)));
        assert!(!lp.contains_point(Vec2::new(0.0, 1.1)));
        // Corner of the bounding box: inside the box, outside the stadium.
        assert!(!lp.contains_point(Vec2::new(2.95, 0.95)));
    }
}
