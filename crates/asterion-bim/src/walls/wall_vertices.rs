//! The vertex set of the body of a STRAIGHT vertical wall.
//!
//! The comparator reads a solid's vertex list as a sorted distinct set. This module builds that set from what the engine already
//! decides, without a Boolean kernel:
//!
//! * the plan corners of the body (the body of a straight vertical wall is ONE profile loop extruded by the width; its plan
//!   corners are what the footprint is made of; a joined end has its corners moved onto the mitre plane or the through
//!   extension, `crate::regen`'s prism).
//! * the openings (Spec W-11b: the opening step subtracts every cutter from the wall in turn and the imprint / merge
//!   options switch on; the resulting faces of the wall are the elevation face minus the UNION of the cuts, each clipped to
//!   the host, so the vertices of the body are the corner points of the net region on the front face and on the back face /
//!   W-10*: the cut of a door or window is the same relation as a rectangular opening).
//! * the imprint points a NESTED cut leaves on the boundary of the net region (the same points `imprint_point_count` counts).
//!
//! * The COORDINATES of the imprint come from the Boolean of the join step and are not specified. ENGINE CHOICE: a face of
//!   the neighbour that lies in a face of this wall imprints its contact rectangle (the corners on the face's boundary are
//!   vertices; where the neighbour is taller or reaches lower, the corner on the shared top / bottom edge is not), and a
//!   transversal crossing of two footprint edges (walls that cross in an X) is a vertex line over the shared height. It
//!   reproduces 3,799 of 4,429 straight wall solids of the recorded rounds and agrees with the engine's own vertex COUNT
//!   wherever it is complete.
//!
//! What this does NOT build: the oblique butt and square-off corner (the engine's footprint there is its analytic one), the
//! nodes of three or more walls, the cut of a floor joined to the wall, the unions of several crossings, a reveal, or a
//! sketched profile.

use crate::cut;
use crate::model::{BimModel, Wall};
use asterion_core::ElementId;
use crate::regen::{self, HostOutline, WallState};
use asterion_geom::vec::{Vec2, Vec3};

const EPS: f64 = 1e-9;

/// The vertices of a straight wall's body: corners of the net elevation region on both faces, with the end corners taken from the
/// (joined) footprint. `None` for a wall with no body, a footprint that is not four corners, or an elevation region the kernel
/// cannot form.
pub fn straight_wall_vertices(model: &BimModel, w: &Wall, st: &WallState) -> Option<Vec<Vec3>> {
    let solid = st.solid.as_ref()?;
    if solid.footprint.len() != 4 {
        return None;
    }
    let dir = st.direction;
    let n = dir.perp_ccw();
    let half = st.width / 2.0;
    let corner = corners_by_end_side(w, &solid.footprint)?;

    let ext_start = crate::joins::end_extension(model, w, 0);
    let ext_end = crate::joins::end_extension(model, w, 1);
    let outline = HostOutline { u_lo: -ext_start, u_hi: st.length + ext_end, z_lo: st.base_z, z_hi: st.top_z };
    let cuts: Vec<cut::Rect> = regen::host_cuts(model, w.id)
        .into_iter()
        .map(|(_, _, r)| cut::Rect { u_lo: r.u_lo, u_hi: r.u_hi, z_lo: r.z_lo, z_hi: r.z_hi })
        .collect();
    let pieces = regen::cut_pieces(&outline, &cuts)?;

    // the (station, z) corners of the net region and the imprint points
    let mut uz: Vec<(f64, f64)> = Vec::new();
    let mut push = |p: (f64, f64)| {
        if !uz.iter().any(|q| (q.0 - p.0).abs() < EPS && (q.1 - p.1).abs() < EPS) {
            uz.push(p);
        }
    };
    for piece in &pieces {
        for lp in piece.loops() {
            for c in lp.curves() {
                if let asterion_geom::Curve::Line(l) = c {
                    push((l.start.x, l.start.y));
                }
            }
        }
    }
    let host = cut::Rect { u_lo: outline.u_lo, u_hi: outline.u_hi, z_lo: outline.z_lo, z_hi: outline.z_hi };
    for p in regen::imprint_points(&cuts, &pieces, &host) {
        push(p);
    }

    let mut out = Vec::with_capacity(uz.len() * 2 + 8);
    for (u, z) in uz {
        for side in 0..2usize {
            let sign = if side == 1 { 1.0 } else { -1.0 };
            let xy = if (u - outline.u_lo).abs() < EPS {
                corner[0][side]
            } else if (u - outline.u_hi).abs() < EPS {
                corner[1][side]
            } else {
                w.start + dir * u + n * (sign * half)
            };
            out.push(Vec3::from_xy(xy, z));
        }
    }
    out.extend(contact_imprints(model, w, &corner, st.base_z, st.top_z));
    Some(out)
}

