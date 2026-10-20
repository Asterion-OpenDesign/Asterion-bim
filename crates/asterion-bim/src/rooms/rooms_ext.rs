//! OBSERVED through the public API and 36b.
//!
//! Everything else is an ENGINE CHOICE named with its question.

use crate::model::BimModel;
use crate::regen::RoomSegment;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use asterion_geom::Vec2;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The limit offset of a room made by `NewRoom(Phase)`, in metres. OBSERVED: 4.0, kept when the room is placed (its volume
/// is then area x 4.0). ENGINE CHOICE: whether it comes from the template (its "Level 2" is at 4.0 m) is not specified.
pub const UNPLACED_ROOM_LIMIT_OFFSET_M: f64 = 4.0;

pub mod codes {
    pub const NO_CIRCUIT: &str = "room.no_circuit_at_point";
    pub const NOT_A_ROOM: &str = "room.not_a_room";
    pub const ZERO_HEIGHT: &str = "room.zero_height";
    pub const UNPLACED_WARNING: &str = "room.unplaced_remains_in_project";
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoomsExt {
    /// Rooms that are not placed (no location, no boundary, Area 0).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub unplaced: BTreeSet<ElementId>,
    /// The box an unplaced room still reports: the one it had when it was unplaced (OBSERVED: `Unplace` leaves the room's
    /// bounding box as it was). `(min, max)`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub kept_box: BTreeMap<ElementId, ([f64; 3], [f64; 3])>,
    /// Column types whose family has a solid (`give_family_solid`). OBSERVED: a column of the template family, which has no
    /// solid, does not bound a room whatever its Room Bounding flag; once the family has an extrusion it does.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub solid_column_types: BTreeSet<ElementId>,
}

impl RoomsExt {
    pub fn is_empty(&self) -> bool {
        self.unplaced.is_empty() && self.kept_box.is_empty() && self.solid_column_types.is_empty()
    }
}

/// Whether a room is placed (a probe room that is not in the model, such as the Room tool's preview, counts as placed).
pub fn is_placed(m: &BimModel, id: ElementId) -> bool {
    !m.rooms_ext.unplaced.contains(&id)
}

/// Whether a room has a level (a room made by `NewRoom(Phase)` has none until it is placed).
pub fn has_level(m: &BimModel, id: ElementId) -> bool {
    m.rooms.get(&id).map_or(false, |r| m.levels.contains_key(&r.level))
}

/// The footprints of the columns that bound the rooms of `level`: Room Bounding on (Spec M-03), a family with a solid
/// (OBSERVED), and a body that spans the level's computation plane (Spec M-03, as for walls).
pub fn bounding_column_footprints(m: &BimModel, level: ElementId) -> Vec<(ElementId, Vec<Vec2>)> {
    let Some(l) = m.levels.get(&level) else { return Vec::new() };
    let plane = l.elevation + crate::datum_extents::computation_height(m, level);
    let tol = asterion_geom::Tolerances::default().vertex;
    let mut out = Vec::new();
    for c in m.columns.values() {
        if !c.bounds_rooms(crate::columns::AREA_SETTINGS_COLUMNS_ENABLED) || !m.rooms_ext.solid_column_types.contains(&c.type_id) {
            continue;
        }
        let Ok(s) = crate::columns::column_state(m, c) else { continue };
        if plane < s.base_z - tol || plane > s.top_z + tol {
            continue;
        }
        out.push((c.id, s.footprint.clone()));
    }
    out
}

/// The column whose footprint has the boundary segment `a`-`b` on one of its edges.
pub fn column_host(m: &BimModel, level: ElementId, a: Vec2, b: Vec2) -> Option<ElementId> {
    let mid = (a + b) * 0.5;
    let dir = (b - a).normalized()?;
    for (id, fp) in bounding_column_footprints(m, level) {
        let n = fp.len();
        for i in 0..n {
            let (p, q) = (fp[i], fp[(i + 1) % n]);
            let Some(e) = (q - p).normalized() else { continue };
            if e.cross(dir).abs() > 1e-6 {
                continue;
            }
            let off = (mid - p).cross(e).abs();
            let along = (mid - p).dot(e);
            if off < 1e-6 && along > -1e-6 && along < (q - p).length() + 1e-6 {
                return Some(id);
            }
        }
    }
    None
}

/// The point used to decide which region holds a room: nudged off a boundary it lies on, downward (and to the left when it
/// lies on a horizontal edge's end). ENGINE CHOICE (two samples: the lower half).
pub fn off_boundary(p: Vec2, r: &(Vec<Vec2>, Vec<Vec<Vec2>>)) -> Vec2 {
    let on = |poly: &[Vec2]| {
        let n = poly.len();
        (0..n).any(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            let d = b - a;
            let len = d.length();
            len > 0.0 && ((p - a).cross(d) / len).abs() < 1e-9 && (p - a).dot(d) >= -1e-9 && (p - a).dot(d) <= len * len + 1e-9
        })
    };
    if on(&r.0) || r.1.iter().any(|h| on(h)) {
        Vec2::new(p.x - 1e-7, p.y - 1e-6)
    } else {
        p
    }
}

fn reverse_of(a: &RoomSegment, b: &RoomSegment) -> bool {
    (a.start - b.end).length() < 1e-7 && (a.end - b.start).length() < 1e-7
}

/// Remove the spikes of a loop (a segment followed by its own reverse: a separation line that ends free inside the region)
/// and return their total length (both faces). The vertex where the spike started stays (OBSERVED: the wall the line starts
/// from keeps two segments).
pub fn remove_spikes(segs: &mut Vec<RoomSegment>) -> f64 {
    let mut removed = 0.0;
    // the flat end of a separation's sliver collapses to a point once its faces are put back on the line
    segs.retain(|s| s.arc.is_some() || s.length() > 1e-7);
    loop {
        let n = segs.len();
        if n < 3 {
            break;
        }
        let Some(i) = (0..n).find(|i| reverse_of(&segs[*i], &segs[(i + 1) % n])) else { break };
        let j = (i + 1) % n;
        removed += segs[i].length() + segs[j].length();
        let (hi, lo) = if i > j { (i, j) } else { (j, i) };
        segs.remove(hi);
        segs.remove(lo);
    }
    removed
}

/// Split the separation segments of a loop at the ends of other separations that meet them from the far side. OBSERVED: a
/// separation started on another one splits it in the loop of the room on the other side too (a loop of 5 segments, the
/// first separation in two pieces). ENGINE CHOICE: walls are not split this way. Returns whether a segment was split.
pub fn split_at_separation_ends(m: &BimModel, level: ElementId, segs: &mut Vec<RoomSegment>) -> bool {
    let ends: Vec<(ElementId, Vec2)> = m
        .room_separations
        .values()
        .filter(|s| s.level == level)
        .flat_map(|s| [(s.id, s.start), (s.id, s.end)])
        .collect();
    let mut out: Vec<RoomSegment> = Vec::with_capacity(segs.len());
    let mut split = false;
    for s in segs.iter() {
        let host_is_sep = s.host.map_or(false, |h| m.room_separations.contains_key(&h));
        if !host_is_sep || s.arc.is_some() {
            out.push(s.clone());
            continue;
        }
        let d = s.end - s.start;
        let len = d.length();
        let mut cuts: Vec<f64> = ends
            .iter()
            .filter(|(id, _)| Some(*id) != s.host)
            .filter_map(|(_, p)| {
                let t = (*p - s.start).dot(d) / (len * len);
                let off = ((*p - s.start).cross(d) / len).abs();
                (off < 1e-6 && t > 1e-6 && t < 1.0 - 1e-6).then_some(t)
            })
            .collect();
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        let mut a = s.start;
        for t in cuts {
            let b = s.start + d * t;
            out.push(RoomSegment { start: a, end: b, host: s.host, arc: None });
            a = b;
            split = true;
        }
        out.push(RoomSegment { start: a, end: s.end, host: s.host, arc: None });
    }
    *segs = out;
    split
}

/// Split a finish loop at its bridges (a separation traversed both ways, joining the outline to a free-standing body): the
/// outer loop without them, and the inner loops they led to, each started right after its bridge (OBSERVED: the centre
/// boundary reads the walls' loop and the column's loop apart, the bridging separation in neither).
pub fn split_bridges(m: &BimModel, segs: &[RoomSegment]) -> (Vec<RoomSegment>, Vec<Vec<RoomSegment>>) {
    let n = segs.len();
    for i in 0..n {
        if !segs[i].host.map_or(false, |h| m.room_separations.contains_key(&h)) {
            continue;
        }
        if let Some(j) = (i + 2..n).find(|j| reverse_of(&segs[i], &segs[*j])) {
            let mut outer: Vec<RoomSegment> = segs[..i].to_vec();
            outer.extend_from_slice(&segs[j + 1..]);
            let inner: Vec<RoomSegment> = segs[i + 1..j].to_vec();
            let (outer, mut more) = split_bridges(m, &outer);
            more.insert(0, inner);
            return (outer, more);
        }
    }
    (segs.to_vec(), Vec::new())
}

/// The centre boundary of a loop that has column faces in it: a straight host's segment runs between the corners where its
/// host line meets its neighbours'; a column's segment is its own face (OBSERVED: the centre loop of a room with a column
/// notch runs on the column's faces, and the wall's centre line stops on the face's line).
pub fn center_segments_with_columns(m: &BimModel, segs: &[RoomSegment], host_line: impl Fn(ElementId) -> Option<(Vec2, Vec2)>) -> Vec<RoomSegment> {
    let n = segs.len();
    let line_of = |i: usize| -> (Vec2, Vec2) {
        let s = &segs[i];
        match s.host {
            Some(h) if m.columns.contains_key(&h) => (s.start, s.end),
            Some(h) => host_line(h).unwrap_or((s.start, s.end)),
            None => (s.start, s.end),
        }
    };
    let corner = |i: usize| -> Vec2 {
        let ((p0, p1), (q0, q1)) = (line_of(i), line_of((i + 1) % n));
        let (d1, d2) = (p1 - p0, q1 - q0);
        let den = d1.cross(d2);
        if den.abs() > 1e-12 {
            p0 + d1 * ((q0 - p0).cross(d2) / den)
        } else {
            // two pieces of one host (a wall split where a separation started): the finish corner moved onto the host line
            // (OBSERVED: the wall's centre segments meet at (0, 2) for a finish corner at (0.1, 2))
            let e = segs[i].end;
            match d1.normalized() {
                Some(u) => p0 + u * (e - p0).dot(u),
                None => e,
            }
        }
    };
    let corners: Vec<Vec2> = (0..n).map(corner).collect();
    (0..n).map(|i| RoomSegment { start: corners[(i + n - 1) % n], end: corners[i], host: segs[i].host, arc: None }).collect()
}

/// OBSERVED: the middle of the circuit's extent measured on the centre lines of its bounds (the halves 0..3 and 3..6 of a 6
/// m room split by a separation line give x = 1.5 and 4.5, where the finish region's centroids are 1.55 and 4.45).
pub fn auto_point(m: &BimModel, level: ElementId, phase: u8, poly: &[Vec2], islands: &[Vec<Vec2>]) -> Vec2 {
    let inside = |p: Vec2| asterion_geom::region::polygon_contains_point(poly, p) && !islands.iter().any(|h| asterion_geom::region::polygon_contains_point(h, p));
    let center = crate::regen::region_center_loop(m, level, phase, poly);
    if !center.is_empty() {
        let (mut lo, mut hi) = (Vec2::new(f64::INFINITY, f64::INFINITY), Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY));
        for s in &center {
            for p in [s.start, s.end] {
                lo = Vec2::new(lo.x.min(p.x), lo.y.min(p.y));
                hi = Vec2::new(hi.x.max(p.x), hi.y.max(p.y));
            }
        }
        let c = (lo + hi) * 0.5;
        if inside(c) {
            return c;
        }
    }
    let c = crate::regen::region_seed(poly);
    if inside(c) {
        return c;
    }
    poly[0]
}

