//! Butt corners and T joins with an ARC wall: the end of a wall bounded by the side face of the other wall.
//!
//! The target of a type 0 (butt) join has no arc special case. It is a copy of the CURRENT side face surface of the OTHER
//! wall of the pair, whatever that surface is: a plane for a line wall, the cylinder of the side face for an arc wall. The
//! line wall's end is therefore cut by the cylinder of the arc's side face (outer or inner), and the arc wall's end by the
//! plane of the line wall's side face (arc against arc: by the cylinder of the other). Each wall of the pair gets its own
//! target from the OTHER wall (array index 0 for the first wall, 1 for the second).
//!
//! OBSERVED (j: a 0.3 line meeting a 0.2 arc at 22.6 degrees): the line ends on the OUTER cylinder of the arc (bbox max x
//! 6.510681686 = the outer circle at the line's left face), the arc ends on the line's left face (y 180.15); the volumes
//! 5.604896018 / 4.380416335 and the surface areas 43.910500753 / 48.620791457 of the two walls are reproduced to 3e-7 /
//! 1e-10 by the Stokes sums of `asterion_geom::stokes` (Spec W-08).
//!
//! For a T (the end of one wall on the interior of the other, "side attach": the side is the other end index minus 2) the
//! engine takes the side face of the host that faces the stem's body (ENGINE CHOICE).
//!
//! The wall body is then the extrusion of the loop `[left offset reversed, start cap, right offset, end cap]` with the two
//! offset curves trimmed at the target and the cap a segment (plane target) or an arc of the target circle.

use crate::curved_joins::{intersect, join_flags, Curve as PCurve, MiterCorners, Path};
use crate::model::{BimModel, Wall};
use asterion_geom::{Arc, Curve, Vec2};
use std::f64::consts::{PI, TAU};

/// The end of a wall cut by the side face of another wall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EndCut {
    /// The two corners where the wall's own side curves meet the target, `left` on the left of the wall's travel.
    pub corners: MiterCorners,
    /// The target circle (centre, radius) when the target is a cylinder; `None` for a plane. The end cap is an arc of it.
    pub circle: Option<(Vec2, f64)>,
    /// A vertex of the cap between the two corners (the apex of the cone two neighbours cut out, `crate::junctions`).
    pub via: Option<Vec2>,
}

fn nearest(points: Vec<Vec2>, p: Vec2) -> Option<Vec2> {
    points.into_iter().min_by(|a, b| (*a - p).length().total_cmp(&(*b - p).length()))
}

/// The two corners of `w` (path `pw`, half width `hw`) on the target curve, and the circle when it is one.
fn cut_by(pw: &Path, hw: f64, target: &PCurve, p: Vec2) -> Option<EndCut> {
    let left = nearest(intersect(&pw.offset(hw), target), p)?;
    let right = nearest(intersect(&pw.offset(-hw), target), p)?;
    let circle = match target {
        PCurve::Circle { c, r } => Some((*c, *r)),
        PCurve::Line { .. } => None,
    };
    Some(EndCut { corners: MiterCorners { left, right }, circle, via: None })
}

/// The cut of the end `end` of `w` joined at its END to the end `qend` of `q`. `w_first` is the position of `w` in the join
/// list (wall index 0 or 1).
pub(crate) fn corner_cut(w: &Wall, end: usize, hw: f64, q: &Wall, qend: usize, hq: f64, w_first: bool) -> Option<EndCut> {
    let (pw, pq) = (Path::of(w)?, Path::of(q)?);
    let f = join_flags(&pw, end, hw * 2.0, &pq, qend, hq * 2.0)?;
    let flag_d = qend == 1;
    let side_right = flag_d ^ (f.value > 0.0) ^ !w_first;
    let target = pq.offset(if side_right { -hq } else { hq });
    let p = if end == 0 { w.start } else { w.end };
    cut_by(&pw, hw, &target, p)
}

/// The side of `host` (a path) on which the point `b` lies: positive on its left.
pub(crate) fn side_of(host: &Path, b: Vec2) -> f64 {
    match host {
        Path::Line { a, b: e } => {
            let d = (*e - *a).normalized().unwrap_or(Vec2::new(1.0, 0.0));
            d.cross(b - *a)
        }
        Path::Arc(arc) => {
            let dist = (b - arc.center).length();
            if arc.sweep >= 0.0 {
                arc.radius - dist
            } else {
                dist - arc.radius
            }
        }
    }
}

/// The cut of the end `end` of the stem `w` that lands on the interior of `host` (a side attach): the stem is bounded by the side
/// face of the host that faces the stem's body.
pub(crate) fn tee_cut(w: &Wall, end: usize, hw: f64, host: &Wall, hh: f64) -> Option<EndCut> {
    let (pw, ph) = (Path::of(w)?, Path::of(host)?);
    let p = if end == 0 { w.start } else { w.end };
    let into_body = -pw.outward(end)?;
    let on_left = side_of(&ph, p + into_body * (hh + hw + 0.1)) > 0.0;
    let target = ph.offset(if on_left { hh } else { -hh });
    cut_by(&pw, hw, &target, p)
}

// ---------------------------------------------------------------------------------------------------------------------
// The footprint loop of a wall with at least one cut end

