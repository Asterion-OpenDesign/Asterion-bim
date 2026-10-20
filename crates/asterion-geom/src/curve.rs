//! Planar curves: line segments and circular arcs.
//!
//! Scope and conventions:
//!
//! * Everything is **analytic**: lengths, areas, bounding boxes and intersections are closed-form, never tessellated. A
//!   floor whose boundary contains an arc must still match the reference's `Area` / `Perimeter` to 1e-4, which rules out
//!   polygonal approximation.
//! * Parameter `t` of every curve is normalized to `[0, 1]`; `point_at(0)` is
//!   the start and `point_at(1)` the end. This keeps a `Polycurve` traversal
//!   independent of the curve kind.
//! * An arc is `(centre, radius, start_angle, sweep)` with **signed sweep**:
//!   positive = counter-clockwise. Direction is therefore part of the data, not
//!   a separate flag, and `reversed()` is exact.
//! * Normals follow `Vec2::perp_ccw` (left of the tangent), so a positive
//!   offset moves a curve to its left — the same convention the wall location
//!   line uses in `asterion-bim`.

use crate::error::{GeomError, GeomResult};
use crate::vec::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

/// Axis-aligned bounding rectangle in plan.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Aabb2 {
    pub min: Vec2,
    pub max: Vec2,
}

impl Aabb2 {
    pub fn from_point(p: Vec2) -> Self {
        Aabb2 { min: p, max: p }
    }
    pub fn from_points(points: &[Vec2]) -> Option<Self> {
        let mut it = points.iter();
        let first = *it.next()?;
        let mut b = Aabb2::from_point(first);
        for p in it {
            b = b.with_point(*p);
        }
        Some(b)
    }
    pub fn with_point(self, p: Vec2) -> Self {
        Aabb2 {
            min: Vec2::new(self.min.x.min(p.x), self.min.y.min(p.y)),
            max: Vec2::new(self.max.x.max(p.x), self.max.y.max(p.y)),
        }
    }
    pub fn union(self, o: Aabb2) -> Self {
        self.with_point(o.min).with_point(o.max)
    }
    pub fn width(self) -> f64 {
        self.max.x - self.min.x
    }
    pub fn height(self) -> f64 {
        self.max.y - self.min.y
    }
    pub fn centre(self) -> Vec2 {
        self.min.lerp(self.max, 0.5)
    }
    pub fn contains_point(self, p: Vec2, tolerance: f64) -> bool {
        p.x >= self.min.x - tolerance
            && p.x <= self.max.x + tolerance
            && p.y >= self.min.y - tolerance
            && p.y <= self.max.y + tolerance
    }
    /// Lift to 3D between `z0` and `z1`.
    pub fn extruded(self, z0: f64, z1: f64) -> (Vec3, Vec3) {
        (Vec3::from_xy(self.min, z0), Vec3::from_xy(self.max, z1))
    }
}

/// Normalize an angle to `[0, 2*pi)`.
pub fn normalize_angle_positive(a: f64) -> f64 {
    let m = a % TAU;
    if m < 0.0 {
        m + TAU
    } else {
        m
    }
}

/// Normalize an angle to `(-pi, pi]`.
pub fn normalize_angle_signed(a: f64) -> f64 {
    let m = normalize_angle_positive(a);
    if m > PI {
        m - TAU
    } else {
        m
    }
}

/// A straight segment between two points.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineSegment {
    pub start: Vec2,
    pub end: Vec2,
}

