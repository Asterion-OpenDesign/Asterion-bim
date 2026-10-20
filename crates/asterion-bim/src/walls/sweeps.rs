//! Wall sweeps and reveals (OBSERVED).
//!
//! * The template has one sweep type ("Cornice") and one reveal type ("Reveal").
//! * A HORIZONTAL sweep runs the length of the wall's location line, `distance` above the wall's base (a flipped
//!   profile hangs below that height), on the exterior or interior face, `offset` away from the face; the profile is
//!   a square, so the body is the plan strip extruded 152.4 mm. A VERTICAL one is a 152.4 mm strip along the wall at
//!   `distance` from its start, as tall as the wall. "Measured from the top" had no effect on a horizontal sweep
//!   (ENGINE CHOICE: ignored there).
//! * The body is cut by the openings of the host (doors), trimmed by the other walls it runs into (a joined corner)
//!   and mitred against the sweep of a neighbouring wall at the same height (the overlap is split along the line
//!   through the two inner corners; a mitred piece reports two edges fewer).
//! * A reveal cuts the wall: its volume goes from the wall's, the wall gains the three faces, twelve edges and eight
//!   vertices of a groove, its Area parameter does not change. A reveal that cuts nothing has a box solid; one that
//!   cuts has no solid.
//! * Sweeps follow their wall when it is edited or moved, are deleted with it, are not copied with it, keep the side
//!   they were made on when the wall is flipped, and two at the same place post a warning.

use crate::annot::{SweepKind, SweepType, WallSweep};
use crate::model::BimModel;
use crate::spec::codes;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use asterion_geom::{Region, Tolerances, Vec2};

pub const TEMPLATE_SWEEP_TYPE: &str = "Cornice";
pub const TEMPLATE_REVEAL_TYPE: &str = "Reveal";
/// ENGINE CHOICE: see [`sweep_state`].
const REVEAL_BOX_OVERSHOOT_M: f64 = 0.03048;

/// OBSERVED: the profile of the template types, a square of 152.4 mm (6 inches).
pub const PROFILE_M: f64 = 0.1524;

fn bad(msg: &str) -> Failure {
    Failure::error(codes::SWEEP_INVALID, msg.to_owned())
}

/// The types of a kind, in name order; the template's one is created on first use.
pub fn types_of(m: &mut BimModel, cx: &mut TxnContext<'_>, kind: SweepKind) -> Vec<ElementId> {
    let name = if kind == SweepKind::Sweep { TEMPLATE_SWEEP_TYPE } else { TEMPLATE_REVEAL_TYPE };
    if !m.annot.sweep_types.values().any(|t| t.name == name) {
        let id = cx.new_id();
        m.annot.sweep_types.insert(id, SweepType { id, name: name.to_owned(), kind, profile: PROFILE_M });
    }
    let mut v: Vec<&SweepType> = m.annot.sweep_types.values().filter(|t| t.kind == kind).collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v.iter().map(|t| t.id).collect()
}

/// The `index`-th type of a kind (the harness lists them by name).
pub fn use_type(m: &mut BimModel, cx: &mut TxnContext<'_>, kind: SweepKind, index: usize) -> Result<ElementId, Failure> {
    let ids = types_of(m, cx, kind);
    ids.get(index).copied().ok_or_else(|| bad("No sweep or reveal type at that index."))
}

/// Of a sweep or reveal type.
pub fn duplicate_type(m: &mut BimModel, cx: &mut TxnContext<'_>, source: ElementId, name: &str) -> Result<ElementId, Failure> {
    let t = m.annot.sweep_types.get(&source).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such sweep type.").on(&[source]))?.clone();
    // The duplicate's name checks
    let name = &crate::type_names::check_duplicate_name(name, |n| m.annot.sweep_types.values().any(|x| x.name == n))?;
    let id = cx.new_id();
    m.annot.sweep_types.insert(id, SweepType { id, name: name.to_owned(), ..t });
    Ok(id)
}

/// The settings.
#[derive(Clone, Copy, Debug)]
pub struct SweepInfo {
    pub kind: SweepKind,
    pub vertical: bool,
    pub distance: f64,
    pub from_top: bool,
    pub flipped: bool,
    pub offset: f64,
    pub exterior: bool,
    pub sweep_intersects_wall: bool,
}

impl SweepInfo {
    pub fn new(kind: SweepKind, vertical: bool) -> Self {
        SweepInfo { kind, vertical, distance: 0.0, from_top: false, flipped: false, offset: 0.0, exterior: true, sweep_intersects_wall: true }
    }
}

