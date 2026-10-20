//! Walls that CROSS in an X: the implicit join, its clusters and the cut it makes.
//!
//! Everything here is a pure function of the model (nothing is cached). The facts it reproduces are OBSERVED in the lab
//! harness; none of them is validated by an accepted specification row, so every rule below is an ENGINE CHOICE that fits
//! the named observation, and the question that would turn it into a validated rule is named next to it.
//!
//! OBSERVED:
//!
//! * the later wall of a crossing pair is CUT by the earlier one.
//! * `JoinGeometry` on a crossing pair refuses ("already joined"): the pair is joined by default;
//!   `SwitchJoinOrder` exchanges who is cut; `UnjoinGeometry` removes the cut, and no later edit
//!   joins the pair again;
//! * a stem that leaves less than some remnant beyond the other wall (0.5 m and 0.15 m measured) is no crossing at all:
//!   neither wall is cut and no join edge is added (round 25 measured 0.55, 0.65, 0.75, 0.85 and 0.95 m stubs, and host
//!   remnants of 0.6 and 0.9 m, as crossings, so the threshold lies in (0.5, 0.55]).
//! * three walls through one point: the third is cut by ONE band, not by the union of two, and the host counts its
//!   crossings once (16 edges): the crossings whose bands overlap along the wall form one CLUSTER.
//! * the cutter's own door opening is not cut from the cut wall (the band is the cutter's NET body), and a door of the cut
//!   wall at the crossing also notches the cutter.

use crate::model::{BimModel, CrossState, Wall};
use crate::regen::{host_cuts, wall_frame, wall_view_body, CutRect};
use asterion_core::ElementId;
use asterion_geom::{union_all, Region, Tolerances, Vec2};

/// A remnant of either wall beyond the crossing point must be LONGER than this for the pair to be an X (m). OBSERVED: 0.5
/// and 0.15 m are no crossing, 0.55 m and more are. ENGINE CHOICE inside the measured interval (0.5, 0.55]: the reference
/// product's own limit is not specified.
pub const MIN_REMNANT_M: f64 = 0.5;

/// One crossing of `w` by another wall.
#[derive(Clone, Copy, Debug)]
pub struct RawCrossing {
    pub q: ElementId,
    /// Station of the intersection along `w`.
    pub s: f64,
    /// Half extent of `q`'s band along `w`'s location line.
    pub half: f64,
    /// True when `w` is the CUT wall of the pair (before any cluster rule).
    pub cut: bool,
}

fn half_width(model: &BimModel, w: &Wall) -> f64 {
    model.wall_types.get(&w.type_id).map(|t| t.structure.width() / 2.0).unwrap_or(0.0)
}

/// Length of the location line of a wall (the arc length for an arc wall).
fn path_length(w: &Wall) -> f64 {
    match crate::curved_joins::Path::of(w) {
        Some(crate::curved_joins::Path::Arc(a)) => a.radius * a.sweep.abs(),
        _ => (w.end - w.start).length(),
    }
}

/// Station of the point `p` of the path of `w` along it (the arc length for an arc wall), `None` when `p` is not on the path.
fn station(w: &Wall, p: Vec2, tol: f64) -> Option<f64> {
    match crate::curved_joins::Path::of(w)? {
        crate::curved_joins::Path::Line { a, b } => {
            let d = (b - a).normalized()?;
            ((p - a).cross(d).abs() <= tol).then(|| (p - a).dot(d))
        }
        crate::curved_joins::Path::Arc(arc) => {
            if ((p - arc.center).length() - arc.radius).abs() > tol {
                return None;
            }
            let dir = if arc.sweep >= 0.0 { 1.0 } else { -1.0 };
            let a = (p.y - arc.center.y).atan2(p.x - arc.center.x);
            let tau = std::f64::consts::TAU;
            let travelled = (dir * (a - arc.start_angle)).rem_euclid(tau);
            // a point just before the start reads as a travel of almost a full turn: fold it to a small negative station
            let t = if travelled > arc.sweep.abs() + (tau - arc.sweep.abs()) / 2.0 { travelled - tau } else { travelled };
            Some(t * arc.radius)
        }
    }
}