impl LineSegment {
    pub const fn new(start: Vec2, end: Vec2) -> Self {
        LineSegment { start, end }
    }
    pub fn length(&self) -> f64 {
        self.start.distance_to(self.end)
    }
    pub fn direction(&self) -> Vec2 {
        self.end - self.start
    }
    pub fn point_at(&self, t: f64) -> Vec2 {
        self.start.lerp(self.end, t)
    }
    /// Unit tangent; `None` only for a degenerate (zero-length) segment.
    pub fn tangent(&self) -> Option<Vec2> {
        self.direction().normalized()
    }
    pub fn aabb(&self) -> Aabb2 {
        Aabb2::from_point(self.start).with_point(self.end)
    }
    pub fn reversed(&self) -> Self {
        LineSegment::new(self.end, self.start)
    }
    /// Parallel copy `distance` to the **left** of the travel direction.
    pub fn offset(&self, distance: f64) -> GeomResult<Self> {
        let n = self
            .tangent()
            .ok_or(GeomError::CurveTooShort { length: 0.0, tolerance: 0.0 })?
            .perp_ccw();
        let d = n * distance;
        Ok(LineSegment::new(self.start + d, self.end + d))
    }
    /// Signed area term of Green's theorem, `1/2 * integral(x dy - y dx)`.
    /// For a segment this is the shoelace term.
    pub fn signed_area_term(&self) -> f64 {
        0.5 * self.start.cross(self.end)
    }
    /// Same line, new start point (trim or extend).
    pub fn with_start(&self, p: Vec2) -> Self {
        LineSegment::new(p, self.end)
    }
    /// Same line, new end point (trim or extend).
    pub fn with_end(&self, p: Vec2) -> Self {
        LineSegment::new(self.start, p)
    }
    /// Closest point on the segment and its parameter, clamped to `[0, 1]`.
    pub fn closest_point(&self, p: Vec2) -> (Vec2, f64) {
        let t = self.closest_parameter_unbounded(p).unwrap_or(0.0).clamp(0.0, 1.0);
        (self.point_at(t), t)
    }
    /// Closest parameter on the *infinite* line through the segment.
    pub fn closest_parameter_unbounded(&self, p: Vec2) -> Option<f64> {
        let d = self.direction();
        let dd = d.dot(d);
        (dd > 0.0).then(|| (p - self.start).dot(d) / dd)
    }
}

/// A circular arc: centre, radius, start angle and **signed** sweep
/// (counter-clockwise positive). `|sweep|` may be up to `2*pi` (full circle).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Arc {
    pub centre: Vec2,
    pub radius: f64,
    pub start_angle: f64,
    pub sweep: f64,
}

impl Arc {
    /// Raw constructor; validates radius and sweep.
    pub fn new(centre: Vec2, radius: f64, start_angle: f64, sweep: f64) -> GeomResult<Self> {
        if !(radius > 0.0) || sweep == 0.0 || !sweep.is_finite() || !radius.is_finite() {
            return Err(GeomError::DegenerateArc { radius, sweep });
        }
        Ok(Arc { centre, radius, start_angle, sweep })
    }

    /// Arc from centre, radius and the two angles, with an explicit direction.
    /// The sweep is the positive travel from `start_angle` to `end_angle` in
    /// the requested direction; equal angles mean a full circle.
    pub fn from_center_angles(
        centre: Vec2,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
        counter_clockwise: bool,
    ) -> GeomResult<Self> {
        let mut delta = normalize_angle_positive(if counter_clockwise {
            end_angle - start_angle
        } else {
            start_angle - end_angle
        });
        if delta == 0.0 {
            delta = TAU;
        }
        let sweep = if counter_clockwise { delta } else { -delta };
        Arc::new(centre, radius, start_angle, sweep)
    }

    /// Arc through three points (`start`, an intermediate point on the arc, and
    /// `end`) — the sketch workhorse. Direction follows the orientation of the
    /// three points.
    pub fn from_three_points(start: Vec2, through: Vec2, end: Vec2, tolerance: f64) -> GeomResult<Self> {
        let a = start;
        let b = through;
        let c = end;
        let area2 = (b - a).cross(c - a);
        // Collinearity is scale-aware: compare the triangle height against the
        // vertex tolerance, using the longest side as the base.
        let base = (b - a).length().max((c - b).length()).max((c - a).length());
        if base <= 0.0 || (area2 / base).abs() <= tolerance {
            return Err(GeomError::PointsCollinear { area2 });
        }
        // Circumcentre.
        let d = 2.0 * area2;
        let ua = a.dot(a);
        let ub = b.dot(b);
        let uc = c.dot(c);
        let cx = (ua * (b.y - c.y) + ub * (c.y - a.y) + uc * (a.y - b.y)) / d;
        let cy = (ua * (c.x - b.x) + ub * (a.x - c.x) + uc * (b.x - a.x)) / d;
        let centre = Vec2::new(cx, cy);
        let radius = centre.distance_to(a);
        let ccw = area2 > 0.0;
        let start_angle = (a - centre).angle();
        let end_angle = (c - centre).angle();
        Arc::from_center_angles(centre, radius, start_angle, end_angle, ccw)
    }