pub fn create_wall_sweep(m: &mut BimModel, cx: &mut TxnContext<'_>, wall: ElementId, type_id: ElementId, info: SweepInfo) -> Result<ElementId, Failure> {
    // OBSERVED (`wr2` / `sr2`): the reference product creates a sweep on an ARC wall (`WallAllowsWallSweep` true, the sweep
    // follows the arc); its geometry is `crate::arc_sweeps`.
    let w = m.walls.get(&wall).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such wall.").on(&[wall]))?;
    let ty = m.annot.sweep_types.get(&type_id).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such sweep type.").on(&[type_id]))?;
    if ty.kind != info.kind {
        return Err(bad("The type is not a type of that kind."));
    }
    let orient = if w.flipped { -1i8 } else { 1i8 };
    let id = cx.new_id();
    let s = WallSweep {
        id,
        type_id,
        wall,
        kind: info.kind,
        vertical: info.vertical,
        distance: info.distance,
        from_top: info.from_top,
        flipped: info.flipped,
        offset: info.offset,
        exterior_requested: info.exterior,
        side_sign: if info.exterior { orient } else { -orient },
        sweep_intersects_wall: info.sweep_intersects_wall,
    };
    // two wall sweeps at one place: a warning (OBSERVED). Spec SWP-03: the pair is compared only when both are on the same
    // side with the same vertical flag and NEITHER is a reveal - overlapping reveals, and a reveal over a sweep, are never
    // reported (not measured: ENGINE CHOICE for the profile overlap, the square profile of the template types).
    let clash: Vec<ElementId> = m
        .annot
        .sweeps
        .values()
        .filter(|o| o.kind == SweepKind::Sweep && s.kind == SweepKind::Sweep)
        .filter(|o| o.wall == wall && o.vertical == s.vertical && o.side_sign == s.side_sign && (o.distance - s.distance).abs() < PROFILE_M - 1e-9)
        .map(|o| o.id)
        .collect();
    let outside = !position_in_wall_for(m, &s);
    m.annot.sweeps.insert(id, s);
    if outside {
        cx.report(Failure::warning(codes::SWEEP_OUTSIDE_WALL, "Could not create Wall Sweep. Sweep position is outside of its wall. Please check sweep parameters.").on(&[id]));
    } else if !clash.is_empty() {
        let mut ids = clash;
        ids.push(id);
        cx.report(Failure::warning(codes::SWEEP_OVERLAP, "Could not create Wall Sweep. Two wall sweeps are overlapping.").on(&ids));
    }
    Ok(id)
}

fn position_in_wall_for(m: &BimModel, s: &WallSweep) -> bool {
    position_in_wall(m, s)
}

/// Delete the sweeps of a wall (returns their ids).
pub fn take_for_wall(m: &mut BimModel, wall: ElementId) -> Vec<ElementId> {
    let ids: Vec<ElementId> = m.annot.sweeps.values().filter(|s| s.wall == wall).map(|s| s.id).collect();
    for i in &ids {
        m.annot.sweeps.remove(i);
    }
    ids
}

fn v2(p: [f64; 2]) -> Vec2 {
    Vec2::new(p[0], p[1])
}

/// The plan strip of a sweep as a rectangle's corners, and its z range.
struct Strip {
    /// The strip along the face of the wall, between the ends of the face (the plain strip).
    corners: Vec<Vec2>,
    /// The same strip run on into the corner square at the ends of a join on the outside (to be mitred with the
    /// neighbour's strip).
    ext_corners: Vec<Vec2>,
    z0: f64,
    z1: f64,
    /// A point inside the strip near the middle of the wall (to pick the side of a mitre).
    far: Vec2,
    /// The two lines of the strip along the wall: (point, direction) of the inner and the outer edge.
    inner: (Vec2, Vec2),
    outer: (Vec2, Vec2),
    /// The strip in the wall's own frame: along the location line, across it (positive to the left), up.
    u: (f64, f64),
    v: (f64, f64),
}

/// Whether the position of a sweep is inside its wall: OBSERVED, a distance of -0.5 and one of 3.5 along a 3 m wall are
/// refused with a warning ("Sweep position is outside of its wall") and leave a sweep with no body, while 0.0 and 3.0 are
/// fine. ENGINE CHOICE: the range is `[0, wall height]` (`[0, wall length]` for a vertical sweep), edges included; the
/// values between 3.0 and 3.5 are not measured.
pub fn position_in_wall(m: &BimModel, sw: &WallSweep) -> bool {
    let Some(w) = m.walls.get(&sw.wall) else { return false };
    let Ok((_, _, len, _)) = crate::regen::wall_frame(m, w) else { return false };
    let Ok((_, base, top)) = crate::regen::wall_view_body(m, w) else { return false };
    let limit = if sw.vertical { len } else { top - base };
    sw.distance >= -1e-9 && sw.distance <= limit + 1e-9
}