/// The geometric test: `Some((s, half))` when `q` crosses `w` in an X.
fn geometric(model: &BimModel, w: &Wall, q: &Wall) -> Option<(f64, f64)> {
    geometric_all(model, w, q).into_iter().next()
}

/// Every crossing of the pair. For arcs the location lines meet at the points of `crate::curved_joins::intersect` (ENGINE
/// CHOICE: the line rules of rounds 21 / 23 carried over; the station is the arc length and the half extent of the band
/// along `w` is `h_q / sin` of the angle of the two tangents at the crossing).
fn geometric_all(model: &BimModel, w: &Wall, q: &Wall) -> Vec<(f64, f64)> {
    if w.id == q.id || q.base_level != w.base_level {
        return Vec::new();
    }
    if w.through.is_some() || q.through.is_some() {
        return arc_crossings(model, w, q);
    }
    geometric_lines(model, w, q).into_iter().collect()
}

fn arc_crossings(model: &BimModel, w: &Wall, q: &Wall) -> Vec<(f64, f64)> {
    arc_crossing_points(model, w, q).into_iter().map(|(s, half, _)| (s, half)).collect()
}

/// The crossings of an arc pair (or an arc and a line) with their point: `(station, half extent, point)`.
pub(crate) fn arc_crossing_points(model: &BimModel, w: &Wall, q: &Wall) -> Vec<(f64, f64, Vec2)> {
    use crate::curved_joins::{intersect, Path};
    let (Some(pw), Some(pq)) = (Path::of(w), Path::of(q)) else { return Vec::new() };
    let (hw, hq) = (half_width(model, w), half_width(model, q));
    let tol = Tolerances::default().vertex;
    let (lw, lq) = (path_length(w), path_length(q));
    let mut out: Vec<(f64, f64, Vec2)> = Vec::new();
    for x in intersect(&pw.offset(0.0), &pq.offset(0.0)) {
        let (Some(s), Some(t)) = (station(w, x, 1e-6), station(q, x, 1e-6)) else { continue };
        let (Some(tw), Some(tq)) = (crate::joins::path_dir_at(w, x), crate::joins::path_dir_at(q, x)) else { continue };
        let sin = tw.cross(tq).abs();
        if sin < 1e-3 {
            continue;
        }
        let clear_w = s.min(lw - s) > hq / sin + tol;
        let clear_q = t.min(lq - t) > hw / sin + tol;
        let remnant = s.min(lw - s).min(t.min(lq - t)) > MIN_REMNANT_M + 1e-6;
        if s > tol && s < lw - tol && t > tol && t < lq - tol && clear_w && clear_q && remnant {
            out.push((s, hq / sin, x));
        }
    }
    out
}

fn geometric_lines(model: &BimModel, w: &Wall, q: &Wall) -> Option<(f64, f64)> {
    let dw = (w.end - w.start).normalized()?;
    let dq = (q.end - q.start).normalized()?;
    let sin = dw.cross(dq).abs();
    if sin < 1e-3 {
        return None;
    }
    let lw = (w.end - w.start).length();
    let lq = (q.end - q.start).length();
    let r = q.start - w.start;
    let s = r.cross(dq) / dw.cross(dq);
    let t = r.cross(dw) / dw.cross(dq);
    let (hw, hq) = (half_width(model, w), half_width(model, q));
    let tol = Tolerances::default().vertex;
    // interior to both, and no end of either wall inside the other's body (a corner or a T)
    let clear_w = s.min(lw - s) > hq / sin + tol;
    let clear_q = t.min(lq - t) > hw / sin + tol;
    let remnant = s.min(lw - s).min(t.min(lq - t)) > MIN_REMNANT_M + 1e-6;
    (s > tol && s < lw - tol && t > tol && t < lq - tol && clear_w && clear_q && remnant).then_some((s, hq / sin))
}

/// True when `a` and `b` cross in an X, whatever the user did to their join.
pub fn is_crossing_pair(model: &BimModel, a: &Wall, b: &Wall) -> bool {
    geometric(model, a, b).is_some()
}

