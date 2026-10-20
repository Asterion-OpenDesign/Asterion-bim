//! Three or more straight walls with an end at one point: the first two walls of the join array take the pair rule, every later
//! wall is bounded by the side faces of its two geometric neighbours.
//!
//! * The join array of an auto-join is the order of attachment, the first pair is `[target, attached]` and every further
//!   wall is appended; the engine's order of attachment is the creation order (ascending id). A wall whose end does not
//!   allow the join is not in the array (OBSERVED r).
//! * Spec W-05a: the walls at array positions 0 and 1 are built against each other (a collinear pair, builds no target for
//!   either: OBSERVED, `q1` and `q2` of the plus created north / south / east / west keep their unjoined volume).
//! * The wall at position `k >= 2` looks at the entries before it; each gets the angle from the wall's own tangent to its
//!   tangent (both pointing into their bodies), the neighbour A is the record with the LARGEST angle and B the one with the
//!   SMALLEST (an angle within 0.1 degrees of zero loses); ties: the wider wall, then the lower element id.
//! * A neighbour is excluded when its tangent makes an angle of at most 5 degrees or at least 175 degrees with the wall's,
//!   or when its offset curve overlaps that of A; each neighbour's path is offset by half the width of ITS OWN wall. With
//!   ONE contributing neighbour the target is its side face (a plane), with TWO the shell bounded by both offset curves
//!   from their intersection outward: the end of the wall is the cone of that shell cut out of its body.

use crate::curved_ends::EndCut;
use crate::curved_joins::MiterCorners;
use crate::model::{BimModel, Wall};
use asterion_core::ElementId;
use asterion_geom::{Tolerances, Vec2};
use std::f64::consts::{PI, TAU};

/// The walls with an end at one point, in the order of the join array.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Junction {
    pub members: Vec<(ElementId, usize)>,
}

/// The junction `w`'s end `end` belongs to: three or more straight walls of one base level with an end at the point, the join allowed
/// at each of those ends, and no wall passing through the point (that is a T, handled apart). `None` for any other end.
pub(crate) fn junction(model: &BimModel, w: &Wall, end: usize) -> Option<Junction> {
    if end > 1 || w.through.is_some() || !w.join_allowed[end] {
        return None;
    }
    let tol = Tolerances::default().vertex;
    let p = if end == 0 { w.start } else { w.end };
    let mut members: Vec<(ElementId, usize)> = Vec::new();
    for q in crate::joins::walls_coexisting(model, w, p) {
        if q.id != w.id && crate::joins::on_path_interior(q, p, tol) {
            return None;
        }
        if q.base_level != w.base_level {
            continue;
        }
        let qe = (0..2).find(|e| ((if *e == 0 { q.start } else { q.end }) - p).length() <= tol);
        if let Some(qe) = qe {
            if q.through.is_some() {
                return None;
            }
            if q.join_allowed[qe] {
                members.push((q.id, qe));
            }
        }
    }
    members.sort_by_key(|(id, _)| id.0);
    members.dedup_by_key(|(id, _)| id.0);
    if members.len() < 3 || !members.iter().any(|(id, _)| *id == w.id) {
        return None;
    }
    // walls that run the SAME way from the point overlap each other (a copy at a zero translation): not a join
    let dirs: Vec<Vec2> = members.iter().filter_map(|(id, e)| dir(model, *id, *e)).collect();
    for i in 0..dirs.len() {
        for k in (i + 1)..dirs.len() {
            if dirs[i].cross(dirs[k]).abs() <= 0.0017453292519943279 && dirs[i].dot(dirs[k]) > 0.0 {
                return None;
            }
        }
    }
    Some(Junction { members })
}

fn dir(model: &BimModel, id: ElementId, end: usize) -> Option<Vec2> {
    crate::joins::end_dir(model.walls.get(&id)?, end)
}

/// Angle counter-clockwise from `a` to `b`, in `[0, 2 pi)` (values above 6.28144 read as 0).
fn ccw_angle(a: Vec2, b: Vec2) -> f64 {
    let v = (b.y.atan2(b.x) - a.y.atan2(a.x)).rem_euclid(TAU);
    if v > 6.28144 {
        0.0
    } else {
        v
    }
}