    /// Arc from its two endpoints and a *bulge* (DXF convention):
    /// `bulge = tan(sweep / 4)`, positive = counter-clockwise.
    pub fn from_endpoints_bulge(start: Vec2, end: Vec2, bulge: f64) -> GeomResult<Self> {
        if bulge == 0.0 {
            return Err(GeomError::DegenerateArc { radius: f64::INFINITY, sweep: 0.0 });
        }
        let chord = end - start;
        let chord_len = chord.length();
        if chord_len <= 0.0 {
            return Err(GeomError::DegenerateArc { radius: 0.0, sweep: 0.0 });
        }
        let sweep = 4.0 * bulge.atan();
        let radius = (chord_len / 2.0) / (sweep / 2.0).sin().abs();
        // Centre lies on the chord's perpendicular bisector.
        let mid = start.lerp(end, 0.5);
        let h = (radius * radius - chord_len * chord_len / 4.0).max(0.0).sqrt();
        // For a CCW arc the centre is to the left of the chord when
        // |sweep| < pi, and to the right when |sweep| > pi.
        let left = chord.normalized().unwrap().perp_ccw();
        let side = if sweep > 0.0 { 1.0 } else { -1.0 };
        let flip = if sweep.abs() > PI { -1.0 } else { 1.0 };
        let centre = mid + left * (side * flip * h);
        let start_angle = (start - centre).angle();
        Arc::new(centre, radius, start_angle, sweep)
    }

    pub fn end_angle(&self) -> f64 {
        self.start_angle + self.sweep
    }
    pub fn is_counter_clockwise(&self) -> bool {
        self.sweep > 0.0
    }
    pub fn is_full_circle(&self) -> bool {
        (self.sweep.abs() - TAU).abs() <= f64::EPSILON * 8.0
    }
    pub fn start(&self) -> Vec2 {
        self.point_at_angle(self.start_angle)
    }
    pub fn end(&self) -> Vec2 {
        self.point_at_angle(self.end_angle())
    }
    /// The clockwise arc runs the opposite way, the reference product's arc of normal -Z. With the normalised parameter the
    /// derivative is multiplied by the parameter span (`|sweep|`).
    pub fn raw_derivative_at_angle(&self, angle: f64) -> Vec2 {
        let d = Vec2::new(-angle.sin(), angle.cos()) * self.radius;
        if self.is_counter_clockwise() {
            d
        } else {
            -d
        }
    }