/// The crossings of `w` that are joined (not unjoined by the user), with who is cut.
pub fn raw_crossings(model: &BimModel, w: &Wall) -> Vec<RawCrossing> {
    let mut out = Vec::new();
    for q in crate::joins::walls_around(model, w, 0.0) {
        if q.id != w.id && !crate::joins::coexists(model, w, q) {
            continue; // A wall that does not coexist is not in the join array
        }
        let default_cut = q.id.0 < w.id.0;
        let cut = match model.cross_override(w.id, q.id) {
            Some(CrossState::Unjoined) => continue,
            Some(CrossState::Swapped) => !default_cut,
            None => default_cut,
        };
        for (s, half) in geometric_all(model, w, q) {
            out.push(RawCrossing { q: q.id, s, half, cut });
        }
    }
    out
}

/// Crossings of `w` whose bands overlap along `w`: one cluster.
#[derive(Clone, Debug)]
pub struct Cluster {
    /// `w` is cut inside this cluster.
    pub cut: bool,
    /// The wall that cuts `w` here: the lowest id among the cutters of the cluster (ENGINE CHOICE from the third wall).
    pub cutter: Option<ElementId>,
    /// The walls `w` cuts here.
    pub victims: Vec<ElementId>,
}

pub fn clusters(model: &BimModel, w: &Wall) -> Vec<Cluster> {
    let mut raw = raw_crossings(model, w);
    raw.sort_by(|a, b| a.s.total_cmp(&b.s));
    let tol = Tolerances::default().vertex;
    let mut groups: Vec<(f64, Vec<RawCrossing>)> = Vec::new();
    for c in raw {
        match groups.last_mut() {
            Some((hi, members)) if c.s - c.half <= *hi + tol => {
                *hi = hi.max(c.s + c.half);
                members.push(c);
            }
            _ => groups.push((c.s + c.half, vec![c])),
        }
    }
    groups
        .into_iter()
        .map(|(_, members)| {
            let cutter = members.iter().filter(|c| c.cut).map(|c| c.q).min();
            let victims = members.iter().filter(|c| !c.cut).map(|c| c.q).collect();
            Cluster { cut: cutter.is_some(), cutter, victims }
        })
        .collect()
}

/// The walls that cut `w` (one per cut cluster).
pub fn cutters(model: &BimModel, w: &Wall) -> Vec<ElementId> {
    clusters(model, w).into_iter().filter_map(|c| c.cutter).collect()
}

// ------------------------------------------------------------------ solids

/// A vertical box: a plan region over a z range (absolute).
struct Box3 {
    region: Region,
    z_lo: f64,
    z_hi: f64,
}

fn plan_box(model: &BimModel, wall: &Wall, r: &CutRect) -> Option<Box3> {
    let (dir, orient, _len, width) = wall_frame(model, wall).ok()?;
    let h = orient * (width / 2.0);
    let (a, b) = (wall.start + dir * r.u_lo, wall.start + dir * r.u_hi);
    let poly: Vec<Vec2> = vec![a - h, b - h, b + h, a + h];
    let region = Region::from_polygon(&poly, &Tolerances::default()).ok()?;
    Some(Box3 { region, z_lo: r.z_lo, z_hi: r.z_hi })
}

/// The cut of every hosted door and window of `wall`, as plan boxes through the wall's thickness.
fn opening_boxes(model: &BimModel, wall: &Wall) -> Vec<Box3> {
    host_cuts(model, wall.id).iter().filter_map(|(_, _, r)| plan_box(model, wall, r)).collect()
}

fn footprint(model: &BimModel, w: &Wall) -> Option<(Region, f64, f64)> {
    let (fp, z0, z1) = wall_view_body(model, w).ok()?;
    if fp.len() < 3 || z1 <= z0 {
        return None;
    }
    Some((Region::from_polygon(&fp, &Tolerances::default()).ok()?, z0, z1))
}

fn perimeter(regions: &[Region]) -> f64 {
    regions.iter().map(|r| r.outer_perimeter() + r.hole_perimeter()).sum()
}

fn area(regions: &[Region]) -> f64 {
    regions.iter().map(|r| r.area()).sum()
}