/// The offset curve of the neighbour `q` (its end `qe`): the side facing the wall whose body direction is `t_w`, as (point, direction).
fn facing_offset(model: &BimModel, q: &Wall, qe: usize, t_w: Vec2, p: Vec2) -> Option<(Vec2, Vec2)> {
    let t_q = crate::joins::end_dir(q, qe)?;
    let hq = crate::joins::half_width(model, q);
    let n_q = t_q.perp_ccw();
    let s = if t_q.cross(t_w) >= 0.0 { 1.0 } else { -1.0 };
    Some((p + n_q * (s * hq), t_q))
}

fn line_intersection(p1: Vec2, d1: Vec2, p2: Vec2, d2: Vec2) -> Option<Vec2> {
    let den = d1.cross(d2);
    if den.abs() < 1e-12 {
        return None;
    }
    Some(p1 + d1 * ((p2 - p1).cross(d2) / den))
}

/// The cut of the end `end` of the wall `w` in its junction (`None` when the end builds no target or the geometry is not decided).
pub(crate) fn cut(model: &BimModel, w: &Wall, end: usize) -> Option<EndCut> {
    cut_with_contributors(model, w, end).map(|c| c.0)
}

/// The walls whose side faces bound the end `end` of `w` in its junction (the other of the first pair, or the neighbours A
/// and B that contribute to a later wall): the end is cut only over the heights they share with `w` (OBSERVED v).
pub(crate) fn contributors(model: &BimModel, w: &Wall, end: usize) -> Vec<ElementId> {
    cut_with_contributors(model, w, end).map(|c| c.1).unwrap_or_default()
}

fn cut_with_contributors(model: &BimModel, w: &Wall, end: usize) -> Option<(EndCut, Vec<ElementId>)> {
    let j = junction(model, w, end)?;
    let idx = j.members.iter().position(|(id, _)| *id == w.id)?;
    let hw = crate::joins::half_width(model, w);
    let p = if end == 0 { w.start } else { w.end };
    if idx < 2 {
        // the pair rule against the other of the first two entries
        let (qid, qe) = j.members[1 - idx];
        let q = model.walls.get(&qid)?;
        return pair_cut(model, w, end, hw, q, qe, idx == 0).map(|c| (c, vec![qid]));
    }
    later_cut(model, w, end, hw, p, &j, idx)
}

fn pair_cut(model: &BimModel, w: &Wall, end: usize, hw: f64, q: &Wall, qe: usize, w_first: bool) -> Option<EndCut> {
    let hq = crate::joins::half_width(model, q);
    let (dw, dq) = (crate::joins::end_dir(w, end)?, crate::joins::end_dir(q, qe)?);
    let (collinear, forced) = crate::join_end::corner_flags(dw, dq);
    if collinear {
        return None;
    }
    let stored = if w_first { model.join_type_at(w.id, end) } else { model.join_type_at(q.id, qe) };
    use crate::curved_joins as cj;
    use crate::model::WallJoinKind as K;
    if forced || stored == K::Miter {
        let (pa, pb) = (cj::Path::of(w)?, cj::Path::of(q)?);
        let p = if end == 0 { w.start } else { w.end };
        let corners = cj::mitre_corners(&pa, end, hw, &pb, qe, hq, p)?;
        return Some(EndCut { corners, circle: None, via: None });
    }
    if stored == K::SquareOff && w_first {
        return None;
    }
    crate::curved_ends::corner_cut(w, end, hw, q, qe, hq, w_first)
}