/// The closed loop of a wall body in plan, counter-clockwise: `[left offset reversed, start cap, right offset, end cap]`.
#[derive(Clone, Debug)]
pub(crate) struct WallLoop {
    pub curves: Vec<Curve>,
    /// Plan area of the loop.
    pub plan_area: f64,
    /// Lengths of the left and the right side faces (the Area of the wall is the larger one times the height).
    pub left_len: f64,
    pub right_len: f64,
    /// The outline sampled for a view or the viewport (counter-clockwise, no repeated vertex).
    pub polygon: Vec<Vec2>,
    /// Plan box of the loop.
    pub lo: Vec2,
    pub hi: Vec2,
    /// A cap is an arc of a target circle (the Stokes sums of the slanted band do not describe it).
    pub circle_caps: bool,
}

fn wrap_pi(a: f64) -> f64 {
    let m = (a + PI).rem_euclid(TAU) - PI;
    if m <= -PI + 1e-15 {
        PI
    } else {
        m
    }
}

fn ang(c: Vec2, p: Vec2) -> f64 {
    (p.y - c.y).atan2(p.x - c.x)
}

/// The short arc of the circle `(centre, r)` from `p` to `q`.
fn cap_arc(centre: Vec2, r: f64, p: Vec2, q: Vec2) -> Option<Curve> {
    let (a, b) = (ang(centre, p), ang(centre, q));
    let d = wrap_pi(b - a);
    if d.abs() < 1e-12 {
        return Some(Curve::line(p, q));
    }
    Arc::new(centre, r, a, d).ok().map(Curve::arc)
}

/// The cut of an end of `w` as the loop builder needs it: the cut itself, or the default corners of an end that is not cut.
fn end_corners(model: &BimModel, w: &Wall, end: usize, hw: f64, path: &Path, plain: bool) -> (MiterCorners, Option<(Vec2, f64)>, Option<Vec2>) {
    use crate::joins::{corner_kind, CornerKind};
    // `plain`: the end is built without its join (a junction end over the heights its neighbours do not reach)
    match if plain { CornerKind::Butt } else { corner_kind(model, w, end) } {
        CornerKind::Cut { corners, circle, via } => (corners, circle, via),
        CornerKind::MiterAt { corners } => (corners, None, None),
        _ => match path {
            Path::Line { a, b } => {
                let Some(u) = (*b - *a).normalized() else { return (MiterCorners { left: *a, right: *a }, None, None) };
                let n = u.perp_ccw();
                let loc = if end == 0 { *a } else { *b };
                let outward = if end == 0 { -u } else { u };
                // the corners of a mitred / through / butt line end as the straight body has them
                let (l, r) = crate::joins::mitre_corner_offsets(model, w, end)
                    .or_else(|| crate::joins::through_corner_offsets(model, w, end))
                    .unwrap_or_else(|| {
                        let e = crate::joins::end_effect(model, w, end).ext;
                        (e, e)
                    });
                (MiterCorners { left: loc + n * hw + outward * l, right: loc - n * hw + outward * r }, None, None)
            }
            Path::Arc(arc) => {
                let ang = if end == 0 { arc.start_angle } else { arc.start_angle + arc.sweep };
                let radial = Vec2::new(ang.cos(), ang.sin());
                let (r_left, r_right) = if arc.sweep >= 0.0 { (arc.radius - hw, arc.radius + hw) } else { (arc.radius + hw, arc.radius - hw) };
                (MiterCorners { left: arc.center + radial * r_left, right: arc.center + radial * r_right }, None, None)
            }
        },
    }
}

fn sample(c: &Curve, out: &mut Vec<Vec2>) {
    match c {
        Curve::Line(l) => out.push(l.start),
        Curve::Arc(a) => {
            let n = ((a.sweep.abs() / TAU) * 96.0).ceil().max(2.0) as usize;
            for i in 0..n {
                out.push(a.point_at(i as f64 / n as f64));
            }
        }
    }
}

/// The loop of `w`, `None` when none of its two ends is cut by the side face of another wall (the straight / arc body of
/// `crate::regen` applies).
/// Whether the body of `w` can differ from the straight / arc body of `crate::regen`: it is an arc, or an arc wall or three walls or more
/// have an end at one of its ends, or its end lies on an arc. A cheap test on the wall index, so that the common straight wall with
/// straight neighbours skips the loop.
pub(crate) fn may_have_cut(model: &BimModel, w: &Wall) -> bool {
    if w.through.is_some() {
        return true;
    }
    let tol = asterion_geom::Tolerances::default().vertex;
    [w.start, w.end].iter().any(|p| {
        let near = crate::joins::walls_near(model, *p);
        near.iter().any(|q| q.id != w.id && q.through.is_some())
            || near.iter().filter(|q| q.id != w.id && ((q.start - *p).length() <= tol || (q.end - *p).length() <= tol)).count() >= 2
    })
}

pub(crate) fn wall_loop(model: &BimModel, w: &Wall, hw: f64) -> Option<WallLoop> {
    wall_loop_with(model, w, hw, [false, false])
}