/// The solid removed from one wall body: its volume, the change of its surface area, and how the
/// cutters took part.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Removal {
    volume: f64,
    surface: f64,
    /// Volume the cutters take (own openings aside), and whether they take the band at only some
    /// heights.
    cutter_volume: f64,
    partial: bool,
}

/// The solid removed from `own` (a footprint between `z0` and `z1`) by its own openings, the
/// openings of other walls that carve it (`carve`) and the NET bodies of its cutters (each cutter's
/// footprint minus that cutter's own openings). Exact: the body is cut into slabs of constant
/// section, and the section removed in each slab is a plan region. The surface change counts the
/// lateral faces of every slab and the horizontal faces where neighbouring slabs differ.
fn slab_removal(own: &Region, z0: f64, z1: f64, own_open: &[Box3], carve: &[Box3], cutters: &[Cutter]) -> Removal {
    let tol = Tolerances::default();
    let mut zs = vec![z0, z1];
    let all_boxes = own_open.iter().chain(carve).chain(cutters.iter().flat_map(|c| c.holes.iter()));
    for z in all_boxes.flat_map(|b| [b.z_lo, b.z_hi]).chain(cutters.iter().flat_map(|c| [c.z_lo, c.z_hi])) {
        if z > z0 + 1e-9 && z < z1 - 1e-9 {
            zs.push(z);
        }
    }
    zs.sort_by(f64::total_cmp);
    zs.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    let own_perim = own.outer_perimeter() + own.hole_perimeter();
    let cutter_full: f64 = cutters
        .iter()
        .map(|c| own.intersection(&c.fp, &tol).map(|v| area(&v)).unwrap_or(0.0))
        .sum();
    let mut out = Removal::default();
    let mut prev: Vec<Region> = Vec::new();
    let mut cutter_areas: Vec<f64> = Vec::new();
    for k in 0..zs.len().saturating_sub(1) {
        let (za, zb) = (zs[k], zs[k + 1]);
        let dz = zb - za;
        let zm = 0.5 * (za + zb);
        let active = |b: &Box3| b.z_lo < zm && zm < b.z_hi;
        // the net bodies of the cutters in this slab
        let mut removed: Vec<Region> = Vec::new();
        let mut cutter_area = 0.0;
        for c in cutters.iter().filter(|c| c.z_lo < zm && zm < c.z_hi) {
            let mut pieces = vec![c.fp.clone()];
            for h in c.holes.iter().filter(|h| active(h)) {
                let mut next = Vec::new();
                for p in &pieces {
                    match p.difference(&h.region, &tol) {
                        Ok(d) => next.extend(d),
                        Err(_) => next.push(p.clone()),
                    }
                }
                pieces = next;
            }
            for p in &pieces {
                if let Ok(i) = own.intersection(p, &tol) {
                    cutter_area += area(&i);
                }
            }
            removed.extend(pieces);
        }
        cutter_areas.push(cutter_area);
        removed.extend(own_open.iter().chain(carve).filter(|b| active(b)).map(|b| b.region.clone()));
        let mut r: Vec<Region> = Vec::new();
        if !removed.is_empty() {
            let merged = union_all(&removed, &tol).unwrap_or(removed);
            for u in &merged {
                if let Ok(i) = own.intersection(u, &tol) {
                    r.extend(i);
                }
            }
        }
        let a = area(&r);
        out.volume += a * dz;
        let mut net = vec![own.clone()];
        for piece in &r {
            let mut next = Vec::new();
            for n in &net {
                match n.difference(piece, &tol) {
                    Ok(d) => next.extend(d),
                    Err(_) => next.push(n.clone()),
                }
            }
            net = next;
        }
        out.surface += dz * (perimeter(&net) - own_perim);
        if k == 0 {
            out.surface -= a;
        } else {
            let mut inter = 0.0;
            for p in &prev {
                for q in &r {
                    if let Ok(i) = p.intersection(q, &tol) {
                        inter += area(&i);
                    }
                }
            }
            out.surface += area(&prev) + a - 2.0 * inter;
        }
        prev = r;
    }
    out.surface -= area(&prev);
    // The cutter volume proper: the slabs again, but counting only the cutters' removal
    out.cutter_volume = zs.windows(2).zip(&cutter_areas).map(|(z, a)| a * (z[1] - z[0])).sum();
    out.partial = cutter_areas.iter().any(|a| *a > 1e-9) && cutter_areas.iter().any(|a| *a < cutter_full - 1e-9);
    out
}