    pub fn point_at_angle(&self, angle: f64) -> Vec2 {
        self.centre + Vec2::from_angle(angle) * self.radius
    }
    /// Exact arc length `r * |sweep|`.
    pub fn length(&self) -> f64 {
        self.radius * self.sweep.abs()
    }
    pub fn angle_at(&self, t: f64) -> f64 {
        self.start_angle + self.sweep * t
    }
    pub fn point_at(&self, t: f64) -> Vec2 {
        self.point_at_angle(self.angle_at(t))
    }
    /// Unit tangent in the travel direction.
    pub fn tangent_at(&self, t: f64) -> Vec2 {
        let a = self.angle_at(t);
        let radial = Vec2::from_angle(a);
        if self.is_counter_clockwise() {
            radial.perp_ccw()
        } else {
            -radial.perp_ccw()
        }
    }
    /// Chord of the whole arc.
    pub fn chord(&self) -> LineSegment {
        LineSegment::new(self.start(), self.end())
    }
    /// Distance from the chord's midpoint to the arc (`sagitta`).
    pub fn sagitta(&self) -> f64 {
        self.radius * (1.0 - (self.sweep / 2.0).cos().abs())
    }
    /// DXF-style bulge value, `tan(sweep / 4)`.
    pub fn bulge(&self) -> f64 {
        (self.sweep / 4.0).tan()
    }
    pub fn reversed(&self) -> Self {
        Arc {
            centre: self.centre,
            radius: self.radius,
            start_angle: self.end_angle(),
            sweep: -self.sweep,
        }
    }
    /// Is `angle` inside the swept range (tolerance in radians)?
    pub fn contains_angle(&self, angle: f64, angle_tolerance: f64) -> bool {
        if self.is_full_circle() {
            return true;
        }
        let travelled = if self.is_counter_clockwise() {
            normalize_angle_positive(angle - self.start_angle)
        } else {
            normalize_angle_positive(self.start_angle - angle)
        };
        let span = self.sweep.abs();
        travelled <= span + angle_tolerance || travelled >= TAU - angle_tolerance
    }
    /// Exact bounding box: the endpoints plus every axis extreme inside the sweep.
    pub fn aabb(&self) -> Aabb2 {
        let mut b = Aabb2::from_point(self.start()).with_point(self.end());
        for k in 0..4 {
            let a = k as f64 * PI / 2.0;
            if self.contains_angle(a, 0.0) {
                b = b.with_point(self.point_at_angle(a));
            }
        }
        b
    }
    /// Closest point on the arc and its parameter in `[0, 1]`. Exact: the
    /// radial projection when its angle is inside the sweep, otherwise the
    /// nearer endpoint.
    pub fn closest_point(&self, p: Vec2) -> (Vec2, f64) {
        let d = p - self.centre;
        if d.length() > 0.0 {
            let angle = d.angle();
            if self.contains_angle(angle, 0.0) {
                let travelled = if self.is_counter_clockwise() {
                    normalize_angle_positive(angle - self.start_angle)
                } else {
                    normalize_angle_positive(self.start_angle - angle)
                };
                let t = (travelled / self.sweep.abs()).clamp(0.0, 1.0);
                return (self.point_at_angle(angle), t);
            }
        }
        let (s, e) = (self.start(), self.end());
        if p.distance_to(s) <= p.distance_to(e) {
            (s, 0.0)
        } else {
            (e, 1.0)
        }
    }
    /// Parameter in `[0, 1]` of a point assumed to lie on the arc.
    pub fn parameter_at_point(&self, p: Vec2) -> f64 {
        self.closest_point(p).1
    }
    /// Point of the arc that is farthest along `direction` (support function).
    /// Exact: either the radial extreme, when that angle is inside the sweep,
    /// or one of the endpoints.
    pub fn support_point(&self, direction: Vec2) -> Vec2 {
        let mut best = self.start();
        let mut score = best.dot(direction);
        let candidate_end = self.end();
        if candidate_end.dot(direction) > score {
            best = candidate_end;
            score = candidate_end.dot(direction);
        }
        if let Some(u) = direction.normalized() {
            let radial = self.centre + u * self.radius;
            if self.contains_angle(u.angle(), 0.0) && radial.dot(direction) > score {
                best = radial;
            }
        }
        best
    }
    /// Concentric copy `distance` to the left of the travel direction. For a
    /// counter-clockwise arc the left side is the centre, so the radius shrinks.
    pub fn offset(&self, distance: f64) -> GeomResult<Self> {
        let sign = if self.is_counter_clockwise() { -1.0 } else { 1.0 };
        let radius = self.radius + sign * distance;
        if !(radius > 0.0) {
            return Err(GeomError::OffsetCollapsesCurve { radius: self.radius, distance });
        }
        Arc::new(self.centre, radius, self.start_angle, self.sweep)
    }
    /// Signed area term of Green's theorem for the arc,
    /// `1/2 * integral(x dy - y dx)`.
    ///
    /// Closed form, derived once: with `P(theta) = c + r * (cos, sin)`,
    /// `x dy - y dx = (r^2 + cx * r * cos + cy * r * sin) dtheta`, so the term
    /// is `1/2 * (r^2 * sweep + cx * (y1 - y0) - cy * (x1 - x0))`.
    /// Equivalently: the chord's shoelace term plus the circular segment
    /// `1/2 * r^2 * (sweep - sin sweep)`.
    pub fn signed_area_term(&self) -> f64 {
        let p0 = self.start();
        let p1 = self.end();
        0.5 * (self.radius * self.radius * self.sweep
            + self.centre.x * (p1.y - p0.y)
            - self.centre.y * (p1.x - p0.x))
    }
    /// Signed area between the arc and its chord (positive for a CCW arc).
    pub fn circular_segment_area(&self) -> f64 {
        0.5 * self.radius * self.radius * (self.sweep - self.sweep.sin())
    }
    /// Same circle and direction, new start point (trim or extend). `p` is
    /// projected radially onto the circle; the end angle is preserved.
    pub fn with_start(&self, p: Vec2) -> GeomResult<Self> {
        let end = self.end_angle();
        let start = (p - self.centre).angle();
        let sweep = closest_sweep(end - start, self.sweep);
        Arc::new(self.centre, self.radius, start, sweep)
    }
    /// Same circle and direction, new end point (trim or extend). `p` is
    /// projected radially onto the circle; the start angle is preserved.
    pub fn with_end(&self, p: Vec2) -> GeomResult<Self> {
        let end = (p - self.centre).angle();
        let sweep = closest_sweep(end - self.start_angle, self.sweep);
        Arc::new(self.centre, self.radius, self.start_angle, sweep)
    }
}