/// The outline of a wall's footprint united with the footprints of the walls that reach the ends of its location line.
fn joined_outline(m: &BimModel, wall: ElementId, fp: &[Vec2]) -> Option<Vec<Vec2>> {
    let mine = region(fp)?;
    let mut all = vec![mine.clone()];
    for (id, other) in &m.walls {
        if *id != wall && touches_an_end(m, wall, other) {
            if let Ok((ofp, _, _)) = crate::regen::wall_view_body(m, other) {
                if let Some(r) = region(&ofp) {
                    all.push(r);
                }
            }
        }
    }
    if all.len() == 1 {
        return None;
    }
    let united = asterion_geom::union_all(&all, &Tolerances::default()).ok()?;
    let probe = Vec2::new(fp.iter().map(|p| p.x).sum::<f64>() / fp.len() as f64, fp.iter().map(|p| p.y).sum::<f64>() / fp.len() as f64);
    let region = united.iter().find(|r| r.contains_point(probe)).or_else(|| united.first())?;
    let pts = region.outer_corner_points();
    (pts.len() >= 3).then_some(pts)
}

/// The extent along the location line `(u_min, u_max)` of the edges of a wall's outline that lie on the face line
/// `v = hw` (positive to the side `n`): where the face of the wall begins and ends once its joins are made.
fn face_extent(fp: &[Vec2], start: Vec2, d: Vec2, n: Vec2, hw: f64) -> Option<(f64, f64)> {
    let eps = 1e-9;
    let mut us: Vec<f64> = Vec::new();
    let dist = |p: Vec2| (p - start).dot(n) - hw;
    let uof = |p: Vec2| (p - start).dot(d);
    for i in 0..fp.len() {
        let (a, b) = (fp[i], fp[(i + 1) % fp.len()]);
        let (da, db) = (dist(a), dist(b));
        // only the edges that lie ON the face line are the face
        if da.abs() < eps && db.abs() < eps {
            us.push(uof(a));
            us.push(uof(b));
        }
    }
    if us.is_empty() {
        return None;
    }
    Some((us.iter().copied().fold(f64::INFINITY, f64::min), us.iter().copied().fold(f64::NEG_INFINITY, f64::max)))
}

fn strip(m: &BimModel, sw: &WallSweep) -> Option<Strip> {
    if !position_in_wall(m, sw) {
        return None;
    }
    let w = m.walls.get(&sw.wall)?;
    let (d, _, len, width) = crate::regen::wall_frame(m, w).ok()?;
    let (fp, base, top) = crate::regen::wall_view_body(m, w).ok()?;
    let n = Vec2::new(-d.y, d.x) * f64::from(sw.side_sign);
    let hw = width / 2.0;
    let s = PROFILE_M;
    let (v0, v1) = if sw.kind == SweepKind::Reveal { (hw - s, hw) } else { (hw + sw.offset, hw + sw.offset + s) };
    // where the face of the wall begins and ends (a join shortens it on the inside of a corner and lengthens it on
    // the outside; a butt join leaves it)
    // the face runs along the outline of the wall joined to the walls that reach its ends
    let poly = joined_outline(m, sw.wall, &fp).unwrap_or_else(|| fp.clone());
    let (fa, fb) = face_extent(&poly, w.start, d, n, hw).unwrap_or((0.0, len));
    let eps = 1e-9;
    let reach = v1 - hw; // how far beyond the face the strip stands
    // the strip run on into the corner square at an outside join (to be mitred with the neighbour's strip)
    let (xa, xb) = (if fa < -eps { fa - reach.max(0.0) } else { fa }, if fb > len + eps { fb + reach.max(0.0) } else { fb });
    let (u0, u1) = if sw.vertical {
        // OBSERVED: a vertical sweep is measured from the wall's start whatever the "measured from" says; a flipped profile
        // hangs before the distance as a horizontal one hangs below it, and a vertical REVEAL hangs before its distance
        // unless it is flipped (the other way round from a sweep)
        let before = if sw.kind == SweepKind::Reveal { !sw.flipped } else { sw.flipped };
        let u0 = if before { sw.distance - s } else { sw.distance };
        (u0, u0 + s)
    } else if sw.kind == SweepKind::Reveal {
        // OBSERVED (one L): the groove of a reveal at a join runs to the far side of the neighbour's groove: to the outer
        // corner of the footprint on the outside of the corner, and `profile - half thickness` past the end of the location
        // line on the inside. ENGINE CHOICE: the same at every join.
        let a = if fa < -eps { fa } else if fa > eps { -(s - hw) } else { 0.0 };
        let b = if fb > len + eps { fb } else if fb < len - eps { len + (s - hw) } else { len };
        (a, b)
    } else {
        (fa, fb)
    };
    let (z0, z1) = if sw.vertical {
        (base, top)
    } else {
        let z0 = base + if sw.flipped { sw.distance - s } else { sw.distance };
        (z0, z0 + s)
    };
    let p = |u: f64, v: f64| w.start + d * u + n * v;
    let (e0, e1) = if sw.vertical || sw.kind == SweepKind::Reveal { (u0, u1) } else { (xa, xb) };
    Some(Strip {
        corners: vec![p(u0, v0), p(u1, v0), p(u1, v1), p(u0, v1)],
        ext_corners: vec![p(e0, v0), p(e1, v0), p(e1, v1), p(e0, v1)],
        z0,
        z1,
        far: p(0.5 * len, 0.5 * (v0 + v1)),
        inner: (p(0.0, v0), d),
        outer: (p(0.0, v1), d),
        u: (u0, u1),
        v: {
            let sg = f64::from(sw.side_sign);
            ((sg * v0).min(sg * v1), (sg * v0).max(sg * v1))
        },
    })
}

