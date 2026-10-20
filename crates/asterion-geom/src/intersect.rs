//! Curve intersections, tolerance-aware.
//!
//! All results are **analytic**. Every operation takes the modelling `Tolerances` so that the decisions it makes (parallel?
//! tangent? inside the parameter range?) use the same numbers the rest of ASTERION uses — the observed reference-compatible
//! values by default.
//!
//! Parameter-range slack: a hit is accepted when it lies within
//! `Tolerances::vertex` **in metres along the curve** of the valid range, which
//! for an arc means an angular slack of `vertex / radius`. That keeps the test
//! scale-independent: a 10 m radius arc does not get a 10x looser end test.

use crate::curve::{normalize_angle_positive, Arc, Curve, LineSegment};
use crate::tolerance::Tolerances;
use crate::vec::Vec2;

/// Result of intersecting two straight segments.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SegmentHit {
    /// No intersection (disjoint, or parallel and not collinear).
    None,
    /// Exactly one crossing point.
    Point(Vec2),
    /// Collinear and overlapping; the shared piece is returned.
    Overlap(LineSegment),
}

/// Are two directions parallel within the angle tolerance?
pub fn directions_parallel(d0: Vec2, d1: Vec2, tol: &Tolerances) -> bool {
    let l0 = d0.length();
    let l1 = d1.length();
    if l0 == 0.0 || l1 == 0.0 {
        return true;
    }
    (d0.cross(d1) / (l0 * l1)).abs() <= tol.angle.sin()
}

/// Intersection of the two **infinite lines** through the given segments.
/// `None` when they are parallel within the angle tolerance.
pub fn intersect_lines_unbounded(a: &LineSegment, b: &LineSegment, tol: &Tolerances) -> Option<Vec2> {
    let d0 = a.direction();
    let d1 = b.direction();
    if directions_parallel(d0, d1, tol) {
        return None;
    }
    let t = (b.start - a.start).cross(d1) / d0.cross(d1);
    Some(a.start + d0 * t)
}

/// Intersection of two **bounded** segments.
pub fn intersect_segments(a: &LineSegment, b: &LineSegment, tol: &Tolerances) -> SegmentHit {
    let d0 = a.direction();
    let d1 = b.direction();
    let (l0, l1) = (d0.length(), d1.length());
    if l0 == 0.0 || l1 == 0.0 {
        return SegmentHit::None;
    }
    let denom = d0.cross(d1);
    if directions_parallel(d0, d1, tol) {
        // Collinear? Distance from b.start to the line of a.
        let offset = (b.start - a.start).cross(d0) / l0;
        if offset.abs() > tol.vertex {
            return SegmentHit::None;
        }
        let s0: f64 = 0.0;
        let s1: f64 = 1.0;
        let u0 = a.closest_parameter_unbounded(b.start).unwrap();
        let u1 = a.closest_parameter_unbounded(b.end).unwrap();
        let (lo_b, hi_b) = if u0 <= u1 { (u0, u1) } else { (u1, u0) };
        let lo = s0.max(lo_b);
        let hi = s1.min(hi_b);
        let slack = tol.vertex / l0;
        if hi < lo - slack {
            return SegmentHit::None;
        }
        if (hi - lo).abs() <= slack {
            return SegmentHit::Point(a.point_at((lo + hi) / 2.0));
        }
        return SegmentHit::Overlap(LineSegment::new(a.point_at(lo), a.point_at(hi)));
    }
    let diff = b.start - a.start;
    let s = diff.cross(d1) / denom;
    let t = diff.cross(d0) / denom;
    let slack_a = tol.vertex / l0;
    let slack_b = tol.vertex / l1;
    if s >= -slack_a && s <= 1.0 + slack_a && t >= -slack_b && t <= 1.0 + slack_b {
        SegmentHit::Point(a.point_at(s.clamp(0.0, 1.0)))
    } else {
        SegmentHit::None
    }
}

/// Intersections of the **infinite line** through `line` with the full circle
/// `(centre, radius)`, ignoring any arc sweep. Ordered by the parameter along
/// the line. A tangency within `Tolerances::vertex` returns a single point.
pub fn intersect_line_circle(
    line: &LineSegment,
    centre: Vec2,
    radius: f64,
    tol: &Tolerances,
) -> Vec<Vec2> {
    let d = line.direction();
    let len = d.length();
    if len == 0.0 || !(radius > 0.0) {
        return Vec::new();
    }
    let u = Vec2::new(d.x / len, d.y / len);
    // Foot of the perpendicular from the centre onto the line.
    let s_foot = (centre - line.start).dot(u);
    let foot = line.start + u * s_foot;
    let h = foot.distance_to(centre);
    let mut hits: Vec<Vec2> = Vec::new();
    if h > radius + tol.vertex {
        return hits;
    }
    if h >= radius - tol.vertex {
        // Tangent within tolerance: project the foot onto the circle so the
        // returned point is exactly on it.
        hits.push(project_point_to_circle(foot, centre, radius));
    } else {
        let half = (radius * radius - h * h).max(0.0).sqrt();
        hits.push(foot - u * half);
        hits.push(foot + u * half);
    }
    hits
}

