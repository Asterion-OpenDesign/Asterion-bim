//! Rooms bounded by ARC walls.
//!
//! The enclosed regions of a level are holes of the union of the wall bodies, and an arc wall's body is a sampled polygon
//! (`regen::arc_body_prism`: 96 segments a turn). The engine's room boundary must not be that sampling: this module turns
//! every run of consecutive region vertices that lie on the inner or outer circle of one arc wall back into ONE exact arc
//! segment, ends the arc exactly where the circle meets the neighbouring straight face (the region's own vertex there sits
//! on a chord), and gives the room the exact area (polygon of the segment ends plus the circular segments) and perimeter
//! (arc lengths). ENGINE CHOICE until round 32 measures the reference product's segments.

use crate::model::BimModel;
use crate::regen::{arc_from_points, Arc2, RoomSegment};
use asterion_core::ElementId;
use asterion_geom::Vec2;
use std::f64::consts::TAU;

/// Distance from a circle within which a region vertex is "on" it (the samples are exact to fp).
const ON_CIRCLE_M: f64 = 1e-7;

/// One circle of an arc wall's body: the wall, its location arc and the radius of this face.
#[derive(Clone, Copy, Debug)]
struct Circle {
    wall: ElementId,
    arc: Arc2,
    radius: f64,
}

impl Circle {
    fn on(&self, p: Vec2) -> bool {
        let d = (p - self.arc.center).length();
        if (d - self.radius).abs() > ON_CIRCLE_M {
            return false;
        }
        let ang = (p.y - self.arc.center.y).atan2(p.x - self.arc.center.x);
        let rel = if self.arc.sweep >= 0.0 { (ang - self.arc.start_angle).rem_euclid(TAU) } else { (self.arc.start_angle - ang).rem_euclid(TAU) };
        let eps = 1e-9 / self.radius.max(1e-9);
        rel <= self.arc.sweep.abs() + eps || rel >= TAU - eps
    }

    /// The intersection of this circle with the line through `a` and `b` nearest to `near`.
    fn meet_line(&self, a: Vec2, b: Vec2, near: Vec2) -> Option<Vec2> {
        let d = (b - a).normalized()?;
        let f = a - self.arc.center;
        let bq = f.dot(d);
        let c = f.dot(f) - self.radius * self.radius;
        let disc = bq * bq - c;
        if disc < 0.0 {
            return None;
        }
        let s = disc.sqrt();
        let p1 = a + d * (-bq + s);
        let p2 = a + d * (-bq - s);
        Some(if (p1 - near).length() <= (p2 - near).length() { p1 } else { p2 })
    }
}

/// The circles of the room-bounding arc walls of `level` (inner and outer face of each).
fn circles(model: &BimModel, level: ElementId) -> Vec<Circle> {
    let mut out = Vec::new();
    for w in model.walls.values().filter(|w| w.through.is_some() && crate::regen::wall_bounds_level(model, w, level)) {
        let Some(arc) = w.through.and_then(|t| arc_from_points(w.start, w.end, t)) else { continue };
        let hw = model.wall_types.get(&w.type_id).map_or(0.0, |t| t.structure.width() / 2.0);
        for r in [arc.radius - hw, arc.radius + hw] {
            if r > 0.0 {
                out.push(Circle { wall: w.id, arc, radius: r });
            }
        }
    }
    out
}

/// A piece of the collapsed boundary: a plain vertex, or a run of vertices on one circle.
enum Piece {
    Point(Vec2),
    Run { circle: usize, first: usize, last: usize },
}