/// A cutter's NET body: its footprint over its own height, minus its own openings.
struct Cutter {
    fp: Region,
    z_lo: f64,
    z_hi: f64,
    holes: Vec<Box3>,
}

/// The solid a set of vertical boxes removes from a wall body: `(volume, change of the surface area)`, for the slab
/// machinery above (used by reveals, [`crate::sweeps`]).
pub(crate) fn removal_by_boxes(own: &Region, z0: f64, z1: f64, open: &[(Region, f64, f64)], boxes: &[(Region, f64, f64)]) -> (f64, f64) {
    let to_box = |(r, a, b): &(Region, f64, f64)| Box3 { region: r.clone(), z_lo: *a, z_hi: *b };
    let open: Vec<Box3> = open.iter().map(to_box).collect();
    let mut all: Vec<Box3> = open.iter().map(|b| to_box(&(b.region.clone(), b.z_lo, b.z_hi))).collect();
    all.extend(boxes.iter().map(to_box));
    // what the boxes take beyond what the openings already took out of the body
    let base = slab_removal(own, z0, z1, &open, &[], &[]);
    let r = slab_removal(own, z0, z1, &all, &[], &[]);
    (r.volume - base.volume, r.surface - base.surface)
}

fn cutter_nets(model: &BimModel, ids: &[ElementId]) -> Vec<Cutter> {
    ids.iter()
        .filter_map(|id| model.walls.get(id))
        .filter_map(|q| footprint(model, q).map(|(fp, z_lo, z_hi)| Cutter { fp, z_lo, z_hi, holes: opening_boxes(model, q) }))
        .collect()
}

fn carve_boxes(model: &BimModel, victims: &[ElementId]) -> Vec<Box3> {
    victims.iter().filter_map(|id| model.walls.get(id)).flat_map(|q| opening_boxes(model, q)).collect()
}

/// What the crossings of `w` add to its solid, beyond its own openings.
struct Effect {
    /// Cut clusters whose cutter takes volume.
    cut_effective: Vec<bool>,
    /// Carve boxes (openings of the walls `w` cuts) that overlap `w`'s body, and whether one
    /// spans the whole height of `w`.
    carves: usize,
    carve_through: bool,
    base: Removal,
    all: Removal,
}

fn effect(model: &BimModel, w: &Wall, cl: &[Cluster]) -> Option<Effect> {
    let (own, z0, z1) = footprint(model, w)?;
    let cutter_ids: Vec<ElementId> = cl.iter().filter_map(|c| c.cutter).collect();
    let victims: Vec<ElementId> = cl.iter().flat_map(|c| c.victims.iter().copied()).collect();
    let own_open = opening_boxes(model, w);
    let carve = carve_boxes(model, &victims);
    let nets = cutter_nets(model, &cutter_ids);
    let tol = Tolerances::default();
    let base = slab_removal(&own, z0, z1, &own_open, &[], &[]);
    let all = slab_removal(&own, z0, z1, &own_open, &carve, &nets);
    // per cutter: does it take anything of its own?
    let mut cut_effective = Vec::new();
    for c in cl {
        let eff = match c.cutter {
            Some(q) => {
                let one = cutter_nets(model, &[q]);
                let r = slab_removal(&own, z0, z1, &own_open, &[], &one);
                r.volume - base.volume > 1e-9
            }
            None => false,
        };
        cut_effective.push(eff);
    }
    let mut carves = 0;
    let mut carve_through = false;
    for b in &carve {
        let overlaps = own.intersection(&b.region, &tol).map(|v| area(&v) > 1e-9).unwrap_or(false);
        let z_ok = b.z_hi > z0 + 1e-9 && b.z_lo < z1 - 1e-9;
        if overlaps && z_ok {
            carves += 1;
            if b.z_lo <= z0 + 1e-9 && b.z_hi >= z1 - 1e-9 {
                carve_through = true;
            }
        }
    }
    Some(Effect { cut_effective, carves, carve_through, base, all })
}