/// Pick the sweep congruent to `delta` (mod `2*pi`) that keeps the original
/// direction and stays closest to `reference`. Used by `Arc::with_start` /
/// `with_end` so that trimming or extending an arc never silently flips its
/// direction or jumps a full turn.
fn closest_sweep(delta: f64, reference: f64) -> f64 {
    let positive = normalize_angle_positive(delta);
    let candidates = if reference > 0.0 {
        [positive, positive + TAU]
    } else {
        [positive - TAU, positive - TAU - TAU]
    };
    let mut best = candidates[0];
    for c in candidates {
        if (c - reference).abs() < (best - reference).abs() {
            best = c;
        }
    }
    best
}

/// A planar curve: the two kinds ASTERION models analytically today.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Curve {
    Line(LineSegment),
    Arc(Arc),
}

impl Curve {
    pub fn line(start: Vec2, end: Vec2) -> Self {
        Curve::Line(LineSegment::new(start, end))
    }
    pub fn arc(arc: Arc) -> Self {
        Curve::Arc(arc)
    }
    pub fn start(&self) -> Vec2 {
        match self {
            Curve::Line(l) => l.start,
            Curve::Arc(a) => a.start(),
        }
    }
    pub fn end(&self) -> Vec2 {
        match self {
            Curve::Line(l) => l.end,
            Curve::Arc(a) => a.end(),
        }
    }
    /// Exact length (chord for a segment, `r * |sweep|` for an arc).
    pub fn length(&self) -> f64 {
        match self {
            Curve::Line(l) => l.length(),
            Curve::Arc(a) => a.length(),
        }
    }
    pub fn point_at(&self, t: f64) -> Vec2 {
        match self {
            Curve::Line(l) => l.point_at(t),
            Curve::Arc(a) => a.point_at(t),
        }
    }
    /// Unit tangent at `t`; `None` only for a degenerate segment.
    pub fn tangent_at(&self, t: f64) -> Option<Vec2> {
        match self {
            Curve::Line(l) => l.tangent(),
            Curve::Arc(a) => Some(a.tangent_at(t)),
        }
    }
    pub fn midpoint(&self) -> Vec2 {
        self.point_at(0.5)
    }
    pub fn aabb(&self) -> Aabb2 {
        match self {
            Curve::Line(l) => l.aabb(),
            Curve::Arc(a) => a.aabb(),
        }
    }
    pub fn reversed(&self) -> Self {
        match self {
            Curve::Line(l) => Curve::Line(l.reversed()),
            Curve::Arc(a) => Curve::Arc(a.reversed()),
        }
    }
    pub fn offset(&self, distance: f64) -> GeomResult<Self> {
        match self {
            Curve::Line(l) => l.offset(distance).map(Curve::Line),
            Curve::Arc(a) => a.offset(distance).map(Curve::Arc),
        }
    }
    pub fn signed_area_term(&self) -> f64 {
        match self {
            Curve::Line(l) => l.signed_area_term(),
            Curve::Arc(a) => a.signed_area_term(),
        }
    }
    /// Closest point on the curve and its parameter in `[0, 1]`.
    pub fn closest_point(&self, p: Vec2) -> (Vec2, f64) {
        match self {
            Curve::Line(l) => l.closest_point(p),
            Curve::Arc(a) => a.closest_point(p),
        }
    }
    /// Parameter in `[0, 1]` of a point assumed to lie on the curve.
    pub fn parameter_at_point(&self, p: Vec2) -> f64 {
        self.closest_point(p).1
    }
    /// Same curve, new start point (trim or extend).
    pub fn with_start(&self, p: Vec2) -> GeomResult<Self> {
        Ok(match self {
            Curve::Line(l) => Curve::Line(l.with_start(p)),
            Curve::Arc(a) => Curve::Arc(a.with_start(p)?),
        })
    }
    /// Same curve, new end point (trim or extend).
    pub fn with_end(&self, p: Vec2) -> GeomResult<Self> {
        Ok(match self {
            Curve::Line(l) => Curve::Line(l.with_end(p)),
            Curve::Arc(a) => Curve::Arc(a.with_end(p)?),
        })
    }
    /// First moment about the y axis along the curve, `integral of x ds`.
    /// Exact: `len * (x0 + x1) / 2` for a segment and
    /// `r * cx * |sweep| + r^2 * sign(sweep) * (sin a1 - sin a0)` for an arc.
    /// A sweep needs it for the line-centroid of the profile boundary
    /// (generalised Pappus for lateral area).
    pub fn first_moment_x(&self) -> f64 {
        match self {
            Curve::Line(l) => l.length() * (l.start.x + l.end.x) / 2.0,
            Curve::Arc(a) => {
                let (a0, a1) = (a.start_angle, a.end_angle());
                a.radius * a.centre.x * a.sweep.abs()
                    + a.radius * a.radius * a.sweep.signum() * (a1.sin() - a0.sin())
            }
        }
    }
    /// Farthest point along `direction` (support function); exact for arcs.
    pub fn support_point(&self, direction: Vec2) -> Vec2 {
        match self {
            Curve::Line(l) => {
                if l.end.dot(direction) > l.start.dot(direction) {
                    l.end
                } else {
                    l.start
                }
            }
            Curve::Arc(a) => a.support_point(direction),
        }
    }
    pub fn is_arc(&self) -> bool {
        matches!(self, Curve::Arc(_))
    }
    pub fn as_arc(&self) -> Option<&Arc> {
        match self {
            Curve::Arc(a) => Some(a),
            _ => None,
        }
    }
    /// ASTERION's own [`Curve::offset`] is positive to the LEFT, so this is `offset(-d)`.
    pub fn offset_reference(&self, d: f64) -> GeomResult<Self> {
        self.offset(-d)
    }