/// Intersections of two full circles, ignoring any arc sweep.
pub fn intersect_circles(
    c0: Vec2,
    r0: f64,
    c1: Vec2,
    r1: f64,
    tol: &Tolerances,
) -> Vec<Vec2> {
    let delta = c1 - c0;
    let d = delta.length();
    let mut hits: Vec<Vec2> = Vec::new();
    if d <= tol.vertex || d > r0 + r1 + tol.vertex || d < (r0 - r1).abs() - tol.vertex {
        return hits;
    }
    let u = Vec2::new(delta.x / d, delta.y / d);
    let x = (d * d + r0 * r0 - r1 * r1) / (2.0 * d);
    let hsq = r0 * r0 - x * x;
    let base = c0 + u * x;
    if hsq <= 0.0 {
        hits.push(project_point_to_circle(base, c0, r0));
    } else {
        let h = hsq.sqrt();
        let p = u.perp_ccw();
        hits.push(base + p * h);
        hits.push(base - p * h);
    }
    hits
}

/// Intersections of the **infinite line** through `line` with the (bounded) arc.
/// At most two points, ordered by the parameter along the line.
pub fn intersect_line_arc_unbounded(line: &LineSegment, arc: &Arc, tol: &Tolerances) -> Vec<Vec2> {
    let mut hits = intersect_line_circle(line, arc.centre, arc.radius, tol);
    hits.retain(|p| arc_contains_point(arc, *p, tol));
    hits
}

/// Intersections of the two curves' **supporting geometry** (infinite lines,
/// full circles), ignoring both parameter ranges.
///
/// This is what offsetting needs: at a convex corner the two offset curves no
/// longer touch, so the mitre point lies beyond both of their ends.
pub fn intersect_curves_unbounded(a: &Curve, b: &Curve, tol: &Tolerances) -> Vec<Vec2> {
    match (a, b) {
        (Curve::Line(l0), Curve::Line(l1)) => {
            intersect_lines_unbounded(l0, l1, tol).into_iter().collect()
        }
        (Curve::Line(l), Curve::Arc(c)) | (Curve::Arc(c), Curve::Line(l)) => {
            intersect_line_circle(l, c.centre, c.radius, tol)
        }
        (Curve::Arc(c0), Curve::Arc(c1)) => {
            intersect_circles(c0.centre, c0.radius, c1.centre, c1.radius, tol)
        }
    }
}

/// Intersections of a **bounded** segment with a **bounded** arc.
pub fn intersect_segment_arc(line: &LineSegment, arc: &Arc, tol: &Tolerances) -> Vec<Vec2> {
    let len = line.length();
    if len == 0.0 {
        return Vec::new();
    }
    let slack = tol.vertex / len;
    intersect_line_arc_unbounded(line, arc, tol)
        .into_iter()
        .filter(|p| {
            let s = line.closest_parameter_unbounded(*p).unwrap_or(-1.0);
            s >= -slack && s <= 1.0 + slack
        })
        .collect()
}

/// Intersections of two bounded arcs (circle-circle, then angle filtering).
///
/// **Coincident arcs** (same centre and radius within tolerance) are the arc
/// analogue of a collinear segment overlap: there is no isolated intersection,
/// so the *endpoints of the shared angular span* are returned instead. Boolean
/// operations need exactly that, to be able to split both boundaries at the
/// points where the overlap starts and stops.
pub fn intersect_arcs(a: &Arc, b: &Arc, tol: &Tolerances) -> Vec<Vec2> {
    if a.centre.distance_to(b.centre) <= tol.vertex && (a.radius - b.radius).abs() <= tol.vertex {
        return coincident_arc_overlap_points(a, b, tol);
    }
    let mut hits = intersect_circles(a.centre, a.radius, b.centre, b.radius, tol);
    hits.retain(|q| arc_contains_point(a, *q, tol) && arc_contains_point(b, *q, tol));
    hits
}