fn later_cut(model: &BimModel, w: &Wall, end: usize, hw: f64, p: Vec2, j: &Junction, idx: usize) -> Option<(EndCut, Vec<ElementId>)> {
    let t_w = crate::joins::end_dir(w, end)?;
    // the records of the entries before the wall
    struct Rec {
        id: ElementId,
        end: usize,
        angle: f64,
        width: f64,
    }
    let mut recs: Vec<Rec> = Vec::new();
    for (qid, qe) in &j.members[..idx] {
        let q = model.walls.get(qid)?;
        let t_q = dir(model, *qid, *qe)?;
        recs.push(Rec { id: *qid, end: *qe, angle: ccw_angle(t_w, t_q), width: crate::joins::half_width(model, q) * 2.0 });
    }
    if recs.len() < 2 {
        return None;
    }
    let tol = Tolerances::default().vertex;
    let zero = 0.0017453292519943279;
    // an angle within 0.1 degrees of zero loses against any record with a non-zero angle; angles closer than 0.1 degrees tie
    let better = |a: &Rec, b: &Rec, largest: bool| -> bool {
        if (a.angle <= zero) != (b.angle <= zero) {
            return a.angle > zero;
        }
        if (a.angle - b.angle).abs() >= zero {
            return if largest { a.angle > b.angle } else { a.angle < b.angle };
        }
        if (a.width - b.width).abs() > tol {
            return a.width > b.width;
        }
        a.id.0 < b.id.0
    };
    let mut a_i = 0;
    let mut b_i = 0;
    for i in 1..recs.len() {
        if better(&recs[i], &recs[a_i], true) {
            a_i = i;
        }
        if better(&recs[i], &recs[b_i], false) {
            b_i = i;
        }
    }
    if a_i == b_i {
        return None;
    }
    let (ra, rb) = (&recs[a_i], &recs[b_i]);
    // exclusion by angle ((2a), versions >= 1: 5 and 5 degrees)
    let lim = 5.0_f64.to_radians();
    let angle_ok = |r: &Rec| -> bool {
        let t_q = dir(model, r.id, r.end).unwrap_or(t_w);
        let x = t_w.dot(t_q).clamp(-1.0, 1.0).acos();
        x > lim && x < PI - lim
    };
    let (mut use_a, mut use_b) = (angle_ok(ra), angle_ok(rb));
    let (qa, qb) = (model.walls.get(&ra.id)?, model.walls.get(&rb.id)?);
    let oa = facing_offset(model, qa, ra.end, t_w, p)?;
    let ob = facing_offset(model, qb, rb.end, t_w, p)?;
    // exclusion by overlap ((2c)): the offset curves of A and B do not meet and coincide
    if use_a && use_b && oa.1.cross(ob.1).abs() < 1e-9 && (ob.0 - oa.0).cross(oa.1).abs() < tol {
        use_b = false;
    }
    let n_w = (w.end - w.start).normalized()?.perp_ccw();
    let (l0, r0) = (w.start + n_w * hw, w.start - n_w * hw);
    let side_pt = |left: bool| if left { l0 } else { r0 };
    let d_w = (w.end - w.start).normalized()?;
    let nearest = |pts: Vec<Vec2>| pts.into_iter().min_by(|x, y| (*x - p).length().total_cmp(&(*y - p).length()));
    match (use_a, use_b) {
        (false, false) => None,
        (true, false) | (false, true) => {
            let o = if use_a { oa } else { ob };
            let left = nearest(line_intersection(side_pt(true), d_w, o.0, o.1).into_iter().collect())?;
            let right = nearest(line_intersection(side_pt(false), d_w, o.0, o.1).into_iter().collect())?;
            Some((EndCut { corners: MiterCorners { left, right }, circle: None, via: None }, vec![if use_a { ra.id } else { rb.id }]))
        }
        (true, true) => {
            let apex = line_intersection(oa.0, oa.1, ob.0, ob.1)?;
            // the cone's boundary: the two rays from the apex along the neighbours
            let big = 1.0e3;
            let rays = [(apex, apex + oa.1 * big), (apex, apex + ob.1 * big)];
            let cross_ray = |side: bool| -> Option<Vec2> {
                let mut found = Vec::new();
                for (s, e) in rays {
                    let d = e - s;
                    if let Some(x) = line_intersection(side_pt(side), d_w, s, d) {
                        let t = (x - s).dot(d) / d.dot(d);
                        if (-1e-9..=1.0 + 1e-9).contains(&t) {
                            found.push(x);
                        }
                    }
                }
                nearest(found)
            };
            let (left, right) = (cross_ray(true)?, cross_ray(false)?);
            // the apex is a vertex of the cap when it lies between the two side lines
            let s_apex = (apex - w.start).dot(n_w);
            let via = (s_apex.abs() < hw - 1e-9).then_some(apex);
            Some((EndCut { corners: MiterCorners { left, right }, circle: None, via }, vec![ra.id, rb.id]))
        }
    }
}