    /// Polyline approximation — **only** for display and hit-testing. Never use
    /// it for quantities: areas and lengths have closed forms above.
    pub fn tessellate(&self, max_sagitta: f64) -> Vec<Vec2> {
        match self {
            Curve::Line(l) => vec![l.start, l.end],
            Curve::Arc(a) => {
                let max_sagitta = max_sagitta.max(1e-9);
                // sagitta of a sub-arc of angle da is r * (1 - cos(da/2))
                let ratio = (1.0 - max_sagitta / a.radius).clamp(-1.0, 1.0);
                let da = 2.0 * ratio.acos();
                let n = if da > 0.0 {
                    (a.sweep.abs() / da).ceil().max(1.0) as usize
                } else {
                    16
                };
                (0..=n).map(|i| a.point_at(i as f64 / n as f64)).collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VTOL: f64 = 1e-12;

    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-12, "{a} != {b}");
    }
    fn near_pt(a: Vec2, b: Vec2) {
        assert!(a.distance_to(b) < 1e-12, "{a:?} != {b:?}");
    }

    #[test]
    fn segment_metrics() {
        let s = LineSegment::new(Vec2::new(0.0, 0.0), Vec2::new(3.0, 4.0));
        near(s.length(), 5.0);
        near_pt(s.point_at(0.5), Vec2::new(1.5, 2.0));
        near_pt(s.tangent().unwrap(), Vec2::new(0.6, 0.8));
    }

    #[test]
    fn semicircle_length_and_points() {
        // Unit semicircle, CCW from (1,0) to (-1,0).
        let a = Arc::from_center_angles(Vec2::new(0.0, 0.0), 1.0, 0.0, PI, true).unwrap();
        near(a.length(), PI);
        near_pt(a.start(), Vec2::new(1.0, 0.0));
        near_pt(a.end(), Vec2::new(-1.0, 0.0));
        near_pt(a.point_at(0.5), Vec2::new(0.0, 1.0));
        // Tangent at the top of a CCW arc points towards -X.
        near_pt(a.tangent_at(0.5), Vec2::new(-1.0, 0.0));
        near(a.sagitta(), 1.0);
    }

    #[test]
    fn quarter_circle_area_term_matches_hand_value() {
        // Quarter circle r=2 centred at origin, CCW from 0 to pi/2.
        let a = Arc::from_center_angles(Vec2::new(0.0, 0.0), 2.0, 0.0, PI / 2.0, true).unwrap();
        near(a.length(), PI);
        // Green term for a centred arc is r^2 * sweep / 2 = 4 * (pi/2) / 2 = pi.
        near(a.signed_area_term(), PI);
        // chord shoelace + circular segment must give the same number
        let chord = a.chord().signed_area_term();
        near(chord + a.circular_segment_area(), PI);
    }

    #[test]
    fn arc_aabb_is_exact_not_tessellated() {
        // CCW arc from -45 deg to +45 deg, r = 1: the x extreme (angle 0) is inside.
        let a = Arc::from_center_angles(
            Vec2::new(0.0, 0.0),
            1.0,
            -PI / 4.0,
            PI / 4.0,
            true,
        )
        .unwrap();
        let b = a.aabb();
        near(b.max.x, 1.0);
        near(b.min.x, (PI / 4.0).cos());
        near(b.max.y, (PI / 4.0).sin());
        near(b.min.y, -(PI / 4.0).sin());
    }

    #[test]
    fn full_circle_aabb_covers_all_four_extremes() {
        let a = Arc::from_center_angles(Vec2::new(1.0, 2.0), 3.0, 0.0, 0.0, true).unwrap();
        assert!(a.is_full_circle());
        near(a.length(), TAU * 3.0);
        let b = a.aabb();
        near(b.min.x, -2.0);
        near(b.max.x, 4.0);
        near(b.min.y, -1.0);
        near(b.max.y, 5.0);
    }

    #[test]
    fn reversed_arc_is_exact_inverse() {
        let a = Arc::from_center_angles(Vec2::new(1.0, 1.0), 2.5, 0.3, 2.0, true).unwrap();
        let r = a.reversed();
        near_pt(r.start(), a.end());
        near_pt(r.end(), a.start());
        near(r.length(), a.length());
        near(r.signed_area_term(), -a.signed_area_term());
    }

    #[test]
    fn arc_from_three_points_recovers_known_circle() {
        // Points on the unit circle at 0, 90 and 180 degrees.
        let a = Arc::from_three_points(
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(-1.0, 0.0),
            VTOL,
        )
        .unwrap();
        near_pt(a.centre, Vec2::new(0.0, 0.0));
        near(a.radius, 1.0);
        assert!(a.is_counter_clockwise());
        near(a.sweep, PI);
    }

    #[test]
    fn arc_from_three_points_rejects_collinear() {
        let e = Arc::from_three_points(
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(2.0, 0.0),
            1e-9,
        )
        .unwrap_err();
        assert_eq!(e.code(), crate::error::codes::POINTS_COLLINEAR);
    }

    #[test]
    fn bulge_round_trip() {
        for sweep in [0.5, PI / 2.0, 3.0, -0.75, -PI, 4.0] {
            let a = Arc::from_center_angles(
                Vec2::new(-2.0, 0.5),
                1.75,
                0.2,
                0.2 + sweep,
                sweep > 0.0,
            )
            .unwrap();
            let b = Arc::from_endpoints_bulge(a.start(), a.end(), a.bulge()).unwrap();
            near_pt(b.centre, a.centre);
            near(b.radius, a.radius);
            near(b.sweep, a.sweep);
        }
    }

    #[test]
    fn offset_left_shrinks_ccw_arc_and_grows_cw_arc() {
        let ccw = Arc::from_center_angles(Vec2::new(0.0, 0.0), 2.0, 0.0, PI, true).unwrap();
        near(ccw.offset(0.5).unwrap().radius, 1.5);
        let cw = ccw.reversed();
        near(cw.offset(0.5).unwrap().radius, 2.5);
        assert_eq!(
            ccw.offset(2.0).unwrap_err().code(),
            crate::error::codes::OFFSET_COLLAPSES_CURVE
        );
    }

    #[test]
    fn segment_offset_moves_to_the_left() {
        let s = LineSegment::new(Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0));
        let o = s.offset(0.25).unwrap();
        near_pt(o.start, Vec2::new(0.0, 0.25));
        near_pt(o.end, Vec2::new(1.0, 0.25));
    }