/// The four plan corners of a straight wall's footprint by (end, side): end 1 = the end of the location curve, side 1 = the left
/// (`+n`) face. `None` when the footprint is not four distinct corners.
fn corners_by_end_side(w: &Wall, footprint: &[Vec2]) -> Option<[[Vec2; 2]; 2]> {
    if footprint.len() != 4 {
        return None;
    }
    let dir = (w.end - w.start).normalized()?;
    let n = dir.perp_ccw();
    let mid = (w.start + w.end) * 0.5;
    let mut corner: [[Option<Vec2>; 2]; 2] = [[None; 2]; 2];
    for c in footprint {
        let end = usize::from((*c - mid).dot(dir) >= 0.0);
        let side = usize::from((*c - w.start).dot(n) >= 0.0);
        if corner[end][side].is_some() {
            return None;
        }
        corner[end][side] = Some(*c);
    }
    Some([[corner[0][0]?, corner[0][1]?], [corner[1][0]?, corner[1][1]?]])
}

/// The edges of a four-corner footprint (two side faces, two end caps).
fn edges(c: &[[Vec2; 2]; 2]) -> [(Vec2, Vec2); 4] {
    [(c[0][0], c[1][0]), (c[0][1], c[1][1]), (c[0][0], c[0][1]), (c[1][0], c[1][1])]
}