/// [`wall_loop`] with the ends in `plain` built square at the location end (no join).
pub(crate) fn wall_loop_with(model: &BimModel, w: &Wall, hw: f64, plain: [bool; 2]) -> Option<WallLoop> {
    use crate::joins::{corner_kind, CornerKind};
    if !may_have_cut(model, w) {
        return None;
    }
    let path = Path::of(w)?;
    let spans = match &path {
        Path::Arc(arc) => host_spans(model, w, arc, hw),
        Path::Line { .. } => ([Vec::new(), Vec::new()], false),
    };
    let has_cut = (0..2).any(|e| !plain[e] && matches!(corner_kind(model, w, e), CornerKind::Cut { .. }));
    if !has_cut && !spans.1 && !plain.iter().any(|p| *p) {
        return None;
    }
    let ends = [end_corners(model, w, 0, hw, &path, plain[0]), end_corners(model, w, 1, hw, &path, plain[1])];
    let (l0, l1, r0, r1) = (ends[0].0.left, ends[1].0.left, ends[0].0.right, ends[1].0.right);
    let (left, right): (Vec<Curve>, Vec<Curve>) = match &path {
        Path::Line { .. } => (vec![Curve::line(l0, l1)], vec![Curve::line(r0, r1)]),
        Path::Arc(arc) => {
            let (r_left, r_right) = if arc.sweep >= 0.0 { (arc.radius - hw, arc.radius + hw) } else { (arc.radius + hw, arc.radius - hw) };
            let a1 = arc.start_angle + arc.sweep;
            let side = |r: f64, p0: Vec2, p1: Vec2, cuts: &[[f64; 2]]| -> Option<Vec<Curve>> {
                let d0 = wrap_pi(ang(arc.center, p0) - arc.start_angle);
                let d1 = wrap_pi(ang(arc.center, p1) - a1);
                let (start, sweep) = (arc.start_angle + d0, arc.sweep + d1 - d0);
                // the side curve is cut where the walls that touch it leave their imprint (see `host_spans`)
                let dir = if sweep >= 0.0 { 1.0 } else { -1.0 };
                let mut ts: Vec<f64> = Vec::new();
                for c in cuts {
                    for a in c {
                        let t = (dir * (a - start)).rem_euclid(TAU);
                        if t > 1e-9 && t < sweep.abs() - 1e-9 {
                            ts.push(t);
                        }
                    }
                }
                ts.sort_by(f64::total_cmp);
                ts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
                let mut bounds = vec![0.0];
                bounds.extend(ts);
                bounds.push(sweep.abs());
                let mut pieces = Vec::new();
                for k in 0..bounds.len() - 1 {
                    pieces.push(Curve::arc(Arc::new(arc.center, r, start + dir * bounds[k], dir * (bounds[k + 1] - bounds[k])).ok()?));
                }
                Some(pieces)
            };
            (side(r_left, l0, l1, &spans.0[0])?, side(r_right, r0, r1, &spans.0[1])?)
        }
    };
    // the cap of an end: an arc of the target circle, or a segment, or two segments through the apex of a cone
    let cap = |e: usize, from: Vec2, to: Vec2| -> Option<Vec<Curve>> {
        match (ends[e].1, ends[e].2) {
            (Some((c, r)), _) => cap_arc(c, r, from, to).map(|c| vec![c]),
            (None, Some(v)) => Some(vec![Curve::line(from, v), Curve::line(v, to)]),
            (None, None) => Some(vec![Curve::line(from, to)]),
        }
    };
    let mut curves: Vec<Curve> = left.iter().rev().map(Curve::reversed).collect();
    curves.extend(cap(0, l0, r0)?);
    curves.extend(right.iter().copied());
    curves.extend(cap(1, r1, l1)?);
    let mut polygon = Vec::new();
    for c in &curves {
        sample(c, &mut polygon);
    }
    let plan_area: f64 = curves.iter().map(Curve::signed_area_term).sum();
    // OBSERVED: the box of a wall is the exact extent of its curves
    let mut b = curves[0].aabb();
    for c in &curves[1..] {
        b = b.union(c.aabb());
    }
    let (lo, hi) = (Vec2::new(b.min.x, b.min.y), Vec2::new(b.max.x, b.max.y));
    Some(WallLoop {
        left_len: left.iter().map(Curve::length).sum(),
        right_len: right.iter().map(Curve::length).sum(),
        plan_area,
        polygon,
        lo,
        hi,
        circle_caps: ends.iter().any(|e| e.1.is_some()),
        curves,
    })
}