    #[test]
    fn tessellation_respects_sagitta_but_is_display_only() {
        let a = Arc::from_center_angles(Vec2::new(0.0, 0.0), 1.0, 0.0, TAU / 2.0, true).unwrap();
        let pts = Curve::Arc(a).tessellate(0.001);
        assert!(pts.len() > 20, "{} points", pts.len());
        // Every sampled point is on the circle.
        for p in &pts {
            near(p.length(), 1.0);
        }
    }

    #[test]
    fn contains_angle_wraps_across_pi() {
        // CCW arc from 3.0 rad sweeping +0.5: it crosses +pi, so its end angle
        // 3.5 rad is the same direction as -2.783 rad.
        let a = Arc::from_center_angles(Vec2::new(0.0, 0.0), 1.0, 3.0, 3.5, true).unwrap();
        near(a.sweep, 0.5);
        assert!(a.contains_angle(3.0, 0.0));
        assert!(a.contains_angle(3.4, 0.0));
        // -2.9 rad == 3.383 rad: inside, reached only by wrapping past +pi.
        assert!(a.contains_angle(-2.9, 0.0));
        // Just before the start and well past the end are both outside.
        assert!(!a.contains_angle(2.9, 0.0));
        assert!(!a.contains_angle(0.0, 0.0));
        assert!(!a.contains_angle(-2.6, 0.0));
    }