/// The finish boundary of the region `poly` (counter-clockwise) with every sampled arc collapsed into
/// an exact arc segment. With no arc wall on the level it is the plain polygon, segment by segment.
pub fn finish_segments(model: &BimModel, level: ElementId, poly: &[Vec2], host_of_line: impl Fn(Vec2, Vec2) -> Option<ElementId>) -> Vec<RoomSegment> {
    let n = poly.len();
    let cs = circles(model, level);
    let attr: Vec<Vec<usize>> = poly.iter().map(|p| cs.iter().enumerate().filter(|(_, c)| c.on(*p)).map(|(i, _)| i).collect()).collect();
    if cs.is_empty() || attr.iter().all(|a| a.is_empty()) {
        return (0..n).map(|i| RoomSegment { start: poly[i], end: poly[(i + 1) % n], host: host_of_line(poly[i], poly[(i + 1) % n]), arc: None }).collect();
    }
    // start the walk at an off-circle vertex when there is one, else at a junction of two runs (a vertex
    // no single circle runs through), so no run is cut by the wrap-around; a run may close the loop by
    // ending at the start vertex (offset `n`)
    let s0 = (0..n)
        .find(|k| attr[*k].is_empty())
        .or_else(|| (0..n).find(|k| !attr[*k].iter().any(|c| attr[(*k + n - 1) % n].contains(c) && attr[(*k + 1) % n].contains(c))))
        .unwrap_or(0);
    let mut pieces: Vec<Piece> = Vec::new();
    let mut k = 0usize; // offset from s0
    let idx = |o: usize| (s0 + o) % n;
    while k < n {
        let i = idx(k);
        if attr[i].is_empty() {
            pieces.push(Piece::Point(poly[i]));
            k += 1;
            continue;
        }
        // the circle that continues to the next vertex, else the first candidate
        let next = idx(k + 1);
        let circle = attr[i].iter().copied().find(|c| attr[next].contains(c)).unwrap_or(attr[i][0]);
        let mut j = k;
        while j + 1 <= n && attr[idx(j + 1)].contains(&circle) {
            j += 1;
        }
        if j == k {
            pieces.push(Piece::Point(poly[i]));
            k += 1;
            continue;
        }
        pieces.push(Piece::Run { circle, first: k, last: j });
        // another circle may continue from the shared end vertex (an arc meeting an arc)
        let shared_next = j < n && attr[idx(j)].iter().any(|c| *c != circle && attr[idx(j + 1)].contains(c));
        k = if shared_next { j } else { j + 1 };
    }
    // the pieces as segments; a run's ends are moved to the exact meeting with the neighbouring edges
    let m = pieces.len();
    let point_of = |p: &Piece, at_end: bool| -> Vec2 {
        match p {
            Piece::Point(v) => *v,
            Piece::Run { first, last, .. } => poly[idx(if at_end { *last } else { *first })],
        }
    };
    let mut out: Vec<RoomSegment> = Vec::new();
    for (pi, piece) in pieces.iter().enumerate() {
        let prev = &pieces[(pi + m - 1) % m];
        let nextp = &pieces[(pi + 1) % m];
        match piece {
            Piece::Point(a) => {
                // a line to the next plain vertex - unless that vertex is the chord vertex before a run, whose
                // line the run emits itself, ended where the circle meets it
                if let Piece::Point(b) = nextp {
                    if !matches!(pieces[(pi + 2) % m], Piece::Run { .. }) {
                        out.push(RoomSegment { start: *a, end: *b, host: host_of_line(*a, *b), arc: None });
                    }
                }
            }
            Piece::Run { circle, first, last } => {
                let c = &cs[*circle];
                let mut start = poly[idx(*first)];
                let mut end = poly[idx(*last)];
                let mut trailing: Option<RoomSegment> = None;
                // the previous piece is a plain vertex on the neighbour's face line: the arc starts where
                // the circle meets that line, and the line segment from the vertex before ends there too
                if let Piece::Point(v) = prev {
                    let before = point_of(&pieces[(pi + m - 2) % m], true);
                    if let Some(x) = c.meet_line(before, *v, *v).filter(|x| (*x - *v).length() < 0.5) {
                        start = x;
                        out.push(RoomSegment { start: before, end: x, host: host_of_line(before, *v), arc: None });
                    } else {
                        // no chord vertex: the circle does not meet the face line near `v` (a mitred corner, puts the run's
                        // first vertex ON the neighbour's face line), so `v` runs to the first vertex along that face
                        out.push(RoomSegment { start: before, end: *v, host: host_of_line(before, *v), arc: None });
                        out.push(RoomSegment { start: *v, end: start, host: host_of_line(*v, start).or(Some(c.wall)), arc: None });
                    }
                }
                if let Piece::Point(v) = nextp {
                    let after = point_of(&pieces[(pi + 2) % m], false);
                    if let Some(x) = c.meet_line(*v, after, *v).filter(|x| (*x - *v).length() < 0.5) {
                        end = x;
                        // the following line starts at x: emitted by the Point piece as (v, after); fix it up
                        // after the loop (see below) by remembering the replacement
                    } else if (poly[idx(*last)] - *v).length() > 1e-9 {
                        // a mitred corner: the run's last vertex is on the neighbour's face line, which runs on to `v`
                        trailing = Some(RoomSegment { start: poly[idx(*last)], end: *v, host: host_of_line(poly[idx(*last)], *v), arc: None });
                    }
                }
                let mid = poly[idx((*first + *last) / 2)];
                let arc = arc_from_points(start, end, mid);
                out.push(RoomSegment { start, end, host: Some(c.wall), arc });
                if let Some(t) = trailing.take() {
                    out.push(t);
                }
            }
        }
    }
    // stitch: every segment starts where the previous one ended (the exact arc ends replace the chord
    // vertices of the following lines)
    let k = out.len();
    for i in 0..k {
        let prev_end = out[(i + k - 1) % k].end;
        if out[i].arc.is_none() && (out[i].start - prev_end).length() > 1e-12 && (out[i].start - prev_end).length() < 0.5 {
            out[i].start = prev_end;
        }
    }
    out.retain(|s| s.length() > 1e-9);
    out
}