/// Endpoints of the angular span shared by two arcs on the same circle.
///
/// The boundary angles of an overlap are always among the four arc endpoints,
/// so the four are tested for membership in both arcs and the survivors are
/// returned (de-duplicated).
pub fn coincident_arc_overlap_points(a: &Arc, b: &Arc, tol: &Tolerances) -> Vec<Vec2> {
    let slack = (tol.vertex / a.radius).max(f64::EPSILON * 8.0);
    let candidates = [a.start_angle, a.end_angle(), b.start_angle, b.end_angle()];
    let mut out: Vec<Vec2> = Vec::new();
    for angle in candidates {
        if a.contains_angle(angle, slack) && b.contains_angle(angle, slack) {
            let p = a.point_at_angle(angle);
            if !out.iter().any(|q| q.distance_to(p) <= tol.vertex) {
                out.push(p);
            }
        }
    }
    out
}

/// Dispatching intersection for the `Curve` enum.
pub fn intersect_curves(a: &Curve, b: &Curve, tol: &Tolerances) -> Vec<Vec2> {
    match (a, b) {
        (Curve::Line(l0), Curve::Line(l1)) => match intersect_segments(l0, l1, tol) {
            SegmentHit::None => Vec::new(),
            SegmentHit::Point(p) => vec![p],
            SegmentHit::Overlap(s) => vec![s.start, s.end],
        },
        (Curve::Line(l), Curve::Arc(c)) => intersect_segment_arc(l, c, tol),
        (Curve::Arc(c), Curve::Line(l)) => intersect_segment_arc(l, c, tol),
        (Curve::Arc(c0), Curve::Arc(c1)) => intersect_arcs(c0, c1, tol),
    }
}

/// Is `p` on the arc (both on its circle and inside its sweep)?
pub fn arc_contains_point(arc: &Arc, p: Vec2, tol: &Tolerances) -> bool {
    if (p.distance_to(arc.centre) - arc.radius).abs() > tol.vertex.max(1e-9) {
        return false;
    }
    let angle_slack = (tol.vertex / arc.radius).max(f64::EPSILON * 8.0);
    arc.contains_angle((p - arc.centre).angle(), angle_slack)
}

/// Signed radial deviation of `p` from the arc's circle, and the travelled
/// angle from the arc start. Useful for projections and snapping.
pub fn arc_local_coords(arc: &Arc, p: Vec2) -> (f64, f64) {
    let r = p.distance_to(arc.centre);
    let travelled = if arc.is_counter_clockwise() {
        normalize_angle_positive((p - arc.centre).angle() - arc.start_angle)
    } else {
        normalize_angle_positive(arc.start_angle - (p - arc.centre).angle())
    };
    (r - arc.radius, travelled)
}