fn region(poly: &[Vec2]) -> Option<Region> {
    Region::from_polygon(poly, &Tolerances::default()).ok()
}

fn area(rs: &[Region]) -> f64 {
    rs.iter().map(|r| r.area()).sum()
}

fn subtract(pieces: Vec<Region>, cutter: &Region) -> Vec<Region> {
    let tol = Tolerances::default();
    let mut out = Vec::new();
    for p in pieces {
        match p.difference(cutter, &tol) {
            Ok(d) => out.extend(d),
            Err(_) => out.push(p),
        }
    }
    out
}

fn intersect(a: (Vec2, Vec2), b: (Vec2, Vec2)) -> Option<Vec2> {
    let c = a.1.cross(b.1);
    if c.abs() < 1e-9 {
        return None;
    }
    let t = (b.0 - a.0).cross(b.1) / c;
    Some(a.0 + a.1 * t)
}

/// A sweep's solid: the pieces of its plan region, its z range and whether a mitre shaped it.
pub struct Body {
    pub pieces: Vec<Region>,
    pub z0: f64,
    pub z1: f64,
    pub mitred: bool,
}

/// Whether `other` (its footprint) reaches an end of the location line of `wall`.
fn touches_an_end(m: &BimModel, wall: ElementId, other: &crate::model::Wall) -> bool {
    let Some(w) = m.walls.get(&wall) else { return false };
    let Ok((_, _, _, ow)) = crate::regen::wall_frame(m, other) else { return false };
    let seg = other.end - other.start;
    let l2 = seg.dot(seg);
    [w.start, w.end].iter().any(|p| {
        let t = if l2 > 0.0 { ((*p - other.start).dot(seg) / l2).clamp(0.0, 1.0) } else { 0.0 };
        (*p - (other.start + seg * t)).length() <= ow / 2.0 + 1e-6
    })
}

/// The clipped region of a sweep (walls, doors), before any mitre.
fn clipped(m: &BimModel, sw: &WallSweep, st: &Strip, extended: bool) -> Option<Vec<Region>> {
    let mut pieces = vec![region(if extended { &st.ext_corners } else { &st.corners })?];
    if sw.kind == SweepKind::Sweep {
        // the other walls it runs into: a vertical sweep any wall; a horizontal one the walls joined at an END of its wall
        // (OBSERVED: a T join does not cut the sweep of the through wall, a corner join does)
        for (id, other) in &m.walls {
            if *id == sw.wall || !(sw.vertical || touches_an_end(m, sw.wall, other)) {
                continue;
            }
            if let Ok((fp, b, t)) = crate::regen::wall_view_body(m, other) {
                if t > st.z0 + 1e-9 && b < st.z1 - 1e-9 && fp.len() >= 3 {
                    if let Some(r) = region(&fp) {
                        pieces = subtract(pieces, &r);
                    }
                }
            }
        }
        // the openings of the host
        if sw.sweep_intersects_wall {
            let w = m.walls.get(&sw.wall)?;
            let (d, _, _, width) = crate::regen::wall_frame(m, w).ok()?;
            let n = Vec2::new(-d.y, d.x);
            for (_, _, r) in crate::regen::host_cuts(m, sw.wall) {
                if r.z_hi > st.z0 + 1e-9 && r.z_lo < st.z1 - 1e-9 {
                    let (a, b) = (w.start + d * r.u_lo, w.start + d * r.u_hi);
                    let h = n * (width / 2.0 + sw.offset + PROFILE_M + 0.5);
                    if let Some(cut) = region(&[a - h, b - h, b + h, a + h]) {
                        pieces = subtract(pieces, &cut);
                    }
                }
            }
        }
    }
    Some(pieces)
}