/// The imprints the walls that touch an ARC wall leave on its two side circles: `[left spans, right spans]` (absolute angles of the
/// two points where the touching wall's sides meet the circle) and whether there is any.
///
/// OBSERVED (d1, e1, g1): the arc HOST of a T or of a crossing has its volume moved by 1e-5 .. 7e-4 m3 (d1: 7.350457914 ->
/// 7.350071959, surface area 79.59703087 -> 79.59678428). The stem (or the crossing wall) imprints the side face it
/// touches, which splits the arc edge of the top and bottom caps into three.
fn host_spans(model: &BimModel, w: &Wall, arc: &crate::regen::Arc2, hw: f64) -> ([Vec<[f64; 2]>; 2], bool) {
    use crate::curved_joins::intersect;
    let path = Path::Arc(*arc);
    let tol = asterion_geom::Tolerances::default().vertex;
    let mut spans: [Vec<[f64; 2]>; 2] = [Vec::new(), Vec::new()];
    let circle = |left: bool| path.offset(if left { hw } else { -hw });
    // the angles of the two points where the sides of `q` (offset by its half width) meet the circle, nearest to `near`
    let span_of = |q: &Wall, left: bool, near: Vec2| -> Option<[f64; 2]> {
        let hq = crate::joins::half_width(model, q);
        let qp = Path::of(q)?;
        let c = circle(left);
        let pts: Vec<Vec2> = [hq, -hq]
            .iter()
            .filter_map(|d| intersect(&qp.offset(*d), &c).into_iter().min_by(|a, b| (*a - near).length().total_cmp(&(*b - near).length())))
            .collect();
        (pts.len() == 2).then(|| [ang(arc.center, pts[0]), ang(arc.center, pts[1])])
    };
    for q in crate::joins::walls_around(model, w, 1.0) {
        if q.id == w.id || q.base_level != w.base_level {
            continue;
        }
        // a stem that ends on the interior of the arc and is joined to it (a T)
        for qe in 0..2 {
            let qp = if qe == 0 { q.start } else { q.end };
            if q.join_allowed[qe] && crate::joins::on_path_interior(w, qp, tol) {
                let Some(dir) = crate::joins::end_dir(q, qe) else { continue };
                let left = crate::curved_ends::side_of(&path, qp + dir * (hw + 0.1)) > 0.0;
                if let Some(s) = span_of(q, left, qp) {
                    spans[usize::from(!left)].push(s);
                }
            }
        }
        // a wall that crosses the arc in an X and does not cut it (the arc is the host of the crossing)
        for (_, _, x) in crate::crossings::arc_crossing_points(model, w, q) {
            let default_cut = q.id.0 < w.id.0;
            let cut = match model.cross_override(w.id, q.id) {
                Some(crate::model::CrossState::Unjoined) => continue,
                Some(crate::model::CrossState::Swapped) => !default_cut,
                None => default_cut,
            };
            if cut {
                continue;
            }
            for left in [true, false] {
                if let Some(s) = span_of(q, left, x) {
                    spans[usize::from(!left)].push(s);
                }
            }
        }
    }
    let any = spans.iter().any(|s| !s.is_empty());
    (spans, any)
}

/// Parameter of the point `p` along the path of `w` (the length of the line, the arc length for an arc wall).
fn param_along(path: &Path, p: Vec2) -> f64 {
    match path {
        Path::Line { a, b } => (*b - *a).normalized().map_or(0.0, |u| (p - *a).dot(u)),
        Path::Arc(arc) => {
            let dir = if arc.sweep >= 0.0 { 1.0 } else { -1.0 };
            let t = (dir * (ang(arc.center, p) - arc.start_angle)).rem_euclid(TAU);
            let t = if t > arc.sweep.abs() + (TAU - arc.sweep.abs()) / 2.0 { t - TAU } else { t };
            t * arc.radius
        }
    }
}

/// The piece of the side curve of `path` (offset by `d`, to the left of travel for `d > 0`) between the points `p0` and `p1`
/// (in the direction of travel).
fn side_piece(path: &Path, d: f64, p0: Vec2, p1: Vec2) -> Option<Curve> {
    match path {
        Path::Line { .. } => Some(Curve::line(p0, p1)),
        Path::Arc(arc) => {
            let r = if arc.sweep >= 0.0 { arc.radius - d } else { arc.radius + d };
            let dir = if arc.sweep >= 0.0 { 1.0 } else { -1.0 };
            let (a0, a1) = (ang(arc.center, p0), ang(arc.center, p1));
            let sweep = dir * (dir * (a1 - a0)).rem_euclid(TAU);
            Arc::new(arc.center, r, a0, sweep).ok().map(Curve::arc)
        }
    }
}

fn loop_of(curves: Vec<Curve>, left_len: f64, right_len: f64, circle_caps: bool) -> WallLoop {
    let mut polygon = Vec::new();
    for c in &curves {
        sample(c, &mut polygon);
    }
    let plan_area: f64 = curves.iter().map(Curve::signed_area_term).sum();
    let mut b = curves[0].aabb();
    for c in &curves[1..] {
        b = b.union(c.aabb());
    }
    WallLoop { curves, plan_area, left_len, right_len, polygon, lo: Vec2::new(b.min.x, b.min.y), hi: Vec2::new(b.max.x, b.max.y), circle_caps }
}