/// `NewRoom(Phase)`: a room with no level and no location (OBSERVED: `Location` null, Area 0, no boundary, limit offset
/// [`UNPLACED_ROOM_LIMIT_OFFSET_M`]). The name and number are set as given (the harness writes both).
pub fn create_unplaced_room(m: &mut BimModel, cx: &mut TxnContext<'_>, phase: u8, name: Option<&str>, number: Option<&str>) -> Result<ElementId, Failure> {
    let id = cx.new_id();
    let auto = crate::ops::next_room_number(m);
    m.rooms.insert(
        id,
        crate::model::Room {
            id,
            level: ElementId::INVALID,
            point: Vec2::new(0.0, 0.0),
            base_offset: 0.0,
            limit_offset: UNPLACED_ROOM_LIMIT_OFFSET_M,
            upper_limit: None,
            name: name.unwrap_or(crate::ops::DEFAULT_ROOM_NAME).to_owned(),
            number: number.map(str::to_owned).unwrap_or(auto),
            tagged: false,
        },
    );
    m.rooms_ext.unplaced.insert(id);
    if phase != crate::phases::DEFAULT_PHASE {
        m.phases.rooms.insert(id, phase);
    }
    Ok(id)
}

/// `Room.Unplace`. OBSERVED: the room stays in the project, unplaced (Area 0, no boundary), its limit offset and height
/// read 0, its upper limit none, its box is kept; the warning "A Room was deleted from all model views but still remains in
/// this project" is posted. The redundant room of its region takes the region (Spec M-04, the next lowest id).
pub fn unplace_room(m: &mut BimModel, cx: &mut TxnContext<'_>, id: ElementId) -> Result<(), Failure> {
    let r = m.rooms.get(&id).cloned().ok_or_else(|| Failure::error(codes::NOT_A_ROOM, "The element is not a room.").on(&[id]))?;
    if is_placed(m, id) {
        if let Ok(s) = crate::regen::room_state(m, &r) {
            m.rooms_ext.kept_box.insert(id, (s.bbox_min.to_array(), s.bbox_max.to_array()));
        }
    }
    m.rooms_ext.unplaced.insert(id);
    let rr = m.rooms.get_mut(&id).expect("checked");
    rr.limit_offset = 0.0;
    rr.upper_limit = None;
    cx.report(Failure::warning(codes::UNPLACED_WARNING, "A Room was deleted from all model views but still remains in this project."));
    Ok(())
}