/// The body of a sweep.
pub fn body(m: &BimModel, sw: &WallSweep) -> Option<Body> {
    let st = strip(m, sw)?;
    let plain = clipped(m, sw, &st, false)?;
    let mut pieces = clipped(m, sw, &st, true)?;
    let mut mitred = false;
    if sw.kind == SweepKind::Sweep && !sw.vertical {
        for other in m.annot.sweeps.values() {
            if other.id == sw.id || other.wall == sw.wall || other.kind != SweepKind::Sweep || other.vertical {
                continue;
            }
            // only at a corner: each wall reaches an end of the other (OBSERVED: the sweeps of a T do not mitre)
            let (Some(ow), Some(mw)) = (m.walls.get(&other.wall), m.walls.get(&sw.wall)) else { continue };
            if !(touches_an_end(m, sw.wall, ow) && touches_an_end(m, other.wall, mw)) {
                continue;
            }
            let Some(ost) = strip(m, other) else { continue };
            if (ost.z0 - st.z0).abs() > 1e-9 || (ost.z1 - st.z1).abs() > 1e-9 {
                continue;
            }
            let Some(opieces) = clipped(m, other, &ost, true) else { continue };
            // the overlap of the two clipped strips
            let tol = Tolerances::default();
            let mut overlap = 0.0;
            for a in &pieces {
                for b in &opieces {
                    if let Ok(i) = a.intersection(b, &tol) {
                        overlap += area(&i);
                    }
                }
            }
            if overlap < 1e-9 {
                continue;
            }
            let (Some(p_in), Some(p_out)) = (intersect(st.inner, ost.inner), intersect(st.outer, ost.outer)) else { continue };
            let Some(dir) = (p_out - p_in).normalized() else { continue };
            // the half plane on this strip's side of the mitre line
            let side = dir.cross(st.far - p_in).signum();
            let nrm = Vec2::new(-dir.y, dir.x) * side;
            let big = 1.0e4;
            let half = region(&[p_in - dir * big, p_in + dir * big, p_in + dir * big + nrm * big, p_in - dir * big + nrm * big]);
            if let Some(h) = half {
                let mut next = Vec::new();
                for a in &pieces {
                    match a.intersection(&h, &tol) {
                        Ok(i) => next.extend(i),
                        Err(_) => next.push(a.clone()),
                    }
                }
                pieces = next;
                mitred = true;
            }
        }
    }
    // without a neighbour to mitre with, the strip stops at the end of the face
    if !mitred {
        pieces = plain;
    }
    Some(Body { pieces, z0: st.z0, z1: st.z1, mitred })
}

/// The measured state of a sweep or reveal.
#[derive(Clone, Debug, PartialEq)]
pub struct SweepState {
    pub bbox: ([f64; 3], [f64; 3]),
    pub solid_count: usize,
    pub volume: f64,
    pub surface_area: f64,
    pub faces: usize,
    pub edges: usize,
    pub endpoints: usize,
}

/// Whether a reveal cuts the wall it is on (its body is inside the wall's solid).
fn sweep_intersects_wall(m: &BimModel, sw: &WallSweep, b: &Body) -> bool {
    if sw.kind != SweepKind::Reveal || !sw.sweep_intersects_wall {
        return false;
    }
    let Some(w) = m.walls.get(&sw.wall) else { return false };
    let Ok((_, base, top)) = crate::regen::wall_view_body(m, w) else { return false };
    b.z1 > base + 1e-9 && b.z0 < top - 1e-9
}