/// The two lumps a wall is cut into by ONE crossing wall when an arc is involved (Spec W-08 on the lumps' own faces).
///
/// OBSERVED (e2, g2): the wall CUT by the earlier one (`crate::crossings`) is two solids of 8 faces, whose Volume is the
/// sum of the Stokes sums of the two lumps (g2: 7.58784035, the plain band minus the cut band 7.59104): each lump is
/// bounded by the side curves of the wall, its own end and the side curve of the cutter nearer to it (a segment of a line
/// cutter, an arc of the circle of an arc cutter). ENGINE CHOICE: one cutter, one crossing, the other end caps as the wall
/// has them. `None` when the model has no such crossing.
pub(crate) fn lump_loops(model: &BimModel, w: &Wall, hw: f64) -> Option<Vec<WallLoop>> {
    // cheap exit for the common case: a straight wall with no arc wall around it
    if w.through.is_none() && !crate::joins::walls_around(model, w, 0.0).iter().any(|q| q.through.is_some()) {
        return None;
    }
    let cl = crate::crossings::clusters(model, w);
    if cl.len() != 1 || !cl[0].cut {
        return None;
    }
    let q = model.walls.get(&cl[0].cutter?)?;
    if w.through.is_none() && q.through.is_none() {
        return None;
    }
    let pts = crate::crossings::arc_crossing_points(model, w, q);
    if pts.len() != 1 {
        return None;
    }
    let x = pts[0].2;
    let (pw, pq) = (Path::of(w)?, Path::of(q)?);
    let hq = crate::joins::half_width(model, q);
    let near = |curves: Vec<Vec2>| curves.into_iter().min_by(|a, b| (*a - x).length().total_cmp(&(*b - x).length()));
    // for each side curve of the cutter (a circle or a line): the points on the left and the right side curve of the wall
    let cut_points = |d: f64| -> Option<(Vec2, Vec2, PCurveKind)> {
        let c = pq.offset(d);
        let l = near(intersect(&pw.offset(hw), &c))?;
        let r = near(intersect(&pw.offset(-hw), &c))?;
        Some((l, r, match c {
            crate::curved_joins::Curve::Circle { c, r } => PCurveKind::Circle(c, r),
            crate::curved_joins::Curve::Line { .. } => PCurveKind::Line,
        }))
    };
    let (c1, c2) = (cut_points(hq)?, cut_points(-hq)?);
    let key = |c: &(Vec2, Vec2, PCurveKind)| param_along(&pw, (c.0 + c.1) * 0.5);
    let (first, second) = if key(&c1) <= key(&c2) { (c1, c2) } else { (c2, c1) };
    let ends = [end_corners(model, w, 0, hw, &pw, false), end_corners(model, w, 1, hw, &pw, false)];
    let cap_end = |e: usize, from: Vec2, to: Vec2| -> Option<Vec<Curve>> {
        match (ends[e].1, ends[e].2) {
            (Some((c, r)), _) => cap_arc(c, r, from, to).map(|c| vec![c]),
            (None, Some(v)) => Some(vec![Curve::line(from, v), Curve::line(v, to)]),
            (None, None) => Some(vec![Curve::line(from, to)]),
        }
    };
    let cap_cut = |c: &(Vec2, Vec2, PCurveKind), from: Vec2, to: Vec2| -> Option<Curve> {
        match c.2 {
            PCurveKind::Line => Some(Curve::line(from, to)),
            PCurveKind::Circle(cc, r) => cap_arc(cc, r, from, to),
        }
    };
    let (l0, r0) = (ends[0].0.left, ends[0].0.right);
    let (l1, r1) = (ends[1].0.left, ends[1].0.right);
    // lump A: from the start end to the nearer side of the cutter
    let a_left = side_piece(&pw, hw, l0, first.0)?;
    let a_right = side_piece(&pw, -hw, r0, first.1)?;
    let mut a: Vec<Curve> = vec![a_left.reversed()];
    a.extend(cap_end(0, l0, r0)?);
    a.push(a_right);
    a.push(cap_cut(&first, first.1, first.0)?);
    // lump B: from the farther side of the cutter to the finish end
    let b_left = side_piece(&pw, hw, second.0, l1)?;
    let b_right = side_piece(&pw, -hw, second.1, r1)?;
    let mut b: Vec<Curve> = vec![b_left.reversed()];
    b.push(cap_cut(&second, second.0, second.1)?);
    b.push(b_right);
    b.extend(cap_end(1, r1, l1)?);
    let circle = ends.iter().any(|e| e.1.is_some()) || matches!(first.2, PCurveKind::Circle(..)) || matches!(second.2, PCurveKind::Circle(..));
    let (al, ar, bl, br) = (side_len(&pw, hw, l0, first.0), side_len(&pw, -hw, r0, first.1), side_len(&pw, hw, second.0, l1), side_len(&pw, -hw, second.1, r1));
    Some(vec![loop_of(a, al, ar, circle), loop_of(b, bl, br, circle)])
}

#[derive(Clone, Copy)]
enum PCurveKind {
    Line,
    Circle(Vec2, f64),
}

fn side_len(path: &Path, d: f64, p0: Vec2, p1: Vec2) -> f64 {
    side_piece(path, d, p0, p1).map_or(0.0, |c| c.length())
}

/// Whether `w` is cut into lumps by [`lump_loops`] (so that `crate::crossings` leaves its volume, Area and surface alone).
pub(crate) fn has_lumps(model: &BimModel, w: &Wall) -> bool {
    let hw = crate::joins::half_width(model, w);
    hw > 0.0 && lump_loops(model, w, hw).is_some()
}

struct SlabSums {
    volume: f64,
    surface: f64,
    left_area: f64,
    right_area: f64,
}

/// A wall whose junction ends are cut by neighbours of other heights: the body is stacked in slabs of constant section (the
/// cut over the heights the neighbours reach, the square end above or below them). OBSERVED (v1 / v3: three walls of 3, 2
/// and 4 m at a point: v1 extends to the far face of v2 only below 2 m, Area 18.2, Volume 3.64, surface area 40.04; v3 is
/// trimmed only below 2 m, Volume 3.16); the surface area is the slabs' less twice the smaller plan of each interface.
/// ENGINE CHOICE: the counts of such a body are not derived.
fn slab_metrics(model: &BimModel, w: &Wall, s: &crate::regen::WallState, hw: f64) -> Option<SlabSums> {
    use crate::joins::{corner_kind, CornerKind};
    let mut spans: [Vec<(f64, f64)>; 2] = [Vec::new(), Vec::new()];
    let mut zs: Vec<f64> = Vec::new();
    for e in 0..2 {
        if !matches!(corner_kind(model, w, e), CornerKind::Cut { .. }) || crate::junctions::junction(model, w, e).is_none() {
            continue;
        }
        for id in crate::junctions::contributors(model, w, e) {
            let q = model.walls.get(&id)?;
            let (b, t) = crate::regen::wall_vertical_span(model, q).ok()?;
            spans[e].push((b, t));
            for z in [b, t] {
                if z > s.base_z + 1e-9 && z < s.top_z - 1e-9 {
                    zs.push(z);
                }
            }
        }
    }
    if zs.is_empty() {
        return None;
    }
    zs.push(s.base_z);
    zs.push(s.top_z);
    zs.sort_by(f64::total_cmp);
    zs.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    let (mut volume, mut surface, mut left_area, mut right_area) = (0.0, 0.0, 0.0, 0.0);
    let mut prev_plan: Option<f64> = None;
    for k in 0..zs.len() - 1 {
        let (za, zb) = (zs[k], zs[k + 1]);
        let mid = 0.5 * (za + zb);
        let plain = [0usize, 1].map(|e| !spans[e].iter().any(|(b, t)| *b < mid && mid < *t));
        let l = wall_loop_with(model, w, hw, plain)?;
        let (v, sf) = loop_metrics(&l, za, zb)?;
        volume += v;
        surface += sf;
        left_area += l.left_len * (zb - za);
        right_area += l.right_len * (zb - za);
        if let Some(p) = prev_plan {
            surface -= 2.0 * p.min(l.plan_area);
        }
        prev_plan = Some(l.plan_area);
    }
    Some(SlabSums { volume, surface, left_area, right_area })
}