/// The signed area of the circular segment an arc adds to the polygon of its chord: positive when the
/// arc bulges to the RIGHT of its chord (a counter-clockwise arc), which is outward for a
/// counter-clockwise boundary.
pub fn chord_segment_area(arc: &Arc2) -> f64 {
    let t = arc.sweep.abs();
    let seg = 0.5 * arc.radius * arc.radius * (t - t.sin());
    if arc.sweep >= 0.0 {
        seg
    } else {
        -seg
    }
}

/// Area of a counter-clockwise boundary of segments: the polygon of the segment starts plus the
/// circular segments of the arcs.
pub fn area(segs: &[RoomSegment]) -> f64 {
    let pts: Vec<Vec2> = segs.iter().map(|s| s.start).collect();
    let poly = asterion_geom::polygon_signed_area(&pts).abs();
    poly + segs.iter().filter_map(|s| s.arc.as_ref()).map(chord_segment_area).sum::<f64>()
}

/// Axis-aligned extent of a boundary of segments (arc extremes included).
pub fn extent(segs: &[RoomSegment]) -> (Vec2, Vec2) {
    let mut lo = Vec2::new(f64::INFINITY, f64::INFINITY);
    let mut hi = Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut take = |p: Vec2| {
        lo = Vec2::new(lo.x.min(p.x), lo.y.min(p.y));
        hi = Vec2::new(hi.x.max(p.x), hi.y.max(p.y));
    };
    for s in segs {
        take(s.start);
        take(s.end);
        if let Some(a) = &s.arc {
            let (l, h) = a.extent(a.radius, a.radius);
            take(l);
            take(h);
        }
    }
    (lo, hi)
}

/// What a room whose boundary has ARCS reports besides its exact area: its Perimeter, its Volume and its box.
#[derive(Clone, Copy, Debug)]
pub struct ArcRoomQuantities {
    pub perimeter: f64,
    pub volume: f64,
    pub lo: Vec2,
    pub hi: Vec2,
}

fn wrap_pi(x: f64) -> f64 {
    (x + std::f64::consts::PI).rem_euclid(TAU) - std::f64::consts::PI
}