pub fn sweep_state(m: &BimModel, id: ElementId) -> Option<SweepState> {
    let sw = m.annot.sweeps.get(&id)?;
    if m.walls.get(&sw.wall).map_or(false, |w| w.through.is_some()) {
        return crate::arc_sweeps::sweep_state(m, sw);
    }
    let b = body(m, sw)?;
    let pts: Vec<Vec2> = b.pieces.iter().flat_map(|r| r.outer_corner_points()).collect();
    if pts.is_empty() {
        return None;
    }
    let mut lo = [pts.iter().map(|p| p.x).fold(f64::INFINITY, f64::min), pts.iter().map(|p| p.y).fold(f64::INFINITY, f64::min), b.z0];
    let mut hi = [pts.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max), pts.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max), b.z1];
    // OBSERVED (one vertical reveal): its box reaches 0.03048 m (0.1 ft) beyond the face it is cut in
    if sw.kind == SweepKind::Reveal && sw.vertical {
        if let Some(w) = m.walls.get(&sw.wall) {
            if let Ok((d, _, _, _)) = crate::regen::wall_frame(m, w) {
                let n = Vec2::new(-d.y, d.x) * f64::from(sw.side_sign);
                for (axis, c) in [(0usize, n.x), (1usize, n.y)] {
                    if c > 1e-9 {
                        hi[axis] += REVEAL_BOX_OVERSHOOT_M * c;
                    } else if c < -1e-9 {
                        lo[axis] += REVEAL_BOX_OVERSHOOT_M * c;
                    }
                }
            }
        }
    }
    let cutting = sweep_intersects_wall(m, sw, &b);
    let hz = b.z1 - b.z0;
    let a = area(&b.pieces);
    let perim: f64 = b.pieces.iter().map(|r| r.outer_perimeter() + r.hole_perimeter()).sum();
    let (mut faces, mut edges, mut endpoints) = (0usize, 0usize, 0usize);
    for r in &b.pieces {
        let n = r.outer_corner_points().len();
        faces += n + 2;
        edges += 3 * n - if b.mitred { 2 } else { 0 };
        endpoints += 2 * n;
    }
    if cutting {
        return Some(SweepState { bbox: (lo, hi), solid_count: 0, volume: 0.0, surface_area: 0.0, faces: 0, edges: 0, endpoints: 0 });
    }
    // a reveal above or below the wall: the part of its body outside the wall
    let mut volume = a * hz;
    if sw.kind == SweepKind::Reveal {
        if let Some(w) = m.walls.get(&sw.wall) {
            if let Ok((_, base, top)) = crate::regen::wall_view_body(m, w) {
                let outside = (b.z1 - b.z0.max(top)).max(0.0) + (b.z1.min(base) - b.z0).max(0.0);
                volume = a * outside.min(hz);
            }
        }
    }
    Some(SweepState { bbox: (lo, hi), solid_count: 1, volume, surface_area: 2.0 * a + perim * hz, faces, edges, endpoints })
}

/// What the cutting reveals and sweeps do to a wall's solid.
pub struct CutEffect {
    pub volume: f64,
    pub surface: f64,
    pub faces: usize,
    pub edges: usize,
    pub vertices: usize,
}

/// The effect of the cutting sweeps and reveals on a wall: the volume and surface they remove (net of the wall's
/// openings) and the topology they add.
///
/// OBSERVED: the counts are those of [`crate::boxtopo`] (a groove along the wall, a notch at the top, overlapping cuts that
/// make one groove, a cut interrupted by a door, a vertical reveal crossing a horizontal one). ENGINE CHOICE: the wall is
/// taken as a box with square ends (a mitred end does not change what a cut adds).
pub fn reveal_effect(m: &BimModel, wall: &crate::model::Wall) -> Option<CutEffect> {
    if wall.through.is_some() {
        return None; // an arc wall: `crate::arc_sweeps::apply`
    }
    let cutting: Vec<&WallSweep> = m.annot.sweeps.values().filter(|s| s.wall == wall.id && s.sweep_intersects_wall).collect();
    if cutting.is_empty() {
        return None;
    }
    let (fp, base, top) = crate::regen::wall_view_body(m, wall).ok()?;
    let own = region(&fp)?;
    let (d, _, len, width) = crate::regen::wall_frame(m, wall).ok()?;
    let hw = width / 2.0;
    let mut boxes = Vec::new();
    let mut aabbs: Vec<crate::boxtopo::Aabb> = Vec::new();
    let tol = Tolerances::default();
    for r in &cutting {
        // a sweep outside the wall (also the wall as a view clips it) cuts nothing
        let Some(st) = strip(m, r) else { continue };
        let reg = region(&st.corners)?;
        // it must take something of the wall
        if !own.intersection(&reg, &tol).map_or(false, |v| v.iter().any(|x| x.area() > 1e-12)) {
            continue;
        }
        let bx = [st.u.0.max(0.0), st.u.1.min(len), st.v.0.max(-hw), st.v.1.min(hw), st.z0.max(base), st.z1.min(top)];
        if bx[1] <= bx[0] + 1e-9 || bx[3] <= bx[2] + 1e-9 || bx[5] <= bx[4] + 1e-9 {
            continue;
        }
        boxes.push((reg, st.z0, st.z1));
        aabbs.push(bx);
    }
    if boxes.is_empty() {
        return None;
    }
    // the openings of the wall, as plan boxes and as boxes of the wall's own frame
    let n = Vec2::new(-d.y, d.x);
    let mut open = Vec::new();
    let mut open_aabb: Vec<crate::boxtopo::Aabb> = Vec::new();
    for (_, _, r) in crate::regen::host_cuts(m, wall.id) {
        let (a, b) = (wall.start + d * r.u_lo, wall.start + d * r.u_hi);
        let h = n * (hw + 0.5);
        if let Some(reg) = region(&[a - h, b - h, b + h, a + h]) {
            open.push((reg, r.z_lo, r.z_hi));
        }
        open_aabb.push([r.u_lo, r.u_hi, -hw - 0.5, hw + 0.5, r.z_lo, r.z_hi]);
    }
    let (volume, surface) = crate::crossings::removal_by_boxes(&own, base, top, &open, &boxes);
    if volume < 1e-9 {
        return None;
    }
    let t = crate::boxtopo::cut_delta([0.0, len, -hw, hw, base, top], &open_aabb, &aabbs);
    Some(CutEffect { volume, surface, faces: t.faces, edges: t.edges, vertices: t.vertices })
}