/// Edges, vertices and faces added to the lone wall's `(12, 8, 6)` by the junctions `w` belongs to, summed over its ends.
///
/// * the first pair collinear: both walls keep a plain body, 8 / 8; every later wall is a stem, 10 / 8;
/// * the first pair not collinear: the first wall 10 / 12, the second 9 / 8 (the butt corner), a later wall 10 / 8 with one more face
///   when its end is the cone of two neighbours; when a later wall is collinear with the first wall (the + of four) the end faces of
///   those two coincide: both 8 / 8, and the other later wall is a butt 9 / 8.
pub(crate) fn offsets(model: &BimModel, w: &Wall) -> (i64, i64, i64) {
    let (mut de, mut dv, mut df) = (0, 0, 0);
    for end in 0..2 {
        let Some(j) = junction(model, w, end) else { continue };
        let Some(idx) = j.members.iter().position(|(id, _)| *id == w.id) else { continue };
        let tdir = |i: usize| dir(model, j.members[i].0, j.members[i].1);
        let collinear = |a: usize, b: usize| match (tdir(a), tdir(b)) {
            (Some(x), Some(y)) => crate::join_end::corner_flags(x, y).0,
            _ => false,
        };
        let first_pair_collinear = collinear(0, 1);
        let (e, v) = if first_pair_collinear {
            if idx < 2 {
                (8, 8)
            } else {
                (10, 8)
            }
        } else if idx == 0 {
            if j.members.len() >= 4 && (2..j.members.len()).any(|k| collinear(0, k)) {
                (8, 8)
            } else {
                (10, 12)
            }
        } else if idx == 1 {
            (9, 8)
        } else if collinear(0, idx) {
            (8, 8)
        } else if j.members.len() >= 4 {
            (9, 8)
        } else {
            (10, 8)
        };
        de += e - 12;
        dv += v - 8;
        // the end of a wall cut by the cone of two neighbours adds a face
        if let Some(c) = cut(model, w, end) {
            if c.via.is_some() {
                df += 1;
            }
        }
    }
    (de, dv, df)
}

#[cfg(test)]
mod tests {
    use crate::model::{CompoundStructure, Layer, LayerFunction};
    use crate::ops;
    use asterion_geom::Vec2;

    fn walls(list: &[((f64, f64), (f64, f64))]) -> (crate::BimDocument, Vec<asterion_core::ElementId>) {
        let mut d = crate::new_document();
        let out = d.transact("w", false, |m, cx| {
            let level = ops::create_level(m, cx, "L0", 0.0)?;
            let t = ops::create_wall_type(m, cx, "T", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
            let mut ids = Vec::new();
            for (a, b) in list {
                let spec = ops::NewWall { wall_type: t, level, start: Vec2::new(a.0, a.1), end: Vec2::new(b.0, b.1), height: 3.0, base_offset: 0.0, flip: false, structural: false };
                ids.push(ops::create_wall(m, cx, spec)?);
            }
            Ok(ids)
        });
        let ids = out.value.unwrap();
        (d, ids)
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-4
    }

    /// Round 33 y: three 0.2 walls at 120 degrees. The first wall extends to the far face of the second, the second butts
    /// against the first, the third is cut by the cone of both: 3.06928, 2.93072, 2.94804 m3.
    #[test]
    fn a_y_of_three_walls_follows_the_join_array() {
        let (d, ids) = walls(&[((0.0, 0.0), (5.0, 0.0)), ((0.0, 0.0), (-2.5, 4.330127019)), ((0.0, 0.0), (-2.5, -4.330127019))]);
        let m = d.model();
        let s: Vec<_> = ids.iter().map(|i| crate::regen::wall_state(m, &m.walls[i]).unwrap()).collect();
        assert!(near(s[0].volume, 3.06928) && near(s[1].volume, 2.93072) && near(s[2].volume, 2.94804), "{} {} {}", s[0].volume, s[1].volume, s[2].volume);
        assert_eq!((s[0].solid_edge_count, s[0].solid_vertex_count), (10, 12));
        assert_eq!((s[1].solid_edge_count, s[1].solid_vertex_count), (9, 8));
        assert_eq!((s[2].solid_edge_count, s[2].solid_vertex_count, s[2].solid_face_count), (10, 8, 7));
    }

    /// A + created north, south, east, west: the collinear first pair builds no target (unjoined 3.0), the east and west
    /// walls butt on the side faces of the first (2.94).
    #[test]
    fn a_plus_whose_first_pair_is_collinear_leaves_that_pair_alone() {
        let (d, ids) = walls(&[((0.0, 0.0), (0.0, 5.0)), ((0.0, 0.0), (0.0, -5.0)), ((0.0, 0.0), (5.0, 0.0)), ((0.0, 0.0), (-5.0, 0.0))]);
        let m = d.model();
        let v: Vec<f64> = ids.iter().map(|i| crate::regen::wall_state(m, &m.walls[i]).unwrap().volume).collect();
        assert!(near(v[0], 3.0) && near(v[1], 3.0) && near(v[2], 2.94) && near(v[3], 2.94), "{v:?}");
    }
}