/// The join incidences the crossings of `w` add to its counts: one per cluster, and only where the
/// crossing changes a solid (a cluster that removes nothing adds no edge).
pub fn classes(model: &BimModel, w: &Wall) -> Vec<crate::joins::JoinClass> {
    use crate::joins::JoinClass;
    let cl = clusters(model, w);
    if cl.is_empty() {
        return Vec::new();
    }
    let Some(e) = effect(model, w, &cl) else { return Vec::new() };
    let mut out = Vec::new();
    let top = |q: &Wall| footprint(model, q).map(|(_, _, z1)| z1);
    let own_top = top(w);
    for (i, c) in cl.iter().enumerate() {
        if c.cut {
            if e.cut_effective[i] {
                // OBSERVED (`h2`): a cut wall LOWER than its cutter has 22 edges, not 20
                let lower = match (own_top, c.cutter.and_then(|q| model.walls.get(&q)).and_then(top)) {
                    (Some(a), Some(b)) => a < b - 1e-9,
                    _ => false,
                };
                out.push(if lower { JoinClass::CrossCutLower } else { JoinClass::CrossCut });
            }
        } else {
            // a host: effective when a victim loses volume to `w`, or `w` itself is carved
            let victim_cut = c.victims.iter().filter_map(|id| model.walls.get(id)).any(|q| {
                let qcl = clusters(model, q);
                effect(model, q, &qcl).map_or(false, |qe| {
                    qcl.iter().zip(&qe.cut_effective).any(|(qc, eff)| *eff && qc.cutter == Some(w.id))
                })
            });
            if victim_cut || e.carves > 0 {
                // OBSERVED (`h1`): a host whose crossing walls are all LOWER than it has 14 edges and 12 vertices, not 16
                // and 16
                let all_lower = !c.victims.is_empty()
                    && c.victims.iter().filter_map(|id| model.walls.get(id)).filter_map(top).all(|t| matches!(own_top, Some(a) if t < a - 1e-9));
                out.push(if all_lower { JoinClass::CrossHostLower } else { JoinClass::CrossHost });
            }
        }
    }
    out
}