/// The imprint of the walls JOINED to this one on its faces. A joined neighbour's footprint edge that is collinear with an
/// edge of this wall's footprint, over a stretch of positive length and over a stretch of positive height, is a face of the
/// neighbour lying in a face of this wall (a butting end cap on a side face, a flush side face). The Boolean of the join
/// imprints that contact rectangle on this wall's face (the join step's edge and vertex counts per join class; the target
/// geometry of the later walls of a join), so the corners of the contact rectangle are vertices of this wall, the ones at
/// its own corners included (the comparator merges the coincident ones). A MITRE cap coincides with the neighbour's cap and
/// adds nothing.
fn contact_imprints(model: &BimModel, w: &Wall, corner: &[[Vec2; 2]; 2], z0: f64, z1: f64) -> Vec<Vec3> {
    const TOL: f64 = 1e-7;
    let mut neighbours: Vec<ElementId> = Vec::new();
    let mut add = |q: &Wall| {
        if q.id != w.id && q.through.is_none() && !neighbours.contains(&q.id) {
            neighbours.push(q.id);
        }
    };
    for end in 0..2 {
        for id in crate::joins::elements_at_join(model, w, end) {
            if let Some(q) = model.walls.get(&id) {
                add(q);
            }
        }
    }
    // the walls that cross this one in an X
    for (id, _) in crate::joins::crossings(model, w) {
        if let Some(q) = model.walls.get(&id) {
            add(q);
        }
    }
    // the stems of a T: walls with an end on this wall's interior and that end joinable
    let tol = asterion_geom::Tolerances::default().vertex;
    for q in crate::joins::walls_around(model, w, 0.0) {
        for qe in 0..2 {
            let p = if qe == 0 { q.start } else { q.end };
            let transversal = match ((w.end - w.start).normalized(), (q.end - q.start).normalized()) {
                (Some(a), Some(b)) => a.cross(b).abs() > 0.0017453292519943279,
                _ => false,
            };
            if transversal && q.join_allowed[qe] && crate::joins::pair_in_join(model, w, q, false) && crate::joins::on_path_interior(w, p, tol) {
                add(q);
            }
        }
    }
    let mut out = Vec::new();
    for q in neighbours.iter().filter_map(|id| model.walls.get(id)) {
        let Ok((fp, qz0, qz1)) = regen::wall_view_body(model, q) else { continue };
        let Some(qc) = corners_by_end_side(q, &fp) else { continue };
        let (zlo, zhi) = (z0.max(qz0), z1.min(qz1));
        if zhi - zlo <= TOL {
            continue;
        }
        for (a0, a1) in edges(corner) {
            let Some(ad) = (a1 - a0).normalized() else { continue };
            let alen = (a1 - a0).length();
            for (b0, b1) in edges(&qc) {
                // a transversal crossing of the two footprints' edges, interior to both: the edge of the neighbour's face
                // that passes through this face (a wall that crosses another is cut by it / cuts it; the Boolean leaves the
                // crossing line of the two faces as an edge of both bodies)
                if let Some(bd) = (b1 - b0).normalized() {
                    let denom = ad.cross(bd);
                    if denom.abs() > 1e-9 {
                        let t = (b0 - a0).cross(bd) / denom;
                        let u = (b0 - a0).cross(ad) / denom;
                        let blen = (b1 - b0).length();
                        if t > TOL && t < alen - TOL && u > TOL && u < blen - TOL {
                            for z in [zlo, zhi] {
                                if (z - z0).abs() <= TOL || (z - z1).abs() <= TOL {
                                    out.push(Vec3::from_xy(a0 + ad * t, z));
                                }
                            }
                        }
                    }
                }
                // collinear: both ends of the neighbour's edge on the line of this edge
                if ad.cross(b0 - a0).abs() > TOL || ad.cross(b1 - a0).abs() > TOL {
                    continue;
                }
                let (t0, t1) = ((b0 - a0).dot(ad), (b1 - a0).dot(ad));
                let (lo, hi) = (t0.min(t1).max(0.0), t0.max(t1).min(alen));
                if hi - lo <= TOL {
                    continue;
                }
                for t in [lo, hi] {
                    for z in [zlo, zhi] {
                        // Only the corners on the face's boundary are
                        let on_edge = t <= TOL || t >= alen - TOL || (z - z0).abs() <= TOL || (z - z1).abs() <= TOL;
                        let dropped = ((z - z1).abs() <= TOL && qz1 > z1 + TOL) || ((z - z0).abs() <= TOL && qz0 < z0 - TOL);
                        if on_edge && !dropped {
                            out.push(Vec3::from_xy(a0 + ad * t, z));
                        }
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ops, CompoundStructure, Layer, LayerFunction};

    fn distinct(v: &[Vec3]) -> usize {
        let mut out: Vec<Vec3> = Vec::new();
        for p in v {
            if !out.iter().any(|q| (p.x - q.x).abs() < 1e-6 && (p.y - q.y).abs() < 1e-6 && (p.z - q.z).abs() < 1e-6) {
                out.push(*p);
            }
        }
        out.len()
    }

    /// (the body is one profile loop extruded by the width): a lone wall has the 8 corners of its prism.
    #[test]
    fn a_lone_wall_has_eight_corners_and_a_door_at_the_base_adds_eight() {
        let mut doc = crate::new_document();
        let wall = doc
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L", 0.0)?;
                let cs = CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?;
                let wt = ops::create_wall_type(m, cx, "W", cs)?;
                let v = Vec2::new;
                ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l, start: v(0.0, 0.0), end: v(6.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false })
            })
            .value
            .expect("fixture");
        let m = doc.model();
        let w = &m.walls[&wall];
        let st = regen::wall_state(m, w).unwrap();
        assert_eq!(distinct(&straight_wall_vertices(m, w, &st).unwrap()), 8);
        // Spec W-11b: a door cuts its host; its notch adds the corner points of the net region on both faces (16 points for
        // a 1.0 x 2.0 door at station 2.0)
        let out = doc.transact("door", false, |m, cx| {
            let l = *m.levels.keys().next().unwrap();
            let dt = ops::create_hosted_type(m, cx, "D", crate::model::HostedKind::Door, 1.0, 2.0, 0.0, 1.0, 2.0, 0.0, 0.0)?;
            ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: dt, host: wall, level: l, host_station: 2.0, sill_height: None })
        });
        assert!(out.committed(), "{:?}", out.failures);
        let m = doc.model();
        let w = &m.walls[&wall];
        let st = regen::wall_state(m, w).unwrap();
        let v = straight_wall_vertices(m, w, &st).unwrap();
        assert_eq!(distinct(&v), 16);
        assert_eq!(st.solid_vertex_count, 16, "the set agrees with the engine's own vertex count");
    }

    /// An L corner whose stem butts the through wall: the stem's end cap lies in the through wall's back face, so its two
    /// inner corners are vertices of the through wall (10 points for the wall that has one butting neighbour on each end).
    #[test]
    fn a_butting_stem_imprints_its_end_cap_on_the_through_wall() {
        let mut doc = crate::new_document();
        let (a, b) = doc
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L", 0.0)?;
                let cs = CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?;
                let wt = ops::create_wall_type(m, cx, "W", cs)?;
                let v = Vec2::new;
                let new = |s, e| ops::NewWall { wall_type: wt, level: l, start: s, end: e, height: 3.0, base_offset: 0.0, flip: false, structural: false };
                let a = ops::create_wall(m, cx, new(v(0.0, 0.0), v(6.0, 0.0)))?;
                let b = ops::create_wall(m, cx, new(v(6.0, 0.0), v(6.0, 4.0)))?;
                Ok((a, b))
            })
            .value
            .expect("fixture");
        let m = doc.model();
        for id in [a, b] {
            let w = &m.walls[&id];
            let st = regen::wall_state(m, w).unwrap();
            let v = straight_wall_vertices(m, w, &st).unwrap();
            assert_eq!(distinct(&v), st.solid_vertex_count, "wall {id:?}: the set has as many points as the engine's vertex count");
        }
    }
}