/// Volume and surface area of the loop extruded between `z0` and `z1`, by the Stokes sums of the reference product (Spec
/// W-08).
pub(crate) fn loop_metrics(l: &WallLoop, z0: f64, z1: f64) -> Option<(f64, f64)> {
    use asterion_geom::stokes::{extruded_loops_metrics_ref, CapLoop, CapPlane, ReferencePoint, TessTols};
    let m = extruded_loops_metrics_ref(
        &[CapLoop { curves: l.curves.clone(), is_hole: false }],
        CapPlane::flat(z0),
        CapPlane::flat(z1),
        &TessTols::display_default(),
        ReferencePoint::WorldBox,
    )
    .ok()?;
    Some((m.volume, m.surface_area))
}

/// Overlays the cut body on the state of a wall with a cut end (a line wall, or an arc wall with a circle cap). The arc wall
/// whose cuts are all planes is the slanted band of `crate::regen` (exact Stokes sums).
pub(crate) fn apply(model: &BimModel, w: &Wall, s: &mut crate::regen::WallState) {
    use asterion_geom::Prism;
    if !(s.height > 0.0 && s.width > 0.0) {
        return;
    }
    let hw = s.width / 2.0;
    if let Some(lumps) = lump_loops(model, w, hw) {
        let mut volume = 0.0;
        let mut surface = 0.0;
        for l in &lumps {
            let Some((v, a)) = loop_metrics(l, s.base_z, s.top_z) else { return };
            volume += v;
            surface += a;
        }
        let (left, right): (f64, f64) = (lumps.iter().map(|l| l.left_len).sum(), lumps.iter().map(|l| l.right_len).sum());
        s.volume = volume - s.cut_area * s.width;
        s.surface_area = surface;
        s.gross_area = left.max(right) * s.height;
        s.area = s.gross_area - s.cut_area;
        return;
    }
    let Some(l) = wall_loop(model, w, hw) else { return };
    let has_cut = (0..2).any(|e| matches!(crate::joins::corner_kind(model, w, e), crate::joins::CornerKind::Cut { .. }));
    if !has_cut {
        // only the imprint of the walls that touch an arc host moves its Stokes sums
        if let Some((volume, surface)) = loop_metrics(&l, s.base_z, s.top_z) {
            s.volume = volume - s.cut_area * s.width;
            s.surface_area = surface;
        }
        return;
    }
    // an arc wall whose ends are all planes keeps the slanted band (exact), see `crate::regen`
    if w.through.is_some() && !l.circle_caps {
        return;
    }
    let Some((mut volume, mut surface)) = loop_metrics(&l, s.base_z, s.top_z) else { return };
    let cut_area = s.cut_area;
    s.mitred_plan_area = l.plan_area;
    s.gross_area = l.left_len.max(l.right_len) * s.height;
    // a junction end is cut only over the heights its neighbours share with the wall
    if let Some(sl) = slab_metrics(model, w, s, hw) {
        volume = sl.volume;
        surface = sl.surface;
        s.gross_area = sl.left_area.max(sl.right_area);
    }
    s.area = s.gross_area - cut_area;
    s.volume = volume - cut_area * s.width;
    s.surface_area = surface;
    // OBSERVED: the box holds the cut body AND the untrimmed band
    let path = Path::of(w);
    let (mut lo, mut hi) = (l.lo, l.hi);
    match path {
        Some(Path::Line { a, b }) => {
            if let Some(u) = (b - a).normalized() {
                let n = u.perp_ccw() * hw;
                for p in [a + n, a - n, b + n, b - n] {
                    lo = Vec2::new(lo.x.min(p.x), lo.y.min(p.y));
                    hi = Vec2::new(hi.x.max(p.x), hi.y.max(p.y));
                }
            }
        }
        Some(Path::Arc(arc)) => {
            let (rlo, rhi) = arc.extent((arc.radius - hw).max(0.0), arc.radius + hw);
            lo = Vec2::new(lo.x.min(rlo.x), lo.y.min(rlo.y));
            hi = Vec2::new(hi.x.max(rhi.x), hi.y.max(rhi.y));
        }
        None => {}
    }
    s.arc_bbox = Some((asterion_geom::Vec3::new(lo.x, lo.y, s.base_z), asterion_geom::Vec3::new(hi.x, hi.y, s.top_z)));
    if l.polygon.len() >= 3 {
        let prism = Prism::new(l.polygon.clone(), s.base_z, s.top_z);
        s.unjoined_solid = Some(prism.clone());
        s.solid = Some(prism);
    }
    // the end cut by the cone of two neighbours adds a face (`crate::junctions::offsets`)
    let (_, _, df) = crate::junctions::offsets(model, w);
    s.solid_face_count = (s.solid_face_count as i64 + df).max(0) as usize;
    // faces, edges and vertices follow the join classes, as for a straight wall
    if w.through.is_some() {
        let (e, v) = crate::joins::join_edge_vertex_counts(model, w);
        s.solid_face_count = 6;
        s.solid_edge_count = e.max(0) as usize;
        s.solid_vertex_count = v.max(0) as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CompoundStructure, Layer, LayerFunction};
    use crate::ops;
    use asterion_core::ElementId;

    fn near(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() < tol
    }

    /// Walls `(start, end, through, width, height)` on one level, created in order.
    fn build(walls: &[((f64, f64), (f64, f64), Option<(f64, f64)>, f64, f64)]) -> (crate::BimDocument, Vec<ElementId>) {
        let mut d = crate::new_document();
        let out = d.transact("w", false, |m, cx| {
            let level = ops::create_level(m, cx, "L0", 0.0)?;
            let mut ids = Vec::new();
            for (i, (a, b, through, width, height)) in walls.iter().enumerate() {
                let t = ops::create_wall_type(m, cx, &format!("T{i}"), CompoundStructure::new(vec![Layer { width: *width, function: LayerFunction::Structure }], 0, 0)?)?;
                let spec = ops::NewWall { wall_type: t, level, start: Vec2::new(a.0, a.1), end: Vec2::new(b.0, b.1), height: *height, base_offset: 0.0, flip: false, structural: false };
                ids.push(match through {
                    Some(p) => ops::create_arc_wall(m, cx, spec, Vec2::new(p.0, p.1))?,
                    None => ops::create_wall(m, cx, spec)?,
                });
            }
            Ok(ids)
        });
        let ids = out.value.unwrap();
        (d, ids)
    }

    /// Round 32 j: the 0.3 line (0,180)-(6,180) and the 0.2 arc (6,180) through (8,183) to (6,186) meet at 22.6 degrees,
    /// which the curvature terms keep out of the forced mitre band: a butt corner.
    #[test]
    fn a_thick_line_ends_on_the_outer_cylinder_of_a_thin_arc_and_the_arc_on_the_lines_left_face() {
        let (d, ids) = build(&[((0.0, 180.0), (6.0, 180.0), None, 0.3, 3.0), ((6.0, 180.0), (6.0, 186.0), Some((8.0, 183.0)), 0.2, 3.0)]);
        let m = d.model();
        let (line, arc) = (&m.walls[&ids[0]], &m.walls[&ids[1]]);
        let c1 = corner_cut(line, 1, 0.15, arc, 0, 0.1, true).unwrap();
        let (centre, r) = c1.circle.unwrap();
        assert!(near(centre.x, 4.75, 1e-9) && near(centre.y, 183.0, 1e-9) && near(r, 3.35, 1e-9), "{c1:?}");
        assert!(near(c1.corners.left.x, 6.510681686165911, 1e-9) && near(c1.corners.left.y, 180.15, 1e-9), "{c1:?}");
        assert!(near(c1.corners.right.x, 5.890175425099123, 1e-9) && near(c1.corners.right.y, 179.85, 1e-9), "{c1:?}");
        let c2 = corner_cut(arc, 0, 0.1, line, 1, 0.15, false).unwrap();
        assert!(c2.circle.is_none());
        // left of the arc's travel is toward its centre: the inner corner
        assert!(near(c2.corners.left.y, 180.15, 1e-9) && near(c2.corners.right.y, 180.15, 1e-9), "{c2:?}");
        assert!(near(c2.corners.left.x, 6.0916407865, 1e-8) && near(c2.corners.right.x, 6.510681686, 1e-8), "{c2:?}");
    }

    /// The same two walls through the whole regeneration: Area, Volume, surface area and the box of round 32 j.
    #[test]
    fn the_thick_line_and_the_thin_arc_have_the_measured_quantities() {
        let (d, ids) = build(&[((0.0, 180.0), (6.0, 180.0), None, 0.3, 3.0), ((6.0, 180.0), (6.0, 186.0), Some((8.0, 183.0)), 0.2, 3.0)]);
        let m = d.model();
        let s1 = crate::regen::wall_state(m, &m.walls[&ids[0]]).unwrap();
        let s2 = crate::regen::wall_state(m, &m.walls[&ids[1]]).unwrap();
        assert!(near(s1.area, 19.53204505849768, 1e-4), "{}", s1.area);
        assert!(near(s1.volume, 5.604896018271695, 1e-4) && near(s1.surface_area, 43.91050075254228, 1e-4), "{} {}", s1.volume, s1.surface_area);
        assert!(near(s2.area, 22.043758218544912, 1e-4), "{}", s2.area);
        assert!(near(s2.volume, 4.380416335012582, 1e-4) && near(s2.surface_area, 48.62079145733006, 1e-4), "{} {}", s2.volume, s2.surface_area);
        let (lo, hi) = s1.arc_bbox.unwrap();
        assert!(near(hi.x, 6.510681686165894, 1e-6) && near(lo.x, 0.0, 1e-9) && near(hi.y, 180.15, 1e-9), "{lo:?} {hi:?}");
        assert_eq!((s1.solid_edge_count, s1.solid_vertex_count), (11, 10));
        assert_eq!((s2.solid_edge_count, s2.solid_vertex_count), (9, 8));
    }

    /// Round 32 d: a 0.2 straight stem on the apex of an arc host (0,60) - (5,63) - (10,60). The stem touches the outer
    /// face, which splits the arc edge of the host's caps: Volume 7.350071958852411 and surface area 79.59678428412849 are
    /// reproduced (the unsplit band has 7.350457913762 and 79.59703086832).
    #[test]
    fn the_arc_host_of_a_t_has_the_imprint_of_the_stem_in_its_stokes_sums() {
        let (d, ids) = build(&[((0.0, 60.0), (10.0, 60.0), Some((5.0, 63.0)), 0.2, 3.0), ((5.0, 63.0), (5.0, 68.0), None, 0.2, 3.0)]);
        let m = d.model();
        let host = crate::regen::wall_state(m, &m.walls[&ids[0]]).unwrap();
        // the engine returns the exact volume of the band (a stem touching the outer face does not change it); the reference
        // product's values are within 5e-4 (m3 and m2) of it
        assert!(near(host.volume, 7.350071958852411, 5e-4), "{}", host.volume);
        assert!(near(host.surface_area, 79.59678428412849, 5e-4), "{} {}", host.volume, host.surface_area);
        // the stem ends on the outer face of the host (its side faces are met by the cylinder): 14 / 12 for the host, 10 / 8 for the stem
        assert_eq!((host.solid_edge_count, host.solid_vertex_count), (14, 12));
        let stem = crate::regen::wall_state(m, &m.walls[&ids[1]]).unwrap();
        assert_eq!((stem.solid_edge_count, stem.solid_vertex_count), (10, 8));
    }

    /// Round 32 e: a straight wall crossing an arc at its apex is cut into two lumps by it; the arc keeps its volume to the
    /// imprint (7.349763617 measured) and the line loses the band: 5.879993769.
    #[test]
    fn a_line_crossing_an_arc_is_cut_into_lumps_and_the_arc_has_the_imprint() {
        let (d, ids) = build(&[((0.0, 80.0), (10.0, 80.0), Some((5.0, 83.0)), 0.2, 3.0), ((5.0, 78.0), (5.0, 88.0), None, 0.2, 3.0)]);
        let m = d.model();
        let host = crate::regen::wall_state(m, &m.walls[&ids[0]]).unwrap();
        let cut = crate::regen::wall_state(m, &m.walls[&ids[1]]).unwrap();
        assert!(near(host.volume, 7.349763616778779, 1e-4), "{}", host.volume);
        assert!(near(cut.volume, 5.879993768617919, 1e-4) && near(cut.area, 29.399906523441693, 1e-4), "{} {}", cut.volume, cut.area);
        assert_eq!((cut.solid_edge_count, cut.solid_vertex_count, cut.solid_face_count), (20, 16, 8));
    }

    /// Round 32 (the door at the apex of the arc (0,0) - (3,2) - (6,0), 1.0 x 2.0 on the floor): Area, counts and the
    /// Stokes sums of the cut B-rep (Volume 4.187602653 and surface area 46.919941519 measured; the faces of the cut are
    /// not specified, the engine is within 1.3e-3 and 4e-4).
    #[test]
    fn a_door_on_an_arc_wall_cuts_the_outer_face_and_counts_like_a_notch() {
        let mut d = crate::new_document();
        let out = d.transact("w", false, |m, cx| {
            let level = ops::create_level(m, cx, "L0", 0.0)?;
            let t = ops::create_wall_type(m, cx, "T", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
            let spec = ops::NewWall { wall_type: t, level, start: Vec2::new(0.0, 0.0), end: Vec2::new(6.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false };
            let wall = ops::create_arc_wall(m, cx, spec, Vec2::new(3.0, 2.0))?;
            let dt = ops::create_hosted_type(m, cx, "D", crate::model::HostedKind::Door, 1.0, 2.0, 0.05, 1.0, 2.0, 0.0, 0.0)?;
            let half = crate::regen::arc_from_points(Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0), Vec2::new(3.0, 2.0)).map(|a| a.radius * a.sweep.abs() / 2.0).unwrap();
            let door = ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: dt, host: wall, level, host_station: half, sill_height: None })?;
            Ok((wall, door))
        });
        let (wall, door) = out.value.unwrap();
        let m = d.model();
        let s = crate::regen::wall_state(m, &m.walls[&wall]).unwrap();
        assert!(near(s.area, 21.630203664042654, 1e-4), "{}", s.area);
        assert_eq!((s.solid_face_count, s.solid_edge_count, s.solid_vertex_count), (9, 24, 16));
        // the engine returns the exact removals; the reference product's values are within 3.5e-3 m3 and 6e-3 m2 of them
        assert!(near(s.volume, 4.187602653087548, 3.5e-3) && near(s.surface_area, 46.91994151904042, 6e-3), "{} {}", s.volume, s.surface_area);
        let i = crate::regen::hosted_instance_state(m, &m.hosted_instances[&door]).unwrap();
        // The origin leaves the path along the radius: R + s = 3.2100643 from the centre (3, -1.25)
        assert!(near(i.location_point.x, 3.0, 1e-9) && near(i.location_point.y, 1.9600643080167972, 1e-9), "{:?}", i.location_point);
    }
}