fn project_point_to_circle(p: Vec2, centre: Vec2, radius: f64) -> Vec2 {
    match (p - centre).normalized() {
        Some(u) => centre + u * radius,
        None => centre + Vec2::new(radius, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn tol() -> Tolerances {
        Tolerances::default()
    }
    fn near_pt(a: Vec2, b: Vec2) {
        assert!(a.distance_to(b) < 1e-9, "{a:?} != {b:?}");
    }

    #[test]
    fn crossing_segments_meet_at_the_expected_point() {
        let a = LineSegment::new(Vec2::new(-1.0, 0.0), Vec2::new(1.0, 0.0));
        let b = LineSegment::new(Vec2::new(0.0, -1.0), Vec2::new(0.0, 2.0));
        match intersect_segments(&a, &b, &tol()) {
            SegmentHit::Point(p) => near_pt(p, Vec2::new(0.0, 0.0)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parallel_segments_do_not_intersect_but_unbounded_lines_can() {
        let a = LineSegment::new(Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0));
        let b = LineSegment::new(Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0));
        assert_eq!(intersect_segments(&a, &b, &tol()), SegmentHit::None);
        assert!(intersect_lines_unbounded(&a, &b, &tol()).is_none());
        // Non-parallel but disjoint segments: the lines still cross.
        let c = LineSegment::new(Vec2::new(5.0, 1.0), Vec2::new(5.0, 2.0));
        assert_eq!(intersect_segments(&a, &c, &tol()), SegmentHit::None);
        near_pt(
            intersect_lines_unbounded(&a, &c, &tol()).unwrap(),
            Vec2::new(5.0, 0.0),
        );
    }

    #[test]
    fn collinear_overlap_is_reported_as_overlap() {
        let a = LineSegment::new(Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0));
        let b = LineSegment::new(Vec2::new(3.0, 0.0), Vec2::new(7.0, 0.0));
        match intersect_segments(&a, &b, &tol()) {
            SegmentHit::Overlap(s) => {
                near_pt(s.start, Vec2::new(3.0, 0.0));
                near_pt(s.end, Vec2::new(4.0, 0.0));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn horizontal_line_cuts_a_semicircle_twice() {
        // Unit upper semicircle; the line y = 0.6 cuts it at x = +-0.8.
        let arc = Arc::from_center_angles(Vec2::new(0.0, 0.0), 1.0, 0.0, PI, true).unwrap();
        let line = LineSegment::new(Vec2::new(-2.0, 0.6), Vec2::new(2.0, 0.6));
        let hits = intersect_segment_arc(&line, &arc, &tol());
        assert_eq!(hits.len(), 2, "{hits:?}");
        near_pt(hits[0], Vec2::new(-0.8, 0.6));
        near_pt(hits[1], Vec2::new(0.8, 0.6));
    }

    #[test]
    fn sweep_filtering_drops_the_hit_outside_the_arc() {
        // Lower semicircle only: y = -0.6 hits it, y = +0.6 does not.
        let arc = Arc::from_center_angles(Vec2::new(0.0, 0.0), 1.0, PI, 0.0, true).unwrap();
        let below = LineSegment::new(Vec2::new(-2.0, -0.6), Vec2::new(2.0, -0.6));
        let above = LineSegment::new(Vec2::new(-2.0, 0.6), Vec2::new(2.0, 0.6));
        assert_eq!(intersect_segment_arc(&below, &arc, &tol()).len(), 2);
        assert_eq!(intersect_segment_arc(&above, &arc, &tol()).len(), 0);
    }

    #[test]
    fn tangent_line_gives_exactly_one_point() {
        let arc = Arc::from_center_angles(Vec2::new(0.0, 0.0), 2.0, 0.0, PI, true).unwrap();
        let line = LineSegment::new(Vec2::new(-1.0, 2.0), Vec2::new(1.0, 2.0));
        let hits = intersect_segment_arc(&line, &arc, &tol());
        assert_eq!(hits.len(), 1, "{hits:?}");
        near_pt(hits[0], Vec2::new(0.0, 2.0));
    }

    #[test]
    fn bounded_segment_shorter_than_the_chord_misses() {
        let arc = Arc::from_center_angles(Vec2::new(0.0, 0.0), 1.0, 0.0, PI, true).unwrap();
        let line = LineSegment::new(Vec2::new(-0.2, 0.6), Vec2::new(0.2, 0.6));
        assert_eq!(intersect_segment_arc(&line, &arc, &tol()).len(), 0);
        assert_eq!(intersect_line_arc_unbounded(&line, &arc, &tol()).len(), 2);
    }

    #[test]
    fn two_unit_circles_at_distance_one_meet_at_the_classic_points() {
        let a = Arc::from_center_angles(Vec2::new(0.0, 0.0), 1.0, 0.0, 0.0, true).unwrap();
        let b = Arc::from_center_angles(Vec2::new(1.0, 0.0), 1.0, 0.0, 0.0, true).unwrap();
        let hits = intersect_arcs(&a, &b, &tol());
        assert_eq!(hits.len(), 2, "{hits:?}");
        let y = (3.0_f64).sqrt() / 2.0;
        let mut ys: Vec<f64> = hits.iter().map(|p| p.y).collect();
        ys.sort_by(|p, q| p.total_cmp(q));
        assert!((ys[0] + y).abs() < 1e-12 && (ys[1] - y).abs() < 1e-12);
        for h in &hits {
            assert!((h.x - 0.5).abs() < 1e-12);
        }
    }

    #[test]
    fn grid_arc_meets_grid_line_where_expected() {
        // An arc grid line of radius 10 centred at the origin, from 0 to 90 deg,
        // crossed by the vertical line x = 6 => y = 8.
        let arc = Arc::from_center_angles(Vec2::new(0.0, 0.0), 10.0, 0.0, PI / 2.0, true).unwrap();
        let line = LineSegment::new(Vec2::new(6.0, -20.0), Vec2::new(6.0, 20.0));
        let hits = intersect_segment_arc(&line, &arc, &tol());
        assert_eq!(hits.len(), 1, "{hits:?}");
        near_pt(hits[0], Vec2::new(6.0, 8.0));
    }

    #[test]
    fn dispatch_matches_the_specific_routines() {
        let arc = Arc::from_center_angles(Vec2::new(0.0, 0.0), 1.0, 0.0, PI, true).unwrap();
        let line = LineSegment::new(Vec2::new(-2.0, 0.6), Vec2::new(2.0, 0.6));
        let direct = intersect_segment_arc(&line, &arc, &tol());
        let via = intersect_curves(&Curve::Arc(arc), &Curve::Line(line), &tol());
        assert_eq!(direct.len(), via.len());
    }
}