/// Apply the reveals to the regenerated wall.
pub fn apply_reveals(m: &BimModel, wall: &crate::model::Wall, s: &mut crate::regen::WallState) {
    if let Some(e) = reveal_effect(m, wall) {
        s.volume -= e.volume;
        s.surface_area += e.surface;
        s.solid_face_count += e.faces;
        s.solid_edge_count += e.edges;
        s.solid_vertex_count += e.vertices;
    }
}

#[allow(dead_code)]
fn _unused(p: [f64; 2]) -> Vec2 {
    v2(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ops, CompoundStructure, Layer, LayerFunction};

    fn fixture(walls: &[((f64, f64), (f64, f64))]) -> (crate::BimDocument, Vec<ElementId>) {
        let mut d = crate::new_document();
        let ids = d
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let wt = ops::create_wall_type(m, cx, "W", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
                let mut v = Vec::new();
                for (a, b) in walls {
                    v.push(ops::create_wall(
                        m,
                        cx,
                        ops::NewWall { wall_type: wt, level: l, start: Vec2::new(a.0, a.1), end: Vec2::new(b.0, b.1), height: 3.0, base_offset: 0.0, flip: false, structural: false },
                    )?);
                }
                Ok(v)
            })
            .value
            .unwrap();
        (d, ids)
    }

    fn make(d: &mut crate::BimDocument, wall: ElementId, kind: SweepKind, f: impl FnOnce(&mut SweepInfo)) -> ElementId {
        d.transact("s", false, |m, cx| {
            let ty = use_type(m, cx, kind, 0)?;
            let mut info = SweepInfo::new(kind, false);
            f(&mut info);
            create_wall_sweep(m, cx, wall, ty, info)
        })
        .value
        .unwrap()
    }

    #[test]
    fn a_sweep_outside_its_wall_has_no_body_and_posts_a_warning() {
        // OBSERVED: distances of -0.5 and 3.5 on a 3 m wall; 3.0 is fine
        let (mut d, w) = fixture(&[((0.0, 0.0), (6.0, 0.0))]);
        let ty = d.transact("t", false, |m, cx| Ok(types_of(m, cx, SweepKind::Sweep)[0])).value.unwrap();
        let mut make = |dist: f64| {
            let mut i = SweepInfo::new(SweepKind::Sweep, false);
            i.distance = dist;
            d.transact("s", false, |m, cx| create_wall_sweep(m, cx, w[0], ty, i.clone()))
        };
        let (far, near) = (make(3.5), make(3.0));
        assert!(far.committed() && far.failures.iter().any(|f| f.code == codes::SWEEP_OUTSIDE_WALL));
        assert!(near.committed() && near.failures.is_empty());
        let (far_id, near_id) = (far.value.unwrap(), near.value.unwrap());
        assert!(sweep_state(d.model(), far_id).is_none());
        assert!(sweep_state(d.model(), near_id).is_some());
    }

    #[test]
    fn a_sweep_is_a_152_mm_square_along_the_wall_and_a_flipped_one_hangs_below_its_distance() {
        // OBSERVED: volume 0.13935 m3, surface 3.70405, 6 faces, 12 edges, 8 vertices, box y 0.1 .. 0.2524, z 0.5 .. 0.6524
        let (mut d, w) = fixture(&[((0.0, 0.0), (6.0, 0.0))]);
        let a = make(&mut d, w[0], SweepKind::Sweep, |i| i.distance = 0.5);
        let s = sweep_state(d.model(), a).unwrap();
        assert!((s.volume - 0.13935456).abs() < 1e-7 && (s.surface_area - 3.70405152).abs() < 1e-7, "{s:?}");
        assert_eq!((s.faces, s.edges, s.endpoints), (6, 12, 8));
        assert!((s.bbox.0[1] - 0.1).abs() < 1e-9 && (s.bbox.1[1] - 0.2524).abs() < 1e-9 && (s.bbox.0[2] - 0.5).abs() < 1e-9);
        let b = make(&mut d, w[0], SweepKind::Sweep, |i| {
            i.distance = 2.0;
            i.flipped = true
        });
        assert!((sweep_state(d.model(), b).unwrap().bbox.0[2] - 1.8476).abs() < 1e-9);
        let c = make(&mut d, w[0], SweepKind::Sweep, |i| {
            i.distance = 1.0;
            i.exterior = false
        });
        assert!((sweep_state(d.model(), c).unwrap().bbox.1[1] + 0.1).abs() < 1e-9);
    }

    #[test]
    fn a_reveal_cuts_the_wall_and_a_sweep_does_not() {
        // OBSERVED: wall 3.6 m3, 6 / 12 / 8, surface 39.6; with the reveal 3.46065, 9 / 24 / 16, 41.38235, Area unchanged
        let (mut d, w) = fixture(&[((0.0, 0.0), (6.0, 0.0))]);
        make(&mut d, w[0], SweepKind::Sweep, |i| i.distance = 0.5);
        let ws = crate::regen::wall_state(d.model(), &d.model().walls[&w[0]]).unwrap();
        assert!((ws.volume - 3.6).abs() < 1e-9 && ws.solid_face_count == 6);
        let r = make(&mut d, w[0], SweepKind::Reveal, |i| i.distance = 1.0);
        let ws = crate::regen::wall_state(d.model(), &d.model().walls[&w[0]]).unwrap();
        assert!((ws.volume - 3.46065).abs() < 1e-5 && (ws.surface_area - 41.38235).abs() < 1e-4, "{} {}", ws.volume, ws.surface_area);
        assert_eq!((ws.solid_face_count, ws.solid_edge_count, ws.solid_vertex_count), (9, 24, 16));
        assert!((ws.area - 18.0).abs() < 1e-9);
        // the reveal that cuts has no solid
        assert_eq!(sweep_state(d.model(), r).unwrap().solid_count, 0);
    }

    #[test]
    fn a_sweep_is_trimmed_by_the_wall_it_runs_into_and_mitred_against_its_neighbour() {
        // OBSERVED (two joined walls): alone 0.13703 m3 (5.9 m); both 0.13526 and 0.11204, 10 edges each
        let (mut d, w) = fixture(&[((0.0, 0.0), (6.0, 0.0)), ((6.0, 0.0), (6.0, 5.0))]);
        let a = make(&mut d, w[0], SweepKind::Sweep, |i| i.distance = 0.5);
        let s = sweep_state(d.model(), a).unwrap();
        assert!((s.volume - 0.137033).abs() < 1e-5 && (s.bbox.1[0] - 5.9).abs() < 1e-9, "{s:?}");
        assert_eq!(s.edges, 12);
        let b = make(&mut d, w[1], SweepKind::Sweep, |i| i.distance = 0.5);
        let (sa, sb) = (sweep_state(d.model(), a).unwrap(), sweep_state(d.model(), b).unwrap());
        assert!((sa.volume - 0.135263).abs() < 1e-5 && (sb.volume - 0.112037).abs() < 1e-5, "{} {}", sa.volume, sb.volume);
        assert_eq!((sa.edges, sb.edges), (10, 10));
    }

    #[test]
    fn two_sweeps_at_one_place_warn_and_the_wall_deletion_takes_them() {
        let (mut d, w) = fixture(&[((0.0, 0.0), (6.0, 0.0))]);
        make(&mut d, w[0], SweepKind::Sweep, |i| i.distance = 0.5);
        let out = d.transact("s2", false, |m, cx| {
            let ty = use_type(m, cx, SweepKind::Sweep, 0)?;
            let mut i = SweepInfo::new(SweepKind::Sweep, false);
            i.distance = 0.5;
            create_wall_sweep(m, cx, w[0], ty, i)
        });
        assert!(out.committed() && out.failures.iter().any(|f| f.code == codes::SWEEP_OVERLAP));
        assert_eq!(d.model().annot.sweeps.len(), 2);
        assert!(d.transact("x", false, |m, cx| use_type(m, cx, SweepKind::Sweep, 1)).value.is_none());
    }
}