/// `NewRoom(room, circuit)` with the circuit of the room's phase that holds `point` on `level`. OBSERVED: the same element
/// comes back, placed at the circuit's own interior point ([`auto_point`]); a circuit that already has a room takes it as a
/// redundant room (Spec M-04); a room that was unplaced (height 0) is refused at commit "Room must have a height greater
/// than 0". The placed room is tagged (OBSERVED: one more Room Tag per placement).
pub fn place_room(m: &mut BimModel, cx: &mut TxnContext<'_>, id: ElementId, level: ElementId, point: Vec2) -> Result<(), Failure> {
    if !m.rooms.contains_key(&id) {
        return Err(Failure::error(codes::NOT_A_ROOM, "The element is not a room.").on(&[id]));
    }
    m.level(level)?;
    let phase = crate::phases::room_phase(m, id);
    let regions = crate::regen::enclosed_regions_full_in(m, level, phase);
    let Some((poly, islands)) = regions.iter().find(|(p, isl)| {
        asterion_geom::region::polygon_contains_point(p, point) && !isl.iter().any(|h| asterion_geom::region::polygon_contains_point(h, point))
    }) else {
        return Err(Failure::error(codes::NO_CIRCUIT, "No circuit of the level's plan topology holds the point."));
    };
    let at = auto_point(m, level, phase, poly, islands);
    {
        let r = m.rooms.get_mut(&id).expect("checked");
        r.level = level;
        r.point = at;
        r.tagged = true;
    }
    m.rooms_ext.unplaced.remove(&id);
    m.rooms_ext.kept_box.remove(&id);
    let r = m.rooms[&id].clone();
    if let Ok(s) = crate::regen::room_state(m, &r) {
        if s.unbounded_height * crate::spec::FEET_PER_METRE <= 1e-9 {
            cx.report(Failure::error(codes::ZERO_HEIGHT, "Room must have a height greater than 0.").on(&[id]));
        }
    }
    Ok(())
}

/// `give_family_solid`: the column family of a type gets an extrusion (the harness's `EditFamily` + `NewExtrusion` +
/// `LoadFamily`), so its columns bound rooms. The extrusion's size is the type's width and depth in the measurements
/// (0.6 x 0.6); the engine keeps the type's size.
pub fn give_column_family_solid(m: &mut BimModel, column_type: ElementId) -> Result<(), Failure> {
    m.column_type(column_type)?;
    m.rooms_ext.solid_column_types.insert(column_type);
    Ok(())
}

/// Forget what deleted rooms stored here.
pub fn elements_deleted(m: &mut BimModel, gone: &[ElementId]) {
    for g in gone {
        m.rooms_ext.unplaced.remove(g);
        m.rooms_ext.kept_box.remove(g);
        m.rooms_ext.solid_column_types.remove(g);
    }
}