/// The signed sweep of the arc segment `s` from its start to its end.
fn segment_sweep(s: &RoomSegment, a: &Arc2) -> f64 {
    let a0 = (s.start.y - a.center.y).atan2(s.start.x - a.center.x);
    let a1 = (s.end.y - a.center.y).atan2(s.end.x - a.center.x);
    let mag = a.sweep.abs();
    if wrap_pi(a0 + mag - a1).abs() < 1e-6 {
        mag
    } else if wrap_pi(a0 - mag - a1).abs() < 1e-6 {
        -mag
    } else {
        a.sweep
    }
}

/// The analytic curves of a boundary of segments, counter-clockwise (the orientation of a material loop).
fn boundary_curves(segs: &[RoomSegment]) -> Option<Vec<asterion_geom::Curve>> {
    let mut curves = Vec::new();
    for s in segs {
        match &s.arc {
            None => curves.push(asterion_geom::Curve::line(s.start, s.end)),
            Some(a) => {
                let a0 = (s.start.y - a.center.y).atan2(s.start.x - a.center.x);
                curves.push(asterion_geom::Curve::arc(asterion_geom::Arc::new(a.center, a.radius, a0, segment_sweep(s, a)).ok()?));
            }
        }
    }
    let area: f64 = curves.iter().map(|c| c.signed_area_term()).sum();
    if area < 0.0 {
        curves = curves.iter().rev().map(|c| c.reversed()).collect();
    }
    Some(curves)
}

/// The Perimeter, Volume and box of a room whose Finish boundary has arcs.
///
/// 19.354714135 = 20.954714135
/// - 1.6; a round room of two tangent half arcs is its centre circle, 6 pi; an arc of 0.3 with lines of 0.2: -0.5), to
///   1e-9. The Volume is that of the finish loop extruded by the height of the room, exact (the reference product reports
///   74.476896 against the exact 74.475838) and the box is that of the edge sample points (the reference product's box is
///   5.898426 for an arc that reaches 5.9). `None` when a segment has no host or the loop is degenerate.
pub fn arc_room_quantities(
    segs: &[RoomSegment],
    center: &[RoomSegment],
    center_len: f64,
    half_width: impl Fn(ElementId) -> Option<f64>,
    z0: f64,
    height: f64,
) -> Option<ArcRoomQuantities> {
    if segs.len() < 2 || segs.iter().all(|s| s.arc.is_none()) || center.len() != segs.len() {
        return None;
    }
    let curves = boundary_curves(segs)?;
    // the orientation of the segments as given
    let pts: Vec<Vec2> = center
        .iter()
        .flat_map(|s| match &s.arc {
            Some(a) => {
                let a0 = (s.start.y - a.center.y).atan2(s.start.x - a.center.x);
                let m = a.center + Vec2::new((a0 + segment_sweep(s, a) / 2.0).cos(), (a0 + segment_sweep(s, a) / 2.0).sin()) * a.radius;
                vec![s.start, m]
            }
            None => vec![s.start],
        })
        .collect();
    let sign = if asterion_geom::polygon_signed_area(&pts) > 0.0 { 1.0 } else { -1.0 };
    let tangent = |s: &RoomSegment, at_end: bool| -> Option<Vec2> {
        match &s.arc {
            None => (s.end - s.start).normalized(),
            Some(a) => {
                let p = if at_end { s.end } else { s.start };
                let radial = (p - a.center).normalized()?;
                Some(radial.perp_ccw() * segment_sweep(s, a).signum())
            }
        }
    };
    let n = segs.len();
    let mut trims = 0.0;
    for i in 0..n {
        // the tangents are those of the CENTRE curves of the two walls at the corner (their location lines)
        let (a, b) = (&center[i], &center[(i + 1) % n]);
        let (ha, hb) = (half_width(segs[i].host?)?, half_width(segs[(i + 1) % n].host?)?);
        let (t_in, t_out) = (tangent(a, true)?, tangent(b, false)?);
        let turn = t_in.cross(t_out).atan2(t_in.dot(t_out));
        let theta = std::f64::consts::PI - sign * turn;
        if theta >= std::f64::consts::PI - 1e-9 {
            continue; // a tangent continuation trims nothing
        }
        if !(theta > 1e-9) {
            return None;
        }
        trims += (ha + hb) / (theta / 2.0).tan();
    }
    let perimeter = center_len - trims;
    let tols = asterion_geom::stokes::TessTols::display_default();
    let loop_ = asterion_geom::stokes::CapLoop { curves: curves.clone(), is_hole: false };
    let q = asterion_geom::stokes::extruded_loops_metrics(
        &[loop_],
        asterion_geom::stokes::CapPlane::flat(z0),
        asterion_geom::stokes::CapPlane::flat(z0 + height),
        &tols,
    )
    .ok()?;
    let (mut lo, mut hi) = (Vec2::new(f64::INFINITY, f64::INFINITY), Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY));
    for c in &curves {
        let (a, b) = asterion_geom::stokes::sampled_curve_box(c, &tols)?;
        lo = Vec2::new(lo.x.min(a.x), lo.y.min(a.y));
        hi = Vec2::new(hi.x.max(b.x), hi.y.max(b.y));
    }
    Some(ArcRoomQuantities { perimeter, volume: q.volume, lo, hi })
}