/// Applies the crossings to the quantities of `s`: volume, Area, surface and the solid's counts.
pub fn apply(model: &BimModel, w: &Wall, s: &mut crate::regen::WallState) {
    if s.width <= 0.0 || s.height <= 0.0 {
        return;
    }
    let cl = clusters(model, w);
    if cl.is_empty() {
        return;
    }
    let Some(e) = effect(model, w, &cl) else { return };
    let dv = e.all.volume - e.base.volume;
    if dv.abs() < 1e-12 && !e.cut_effective.iter().any(|x| *x) && e.carves == 0 {
        return;
    }
    // a wall cut into lumps with an arc involved has its Stokes sums from the lumps (`crate::curved_ends::lump_loops`)
    if !crate::curved_ends::has_lumps(model, w) {
        s.volume -= dv;
        s.area -= dv / s.width;
        s.surface_area += e.all.surface - e.base.surface;
    }
    let (je, jv) = crate::joins::join_edge_vertex_counts(model, w);
    // what the wall's own openings add to a bare box (the join offsets are already in the counts)
    let d_faces = s.solid_face_count as i64 - 6;
    let d_edges = s.solid_edge_count as i64 - je;
    let d_verts = s.solid_vertex_count as i64 - jv;
    if e.cut_effective.iter().any(|x| *x) {
        // two lumps (OBSERVED): 8 faces, 24 + 2 (je - 12) edges, 16 + (jv - 8) vertices; the band at only some heights
        // keeps one lump with a notch: 9 faces (round 23, `v2`)
        let faces = if e.all.partial { 9 } else { 8 } + d_faces;
        s.solid_face_count = faces.max(0) as usize;
        s.solid_edge_count = (24 + 2 * (je - 12) + d_edges).max(0) as usize;
        s.solid_vertex_count = (16 + (jv - 8) + d_verts).max(0) as usize;
    }
    if e.carves > 0 {
        let n = e.carves as i64;
        if e.carve_through {
            // the opening of the other wall takes the whole height: two lumps (OBSERVED, `w1` clipped)
            s.solid_face_count = (8 + d_faces).max(0) as usize;
            s.solid_edge_count = (20 + (je - 12) + d_edges).max(0) as usize;
            s.solid_vertex_count = (16 + (jv - 8) + d_verts).max(0) as usize;
        } else {
            // a notch from the base: +3 faces, +8 edges, +4 vertices (OBSERVED once, `w1`)
            s.solid_face_count = (s.solid_face_count as i64 + 3 * n) as usize;
            s.solid_edge_count = (s.solid_edge_count as i64 + 8 * n) as usize;
            s.solid_vertex_count = (s.solid_vertex_count as i64 + 4 * n) as usize;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::codes;
    use crate::{ops, BimDocument, CompoundStructure, Layer, LayerFunction};
    use asterion_core::txn::TxnContext;

    fn build(walls: &[(f64, f64, f64, f64, f64)]) -> (BimDocument, Vec<ElementId>) {
        // (height, x0, y0, x1, y1), all 0.2 thick
        let mut doc = crate::new_document();
        let ids = doc
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let cs = CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?;
                let wt = ops::create_wall_type(m, cx, "W", cs)?;
                let mut ids = Vec::new();
                for (h, x0, y0, x1, y1) in walls {
                    ids.push(ops::create_wall(
                        m,
                        cx,
                        ops::NewWall { wall_type: wt, level: l, start: Vec2::new(*x0, *y0), end: Vec2::new(*x1, *y1), height: *h, base_offset: 0.0, flip: false, structural: false },
                    )?);
                }
                Ok(ids)
            })
            .value
            .expect("fixture");
        (doc, ids)
    }

    fn state(doc: &BimDocument, id: ElementId) -> crate::regen::WallState {
        crate::regen::wall_state(doc.model(), doc.model().wall(id).unwrap()).unwrap()
    }

    #[test]
    fn a_stem_that_passes_less_than_the_remnant_is_no_crossing() {
        // OBSERVED (`g`): remnants of 0.5 and 0.15 m beyond the other wall: neither wall is cut
        let (doc, ids) = build(&[(3.0, 0.0, 0.0, 10.0, 0.0), (3.0, 5.0, -0.5, 5.0, 5.0)]);
        assert!(raw_crossings(doc.model(), doc.model().wall(ids[0]).unwrap()).is_empty());
        assert_eq!((state(&doc, ids[0]).solid_edge_count, state(&doc, ids[1]).solid_edge_count), (12, 12));
        // 1.0 m: an X (round 23 `f3`)
        let (doc, ids) = build(&[(3.0, 0.0, 0.0, 10.0, 0.0), (3.0, 5.0, -1.0, 5.0, 5.0)]);
        assert_eq!(raw_crossings(doc.model(), doc.model().wall(ids[0]).unwrap()).len(), 1);
    }

    #[test]
    fn three_walls_through_one_point_cut_the_third_once_and_the_host_counts_one_cluster() {
        // OBSERVED (`t1..t3`): t3 loses 0.8485 m2, ONE band (not the union of two); the host t1 keeps 16 edges and 16
        // vertices; t2 and t3 have 20 and 16
        let s = 0.5_f64.sqrt();
        let (doc, ids) = build(&[
            (3.0, 0.0, 0.0, 10.0, 0.0),
            (3.0, 5.0, -5.0, 5.0, 5.0),
            (3.0, 5.0 - 5.0 * s, -5.0 * s, 5.0 + 5.0 * s, 5.0 * s),
        ]);
        let (t1, t2, t3) = (state(&doc, ids[0]), state(&doc, ids[1]), state(&doc, ids[2]));
        assert_eq!((t1.solid_edge_count, t1.solid_vertex_count, t1.solid_face_count), (16, 16, 6));
        assert_eq!((t2.solid_edge_count, t2.solid_vertex_count, t2.solid_face_count), (20, 16, 8));
        assert_eq!((t3.solid_edge_count, t3.solid_vertex_count, t3.solid_face_count), (20, 16, 8));
        assert!((t3.area - (30.0 - 0.2 / s * 3.0)).abs() < 1e-6, "{}", t3.area);
    }

    #[test]
    fn join_unjoin_and_switch_on_a_crossing_pair() {
        // OBSERVED: JoinGeometry refuses ("already joined"); SwitchJoinOrder exchanges who is cut; UnjoinGeometry uncuts
        // both and no later edit joins them again
        let (mut doc, ids) = build(&[(3.0, 0.0, 0.0, 10.0, 0.0), (3.0, 5.0, -5.0, 5.0, 5.0)]);
        let (a, b) = (ids[0], ids[1]);
        assert!((state(&doc, b).volume - 5.88).abs() < 1e-9 && (state(&doc, a).volume - 6.0).abs() < 1e-9);
        let join = doc.transact("join", false, |m, cx| ops::join_elements(m, cx, a, b));
        assert!(join.value.is_none() && join.failures[0].code == codes::JOIN_ALREADY_JOINED);
        assert!(doc.transact("switch", false, |m, cx| ops::swap_join_priority(m, cx, a, b)).committed());
        assert!((state(&doc, a).volume - 5.88).abs() < 1e-9 && (state(&doc, b).volume - 6.0).abs() < 1e-9);
        assert!(doc.transact("unjoin", false, |m, cx| ops::unjoin_elements(m, cx, a, b)).committed());
        assert!((state(&doc, a).volume - 6.0).abs() < 1e-9 && (state(&doc, b).volume - 6.0).abs() < 1e-9);
        // an edit of the cut wall does not join them again
        assert!(doc.transact("flip", false, |m, cx| ops::flip_wall(m, cx, b)).committed());
        assert!((state(&doc, a).volume - 6.0).abs() < 1e-9);
        // joining them again is allowed, with the default order
        assert!(doc.transact("rejoin", false, |m, cx| ops::join_elements(m, cx, a, b)).committed());
        assert!((state(&doc, b).volume - 5.88).abs() < 1e-9);
    }

    #[test]
    fn a_hole_in_the_cutter_is_not_cut_from_the_other_wall_and_a_door_of_the_cut_wall_notches_the_cutter() {
        // OBSERVED (`v` and `w`): a door in the CUTTER at the crossing leaves the band of the cut wall whole below the door
        // (0.2 m2 of 0.6 removed); a door in the CUT wall at the crossing also notches the cutter (0.4 m2) and removes only
        // 1.6 m2 more from the cut wall
        let mut doc = crate::new_document();
        let ((a, b), (c, d)) = doc
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let cs = CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?;
                let wt = ops::create_wall_type(m, cx, "W", cs)?;
                let dt = ops::create_hosted_type(m, cx, "D", crate::model::HostedKind::Door, 1.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0)?;
                let w = |m: &mut crate::BimModel, cx: &mut TxnContext<'_>, a: (f64, f64), b: (f64, f64)| {
                    ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l, start: Vec2::new(a.0, a.1), end: Vec2::new(b.0, b.1), height: 3.0, base_offset: 0.0, flip: false, structural: false })
                };
                let v1 = w(m, cx, (0.0, 40.0), (10.0, 40.0))?;
                let v2 = w(m, cx, (5.0, 35.0), (5.0, 45.0))?;
                ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: dt, host: v1, level: l, host_station: 5.0, sill_height: None })?;
                let w1 = w(m, cx, (0.0, 20.0), (10.0, 20.0))?;
                let w2 = w(m, cx, (5.0, 15.0), (5.0, 25.0))?;
                ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: dt, host: w2, level: l, host_station: 5.0, sill_height: None })?;
                Ok(((v1, v2), (w1, w2)))
            })
            .value
            .expect("fixture");
        assert!((state(&doc, b).area - 29.8).abs() < 1e-9 && (state(&doc, b).solid_face_count == 9), "{} {}", state(&doc, b).area, state(&doc, b).solid_face_count);
        assert!((state(&doc, a).area - 28.0).abs() < 1e-9);
        assert!((state(&doc, c).area - 29.6).abs() < 1e-9 && state(&doc, c).solid_face_count == 9, "{} {}", state(&doc, c).area, state(&doc, c).solid_face_count);
        assert!((state(&doc, d).area - 27.8).abs() < 1e-9 && state(&doc, d).solid_face_count == 11);
    }
}