    #[test]
    fn contains_angle_for_a_clockwise_arc() {
        // CW arc from 3.0 rad sweeping -0.5 (end 2.5 rad).
        let a = Arc::from_center_angles(Vec2::new(0.0, 0.0), 1.0, 3.0, 2.5, false).unwrap();
        near(a.sweep, -0.5);
        assert!(a.contains_angle(3.0, 0.0));
        assert!(a.contains_angle(2.6, 0.0));
        assert!(!a.contains_angle(3.1, 0.0));
        assert!(!a.contains_angle(0.0, 0.0));
    }

    /// In the reference product's convention a positive offset goes to the right of the tangent about +Z - a +X line moves
    /// to -Y, a counter-clockwise arc grows, a clockwise one shrinks; ASTERION's own offset is the opposite sign.
    #[test]
    fn the_reference_offset_goes_to_the_right_of_the_tangent() {
        let l = Curve::line(Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0));
        near_pt(l.offset_reference(1.0).unwrap().start(), Vec2::new(0.0, -1.0));
        let ccw = Curve::Arc(Arc::new(Vec2::new(0.0, 0.0), 2.0, 0.0, 1.0).unwrap());
        near(ccw.offset_reference(0.5).unwrap().as_arc().unwrap().radius, 2.5);
        let cw = Curve::Arc(Arc::new(Vec2::new(0.0, 0.0), 2.0, 0.0, -1.0).unwrap());
        near(cw.offset_reference(0.5).unwrap().as_arc().unwrap().radius, 1.5);
    }

    /// The raw derivative of an arc (parameter = angle) has length R and points along the travel; the normalised one is
    /// that times the span.
    #[test]
    fn the_raw_arc_derivative_has_length_r() {
        let a = Arc::new(Vec2::new(0.0, 0.0), 3.0, 0.0, PI / 2.0).unwrap();
        near_pt(a.raw_derivative_at_angle(0.0), Vec2::new(0.0, 3.0));
        near_pt(a.raw_derivative_at_angle(0.0) * (1.0 / 3.0), a.tangent_at(0.0));
        let c = a.reversed();
        near_pt(c.raw_derivative_at_angle(PI / 2.0) * (1.0 / 3.0), c.tangent_at(0.0));
    }
}