/// The centre boundary: each finish segment moved to its host's location curve, cornered where the
/// location curves of neighbouring hosts meet. An arc host's segment is an arc on its location circle.
pub fn center_segments(model: &BimModel, segs: &[RoomSegment], host_line: impl Fn(ElementId) -> Option<(Vec2, Vec2)>) -> Vec<RoomSegment> {
    let m = segs.len();
    let loc_arc = |h: ElementId| -> Option<Arc2> {
        let w = model.walls.get(&h)?;
        arc_from_points(w.start, w.end, w.through?)
    };
    let corner = |i: usize| -> Vec2 {
        let (a, b) = (&segs[i], &segs[(i + 1) % m]);
        let fallback = a.end;
        let (Some(ha), Some(hb)) = (a.host, b.host) else { return fallback };
        match (loc_arc(ha), loc_arc(hb)) {
            (None, None) => {
                if let (Some((p0, p1)), Some((q0, q1))) = (host_line(ha), host_line(hb)) {
                    let (d1, d2) = (p1 - p0, q1 - q0);
                    let den = d1.cross(d2);
                    if den.abs() > 1e-12 {
                        return p0 + d1 * ((q0 - p0).cross(d2) / den);
                    }
                }
                fallback
            }
            (Some(arc), None) | (None, Some(arc)) => {
                let other = if loc_arc(ha).is_some() { hb } else { ha };
                let Some((q0, q1)) = host_line(other) else { return fallback };
                Circle { wall: other, arc, radius: arc.radius }.meet_line(q0, q1, fallback).unwrap_or(fallback)
            }
            (Some(a1), Some(a2)) => {
                if (a1.center - a2.center).length() < 1e-9 && (a1.radius - a2.radius).abs() < 1e-9 {
                    // one circle: the junction is the finish corner moved radially onto it
                    let r = (fallback - a1.center).normalized().unwrap_or(Vec2::new(1.0, 0.0));
                    return a1.center + r * a1.radius;
                }
                // two circles: the intersection nearest the finish corner
                let d = (a2.center - a1.center).length();
                let (r1, r2) = (a1.radius, a2.radius);
                if d < 1e-12 || d > r1 + r2 || d < (r1 - r2).abs() {
                    return fallback;
                }
                let x = (d * d + r1 * r1 - r2 * r2) / (2.0 * d);
                let h = (r1 * r1 - x * x).max(0.0).sqrt();
                let u = (a2.center - a1.center) * (1.0 / d);
                let v = Vec2::new(-u.y, u.x);
                let p = a1.center + u * x;
                let (c1, c2) = (p + v * h, p - v * h);
                if (c1 - fallback).length() <= (c2 - fallback).length() {
                    c1
                } else {
                    c2
                }
            }
        }
    };
    let corners: Vec<Vec2> = (0..m).map(corner).collect();
    (0..m)
        .map(|i| {
            let (start, end) = (corners[(i + m - 1) % m], corners[i]);
            let arc = segs[i].host.and_then(loc_arc).and_then(|la| {
                // the location arc between the two corners, through the point at the finish arc's middle angle
                let f = segs[i].arc.as_ref()?;
                let mid = f.point_at(0.5, f.radius);
                let r = (mid - la.center).normalized()?;
                arc_from_points(start, end, la.center + r * la.radius)
            });
            RoomSegment { start, end, host: segs[i].host, arc }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ops, CompoundStructure, Layer, LayerFunction};

    fn doc_with(walls: &[((f64, f64), (f64, f64), Option<(f64, f64)>)]) -> (crate::BimDocument, ElementId, Vec<ElementId>) {
        let mut d = crate::new_document();
        let (l, ids) = d
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let wt = ops::create_wall_type(m, cx, "W", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
                let mut ids = Vec::new();
                for (a, b, t) in walls {
                    let spec = ops::NewWall { wall_type: wt, level: l, start: Vec2::new(a.0, a.1), end: Vec2::new(b.0, b.1), height: 3.0, base_offset: 0.0, flip: false, structural: false };
                    ids.push(match t {
                        Some(t) => ops::create_arc_wall(m, cx, spec, Vec2::new(t.0, t.1))?,
                        None => ops::create_wall(m, cx, spec)?,
                    });
                }
                Ok((l, ids))
            })
            .value
            .unwrap();
        (d, l, ids)
    }

    #[test]
    fn a_room_closed_by_an_outward_arc_has_one_exact_arc_segment() {
        // three straight walls and an arc bulging out (made last, so the straight walls run through the
        // corners and their bodies cover the arc's ends): the finish boundary is three lines and one arc on
        // the arc wall's inner circle, the area is the chord polygon plus the circular segment, exactly
        let (mut d, l, ids) = doc_with(&[((0.0, 0.0), (6.0, 0.0), None), ((6.0, 0.0), (6.0, 4.0), None), ((0.0, 4.0), (0.0, 0.0), None), ((6.0, 4.0), (0.0, 4.0), Some((3.0, 5.0)))]);
        let room = d.transact("room", false, |m, cx| ops::create_room(m, cx, l, Vec2::new(3.0, 2.0))).value.unwrap();
        let m = d.model();
        let st = crate::regen::room_state(m, &m.rooms[&room]).unwrap();
        assert!(st.enclosed);
        let arcs: Vec<&RoomSegment> = st.finish.iter().filter(|s| s.arc.is_some()).collect();
        assert_eq!(arcs.len(), 1, "{:?}", st.finish);
        assert_eq!(arcs[0].host, Some(ids[3]));
        let a = arcs[0].arc.unwrap();
        // the arc (6,4)-(0,4) through (3,5): centre (3, 0), radius 5; the inner face is 4.9
        assert!((a.center.x - 3.0).abs() < 1e-9 && a.center.y.abs() < 1e-9 && (a.radius - 4.9).abs() < 1e-9, "{a:?}");
        // its ends are on the straight walls' inner faces x = 5.9 and x = 0.1
        assert!((arcs[0].start.x - 5.9).abs() < 1e-9 && (arcs[0].end.x - 0.1).abs() < 1e-9, "{:?}", arcs[0]);
        assert_eq!(st.finish.len(), 4, "{:?}", st.finish);
        // area: the polygon of the four corners plus the circular segment of the arc
        let y_at = |x: f64| (4.9f64 * 4.9 - (x - 3.0) * (x - 3.0)).sqrt();
        let chord_poly = 5.8 * (y_at(5.9) - 0.1);
        let half = (5.8f64 / 2.0 / 4.9).asin();
        let seg = 0.5 * 4.9 * 4.9 * (2.0 * half - (2.0 * half).sin());
        assert!((st.area - (chord_poly + seg)).abs() < 1e-9, "{} {}", st.area, chord_poly + seg);
        // the Finish boundary length is the sum of its segments; Room.Perimeter is the Center length less the corner trims
        // (OBSERVED 093): 6 + 4 + 4 + 5 * 2 asin(3/5), less 0.2 at each 90 degree corner and 0.1 at each corner of a side
        // and the arc
        let finish_len = 5.8 + 2.0 * (y_at(5.9) - 0.1) + 4.9 * 2.0 * half;
        let fsum: f64 = st.finish.iter().map(|s| s.length()).sum();
        assert!((fsum - finish_len).abs() < 1e-9, "{fsum}");
        let centre = 14.0 + 5.0 * 2.0 * (3.0f64 / 5.0).asin();
        assert!((st.perimeter - (centre - 0.4 - 0.2)).abs() < 1e-9, "{}", st.perimeter);
        // the centre boundary runs on the location circle (radius 5) between the location lines x = 6 and x = 0
        let ca: Vec<&RoomSegment> = st.center.iter().filter(|s| s.arc.is_some()).collect();
        assert_eq!(ca.len(), 1);
        assert!((ca[0].arc.unwrap().radius - 5.0).abs() < 1e-9 && (ca[0].start.x - 6.0).abs() < 1e-9 && (ca[0].end.x - 0.0).abs() < 1e-9, "{:?}", ca[0]);
        // The box of a room with an arc is that of the stored sample points, so the apex of the arc may fall short of 4.9
        assert!(st.bbox_max.y <= 4.9 + 1e-12 && st.bbox_max.y > 4.89, "{}", st.bbox_max.y);
    }

    #[test]
    fn a_round_room_of_two_half_arcs_has_two_arc_segments_and_the_exact_disc_area() {
        let (mut d, l, _ids) = doc_with(&[((0.0, 40.0), (6.0, 40.0), Some((3.0, 43.0))), ((6.0, 40.0), (0.0, 40.0), Some((3.0, 37.0)))]);
        let room = d.transact("room", false, |m, cx| ops::create_room(m, cx, l, Vec2::new(3.0, 40.0))).value.unwrap();
        let m = d.model();
        let st = crate::regen::room_state(m, &m.rooms[&room]).unwrap();
        assert!(st.enclosed);
        assert_eq!(st.finish.len(), 2, "{:?}", st.finish);
        assert!(st.finish.iter().all(|s| s.arc.is_some()));
        let r = 2.9;
        assert!((st.area - std::f64::consts::PI * r * r).abs() < 1e-9, "{}", st.area);
        // OBSERVED (093 c): the Perimeter of a room with arcs is the Center length less the corner trims (6 pi here: the
        // two half arcs are tangent, no trim)
        assert!((st.perimeter - TAU * 3.0).abs() < 1e-9, "{}", st.perimeter);
    }

    #[test]
    fn straight_rooms_are_unchanged() {
        let (mut d, l, _) = doc_with(&[((0.0, 0.0), (6.0, 0.0), None), ((6.0, 0.0), (6.0, 4.0), None), ((6.0, 4.0), (0.0, 4.0), None), ((0.0, 4.0), (0.0, 0.0), None)]);
        let room = d.transact("room", false, |m, cx| ops::create_room(m, cx, l, Vec2::new(3.0, 2.0))).value.unwrap();
        let m = d.model();
        let st = crate::regen::room_state(m, &m.rooms[&room]).unwrap();
        assert!((st.area - 22.04).abs() < 1e-9 && (st.perimeter - 19.2).abs() < 1e-9 && st.finish.iter().all(|s| s.arc.is_none()));
    }
}
