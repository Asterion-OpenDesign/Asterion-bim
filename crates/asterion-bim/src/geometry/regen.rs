//! Deterministic regeneration: derived wall state is a pure function of the
//! persistent model. Nothing derived is cached, so regeneration can never be stale.

use crate::cut;
use crate::model::{
    BimModel, HostedInstance, InsertKind, Slab, SlabAnchor, SlabRole, Wall, WallOpening,
};
use crate::spec::codes;
use asterion_core::{ElementId, Failure};
use crate::roof::{roof_solid, RoofInput, RoofSolid};
use asterion_geom::{Polycurve, Prism, Region, Tolerances, Vec2, Vec3};

/// Regenerated state of one slab - floor, ceiling or roof (all SI units).
///
/// Nothing here is stored. A thickness change, a level move or a slope edit recomputes every field
/// with zero bookkeeping, which is round 9's central measured lesson applied to slabs.
#[derive(Clone, Debug, PartialEq)]
pub struct SlabState {
    pub role: SlabRole,
    pub thickness: f64,
    /// Highest point of the slab's solid.
    pub top_z: f64,
    /// Lowest point of the slab's solid.
    pub bottom_z: f64,
    /// Plan (projected) area of the boundary polygon.
    pub plan_area: f64,
    /// The reported Area.
    ///
    /// OBSERVED for a floor and a ceiling: the plan area. OBSERVED for a roof: the **underside**, inclined, up to the
    /// cut-off - `24 x sqrt(1.25) = 26.8328` for a 0.5 slope, and `6 x 2 x sqrt(1.25) = 13.4164` once a cut-off clips it.
    pub area: f64,
    /// Boundary length in plan. A roof does not report it (the reference's roof has no Perimeter).
    pub perimeter: f64,
    /// OBSERVED: derived from the SOLID, not from `area * thickness` - a cut roof breaks that identity (`Area` 13.4164,
    /// `Volume` 3.3499, `Area x t` 4.0249).
    pub volume: f64,
    pub surface_area: f64,
    /// The reported Slope: the common value of the slope-defining segments, `None` when nothing
    /// defines one or the definers disagree (OBSERVED: 0 for a mixed-slope hip).
    pub slope: Option<f64>,
    /// True when more than one segment defines a slope: a gable or a hip.
    pub multi_plane: bool,
    /// Flat floors and ceilings keep the exact `Prism` path that produced round 6's 222/222.
    pub solid: Option<Prism>,
    /// Roofs: the lower-envelope solid with its cut-off (see [`crate::roof`]).
    pub sloped: Option<RoofSolid>,
    pub bbox_min: Vec3,
    pub bbox_max: Vec3,
    pub face_count: usize,
    pub edge_count: usize,
    pub vertex_count: usize,
    /// Slope-defining ARC edges of a roof that the body leaves out (their segmentation is not specified:). 0 for every
    /// other slab.
    pub skipped_arc_slopes: usize,
}

/// The elevation of a roof's cut-off plane, when it has one and the level still exists.
pub fn roof_cutoff_z(model: &BimModel, f: &Slab) -> Option<f64> {
    let (level, offset) = f.cutoff?;
    model.levels.get(&level).map(|l| l.elevation + offset)
}

// ------------------------------------------------------------------ rooms
//
// A room is DERIVED: its enclosure, area, perimeter, volume and boundary are recomputed from the walls on every read.
//
// With state derived on read, the same function of the model. Spec M-01, M-01b: the Finish boundary is each room-bounding
// vertical wall's path curve (its centreline of the total width, so it does NOT depend on the Location Line parameter)
// offset by half the total wall width toward the room; the Center boundary is the path curves themselves. UNKNOWN (Spec
// M-01): the two core locations (CoreCenter, CoreBoundary) are not modelled; the sign convention of the offset; the corner
// cleaning, so the boundary here is the hole of the union of the wall bodies (equal for single-layer rectangular rooms -
// the only case corroborated); the non-vertical walls (wall-shape path); and the rejection of a loop that is not continuous
// within the short-curve tolerance.

/// One boundary segment of a room, with the element that hosts it.
#[derive(Clone, Debug, PartialEq)]
pub struct RoomSegment {
    pub start: Vec2,
    pub end: Vec2,
    pub host: Option<ElementId>,
    /// The segment is an ARC from `start` to `end` on this circle (a segment on an arc wall's face or
    /// location curve, `crate::arc_rooms`); `None` is a straight segment.
    pub arc: Option<Arc2>,
}

impl RoomSegment {
    pub fn length(&self) -> f64 {
        match &self.arc {
            Some(a) => a.radius * a.sweep.abs(),
            None => (self.end - self.start).length(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RoomState {
    /// Whether the seed lies in an enclosed region at all.
    pub enclosed: bool,
    /// OBSERVED: when several rooms share one region, ONE keeps the area and the rest report 0
    /// while still returning their boundary.
    pub redundant: bool,
    pub area: f64,
    /// At the `Finish` face (the value the room reports).
    pub perimeter: f64,
    pub perimeter_center: f64,
    pub volume: f64,
    pub unbounded_height: f64,
    pub level_z: f64,
    pub finish: Vec<RoomSegment>,
    pub center: Vec<RoomSegment>,
    /// The inner loops (islands such as a free-standing column), Finish and Center (`crate::rooms_ext`).
    pub islands_finish: Vec<Vec<RoomSegment>>,
    pub islands_center: Vec<Vec<RoomSegment>>,
    pub bbox_min: Vec3,
    pub bbox_max: Vec3,
}

/// Half width of the sliver that stands in for a zero-thickness separation line.
const SEPARATION_HALF_WIDTH: f64 = 1.0e-3;

/// The level's room computation elevation offset, in metres: the height above the level at which the plane that decides
/// what bounds a room is taken (Spec M-03: z = level elevation + computation offset; Spec M-07: the same offset is what a
/// room's base offset is compared with).
///
/// UNKNOWN (Spec M-07): the writer of the offset and the value shipped templates carry; the engine's levels hold no such
/// field, and 0 is the value the harness reports for every level it measured (OBSERVED).
pub const ROOM_COMPUTATION_OFFSET_M: f64 = 0.0;

/// Whether a wall bounds the rooms of `level` (Spec M-03, wall branch).
///
/// Where the plane is clearly inside or clearly outside the span the rule decides; within the vertex-tolerance band of a
/// face the engine keeps its previous test (the wall is based on that level): UNKNOWN (Spec M-03) - the rule says "inside
/// ...
pub(crate) fn wall_bounds_level(model: &BimModel, w: &Wall, level: ElementId) -> bool {
    // inside `with_region_memo` (a batch over an immutable model) the answer for a (wall, level) is computed once: an arc wall's
    // answer needs its full state (the integrations of its quantities), which is milliseconds per wall
    let key = (model as *const BimModel as usize, model.walls.version(), model.levels.version(), model.wall_types.version(), w.id.0, level.0);
    if let Some(hit) = BOUNDS_MEMO.with(|m| m.borrow().as_ref().and_then(|map| map.get(&key).copied())) {
        return hit;
    }
    let answer = wall_bounds_level_uncached(model, w, level);
    BOUNDS_MEMO.with(|m| {
        if let Some(map) = m.borrow_mut().as_mut() {
            map.insert(key, answer);
        }
    });
    answer
}

thread_local! {
    static BOUNDS_MEMO: std::cell::RefCell<Option<std::collections::HashMap<(usize, u64, u64, u64, u64, u64), bool>>> = const { std::cell::RefCell::new(None) };
}

fn wall_bounds_level_uncached(model: &BimModel, w: &Wall, level: ElementId) -> bool {
    if !w.room_bounding {
        return false;
    }
    let Some(l) = model.levels.get(&level) else { return false };
    let own_level = w.base_level == level;
    // Only the z span of the body is needed. For a straight wall it comes from the wall's constraints (the same arithmetic as
    // straight_body, with no regeneration of joins, openings and the exact boolean cut: that cost minutes per refresh of a real
    // document, once per wall for every boundary segment of every room); the two lookups below are exactly what makes
    // wall_state_base fail for a straight wall. An arc wall keeps the full state.
    let (base_z, top_z) = if w.through.is_none() {
        if wall_frame(model, w).is_err() {
            return own_level;
        }
        match wall_vertical_span(model, w) {
            Ok(span) => span,
            Err(_) => return own_level,
        }
    } else {
        let Ok(s) = wall_state_base(model, w) else { return own_level };
        (s.base_z, s.top_z)
    };
    let plane = l.elevation + crate::datum_extents::computation_height(model, level);
    let tol = Tolerances::default().vertex;
    if plane < base_z - tol || plane > top_z + tol {
        return false;
    }
    if plane > base_z + tol && plane < top_z - tol {
        return true;
    }
    own_level
}

/// Every enclosed region on a level, as counter-clockwise polygons, in a stable order
/// (left to right, then bottom to top).
///
/// A region is a HOLE of the union of the room-bounding walls' bodies and the separation lines.
/// A door changes nothing because a wall's body has no opening in its footprint (OBSERVED, P9).
pub fn enclosed_regions(model: &BimModel, level: ElementId) -> Vec<Vec<Vec2>> {
    enclosed_regions_in(model, level, crate::phases::DEFAULT_PHASE)
}

/// [`enclosed_regions`] for the rooms of one phase: only the walls that stand in that phase bound them
/// (`crate::phases::exists_in_phase`, OBSERVED, ENGINE CHOICE).
pub fn enclosed_regions_in(model: &BimModel, level: ElementId, phase: u8) -> Vec<Vec<Vec2>> {
    enclosed_regions_full_in(model, level, phase).into_iter().map(|(outer, _)| outer).collect()
}

/// [`enclosed_regions_in`] with, for every region, the ISLANDS inside it: the bodies of the union that stand free in the
/// region (a room-bounding column, OBSERVED: the room keeps an inner loop on the column's faces), each as a clockwise
/// polygon.
pub fn enclosed_regions_full_in(model: &BimModel, level: ElementId, phase: u8) -> Vec<(Vec<Vec2>, Vec<Vec<Vec2>>)> {
    // inside `with_region_memo` the regions of a (level, phase) are computed once for the whole batch
    let key = region_memo_key(model, level, phase);
    if let Some(hit) = REGION_MEMO.with(|m| m.borrow().as_ref().and_then(|map| map.get(&key).cloned())) {
        return hit;
    }
    let out = enclosed_regions_full_in_uncached(model, level, phase);
    REGION_MEMO.with(|m| {
        if let Some(map) = m.borrow_mut().as_mut() {
            map.insert(key, out.clone());
        }
    });
    out
}

type RegionKey = (usize, u64, u64, u64, u64, u64, u64, u8);
type Regions = Vec<(Vec<Vec2>, Vec<Vec<Vec2>>)>;

thread_local! {
    static REGION_MEMO: std::cell::RefCell<Option<std::collections::HashMap<RegionKey, Regions>>> = const { std::cell::RefCell::new(None) };
}

/// The memo key: which model (by address) and the versions of every map the regions are derived from, so that even a model
/// mutated between two calls inside one scope could never be answered from a stale entry.
fn region_memo_key(model: &BimModel, level: ElementId, phase: u8) -> RegionKey {
    (model as *const BimModel as usize, model.walls.version(), model.room_separations.version(), model.columns.version(), model.levels.version(), model.wall_types.version(), level.0, phase)
}

/// Runs `f` with the enclosed regions of each (level, phase) computed ONCE: every room of a level shares the union of the same
/// walls, and the union is the expensive part of a room's state (a boolean over every wall of the level). Like `ViewBodies` it
/// lives only for the batch - the model is borrowed immutably by whoever calls this - and is never stored with the model.
pub fn with_region_memo<R>(f: impl FnOnce() -> R) -> R {
    struct Guard(bool);
    impl Drop for Guard {
        fn drop(&mut self) {
            if self.0 {
                REGION_MEMO.with(|m| *m.borrow_mut() = None);
                BOUNDS_MEMO.with(|b| *b.borrow_mut() = None);
            }
        }
    }
    let started = REGION_MEMO.with(|m| {
        let mut m = m.borrow_mut();
        if m.is_none() {
            *m = Some(std::collections::HashMap::new());
            BOUNDS_MEMO.with(|b| *b.borrow_mut() = Some(std::collections::HashMap::new()));
            true
        } else {
            false
        }
    });
    let _guard = Guard(started);
    f()
}

fn enclosed_regions_full_in_uncached(model: &BimModel, level: ElementId, phase: u8) -> Vec<(Vec<Vec2>, Vec<Vec<Vec2>>)> {
    let tol = Tolerances::default();
    let mut bodies: Vec<Region> = Vec::new();
    // Spec M-03: a room-bounding column (Columns category, Room Bounding flag) bounds the rooms of the level; OBSERVED only
    // once its family has a solid (`crate::rooms_ext`)
    for (cid, fp) in crate::rooms_ext::bounding_column_footprints(model, level) {
        if crate::phases::exists_in_phase(model, cid, phase) {
            if let Ok(r) = Region::from_polygon(&fp, &tol) {
                bodies.push(r);
            }
        }
    }
    for w in model.walls.values().filter(|w| wall_bounds_level(model, w, level) && crate::phases::exists_in_phase(model, w.id, phase)) {
        let Ok(s) = wall_state_base(model, w) else { continue };
        let Some(solid) = s.solid.as_ref() else { continue };
        if let Ok(r) = Region::from_polygon(&solid.footprint, &tol) {
            bodies.push(r);
        }
    }
    for sep in model.room_separations.values().filter(|s| s.level == level) {
        let line = asterion_geom::Curve::line(sep.start, sep.end);
        // A separation line has no thickness. The kernel rejects a band thinner than its short-curve
        // tolerance, so the line is closed with a sliver of SEPARATION_HALF_WIDTH and the vertices
        // that land on the sliver's face are moved back onto the line below.
        if let Ok(r) = asterion_geom::offset_band(&[line], SEPARATION_HALF_WIDTH, SEPARATION_HALF_WIDTH, &tol) {
            bodies.push(r);
        }
    }
    let Ok(union) = asterion_geom::union_all(&bodies, &tol) else { return Vec::new() };
    let mut out: Vec<Vec<Vec2>> = Vec::new();
    let outers: Vec<Vec<Vec2>> = union.iter().map(|r| r.outer().corner_points()).collect();
    for r in &union {
        for h in r.holes() {
            let mut pts = h.corner_points();
            if pts.len() < 3 {
                continue;
            }
            // Put the vertices that sit on a separation sliver's face back on the line itself, so
            // the region is bounded by the line and not by a sliver 0.5 mm away from it
            // (OBSERVED, P11: Area 22.42 = (6.0 - 0.1) x 3.8 exactly).
            for sep in model.room_separations.values().filter(|s| s.level == level) {
                let Some(d) = (sep.end - sep.start).normalized() else { continue };
                let n = Vec2::new(-d.y, d.x);
                let len = (sep.end - sep.start).length();
                for p in pts.iter_mut() {
                    let along = (*p - sep.start).dot(d);
                    let s = (*p - sep.start).dot(n);
                    if (s.abs() - SEPARATION_HALF_WIDTH).abs() < 1e-7 && along > -1e-3 && along < len + 1e-3 {
                        *p = *p - n * s;
                    }
                }
            }
            if asterion_geom::polygon_signed_area(&pts) < 0.0 {
                pts.reverse();
            }
            out.push(pts);
        }
    }
    out.sort_by(|a, b| {
        let key = |p: &Vec<Vec2>| {
            (
                p.iter().map(|v| v.x).fold(f64::INFINITY, f64::min),
                p.iter().map(|v| v.y).fold(f64::INFINITY, f64::min),
            )
        };
        let (ka, kb) = (key(a), key(b));
        ka.0.total_cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
    });
    // ENGINE CHOICE: regions from left to right, and at the same left edge from the top down (OBSERVED: `NewRooms2` made
    // its rooms in the left half, then the upper and the lower right quarter)
    out.sort_by(|a, b| {
        let key = |p: &Vec<Vec2>| (p.iter().map(|v| v.x).fold(f64::INFINITY, f64::min), p.iter().map(|v| v.y).fold(f64::NEG_INFINITY, f64::max));
        let (ka, kb) = (key(a), key(b));
        ka.0.total_cmp(&kb.0).then(kb.1.total_cmp(&ka.1))
    });
    out.into_iter()
        .map(|hole| {
            let islands: Vec<Vec<Vec2>> = outers
                .iter()
                .filter(|o| o.len() >= 3 && o.iter().all(|p| point_in_polygon(*p, &hole)))
                .map(|o| {
                    let mut o = o.clone();
                    if asterion_geom::polygon_signed_area(&o) > 0.0 {
                        o.reverse();
                    }
                    o
                })
                .collect();
            (hole, islands)
        })
        .collect()
}

/// The centre boundary of an enclosed region (the loop a room placed in it would report), for [`crate::rooms_ext`].
pub fn region_center_loop(model: &BimModel, level: ElementId, phase: u8, poly: &[Vec2]) -> Vec<RoomSegment> {
    let segs: Vec<RoomSegment> = crate::arc_rooms::finish_segments(model, level, poly, |a, b| segment_host(model, level, phase, a, b));
    if segs.iter().any(|s| s.host.map_or(false, |h| model.columns.contains_key(&h))) {
        crate::rooms_ext::center_segments_with_columns(model, &segs, |h| host_line(model, h))
    } else {
        crate::arc_rooms::center_segments(model, &segs, |h| host_line(model, h))
    }
}

/// Whether `p` lies in the enclosed region `poly` (even-odd rule).
pub fn point_in_region(p: Vec2, poly: &[Vec2]) -> bool {
    point_in_polygon(p, poly)
}

fn point_in_polygon(p: Vec2, poly: &[Vec2]) -> bool {
    let n = poly.len();
    let mut c = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            c = !c;
        }
        j = i;
    }
    c
}

/// The interior point a batch room is placed at: the polygon's centroid.
pub fn region_seed(poly: &[Vec2]) -> Vec2 {
    let n = poly.len();
    let a = asterion_geom::polygon_signed_area(poly);
    if a.abs() < 1e-12 {
        return poly[0];
    }
    let (mut cx, mut cy) = (0.0, 0.0);
    for i in 0..n {
        let (u, v) = (poly[i], poly[(i + 1) % n]);
        let cr = u.x * v.y - v.x * u.y;
        cx += (u.x + v.x) * cr;
        cy += (u.y + v.y) * cr;
    }
    Vec2::new(cx / (6.0 * a), cy / (6.0 * a))
}

/// Whose face a boundary segment lies on.
fn segment_host(model: &BimModel, level: ElementId, phase: u8, a: Vec2, b: Vec2) -> Option<ElementId> {
    let m = (a + b) * 0.5;
    let dir = (b - a).normalized()?;
    // a face of a room-bounding column (OBSERVED: the segments on the column read the column as host)
    if let Some(c) = crate::rooms_ext::column_host(model, level, a, b) {
        return Some(c);
    }
    // the conditions are all pure and ANDed, so the cheap geometric ones go first: whether a wall bounds the level (a wall
    // regeneration) is only asked of the few walls that lie along this segment, not of every wall of the model
    for w in model.walls.values() {
        let Some(wd) = (w.end - w.start).normalized() else { continue };
        if wd.cross(dir).abs() > 1e-6 {
            continue;
        }
        let hw = model.wall_types.get(&w.type_id).map(|t| t.structure.width() / 2.0).unwrap_or(0.0);
        let off = (m - w.start).cross(wd).abs();
        let along = (m - w.start).dot(wd);
        let len = (w.end - w.start).length();
        if (off - hw).abs() < 1e-6 && along > -hw - 1e-6 && along < len + hw + 1e-6 && wall_bounds_level(model, w, level) && crate::phases::exists_in_phase(model, w.id, phase) {
            return Some(w.id);
        }
    }
    for s in model.room_separations.values().filter(|s| s.level == level) {
        let Some(sd) = (s.end - s.start).normalized() else { continue };
        if sd.cross(dir).abs() < 1e-6 && (m - s.start).cross(sd).abs() < 1e-5 {
            return Some(s.id);
        }
    }
    None
}

fn host_line(model: &BimModel, id: ElementId) -> Option<(Vec2, Vec2)> {
    if let Some(w) = model.walls.get(&id) {
        return Some((w.start, w.end));
    }
    model.room_separations.get(&id).map(|s| (s.start, s.end))
}

/// The room's state, derived.
pub fn room_state(model: &BimModel, room: &crate::model::Room) -> Result<RoomState, Failure> {
    // OBSERVED: an unplaced room has no boundary, Area / Perimeter / Volume 0 and height 0, and keeps the box it had when
    // it was unplaced (`crate::rooms_ext`)
    if !crate::rooms_ext::is_placed(model, room.id) {
        let z = model.levels.get(&room.level).map_or(0.0, |l| l.elevation);
        let (lo, hi) = model.rooms_ext.kept_box.get(&room.id).copied().unwrap_or(([room.point.x, room.point.y, z], [room.point.x, room.point.y, z]));
        return Ok(RoomState {
            enclosed: false,
            redundant: false,
            area: 0.0,
            perimeter: 0.0,
            perimeter_center: 0.0,
            volume: 0.0,
            unbounded_height: 0.0,
            level_z: z,
            finish: Vec::new(),
            center: Vec::new(),
            islands_finish: Vec::new(),
            islands_center: Vec::new(),
            bbox_min: Vec3::new(lo[0], lo[1], lo[2]),
            bbox_max: Vec3::new(hi[0], hi[1], hi[2]),
        });
    }
    let level = model.level(room.level)?;
    let upper_z = match room.upper_limit {
        Some(u) => model.level(u)?.elevation,
        None => level.elevation,
    };
    // OBSERVED (P4): upper limit L1 (3.0) + limit offset 0.5 - base offset 0 = 3.5, and the height is INDEPENDENT of the
    // walls' height (P15).
    let unbounded_height = upper_z + room.limit_offset - (level.elevation + room.base_offset);

    let phase = crate::phases::room_phase(model, room.id);
    let full = enclosed_regions_full_in(model, room.level, phase);
    // ENGINE CHOICE: a room point that lies on a boundary belongs to the region below it (OBSERVED: a separation drawn
    // through a room's point leaves the room in the lower half, twice)
    let in_region = |p: Vec2, r: &(Vec<Vec2>, Vec<Vec<Vec2>>)| {
        let p = crate::rooms_ext::off_boundary(p, r);
        point_in_polygon(p, &r.0) && !r.1.iter().any(|h| point_in_polygon(p, h))
    };
    let found = full.iter().position(|r| in_region(room.point, r));
    let Some(idx) = found else {
        return Ok(RoomState {
            enclosed: false,
            redundant: false,
            area: 0.0,
            perimeter: 0.0,
            perimeter_center: 0.0,
            volume: 0.0,
            unbounded_height,
            level_z: level.elevation,
            finish: Vec::new(),
            center: Vec::new(),
            islands_finish: Vec::new(),
            islands_center: Vec::new(),
            bbox_min: Vec3::new(room.point.x, room.point.y, level.elevation),
            bbox_max: Vec3::new(room.point.x, room.point.y, level.elevation),
        });
    };
    let poly = &full[idx].0;
    let islands = &full[idx].1;

    // Spec M-04: rooms that share one enclosed circuit form a group ordered by ElementId; EVERY member maps to the SMALLEST
    // id of the group, a room is redundant when its representative exists and differs from itself, and a redundant room
    // returns the special value in place of Area / Perimeter / Volume (0) while its boundary is still computed. The
    // tie-break is the LOWEST ElementId, not a creation time. UNKNOWN (Spec M-04): what makes two rooms share a Space
    // beyond one circuit (design options, phases), and the phase / level / design-option skip conditions of the group loop.
    let redundant = model
        .rooms
        .values()
        .any(|o| {
            o.id.0 < room.id.0
                && o.level == room.level
                && crate::rooms_ext::is_placed(model, o.id)
                && crate::phases::room_phase(model, o.id) == phase
                && in_region(o.point, &full[idx])
        });

    // Finish boundary: the polygon's edges, started at the segment whose host has the lowest id. An arc wall's sampled
    // outline is collapsed into its exact arc.
    let mut segs: Vec<RoomSegment> = crate::arc_rooms::finish_segments(model, room.level, poly, |a, b| segment_host(model, room.level, phase, a, b));
    let first = segs
        .iter()
        .enumerate()
        .min_by(|(_, x), (_, y)| {
            let kx = x.host.map_or(u64::MAX, |h| h.0);
            let ky = y.host.map_or(u64::MAX, |h| h.0);
            kx.cmp(&ky)
                .then(x.start.y.total_cmp(&y.start.y))
                .then(x.start.x.total_cmp(&y.start.x))
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    segs.rotate_left(first);
    // A separation line that ends free inside the region (a spike of the region's outline) is not part of the loop, its two
    // faces count in the perimeter (OBSERVED: loop of 5 segments, Perimeter 19.2 + 2 x 2.9 = 25.0)
    let dangling_len = crate::rooms_ext::remove_spikes(&mut segs);
    let split = crate::rooms_ext::split_at_separation_ends(model, room.level, &mut segs);
    // A separation that bridges the outline to a free-standing body (a column) stays in the finish loop, traversed both
    // ways; the centre boundary drops it and splits into two loops (OBSERVED, `crate::rooms_ext::split_bridges`)
    let (center_outer, center_inner) = crate::rooms_ext::split_bridges(model, &segs);

    // Centre boundary: each segment runs between the corners where its host's location curve meets its neighbours' (an arc
    // host's segment is an arc on its location circle, `crate::arc_rooms`). A loop with column faces runs its centre
    // segments on those faces (`crate::rooms_ext`, OBSERVED).
    let straight_only = center_outer.iter().chain(center_inner.iter().flatten()).all(|s| s.arc.is_none());
    let special = split || dangling_len > 0.0 || !center_inner.is_empty() || segs.iter().any(|s| s.host.map_or(false, |h| model.columns.contains_key(&h)));
    let center: Vec<RoomSegment> = if special && straight_only {
        crate::rooms_ext::center_segments_with_columns(model, &center_outer, |h| host_line(model, h))
    } else {
        crate::arc_rooms::center_segments(model, &center_outer, |h| host_line(model, h))
    };
    let bridged_center: Vec<Vec<RoomSegment>> =
        center_inner.iter().map(|l| crate::rooms_ext::center_segments_with_columns(model, l, |h| host_line(model, h))).collect();
    // The islands (a free-standing column, OBSERVED): an inner loop each, clockwise, started at its corner of largest x
    // then smallest y (the measured loop starts at (3.3, 1.7) on a column 2.7..3.3 x 1.7..2.3); its centre loop is its
    // finish loop. ENGINE CHOICE: the start corner and an island that is not a column.
    let mut islands_finish: Vec<Vec<RoomSegment>> = Vec::new();
    for isl in islands {
        let n = isl.len();
        let start = (0..n).max_by(|a, b| isl[*a].x.total_cmp(&isl[*b].x).then(isl[*b].y.total_cmp(&isl[*a].y))).unwrap_or(0);
        let ring: Vec<RoomSegment> = (0..n)
            .map(|k| {
                let (a, b) = (isl[(start + k) % n], isl[(start + k + 1) % n]);
                RoomSegment { start: a, end: b, host: segment_host(model, room.level, phase, a, b), arc: None }
            })
            .collect();
        islands_finish.push(ring);
    }
    let islands_center: Vec<Vec<RoomSegment>> = bridged_center.into_iter().chain(islands_finish.iter().cloned()).collect();

    // the area of the exact boundary: the polygon of the segment ends plus the circular segments of the arcs (equal to the
    // polygon's area when no segment is an arc), less the islands' areas; the perimeter is the outer loop's only (OBSERVED:
    // 19.2 with and without the column)
    let area_raw = crate::arc_rooms::area(&segs) - islands.iter().map(|h| asterion_geom::polygon_signed_area(h).abs()).sum::<f64>();
    let perimeter_raw: f64 = segs.iter().map(|s| s.length()).sum::<f64>() + dangling_len;
    let perimeter_center_raw: f64 = center.iter().map(|s| s.length()).sum();
    let computing = !model.volume_computation_off;
    let (area, perimeter, perimeter_center) = if redundant {
        (0.0, 0.0, 0.0)
    } else {
        (area_raw, perimeter_raw, perimeter_center_raw)
    };
    // Spec M-02: the Volume is that of the room's own prism (boundary loops extruded by the stored height = top elevation -
    // bottom elevation); when the height is not above 1e-9 the geometry step creates no geometry and Volume reads 0 - it is
    // never negative. The compute-room-volumes switch gates the value (`computing`); the redundant case reaches here with
    // `area` 0 (Spec M-04). and the value of the switch in a shipped template (file data): ON is the harness's observation,
    // the code default of a document with no area settings is OFF.
    let has_solid = unbounded_height * crate::spec::FEET_PER_METRE > 1e-9;
    // a room with ARCS: Perimeter (corner-trimmed centre length), Volume (exact, of the loop) and box (sample points); see
    // `crate::arc_rooms::arc_room_quantities`
    let simple_loop = islands.is_empty() && dangling_len == 0.0 && !split && center_inner.is_empty();
    let arc_q = if simple_loop && !redundant {
        let hw = |h: ElementId| model.walls.get(&h).and_then(|w| model.wall_types.get(&w.type_id)).map(|t| t.structure.width() / 2.0);
        crate::arc_rooms::arc_room_quantities(&segs, &center, perimeter_center, hw, level.elevation + room.base_offset, unbounded_height)
    } else {
        None
    };
    let perimeter = arc_q.map_or(perimeter, |q| q.perimeter);
    let mut volume = if computing && has_solid { arc_q.map_or(area * unbounded_height, |q| q.volume) } else { 0.0 };
    // Spec M-02: with volumes computed the prism is trimmed by the room-bounding slabs (`crate::room_trim`)
    let mut top_rel = unbounded_height;
    if computing && has_solid && !redundant {
        let base = level.elevation + room.base_offset;
        if let Some((removed, h)) = crate::room_trim::trim(model, room, poly, base, base + unbounded_height) {
            volume = (volume - removed).max(0.0);
            top_rel = h;
        }
    }
    let (lo, hi) = arc_q.map_or_else(|| crate::arc_rooms::extent(&segs), |q| (q.lo, q.hi));
    let (lo_x, hi_x, lo_y, hi_y) = (lo.x, hi.x, lo.y, hi.y);
    Ok(RoomState {
        enclosed: true,
        redundant,
        area,
        perimeter,
        perimeter_center,
        volume,
        unbounded_height,
        level_z: level.elevation,
        finish: segs,
        center,
        islands_finish,
        islands_center,
        bbox_min: Vec3::new(lo_x, lo_y, level.elevation),
        bbox_max: Vec3::new(hi_x, hi_y, level.elevation + top_rel),
    })
}

/// What a floor takes out of a wall it is joined to, or `None` when they do not intersect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloorWallOverlap {
    /// Plan area of the wall's body inside the floor's outline.
    pub plan_area: f64,
    /// Vertical extent shared by the floor's thickness and the wall's height.
    pub height: f64,
    /// Whether the band's lower / upper face lies strictly inside the wall (a NEW face) rather than
    /// at its base or top (a face that goes away).
    pub lower_interior: bool,
    pub upper_interior: bool,
    /// Length of the wall's location line inside the outline, and of the whole line.
    pub inside_len: f64,
    pub full_len: f64,
    pub fully_inside: bool,
}

/// Length of segment `a-b` inside `poly` (even-odd).
fn segment_inside_length(a: Vec2, b: Vec2, poly: &[Vec2]) -> f64 {
    let d = b - a;
    let len = d.length();
    if len < 1e-12 || poly.len() < 3 {
        return 0.0;
    }
    let mut ts: Vec<f64> = vec![0.0, 1.0];
    let n = poly.len();
    for i in 0..n {
        let (p, q) = (poly[i], poly[(i + 1) % n]);
        let e = q - p;
        let den = d.cross(e);
        if den.abs() < 1e-12 {
            continue;
        }
        let t = (p - a).cross(e) / den;
        let u = (p - a).cross(d) / den;
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
            ts.push(t);
        }
    }
    ts.sort_by(|x, y| x.total_cmp(y));
    let inside = |p: Vec2| {
        let mut c = false;
        let mut j = n - 1;
        for i in 0..n {
            let (u, v) = (poly[i], poly[j]);
            if (u.y > p.y) != (v.y > p.y) && p.x < (v.x - u.x) * (p.y - u.y) / (v.y - u.y) + u.x {
                c = !c;
            }
            j = i;
        }
        c
    };
    let mut total = 0.0;
    for w in ts.windows(2) {
        if w[1] - w[0] < 1e-12 {
            continue;
        }
        let mid = a + d * ((w[0] + w[1]) / 2.0);
        if inside(mid) {
            total += (w[1] - w[0]) * len;
        }
    }
    total
}

/// The overlap of a floor and a wall, or `None` when they do not intersect.
pub fn floor_wall_overlap(model: &BimModel, floor: ElementId, wall: ElementId) -> Option<FloorWallOverlap> {
    let f = model.floors.get(&floor)?;
    let w = model.walls.get(&wall)?;
    let fs = slab_state_base(model, f).ok()?;
    let ws = wall_state_base(model, w).ok()?;
    let full_len = (w.end - w.start).length();
    let inside_len = segment_inside_length(w.start, w.end, &f.outline_polygon());
    if inside_len <= 1e-9 || ws.width <= 0.0 {
        return None;
    }
    let lo = fs.bottom_z.max(ws.base_z);
    let hi = fs.top_z.min(ws.top_z);
    if hi - lo <= 1e-9 {
        return None;
    }
    let fraction = if full_len > 0.0 { (inside_len / full_len).min(1.0) } else { 0.0 };
    Some(FloorWallOverlap {
        plan_area: ws.mitred_plan_area * fraction,
        height: hi - lo,
        lower_interior: fs.bottom_z > ws.base_z + 1e-9,
        upper_interior: fs.top_z < ws.top_z - 1e-9,
        inside_len,
        full_len,
        fully_inside: (inside_len - full_len).abs() < 1e-9,
    })
}

/// The top (maximum z) of a roof's geometry outline as the roof is NOW, cut-off in force.
///
/// Spec R-05: the second branch of the rewrite that follows the deletion of a roof's cut-off level starts from this value.
/// `None` when the roof has no computable body.
pub fn roof_outline_top_z(model: &BimModel, f: &Slab) -> Option<f64> {
    slab_state_base(model, f).ok().map(|s| s.top_z)
}

/// The lowest point of a roof's UNCLIPPED underside - what its cut-off must stay above.
///
/// Computed with the cut-off removed on purpose: a cut-off at or below the underside clips the
/// whole roof away, and a roof with no body has no solid to ask.
pub fn roof_lowest_bottom_z(model: &BimModel, f: &Slab) -> Option<f64> {
    let level = model.levels.get(&f.level)?;
    let st = model.floor_types.get(&f.type_id)?;
    let fp = roof_footprint(f);
    let mut input = roof_input(model, f, &fp, level.elevation + f.height_offset, st.thickness);
    input.cutoff_z = None;
    roof_solid(&input).map(|r| r.lowest_bottom_z)
}

/// A roof's footprint as the polygon the roof builder works on, with the slope-defining edges
/// re-indexed onto it.
///
/// Straight boundaries are the stored vertices and indices, unchanged. With ARC edges the arcs are replaced by their
/// display samples ([`crate::model::SlabLoop::outline`]): how the footprint roof's shell is built from curved footprint
/// edges is not specified, so a roof with arcs is built on that polygon and its quantities are those of the polygon. A
/// slope-defining ARC edge is skipped (arc segmentation, not specified) and counted.
struct RoofFootprint {
    polygon: Vec<Vec2>,
    slopes: Vec<(usize, f64)>,
    /// The segment Offsets of the slope-defining edges, indexed like `slopes`.
    offsets: Vec<(usize, f64)>,
    /// Slope-defining arc edges left out of the body.
    skipped_arc_slopes: usize,
}

fn roof_footprint(f: &Slab) -> RoofFootprint {
    let defining = |i: usize| -> Option<f64> {
        let s = f.segments.get(i)?;
        (s.defines_slope && s.slope != 0.0 && s.slope.is_finite()).then_some(s.slope)
    };
    let offset = |i: usize| -> f64 { f.segments.get(i).map_or(0.0, |s| if s.offset_from_base.is_finite() { s.offset_from_base } else { 0.0 }) };
    if !f.boundary_arcs.iter().any(|a| a.is_some()) {
        let slopes: Vec<(usize, f64)> = (0..f.segments.len()).filter_map(|i| defining(i).map(|s| (i, s))).collect();
        let offsets = slopes.iter().map(|(i, _)| (*i, offset(*i))).filter(|(_, o)| *o != 0.0).collect();
        return RoofFootprint { polygon: f.boundary.clone(), slopes, offsets, skipped_arc_slopes: 0 };
    }
    let lp = f.boundary_loop();
    let tols = asterion_geom::stokes::TessTols::display_default();
    let (mut polygon, mut slopes, mut skipped) = (Vec::new(), Vec::new(), 0usize);
    let mut offsets: Vec<(usize, f64)> = Vec::new();
    for i in 0..lp.points.len() {
        match lp.edge_curve(i) {
            Ok(asterion_geom::Curve::Arc(a)) => {
                if defining(i).is_some() {
                    skipped += 1;
                }
                match asterion_geom::stokes::arc_samples_m(&a, &tols) {
                    Ok(pts) => polygon.extend(pts[..pts.len() - 1].iter().copied()),
                    Err(_) => polygon.push(lp.points[i]),
                }
            }
            _ => {
                if let Some(s) = defining(i) {
                    if offset(i) != 0.0 {
                        offsets.push((polygon.len(), offset(i)));
                    }
                    slopes.push((polygon.len(), s));
                }
                polygon.push(lp.points[i]);
            }
        }
    }
    RoofFootprint { polygon, slopes, offsets, skipped_arc_slopes: skipped }
}

/// The box of a curve's sample points. A line is its two ends; an arc is its display samples (uniform in the angle, both ends
/// included), so an extreme that falls between two samples is not reached, by less than the display sag of 1 mm. OBSERVED in the
/// reference product: its box of an arc does not reach an extreme that falls between two of its own samples; the engine's samples
/// are its own, so the two boxes differ by about the sag. An arc whose sampling fails keeps its exact box.
pub fn sampled_aabb(c: &asterion_geom::Curve) -> asterion_geom::Aabb2 {
    if let asterion_geom::Curve::Arc(a) = c {
        let tols = asterion_geom::stokes::TessTols::display_default();
        if let Ok(pts) = asterion_geom::stokes::arc_samples_m(a, &tols) {
            if let Some(first) = pts.first() {
                return pts.iter().fold(asterion_geom::Aabb2::from_point(*first), |b, p| b.with_point(*p));
            }
        }
    }
    c.aabb()
}

/// The roof's inputs, derived from the model.
fn roof_input<'a>(model: &BimModel, f: &Slab, fp: &'a RoofFootprint, base_z: f64, thickness: f64) -> RoofInput<'a> {
    RoofInput {
        boundary: &fp.polygon,
        slopes: fp.slopes.clone(),
        base_z,
        offsets: fp.offsets.clone(),
        thickness,
        cutoff_z: roof_cutoff_z(model, f),
    }
}

/// Elevation of a slab's REFERENCE surface above plan point `p`: the top face of a floor, the bottom face of a ceiling -
/// the slope plane when one is defined - and the underside of a roof (its lower envelope of slope planes, before any
/// cut-off). `None` when the slab's level is missing or its slope gives no plane.
pub fn slab_reference_z_at(model: &BimModel, f: &Slab, p: Vec2) -> Option<f64> {
    let base = model.levels.get(&f.level)?.elevation + f.height_offset;
    // OBSERVED: a shape-edited floor or roof - the height of its shape surface
    if let Some(g) = crate::slab_shape::geometry(model, f) {
        return crate::slab_shape::surface_z_at(&g, p);
    }
    if f.role == SlabRole::Roof {
        let thickness = model.floor_types.get(&f.type_id)?.thickness;
        let fp = roof_footprint(f);
        let input = roof_input(model, f, &fp, base, thickness);
        return crate::roof::underside_z_at(&input, p);
    }
    Some(match slope_plane(model, f, base) {
        Some(sp) => sp.cap(base).at(p),
        None => base,
    })
}

/// The reference plane of a sloped floor or ceiling: `z = base + dz + ratio ((p - origin) . dir)`.
#[derive(Clone, Copy, Debug)]
struct SlopePlane {
    origin: Vec2,
    dir: Vec2,
    ratio: f64,
    dz: f64,
}

impl SlopePlane {
    fn cap(&self, base: f64) -> asterion_geom::stokes::CapPlane {
        let g = self.dir * self.ratio;
        asterion_geom::stokes::CapPlane { c0: base + self.dz - g.dot(self.origin), g }
    }
}

/// The slope arrow if there is one, otherwise the slope-defining edge. The plane of a defining edge holds the edge's line
/// at its height and rises along `w` (edge direction x +Z, or its reverse) at the slope ratio. `base` is the slab's base
/// height (level elevation + height offset): an arrow's line starts at its start height, which the plane holds as `dz`
/// above the base.
fn slope_plane(model: &BimModel, f: &Slab, base: f64) -> Option<SlopePlane> {
    if let Some(a) = f.slope_arrow {
        let dir = (a.head - a.tail).normalized()?;
        let (start, ratio) = a.line(model, base);
        return Some(SlopePlane { origin: a.tail, dir, ratio, dz: start - base });
    }
    let e = f.slope_edge?;
    let n = f.boundary.len();
    if e.edge >= n || !e.slope.is_finite() {
        return None;
    }
    let (a, b) = (f.boundary[e.edge], f.boundary[(e.edge + 1) % n]);
    let dir = e.rise_direction(a, b)?;
    Some(SlopePlane { origin: a, dir, ratio: e.slope, dz: e.height })
}

/// Number of joints of a closed loop of curves where the curve that ends there and the curve that starts there are tangent
/// in the same direction (parallel test, 0.1 degree): the vertical edge between their side faces is a hidden edge.
fn tangent_joints(curves: &[asterion_geom::Curve]) -> usize {
    let n = curves.len();
    (0..n)
        .filter(|&i| {
            let (a, b) = (&curves[i], &curves[(i + 1) % n]);
            match (a.tangent_at(1.0), b.tangent_at(0.0)) {
                (Some(ta), Some(tb)) => ta.dot(tb) > 0.0 && crate::sketch::segments_parallel((Vec2::new(0.0, 0.0), ta), (Vec2::new(0.0, 0.0), tb)),
                _ => false,
            }
        })
        .count()
}

/// Floor / ceiling state when any loop has an ARC edge (flat or sloped, holes and islands).
///
/// * Body: the loops swept along the vertical direction between the reference plane (top of a floor, bottom of a ceiling;
///   the slope plane when there is one) and the parallel plane the total layers width away PERPENDICULARLY (vertically `t
///   sqrt(1+g^2)`); side faces vertical, a cylinder per arc.
/// * Area: Spec F-02a - the larger of the top and bottom face areas, each the exact area of the face
///   (`asterion_geom::stokes`): for a flat slab the plan area, for an inclined one the plan area times `sqrt(1 + g^2)`.
/// * Volume: Spec F-02b - the volume of the solid (no shape-edit step, constant offset true), exact: the plan area times
///   the vertical thickness.
/// * Perimeter: Spec F-02c - the sum of the lengths of every loop, inner loops included, in the sketch plane. ASSUMED (Spec
///   F-02c not specified: the per-curve rule): an arc's length is its exact `r |sweep|`.
/// * Zero depth: ONE planar face from the loops; its area is the Area.
fn curved_slab_state(
    f: &Slab,
    thickness: f64,
    reference_z: f64,
    slope: Option<SlopePlane>,
) -> Result<SlabState, Failure> {
    use asterion_geom::stokes::{cap_area, extruded_loops_metrics, CapLoop, CapPlane, TessTols};
    let loops = f.sketch_loops();
    let mut curves: Vec<Vec<asterion_geom::Curve>> = Vec::with_capacity(loops.len());
    for (k, l) in loops.iter().enumerate() {
        curves.push(l.curves().map_err(|i| {
            Failure::error(codes::SLAB_INVALID_ARC, format!("Edge {i} of slab loop {k}: no arc passes through its three points."))
        })?);
    }
    // Material or hole is the parity of the number of OTHER loops containing a loop.
    let roles = crate::sketch::classify(
        &loops
            .iter()
            .map(|l| crate::sketch::SketchLoop::closed_with_arcs(l.points.clone(), l.arcs.clone()))
            .collect::<Vec<_>>(),
    );
    let mut cap_loops: Vec<CapLoop> = Vec::with_capacity(loops.len());
    let (mut plan_area, mut perimeter, mut sides) = (0.0, 0.0, 0usize);
    for (cs, role) in curves.into_iter().zip(&roles) {
        let signed: f64 = cs.iter().map(|c| c.signed_area_term()).sum();
        perimeter += cs.iter().map(|c| c.length()).sum::<f64>();
        sides += cs.len();
        plan_area += if role.material { signed.abs() } else { -signed.abs() };
        // Orientation: material counter-clockwise, holes clockwise (material on the left)
        let oriented = if (signed > 0.0) == role.material { cs } else { cs.iter().rev().map(|c| c.reversed()).collect() };
        cap_loops.push(CapLoop { curves: oriented, is_hole: !role.material });
    }
    let reference = match slope {
        Some(sp) => sp.cap(reference_z),
        None => CapPlane::flat(reference_z),
    };
    let vt = thickness * (1.0 + reference.g.dot(reference.g)).sqrt();
    let (bottom, top) = match f.anchor {
        SlabAnchor::TopAtLevel => (reference.lifted(-vt), reference),
        SlabAnchor::BottomAtLevel => (reference, reference.lifted(vt)),
    };
    let tols = TessTols::display_default();
    let fail = |e: asterion_geom::stokes::StokesError| {
        Failure::error(codes::SLAB_INTEGRATION_FAILED, format!("Slab face integration failed: {e}."))
    };
    let (area, volume, surface_area) = if thickness > 0.0 {
        let q = extruded_loops_metrics(&cap_loops, bottom, top, &tols).map_err(fail)?;
        let area = crate::quantity::host_area(&[q.top_area], &[q.bottom_area]).unwrap_or(0.0);
        let volume = crate::quantity::host_volume(area, Some(q.volume), Some(thickness), true).unwrap_or(0.0);
        (area, volume, q.surface_area)
    } else {
        let a = cap_area(&cap_loops, reference, &tols).map_err(fail)?;
        (a, 0.0, a)
    };
    // extent: the box of the SAMPLE POINTS of the solid's edges (a line's ends exactly, an arc's display samples: an extreme
    // between two samples is not reached). Material loops only. The heights below stay exact (support points of the planes).
    let (mut lo, mut hi) = (Vec2::new(f64::INFINITY, f64::INFINITY), Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY));
    let (mut z_lo, mut z_hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for l in cap_loops.iter().filter(|l| !l.is_hole) {
        for c in &l.curves {
            let b = sampled_aabb(c);
            lo = Vec2::new(lo.x.min(b.min.x), lo.y.min(b.min.y));
            hi = Vec2::new(hi.x.max(b.max.x), hi.y.max(b.max.y));
            let up = c.support_point(top.g);
            let down = c.support_point(-bottom.g);
            z_hi = z_hi.max(top.at(up)).max(top.at(c.start()));
            z_lo = z_lo.min(bottom.at(down)).min(bottom.at(c.start()));
        }
    }
    // The extrusion builder hides the edges between tangent faces (0.1 degree): at a loop vertex where the incoming and
    // outgoing curves are tangent (same direction within 0.1 degree) the vertical edge between their side faces is hidden
    // and not counted. OBSERVED: a disc of two half circles reports 4 edges, a rectangle with a round hole 16.
    let hidden: usize = cap_loops.iter().map(|l| tangent_joints(&l.curves)).sum();
    let flat = slope.is_none();
    let solid = (flat && thickness > 0.0).then(|| Prism::new(f.outline_polygon(), bottom.c0, top.c0));
    Ok(SlabState {
        role: f.role,
        thickness,
        top_z: z_hi,
        bottom_z: z_lo,
        plan_area,
        area,
        perimeter,
        volume,
        surface_area,
        slope: slope.map(|s| s.ratio).or_else(|| f.reported_slope()),
        multi_plane: false,
        solid,
        sloped: None,
        bbox_min: Vec3::new(lo.x, lo.y, z_lo),
        bbox_max: Vec3::new(hi.x, hi.y, z_hi),
        face_count: if thickness > 0.0 { sides + 2 } else { 1 },
        edge_count: if thickness > 0.0 { sides * 3 - hidden } else { sides },
        vertex_count: if thickness > 0.0 { sides * 2 } else { sides },
        skipped_arc_slopes: 0,
    })
}

/// The bodies of a slab (floor, ceiling or roof) for drawing it in every view: the same planes the
/// quantities of [`slab_state`] are computed from, as [`crate::element_geometry::Body`] values.
///
/// * a flat floor or ceiling is its material loops between the bottom and top planes, the holes of its
///   sketch and the shafts / openings that cut it subtracted;
/// * a roof is the convex pieces of its solid ([`crate::roof::roof_pieces`]), the cut-off included;
/// * a shape-edited slab has no body here (`slab_state` keeps its metrics only): [`Geometry::NotGenerated`].
///
/// A slab without a body says so; it never returns an empty `Solid`.
pub fn slab_geometry(model: &BimModel, f: &Slab) -> crate::element_geometry::Geometry {
    use crate::element_geometry::{Body, Geometry, Lin};
    let fail = |e: Failure| Geometry::Failed(e.message);
    let st = match model.slab_type(f.type_id) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let level = match model.level(f.level) {
        Ok(l) => l,
        Err(e) => return fail(e),
    };
    let thickness = st.thickness;
    if !(thickness > 0.0) {
        return Geometry::NotGenerated("a zero-thickness slab has no body (its sketch is a planar face)".into());
    }
    if f.boundary.len() < f.boundary_loop().min_points() {
        return Geometry::Failed("A slab boundary needs at least 3 points.".into());
    }
    let reference_z = level.elevation + f.height_offset;
    if matches!(f.role, SlabRole::Floor | SlabRole::Roof) && model.slab_shapes.contains_key(&f.id) {
        return Geometry::NotGenerated("a shape-edited slab: the engine keeps its quantities, not its body (OQ-SLAB-SHAPE-BODY)".into());
    }
    let ccw = |mut p: Vec<Vec2>| {
        if asterion_geom::polygon_signed_area(&p) < 0.0 {
            p.reverse();
        }
        p
    };
    if f.role == SlabRole::Roof {
        let fp = roof_footprint(f);
        let input = roof_input(model, f, &fp, reference_z, thickness);
        return match crate::roof::roof_pieces(&input) {
            Some(pieces) => Geometry::Solid(
                pieces
                    .into_iter()
                    .map(|p| Body {
                        outer: ccw(p.poly),
                        holes: Vec::new(),
                        bottom: Lin { c0: p.bottom[0], gx: p.bottom[1], gy: p.bottom[2] },
                        top: Lin { c0: p.top[0], gx: p.top[1], gy: p.top[2] },
                    })
                    .collect(),
            ),
            None => Geometry::NotGenerated("the roof builder gives no body for this footprint and slopes".into()),
        };
    }
    // floors and ceilings: the shafts and openings that cut the slab are holes of its sketch (as `slab_state` does)
    let holes = crate::shafts::flat_slab_holes(model, f);
    let cut;
    let f = if holes.is_empty() {
        f
    } else {
        cut = {
            let mut c = f.clone();
            c.loops.extend(holes);
            c
        };
        &cut
    };
    let reference = match slope_plane(model, f, reference_z) {
        Some(sp) => sp.cap(reference_z),
        None => asterion_geom::stokes::CapPlane::flat(reference_z),
    };
    let vt = thickness * (1.0 + reference.g.dot(reference.g)).sqrt();
    let reference_lin = Lin { c0: reference.c0, gx: reference.g.x, gy: reference.g.y };
    let lifted = |dz: f64| Lin { c0: reference_lin.c0 + dz, ..reference_lin };
    let (bottom, top) = match f.anchor {
        SlabAnchor::TopAtLevel => (lifted(-vt), reference_lin),
        SlabAnchor::BottomAtLevel => (reference_lin, lifted(vt)),
    };
    let loops = f.sketch_loops();
    let polys: Vec<Vec<Vec2>> = loops.iter().map(|l| l.outline()).collect();
    let roles = crate::sketch::classify(&loops.iter().map(|l| crate::sketch::SketchLoop::closed_with_arcs(l.points.clone(), l.arcs.clone())).collect::<Vec<_>>());
    let material: Vec<usize> = (0..polys.len()).filter(|i| roles[*i].material).collect();
    let mut bodies: Vec<Body> = Vec::new();
    for &i in &material {
        // a hole belongs to the SMALLEST material loop that contains it
        let owned: Vec<Vec<Vec2>> = (0..polys.len())
            .filter(|&h| !roles[h].material && !polys[h].is_empty())
            .filter(|&h| {
                let q = polys[h][0];
                let owner = material
                    .iter()
                    .filter(|&&k| asterion_geom::region::polygon_contains_point(&polys[k], q))
                    .min_by(|&&a, &&b| asterion_geom::polygon_signed_area(&polys[a]).abs().total_cmp(&asterion_geom::polygon_signed_area(&polys[b]).abs()));
                owner == Some(&i)
            })
            .map(|h| {
                let mut p = polys[h].clone();
                if asterion_geom::polygon_signed_area(&p) > 0.0 {
                    p.reverse();
                }
                p
            })
            .collect();
        bodies.push(Body { outer: ccw(polys[i].clone()), holes: owned, bottom, top });
    }
    if bodies.is_empty() {
        return Geometry::NotGenerated("the slab sketch has no material loop".into());
    }
    Geometry::Solid(bodies)
}

/// Derived state of a slab: a pure function of the model.
pub fn slab_state(model: &BimModel, f: &Slab) -> Result<SlabState, Failure> {
    // OBSERVED: shafts and openings cut floors and ceilings as holes of the sketch (`crate::shafts`)
    let holes = crate::shafts::flat_slab_holes(model, f);
    let holes_empty = holes.is_empty();
    let base_perimeter = if holes_empty { 0.0 } else { slab_state_base(model, f).map_or(0.0, |b| b.perimeter) };
    let cut;
    let f = if holes.is_empty() {
        f
    } else {
        cut = {
            let mut c = f.clone();
            c.loops.extend(holes);
            c
        };
        &cut
    };
    let mut s = slab_state_base(model, f)?;
    if !holes_empty {
        // OBSERVED: the Perimeter of a floor or a ceiling cut by a shaft or an opening is the perimeter of its sketch (20
        // for the 6 x 4 floor with a 2 x 2 shaft), the holes' sides do not count
        s.perimeter = base_perimeter;
    }
    crate::shafts::apply_roof_holes(model, f, &mut s);
    // OBSERVED: the two-cut rafter cuts of a gable's eaves (`crate::rafter_cut`)
    crate::rafter_cut::apply(model, f, &mut s);
    // OBSERVED: when a WALL cuts a floor (the join order switched), the floor loses exactly the wall's plan area inside it,
    // over the height they share: Area 48.000 -> 46.760, Volume 9.600 -> 9.352 for a 6.2 x 0.2 wall and 0.2 of slab.
    if f.role == SlabRole::Floor {
        for j in model.geometry_joins.iter().filter(|j| j.involves(f.id) && j.cutter != f.id) {
            let Some(o) = floor_wall_overlap(model, f.id, j.cutter) else { continue };
            s.area -= o.plan_area;
            s.volume -= o.plan_area * o.height;
            // The strip's two horizontal faces go and its lateral faces appear over the shared
            // height: `-2A + P h`. For the 6.2 x 0.2 wall in 0.2 of slab that is +0.08, which is
            // exactly what the reference reports (101.600 -> 101.680).
            if let Some(w) = model.walls.get(&j.cutter) {
                if let Ok(ws) = wall_state_base(model, w) {
                    let l_eff = if ws.width > 0.0 { o.plan_area / ws.width } else { 0.0 };
                    let p = if o.fully_inside { 2.0 * (l_eff + ws.width) } else { 2.0 * l_eff };
                    s.surface_area += -2.0 * o.plan_area + p * o.height;
                }
            }
            // The strip is a hole: four more sides when the wall lies wholly inside. The reference
            // reports 28 edges where 3 x sides gives 24 - the imprint is not modelled, and the 4 is
            // recorded as a known limitation, not folded in.
            if o.fully_inside {
                s.face_count += 4;
                s.edge_count += 12;
                s.vertex_count += 8;
            }
        }
    }
    // Floors and ceilings share the operation, and an Area at or below 1e-9 (square feet) is error 7 - no area is produced.
    // UNKNOWN: the parameter layer outside the reference product that turns the error into the value a user sees; the
    // engine reports 0 (Spec C-02: a 2 mm sketch commits with Area 0 - that one is 4e-6 m2, far above the threshold, so it
    // is a rounding observation and not this branch).
    if matches!(f.role, SlabRole::Floor | SlabRole::Ceiling)
        && crate::quantity::host_area(&[s.area], &[s.area]).is_err()
    {
        s.area = 0.0;
    }
    Ok(s)
}

fn slab_state_base(model: &BimModel, f: &Slab) -> Result<SlabState, Failure> {
    let st = model.slab_type(f.type_id)?;
    let level = model.level(f.level)?;
    if f.boundary.len() < f.boundary_loop().min_points() {
        return Err(Failure::error(codes::INVALID_BOUNDARY, "A slab boundary needs at least 3 points."));
    }
    let thickness = st.thickness;

    // Which face the level offset pins. OBSERVED for a floor: the TOP sits at `level + height_offset` and the body hangs
    // downwards. OBSERVED for a ceiling and a roof: the BOTTOM sits there and the body grows UP - thickening a ceiling type
    // from 0.200 to 0.350 moved z [2.700..2.900] to [2.700..3.050].
    let reference_z = level.elevation + f.height_offset;
    let (flat_bottom, flat_top) = match f.anchor {
        SlabAnchor::TopAtLevel => (reference_z - thickness, reference_z),
        SlabAnchor::BottomAtLevel => (reference_z, reference_z + thickness),
    };

    let plan_area = asterion_geom::polygon_signed_area(&f.boundary).abs();
    let plan_perimeter = asterion_geom::polygon_perimeter(&f.boundary);
    let multi_plane = f.segments.iter().filter(|s| s.defines_slope && s.slope != 0.0).count() > 1;

    // OBSERVED: a floor or a roof with shape editing (`crate::slab_shape`)
    if matches!(f.role, SlabRole::Floor | SlabRole::Roof) && thickness > 0.0 && model.slab_shapes.contains_key(&f.id) {
        let variable = model.slab_variable_layers.contains_key(&f.type_id);
        if let Some(q) = crate::slab_shape::quantities(model, f, thickness, variable) {
            let lo = Vec2::new(
                f.boundary.iter().map(|p| p.x).fold(f64::INFINITY, f64::min),
                f.boundary.iter().map(|p| p.y).fold(f64::INFINITY, f64::min),
            );
            let hi = Vec2::new(
                f.boundary.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max),
                f.boundary.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max),
            );
            let perimeter = asterion_geom::polygon_perimeter(&f.boundary) + f.loops.iter().map(|l| asterion_geom::polygon_perimeter(l)).sum::<f64>();
            return Ok(SlabState {
                role: f.role,
                thickness,
                top_z: q.bbox_z.1,
                bottom_z: q.bbox_z.0,
                plan_area,
                area: q.area,
                perimeter,
                volume: q.volume_param,
                surface_area: q.surface_area,
                slope: None,
                multi_plane: false,
                solid: None,
                sloped: None,
                bbox_min: Vec3::new(lo.x, lo.y, q.bbox_z.0),
                bbox_max: Vec3::new(hi.x, hi.y, q.bbox_z.1),
                face_count: q.faces,
                edge_count: q.edges,
                vertex_count: q.vertices,
                skipped_arc_slopes: 0,
            });
        }
    }

    if f.role == SlabRole::Roof && thickness > 0.0 {
        let fp = roof_footprint(f);
        let input = roof_input(model, f, &fp, reference_z, thickness);
        if let Some(r) = roof_solid(&input) {
            let (plan_area, plan_perimeter) = if f.has_arcs() {
                // the exact plan quantities of the sketch (the body is the sampled polygon)
                let lp = f.boundary_loop();
                match lp.curves() {
                    Ok(cs) => (
                        cs.iter().map(|c| c.signed_area_term()).sum::<f64>().abs(),
                        cs.iter().map(|c| c.length()).sum::<f64>(),
                    ),
                    Err(_) => (plan_area, plan_perimeter),
                }
            } else {
                (plan_area, plan_perimeter)
            };
            return Ok(SlabState {
                role: f.role,
                thickness,
                top_z: r.bbox_max.z,
                bottom_z: r.bbox_min.z,
                plan_area,
                area: r.area,
                perimeter: plan_perimeter,
                volume: r.volume,
                surface_area: r.surface_area,
                slope: f.reported_slope(),
                multi_plane,
                solid: None,
                bbox_min: r.bbox_min,
                bbox_max: r.bbox_max,
                face_count: r.face_count,
                edge_count: r.edge_count,
                vertex_count: r.vertex_count,
                sloped: Some(r),
                skipped_arc_slopes: fp.skipped_arc_slopes,
            });
        }
    }

    // Floors and ceilings with an arc edge anywhere in the sketch.
    if f.role != SlabRole::Roof && f.has_arcs() {
        return curved_slab_state(f, thickness, reference_z, slope_plane(model, f, reference_z));
    }

    // OBSERVED: a floor with a slope arrow. The top passes through `level + offset` at the arrow's tail and rises toward
    // its head; thickness is PERPENDICULAR, so the vertical distance between the faces is `t * sqrt(1 + g^2)`; the end
    // faces stay VERTICAL; `Area` is the INCLINED face and `Perimeter` the PLAN perimeter - two referentials on one
    // element.
    //
    // (read in the reference product): the builder is the same for a ceiling. The plane that contains the slope line is the
    // REFERENCE plane, the other plane lies on the extrusion direction's side of it, the planes are parallel and separated
    // perpendicularly by the depth (= total layers width), and every loop is projected along the direction, so the side
    // faces are vertical. For a floor the direction is (0,0,-1): the reference plane is the TOP and the body hangs below.
    // For a ceiling it is (0,0,+1): the reference plane is the BOTTOM, at level + offset at the arrow's tail, and the body
    // grows UP.
    //
    // Its `Volume` is the solid volume because a slope arrow / defining edge has no shape-edit step (constant offset TRUE):
    // the sheared prism, plan area x the vertical height `t sqrt(1 + g^2)`. Both are computed through `crate::quantity`.
    if let (Some(sp), true) = (slope_plane(model, f, reference_z), matches!(f.role, SlabRole::Floor | SlabRole::Ceiling) && thickness > 0.0) {
        // The slope plane is the arrow's or the defining edge's. Every loop of the sketch takes part: material or hole is
        // the parity of nesting, the Area and the Volume are those of the net region, the Perimeter includes the inner
        // loops (Spec F-02c).
        let ratio = (1.0 + sp.ratio * sp.ratio).sqrt();
        let vt = thickness * ratio;
        let top_at = |p: Vec2| reference_z + sp.dz + sp.ratio * (p - sp.origin).dot(sp.dir);
        let mut all: Vec<&Vec<Vec2>> = vec![&f.boundary];
        all.extend(f.loops.iter());
        let roles = crate::sketch::classify(
            &all.iter().map(|l| crate::sketch::SketchLoop::closed((*l).clone())).collect::<Vec<_>>(),
        );
        let (mut net_area, mut perimeter, mut sides) = (0.0, 0.0, 0usize);
        let (mut lo, mut hi) = (Vec2::new(f64::INFINITY, f64::INFINITY), Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY));
        let (mut ref_hi, mut ref_lo) = (f64::NEG_INFINITY, f64::INFINITY);
        for (i, l) in all.iter().enumerate() {
            let a = asterion_geom::polygon_signed_area(l).abs();
            net_area += if roles[i].material { a } else { -a };
            perimeter += asterion_geom::polygon_perimeter(l);
            sides += l.len();
            if roles[i].material {
                for p in l.iter() {
                    lo = Vec2::new(lo.x.min(p.x), lo.y.min(p.y));
                    hi = Vec2::new(hi.x.max(p.x), hi.y.max(p.y));
                    ref_hi = ref_hi.max(top_at(*p));
                    ref_lo = ref_lo.min(top_at(*p));
                }
            }
        }
        let (z_hi, z_lo) = match f.anchor {
            // reference plane = top: hangs down
            SlabAnchor::TopAtLevel => (ref_hi, ref_lo - vt),
            // reference plane = bottom: grows up
            SlabAnchor::BottomAtLevel => (ref_hi + vt, ref_lo),
        };
        let incl = net_area * ratio;
        // Top faces and bottom faces are both the inclined planar face; the solid is the sheared prism (plan area x
        // vertical height). A vanishing area is error 7.
        let area = crate::quantity::host_area(&[incl], &[incl]).unwrap_or(0.0);
        let solid_volume = net_area * vt;
        let volume = crate::quantity::host_volume(area, Some(solid_volume), Some(thickness), true)
            .unwrap_or(0.0);
        return Ok(SlabState {
            role: f.role,
            thickness,
            top_z: z_hi,
            bottom_z: z_lo,
            plan_area,
            area,
            perimeter,
            volume,
            surface_area: 2.0 * incl + perimeter * vt,
            slope: Some(sp.ratio),
            multi_plane: false,
            solid: None,
            sloped: None,
            bbox_min: Vec3::new(lo.x, lo.y, z_lo),
            bbox_max: Vec3::new(hi.x, hi.y, z_hi),
            face_count: sides + 2,
            edge_count: sides * 3,
            vertex_count: sides * 2,
            skipped_arc_slopes: 0,
        });
    }

    // OBSERVED: further loops. Material or hole is the PARITY of a loop's nesting depth among all of them; Area is the
    // signed sum; Perimeter INCLUDES the interior loops (24 = 20 + 4); faces are 2 + every side of every loop; edges are 3
    // x sides.
    if !f.loops.is_empty() && thickness > 0.0 {
        let mut all: Vec<&Vec<Vec2>> = vec![&f.boundary];
        all.extend(f.loops.iter());
        // Material or hole is the parity of the number of OTHER loops containing it.
        let roles = crate::sketch::classify(
            &all.iter().map(|l| crate::sketch::SketchLoop::closed((*l).clone())).collect::<Vec<_>>(),
        );
        let mut area = 0.0;
        let mut perimeter = 0.0;
        let mut sides = 0usize;
        let (mut lo_x, mut hi_x, mut lo_y, mut hi_y) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for (i, l) in all.iter().enumerate() {
            let a = asterion_geom::polygon_signed_area(l).abs();
            let material = roles[i].material;
            area += if material { a } else { -a };
            perimeter += asterion_geom::polygon_perimeter(l);
            sides += l.len();
            if material {
                for p in l.iter() {
                    lo_x = lo_x.min(p.x);
                    hi_x = hi_x.max(p.x);
                    lo_y = lo_y.min(p.y);
                    hi_y = hi_y.max(p.y);
                }
            }
        }
        return Ok(SlabState {
            role: f.role,
            thickness,
            top_z: flat_top,
            bottom_z: flat_bottom,
            plan_area,
            area,
            perimeter,
            volume: area * thickness,
            surface_area: 2.0 * area + perimeter * thickness,
            slope: f.reported_slope(),
            multi_plane,
            // The OUTER loop's prism, kept only so a view can classify the slab by its outline.
            solid: Some(Prism::new(f.boundary.clone(), flat_bottom, flat_top)),
            sloped: None,
            bbox_min: Vec3::new(lo_x, lo_y, flat_bottom),
            bbox_max: Vec3::new(hi_x, hi_y, flat_top),
            face_count: sides + 2,
            edge_count: sides * 3,
            vertex_count: sides * 2,
            skipped_arc_slopes: 0,
        });
    }

    let solid = (thickness > 0.0).then(|| Prism::new(f.boundary.clone(), flat_bottom, flat_top));
    let (area, perimeter) = match &solid {
        Some(pr) => (pr.footprint_area(), pr.footprint_perimeter()),
        // A zero-thickness ceiling is legal (section 1a) and has no body, but it still has a
        // sketch, so Area and Perimeter come from the boundary rather than collapsing to 0.
        None => (plan_area, plan_perimeter),
    };
    let n = f.boundary.len();
    Ok(SlabState {
        role: f.role,
        thickness,
        top_z: flat_top,
        bottom_z: flat_bottom,
        plan_area,
        area,
        perimeter,
        volume: area * thickness,
        surface_area: 2.0 * area + perimeter * thickness,
        slope: f.reported_slope(),
        multi_plane,
        bbox_min: Vec3::new(
            f.boundary.iter().map(|p| p.x).fold(f64::INFINITY, f64::min),
            f.boundary.iter().map(|p| p.y).fold(f64::INFINITY, f64::min),
            flat_bottom,
        ),
        bbox_max: Vec3::new(
            f.boundary.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max),
            f.boundary.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max),
            flat_top,
        ),
        face_count: if solid.is_some() { n + 2 } else { 0 },
        edge_count: if solid.is_some() { n * 3 } else { 0 },
        vertex_count: if solid.is_some() { n * 2 } else { 0 },
        solid,
        sloped: None,
        skipped_arc_slopes: 0,
    })
}

/// A floor is a slab whose role is `Floor`.
pub fn floor_state(model: &BimModel, f: &Slab) -> Result<FloorState, Failure> {
    let s = slab_state(model, f)?;
    Ok(FloorState {
        thickness: s.thickness,
        top_z: s.top_z,
        bottom_z: s.bottom_z,
        area: s.area,
        perimeter: s.perimeter,
        volume: s.volume,
        solid: s.solid,
    })
}

/// Regenerated state of one floor (all SI units). Superseded by [`SlabState`].
#[derive(Clone, Debug, PartialEq)]
pub struct FloorState {
    pub thickness: f64,
    /// Top of the slab = level elevation + height offset (OBSERVED).
    pub top_z: f64,
    /// Bottom of the slab = top - thickness: the slab hangs **below** the level (OBSERVED).
    pub bottom_z: f64,
    /// Plan area of the boundary polygon.
    pub area: f64,
    pub perimeter: f64,
    /// area x thickness (OBSERVED).
    pub volume: f64,
    pub solid: Option<Prism>,
}

/// A circular arc: centre, radius, start angle and a SIGNED sweep (positive = counter-clockwise).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arc2 {
    pub center: Vec2,
    pub radius: f64,
    pub start_angle: f64,
    pub sweep: f64,
}

/// The arc through `start`, `through` and `end`, or `None` when the three points are collinear.
pub fn arc_from_points(start: Vec2, end: Vec2, through: Vec2) -> Option<Arc2> {
    let (ax, ay, bx, by, cx, cy) = (start.x, start.y, through.x, through.y, end.x, end.y);
    let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
    // Collinear (or coincident) points have no circumscribed circle.
    let scale = ((bx - ax).abs() + (by - ay).abs() + (cx - ax).abs() + (cy - ay).abs()).max(1e-12);
    if d.abs() < 1e-9 * scale * scale {
        return None;
    }
    let ux = ((ax * ax + ay * ay) * (by - cy) + (bx * bx + by * by) * (cy - ay) + (cx * cx + cy * cy) * (ay - by)) / d;
    let uy = ((ax * ax + ay * ay) * (cx - bx) + (bx * bx + by * by) * (ax - cx) + (cx * cx + cy * cy) * (bx - ax)) / d;
    let center = Vec2::new(ux, uy);
    let radius = (start - center).length();
    let a0 = (start.y - uy).atan2(start.x - ux);
    // Counter-clockwise when start -> through -> end turns left.
    let ccw = (through - start).cross(end - through) > 0.0;
    let a1 = (end.y - uy).atan2(end.x - ux);
    let tau = std::f64::consts::TAU;
    let sweep = if ccw {
        (a1 - a0).rem_euclid(tau)
    } else {
        -((a0 - a1).rem_euclid(tau))
    };
    Some(Arc2 { center, radius, start_angle: a0, sweep })
}

impl Arc2 {
    pub fn point_at(&self, t: f64, r: f64) -> Vec2 {
        let a = self.start_angle + self.sweep * t;
        Vec2::new(self.center.x + r * a.cos(), self.center.y + r * a.sin())
    }

    /// Axis-aligned extent of the annular sector between `r_in` and `r_out`.
    pub(crate) fn extent(&self, r_in: f64, r_out: f64) -> (Vec2, Vec2) {
        let mut pts = vec![
            self.point_at(0.0, r_in),
            self.point_at(0.0, r_out),
            self.point_at(1.0, r_in),
            self.point_at(1.0, r_out),
        ];
        // A cardinal direction inside the sweep puts the outer circle's extreme in the box.
        for k in 0..4 {
            let card = k as f64 * std::f64::consts::FRAC_PI_2;
            let rel = if self.sweep >= 0.0 {
                (card - self.start_angle).rem_euclid(std::f64::consts::TAU)
            } else {
                (self.start_angle - card).rem_euclid(std::f64::consts::TAU)
            };
            if rel <= self.sweep.abs() {
                pts.push(Vec2::new(self.center.x + r_out * card.cos(), self.center.y + r_out * card.sin()));
            }
        }
        let lo = Vec2::new(
            pts.iter().map(|p| p.x).fold(f64::INFINITY, f64::min),
            pts.iter().map(|p| p.y).fold(f64::INFINITY, f64::min),
        );
        let hi = Vec2::new(
            pts.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max),
            pts.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max),
        );
        (lo, hi)
    }
}

/// The straight-wall state with an ARC wall's own quantities laid over it.
///
/// OBSERVED:
///
/// * `Length` is the location arc: `theta x r`, exact;
/// * `Area` is the CONVEX face, `theta x (r + t/2) x h`, to 1e-15 - not the centreline, not the
///   average - and it does not move with flip or location line;
/// * `Volume` is the annular sector `(theta/2)(r_o^2 - r_i^2) h`, computed by `asterion_geom::stokes::arc_band_volume`. The
///   reference product reports a value that is above it by up to 2.3e-4 relative on the six measured walls (it integrates over
///   a tessellated model); the engine returns the exact value and the measurement stays in the tests as a bound;
/// * surface area is the sum of the six face areas (`asterion_geom::stokes::arc_band_surface_area`), exact. The reference
///   product's value is above it by 1.5e-4 .. 1.1e-3 m2 on the six measured walls;
/// * topology is 6 faces (2 cylindrical), 12 edges (4 arcs + 8 lines), 8 endpoints;
/// * the orientation is the LEFT normal of the start tangent, which is outward for a clockwise arc
///   and inward for a counter-clockwise one, and flips with the wall.
fn arc_override(model: &BimModel, w: &Wall, mut s: WallState) -> Result<WallState, Failure> {
    let Some(through) = w.through else { return Ok(s) };
    let Some(arc) = arc_from_points(w.start, w.end, through) else { return Ok(s) };
    let theta = arc.sweep.abs();
    let (t, h) = (s.width, s.height);
    let (r_in, r_out) = ((arc.radius - t / 2.0).max(0.0), arc.radius + t / 2.0);

    s.length = theta * arc.radius;
    s.solid_length = s.length;
    s.mitred_plan_area = s.length * t;
    s.gross_area = theta * r_out * h;
    s.cut_area = 0.0;
    s.area = s.gross_area;
    // The volume and the surface area come from the exact integrals of `asterion_geom::stokes`. The reference product reports
    // values slightly above them (see the documentation of this function); the arc of
    // a wall; assumed to be the arc start (t0 = 0, X radial at the start).
    //
    // An offset radius `R - w/2 < 0` is Err 5 of the offset and a zero radius is an arc of length 0, shorter than the
    // short-curve tolerance (Err 9): the reference builds NO solid for `R <= w/2`. The OBSERVED behaviour is that such a
    // wall is posted at commit as "Can't make Wall." (`check_walls`) and rolled back, so its state is never read: the
    // closed-form sector values below only keep the state total for that check; they are NOT values of the reference
    // product.
    //
    // A wall with no positive height (an imported wall whose height is not specified yet, or a state `check_walls` refuses
    // at commit) has no body: no solid, no Stokes integration. Engine rule, not a product value: the state stays total so
    // readers can report the wall as not representable instead of failing. Same contract as the straight path (`solid` is
    // `None`).
    if !(h > 0.0) {
        let sector = 0.5 * theta * (r_out * r_out - r_in * r_in);
        s.volume = 0.0;
        s.surface_area = 2.0 * sector;
        s.solid = None;
        s.unjoined_solid = None;
        return Ok(s);
    }
    // An end MITRED against another wall (forced by the curved, or stored) is the vertical plane through the two corners of
    // `crate::curved_joins`, not a radial one. The Stokes sums of the reference product are known for radial ends only, so
    // the band is computed in closed form here: the outer face exactly, the volume and the surface area within the arc
    // tolerances of the comparator (OBSERVED: Area 23.698141153 = the outer face, Volume 4.5858581 against 4.5865139,
    // surface 50.134916 against 50.134651).
    let ends: [Option<crate::curved_joins::MiterCorners>; 2] = [0usize, 1].map(|e| match crate::joins::corner_kind(model, w, e) {
        crate::joins::CornerKind::MiterAt { corners } => Some(corners),
        // A plane cut of the arc end by the side face of a line wall is the same slanted cap
        crate::joins::CornerKind::Cut { corners, circle: None, via: None } => Some(corners),
        _ => None,
    });
    if ends.iter().any(Option::is_some) && arc.radius > t / 2.0 {
        let band = crate::curved_joins::slanted_band(&arc, r_in, r_out, ends, h);
        s.mitred_plan_area = band.area;
        s.gross_area = band.outer_face.max(band.inner_face);
        s.area = s.gross_area;
        s.volume = band.area * h;
        s.surface_area = 2.0 * band.area + band.outer_face + band.inner_face + (band.caps[0] + band.caps[1]) * h;
        // the Volume and the surface area are the exact integrals with the caps moved (ENGINE CHOICE: the faces of a mitred
        // arc wall are not specified); the closed forms stay where the sums fail
        {
            let sband = asterion_geom::stokes::ArcBand::wall(arc.center, arc.start_angle, arc.sweep, r_in, r_out, s.base_z, s.top_z);
            let rng = asterion_geom::stokes::EndRanges { outer: band.outer, inner: band.inner };
            let tols = asterion_geom::stokes::TessTols::display_default();
            if let Ok(v) = asterion_geom::stokes::arc_band_volume_slanted(&sband, &rng, &tols) {
                s.volume = v;
            }
            if let Ok(a) = asterion_geom::stokes::arc_band_surface_area_slanted(&sband, &rng, &tols) {
                s.surface_area = a;
            }
        }
        // the counts follow the join classes as for a straight wall (a mitred end 10 / 8, OBSERVED; a butt corner 11 / 10
        // for the first wall and 9 / 8 for the second, OBSERVED f, j)
        let (je, jv) = crate::joins::join_edge_vertex_counts(model, w);
        s.solid_face_count = 6;
        s.solid_edge_count = je.max(0) as usize;
        s.solid_vertex_count = jv.max(0) as usize;
        s.insert_count = 0;
        let radial = (w.start - arc.center).normalized().unwrap_or(Vec2::new(1.0, 0.0));
        let tangent = if arc.sweep >= 0.0 { radial.perp_ccw() } else { Vec2::new(-radial.perp_ccw().x, -radial.perp_ccw().y) };
        let left = tangent.perp_ccw();
        s.orientation = if w.flipped { Vec2::new(-left.x, -left.y) } else { left };
        // OBSERVED (as for the straight walls of round 33): the box of a mitred wall holds its mitred body AND its
        // untrimmed band (a3: min y 3.9615 is the corner of the radial end, not of the mitre)
        let (rlo, rhi) = arc.extent(r_in, r_out);
        s.arc_bbox = Some((
            Vec3::new(band.lo.x.min(rlo.x), band.lo.y.min(rlo.y), s.base_z),
            Vec3::new(band.hi.x.max(rhi.x), band.hi.y.max(rhi.y), s.top_z),
        ));
        if !(s.top_z > s.base_z) {
            return Ok(s);   // QA: no body for a height lost in rounding
        }
        let prism = Prism::new(band.outline, s.base_z, s.top_z);
        s.unjoined_solid = Some(prism.clone());
        s.solid = Some(prism);
        return Ok(s);
    }
    let tols = asterion_geom::stokes::TessTols::display_default();
    if arc.radius <= t / 2.0 {
        let sector = 0.5 * theta * (r_out * r_out - r_in * r_in);
        s.volume = sector * h;
        s.surface_area = 2.0 * sector + theta * (r_out + r_in) * h + 2.0 * (r_out - r_in) * h;
    } else {
        let band = asterion_geom::stokes::ArcBand::wall(
            arc.center,
            arc.start_angle,
            arc.sweep,
            r_in,
            r_out,
            s.base_z,
            s.top_z,
        );
        let wrap = |e: asterion_geom::stokes::StokesError, what: &str, code: &'static str| {
            use asterion_geom::stokes::StokesError as E;
            match e {
                // Err 9 of the thickening route. How the wall regeneration maps it to a failure is not specified;
                // ASTERION's own code, posted at commit by `check_walls`.
                E::CurveTooShort => Failure::error(
                    codes::WALL_PROFILE_CURVE_TOO_SHORT,
                    "A curve of the wall's thickened profile is shorter than the short-curve tolerance.",
                ),
                // The reference facets the face when its analytic path errors; that fallback is not specified, so the
                // failure is surfaced instead of a guess.
                other => Failure::error(code, format!("Arc wall {what} integration failed: {other}.")),
            }
        };
        s.volume = asterion_geom::stokes::arc_band_volume(&band, &tols)
            .map_err(|e| wrap(e, "volume", codes::VOLUME_INTEGRATION_FAILED))?;
        // The solid surface area is the sum of the six exact face areas. A failure of the integration is surfaced.
        s.surface_area = asterion_geom::stokes::arc_band_surface_area(&band, &tols)
            .map_err(|e| wrap(e, "surface area", codes::SURFACE_AREA_INTEGRATION_FAILED))?;
    }
    // N+2 faces, 3N edges, 2N vertices for the N = 4 curve loop (6 / 12 / 8).
    let (faces, edges, vertices) =
        asterion_geom::stokes::extruded_loop_topology(asterion_geom::stokes::WALL_LOOP_CURVES);
    // the join classes move the counts of an arc wall as they do a straight wall's (OBSERVED: the host of a T 14 / 12, a
    // collinear tangent arc 8 / 8, two tangent half arcs 4 / 8)
    let (je, jv) = crate::joins::join_edge_vertex_counts(model, w);
    s.solid_face_count = faces;
    s.solid_edge_count = (edges as i64 + je - 12).max(0) as usize;
    s.solid_vertex_count = (vertices as i64 + jv - 8).max(0) as usize;
    s.insert_count = 0;

    // Orientation: left normal of the start tangent.
    let radial = (w.start - arc.center).normalized().unwrap_or(Vec2::new(1.0, 0.0));
    let tangent = if arc.sweep >= 0.0 { radial.perp_ccw() } else { Vec2::new(-radial.perp_ccw().x, -radial.perp_ccw().y) };
    let left = tangent.perp_ccw();
    s.orientation = if w.flipped { Vec2::new(-left.x, -left.y) } else { left };

    // The body's extent, analytic; and a polygonal prism so a view or the viewport has an outline.
    let (lo, hi) = arc.extent(r_in, r_out);
    s.arc_bbox = Some((Vec3::new(lo.x, lo.y, s.base_z), Vec3::new(hi.x, hi.y, s.top_z)));
    let prism = arc_body_prism(&arc, t, s.base_z, s.top_z);
    s.unjoined_solid = Some(prism.clone());
    s.solid = Some(prism);
    Ok(s)
}

/// The polygonal prism of an arc wall's body (the outline a view draws): the outer arc, then the inner arc back,
/// 96 segments a full turn (at least 6).
fn arc_body_prism(arc: &Arc2, width: f64, base_z: f64, top_z: f64) -> Prism {
    let theta = arc.sweep.abs();
    let (r_in, r_out) = ((arc.radius - width / 2.0).max(0.0), arc.radius + width / 2.0);
    let n = ((theta / std::f64::consts::TAU) * 96.0).ceil().max(6.0) as usize;
    let mut poly: Vec<Vec2> = (0..=n).map(|i| arc.point_at(i as f64 / n as f64, r_out)).collect();
    poly.extend((0..=n).rev().map(|i| arc.point_at(i as f64 / n as f64, r_in)));
    Prism::new(poly, base_z, top_z)
}

/// Regenerated state of one wall (all SI units).
#[derive(Clone, Debug, PartialEq)]
pub struct WallState {
    /// Unit vector along the location curve, start -> end.
    pub direction: Vec2,
    /// Unit normal pointing to the exterior face. OBSERVED: the left normal of the
    /// curve direction (direction rotated +90°), negated when the wall is flipped.
    pub orientation: Vec2,
    /// Location-curve (centreline) length — unchanged by joins (OBSERVED: the Length parameter
    /// does not change when a wall extends into a join).
    pub length: f64,
    /// Length of the wall *solid* after join extension/trim (`length + Σ end extensions`).
    /// Area and Volume are computed from this, not from `length`.
    pub solid_length: f64,
    /// Plan area of the **mitred** body: `solid_length x width + Σ h_q · w · cot γ`.
    ///
    /// OBSERVED: **Volume** follows this; **Area** follows the axial body; and the **bounding box** follows a third body
    /// again (see [`WallState::unjoined_solid`]). Three different solids, all reported by the reference for the same wall,
    /// and conflating any two of them was most of the 62 join mismatches.
    pub mitred_plan_area: f64,
    /// An ARC wall's analytic extent. `None` for a straight wall, whose box comes from the prism.
    pub arc_bbox: Option<(Vec3, Vec3)>,
    pub width: f64,
    /// Elevation of the base level (the location curve lies at this z). OBSERVED.
    pub level_z: f64,
    /// Bottom of the wall body: base level elevation + base offset. OBSERVED.
    pub base_z: f64,
    pub top_z: f64,
    pub height: f64,
    /// Area of the wall's elevation outline **before** any opening is subtracted
    /// (`solid_length x height`). OBSERVED for walls without joins or openings.
    pub gross_area: f64,
    /// Total area removed by this wall's openings, each clipped to the elevation outline (m2). ASSUMED - see
    /// [`WallState::area`].
    pub cut_area: f64,
    /// The wall's Area parameter: `gross_area - cut_area`.
    ///
    /// OBSERVED for walls without joins or openings: `area = length x height`. The public documentation never states
    /// whether it is gross or net; the listed falsifiers are (i) Area gross and only Volume net,
    /// (ii) both gross, (iii) net but computed on one face, (iv) the rough opening subtracted instead. Scenario steps
    ///      `wall-no-opening` -> `one-window-opening` break it or confirm it.
    pub area: f64,
    /// The wall's Volume parameter: `area x width`, i.e. the **net** area times the thickness. OBSERVED for walls without
    /// openings (`Volume = Area x width`); ASSUMED that the identity survives the subtraction.
    pub volume: f64,
    /// Total area of the wall's boundary surface, openings included.
    ///
    /// ASSUMED: `outline perimeter x width + 2 x net area + sum of reveal areas`, where one opening's four reveal faces add
    /// `2 x (w + h) x width`.
    ///
    /// That is only true for a host thick relative to the opening. A cut removes `2 x w x h` from the two large faces and
    /// adds `2 x (w + h) x t` of reveal, so the surface grows only when `t > (w x h) / (w + h)`. The sign is therefore NOT
    /// a safe check that the cut is real; the volume drop is.
    pub surface_area: f64,
    /// Predicted face count of the host solid.
    ///
    /// ASSUMED: `6 + 4n` for `n` openings that actually cut - a box is 6 faces, each opening adds its four reveal faces.
    pub solid_face_count: usize,
    /// Predicted edge count of the host solid.
    pub solid_edge_count: usize,
    /// OBSERVED: `2C` for an opened wall plus the join-class offsets.
    pub solid_vertex_count: usize,
    /// How many inserts actually cut this host - the `n` of `6 + 4n` / `12 + 12n` above.
    ///
    /// It is the count of inserts whose cut rectangle still has positive area after being clipped
    /// to the host outline, which is why it can be smaller than the number of openings pointing at
    /// this wall. Section 7e compares it as `walls.<ref>.insert_count`.
    pub insert_count: usize,
    /// The wall's body **before** any join extends or trims it.
    ///
    /// OBSERVED: the reference's reported bounding box follows this solid while its Area and Volume follow the joined one.
    /// Two different solids, and conflating them was 11 of the 62 join mismatches.
    pub unjoined_solid: Option<Prism>,
    pub location_start: Vec3,
    pub location_end: Vec3,
    /// The **un-pierced** prism over the wall footprint.
    ///
    /// The pierced solid's *metrics* are exposed instead, as [`WallState::volume`], [`WallState::surface_area`],
    /// [`WallState::solid_face_count`] and [`WallState::solid_edge_count`]; the openings' own cut prisms are in
    /// [`OpeningState::solid`]. Reading `solid.volume` therefore returns the GROSS volume on purpose - a pierced-prism type
    /// in `asterion-geom` is the next gap.
    pub solid: Option<Prism>,
}

/// The vertical span `(base_z, top_z)` of a wall's body, from its constraints alone: the same
/// arithmetic as [`wall_state`] without regenerating the solid (no joins, no openings), so a hover can
/// ask "is this wall drawn in this view" for every wall cheaply.
pub fn wall_vertical_span(model: &BimModel, w: &Wall) -> Result<(f64, f64), Failure> {
    let level = model.level(w.base_level)?;
    let base_z = level.elevation + w.base_offset;
    let height = match w.top_level {
        None => w.unconnected_height,
        Some(top) => model.level(top)?.elevation + w.top_offset - base_z,
    };
    Ok((base_z, base_z + height))
}

/// The state of a wall whose height the caller has cut to a view's cut plane: its new top cap (the cut) is read in the
/// world frame (`asterion_geom::stokes::with_clip_cap_world_frame`, ENGINE CHOICE).
pub fn wall_state_clipped(model: &BimModel, w: &Wall) -> Result<WallState, Failure> {
    asterion_geom::stokes::with_clip_cap_world_frame(|| wall_state(model, w))
}

pub fn wall_state(model: &BimModel, w: &Wall) -> Result<WallState, Failure> {
    let mut s = wall_state_base(model, w)?;
    // OBSERVED: a profile sketch replaces the elevation of the body (`crate::wall_profiles`)
    crate::wall_profiles::apply(model, w, &mut s);
    // OBSERVED: a floor joined to a wall CUTS it. The cut element loses EXACTLY `plan area x the thickness it shares with
    // the floor` of volume - 0.248 for a 6.2 x 0.2 wall under 0.2 of slab, 3.720 -> 3.472 - and its reported Area follows
    // (`Area = Volume / width`: 18.600 -> 17.360). Surface area moves by the lateral faces the band removes and the two
    // horizontal faces it adds: for a wall crossed end to end that is `-2wh`, the same -0.08 on all four walls of the
    // probe.
    for j in model.geometry_joins.iter().filter(|j| j.involves(w.id) && j.cutter != w.id) {
        let floor = j.cutter;
        let Some(o) = floor_wall_overlap(model, floor, w.id) else { continue };
        let v = o.plan_area * o.height;
        s.volume -= v;
        if s.width > 0.0 {
            s.area -= v / s.width;
        }
        let l_eff = if s.width > 0.0 { s.mitred_plan_area / s.width } else { 0.0 };
        let inside_eff = if s.width > 0.0 { o.plan_area / s.width } else { 0.0 };
        let p_in = if o.fully_inside { 2.0 * (l_eff + s.width) } else { 2.0 * inside_eff };
        let low = if o.lower_interior { 1.0 } else { -1.0 };
        let up = if o.upper_interior { 1.0 } else { -1.0 };
        s.surface_area += -o.height * p_in + o.plan_area * (low + up);
        // A band that crosses the whole wall leaves two lumps: 8 faces (the planes of both lumps
        // merge), `24 + 2 x join edge offsets` edges and `16 + join vertex offsets` vertices -
        // fitted to four walls with four different join classes, the vertices un-doubled and the
        // edges doubled. ASSUMED for any other overlap, so those keep their counts.
        if o.fully_inside && o.lower_interior && o.upper_interior {
            let (je, jv) = crate::joins::join_edge_vertex_counts(model, w);
            s.solid_face_count = 8;
            s.solid_edge_count = (24 + 2 * (je - 12)).max(0) as usize;
            s.solid_vertex_count = (16 + (jv - 8)).max(0) as usize;
        }
    }
    // OBSERVED: walls that cross in an X - see `crate::crossings` for the rules.
    crate::crossings::apply(model, w, &mut s);
    // OBSERVED: the reveals of the wall cut it
    crate::sweeps::apply_reveals(model, w, &mut s);
    // OBSERVED (one arc wall): a sweep on an ARC wall imprints its ends (`crate::arc_sweeps`)
    crate::arc_sweeps::apply(model, w, &mut s);
    Ok(s)
}

fn wall_state_base(model: &BimModel, w: &Wall) -> Result<WallState, Failure> {
    let s = wall_state_straight(model, w)?;
    let mut s = arc_override(model, w, s)?;
    // An end cut by the side face of an arc wall (a line wall, or an arc wall with a cylinder cap)
    crate::curved_ends::apply(model, w, &mut s);
    apply_arc_inserts(model, w, &mut s);
    Ok(s)
}

/// The doors and windows of an ARC wall cut it with a straight prism of the symbol's width (planes parallel to the tangent
/// frame at the origin of the instance), so the larger (outer) cylinder face loses the arc between the two planes, the plan
/// loses the strip between them, and the faces, edges and vertices follow the elevation outline as for a straight wall
/// (OBSERVED: a door on the floor 9 / 24 / 16, a window floating in the wall 10 / 24 / 16; the outer face area drops by `2
/// R_out asin(w / 2 R_out) h` = 2.0075 for the door of 1.0 x 2.0 on R 3.25, t 0.2).
///
/// ENGINE CHOICE: the Volume and the surface area are the exact removals; the reference product's values come from its
/// tessellated model and differ by about 3e-3 m3. A cut that is not wholly inside the wall keeps the straight-wall
/// arithmetic.
fn apply_arc_inserts(model: &BimModel, w: &Wall, s: &mut WallState) {
    let Some(arc) = w.through.and_then(|t| arc_from_points(w.start, w.end, t)) else { return };
    if !(s.height > 0.0 && s.width > 0.0) {
        return;
    }
    let raw = host_cuts(model, w.id);
    if raw.is_empty() {
        return;
    }
    let (r_in, r_out) = ((arc.radius - s.width / 2.0).max(0.0), arc.radius + s.width / 2.0);
    let host = cut::Rect { u_lo: 0.0, u_hi: s.length, z_lo: s.base_z, z_hi: s.top_z };
    let cuts: Vec<cut::Rect> = raw.iter().map(|(_, _, r)| cut::Rect { u_lo: r.u_lo, u_hi: r.u_hi, z_lo: r.z_lo, z_hi: r.z_hi }).collect();
    let (mut area_removed, mut vol_removed, mut surf_delta) = (0.0, 0.0, 0.0);
    for c in cuts.iter().filter_map(|c| c.normalised().clipped_to(&host)) {
        let a = (c.u_hi - c.u_lo) / 2.0;
        if !(a > 0.0 && a < r_out && (r_in == 0.0 || a < r_in)) {
            continue;
        }
        let h = c.z_hi - c.z_lo;
        let plan = |r: f64| 0.5 * (a * (r * r - a * a).sqrt() + r * r * (a / r).asin());
        let a_plan = 2.0 * (plan(r_out) - plan(r_in));
        let (patch_o, patch_i) = (r_out * 2.0 * (a / r_out).asin() * h, if r_in > 0.0 { r_in * 2.0 * (a / r_in).asin() * h } else { 0.0 });
        let side = ((r_out * r_out - a * a).sqrt() - (r_in * r_in - a * a).max(0.0).sqrt()) * h;
        area_removed += patch_o;
        vol_removed += a_plan * h;
        surf_delta += -(patch_o + patch_i) + 2.0 * side;
        surf_delta += if c.z_lo <= s.base_z + 1e-9 { -a_plan } else { a_plan };
        surf_delta += if c.z_hi >= s.top_z - 1e-9 { -a_plan } else { a_plan };
    }
    s.cut_area = area_removed;
    s.area = s.gross_area - area_removed;
    // ONE cut inside a band with radial ends: the exact quantities of the cut band (`asterion_geom::stokes::arc_band_cut_metrics`)
    let radial_ends = (0..2).all(|e| matches!(crate::joins::corner_kind(model, w, e), crate::joins::CornerKind::Butt));
    let stokes = if cuts.len() == 1 && radial_ends && r_in > 0.0 {
        let c = cuts[0].normalised();
        let a = (c.u_hi - c.u_lo) / 2.0;
        let dir = if arc.sweep >= 0.0 { 1.0 } else { -1.0 };
        let alpha = arc.start_angle + dir * ((c.u_lo + c.u_hi) / 2.0 / arc.radius);
        let (po, pi) = ((a / r_out).asin(), (a / r_in).asin());
        let band = asterion_geom::stokes::ArcBand::wall(arc.center, arc.start_angle, arc.sweep, r_in, r_out, s.base_z, s.top_z);
        let bc = asterion_geom::stokes::BandCut {
            outer: (alpha - dir * po, alpha + dir * po),
            inner: (alpha - dir * pi, alpha + dir * pi),
            z_lo: c.z_lo,
            z_hi: c.z_hi,
        };
        let inside = c.u_lo > 1e-9 && c.u_hi < s.length - 1e-9;
        inside.then(|| asterion_geom::stokes::arc_band_cut_metrics(&band, &bc, &asterion_geom::stokes::TessTols::display_default()).ok()).flatten()
    } else {
        None
    };
    match stokes {
        Some((v, sf)) => {
            s.volume = v;
            s.surface_area = sf;
        }
        None => {
            s.volume -= vol_removed;
            s.surface_area += surf_delta;
        }
    }
    // faces, edges and vertices of the elevation outline, plus the join classes
    let outline = HostOutline { u_lo: 0.0, u_hi: s.length, z_lo: s.base_z, z_hi: s.top_z };
    let cutting = cut::cutting_count(&host, &cuts);
    let (je, jv) = crate::joins::join_edge_vertex_counts(model, w);
    let (base_e, base_v) = match cut_pieces(&outline, &cuts) {
        Some(pieces) if cutting > 0 => {
            let n = cut_curve_count(&pieces) as i64;
            let imprint = imprint_point_count(&cuts, &pieces, &host) as i64;
            (3 * n + 2 * imprint, 2 * n + 2 * imprint)
        }
        _ => (12, 8),
    };
    s.solid_face_count = merged_face_count(&outline, &cuts, s.width).unwrap_or(6 + 4 * cutting);
    s.solid_edge_count = (base_e + je - 12).max(0) as usize;
    s.solid_vertex_count = (base_v + jv - 8).max(0) as usize;
    s.insert_count = cutting;
}

/// `None` only for a path with no direction.
fn straight_wall_prism(start: Vec2, end: Vec2, base_z: f64, top_z: f64, width: f64) -> Option<Prism> {
    // QA: a height lost in rounding (1e-300 m on a level at 20 m) leaves a top that is not above the base: no body, not a panic
    if !(top_z > base_z) {
        return None;
    }
    let profile =
        asterion_geom::wall_profile::StraightWallProfile::new(start, end, base_z, top_z - base_z, width).ok()?;
    Some(Prism::new(profile.plan_corners().to_vec(), base_z, top_z))
}

/// The convex hull of plan points, counter-clockwise (monotone chain); the points themselves when
/// fewer than three distinct ones remain.
fn convex_hull(mut pts: Vec<Vec2>) -> Vec<Vec2> {
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup_by(|a, b| (*a - *b).length() <= 1e-12);
    if pts.len() < 3 {
        return pts;
    }
    let mut hull: Vec<Vec2> = Vec::with_capacity(pts.len() * 2);
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &Vec2>> = if pass == 0 { Box::new(pts.iter()) } else { Box::new(pts.iter().rev()) };
        for p in iter {
            while hull.len() >= start + 2 && (hull[hull.len() - 1] - hull[hull.len() - 2]).cross(*p - hull[hull.len() - 2]) <= 0.0 {
                hull.pop();
            }
            hull.push(*p);
        }
        hull.pop();
    }
    hull
}

/// The corners of a MITRED end moved onto the mitre plane (the side faces trimmed or extended by it). The prism's corners
/// at that end sit at the location end (a mitred end has no axial extension, `crate::joins::end_effect`); any other end is
/// left as it is.
fn mitred_prism(model: &BimModel, w: &Wall, direction: Vec2, prism: Prism) -> Prism {
    let offs = [crate::joins::mitre_corner_offsets(model, w, 0), crate::joins::mitre_corner_offsets(model, w, 1)];
    if offs.iter().all(Option::is_none) {
        return prism;
    }
    let n = direction.perp_ccw();
    let mid = (w.start + w.end) * 0.5;
    let footprint = prism
        .footprint
        .iter()
        .map(|c| {
            let end = usize::from((*c - mid).dot(direction) >= 0.0);
            match offs[end] {
                Some((left, right)) => {
                    let mu = if (*c - mid).dot(n) > 0.0 { left } else { right };
                    let outward = if end == 0 { -direction } else { direction };
                    *c + outward * mu
                }
                None => *c,
            }
        })
        .collect();
    Prism::new(footprint, prism.z0, prism.z1)
}

/// The first part of a straight wall's regeneration: its path, its joined ends and the prism of its body. This is
/// all a plan view needs of the wall (its footprint and its base and top), and it is everything
/// `wall_state_straight` computes before the quantities (volume, area, openings, edge counts): those cost more
/// than the body, and a view of a large document needs the bodies of every wall.
struct StraightBody {
    direction: Vec2,
    orientation: Vec2,
    length: f64,
    width: f64,
    level_z: f64,
    base_z: f64,
    top_z: f64,
    height: f64,
    ext_start: f64,
    ext_end: f64,
    solid_length: f64,
    solid: Option<Prism>,
}

/// A straight wall's frame: the unit direction of its location line, its orientation (the
/// direction's left normal, negated when flipped) and its total layers width. The part of
/// [`straight_body`] that does not look at any other wall (no join), so a symbol that is drawn on
/// a wall can be placed without regenerating the wall.
pub(crate) fn wall_frame(model: &BimModel, w: &Wall) -> Result<(Vec2, Vec2, f64, f64), Failure> {
    let wall_type = model.wall_type(w.type_id)?;
    let delta = w.end - w.start;
    let length = delta.length();
    let direction = delta.normalized().ok_or_else(|| Failure::error(codes::CURVE_TOO_SHORT, "Wall curve has zero length."))?;
    // OBSERVED: a CURTAIN wall's orientation is the negated left normal
    let curtain = model.curtain_types.contains_key(&w.type_id);
    let orientation = if w.flipped != curtain { -direction.perp_ccw() } else { direction.perp_ccw() };
    Ok((direction, orientation, length, wall_type.structure.width()))
}

/// What a plan view needs to draw the symbol of a hosted instance on a STRAIGHT host: the kind, the
/// cut's range along the host, and the host's start, direction, orientation and width - equal to
/// the same fields of [`hosted_instance_state`] and [`wall_state`], without regenerating the host
/// (its joins, openings and quantities). `None` for an arc host or anything that state would fail on.
pub struct HostedPlanFrame {
    pub kind: crate::model::HostedKind,
    pub u_lo: f64,
    pub u_hi: f64,
    pub start: Vec2,
    pub direction: Vec2,
    pub orientation: Vec2,
    pub width: f64,
}

pub fn hosted_plan_frame(model: &BimModel, i: &HostedInstance) -> Option<HostedPlanFrame> {
    let t = model.hosted_type(i.type_id).ok()?;
    let host = model.wall(i.host).ok()?;
    if host.through.is_some() {
        return None;
    }
    model.level(i.level).ok()?;
    model.level(host.base_level).ok()?;
    if let Some(top) = host.top_level {
        model.level(top).ok()?;
    }
    let (direction, orientation, _, width) = wall_frame(model, host).ok()?;
    let rect = hosted_cut_rect(model, i).ok()?;
    Some(HostedPlanFrame { kind: t.kind, u_lo: rect.u_lo, u_hi: rect.u_hi, start: host.start, direction, orientation, width })
}

fn straight_body(model: &BimModel, w: &Wall) -> Result<StraightBody, Failure> {
    let level = model.level(w.base_level)?;
    let (direction, orientation, length, width) = wall_frame(model, w)?;
    let level_z = level.elevation;
    let base_z = level_z + w.base_offset;
    let height = match w.top_level {
        None => w.unconnected_height,
        Some(level) => model.level(level)?.elevation + w.top_offset - base_z,
    };
    let top_z = base_z + height;

    // Automatic wall joins extend/trim the SOLID at each end (not the location curve).
    let ext_start = crate::joins::end_extension(model, w, 0);
    let ext_end = crate::joins::end_extension(model, w, 1);
    let solid_length = (length + ext_start + ext_end).max(0.0);
    // Solid footprint endpoints: extend outward (−direction at start, +direction at end).
    let fstart = w.start - direction * ext_start;
    let fend = w.end + direction * ext_end;

    // The body of a straight vertical wall is ONE profile loop (base path curve, end line, reversed top copy, closing line)
    // moved by -width/2 along t x Z and extruded by the width; its plan corners are what the footprint is made of. A flip
    // changes only the orientation parameter, never the body, which is symmetric about the path curve.
    let solid = (height > 0.0 && width > 0.0 && solid_length > 0.0)
        .then(|| straight_wall_prism(fstart, fend, base_z, top_z, width))
        .flatten()
        .map(|p| mitred_prism(model, w, direction, p));
    Ok(StraightBody { direction, orientation, length, width, level_z, base_z, top_z, height, ext_start, ext_end, solid_length, solid })
}

/// What a plan view needs of a wall: the footprint of its body, and its base and top elevation. For a straight wall
/// only the body is regenerated; for an arc wall the outline of its body, without the volume and surface
/// integrations of its quantities (they were 30 ms of every edit of a 1 183-wall document). The result equals the
/// footprint, `base_z` and `top_z` of [`wall_state`] for the same wall whenever `wall_state` succeeds (a failed
/// quantity integration does not take the wall's outline out of a view).
pub fn wall_view_body(model: &BimModel, w: &Wall) -> Result<(Vec<Vec2>, f64, f64), Failure> {
    let b = straight_body(model, w)?;
    // an arc wall: the outline of its body without the volume and surface integrations of its quantities
    // (the same prism `arc_override` builds; no body when it has no positive height)
    if b.height > 0.0 {
        // A body whose end is cut by the side face of an arc wall is the loop of `crate::curved_ends`
        if let Some(l) = crate::curved_ends::wall_loop(model, w, b.width / 2.0) {
            if l.polygon.len() >= 3 {
                return Ok((l.polygon, b.base_z, b.top_z));
            }
        }
    }
    if let Some(arc) = w.through.and_then(|through| arc_from_points(w.start, w.end, through)) {
        let footprint = if b.height > 0.0 {
            let ends: [Option<crate::curved_joins::MiterCorners>; 2] = [0usize, 1].map(|e| match crate::joins::corner_kind(model, w, e) {
                crate::joins::CornerKind::MiterAt { corners } => Some(corners),
                crate::joins::CornerKind::Cut { corners, circle: None, via: None } => Some(corners),
                _ => None,
            });
            let (r_in, r_out) = ((arc.radius - b.width / 2.0).max(0.0), arc.radius + b.width / 2.0);
            if ends.iter().any(Option::is_some) && arc.radius > b.width / 2.0 {
                crate::curved_joins::slanted_band(&arc, r_in, r_out, ends, b.height).outline
            } else {
                arc_body_prism(&arc, b.width, b.base_z, b.top_z).footprint
            }
        } else {
            Vec::new()
        };
        return Ok((footprint, b.base_z, b.top_z));
    }
    let footprint = b.solid.as_ref().map(|p| p.footprint.clone()).unwrap_or_default();
    Ok((footprint, b.base_z, b.top_z))
}

fn wall_state_straight(model: &BimModel, w: &Wall) -> Result<WallState, Failure> {
    let StraightBody { direction, orientation, length, width, level_z, base_z, top_z, height, ext_start, ext_end, solid_length, solid } =
        straight_body(model, w)?;

    // OBSERVED: the reported bounding box is a THIRD body again - extended where this wall is the through wall, but NOT
    // trimmed where it butts. Measured both ways: `a_h` bbox max x = 5.241421356237309 = 5.000 + its through extension, so
    // the extension IS in the box; `a_d` bbox min = (4.9292893218813445, -0.07071067811865477), the corners of its
    // UNTRIMMED band, so the trim is NOT. Confirmed on `J2`, `thin_V`, `B_H`, `thick_V`, `A_V` and `stem` as well.
    //
    // This is an engine rule, not a geometry gap: the reference never shrinks the box a join
    // trims, and it does grow the box a join extends.
    //
    // OBSERVED (`-097`): the box of a MITRED wall holds its mitred body AND its untrimmed band (the far corner of the mitre
    // grows the box, the near one does not shrink it: `c10_2` min x = 6 - 0.1 sin 10 deg, `b2` min x 5.9134), and the box
    // of a THROUGH wall reaches the corners of its real end plane (`crate::joins::through_corner_offsets`) rather than its
    // axial extension: the band to the location end plus those corners.
    let through = [crate::joins::through_corner_offsets(model, w, 0), crate::joins::through_corner_offsets(model, w, 1)];
    let mitre = [crate::joins::mitre_corner_offsets(model, w, 0), crate::joins::mitre_corner_offsets(model, w, 1)];
    let bbox_ext = |e: usize, ext: f64| if through[e].is_some() { 0.0 } else { ext.max(0.0) };
    let bbox_start = w.start - direction * bbox_ext(0, ext_start);
    let bbox_end = w.end + direction * bbox_ext(1, ext_end);
    let unjoined_solid = (height > 0.0 && width > 0.0 && (bbox_end - bbox_start).length() > 0.0)
        .then(|| straight_wall_prism(bbox_start, bbox_end, base_z, top_z, width))
        .flatten()
        .map(|p| {
            let n = direction.perp_ccw();
            let half = width / 2.0;
            let mut extra: Vec<Vec2> = Vec::new();
            for (e, loc) in [(0usize, w.start), (1usize, w.end)] {
                let outward = if e == 0 { -direction } else { direction };
                if let Some((l, r)) = through[e].or(mitre[e]) {
                    extra.push(loc + n * half + outward * l);
                    extra.push(loc - n * half + outward * r);
                }
            }
            if extra.is_empty() {
                p
            } else {
                let mut pts = p.footprint.clone();
                pts.extend(extra);
                Prism::new(convex_hull(pts), p.z0, p.z1)
            }
        });

    // OBSERVED: the joined body is a boolean difference, not an axial trim. `crate::joins::joined_plan_area` holds the
    // rule.
    //
    // The axial `solid_length` stays the fallback: if the boolean cannot describe the corner, the honest answer is the
    // model that is at least right about the length. OBSERVED: Volume follows the MITRED body, Area follows the AXIAL one.
    // The two are measured apart - `a_d` reports Area 12.603657992645926 (the axial 4.2012193309 x 3.000) and Volume
    // 2.4607315985292 (the mitred solid) - so `Volume = Area x width` does NOT hold for a joined wall. That identity holds
    // everywhere else in the engine and it breaks here in the REFERENCE, not in ASTERION.
    let mitre_correction = crate::joins::mitre_plan_correction(model, w);
    let mitre_cap_extra = crate::joins::mitre_cap_extra_length(model, w);
    let axial_plan_area = solid_length * width;
    let mitred_plan_area = (axial_plan_area + mitre_correction).max(0.0);

    // The elevation outline of the host, in (station along the location curve, absolute z).
    // The solid runs from -ext_start to length + ext_end, so an opening sitting in a join
    // extension is still inside the host (ASSUMED; both agree on an unjoined wall, which is what
    // every round-9 scenario uses).
    let outline = HostOutline { u_lo: -ext_start, u_hi: length + ext_end, z_lo: base_z, z_hi: top_z };

    // OBSERVED: the host loses the area of the **union** of its openings, each **clipped** to the host, and its surface is
    // `2 x net area + perimeter of the net region x thickness`. The rule and its verification against the reference
    // product's own 24 measured values live in `crate::cut`; this is only the wiring.
    //
    // What was here before summed each clipped rectangle and added a per-opening reveal term.
    // Both are right only while openings are disjoint and wholly inside the host - which is what
    // ASTERION used to enforce with three failure codes that round 9 deleted, because the
    // reference clips and merges instead of refusing. With overlap legal, a sum double-counts the
    // shared area and the reveal term double-counts the shared edge.
    //
    // DOCUMENTED (sections 2a, 6c): there is ONE host-side insert relation, so a hosted door or
    // window contributes its cut here exactly as a rectangular opening does, with no special case.
    // That is what makes the host's net Area, Volume and surface area blind to which element kind
    // cut it.
    let host_rect = cut::Rect { u_lo: outline.u_lo, u_hi: outline.u_hi, z_lo: outline.z_lo, z_hi: outline.z_hi };
    let cuts: Vec<cut::Rect> = host_cuts(model, w.id)
        .into_iter()
        .map(|(_, _, r)| cut::Rect { u_lo: r.u_lo, u_hi: r.u_hi, z_lo: r.z_lo, z_hi: r.z_hi })
        .collect();
    // Area stays on the AXIAL body - see the note above `mitre_correction`.
    //
    // The wall Area is the LARGER of the two side faces (a maximum, no mitre-specific code), measured on the geometry
    // regeneration left. The axial length above is that larger face for the recorded corners (equal widths, one oblique
    // end).
    let axial_gross_area = solid_length * height;
    let face_length = crate::joins::larger_side_face_length(model, w, solid_length, mitre_correction);
    let gross_area = face_length * height;
    let cut_area = cut::union_area(&host_rect, &cuts);
    let area = gross_area - cut_area;
    // The two side faces together are `2 x mean x height`, whichever of them is the larger: the surface-area terms below
    // were fitted on that sum, so they keep the axial area.
    let axial_area = axial_gross_area - cut_area;
    let cutting = cut::cutting_count(&host_rect, &cuts);
    let net_perimeter = cut::net_region_perimeter(&host_rect, &cuts);
    Ok(WallState {
        direction,
        orientation,
        length,
        solid_length,
        mitred_plan_area,
        arc_bbox: None,
        width,
        level_z,
        base_z,
        top_z,
        height,
        gross_area,
        cut_area,
        area,
        // OBSERVED: the mitred plan area times the height, less what the openings remove. For an unjoined wall the
        // correction is zero and this is `area * width` exactly, which is why no round-1..4 or round-9 field moves.
        volume: mitred_plan_area * height - cut_area * width,
        // OBSERVED: 2 x net area + the net region's perimeter x thickness. This reproduces every measured surface area, the
        // overlapping and clipped cases included. OBSERVED: `2 x net area + the net region's perimeter x thickness`
        // reproduces every measured surface area with openings, the overlapping and clipped ones included.
        //
        // OBSERVED: a joined wall needs two more terms, and both vanish for an unjoined one, so nothing else moves. The
        // mitre correction changes the body's effective elevation length by `mc / w`, which shows up twice - once in the
        // two large faces and once in the perimeter - and the mitred end cap is longer than the square one it replaces.
        surface_area: 2.0 * axial_area
            + net_perimeter * width
            + 2.0 * (mitre_correction / width) * height
            + 2.0 * mitre_correction
            + mitre_cap_extra * height,
        // OBSERVED: coplanar side faces count as one, so a flush opening adds 3 and a floating one adds 4. The old `6 + 4n`
        // could not express that difference and was falsified by the measurement. The prediction stays only as the fallback
        // for a region the kernel cannot form.
        solid_face_count: merged_face_count(&outline, &cuts, width).unwrap_or(6 + 4 * cutting),
        // OBSERVED: an opened wall has `3C` edges and `2C` vertices, `C` being the boundary curves of the net region; a
        // JOINED wall adds the offset of every join incidence (a step function of the join class, section 2).
        solid_edge_count: {
            let (je, _) = crate::joins::join_edge_vertex_counts(model, w);
            let join_delta = je - 12;
            let base = match cut_pieces(&outline, &cuts) {
                Some(pieces) if cutting > 0 => {
                    let host = cut::Rect {
                        u_lo: outline.u_lo,
                        u_hi: outline.u_hi,
                        z_lo: outline.z_lo,
                        z_hi: outline.z_hi,
                    };
                    3 * cut_curve_count(&pieces) as i64
                        + 2 * imprint_point_count(&cuts, &pieces, &host) as i64
                }
                _ => 12,
            };
            (base + join_delta).max(0) as usize
        },
        solid_vertex_count: {
            let (_, jv) = crate::joins::join_edge_vertex_counts(model, w);
            let join_delta = jv - 8;
            let base = match cut_pieces(&outline, &cuts) {
                Some(pieces) if cutting > 0 => {
                    let host = cut::Rect {
                        u_lo: outline.u_lo,
                        u_hi: outline.u_hi,
                        z_lo: outline.z_lo,
                        z_hi: outline.z_hi,
                    };
                    2 * cut_curve_count(&pieces) as i64
                        + 2 * imprint_point_count(&cuts, &pieces, &host) as i64
                }
                _ => 8,
            };
            (base + join_delta).max(0) as usize
        },
        insert_count: cutting,
        unjoined_solid,
        location_start: Vec3::from_xy(w.start, level_z),
        location_end: Vec3::from_xy(w.end, level_z),
        solid,
    })
}

// ------------------------------------------------------------------ openings / inserts
//
// SPECIFIED, NOT MEASURED: every rule here is ASSUMED unless marked otherwise, and the numbers it commits to are listed in
// the `tests` module below.

/// The host's elevation outline in (station, absolute z) coordinates. Not part of the model: it is
/// recomputed from the host wall on every read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HostOutline {
    pub u_lo: f64,
    pub u_hi: f64,
    pub z_lo: f64,
    pub z_hi: f64,
}

/// A rectangle in host (station, absolute z) coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutRect {
    pub u_lo: f64,
    pub u_hi: f64,
    pub z_lo: f64,
    pub z_hi: f64,
}

impl CutRect {
    pub fn width(&self) -> f64 {
        self.u_hi - self.u_lo
    }

    pub fn height(&self) -> f64 {
        self.z_hi - self.z_lo
    }

    pub fn area(&self) -> f64 {
        self.width() * self.height()
    }

    /// Area shared with `other`; 0 when they only touch or are disjoint.
    pub fn overlap_area(&self, other: &CutRect) -> f64 {
        let du = (self.u_hi.min(other.u_hi) - self.u_lo.max(other.u_lo)).max(0.0);
        let dz = (self.z_hi.min(other.z_hi) - self.z_lo.max(other.z_lo)).max(0.0);
        du * dz
    }
}

impl HostOutline {
    /// Intersection of an opening's rectangle with the host outline, or `None` when the
    /// intersection has no area.
    ///
    /// ASSUMED: the subtracted quantity is the cut rectangle **clipped to the wall outline**. if the measurement shows the
    /// reference clips-and-commits, removing that report is the whole change.
    pub fn clip(&self, o: &WallOpening) -> Option<CutRect> {
        self.clip_rect(o.rect())
    }

    /// The same clip, expressed on the generic cut rectangle so that a hosted instance and a
    /// rectangular opening go through one code path (section 6c).
    pub fn clip_rect(&self, r: CutRect) -> Option<CutRect> {
        let c = CutRect {
            u_lo: r.u_lo.max(self.u_lo),
            u_hi: r.u_hi.min(self.u_hi),
            z_lo: r.z_lo.max(self.z_lo),
            z_hi: r.z_hi.min(self.z_hi),
        };
        (c.u_hi > c.u_lo && c.z_hi > c.z_lo).then_some(c)
    }

    pub fn contains_u(&self, o: &WallOpening, slack: f64) -> bool {
        self.contains_rect_u(o.rect(), slack)
    }

    pub fn contains_z(&self, o: &WallOpening, slack: f64) -> bool {
        self.contains_rect_z(o.rect(), slack)
    }

    pub fn contains_rect_u(&self, r: CutRect, slack: f64) -> bool {
        r.u_lo >= self.u_lo - slack && r.u_hi <= self.u_hi + slack
    }

    pub fn contains_rect_z(&self, r: CutRect, slack: f64) -> bool {
        r.z_lo >= self.z_lo - slack && r.z_hi <= self.z_hi + slack
    }
}

/// The host's elevation outline: a pure function of the host wall.
pub fn host_outline(model: &BimModel, w: &Wall) -> Result<HostOutline, Failure> {
    let s = wall_state(model, w)?;
    let ext_start = crate::joins::end_extension(model, w, 0);
    Ok(HostOutline { u_lo: -ext_start, u_hi: -ext_start + s.solid_length, z_lo: s.base_z, z_hi: s.top_z })
}

/// One entry of the host-side insert relation: *this thing is inserted into that host, and this is
/// the rectangle it cuts*.
///
/// DOCUMENTED: the reference retrieves hosted instances and rectangular openings through a single query with a single
/// native binding behind it, so ASTERION models the relation **once** and the elements separately. This relation is derived
/// and never stored; `regen` is the only thing that consumes it to subtract volume.
///
/// It is needed because section 7e compares a host's `inserts[]` as a sorted list of element refs, which a `{host, kind,
/// cut}` triple cannot produce.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Insert {
    pub host: ElementId,
    pub element: ElementId,
    pub kind: InsertKind,
    /// The cut rectangle, clipped to the host outline. `None` when the cut misses the host
    /// entirely (which is a commit-time failure, so it is only reachable mid-transaction).
    pub cut: Option<CutRect>,
}

/// The insert relation of one host, sorted by the cut's lower station.
///
/// DOCUMENTED (section 7b): "insert list of a host - the `Insert` relation, sorted by `u_min`".
pub fn inserts(model: &BimModel, host: ElementId) -> Result<Vec<Insert>, Failure> {
    let w = model.wall(host)?;
    let outline = host_outline(model, w)?;
    let mut v: Vec<Insert> = host_cuts(model, host)
        .into_iter()
        .map(|(element, kind, r)| Insert { host, element, kind, cut: outline.clip_rect(r) })
        .collect();
    v.sort_by(|a, b| {
        let key = |i: &Insert| i.cut.map(|c| c.u_lo).unwrap_or(f64::INFINITY);
        key(a).total_cmp(&key(b)).then(a.element.cmp(&b.element))
    });
    Ok(v)
}

/// Whether the step of one rectangular opening removes material that no earlier step removed.
///
/// The step records "did cut something" after its boolean, and a warning is posted when it did not. Every rectangular
/// opening owns ONE step in the host's step list, appended in creation order (element id order), so the boolean of an
/// opening runs on the wall geometry that the earlier openings already modified (two consecutive booleans, no union
/// computed by the step). The opening therefore cuts something exactly when its rectangle, clipped to the host, adds area
/// to the union of the rectangles of the EARLIER openings. This is the engine's own reading of "did cut something"; the
/// operation behind the reference product's flag was not specified, so the condition is the geometric one, and what a
/// HOSTED INSTANCE's cut contributes to the "earlier" set is UNKNOWN (the order in which the regen engine runs the steps):
/// instances are left out.
///
/// `None` when `opening` is not a rectangular opening or its host cannot be evaluated.
pub fn opening_cuts_something(model: &BimModel, opening: ElementId) -> Option<bool> {
    let o = model.wall_openings.get(&opening)?;
    let host = model.walls.get(&o.host)?;
    let outline = host_outline(model, host).ok()?;
    let host_rect = cut::Rect { u_lo: outline.u_lo, u_hi: outline.u_hi, z_lo: outline.z_lo, z_hi: outline.z_hi };
    let as_cut = |r: CutRect| cut::Rect { u_lo: r.u_lo, u_hi: r.u_hi, z_lo: r.z_lo, z_hi: r.z_hi };
    let mut rects: Vec<cut::Rect> = model
        .wall_openings
        .values()
        .filter(|p| p.host == o.host && p.id < o.id)
        .map(|p| as_cut(p.rect()))
        .collect();
    let before = cut::union_area(&host_rect, &rects);
    rects.push(as_cut(o.rect()));
    let after = cut::union_area(&host_rect, &rects);
    Some(after - before > 1e-12)
}

/// Regenerated state of one rectangular wall opening (all SI units). Nothing here is stored.
#[derive(Clone, Debug, PartialEq)]
pub struct OpeningState {
    /// DOCUMENTED (section 6b): a wall opening created from two corners always reports a
    /// rectangular boundary (`Opening.IsRectBoundary == true`, `BoundaryCurves == null`).
    pub is_rect_boundary: bool,
    /// The opening's own rectangle in host coordinates, before clipping.
    pub rect: CutRect,
    /// Width along the host location curve (m).
    pub width: f64,
    /// Height (m).
    pub height: f64,
    /// Area of its own rectangle (m2). This is the `openings.<ref>.cut_area` of section 7e.
    pub cut_area: f64,
    /// Area actually removed from the host, i.e. the rectangle clipped to the host outline (m2).
    pub clipped_cut_area: f64,
    /// Depth of the cut: the **full** host thickness.
    /// ASSUMED (section 7b): a `WallOpening` cuts the whole wall width - it has no inset, which is
    /// a window-type property ASTERION does not model in v0.1.
    pub thickness: f64,
    /// `clipped_cut_area x thickness` (m3): what the host's Volume loses.
    pub cut_volume: f64,
    /// World-space corners of the boundary rectangle (section 7e `boundary_rect.{min,max}`),
    /// taken on the host's **exterior** face plane.
    pub boundary_min: Vec3,
    pub boundary_max: Vec3,
    /// The subtracted prism itself, or `None` when the cut misses the host.
    pub solid: Option<Prism>,
    /// Whether the cut stays inside the host horizontally / vertically. False means the
    /// transaction that produced this state is failing at commit (see `ops::check_openings`).
    pub inside_host_u: bool,
    pub inside_host_z: bool,
}

/// Derived state of a wall opening: a pure function of the model.
pub fn opening_state(model: &BimModel, o: &WallOpening) -> Result<OpeningState, Failure> {
    let host = model.wall(o.host)?;
    let s = wall_state(model, host)?;
    let ext_start = crate::joins::end_extension(model, host, 0);
    let outline = HostOutline { u_lo: -ext_start, u_hi: -ext_start + s.solid_length, z_lo: s.base_z, z_hi: s.top_z };
    let slack = crate::spec::insert_fit_tolerance();
    let rect = CutRect { u_lo: o.u_min, u_hi: o.u_max, z_lo: o.z_min, z_hi: o.z_max };
    let clipped = outline.clip(o);
    let clipped_cut_area = clipped.map(|c| c.area()).unwrap_or(0.0);
    let thickness = s.width;
    let half = s.orientation * (thickness / 2.0);
    let at = |u: f64| host.start + s.direction * u;
    let (p_lo, p_hi) = (at(o.u_min) + half, at(o.u_max) + half);
    let solid = clipped.and_then(|c| {
        let (a, b) = (at(c.u_lo), at(c.u_hi));
        (c.z_hi > c.z_lo && thickness > 0.0)
            .then(|| Prism::new(vec![a + half, b + half, b - half, a - half], c.z_lo, c.z_hi))
    });
    Ok(OpeningState {
        is_rect_boundary: true,
        rect,
        width: o.width(),
        height: o.height(),
        cut_area: o.cut_area(),
        clipped_cut_area,
        thickness,
        cut_volume: clipped_cut_area * thickness,
        boundary_min: Vec3::from_xy(p_lo, o.z_min),
        boundary_max: Vec3::from_xy(p_hi, o.z_max),
        solid,
        inside_host_u: outline.contains_u(o, slack),
        inside_host_z: outline.contains_z(o, slack),
    })
}

// ----------------------------------------------------------- hosted instances (derived)
//
// Everything in this block is a pure function of the model; nothing here is stored, and every rule is ASSUMED unless its
// doc comment says otherwise.

/// The cut rectangle of a hosted instance, in host (station, **absolute** z) coordinates.
///
/// ASSUMED (section 7b): `u in [host_station - width/2, host_station + width/2]`,
/// `z in [level.z + sill_height, level.z + sill_height + type.height]`, over the full wall
/// thickness - "the instance is centred on its station".
///
/// Two consequences worth stating, because the measurement can break either:
/// * the cut is driven by the type's `width`/`height`, **not** by `rough_width`/`rough_height` (INFERRED, section 2a: the
///   rough dimensions are documented as reporting values with no stated geometric effect).
/// * the elevation datum is the **instance's level**, not the host's base. For every round-9 scenario the two coincide.
pub fn hosted_cut_rect(model: &BimModel, i: &HostedInstance) -> Result<CutRect, Failure> {
    let t = model.hosted_type(i.type_id)?;
    let level = model.level(i.level)?;
    let z_lo = level.elevation + i.sill_height;
    if let Some(c) = i.cut_loop {
        // the instance's own cut loop (ENGINE CHOICE): relative to its station and sill
        return Ok(CutRect {
            u_lo: i.host_station + c.u_min,
            u_hi: i.host_station + c.u_max,
            z_lo: z_lo + c.z_min,
            z_hi: z_lo + c.z_max,
        });
    }
    let half = i.width_in(t) / 2.0;
    Ok(CutRect { u_lo: i.host_station - half, u_hi: i.host_station + half, z_lo, z_hi: z_lo + i.height_in(t) })
}

/// Every cut carried by `host`, **unclipped**, from *both* element kinds, in ascending element id.
///
/// DOCUMENTED (sections 2a, 6c): the reference retrieves hosted instances and rectangular openings
/// through one query with one native binding behind it, so this is the single place ASTERION
/// enumerates "things inserted into a host". [`wall_state`], [`inserts`] and the commit-time fit
/// checks all consume it, which is why a door and a rectangular opening cannot diverge in the
/// host's arithmetic.
///
/// An instance whose type or level is missing contributes nothing rather than failing: such a
/// model is unreachable through [`crate::ops`], and a host's quantities must stay readable even
/// mid-transaction.
/// Face count of a wall's solid, by the reference's own counting convention.
///
/// OBSERVED (and the geometry lane's `face_count_merged`): **the reference counts coplanar side faces as ONE**. That is the
/// only thing that explains the asymmetry round 9 measured — a floating opening adds **4** faces, an opening flush with the
/// base adds **3** — because a flush opening is not a hole but a **notch**, and its two bottom pieces lie in the same `z =
/// 0` plane. One face per boundary curve gives 10; distinct supporting planes give the measured 9.
///
/// It is the same convention that keeps a joined wall at 6 faces: an axially trimmed box still has
/// four distinct side planes.
///
/// Builds the wall's **elevation** region — the face rectangle minus its openings — and extrudes
/// it through the thickness, which is the solid the reference is describing. Returns `None` when
/// the region cannot be formed, and the caller keeps the earlier prediction rather than guessing.
fn merged_face_count(outline: &HostOutline, cuts: &[cut::Rect], thickness: f64) -> Option<usize> {
    if thickness <= 0.0 {
        return None;
    }
    let pieces = cut_pieces(outline, cuts)?;
    Some(face_count_of(outline, cuts, &pieces))
}

/// The wall's elevation face minus the UNION of its openings, each clipped to the host.
///
/// The openings are unioned before anything is subtracted: subtracting them one at a time leaves
/// an extra split along the seam where two overlap, and the count comes out one too high on every
/// overlapping row - the off-by-one this wiring showed before it followed the kernel's own
/// procedure. Returns every disconnected piece; the reference still reports ONE solid for a host
/// an over-wide opening splits, so the pieces are always counted together.
pub(crate) fn cut_pieces(outline: &HostOutline, cuts: &[cut::Rect]) -> Option<Vec<Region>> {
    let tol = Tolerances::default();
    let rect = |u_lo: f64, z_lo: f64, u_hi: f64, z_hi: f64| -> Option<Region> {
        Polycurve::rectangle(Vec2::new(u_lo, z_lo), Vec2::new(u_hi, z_hi), &tol)
            .ok()
            .map(Region::from_loop)
    };
    let host = rect(outline.u_lo, outline.z_lo, outline.u_hi, outline.z_hi)?;

    // The openings are clipped to the host and then UNIONED before anything is subtracted.
    // Subtracting them one at a time instead leaves an extra split along the seam where two
    // openings overlap, and the face count comes out one too high on every overlapping row - the
    // exact off-by-one this wiring showed before it was aligned with the kernel's own procedure.
    let mut clipped: Vec<Region> = Vec::new();
    for r in cuts {
        let c = r.normalised();
        let o = rect(c.u_lo, c.z_lo, c.u_hi, c.z_hi)?;
        clipped.extend(o.clipped_to(&host, &tol).ok()?);
    }
    let mut pieces = vec![host];
    if !clipped.is_empty() {
        for hole in &asterion_geom::union_all(&clipped, &tol).ok()? {
            let mut next: Vec<Region> = Vec::new();
            for p in &pieces {
                next.extend(p.difference(hole, &tol).ok()?);
            }
            pieces = next;
        }
    }
    if pieces.is_empty() {
        return None;
    }
    Some(pieces)
}

/// Subdivision points a NESTED cut leaves on the boundary of the net region.
///
/// OBSERVED: a cut that lies inside another cut and shares part of its boundary imprints that boundary - every point where
/// its corner meets the container's edge subdivides that edge on the front AND the back face, which is `+2` edges and `+2`
/// vertices per point. A point that is already a vertex adds nothing, which is why a cut identical to its container adds
/// `+0` and a cut in the container's corner adds `+2`.
///
/// Probe measured it in isolation: bottom edge `+4`, top edge `+4`, left edge `+4`, corner `+2`, identical `+0`, strictly
/// inside `+0`, and deleting the bottom-edge one `-4` - the count follows the current set of cuts, not their history.
///
/// **Which imprints survive a merge** was the open question. One probe step answered it: a
/// collinear neighbour that extends the container sideways dropped the count by 6, leaving only the
/// imprints on the container's untouched left edge. The rule that fits every measured row is:
///
/// > an imprint on a container edge survives while the **net region's maximal boundary edge**
/// > carrying it lies WITHIN that container edge. When a neighbour extends the boundary past the
/// > container's own edge, the imprints on it go.
///
/// Round 9 agrees from the other side: the "hanging off the end" opening sits inside a full-width
/// band whose bottom edge is shortened (not extended) by the door notch, so its imprints survive.
///
/// ASSUMED beyond the measured rows: that the points are independent of one another and that this
/// is the whole mechanism. Cuts are compared after clipping to the host, as the cut region is.
fn imprint_point_count(cuts: &[cut::Rect], pieces: &[Region], host: &cut::Rect) -> usize {
    imprint_points(cuts, pieces, host).len()
}

/// The imprint points themselves, as (station, z) in the host's elevation frame (see [`imprint_point_count`]).
pub(crate) fn imprint_points(cuts: &[cut::Rect], pieces: &[Region], host: &cut::Rect) -> Vec<(f64, f64)> {
    const EPS: f64 = 1e-9;
    let clipped: Vec<cut::Rect> = cuts
        .iter()
        .filter_map(|c| c.normalised().clipped_to(host))
        .collect();

    // The net region's maximal straight boundary edges, as (fixed coordinate, lo, hi, horizontal).
    let mut net: Vec<(f64, f64, f64, bool)> = Vec::new();
    for piece in pieces {
        for lp in piece.loops() {
            for c in lp.curves() {
                if let asterion_geom::Curve::Line(l) = c {
                    let (a, b) = (l.start, l.end);
                    if (a.y - b.y).abs() < EPS {
                        net.push((a.y, a.x.min(b.x), a.x.max(b.x), true));
                    } else if (a.x - b.x).abs() < EPS {
                        net.push((a.x, a.y.min(b.y), a.y.max(b.y), false));
                    }
                }
            }
        }
    }

    let inside = |r: &cut::Rect, q: &cut::Rect| {
        r.u_lo >= q.u_lo - EPS && r.u_hi <= q.u_hi + EPS && r.z_lo >= q.z_lo - EPS && r.z_hi <= q.z_hi + EPS
    };

    let mut points: Vec<(f64, f64)> = Vec::new();
    for (i, r) in clipped.iter().enumerate() {
        for (j, q) in clipped.iter().enumerate() {
            if i == j || !inside(r, q) {
                continue;
            }
            for &(u, z) in &[(r.u_lo, r.z_lo), (r.u_hi, r.z_lo), (r.u_hi, r.z_hi), (r.u_lo, r.z_hi)] {
                // The four container edges: (fixed, lo, hi, horizontal).
                let edges = [
                    (q.z_lo, q.u_lo, q.u_hi, true),
                    (q.z_hi, q.u_lo, q.u_hi, true),
                    (q.u_lo, q.z_lo, q.z_hi, false),
                    (q.u_hi, q.z_lo, q.z_hi, false),
                ];
                for &(fixed, lo, hi, horiz) in &edges {
                    let (along, across) = if horiz { (u, z) } else { (z, u) };
                    // Strictly inside the container's edge: a point at its corner is already a vertex.
                    if (across - fixed).abs() > EPS || along <= lo + EPS || along >= hi - EPS {
                        continue;
                    }
                    // ...and strictly inside a maximal net boundary edge that stays within it.
                    let carried = net.iter().any(|&(f, nlo, nhi, h)| {
                        h == horiz
                            && (f - fixed).abs() < EPS
                            && along > nlo + EPS
                            && along < nhi - EPS
                            && nlo >= lo - EPS
                            && nhi <= hi + EPS
                    });
                    if carried {
                        let p = (u, z);
                        if !points.iter().any(|&(pu, pz)| (pu - p.0).abs() < EPS && (pz - p.1).abs() < EPS) {
                            points.push(p);
                        }
                    }
                }
            }
        }
    }
    points
}

/// Boundary curves of the cut region, summed over every piece: the `C` of the `3C` edge rule.
fn cut_curve_count(pieces: &[Region]) -> usize {
    pieces.iter().map(|p| p.curve_count()).sum()
}

/// A face a cutter leaves in the wall: the line it lies on (horizontal at `z`, or vertical at `u`), the side the
/// material is on (+1 above / right, -1 below / left) and the stretches of that line it still covers.
struct CutterFace {
    horizontal: bool,
    at: f64,
    side: i8,
    spans: Vec<(f64, f64)>,
}

/// `spans` minus the open interval `(lo, hi)`.
fn spans_minus(spans: &[(f64, f64)], lo: f64, hi: f64) -> Vec<(f64, f64)> {
    const EPS: f64 = 1e-9;
    let mut out = Vec::new();
    for &(a, b) in spans {
        if hi <= a + EPS || lo >= b - EPS {
            out.push((a, b));
            continue;
        }
        if lo > a + EPS {
            out.push((a, lo));
        }
        if hi < b - EPS {
            out.push((hi, b));
        }
    }
    out
}

/// The faces the cutters leave in the wall, in the order the cuts are made (Spec W-11b; the reference product adds nothing
/// to the Boolean): every rectangular cutter is subtracted from the wall in turn; the faces of the cutter that lie inside
/// the material at that moment are new faces, later cutters take parts of them away (a face with nothing left is gone, one
/// parted in two stays ONE face), and a new face that is coplanar with a face of an earlier cutter, faces the same way and
/// touches it is merged into it (the merge pass is always on). Cutter faces on the rim planes of the wall are not faces:
/// the wall's own faces take that place.
fn cutter_face_count(host: &HostOutline, cuts: &[cut::Rect]) -> usize {
    const EPS: f64 = 1e-9;
    let mut faces: Vec<CutterFace> = Vec::new();
    let mut earlier: Vec<cut::Rect> = Vec::new();
    for raw in cuts {
        let r = raw.normalised();
        if r.is_empty() {
            continue;
        }
        // 1. the earlier faces lose the part the new cutter swallows (their material side lies inside it)
        for f in faces.iter_mut() {
            let (inside_across, span) = if f.horizontal {
                let across = if f.side > 0 {
                    r.z_lo <= f.at + EPS && f.at < r.z_hi - EPS
                } else {
                    r.z_lo + EPS < f.at && f.at <= r.z_hi + EPS
                };
                (across, (r.u_lo, r.u_hi))
            } else {
                let across = if f.side > 0 {
                    r.u_lo <= f.at + EPS && f.at < r.u_hi - EPS
                } else {
                    r.u_lo + EPS < f.at && f.at <= r.u_hi + EPS
                };
                (across, (r.z_lo, r.z_hi))
            };
            if inside_across {
                f.spans = spans_minus(&f.spans, span.0, span.1);
            }
        }
        faces.retain(|f| !f.spans.is_empty());
        // 2. the new faces: the four sides of the cutter, where the material is
        let in_void = |u: f64, z: f64| {
            earlier.iter().any(|q| q.u_lo + EPS < u && u < q.u_hi - EPS && q.z_lo + EPS < z && z < q.z_hi - EPS)
        };
        // (horizontal, line, side the material is on, start and end of the side along its line)
        let sides: [(bool, f64, i8, f64, f64); 4] = [
            (true, r.z_lo, -1, r.u_lo, r.u_hi),
            (true, r.z_hi, 1, r.u_lo, r.u_hi),
            (false, r.u_lo, -1, r.z_lo, r.z_hi),
            (false, r.u_hi, 1, r.z_lo, r.z_hi),
        ];
        for &(horizontal, at, side, lo, hi) in &sides {
            // the side must lie strictly inside the wall across, and its stretch is clipped to the wall along
            let (across_lo, across_hi, along_lo, along_hi) = if horizontal {
                (host.z_lo, host.z_hi, host.u_lo, host.u_hi)
            } else {
                (host.u_lo, host.u_hi, host.z_lo, host.z_hi)
            };
            if !(at > across_lo + EPS && at < across_hi - EPS) {
                continue;
            }
            let (lo, hi) = (lo.max(along_lo), hi.min(along_hi));
            if hi <= lo + EPS {
                continue;
            }
            // breakpoints along the side: the edges of the earlier cutters
            let mut cuts_at: Vec<f64> = vec![lo, hi];
            for q in &earlier {
                let (a, b) = if horizontal { (q.u_lo, q.u_hi) } else { (q.z_lo, q.z_hi) };
                for v in [a, b] {
                    if v > lo + EPS && v < hi - EPS {
                        cuts_at.push(v);
                    }
                }
            }
            cuts_at.sort_by(|x, y| x.total_cmp(y));
            cuts_at.dedup_by(|x, y| (*x - *y).abs() < EPS);
            let mut spans: Vec<(f64, f64)> = Vec::new();
            for w in cuts_at.windows(2) {
                let mid = 0.5 * (w[0] + w[1]);
                // the material is on the outer side of the cutter: probe just beyond the side
                let probe = at + f64::from(side) * 1e-7;
                let void = if horizontal { in_void(mid, probe) } else { in_void(probe, mid) };
                if void {
                    continue;
                }
                match spans.last_mut() {
                    Some(l) if (l.1 - w[0]).abs() < EPS => l.1 = w[1],
                    _ => spans.push((w[0], w[1])),
                }
            }
            if spans.is_empty() {
                continue;
            }
            // 3. merge with the faces it touches (same plane, same facing), else it is a new face
            let touching: Vec<usize> = faces
                .iter()
                .enumerate()
                .filter(|(_, f)| {
                    f.horizontal == horizontal
                        && f.side == side
                        && (f.at - at).abs() < EPS
                        && f.spans.iter().any(|&(a, b)| spans.iter().any(|&(c, d)| a <= d + EPS && c <= b + EPS))
                })
                .map(|(i, _)| i)
                .collect();
            match touching.split_first() {
                None => faces.push(CutterFace { horizontal, at, side, spans }),
                Some((&first, rest)) => {
                    let mut gathered: Vec<(f64, f64)> = faces[first].spans.clone();
                    gathered.extend(spans);
                    for &i in rest {
                        gathered.extend(faces[i].spans.iter().copied());
                    }
                    faces[first].spans = gathered;
                    for &i in rest.iter().rev() {
                        faces.remove(i);
                    }
                }
            }
        }
        earlier.push(r);
    }
    faces.len()
}

fn face_count_of(outline: &HostOutline, cuts: &[cut::Rect], pieces: &[Region]) -> usize {
    // Spec W-11b (the Boolean of the wall opening step, merge pass always on): the faces of the cut wall are the faces of
    // the wall that survive plus the faces of the cutters (`cutter_face_count`). OBSERVED: the two halves of the base
    // parted by a door are still ONE face (the wall's own base face), the two stretches of a band's bottom parted by a door
    // are one face of the band, and a door head and a window head at the same height are two faces (a stretch of wall lies
    // between them). `asterion_geom::distinct_support_count_all` (the earlier fit) merged the latter when the cuts were
    // disjoint, the reference product does not.
    //
    // The count: front and back, plus each of the wall's four rim faces (bottom, top, start end, finish
    // end) that the net region still touches, plus the cutter faces.
    const EPS: f64 = 1e-9;
    let mut rim = [false; 4]; // bottom, top, start end, finish end
    for piece in pieces {
        for lp in piece.loops() {
            for c in lp.curves() {
                let asterion_geom::Curve::Line(l) = c else { continue };
                let (a, b) = (l.start, l.end);
                if (a.y - b.y).abs() < EPS && (a.x - b.x).abs() >= EPS {
                    // material is on the left of the direction
                    let up = b.x > a.x;
                    if (a.y - outline.z_lo).abs() < EPS && up {
                        rim[0] = true;
                    } else if (a.y - outline.z_hi).abs() < EPS && !up {
                        rim[1] = true;
                    }
                } else if (a.x - b.x).abs() < EPS && (a.y - b.y).abs() >= EPS {
                    let right = b.y < a.y;
                    if (a.x - outline.u_lo).abs() < EPS && right {
                        rim[2] = true;
                    } else if (a.x - outline.u_hi).abs() < EPS && !right {
                        rim[3] = true;
                    }
                }
            }
        }
    }
    2 + rim.iter().filter(|r| **r).count() + cutter_face_count(outline, cuts)
}

pub fn host_cuts(model: &BimModel, host: ElementId) -> Vec<(ElementId, InsertKind, CutRect)> {
    let openings = model
        .wall_openings
        .values()
        .filter(|o| o.host == host)
        .map(|o| (o.id, InsertKind::RectOpening, o.rect()));
    let instances = model
        .hosted_instances
        .values()
        .filter(|i| i.host == host)
        .filter_map(|i| hosted_cut_rect(model, i).ok().map(|r| (i.id, InsertKind::HostedInstance, r)));
    let mut v: Vec<(ElementId, InsertKind, CutRect)> = openings.chain(instances).collect();
    v.sort_by_key(|(id, _, _)| *id);
    v
}

/// The host and unclipped cut rectangle of any insert element, whichever kind it is.
///
/// `None` when `element` is not an insert at all. This is the lookup the commit-time fit checks
/// need in order to treat the two element kinds identically (section 7c).
pub fn cut_of(model: &BimModel, element: ElementId) -> Option<(ElementId, InsertKind, CutRect)> {
    if let Some(o) = model.wall_openings.get(&element) {
        return Some((o.host, InsertKind::RectOpening, o.rect()));
    }
    let i = model.hosted_instances.get(&element)?;
    let r = hosted_cut_rect(model, i).ok()?;
    Some((i.host, InsertKind::HostedInstance, r))
}

/// Regenerated state of one hosted door/window instance (all SI units). Nothing here is stored.
#[derive(Clone, Debug, PartialEq)]
pub struct HostedInstanceState {
    pub kind: crate::model::HostedKind,
    /// Type width, i.e. the width of the cut (m).
    pub width: f64,
    /// Type height, i.e. the height of the cut (m).
    pub height: f64,
    /// Stored sill height above the instance's level (m).
    pub sill_height: f64,
    /// `sill_height + type.height` - DERIVED, never stored.
    ///
    /// ASSUMED (section 3c): "`Head Height = Sill Height + type.Height`, maintained by the reference in *both* directions".
    /// ASTERION stores the sill only, so the two can never disagree; writing the head is re-expressed as writing the sill
    /// ([`crate::ops::set_head_height`]).
    pub head_height: f64,
    /// Absolute elevation of the sill and of the head (m), i.e. `level.z + sill` and `+ height`.
    pub sill_z: f64,
    pub head_z: f64,
    /// The instance's own cut rectangle in host coordinates, before clipping.
    pub rect: CutRect,
    /// Area of that rectangle (m2).
    pub cut_area: f64,
    /// Area actually removed from the host - the rectangle clipped to the host outline (m2).
    pub clipped_cut_area: f64,
    /// Depth of the cut: the **full** host thickness.
    ///
    /// ASSUMED (section 7b: "full wall thickness"). The type's `inset` is recorded on the type and
    /// deliberately does **not** move the cut: section 1c's through-thickness placement and
    /// section 7f's scope list both put it outside v0.1.
    pub thickness: f64,
    /// `clipped_cut_area * thickness` (m3): what the host's Volume loses.
    pub cut_volume: f64,
    /// Unit facing vector in plan.
    ///
    /// ASSUMED (section 1d prediction 1): for a freshly placed, unflipped instance
    /// `FacingOrientation == Wall.Orientation` - the family faces the wall's exterior side - and
    /// the flag negates it. The named falsifier is that it is the negation.
    pub facing_orientation: Vec2,
    /// Unit hand vector in plan.
    ///
    /// ASSUMED (section 1d prediction 2): the hand axis is horizontal and perpendicular to facing, i.e. the unit tangent of
    /// the host's location curve, negated by the flag.
    pub hand_orientation: Vec2,
    /// `facing_flipped ^ hand_flipped` - DERIVED (section 1d prediction 3).
    pub mirrored: bool,
    pub can_flip_facing: bool,
    pub can_flip_hand: bool,
    /// Where the reference's `LocationPoint.Point` is predicted to sit.
    ///
    /// ASSUMED (section 1c): "on the wall's location curve at station `HostParameter`, with `z` =
    /// the host level elevation + the family's own base (i.e. the sill/level plane), **not** at the
    /// centre of the opening".
    ///
    /// OBSERVED: it is the **sill elevation** - level elevation + sill height. ASTERION used the bare level elevation,
    /// which is right only while the sill is 0.
    pub location_point: Vec3,
    /// Rotation about the insertion point (radians).
    ///
    /// ASSUMED: the instance's local X axis is its hand axis, so the rotation is the angle from world +X to
    /// [`HostedInstanceState::hand_orientation`], in `(-pi, pi]`.
    pub rotation: f64,
    /// Whether the cut stays inside the host horizontally / vertically. False means the
    /// transaction that produced this state is failing at commit (see `ops::check_inserts`).
    pub inside_host_u: bool,
    pub inside_host_z: bool,
    /// The subtracted prism itself, or `None` when the cut misses the host.
    pub solid: Option<Prism>,
}

/// For an instance on an ARC wall, the unit tangent of the path at the host parameter (the X axis of the instance) and the
/// origin: the path point moved along the radius by `s = sqrt(Rin^2 - w^2 / 4) - Rin` (`Rin = R - t / 2`; with the facing
/// flipped `Rout = R + t / 2`), `w` the width of the symbol cut along X. Unchanged (the path point) when a square root is
/// not positive. The station is the arc length from the start of the path (ENGINE CHOICE: the reference product stores the
/// host parameter; the engine's station is the length along the location line). `None` for a straight host.
pub(crate) fn arc_insert_frame(host: &Wall, i: &HostedInstance, s: &WallState, w: f64) -> Option<(Vec2, Vec2)> {
    let arc = host.through.and_then(|t| arc_from_points(host.start, host.end, t))?;
    let dir = if arc.sweep >= 0.0 { 1.0 } else { -1.0 };
    let angle = arc.start_angle + dir * (i.host_station / arc.radius);
    let radial = Vec2::new(angle.cos(), angle.sin());
    let tangent = radial.perp_ccw() * dir;
    let path_pt = arc.center + radial * arc.radius;
    let (r_in, r_out) = (arc.radius - s.width / 2.0, arc.radius + s.width / 2.0);
    let (a, b) = (r_in * r_in - w * w / 4.0, r_out * r_out - w * w / 4.0);
    let shift = if a > 0.0 && b > 0.0 {
        if i.facing_flipped {
            b.sqrt() - r_out
        } else {
            a.sqrt() - r_in
        }
    } else {
        0.0
    };
    Some((tangent, path_pt + radial * shift))
}

/// Derived state of a hosted instance: a pure function of the model.
pub fn hosted_instance_state(model: &BimModel, i: &HostedInstance) -> Result<HostedInstanceState, Failure> {
    let t = model.hosted_type(i.type_id)?.clone();
    let level = model.level(i.level)?.elevation;
    let host = model.wall(i.host)?;
    let s = wall_state(model, host)?;
    let outline = host_outline(model, host)?;
    let slack = crate::spec::insert_fit_tolerance();
    let rect = hosted_cut_rect(model, i)?;
    let clipped = outline.clip_rect(rect);
    let clipped_cut_area = clipped.map(|c| c.area()).unwrap_or(0.0);
    let thickness = s.width;
    let half = s.orientation * (thickness / 2.0);
    let at = |u: f64| host.start + s.direction * u;
    let solid = clipped.and_then(|c| {
        let (a, b) = (at(c.u_lo), at(c.u_hi));
        (thickness > 0.0).then(|| Prism::new(vec![a + half, b + half, b - half, a - half], c.z_lo, c.z_hi))
    });
    // OBSERVED (step `host-wall-flipped`): flipping the HOST does **not** change the instance's facing. The wall's
    // orientation went [0, -1] while the door's facing stayed [0, 1]. So facing is not the host's orientation negated by
    // the flag - it is the host's orientation *ignoring the host's own flip*, negated by the instance's flag.
    //
    // ASTERION used `s.orientation`, which already carries the host's flip, so a wall flip dragged the instance round with
    // it. `direction.perp_ccw` is the host's unflipped normal by construction., (3): on an ARC host the axes are the
    // tangent frame of the path at the host parameter and the origin leaves the path along the radius
    let arc_frame = arc_insert_frame(host, i, &s, rect.u_hi - rect.u_lo);
    let (tangent, location_xy) = match arc_frame {
        Some((t, p)) => (t, p),
        None => (s.direction, at(i.host_station)),
    };
    let base_facing = tangent.perp_ccw();
    let facing = if i.facing_flipped { -base_facing } else { base_facing };
    let hand = if i.hand_flipped { -tangent } else { tangent };
    Ok(HostedInstanceState {
        kind: t.kind,
        width: i.width_in(&t),
        height: i.height_in(&t),
        sill_height: i.sill_height,
        head_height: i.sill_height + i.height_in(&t),
        sill_z: level + i.sill_height,
        head_z: level + i.sill_height + i.height_in(&t),
        rect,
        cut_area: rect.area(),
        clipped_cut_area,
        thickness,
        cut_volume: clipped_cut_area * thickness,
        facing_orientation: facing,
        hand_orientation: hand,
        mirrored: i.mirrored(),
        can_flip_facing: HostedInstance::CAN_FLIP_FACING,
        can_flip_hand: HostedInstance::CAN_FLIP_HAND,
        location_point: Vec3::from_xy(location_xy, level + i.sill_height),
        // The sign of a floating zero would otherwise make a reversed hand axis report -pi
        // instead of +pi; the convention stated above is the half-open range (-pi, pi].
        rotation: (if hand.y == 0.0 { 0.0 } else { hand.y }).atan2(hand.x),
        inside_host_u: outline.contains_rect_u(rect, slack),
        inside_host_z: outline.contains_rect_z(rect, slack),
        solid,
    })
}

#[cfg(test)]
mod instance_size_tests {
    //! ENGINE CHOICE: an instance's own size replaces its type's in the cut.
    use super::*;
    use crate::{ops, CompoundStructure, Layer, LayerFunction};

    #[test]
    fn an_instance_size_override_changes_its_cut_and_the_hosts_net_quantities_only() {
        let mut doc = crate::new_document();
        let (wall, a, b) = doc
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L", 0.0)?;
                let cs = CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?;
                let wt = ops::create_wall_type(m, cx, "W", cs)?;
                let v = Vec2::new;
                let wall = ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l, start: v(0.0, 0.0), end: v(10.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false })?;
                let dt = ops::create_hosted_type(m, cx, "D", crate::model::HostedKind::Door, 1.0, 2.0, 0.05, 1.0, 2.0, 0.0, 0.0)?;
                let a = ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: dt, host: wall, level: l, host_station: 2.0, sill_height: None })?;
                let b = ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: dt, host: wall, level: l, host_station: 6.0, sill_height: None })?;
                Ok((wall, a, b))
            })
            .value
            .expect("fixture");
        let area0 = wall_state(doc.model(), &doc.model().walls[&wall]).unwrap().area;
        // an own cut loop (off centre: 0.2 m left, wider on the right) replaces the rectangle
        let loop_ = crate::model::InstanceCutLoop { u_min: -0.7, u_max: 1.3, z_min: -0.1, z_max: 2.5 };
        let out = doc.transact("loop", false, |m, cx| ops::set_hosted_instance_cut_loop(m, cx, b, Some(loop_)));
        assert!(out.committed(), "{:?}", out.failures);
        let r = hosted_cut_rect(doc.model(), &doc.model().hosted_instances[&b]).unwrap();
        assert_eq!((r.u_lo, r.u_hi), (5.3, 7.3));
        assert!((r.z_hi - r.z_lo - 2.6).abs() < 1e-12);
        let a_loop = wall_state(doc.model(), &doc.model().walls[&wall]).unwrap().area;
        // 2.0 x 2.4 inside the wall from z 0 (its bottom edge is below the base, clipped): net loses 2.0 x 2.5
        assert!((area0 - a_loop - (2.0 * 2.5 - 2.0)).abs() < 1e-9, "{area0} {a_loop}");
        let bad = crate::model::InstanceCutLoop { u_min: 1.0, u_max: 1.0, z_min: 0.0, z_max: 1.0 };
        assert!(doc.transact("bad", false, |m, cx| ops::set_hosted_instance_cut_loop(m, cx, b, Some(bad))).value.is_none());
        assert!(doc.transact("reset", false, |m, cx| ops::set_hosted_instance_cut_loop(m, cx, b, None)).committed());
        assert!((wall_state(doc.model(), &doc.model().walls[&wall]).unwrap().area - area0).abs() < 1e-9);
        let out = doc.transact("size", false, |m, cx| ops::set_hosted_instance_size(m, cx, a, Some(1.5), Some(2.4)));
        assert!(out.committed(), "{:?}", out.failures);
        let m = doc.model();
        let sa = hosted_instance_state(m, &m.hosted_instances[&a]).unwrap();
        let sb = hosted_instance_state(m, &m.hosted_instances[&b]).unwrap();
        assert_eq!((sa.width, sa.height, sa.head_height), (1.5, 2.4, 2.4));
        assert_eq!((sb.width, sb.height), (1.0, 2.0), "the type and the other instance are untouched");
        assert_eq!((sa.rect.u_lo, sa.rect.u_hi), (1.25, 2.75));
        // the host lost (1.5 x 2.4 + 1.0 x 2.0) m2 instead of 2 x 2.0
        let area1 = wall_state(m, &m.walls[&wall]).unwrap().area;
        assert!(((area0 - area1) - (1.5 * 2.4 - 2.0)).abs() < 1e-9, "{area0} {area1}");
        // back to the type's
        let out = doc.transact("reset", false, |m, cx| ops::set_hosted_instance_size(m, cx, a, None, None));
        assert!(out.committed());
        assert!((wall_state(doc.model(), &doc.model().walls[&wall]).unwrap().area - area0).abs() < 1e-9);
        // bad sizes and non-instances are refused and change nothing
        for (w, h) in [(Some(0.0), None), (None, Some(-1.0)), (Some(f64::NAN), None)] {
            let out = doc.transact("bad", false, |m, cx| ops::set_hosted_instance_size(m, cx, a, w, h));
            assert!(out.value.is_none());
        }
        let out = doc.transact("bad", false, |m, cx| ops::set_hosted_instance_size(m, cx, wall, Some(1.0), None));
        assert!(out.value.is_none());
    }
}

#[cfg(test)]
mod hosted_plan_frame_tests {
    //! The frame a plan symbol is drawn from equals the host's regenerated state, joins included.
    use super::*;
    use crate::{ops, CompoundStructure, Layer, LayerFunction};

    #[test]
    fn the_cheap_frame_equals_the_regenerated_host_and_instance_states() {
        let mut doc = crate::new_document();
        let ids = doc
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L", 0.0)?;
                let top = ops::create_level(m, cx, "T", 3.0)?;
                let cs = CompoundStructure::new(vec![Layer { width: 0.25, function: LayerFunction::Structure }], 0, 0)?;
                let wt = ops::create_wall_type(m, cx, "W", cs)?;
                let v = Vec2::new;
                let new = |a: Vec2, b: Vec2, flip: bool| ops::NewWall { wall_type: wt, level: l, start: a, end: b, height: 3.0, base_offset: 0.0, flip, structural: false };
                // a flipped host, joined at its end to a perpendicular wall
                let host = ops::create_wall(m, cx, new(v(0.0, 0.0), v(6.0, 0.0), true))?;
                ops::create_wall(m, cx, new(v(6.0, 0.0), v(6.0, 4.0), false))?;
                ops::set_top_constraint(m, cx, host, Some(top), Some(0.0))?;
                let dt = ops::create_hosted_type(m, cx, "D", crate::model::HostedKind::Door, 0.9, 2.1, 0.05, 0.9, 2.1, 0.0, 0.0)?;
                let wtp = ops::create_hosted_type(m, cx, "Wn", crate::model::HostedKind::Window, 1.2, 1.0, 0.05, 1.2, 1.0, 0.9, 0.0)?;
                let d = ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: dt, host, level: l, host_station: 2.0, sill_height: None })?;
                let w = ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: wtp, host, level: l, host_station: 4.0, sill_height: None })?;
                Ok((host, d, w))
            })
            .value
            .expect("built");
        let m = doc.model();
        let host = m.walls.get(&ids.0).unwrap();
        let ws = wall_state(m, host).unwrap();
        for i in [ids.1, ids.2] {
            let inst = m.hosted_instances.get(&i).unwrap();
            let st = hosted_instance_state(m, inst).unwrap();
            let f = hosted_plan_frame(m, inst).expect("straight host");
            assert_eq!(f.kind, st.kind);
            assert_eq!((f.u_lo, f.u_hi), (st.rect.u_lo, st.rect.u_hi));
            assert_eq!((f.start, f.direction, f.orientation, f.width), (host.start, ws.direction, ws.orientation, ws.width));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterion_core::ElementId;

    fn opening(u_min: f64, u_max: f64, z_min: f64, z_max: f64) -> WallOpening {
        WallOpening { id: ElementId(1), host: ElementId(2), u_min, u_max, z_min, z_max }
    }

    fn outline() -> HostOutline {
        HostOutline { u_lo: 0.0, u_hi: 6.0, z_lo: 0.0, z_hi: 3.0 }
    }

    /// ASSUMED: the subtracted quantity is the cut rectangle **clipped to the wall outline**, so an opening hanging off the
    /// end could only ever remove the overlapping part.
    #[test]
    fn a_cut_is_clipped_to_the_host_outline() {
        let o = outline();
        assert!((o.clip(&opening(1.0, 2.2, 0.9, 2.1)).unwrap().area() - 1.44).abs() < 1e-12);
        // hanging off the end: only 0.5 x 1.0 of a 1.0 x 1.0 rectangle is inside
        assert!((o.clip(&opening(5.5, 6.5, 0.5, 1.5)).unwrap().area() - 0.5).abs() < 1e-12);
        // wider than the wall is long: clipped to the full outline band
        assert!((o.clip(&opening(-1.0, 7.0, 0.5, 1.5)).unwrap().area() - 6.0).abs() < 1e-12);
        // entirely outside: no area at all
        assert!(o.clip(&opening(7.0, 8.0, 0.5, 1.5)).is_none());
        assert!(o.clip(&opening(1.0, 2.0, 3.5, 4.0)).is_none());
        // touching an edge contributes nothing
        assert!(o.clip(&opening(6.0, 7.0, 0.5, 1.5)).is_none());
    }

    /// Containment is tested with a slack, because "a cut that exactly touches the host's edges"
    /// must commit (section 7c).
    #[test]
    fn containment_allows_a_cut_flush_with_the_host_edges() {
        let o = outline();
        let slack = crate::spec::insert_fit_tolerance();
        let flush = opening(0.0, 6.0, 0.0, 3.0);
        assert!(o.contains_u(&flush, slack) && o.contains_z(&flush, slack));
        assert!(!o.contains_u(&opening(-0.5, 1.0, 0.0, 1.0), slack));
        assert!(!o.contains_u(&opening(5.0, 6.5, 0.0, 1.0), slack));
        assert!(!o.contains_z(&opening(1.0, 2.0, 2.5, 3.5), slack));
        assert!(!o.contains_z(&opening(1.0, 2.0, -0.5, 1.0), slack));
    }

    /// Two cuts that only share an edge are not an overlap.
    #[test]
    fn touching_cuts_do_not_overlap() {
        let a = CutRect { u_lo: 1.0, u_hi: 2.0, z_lo: 0.0, z_hi: 2.0 };
        let b = CutRect { u_lo: 2.0, u_hi: 3.0, z_lo: 0.0, z_hi: 2.0 };
        let c = CutRect { u_lo: 1.5, u_hi: 3.0, z_lo: 1.0, z_hi: 2.0 };
        assert_eq!(a.overlap_area(&b), 0.0);
        assert!((a.overlap_area(&c) - 0.5).abs() < 1e-12);
        assert_eq!(a.overlap_area(&c), c.overlap_area(&a));
    }

    /// Claims the host's surface area GROWS when an opening is cut, and offers that sign as the cheap check that the cut is
    /// a real subtraction. Running the formula shows it only grows when the host is thick relative to the opening: a cut
    /// removes `2*w*h` of face and adds `2*(w+h)*t` of reveal, so the threshold is `t > (w*h)/(w+h)`.
    ///
    /// This test states both sides of that threshold as arithmetic, so the measurement is
    /// interpreted against the formula and not against the prose.
    #[test]
    fn an_opening_grows_the_surface_only_on_a_thick_host() {
        let (w, h) = (1.2_f64, 1.2_f64);
        let threshold = (w * h) / (w + h); // 0.6 m
        let face_lost = 2.0 * w * h; // 2.88 m2 off the two large faces
        let reveal_gained = |t: f64| 2.0 * (w + h) * t;

        // A 0.20 m wall is far below the threshold, so the surface SHRINKS.
        let thin = 0.20;
        assert!(thin < threshold);
        let delta_thin = reveal_gained(thin) - face_lost;
        assert!(delta_thin < 0.0, "a 0.20 m host must lose surface, got {delta_thin}");
        // 39.600 unperforated -> 37.680 with the cut, which is what the replay emits.
        let unperforated = 2.0 * (6.0 * 3.0) + 2.0 * (thin * 3.0) + 2.0 * (6.0 * thin);
        assert!((unperforated - 39.6).abs() < 1e-12);
        assert!((unperforated + delta_thin - 37.68).abs() < 1e-12);

        // A 1.00 m host is above the threshold, so the same cut GROWS the surface.
        let thick = 1.00;
        assert!(thick > threshold);
        assert!(reveal_gained(thick) - face_lost > 0.0);
    }
}

#[cfg(test)]
mod slab_tests {
    use super::*;
    use crate::model::{RoofBearing, SlabRole};
    use crate::{ops, BimDocument};
    use asterion_core::txn::TxnContext;

    /// Run one transaction and flatten the outcome: `Ok` only when it COMMITTED, otherwise the
    /// first error - the rejection channel (`Err`) and the commit-time channel (`report`) both
    /// surface as a `Failure`, which is exactly what these tests need to tell apart by code.
    fn tx<R>(
        doc: &mut BimDocument,
        name: &str,
        f: impl FnOnce(&mut BimModel, &mut TxnContext<'_>) -> Result<R, Failure>,
    ) -> Result<R, Failure> {
        let out = doc.transact(name, false, f);
        match out.value {
            Some(v) if out.committed() => Ok(v),
            _ => Err(out
                .failures
                .into_iter()
                .next()
                .expect("a rolled-back transaction carries its failure")),
        }
    }

    fn near(a: f64, b: f64, what: &str) {
        assert!((a - b).abs() < 1e-9, "{what}: got {a}, want {b}");
    }

    struct Fx {
        doc: BimDocument,
        l0: ElementId,
        ct: ElementId,
        rt: ElementId,
    }

    fn fixture() -> Fx {
        let mut doc = crate::new_document();
        let (l0, ct, rt) = tx(&mut doc, "setup", |m, cx| {
                let l0 = ops::create_level(m, cx, "AST-L0", 0.0)?;
                let ct = ops::create_slab_type(m, cx, "AST-CEIL-200", 0.2, SlabRole::Ceiling)?;
                let rt = ops::create_slab_type(m, cx, "AST-ROOF-300", 0.3, SlabRole::Roof)?;
                Ok((l0, ct, rt))
            })
            .expect("setup commits");
        Fx { doc, l0, ct, rt }
    }

    fn rect(w: f64, d: f64) -> Vec<Vec2> {
        vec![Vec2::new(0.0, 0.0), Vec2::new(w, 0.0), Vec2::new(w, d), Vec2::new(0.0, d)]
    }

    fn state(doc: &BimDocument, id: ElementId) -> SlabState {
        let m = doc.model();
        slab_state(m, m.slab(id).unwrap()).unwrap()
    }

    /// ASSUMED and the single field round 10 uses to decide it: a ceiling's BOTTOM is pinned to the level, so thickening
    /// the type must leave `bbox.min.z` where it is and push the top up. If the reference instead drops the bottom by 0.150
    /// the ceiling is top-anchored like a floor and [`SlabAnchor`] loses its reason to exist.
    #[test]
    fn a_ceiling_hangs_upwards_from_its_level_unlike_a_floor() {
        let mut fx = fixture();
        let (ct, l0) = (fx.ct, fx.l0);
        let c1 = tx(&mut fx.doc, "ceiling", |m, cx| ops::create_slab(m, cx, ct, l0, rect(6.0, 4.0), 2.7, false))
            .unwrap();

        let before = state(&fx.doc, c1);
        near(before.bottom_z, 2.7, "the ceiling's bottom sits ON the level plane");
        near(before.top_z, 2.9, "and the body rises by the type thickness");
        near(before.area, 24.0, "a 6.000 x 4.000 sketch");
        near(before.volume, 4.8, "24.000 x 0.200");

        tx(&mut fx.doc, "thicken", |m, cx| ops::set_slab_type_thickness(m, cx, ct, 0.35))
            .unwrap();
        let after = state(&fx.doc, c1);
        near(after.bottom_z, 2.7, "THE discriminating field: the bottom must not move");
        near(after.top_z, 3.05, "the extra 0.150 goes upwards");
        near(after.area, 24.0, "Area is the sketch and does not care about thickness");
    }

    /// A ceiling with no compound structure is legal (DOCUMENTED, section 1a) - the one case where
    /// a zero thickness is not a rejection. It still has a sketch, so Area and Perimeter survive
    /// while the body does not.
    #[test]
    fn a_zero_thickness_ceiling_is_legal_and_keeps_its_sketch() {
        let mut fx = fixture();
        let l0 = fx.l0;
        let (ct0, c) = tx(&mut fx.doc, "zero", |m, cx| {
                let ct0 = ops::create_slab_type(m, cx, "AST-CEIL-0", 0.0, SlabRole::Ceiling)?;
                let c = ops::create_slab(m, cx, ct0, l0, rect(6.0, 4.0), 2.7, false)?;
                Ok((ct0, c))
            })
            .expect("a zero-thickness ceiling type commits");
        let s = state(&fx.doc, c);
        assert!(s.solid.is_none(), "no compound structure means no body");
        near(s.area, 24.0, "the sketch is still 24.000 m2");
        near(s.perimeter, 20.0, "and still 20.000 m round");
        near(s.volume, 0.0, "but it encloses nothing");
        let _ = ct0;

        // The same thickness on a FLOOR or a ROOF is a rejection, and that asymmetry is the whole
        // reason `role` lives on the type as well as on the instance.
        let mut fx2 = fixture();
        for role in [SlabRole::Floor, SlabRole::Roof] {
            let e = tx(&mut fx2.doc, "zero", |m, cx| ops::create_slab_type(m, cx, "AST-ZERO", 0.0, role))
                .expect_err("zero thickness must be rejected for this role");
            assert_eq!(e.code, codes::INVALID_THICKNESS);
        }
    }

    /// A 6.000 x 4.000 footprint roof, 0.300 thick, sloped 0.500 on edge 0:
    ///
    /// | field | model A projected | model C inclined (implemented) |
    /// |---|---|---|
    /// | Area | 24.000 | **26.8328157300** |
    /// | Volume | 7.200 | **8.0498447190** |
    /// | Perimeter | 20.000 | **20.9442719100** |
    ///
    /// `roofs.r1.params.area` at step `slope-0.5-edge-0` separates them with no ambiguity at all.
    /// Model C is implemented because the reference's own `area_ratio` of sqrt(1 + 0.25) =
    /// 1.1180339887 is what a roofer bills for, but it is ASSUMED, not measured.
    #[test]
    fn the_sloped_roof_prediction_round_ten_will_judge() {
        let mut fx = fixture();
        let (rt, l0) = (fx.rt, fx.l0);
        let r1 = tx(&mut fx.doc, "roof", |m, cx| ops::create_slab(m, cx, rt, l0, rect(6.0, 4.0), 0.0, false))
            .unwrap();

        // Flat until an edge defines a slope. A roof grows UP from its base (OBSERVED).
        let flat = state(&fx.doc, r1);
        near(flat.area, 24.0, "a flat roof is just a slab");
        near(flat.volume, 7.2, "24.000 x 0.300");
        near(flat.bottom_z, 0.0, "the body starts on the base");
        near(flat.top_z, 0.3, "and grows up by the thickness");
        assert!(flat.slope.is_none());

        tx(&mut fx.doc, "slope", |m, cx| ops::set_slab_segment_slope(m, cx, r1, 0, true, 0.5))
            .unwrap();
        let s = state(&fx.doc, r1);
        assert_eq!(s.slope, Some(0.5));
        assert!(s.sloped.is_some() && !s.multi_plane);
        near(s.plan_area, 24.0, "the projection never changes");
        near(s.area, 26.832815730000003, "model C: 24.000 x sqrt(1.25)");
        near(s.volume, 8.049844719000002, "Area x the perpendicular thickness");
        // OBSERVED (`slope-0.5-edge-0`): the underside sits ON the base and the body grows up from it, so the top is the
        // rise plus the perpendicular thickness.
        near(s.bottom_z, 0.0, "the underside holds the eave at the base");
        near(s.top_z, 2.0 + 0.3 * 1.25_f64.sqrt(), "ridge 2.000 + perpendicular 0.300");
        // A roof reports no Perimeter (OBSERVED: `params.perimeter` is null); the plan length stays
        // available for the engine's own use.
        near(s.perimeter, 20.0, "plan perimeter");
    }

    /// The slope is the COMMON value of the slope-defining segments, and `None` when they disagree
    /// (ASSUMED, section 8e). Returning the first one instead would be a guess round 10 could not
    /// tell apart from a measurement - a multi-plane roof genuinely has no single reported slope.
    #[test]
    fn two_disagreeing_slopes_report_no_slope_at_all() {
        let mut fx = fixture();
        let (rt, l0) = (fx.rt, fx.l0);
        let r1 = tx(&mut fx.doc, "roof", |m, cx| ops::create_slab(m, cx, rt, l0, rect(6.0, 4.0), 0.0, false))
            .unwrap();
        tx(&mut fx.doc, "slopes", |m, cx| {
                ops::set_slab_segment_slope(m, cx, r1, 0, true, 0.5)?;
                ops::set_slab_segment_slope(m, cx, r1, 2, true, 0.5)
            })
            .unwrap();
        let both = state(&fx.doc, r1);
        assert_eq!(both.slope, Some(0.5), "two edges agreeing still report their value");
        assert!(both.multi_plane, "but the body is a gable this slice cannot build, and it says so");

        tx(&mut fx.doc, "disagree", |m, cx| ops::set_slab_segment_slope(m, cx, r1, 2, true, 0.25))
            .unwrap();
        assert_eq!(state(&fx.doc, r1).slope, None, "disagreeing definers report nothing");
    }

    /// The three channels, each measured in round 10, and the one assertion this test used to make
    /// that the measurement destroyed.
    ///
    /// Raised at the CALL: a nonexistent segment, a non-finite slope, a role without slope.
    /// Posted at COMMIT (`not_committed`, the transaction rolls back): a slope the roof cannot be
    /// built with and a cut-off at or below the underside. The first version of this test asserted
    /// that both of THOSE were accepted, "because round 9 deleted three invented codes" - an
    /// extrapolation from a different element's behaviour. The reference refused both. Every rule
    /// gets measured, and a pattern from round 9 is not a measurement of round 10.
    #[test]
    fn slope_rejections_are_raised_or_posted_exactly_as_measured() {
        let mut fx = fixture();
        let (rt, l0, ct) = (fx.rt, fx.l0, fx.ct);
        let (r1, c1) = tx(&mut fx.doc, "roof", |m, cx| {
                let r1 = ops::create_slab(m, cx, rt, l0, rect(6.0, 4.0), 0.0, false)?;
                let c1 = ops::create_slab(m, cx, ct, l0, rect(6.0, 4.0), 2.7, false)?;
                Ok((r1, c1))
            })
            .unwrap();

        let bad_index = tx(&mut fx.doc, "bad index", |m, cx| ops::set_slab_segment_slope(m, cx, r1, 9, true, 0.5))
            .expect_err("a nonexistent segment is rejected");
        assert_eq!(bad_index.code, codes::SLAB_SEGMENT_OUT_OF_RANGE);

        let not_finite = tx(&mut fx.doc, "nan", |m, cx| ops::set_slab_segment_slope(m, cx, r1, 0, true, f64::NAN))
            .expect_err("a non-finite slope is rejected");
        assert_eq!(not_finite.code, codes::SLAB_SLOPE_NOT_FINITE);

        let wrong_role = tx(&mut fx.doc, "ceiling slope", |m, cx| ops::set_slab_segment_slope(m, cx, c1, 0, true, 0.5))
            .expect_err("a ceiling has no slope");
        assert_eq!(wrong_role.code, codes::SLAB_ROLE_HAS_NO_SLOPE);

        // OBSERVED (step 23): a slope of a million is POSTED at commit and the whole transaction rolls back, leaving the
        // previous slope in place.
        tx(&mut fx.doc, "ok", |m, cx| ops::set_slab_segment_slope(m, cx, r1, 0, true, 0.5)).unwrap();
        let huge = tx(&mut fx.doc, "absurd", |m, cx| ops::set_slab_segment_slope(m, cx, r1, 0, true, 1.0e6))
            .expect_err("a slope of a million is refused at commit");
        assert_eq!(huge.code, codes::SLAB_SLOPE_DEGENERATE);
        assert_eq!(state(&fx.doc, r1).slope, Some(0.5), "and the transaction rolled back");

        let l2 = tx(&mut fx.doc, "l2", |m, cx| ops::create_level(m, cx, "AST-L2", 6.0)).unwrap();
        let below = tx(&mut fx.doc, "cutoff", |m, cx| ops::set_slab_cutoff(m, cx, r1, Some((l2, -7.0))))
            .expect_err("a cut-off below the underside is refused at commit");
        assert_eq!(below.code, codes::ROOF_CUTOFF_BELOW_BASE);
        assert!(fx.doc.model().slab(r1).unwrap().cutoff.is_none(), "rolled back");

        // A 2 mm sketch IS accepted (OBSERVED for a ceiling: Area 0.0000, Perimeter 0.008).
        tx(&mut fx.doc, "tiny", |m, cx| {
                ops::create_slab(m, cx, ct, l0, rect(0.002, 0.002), 0.0, false)
            })
            .expect("a 2 mm sketch is accepted");

        // A bow-tie is raised at the CALL (OBSERVED: ArgumentException for a ceiling).
        let bow = tx(&mut fx.doc, "bowtie", |m, cx| {
                ops::create_slab(
                    m,
                    cx,
                    ct,
                    l0,
                    vec![Vec2::new(20.0, 0.0), Vec2::new(26.0, 0.0), Vec2::new(20.0, 4.0), Vec2::new(26.0, 4.0)],
                    0.0,
                    false,
                )
            })
            .expect_err("a crossing boundary is refused");
        // Spec C-02: a self hit is a pair, the same failure as two loops that cross.
        assert_eq!(bow.code, codes::SLAB_LOOPS_INTERSECT);

        // And bearing is a roof-only flag.
        tx(&mut fx.doc, "truss", |m, cx| ops::set_slab_bearing(m, cx, r1, RoofBearing::Truss)).unwrap();
        assert_eq!(fx.doc.model().slab(r1).unwrap().bearing, RoofBearing::Truss);
        assert_eq!(
            tx(&mut fx.doc, "ceiling truss", |m, cx| ops::set_slab_bearing(m, cx, c1, RoofBearing::Truss))
                .expect_err("a ceiling has no bearing")
                .code,
            codes::SLAB_ROLE_HAS_NO_SLOPE
        );
    }

    /// Spec R-05: deleting a roof's cut-off level keeps the roof and rewrites its up-to reference to the roof's OWN base
    /// level, whatever level lies below the deleted one, and measures the cut-off from that level: `offset = z -
    /// elevation(base level)` with `z` = top of the roof's solid before the rewrite minus the total layer width of its type
    /// (branch 2 of the rule; the engine cannot evaluate branch 1).
    ///
    /// Three roofs share the deleted cut-off level L2 and have different base levels, thicknesses
    /// and cuts, so no value is a sample: the expected offsets are computed from the formula.
    /// * A: base level L1 (5.0), thickness 0.3, slope 0.5 over 4 m, cut 6.5 clips the roof:
    ///   top 6.5, z = 6.5 - 0.3 = 6.2, offset 1.2 (not 1.5, the previous cut above the base).
    /// * B: base level L0 (0.0), thickness 0.45, cut 9.0 above the roof: the engine's top is the
    ///   rise of the underside (0.5 x 4 = 2.0) plus the thickness measured across the slope
    ///   (0.45 x sqrt(1.25)), z = top - 0.45 (the previous cut was 9.0 above the base).
    /// * C: the round 10 configuration (base 3.0, base offset 0.5, thickness 0.5, cut 4.5): z = 4.5 - 0.5 = 4.0, offset
    ///   1.0, which is what the reference product returned.
    #[test]
    fn deleting_the_cutoff_level_rebases_the_roof_on_its_own_base_level() {
        let mut fx = fixture();
        let (rt, l0) = (fx.rt, fx.l0);
        let (ids, rt2, rt5, lb, l3, l2) = tx(&mut fx.doc, "levels, types and roofs", |m, cx| {
                let lb = ops::create_level(m, cx, "AST-LB", 5.0)?;
                let l3 = ops::create_level(m, cx, "AST-L3", 3.0)?;
                let l2 = ops::create_level(m, cx, "AST-L2", 9.0)?;
                let rt2 = ops::create_slab_type(m, cx, "AST-ROOF-450", 0.45, SlabRole::Roof)?;
                let rt5 = ops::create_slab_type(m, cx, "AST-ROOF-500", 0.5, SlabRole::Roof)?;
                let a = ops::create_slab(m, cx, rt, lb, rect(6.0, 4.0), 0.0, false)?;
                ops::set_slab_segment_slope(m, cx, a, 0, true, 0.5)?;
                ops::set_slab_cutoff(m, cx, a, Some((l2, -2.5)))?;
                let b = ops::create_slab(m, cx, rt2, l0, rect(6.0, 4.0), 0.0, false)?;
                ops::set_slab_segment_slope(m, cx, b, 0, true, 0.5)?;
                ops::set_slab_cutoff(m, cx, b, Some((l2, 0.0)))?;
                let c = ops::create_slab(m, cx, rt5, l3, rect(6.0, 4.0), 0.0, false)?;
                ops::set_slab_segment_slope(m, cx, c, 0, true, 0.5)?;
                ops::set_slab_height_offset(m, cx, c, 0.5)?;
                ops::set_slab_cutoff(m, cx, c, Some((l2, -4.5)))?;
                Ok(([a, b, c], rt2, rt5, lb, l3, l2))
            })
            .expect("setup commits");
        let _ = (rt2, rt5);
        let [a, b, c] = ids;
        // The inputs of the formula, as the engine derives them before the deletion.
        near(state(&fx.doc, a).top_z, 6.5, "A: the cut clips the roof, the top is the cut");
        let b_top = 2.0 + 0.45 * 1.25_f64.sqrt();
        near(state(&fx.doc, b).top_z, b_top, "B: nothing clips the roof, the top is its own");
        near(state(&fx.doc, c).top_z, 4.5, "C: previous cut 4.5");

        let gone = tx(&mut fx.doc, "delete the cut-off level", |m, cx| ops::delete(m, cx, l2))
            .expect("deleting the cut-off level commits");
        assert_eq!(gone, vec![l2], "no roof is deleted with its cut-off level");

        let m = fx.doc.model();
        assert!(m.levels.get(&l2).is_none());
        let cut = |id| m.slab(id).expect("the roof survives").cutoff.expect("and keeps a cut-off");
        let (la, oa) = cut(a);
        assert_eq!(la, lb, "A: up-to level = its own base level");
        near(oa, 1.2, "A: (6.5 - 0.3) - 5.0");
        let (lb_, ob) = cut(b);
        assert_eq!(lb_, l0, "B: its own base level, not the level below the deleted one");
        near(ob, b_top - 0.45, "B: (top - 0.45) - 0.0");
        assert!((ob - 9.0).abs() > 1.0, "B: not the previous cut height above the base");
        let (lc, oc) = cut(c);
        assert_eq!(lc, l3, "C: its own base level");
        near(oc, 1.0, "C: (4.5 - 0.5) - 3.0, the round 10 result");
        near(roof_cutoff_z(m, m.slab(c).unwrap()).unwrap(), 4.0, "C: cut z 4.0");
        // The rewritten cut-offs are valid (the commit above posted no cut-off-below-base error)
        // and the body now ends at z.
        assert!(state(&fx.doc, a).top_z <= 6.2 + 1e-9);
    }

    /// Nothing derived is stored (section 8e): moving the level recomputes the whole body, and a
    /// boundary of a different length cannot keep index-aligned slope data.
    #[test]
    fn every_derived_field_follows_the_model_with_no_bookkeeping() {
        let mut fx = fixture();
        let (rt, l0) = (fx.rt, fx.l0);
        let r1 = tx(&mut fx.doc, "roof", |m, cx| ops::create_slab(m, cx, rt, l0, rect(6.0, 4.0), 0.0, false))
            .unwrap();
        tx(&mut fx.doc, "slope", |m, cx| ops::set_slab_segment_slope(m, cx, r1, 0, true, 0.5))
            .unwrap();
        let before = state(&fx.doc, r1);

        tx(&mut fx.doc, "raise", |m, cx| ops::set_level_elevation(m, cx, l0, 4.0)).unwrap();
        let after = state(&fx.doc, r1);
        near(after.top_z - before.top_z, 4.0, "the whole body rose with its level");
        near(after.area, before.area, "and nothing else changed");

        // A boundary of a different length drops the slope data rather than inventing flat
        // segments the user never drew.
        tx(&mut fx.doc, "retrace", |m, cx| {
                ops::set_slab_boundary(
                    m,
                    cx,
                    r1,
                    vec![Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0), Vec2::new(0.0, 4.0)],
                )
            })
            .unwrap();
        let retraced = state(&fx.doc, r1);
        assert_eq!(retraced.slope, None, "index-aligned data cannot survive a different sketch");
        near(retraced.area, 12.0, "and the new sketch is measured, not remembered");
    }
}

#[cfg(test)]
mod round13_tests {
    //! Rows of round 13 as executable expectations: arc walls, slab loops by nesting parity, and
    //! Join Geometry as a relation.
    use super::*;
    use crate::{ops, CompoundStructure, Layer, LayerFunction};
    use asterion_core::txn::TxnContext;

    fn near(a: f64, b: f64, tol: f64, what: &str) {
        assert!((a - b).abs() <= tol, "{what}: got {a}, want {b}");
    }

    fn tx<R>(
        doc: &mut crate::BimDocument,
        f: impl FnOnce(&mut BimModel, &mut TxnContext<'_>) -> Result<R, Failure>,
    ) -> Result<R, Failure> {
        let out = doc.transact("t", false, f);
        match out.value {
            Some(v) if out.committed() => Ok(v),
            _ => Err(out.failures.into_iter().next().expect("a failure")),
        }
    }

    fn wall_type(m: &mut BimModel, cx: &mut TxnContext<'_>, w: f64) -> ElementId {
        let cs = CompoundStructure::new(vec![Layer { width: w, function: LayerFunction::Structure }], 0, 0).unwrap();
        ops::create_wall_type(m, cx, "WT", cs).unwrap()
    }

    fn new_wall(t: ElementId, l: ElementId, a: (f64, f64), b: (f64, f64)) -> ops::NewWall {
        ops::NewWall {
            wall_type: t,
            level: l,
            start: Vec2::new(a.0, a.1),
            end: Vec2::new(b.0, b.1),
            height: 3.0,
            base_offset: 0.0,
            flip: false,
            structural: false,
        }
    }

    /// Semicircle r 2, t 0.2, h 3: Length 2 pi, Area = theta (r + t/2) h = 19.7920 (the CONVEX
    /// face), 6 faces / 12 edges / 8 endpoints, orientation the left normal of the start tangent.
    #[test]
    fn an_arc_wall_reports_the_convex_face_and_the_measured_topology() {
        let mut doc = crate::new_document();
        let w = tx(&mut doc, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let t = wall_type(m, cx, 0.2);
            ops::create_arc_wall(m, cx, new_wall(t, l, (-2.0, 0.0), (2.0, 0.0)), Vec2::new(0.0, 2.0))
        })
        .unwrap();
        let s = wall_state(doc.model(), doc.model().wall(w).unwrap()).unwrap();
        near(s.length, std::f64::consts::PI * 2.0, 1e-12, "length = theta r");
        near(s.area, std::f64::consts::PI * 2.1 * 3.0, 1e-12, "Area = theta (r + t/2) h");
        // OBSERVED through the public API: the reference product reports a volume of 3.770767875820069 (2.27e-4 above the analytic
        // 3.769911) and a surface area of 41.41253371442017 (1.48e-4 above the analytic 41.412386); it integrates over a tessellated
        // model. The engine returns the exact values, within 3e-4 relative (volume) and 1e-5 relative (area) of the measurements.
        let pi = std::f64::consts::PI;
        near(s.volume, 0.5 * pi * (2.1_f64.powi(2) - 1.9_f64.powi(2)) * 3.0, 1e-12, "exact volume of the semicircle band");
        assert!((s.volume - 3.770767875820069).abs() / 3.770767875820069 < 3e-4, "{}", s.volume);
        let exact_area = pi * (2.1_f64.powi(2) - 1.9_f64.powi(2)) + pi * 2.1 * 3.0 + pi * 1.9 * 3.0 + 2.0 * 0.2 * 3.0;
        near(s.surface_area, exact_area, 1e-10, "exact surface area of the semicircle band");
        assert!((s.surface_area - 41.41253371442017).abs() / 41.41253371442017 < 1e-5, "{}", s.surface_area);
        assert_eq!((s.solid_face_count, s.solid_edge_count, s.solid_vertex_count), (6, 12, 8));
        near(s.orientation.x, -1.0, 1e-12, "outward for a clockwise arc");
        let (lo, hi) = s.arc_bbox.expect("analytic extent");
        near(lo.x, -2.1, 1e-12, "bbox min x");
        near(hi.y, 2.1, 1e-12, "bbox max y");
        near(lo.y, 0.0, 1e-12, "the ends sit on y = 0");
    }

    /// Three collinear points are raised at the call; a radius under the half thickness is posted.
    #[test]
    fn arc_refusals_use_the_two_measured_channels() {
        let mut doc = crate::new_document();
        let (l, t) = tx(&mut doc, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let t = wall_type(m, cx, 0.2);
            Ok((l, t))
        })
        .unwrap();
        let e = tx(&mut doc, |m, cx| {
            ops::create_arc_wall(m, cx, new_wall(t, l, (30.0, 0.0), (34.0, 0.0)), Vec2::new(32.0, 0.0))
        })
        .expect_err("collinear");
        assert_eq!(e.code, codes::INVALID_ARC);
        let e = tx(&mut doc, |m, cx| {
            ops::create_arc_wall(m, cx, new_wall(t, l, (19.95, 0.0), (20.05, 0.0)), Vec2::new(20.0, 0.05))
        })
        .expect_err("radius 0.05 against a half thickness of 0.10");
        assert_eq!(e.code, codes::CANNOT_MAKE_WALL);
    }

    /// A curved wall whose thickened loop has a curve strictly under the short-curve tolerance (0.0025602645572916664 ft)
    /// gets Err 9 from the thicken-and-extrude route and no solid. Here the INNER offset arc is the short one: radius
    /// 0.1003 m against a half thickness of 0.1 m leaves an inner radius of 0.0003 m, and a quarter turn of it is 0.00155
    /// ft. Radius 0.1010 m (inner arc 0.00515 ft) is built.
    #[test]
    fn a_curved_wall_with_a_too_short_offset_arc_is_refused_with_the_k19_error() {
        let quarter = |r: f64| {
            let c = std::f64::consts::FRAC_1_SQRT_2;
            ((r, 0.0), (0.0, r), Vec2::new(r * c, r * c))
        };
        let mut doc = crate::new_document();
        let (l, t) = tx(&mut doc, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let t = wall_type(m, cx, 0.2);
            Ok((l, t))
        })
        .unwrap();
        let (a, b, through) = quarter(0.1003);
        let e = tx(&mut doc, |m, cx| ops::create_arc_wall(m, cx, new_wall(t, l, a, b), through))
            .expect_err("inner offset arc shorter than the short-curve tolerance");
        assert_eq!(e.code, codes::WALL_PROFILE_CURVE_TOO_SHORT);
        let (a, b, through) = quarter(0.1010);
        let w = tx(&mut doc, |m, cx| ops::create_arc_wall(m, cx, new_wall(t, l, a, b), through)).unwrap();
        let s = wall_state(doc.model(), doc.model().wall(w).unwrap()).unwrap();
        assert!(s.volume > 0.0);
        assert_eq!((s.solid_face_count, s.solid_edge_count, s.solid_vertex_count), (6, 12, 8));
    }

    /// An arc wall stored with no positive height (an imported wall whose height is not specified) regenerates without a
    /// body instead of panicking in the prism constructor; the straight path already behaves this way.
    #[test]
    fn an_arc_wall_without_height_has_no_solid_and_does_not_panic() {
        let mut doc = crate::new_document();
        let (l, t) = tx(&mut doc, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let t = wall_type(m, cx, 0.2);
            Ok((l, t))
        })
        .unwrap();
        let w = tx(&mut doc, |m, cx| ops::create_arc_wall(m, cx, new_wall(t, l, (5.0, 0.0), (0.0, 5.0)), Vec2::new(3.5355, 3.5355)))
            .unwrap();
        let s = tx(&mut doc, |m, _| {
            m.walls.get_mut(&w).unwrap().unconnected_height = 0.0;
            Ok(())
        });
        assert!(s.is_ok());
        let s = wall_state(doc.model(), doc.model().wall(w).unwrap()).unwrap();
        assert!(s.solid.is_none() && s.unjoined_solid.is_none());
        assert_eq!(s.volume, 0.0);
        assert!(s.length > 0.0);
    }

    /// Nesting parity decides, and an outside loop is a second island.
    #[test]
    fn a_loop_is_a_hole_or_an_island_by_the_parity_of_its_nesting() {
        let sq = |x: f64, y: f64, s: f64| {
            vec![Vec2::new(x, y), Vec2::new(x + s, y), Vec2::new(x + s, y + s), Vec2::new(x, y + s)]
        };
        let mut doc = crate::new_document();
        let (l, ft) = tx(&mut doc, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let ft = ops::create_floor_type(m, cx, "F", 0.2)?;
            Ok((l, ft))
        })
        .unwrap();
        let outer = vec![Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0), Vec2::new(6.0, 4.0), Vec2::new(0.0, 4.0)];
        let one = tx(&mut doc, |m, cx| {
            ops::create_slab_with_loops(m, cx, ft, l, outer.clone(), vec![sq(2.0, 1.0, 1.0)], 0.0, false)
        })
        .unwrap();
        let s = slab_state(doc.model(), doc.model().slab(one).unwrap()).unwrap();
        near(s.area, 23.0, 1e-12, "24 - 1");
        near(s.perimeter, 24.0, 1e-12, "the interior loop is INCLUDED");
        assert_eq!((s.face_count, s.edge_count), (10, 24));

        let island = tx(&mut doc, |m, cx| {
            ops::create_slab_with_loops(m, cx, ft, l, outer.clone(), vec![sq(50.0, 10.0, 1.0)], 0.0, false)
        })
        .unwrap();
        let s = slab_state(doc.model(), doc.model().slab(island).unwrap()).unwrap();
        near(s.area, 25.0, 1e-12, "a loop OUTSIDE adds: 24 + 1");

        let e = tx(&mut doc, |m, cx| {
            ops::create_slab_with_loops(m, cx, ft, l, outer.clone(), vec![sq(0.0, 1.0, 1.0)], 0.0, false)
        })
        .expect_err("a hole sharing an outer edge is raised at the call");
        assert_eq!(e.code, codes::SLAB_LOOPS_INTERSECT);

        // ... and when the hole is drawn starting along the shared segment (first edge parallel).
        let e = tx(&mut doc, |m, cx| {
            ops::create_slab_with_loops(
                m,
                cx,
                ft,
                l,
                outer.clone(),
                vec![vec![Vec2::new(0.0, 1.0), Vec2::new(0.0, 2.0), Vec2::new(1.0, 2.0), Vec2::new(1.0, 1.0)]],
                0.0,
                false,
            )
        })
        .expect_err("a hole sharing a segment of the outer loop");
        assert_eq!(e.code, codes::SLAB_LINES_OVERLAP);

        // ... while a hole whose edge crosses the outer loop is the not-parallel pair.
        let e = tx(&mut doc, |m, cx| {
            ops::create_slab_with_loops(
                m,
                cx,
                ft,
                l,
                outer.clone(),
                vec![vec![Vec2::new(5.0, 1.0), Vec2::new(8.0, 1.0), Vec2::new(8.0, 2.0), Vec2::new(5.0, 2.0)]],
                0.0,
                false,
            )
        })
        .expect_err("a loop crossing the outer boundary");
        assert_eq!(e.code, codes::SLAB_LOOPS_INTERSECT);
    }

    /// Spec T-02: a type change of a floor/ceiling/roof stores the new type id and nothing else (no sketch, slope, join or
    /// cut-off edit, no refusal inside the change); the same id is a no-op; the depth is re-derived from the new type at
    /// regeneration.
    #[test]
    fn a_slab_type_change_stores_the_type_and_touches_nothing_else() {
        let mut doc = crate::new_document();
        let (c20, c35, floor_t, ceiling, g) = tx(&mut doc, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let c20 = ops::create_slab_type(m, cx, "C20", 0.2, crate::model::SlabRole::Ceiling)?;
            let c35 = ops::create_slab_type(m, cx, "C35", 0.35, crate::model::SlabRole::Ceiling)?;
            let ft = ops::create_floor_type(m, cx, "F", 0.2)?;
            let b = vec![Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0), Vec2::new(6.0, 4.0), Vec2::new(0.0, 4.0)];
            let c = ops::create_sloped_slab(m, cx, c20, l, b, 0.5, false, Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0), 0.25)?;
            Ok((c20, c35, ft, c, 0.25_f64))
        })
        .unwrap();
        let snapshot = doc.model().slab(ceiling).unwrap().clone();
        // the same id: nothing at all
        tx(&mut doc, |m, cx| ops::set_ceiling_type(m, cx, ceiling, c20)).unwrap();
        assert_eq!(*doc.model().slab(ceiling).unwrap(), snapshot);
        // another type of the role: only `type_id` differs
        tx(&mut doc, |m, cx| ops::set_ceiling_type(m, cx, ceiling, c35)).unwrap();
        let mut expected = snapshot.clone();
        expected.type_id = c35;
        assert_eq!(*doc.model().slab(ceiling).unwrap(), expected, "sketch, slope arrow, offsets kept");
        // derived: the vertical depth is the new total width x sqrt(1 + g^2), volume the solid's
        let s = slab_state(doc.model(), doc.model().slab(ceiling).unwrap()).unwrap();
        near(s.thickness, 0.35, 1e-12, "new total layers width");
        near(s.volume, 24.0 * 0.35 * (1.0 + g * g).sqrt(), 1e-12, "solid volume with the new depth");
        // a floor type is not acceptable for a ceiling; nothing changed
        let e = tx(&mut doc, |m, cx| ops::set_slab_type(m, cx, ceiling, floor_t)).expect_err("wrong role");
        assert_eq!(e.code, codes::WRONG_KIND);
        assert_eq!(*doc.model().slab(ceiling).unwrap(), expected);
    }

    /// A type change keeps the sketch and re-derives the depth from the new type's total layer width; the role of an
    /// instance comes from its type.
    #[test]
    fn a_slab_type_change_keeps_the_sketch_and_rederives_the_depth() {
        let mut doc = crate::new_document();
        let (l, f20, f35, ct, floor) = tx(&mut doc, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let f20 = ops::create_floor_type(m, cx, "F20", 0.2)?;
            let f35 = ops::create_floor_type(m, cx, "F35", 0.35)?;
            let ct = ops::create_slab_type(m, cx, "C", 0.2, crate::model::SlabRole::Ceiling)?;
            let b = vec![Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0), Vec2::new(6.0, 4.0), Vec2::new(0.0, 4.0)];
            let hole = vec![Vec2::new(2.0, 1.0), Vec2::new(3.0, 1.0), Vec2::new(3.0, 2.0), Vec2::new(2.0, 2.0)];
            let f = ops::create_slab_with_loops(m, cx, f20, l, b, vec![hole], 0.5, false)?;
            Ok((l, f20, f35, ct, f))
        })
        .unwrap();
        let _ = (l, f20);
        let before = slab_state(doc.model(), doc.model().slab(floor).unwrap()).unwrap();
        near(before.thickness, 0.2, 1e-12, "old depth");
        tx(&mut doc, |m, cx| ops::set_floor_type(m, cx, floor, f35)).unwrap();
        let after = slab_state(doc.model(), doc.model().slab(floor).unwrap()).unwrap();
        near(after.thickness, 0.35, 1e-12, "new total layers width");
        near(after.top_z, before.top_z, 1e-12, "the floor stays pinned at its top plane");
        near(after.bottom_z, before.top_z - 0.35, 1e-12, "and hangs down by the new depth");
        near(after.area, before.area, 1e-12, "sketch and hole kept");
        near(after.volume, before.area * 0.35, 1e-12, "volume follows the type");
        assert_eq!(doc.model().slab(floor).unwrap().loops.len(), 1);
        // a ceiling type cannot be put on a floor
        let e = tx(&mut doc, |m, cx| ops::set_slab_type(m, cx, floor, ct)).expect_err("other category");
        assert_eq!(e.code, codes::WRONG_KIND);
        // and the role wrapper checks the instance too
        let e = tx(&mut doc, |m, cx| ops::set_ceiling_type(m, cx, floor, ct)).expect_err("not a ceiling");
        assert_eq!(e.code, codes::WRONG_KIND);
    }

    /// Translating a slab moves boundary, loops and slope arrow together; quantities are unchanged.
    #[test]
    fn translating_a_slab_moves_boundary_loops_and_slope_arrow_together() {
        let mut doc = crate::new_document();
        let id = tx(&mut doc, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let ft = ops::create_floor_type(m, cx, "F", 0.2)?;
            let b = vec![Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0), Vec2::new(6.0, 4.0), Vec2::new(0.0, 4.0)];
            ops::create_sloped_slab(m, cx, ft, l, b, 0.0, false, Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0), 0.1)
        })
        .unwrap();
        let before = slab_state(doc.model(), doc.model().slab(id).unwrap()).unwrap();
        tx(&mut doc, |m, cx| ops::translate_slab(m, cx, id, Vec2::new(10.0, -2.0))).unwrap();
        let s = doc.model().slab(id).unwrap();
        near(s.boundary[0].x, 10.0, 1e-12, "boundary moved in x");
        near(s.boundary[0].y, -2.0, 1e-12, "boundary moved in y");
        let a = s.slope_arrow.expect("arrow kept");
        near(a.tail.x, 10.0, 1e-12, "arrow tail moved");
        near(a.head.x, 16.0, 1e-12, "arrow head moved");
        let after = slab_state(doc.model(), s).unwrap();
        near(after.area, before.area, 1e-12, "area");
        near(after.volume, before.volume, 1e-12, "volume");
        near(after.bbox_min.x, before.bbox_min.x + 10.0, 1e-12, "bbox follows");
        near(after.top_z, before.top_z, 1e-12, "slope is unchanged: the plane still rises from the tail");
        let e = tx(&mut doc, |m, cx| ops::translate_slab(m, cx, id, Vec2::new(f64::NAN, 0.0)))
            .expect_err("non-finite");
        assert_eq!(e.code, codes::INVALID_BOUNDARY);
    }

    /// An arrow's line starts at the START HEIGHT (its level's elevation plus offset, the slab's base height when it has no
    /// level) and, given by heights, rises to the END HEIGHT over the arrow's plan length; given by slope, at the stored
    /// ratio from the start height.
    #[test]
    fn a_slope_arrow_by_heights_starts_at_the_tail_height_and_ends_at_the_head_height() {
        use crate::model::ArrowHeight;
        let mut doc = crate::new_document();
        let (f, l1) = doc
            .transact("fx", false, |m, cx| {
                let l0 = ops::create_level(m, cx, "L0", 0.0)?;
                let l1 = ops::create_level(m, cx, "L1", 3.0)?;
                let ft = ops::create_floor_type(m, cx, "F200", 0.2)?;
                let b = vec![Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0), Vec2::new(6.0, 4.0), Vec2::new(0.0, 4.0)];
                let f = ops::create_sloped_slab(m, cx, ft, l0, b, 0.5, false, Vec2::new(0.0, 2.0), Vec2::new(6.0, 2.0), 0.1)?;
                Ok((f, l1))
            })
            .value
            .unwrap();
        let z = |doc: &crate::BimDocument, x: f64| {
            let m = doc.model();
            slab_reference_z_at(m, &m.floors[&f], Vec2::new(x, 2.0)).unwrap()
        };
        // no heights: the line starts at the base (0.5) and rises at the stored 0.1
        near(z(&doc, 0.0), 0.5, 1e-12, "base at the tail");
        near(z(&doc, 6.0), 1.1, 1e-12, "slope 0.1");
        // by heights: tail at L0 + 1.0 (offset) and head at L1 + 0.0 -> 1.0 to 3.0 over 6 m
        let tail = Some(ArrowHeight { level: None, offset: 0.5 });
        let head = Some(ArrowHeight { level: Some(l1), offset: 0.0 });
        doc.transact("h", false, |m, cx| ops::set_slab_slope_arrow_heights(m, cx, f, tail, head)).value.unwrap();
        near(z(&doc, 0.0), 1.0, 1e-12, "no level: the base 0.5 plus the offset 0.5");
        near(z(&doc, 6.0), 3.0, 1e-12, "the head at L1");
        let st = slab_state(doc.model(), &doc.model().floors[&f]).unwrap();
        near(st.slope.unwrap(), 2.0 / 6.0, 1e-12, "rise over run from the heights");
        // by slope with a start height: the stored ratio from the tail height
        let tail = Some(ArrowHeight { level: Some(l1), offset: -1.0 });
        doc.transact("s", false, |m, cx| ops::set_slab_slope_arrow_heights(m, cx, f, tail, None)).value.unwrap();
        near(z(&doc, 0.0), 2.0, 1e-12, "L1 - 1.0");
        near(z(&doc, 6.0), 2.6, 1e-12, "slope 0.1 from there");
        // a level that does not resolve falls back to the base height
        let tail = Some(ArrowHeight { level: Some(ElementId(999_999)), offset: 0.25 });
        doc.transact("u", false, |m, cx| ops::set_slab_slope_arrow_heights(m, cx, f, tail, None)).value.unwrap();
        near(z(&doc, 0.0), 0.75, 1e-12, "base 0.5 + 0.25");
        // a slab without an arrow refuses heights
        let flat = doc
            .transact("flat", false, |m, cx| {
                let ft = *m.floor_types.keys().next().unwrap();
                let l0 = *m.levels.keys().next().unwrap();
                ops::create_slab(m, cx, ft, l0, vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0)], 0.0, false)
            })
            .value
            .unwrap();
        assert!(doc.transact("x", false, |m, cx| ops::set_slab_slope_arrow_heights(m, cx, flat, None, None)).value.is_none());
    }

    /// Slope units are tan(angle); an arrow by heights has angle atan2(dh, length).
    #[test]
    fn slope_units_are_a_ratio_and_a_height_arrow_gives_rise_over_run() {
        use crate::model::SlopeArrow;
        near(SlopeArrow::units_from_angle(std::f64::consts::FRAC_PI_4), 1.0, 1e-12, "45 degrees is 1");
        near(SlopeArrow::angle_from_units(0.25), 0.25_f64.atan(), 1e-15, "atan");
        let g = SlopeArrow::slope_from_heights(Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0), 1.0, 2.5);
        near(g, 0.25, 1e-12, "rise 1.5 over run 6");
        let z = SlopeArrow::slope_from_heights(Vec2::new(1.0, 1.0), Vec2::new(1.0, 1.0), 0.0, 3.0);
        near(z, 0.0, 0.0, "an arrow shorter than 1e-9 has angle 0");
    }

    /// The same builder serves a floor and a ceiling. The plane through the slope line is the REFERENCE plane; the slab
    /// lies on the extrusion direction's side of it (down for a floor, up for a ceiling); thickness is PERPENDICULAR to the
    /// planes, the side faces are vertical, the plan footprint is the loop. Slope is a rise/run ratio. Area is the larger
    /// of the summed top / bottom faces (the inclined planes) and the Volume of a slope-arrow ceiling is the SOLID volume
    /// (no shape-edit step), not a formula of the Area.
    #[test]
    fn a_sloped_ceiling_grows_up_from_its_reference_plane_and_a_floor_hangs_down() {
        let mut doc = crate::new_document();
        let g = 0.25; // rise / run
        let (cid, fid, lvl_z) = tx(&mut doc, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 2.0)?;
            let ct = ops::create_slab_type(m, cx, "C", 0.2, crate::model::SlabRole::Ceiling)?;
            let ft = ops::create_floor_type(m, cx, "F", 0.2)?;
            let b = vec![Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0), Vec2::new(6.0, 4.0), Vec2::new(0.0, 4.0)];
            let c = ops::create_sloped_slab(m, cx, ct, l, b.clone(), 0.5, false, Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0), g)?;
            let f = ops::create_sloped_slab(m, cx, ft, l, b, 0.5, false, Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0), g)?;
            Ok((c, f, 2.0))
        })
        .unwrap();
        let ratio = (1.0 + g * g).sqrt();
        let c = slab_state(doc.model(), doc.model().slab(cid).unwrap()).unwrap();
        let f = slab_state(doc.model(), doc.model().slab(fid).unwrap()).unwrap();
        // ceiling: the REFERENCE plane is the bottom, at level + offset at the tail, rising g x run
        near(c.bottom_z, lvl_z + 0.5, 1e-12, "ceiling bottom at the tail");
        near(c.top_z, lvl_z + 0.5 + g * 6.0 + 0.2 * ratio, 1e-12, "grows UP by t x sqrt(1+g^2) vertically");
        // floor: the reference plane is the TOP and the body hangs DOWN
        near(f.top_z, lvl_z + 0.5 + g * 6.0, 1e-12, "floor top at the head");
        near(f.bottom_z, lvl_z + 0.5 - 0.2 * ratio, 1e-12, "hangs DOWN by the same vertical distance");
        // perpendicular thickness: volume = plan x t x sqrt(1+g^2); area is the inclined face
        for s in [&c, &f] {
            near(s.volume, 24.0 * 0.2 * ratio, 1e-12, "plan x vertical thickness");
            near(s.area, 24.0 * ratio, 1e-12, "inclined face");
            near(s.perimeter, 20.0, 1e-12, "plan perimeter: the sides are vertical");
            near(s.bbox_max.x, 6.0, 1e-12, "vertical ends: plan box");
        }
    }

    /// The floor cuts the wall by exactly plan area x shared thickness, and switching the order moves the loss to the
    /// floor.
    #[test]
    fn a_floor_cuts_a_joined_wall_and_switching_the_order_moves_the_loss() {
        let mut doc = crate::new_document();
        let (wl, fl) = tx(&mut doc, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let t = wall_type(m, cx, 0.2);
            let ft = ops::create_floor_type(m, cx, "F", 0.2)?;
            let w = ops::create_wall(m, cx, new_wall(t, l, (0.0, 0.0), (6.0, 0.0)))?;
            let f = ops::create_floor(
                m,
                cx,
                ft,
                l,
                vec![Vec2::new(-1.0, -1.0), Vec2::new(7.0, -1.0), Vec2::new(7.0, 5.0), Vec2::new(-1.0, 5.0)],
                0.5,
                false,
            )?;
            Ok((w, f))
        })
        .unwrap();
        let whole = wall_state(doc.model(), doc.model().wall(wl).unwrap()).unwrap();
        near(whole.volume, 3.6, 1e-12, "uncut");
        tx(&mut doc, |m, cx| ops::join_elements(m, cx, fl, wl)).unwrap();
        let cut = wall_state(doc.model(), doc.model().wall(wl).unwrap()).unwrap();
        near(whole.volume - cut.volume, 6.0 * 0.2 * 0.2, 1e-12, "plan area x shared thickness = 0.24");
        near(cut.area, whole.area - 6.0 * 0.2, 1e-12, "Area follows as Volume / width");
        assert_eq!(cut.solid_face_count, 8, "two lumps, planes merged");

        tx(&mut doc, |m, cx| ops::swap_join_priority(m, cx, fl, wl)).unwrap();
        let back = wall_state(doc.model(), doc.model().wall(wl).unwrap()).unwrap();
        near(back.volume, whole.volume, 1e-12, "the wall is whole again");
        let f = slab_state(doc.model(), doc.model().slab(fl).unwrap()).unwrap();
        near(f.area, 8.0 * 6.0 - 6.0 * 0.2, 1e-12, "the floor lost the wall's strip");
    }

    /// On the engine's one modelled pair. The table makes the FLOOR the cutter whichever order the pair is given in (rule 3
    /// for wall-first, "first cuts second" for floor-first). The switch exchanges the roles and a second switch restores
    /// them.
    #[test]
    fn spec_f05b_the_floor_cuts_the_wall_in_either_order_and_the_switch_exchanges_roles() {
        for floor_first in [true, false] {
            let mut doc = crate::new_document();
            let (wl, fl) = tx(&mut doc, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let t = wall_type(m, cx, 0.2);
                let ft = ops::create_floor_type(m, cx, "F", 0.2)?;
                let w = ops::create_wall(m, cx, new_wall(t, l, (0.0, 0.0), (6.0, 0.0)))?;
                let f = ops::create_floor(
                    m,
                    cx,
                    ft,
                    l,
                    vec![Vec2::new(-1.0, -1.0), Vec2::new(7.0, -1.0), Vec2::new(7.0, 5.0), Vec2::new(-1.0, 5.0)],
                    0.5,
                    false,
                )?;
                Ok((w, f))
            })
            .unwrap();
            let whole = wall_state(doc.model(), doc.model().wall(wl).unwrap()).unwrap();
            if floor_first {
                tx(&mut doc, |m, cx| ops::join_elements(m, cx, fl, wl)).unwrap();
            } else {
                tx(&mut doc, |m, cx| ops::join_elements(m, cx, wl, fl)).unwrap();
            }
            let m = doc.model();
            // The wall HOLDS the step (it is cut), the floor is the id inside it.
            assert!(m.is_cutter_of_join(fl, wl), "floor_first={floor_first}");
            assert!(!m.is_cutter_of_join(wl, fl));
            assert_eq!(m.cutting_elements_of(wl), vec![fl]);
            assert_eq!(m.cut_elements_of(fl), vec![wl]);
            assert!(m.cutting_elements_of(fl).is_empty() && m.cut_elements_of(wl).is_empty());
            let cut = wall_state(m, m.wall(wl).unwrap()).unwrap();
            near(whole.volume - cut.volume, 6.0 * 0.2 * 0.2, 1e-12, "the wall lost the shared band");

            // The roles are exchanged ...
            tx(&mut doc, |m, cx| ops::swap_join_priority(m, cx, wl, fl)).unwrap();
            let m = doc.model();
            assert!(m.is_cutter_of_join(wl, fl) && !m.is_cutter_of_join(fl, wl));
            assert_eq!(m.cut_elements_of(wl), vec![fl]);
            // ... and exchanged back
            tx(&mut doc, |m, cx| ops::swap_join_priority(m, cx, fl, wl)).unwrap();
            assert!(doc.model().is_cutter_of_join(fl, wl));
        }
    }

    /// A room is derived, and these are the rows that define it.
    #[test]
    fn a_room_is_derived_from_the_walls_and_obeys_the_measured_rules() {
        let mut doc = crate::new_document();
        let (l0, l1, t, walls) = tx(&mut doc, |m, cx| {
            let l0 = ops::create_level(m, cx, "L0", 0.0)?;
            let l1 = ops::create_level(m, cx, "L1", 3.0)?;
            let t = wall_type(m, cx, 0.2);
            let mut walls = Vec::new();
            for (a, b) in [((0.0, 0.0), (6.0, 0.0)), ((6.0, 0.0), (6.0, 4.0)), ((6.0, 4.0), (0.0, 4.0))] {
                walls.push(ops::create_wall(m, cx, new_wall(t, l0, a, b))?);
            }
            Ok((l0, l1, t, walls))
        })
        .unwrap();

        // P6: placed in an OPEN region it still exists, with Area 0 and no boundary.
        let r = tx(&mut doc, |m, cx| ops::create_room(m, cx, l0, Vec2::new(3.0, 2.0))).unwrap();
        let open = room_state(doc.model(), doc.model().room(r).unwrap()).unwrap();
        assert!(!open.enclosed);
        near(open.area, 0.0, 1e-12, "open region");
        assert!(open.finish.is_empty());

        // P7: closing the region encloses the same room with no action on it.
        tx(&mut doc, |m, cx| ops::create_wall(m, cx, new_wall(t, l0, (0.0, 4.0), (0.0, 0.0)))).unwrap();
        let s = room_state(doc.model(), doc.model().room(r).unwrap()).unwrap();
        near(s.area, 22.04, 1e-9, "5.8 x 3.8, bounded by the FINISH faces");
        near(s.perimeter, 19.2, 1e-9, "Finish perimeter");
        near(s.perimeter_center, 20.0, 1e-9, "Center perimeter: 8 x half thickness more");
        near(s.unbounded_height, 2.4384, 1e-12, "default 8 ft");
        near(s.volume, 22.04 * 2.4384, 1e-9, "computed by default");
        assert_eq!(s.finish.len(), 4);

        // P15: the height is the room's own, not the walls'.
        tx(&mut doc, |m, cx| ops::set_unconnected_height(m, cx, walls[0], 1.5)).unwrap();
        let s2 = room_state(doc.model(), doc.model().room(r).unwrap()).unwrap();
        near(s2.volume, s.volume, 1e-9, "walls lower than the room change nothing");

        // Upper limit L1 + limit offset 0.5 = 3.5; switching volume computation OFF gives 0.
        tx(&mut doc, |m, cx| {
            ops::set_room_param(m, cx, r, ops::RoomParam::UpperLimit(Some(l1)))?;
            ops::set_room_param(m, cx, r, ops::RoomParam::LimitOffset(0.5))
        })
        .unwrap();
        let s3 = room_state(doc.model(), doc.model().room(r).unwrap()).unwrap();
        near(s3.volume, 22.04 * 3.5, 1e-9, "upper limit 3.0 + offset 0.5");
        tx(&mut doc, |m, cx| ops::set_volume_computation(m, cx, false)).unwrap();
        let s4 = room_state(doc.model(), doc.model().room(r).unwrap()).unwrap();
        near(s4.volume, 0.0, 1e-12, "computation off");
        near(s4.area, 22.04, 1e-9, "and Area untouched");

        // A base offset above the Computation Height (0) is POSTED at commit and rolls back.
        let e = tx(&mut doc, |m, cx| ops::set_room_param(m, cx, r, ops::RoomParam::BaseOffset(0.3)))
            .expect_err("refused at commit");
        assert_eq!(e.code, codes::ROOM_BASE_ABOVE_COMPUTATION_HEIGHT);
        near(doc.model().room(r).unwrap().base_offset, 0.0, 1e-12, "previous value kept");

        // The number is a CURSOR: renamed 101 and deleted, the next room is 102.
        tx(&mut doc, |m, cx| ops::set_room_param(m, cx, r, ops::RoomParam::Number("101".into()))).unwrap();
        tx(&mut doc, |m, cx| ops::delete(m, cx, r).map(|_| ())).unwrap();
        let r2 = tx(&mut doc, |m, cx| ops::create_room(m, cx, l0, Vec2::new(3.0, 2.0))).unwrap();
        assert_eq!(doc.model().room(r2).unwrap().number, "102");

        // Redundancy: a second room in the same region reports 0, the older keeps the area.
        let r3 = tx(&mut doc, |m, cx| ops::create_room(m, cx, l0, Vec2::new(2.0, 2.0))).unwrap();
        let keep = room_state(doc.model(), doc.model().room(r2).unwrap()).unwrap();
        let dup = room_state(doc.model(), doc.model().room(r3).unwrap()).unwrap();
        near(keep.area, 22.04, 1e-9, "the older room keeps the area");
        near(dup.area, 0.0, 1e-12, "the duplicate reports 0");
        assert!(dup.redundant && !dup.finish.is_empty(), "but it still returns its boundary");
    }
}

#[cfg(test)]
mod rooms_rea_tests {
    //! Spec M-07 as executable expectations. Each test names its rule.
    use super::*;
    use crate::{ops, CompoundStructure, Layer, LayerFunction};
    use asterion_core::txn::TxnContext;

    fn tx<R>(
        doc: &mut crate::BimDocument,
        f: impl FnOnce(&mut BimModel, &mut TxnContext<'_>) -> Result<R, Failure>,
    ) -> Result<R, Failure> {
        let out = doc.transact("t", false, f);
        match out.value {
            Some(v) if out.committed() => Ok(v),
            _ => Err(out.failures.into_iter().next().expect("a failure")),
        }
    }

    fn walls(m: &mut BimModel, cx: &mut TxnContext<'_>, level: ElementId, base_offset: f64, height: f64) -> Vec<ElementId> {
        let cs = CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0).unwrap();
        let t = ops::create_wall_type(m, cx, "WT", cs).unwrap();
        let pts = [(0.0, 0.0), (6.0, 0.0), (6.0, 4.0), (0.0, 4.0), (0.0, 0.0)];
        (0..4)
            .map(|i| {
                ops::create_wall(
                    m,
                    cx,
                    ops::NewWall {
                        wall_type: t,
                        level,
                        start: Vec2::new(pts[i].0, pts[i].1),
                        end: Vec2::new(pts[i + 1].0, pts[i + 1].1),
                        height,
                        base_offset,
                        flip: false,
                        structural: false,
                    },
                )
                .unwrap()
            })
            .collect()
    }

    /// Two levels, a 6 x 4 room of 0.2 walls on L0, a room in it.
    fn fixture() -> (crate::BimDocument, ElementId, ElementId, ElementId, Vec<ElementId>) {
        let mut doc = crate::new_document();
        let (l0, l1, w) = tx(&mut doc, |m, cx| {
            let l0 = ops::create_level(m, cx, "L0", 0.0)?;
            let l1 = ops::create_level(m, cx, "L1", 3.0)?;
            let w = walls(m, cx, l0, 0.0, 3.0);
            Ok((l0, l1, w))
        })
        .unwrap();
        let r = tx(&mut doc, |m, cx| ops::create_room(m, cx, l0, Vec2::new(3.0, 2.0))).unwrap();
        (doc, l0, l1, r, w)
    }

    fn state(doc: &crate::BimDocument, r: ElementId) -> RoomState {
        room_state(doc.model(), doc.model().room(r).unwrap()).unwrap()
    }

    /// The body a plan view takes of a wall (`wall_view_body`) is the footprint and the elevations of the full
    /// regeneration, joined corners included - and the same inside a wall grid (`with_wall_index`).
    #[test]
    fn the_view_body_of_a_wall_equals_its_full_regeneration() {
        let (mut doc, l0, _l1, _r, mut ws) = fixture();
        // and an arc wall: its outline is built without its quantity integrations
        let wall_type = doc.model().walls.get(&ws[0]).unwrap().type_id;
        let arc = tx(&mut doc, |m, cx| {
            let spec = ops::NewWall {
                wall_type,
                level: l0,
                start: Vec2::new(20.0, 0.0),
                end: Vec2::new(26.0, 0.0),
                height: 3.0,
                base_offset: 0.0,
                flip: false,
                structural: false,
            };
            ops::create_arc_wall(m, cx, spec, Vec2::new(23.0, 2.0))
        })
        .unwrap();
        ws.push(arc);
        let m = doc.model();
        assert!(m.walls.get(&arc).unwrap().through.is_some());
        assert!(!ws.is_empty());
        let check = || {
            for id in &ws {
                let w = m.walls.get(id).unwrap();
                let full = wall_state(m, w).unwrap();
                let (fp, base, top) = wall_view_body(m, w).unwrap();
                let expect = full.solid.as_ref().map(|p| p.footprint.clone()).unwrap_or_default();
                assert_eq!(fp.len(), expect.len(), "footprint of {id:?}");
                for (a, b) in fp.iter().zip(expect.iter()) {
                    assert!((*a - *b).length() < 1e-12, "{id:?}: {a:?} vs {b:?}");
                }
                assert_eq!((base, top), (full.base_z, full.top_z));
            }
        };
        check();
        crate::joins::with_wall_index(m, check);
    }

    /// Spec M-02: the Volume is the prism's volume and reads 0 when the stored height (top minus bottom) is not above 1e-9
    /// - never negative. A base offset that is refused at commit (Spec M-07) cannot get there, so the height is driven
    /// below zero through the limit offset.
    #[test]
    fn m02_a_non_positive_height_gives_no_solid_and_volume_zero() {
        let (mut doc, _l0, _l1, r, _) = fixture();
        let pos = state(&doc, r);
        assert!(pos.volume > 0.0 && pos.unbounded_height > 0.0);
        tx(&mut doc, |m, cx| ops::set_room_param(m, cx, r, ops::RoomParam::LimitOffset(-0.5))).unwrap();
        let s = state(&doc, r);
        assert!(s.unbounded_height < 0.0);
        assert_eq!(s.volume, 0.0, "a negative height is no geometry, not a negative volume");
        assert!(s.area > 0.0, "the Area is untouched");
        tx(&mut doc, |m, cx| ops::set_room_param(m, cx, r, ops::RoomParam::LimitOffset(0.0))).unwrap();
        assert_eq!(state(&doc, r).volume, 0.0, "a height of exactly 0 is not above 1e-9");
    }

    /// Spec M-04: rooms of one circuit map to the SMALLEST ElementId; the others read 0 for Area, Perimeter and Volume
    /// while their boundary is still returned.
    #[test]
    fn m04_the_lowest_element_id_keeps_the_area_and_the_rest_read_zero() {
        let (mut doc, l0, _l1, r, _) = fixture();
        let r2 = tx(&mut doc, |m, cx| ops::create_room(m, cx, l0, Vec2::new(1.0, 1.0))).unwrap();
        let r3 = tx(&mut doc, |m, cx| ops::create_room(m, cx, l0, Vec2::new(5.0, 3.0))).unwrap();
        assert!(!state(&doc, r).redundant);
        for dup in [r2, r3] {
            let s = state(&doc, dup);
            assert!(s.redundant);
            assert_eq!((s.area, s.perimeter, s.volume), (0.0, 0.0, 0.0));
            assert!(!s.finish.is_empty());
        }
        // Delete the representative: the next smallest id takes over, whatever the creation order.
        tx(&mut doc, |m, cx| ops::delete(m, cx, r).map(|_| ())).unwrap();
        assert!(!state(&doc, r2).redundant && state(&doc, r3).redundant);
    }

    /// Spec M-07: the failure is posted when the base offset is greater than the level's computation offset plus 1e-9 ft;
    /// an offset within that slack, or negative, commits.
    #[test]
    fn m07_the_base_offset_is_compared_with_the_computation_offset_plus_1e9_feet() {
        let (mut doc, _l0, _l1, r, _) = fixture();
        let slack = 0.5e-9 * asterion_core::units::METRES_PER_FOOT;
        tx(&mut doc, |m, cx| ops::set_room_param(m, cx, r, ops::RoomParam::BaseOffset(slack))).unwrap();
        tx(&mut doc, |m, cx| ops::set_room_param(m, cx, r, ops::RoomParam::BaseOffset(-0.3))).unwrap();
        let e = tx(&mut doc, |m, cx| ops::set_room_param(m, cx, r, ops::RoomParam::BaseOffset(0.001)))
            .expect_err("posted");
        assert_eq!(e.code, codes::ROOM_BASE_ABOVE_COMPUTATION_HEIGHT);
        assert!(ops::room_base_offset_above_computation_height(3.0e-9, 0.0));
        assert!(!ops::room_base_offset_above_computation_height(2.0e-10, 0.0));
        // Relative to the level's computation offset, not to zero.
        assert!(!ops::room_base_offset_above_computation_height(0.3, 0.3));
        assert!(ops::room_base_offset_above_computation_height(0.31, 0.3));
    }

    /// Spec M-03: a wall bounds the rooms of a level when the plane z = level elevation + computation offset lies inside
    /// its z span - not merely because it is based on that level.
    #[test]
    fn m03_a_wall_bounds_the_level_whose_plane_it_crosses() {
        // Walls based on L0 but raised 0.5 m: their span [0.5, 3.5] does not hold the plane z = 0.
        let mut doc = crate::new_document();
        let (l0, w) = tx(&mut doc, |m, cx| {
            let l0 = ops::create_level(m, cx, "L0", 0.0)?;
            let w = walls(m, cx, l0, 0.5, 3.0);
            Ok((l0, w))
        })
        .unwrap();
        assert!(!w.is_empty());
        let r = tx(&mut doc, |m, cx| ops::create_room(m, cx, l0, Vec2::new(3.0, 2.0))).unwrap();
        assert!(!state(&doc, r).enclosed, "the raised walls miss the computation plane of L0");

        // Walls of L0, 4.0 m tall, cross the plane of L1 (3.0) by more than the tolerance: they bound a room on L1 too. A
        // wall that merely ends AT L1 (3.0) keeps the old behaviour (UNKNOWN (Spec M-03): the direction of the tolerance),
        // i.e. it does not.
        let mut doc = crate::new_document();
        let (l1, tall) = tx(&mut doc, |m, cx| {
            let l0 = ops::create_level(m, cx, "L0", 0.0)?;
            let l1 = ops::create_level(m, cx, "L1", 3.0)?;
            let w = walls(m, cx, l0, 0.0, 4.0);
            Ok((l1, w))
        })
        .unwrap();
        let r = tx(&mut doc, |m, cx| ops::create_room(m, cx, l1, Vec2::new(3.0, 2.0))).unwrap();
        assert!(state(&doc, r).enclosed, "walls 0..4 cross z = 3");
        tx(&mut doc, |m, cx| ops::set_unconnected_height(m, cx, tall[0], 3.0)).unwrap();
        assert!(!state(&doc, r).enclosed, "one wall now ends exactly at L1: the region is open again");
    }

    /// Spec M-06: the view must be a plan view of a level; one room per enclosed circuit; a placed room is left untouched
    /// so a second call creates nothing and posts the "none created" warning; no enclosed circuit is an error.
    #[test]
    fn m06_the_batch_creator_is_idempotent_per_point() {
        let mut doc = crate::new_document();
        let (l0, view) = tx(&mut doc, |m, cx| {
            let l0 = ops::create_level(m, cx, "L0", 0.0)?;
            let w = walls(m, cx, l0, 0.0, 3.0);
            assert_eq!(w.len(), 4);
            let v = ops::create_view(m, cx, "P0", crate::model::ViewKind::FloorPlan, Some(l0), None)?;
            Ok((l0, v))
        })
        .unwrap();
        let first = tx(&mut doc, |m, cx| ops::auto_place_rooms(m, cx, view)).unwrap();
        assert_eq!(first.len(), 1);
        let out = doc.transact("again", false, |m, cx| ops::auto_place_rooms(m, cx, view));
        assert!(out.committed(), "a warning does not roll back");
        assert_eq!(out.value.unwrap().len(), 0, "nothing is created at a point that holds a placed room");
        assert!(out.failures.iter().any(|f| f.code == codes::ROOM_NONE_CREATED_AUTOMATICALLY));
        assert_eq!(doc.model().rooms.values().filter(|r| r.level == l0).count(), 1);
        // The observed API entry is NOT idempotent (Spec M-06 leaves it open): keep that.
        let more = tx(&mut doc, |m, cx| ops::create_rooms_in_level(m, cx, l0)).unwrap();
        assert_eq!(more.len(), 1);
        // A level with no walls has no circuit: refused, nothing created.
        let (_, empty_view) = tx(&mut doc, |m, cx| {
            let l = ops::create_level(m, cx, "Empty", 9.0)?;
            let v = ops::create_view(m, cx, "PE", crate::model::ViewKind::FloorPlan, Some(l), None)?;
            Ok((l, v))
        })
        .unwrap();
        let e = tx(&mut doc, |m, cx| ops::auto_place_rooms(m, cx, empty_view)).expect_err("no circuit");
        assert_eq!(e.code, codes::ROOM_NO_ENCLOSED_REGION);
        // A view that is not generated by a level is refused.
        let v3d = tx(&mut doc, |m, cx| ops::create_view(m, cx, "3D", crate::model::ViewKind::ThreeD, None, None)).unwrap();
        let e = tx(&mut doc, |m, cx| ops::auto_place_rooms(m, cx, v3d)).expect_err("no level");
        assert_eq!(e.code, codes::VIEW_NO_LEVEL);
    }
}

#[cfg(test)]
mod arc_slab_tests {
    //! Quantities Spec F-02a, F-02b, F-02c, W-08 and slope-defining edges of floors and ceilings.
    use super::*;
    use crate::model::{SlabLoop, SlabSlopeEdge};
    use crate::ops;
    use asterion_core::txn::TxnContext;
    use std::f64::consts::PI;

    fn near(a: f64, b: f64, tol: f64, what: &str) {
        assert!((a - b).abs() <= tol, "{what}: got {a}, want {b}");
    }

    fn tx<R>(
        doc: &mut crate::BimDocument,
        f: impl FnOnce(&mut BimModel, &mut TxnContext<'_>) -> Result<R, Failure>,
    ) -> Result<R, Failure> {
        let out = doc.transact("t", false, f);
        match out.value {
            Some(v) if out.committed() => Ok(v),
            _ => Err(out.failures.into_iter().next().expect("a failure")),
        }
    }

    fn v(x: f64, y: f64) -> Vec2 {
        Vec2::new(x, y)
    }

    /// A 6 x 4 rectangle whose top edge (6,4) -> (0,4) is the arc through (3,5): a circle of radius
    /// 5 centred at (3,0), chord 6, sagitta 1.
    fn arched() -> SlabLoop {
        SlabLoop { points: vec![v(0.0, 0.0), v(6.0, 0.0), v(6.0, 4.0), v(0.0, 4.0)], arcs: vec![None, None, Some(v(3.0, 5.0)), None] }
    }

    /// Analytic plan area and arc length of [`arched`].
    fn arched_exact() -> (f64, f64) {
        let theta = 2.0 * (0.6_f64).asin(); // sin(theta/2) = 3/5
        let segment = 0.5 * 25.0 * (theta - theta.sin());
        (24.0 + segment, 5.0 * theta)
    }

    fn new_floor(doc: &mut crate::BimDocument, boundary: SlabLoop, loops: Vec<SlabLoop>, t: f64) -> Result<ElementId, Failure> {
        tx(doc, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let ft = ops::create_floor_type(m, cx, "F", t)?;
            ops::create_slab_with_curve_loops(m, cx, ft, l, boundary, loops, 0.0, false)
        })
    }

    #[test]
    fn an_arc_edged_floor_has_the_spline_area_and_the_exact_plan_quantities() {
        let mut doc = crate::new_document();
        let id = new_floor(&mut doc, arched(), vec![], 0.2).unwrap();
        let f = doc.model().slab(id).unwrap();
        assert!(f.has_arcs());
        let s = slab_state(doc.model(), f).unwrap();
        let (exact, arc_len) = arched_exact();
        near(s.plan_area, exact, 1e-12, "plan area: rectangle + circular segment");
        near(s.perimeter, 6.0 + 4.0 + arc_len + 4.0, 1e-12, "perimeter: exact arc length (ASSUMED OQ-SLAB-ARC-LENGTH)");
        // the Area is the exact area of the cap. The reference product reports a cap area above the circle (its caps are
        // bounded by curves through sample points; measured 1.5e-4 .. 1.1e-3 m2 above on the arc walls), within 1e-4 relative
        assert!(((s.area - exact) / exact).abs() < 1e-12, "area {} vs analytic {exact}", s.area);
        near(s.volume, s.area * 0.2, 1e-12 * s.volume, "volume of the solid");
        near(s.volume, exact * 0.2, 1e-12 * s.volume, "volume vs the analytic prism");
        near(s.bbox_max.y, 5.0, 1.0e-3, "the apex of the arc bounds the extent, to the display sag");
        near(s.top_z, 0.0, 1e-12, "a floor hangs from its level");
        near(s.bottom_z, -0.2, 1e-12, "by the type thickness");
        // Topology for 4 curves: 6 faces, 12 edges, 8 vertices
        assert_eq!((s.face_count, s.edge_count, s.vertex_count), (6, 12, 8));
        let pr = s.solid.expect("a flat floor keeps a prism for drawing");
        assert!(pr.footprint.len() > 10, "the arc is drawn with its stored samples");
        assert!(pr.footprint.iter().all(|p| p.y <= 5.0 + 1e-12));
    }

    /// A round floor made of two half circles (two points, two arcs) and a circular hole in a
    /// rectangle: the face areas are pi r^2 and 24 - pi r^2 within 1e-4 relative.
    #[test]
    fn a_disc_and_a_round_hole() {
        let mut doc = crate::new_document();
        let disc = SlabLoop { points: vec![v(-3.0, 0.0), v(3.0, 0.0)], arcs: vec![Some(v(0.0, -3.0)), Some(v(0.0, 3.0))] };
        let id = new_floor(&mut doc, disc, vec![], 0.25).unwrap();
        let s = slab_state(doc.model(), doc.model().slab(id).unwrap()).unwrap();
        near(s.plan_area, 9.0 * PI, 1e-12, "plan area");
        assert!(((s.area - 9.0 * PI) / (9.0 * PI)).abs() < 1e-4, "area {}", s.area);
        near(s.perimeter, 6.0 * PI, 1e-12, "perimeter");

        let mut doc = crate::new_document();
        let rect = SlabLoop::polygon(vec![v(0.0, 0.0), v(6.0, 0.0), v(6.0, 4.0), v(0.0, 4.0)]);
        let hole = SlabLoop { points: vec![v(2.0, 2.0), v(4.0, 2.0)], arcs: vec![Some(v(3.0, 1.0)), Some(v(3.0, 3.0))] };
        let id = new_floor(&mut doc, rect, vec![hole], 0.2).unwrap();
        let s = slab_state(doc.model(), doc.model().slab(id).unwrap()).unwrap();
        near(s.plan_area, 24.0 - PI, 1e-12, "plan area: material minus hole");
        assert!(((s.area - (24.0 - PI)) / (24.0 - PI)).abs() < 1e-4, "area {}", s.area);
        near(s.perimeter, 20.0 + 2.0 * PI, 1e-12, "perimeter includes the hole (Spec F-02c)");
        assert_eq!(s.face_count, 4 + 2 + 2, "one side face per curve plus the two caps");
    }

    /// Invalid sketches are refused AT THE CALL with the sketch codes or the arc input rule of ASTERION.
    #[test]
    fn invalid_arc_sketches_are_refused_with_the_right_failure() {
        // an arc through a point on its chord: no circle
        let mut doc = crate::new_document();
        let mut bad = arched();
        bad.arcs[2] = Some(v(3.0, 4.0));
        assert_eq!(new_floor(&mut doc, bad, vec![], 0.2).unwrap_err().code, codes::SLAB_INVALID_ARC);
        // the arc of the top edge bulges DOWN through the bottom edge: the loop crosses itself
        let mut doc = crate::new_document();
        let mut cross = arched();
        cross.arcs[2] = Some(v(3.0, -1.0));
        assert_eq!(new_floor(&mut doc, cross, vec![], 0.2).unwrap_err().code, codes::SLAB_LOOPS_INTERSECT);
        // a hole crossing the arc of the boundary
        let mut doc = crate::new_document();
        let hole = SlabLoop::polygon(vec![v(2.0, 4.5), v(4.0, 4.5), v(4.0, 6.0), v(2.0, 6.0)]);
        assert_eq!(new_floor(&mut doc, arched(), vec![hole], 0.2).unwrap_err().code, codes::SLAB_LOOPS_INTERSECT);
        // a hole touching the arc from inside at its apex (a contact point is an intersection)
        let mut doc = crate::new_document();
        let hole = SlabLoop::polygon(vec![v(2.5, 4.5), v(3.5, 4.5), v(3.0, 5.0)]);
        assert_eq!(new_floor(&mut doc, arched(), vec![hole], 0.2).unwrap_err().code, codes::SLAB_LOOPS_INTERSECT);
        // a hole lying ON the same circle over part of the arc: an overlap range. The code follows the END tangents of the
        // pair - not parallel here (the hole's arc ends at 1.7 rad, the boundary's at 2.21 rad) -> ...
        let (c, r) = (v(3.0, 0.0), 5.0);
        let on = |a: f64| c + Vec2::new(a.cos(), a.sin()) * r;
        let lens = |a0: f64, a1: f64| SlabLoop { points: vec![on(a0), on(a1)], arcs: vec![Some(on(0.5 * (a0 + a1))), None] };
        let mut doc = crate::new_document();
        assert_eq!(new_floor(&mut doc, arched(), vec![lens(1.4, 1.7)], 0.2).unwrap_err().code, codes::SLAB_LOOPS_INTERSECT);
        // ... and parallel when both arcs end at the same point -> lines overlap
        let end = (0.8_f64).atan2(-0.6); // the boundary arc ends at (0, 4)
        let mut doc = crate::new_document();
        assert_eq!(new_floor(&mut doc, arched(), vec![lens(1.9, end)], 0.2).unwrap_err().code, codes::SLAB_LINES_OVERLAP);
        // two points and no arc cannot close
        let mut doc = crate::new_document();
        let two = SlabLoop::polygon(vec![v(0.0, 0.0), v(1.0, 0.0)]);
        assert_eq!(new_floor(&mut doc, two, vec![], 0.2).unwrap_err().code, codes::INVALID_BOUNDARY);
        // a valid hole well inside the arched floor is accepted
        let mut doc = crate::new_document();
        let ok = SlabLoop::polygon(vec![v(1.0, 1.0), v(2.0, 1.0), v(2.0, 2.0)]);
        assert!(new_floor(&mut doc, arched(), vec![ok], 0.2).is_ok());
    }

    /// Moving an arc-edged floor moves its through points: quantities are unchanged; a boundary
    /// edit with the same vertex count keeps the arcs (through points follow a rigid move exactly).
    #[test]
    fn arcs_follow_translation_and_boundary_moves() {
        let mut doc = crate::new_document();
        let id = new_floor(&mut doc, arched(), vec![], 0.2).unwrap();
        let before = slab_state(doc.model(), doc.model().slab(id).unwrap()).unwrap();
        tx(&mut doc, |m, cx| ops::translate_slab(m, cx, id, v(10.0, -2.0))).unwrap();
        let s = doc.model().slab(id).unwrap();
        assert_eq!(s.boundary_arcs[2], Some(v(13.0, 3.0)));
        let after = slab_state(doc.model(), s).unwrap();
        near(after.area, before.area, 1e-9, "area");
        near(after.plan_area, before.plan_area, 1e-9, "plan area");
        let moved: Vec<Vec2> = doc.model().slab(id).unwrap().boundary.iter().map(|p| *p + v(-10.0, 2.0)).collect();
        tx(&mut doc, |m, cx| ops::set_slab_boundary(m, cx, id, moved)).unwrap();
        let s = doc.model().slab(id).unwrap();
        assert_eq!(s.boundary_arcs[2], Some(v(3.0, 5.0)), "the through point followed the move");
        near(slab_state(doc.model(), s).unwrap().plan_area, before.plan_area, 1e-9, "same plan area");
        // a boundary with another vertex count drops the arcs
        let tri = vec![v(0.0, 0.0), v(6.0, 0.0), v(3.0, 4.0)];
        tx(&mut doc, |m, cx| ops::set_slab_boundary(m, cx, id, tri)).unwrap();
        assert!(!doc.model().slab(id).unwrap().has_arcs());
    }

    /// A slope-defining edge holds the line of the edge at its height and rises along `w` = edge direction x +Z (the right
    /// of the travel) or its reverse; the thickness is perpendicular to the plane, `t sqrt(1 + g^2)` vertically; Spec
    /// F-02a: the inclined Area.
    #[test]
    fn a_slope_defining_edge_sets_the_plane_heights() {
        let mut doc = crate::new_document();
        let rect = || SlabLoop::polygon(vec![v(0.0, 0.0), v(6.0, 0.0), v(6.0, 4.0), v(0.0, 4.0)]);
        let id = new_floor(&mut doc, rect(), vec![], 0.3).unwrap();
        // edge 0 runs (0,0) -> (6,0); its right is -y, so `reversed` rises INTO the rectangle (+y)
        let e = SlabSlopeEdge { edge: 0, slope: 0.5, height: 0.25, reversed: true };
        tx(&mut doc, |m, cx| ops::set_slab_slope_edge(m, cx, id, Some(e))).unwrap();
        let m = doc.model();
        let f = m.slab(id).unwrap();
        near(slab_reference_z_at(m, f, v(3.0, 0.0)).unwrap(), 0.25, 1e-12, "on the edge: its height");
        near(slab_reference_z_at(m, f, v(0.0, 4.0)).unwrap(), 0.25 + 2.0, 1e-12, "4 m inside: + 0.5 x 4");
        near(slab_reference_z_at(m, f, v(5.0, 1.0)).unwrap(), 0.75, 1e-12, "1 m inside");
        let s = slab_state(m, f).unwrap();
        near(s.top_z, 2.25, 1e-12, "top = highest point of the top plane");
        near(s.bottom_z, 0.25 - 0.3 * 1.25_f64.sqrt(), 1e-12, "bottom: perpendicular thickness");
        near(s.area, 24.0 * 1.25_f64.sqrt(), 1e-12, "inclined area");
        near(s.volume, 24.0 * 0.3 * 1.25_f64.sqrt(), 1e-12, "plan area x vertical thickness");
        near(s.perimeter, 20.0, 1e-12, "the loops are not moved by the slope (Spec F-02c)");
        assert_eq!(s.slope, Some(0.5));
        // not reversed: the plane rises to the right of the travel, OUT of the rectangle
        let e2 = SlabSlopeEdge { reversed: false, ..e };
        tx(&mut doc, |m, cx| ops::set_slab_slope_edge(m, cx, id, Some(e2))).unwrap();
        let m = doc.model();
        near(slab_reference_z_at(m, m.slab(id).unwrap(), v(0.0, 4.0)).unwrap(), 0.25 - 2.0, 1e-12, "falls inward");
    }

    /// A ceiling with a defining edge and a straight hole: the reference plane is the BOTTOM, the hole is subtracted from
    /// the inclined area, the perimeter keeps the hole (Spec F-02c).
    #[test]
    fn a_sloped_ceiling_with_a_hole() {
        let mut doc = crate::new_document();
        let id = tx(&mut doc, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let ct = ops::create_slab_type(m, cx, "C", 0.05, crate::model::SlabRole::Ceiling)?;
            let b = vec![v(0.0, 0.0), v(6.0, 0.0), v(6.0, 4.0), v(0.0, 4.0)];
            let hole = vec![v(1.0, 1.0), v(2.0, 1.0), v(2.0, 2.0), v(1.0, 2.0)];
            let id = ops::create_slab_with_loops(m, cx, ct, l, b, vec![hole], 2.6, false)?;
            ops::set_slab_slope_edge(m, cx, id, Some(SlabSlopeEdge { edge: 0, slope: 0.25, height: 0.0, reversed: true }))?;
            Ok(id)
        })
        .unwrap();
        let s = slab_state(doc.model(), doc.model().slab(id).unwrap()).unwrap();
        let k = 1.0625_f64.sqrt();
        near(s.area, 23.0 * k, 1e-12, "net inclined area");
        near(s.volume, 23.0 * 0.05 * k, 1e-12, "net volume");
        near(s.perimeter, 24.0, 1e-12, "perimeter with the hole");
        near(s.bottom_z, 2.6, 1e-12, "the bottom holds the edge at the ceiling height");
        near(s.top_z, 2.6 + 1.0 + 0.05 * k, 1e-12, "the top: rise + vertical thickness");
    }

    /// The slope refusals: a second slope, an arc edge (ENGINE CHOICE), a roof (per-segment slopes), an index outside the
    /// boundary.
    #[test]
    fn slope_edge_refusals() {
        let mut doc = crate::new_document();
        let id = new_floor(&mut doc, arched(), vec![], 0.2).unwrap();
        let e = SlabSlopeEdge { edge: 2, slope: 0.1, height: 0.0, reversed: false };
        let err = tx(&mut doc, |m, cx| ops::set_slab_slope_edge(m, cx, id, Some(e))).unwrap_err();
        assert_eq!(err.code, codes::SLAB_SLOPE_EDGE_NOT_LINE);
        let err = tx(&mut doc, |m, cx| ops::set_slab_slope_edge(m, cx, id, Some(SlabSlopeEdge { edge: 9, ..e }))).unwrap_err();
        assert_eq!(err.code, codes::SLAB_SEGMENT_OUT_OF_RANGE);
        // an arc-edged floor with a straight defining edge regenerates through the face integrals
        tx(&mut doc, |m, cx| ops::set_slab_slope_edge(m, cx, id, Some(SlabSlopeEdge { edge: 0, reversed: true, ..e }))).unwrap();
        let s = slab_state(doc.model(), doc.model().slab(id).unwrap()).unwrap();
        let (exact, _) = arched_exact();
        let k = (1.0_f64 + 0.01).sqrt();
        assert!(((s.area - exact * k) / (exact * k)).abs() < 1e-4, "inclined spline area {}", s.area);
        near(s.top_z, 0.1 * 5.0, 1e-12, "the apex of the arc is the highest point");

        let mut doc = crate::new_document();
        let id = tx(&mut doc, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let ft = ops::create_floor_type(m, cx, "F", 0.2)?;
            let b = vec![v(0.0, 0.0), v(6.0, 0.0), v(6.0, 4.0), v(0.0, 4.0)];
            ops::create_sloped_slab(m, cx, ft, l, b, 0.0, false, v(0.0, 0.0), v(0.0, 4.0), 0.1)
        })
        .unwrap();
        let err = tx(&mut doc, |m, cx| ops::set_slab_slope_edge(m, cx, id, Some(SlabSlopeEdge { edge: 0, slope: 0.1, height: 0.0, reversed: false })))
            .unwrap_err();
        assert_eq!(err.code, codes::SLAB_TOO_MANY_SLOPES);

        let mut doc = crate::new_document();
        let id = tx(&mut doc, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let rt = ops::create_slab_type(m, cx, "R", 0.3, crate::model::SlabRole::Roof)?;
            ops::create_slab(m, cx, rt, l, vec![v(0.0, 0.0), v(6.0, 0.0), v(6.0, 4.0), v(0.0, 4.0)], 0.0, false)
        })
        .unwrap();
        let err = tx(&mut doc, |m, cx| ops::set_slab_slope_edge(m, cx, id, Some(SlabSlopeEdge { edge: 0, slope: 0.1, height: 0.0, reversed: false })))
            .unwrap_err();
        assert_eq!(err.code, codes::SLAB_ROLE_HAS_NO_SLOPE);
    }

    /// A sloped floor (slope arrow) with a hole: the hole now counts (it was ignored before the
    /// generalisation) - Area (24 - 1) x sqrt(1 + g^2).
    #[test]
    fn a_slope_arrow_floor_subtracts_its_holes() {
        let mut doc = crate::new_document();
        let id = tx(&mut doc, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let ft = ops::create_floor_type(m, cx, "F", 0.2)?;
            let b = vec![v(0.0, 0.0), v(6.0, 0.0), v(6.0, 4.0), v(0.0, 4.0)];
            let id = ops::create_sloped_slab(m, cx, ft, l, b, 0.0, false, v(0.0, 0.0), v(6.0, 0.0), 0.5)?;
            m.slab_mut(id)?.loops = vec![vec![v(1.0, 1.0), v(2.0, 1.0), v(2.0, 2.0), v(1.0, 2.0)]];
            Ok(id)
        })
        .unwrap();
        let s = slab_state(doc.model(), doc.model().slab(id).unwrap()).unwrap();
        near(s.area, 23.0 * 1.25_f64.sqrt(), 1e-12, "net inclined area");
        near(s.perimeter, 24.0, 1e-12, "perimeter with the hole");
        near(slab_reference_z_at(doc.model(), doc.model().slab(id).unwrap(), v(6.0, 2.0)).unwrap(), 3.0, 1e-12, "head end");
    }

    /// Roofs: plane heights of a shed roof at given points (the underside slope-defining edge); an arc edge elsewhere in
    /// the footprint is built on its stored samples (ASSUMED); a slope-defining ARC edge is left out of the body and
    /// counted.
    #[test]
    fn roof_plane_heights_and_arc_footprints() {
        let mut doc = crate::new_document();
        let (rect_roof, arc_roof) = tx(&mut doc, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 3.0)?;
            let rt = ops::create_slab_type(m, cx, "R", 0.3, crate::model::SlabRole::Roof)?;
            let a = ops::create_slab(m, cx, rt, l, vec![v(0.0, 0.0), v(6.0, 0.0), v(6.0, 4.0), v(0.0, 4.0)], 0.0, false)?;
            ops::set_slab_segment_slope(m, cx, a, 0, true, 0.5)?;
            let b = ops::create_slab_with_curve_loops(m, cx, rt, l, arched(), vec![], 0.0, false)?;
            ops::set_slab_segment_slope(m, cx, b, 0, true, 0.5)?;
            Ok((a, b))
        })
        .unwrap();
        let m = doc.model();
        let a = m.slab(rect_roof).unwrap();
        near(slab_reference_z_at(m, a, v(3.0, 0.0)).unwrap(), 3.0, 1e-12, "the defining edge sits at the base");
        near(slab_reference_z_at(m, a, v(3.0, 2.0)).unwrap(), 4.0, 1e-12, "2 m in: + 0.5 x 2");
        near(slab_reference_z_at(m, a, v(1.0, 4.0)).unwrap(), 5.0, 1e-12, "far side");
        let b = m.slab(arc_roof).unwrap();
        let s = slab_state(m, b).unwrap();
        near(slab_reference_z_at(m, b, v(3.0, 5.0)).unwrap(), 5.5, 1e-12, "the apex of the arc, 5 m from the edge");
        near(s.bbox_max.y, 5.0, 1.0e-3, "the body reaches the apex sample, to the display sag");
        near(s.bbox_min.z, 3.0, 1e-12, "underside on the base");
        let (exact, _) = arched_exact();
        near(s.plan_area, exact, 1e-12, "the exact plan area of the sketch");
        assert!(((s.area - exact * 1.25_f64.sqrt()) / s.area).abs() < 1e-3, "the inclined area of the sampled body {}", s.area);
        assert_eq!(s.skipped_arc_slopes, 0);
        // a slope-defining arc edge is refused by the op...
        let err = tx(&mut doc, |m, cx| ops::set_slab_segment_slope(m, cx, arc_roof, 2, true, 0.5)).unwrap_err();
        assert_eq!(err.code, codes::SLAB_SLOPE_EDGE_NOT_LINE);
        // ...and one stored by an importer is left out of the body and counted
        tx(&mut doc, |m, _| {
            m.slab_mut(arc_roof)?.segments[2] = crate::model::SlabSegment { defines_slope: true, slope: 0.5, ..Default::default() };
            Ok(())
        })
        .unwrap();
        let s = slab_state(doc.model(), doc.model().slab(arc_roof).unwrap()).unwrap();
        assert_eq!(s.skipped_arc_slopes, 1);
    }
}

#[cfg(test)]
mod opening_face_tests {
    use super::*;

    fn faces(host: (f64, f64), cuts: &[(f64, f64, f64, f64)]) -> usize {
        let outline = HostOutline { u_lo: 0.0, u_hi: host.0, z_lo: 0.0, z_hi: host.1 };
        let rects: Vec<cut::Rect> = cuts.iter().map(|c| cut::Rect::new(c.0, c.2, c.1, c.3)).collect();
        merged_face_count(&outline, &rects, 0.2).expect("a region")
    }

    /// Spec W-11b, round 10: a 10 x 3 wall, openings made one at a time; the reference product reports 6, 10, 13, 16, 18,
    /// 18, 18, 21, 24 faces and 20 once the first one is deleted again.
    #[test]
    fn each_cutter_adds_its_own_faces_and_coplanar_ones_of_different_cutters_do_not_merge() {
        let o1 = (1.0, 2.0, 1.0, 2.0); // floating: 4
        let o2 = (3.0, 4.0, 0.0, 2.0); // touches the base: 3
        let o3 = (5.0, 6.0, 1.0, 3.0); // touches the top: 3 (its bottom z = 1 is not merged with o1's)
        let o4 = (7.0, 8.0, 0.0, 3.0); // full height: 2
        let o5 = (1.2, 1.8, 1.2, 1.8); // inside o1: 0
        let o6 = (12.0, 13.0, 1.0, 2.0); // off the wall: 0
        let o7 = (9.5, 10.5, 1.0, 2.0); // straddles the end: 3
        let o8 = (-0.5, 0.5, 1.0, 2.0); // straddles the start: 3
        let host = (10.0, 3.0);
        assert_eq!(faces(host, &[]), 6);
        assert_eq!(faces(host, &[o1]), 10);
        assert_eq!(faces(host, &[o1, o2]), 13);
        assert_eq!(faces(host, &[o1, o2, o3]), 16);
        assert_eq!(faces(host, &[o1, o2, o3, o4]), 18);
        assert_eq!(faces(host, &[o1, o2, o3, o4, o5]), 18);
        assert_eq!(faces(host, &[o1, o2, o3, o4, o5, o6]), 18);
        assert_eq!(faces(host, &[o1, o2, o3, o4, o5, o6, o7]), 21);
        assert_eq!(faces(host, &[o1, o2, o3, o4, o5, o6, o7, o8]), 24);
        assert_eq!(faces(host, &[o2, o3, o4, o7, o8]), 20, "o5 and o6 were deleted before o1");
    }

    /// Round 9 (a 6 x 3 wall): the faces of one cutter parted by a later one are still one face each.
    #[test]
    fn a_face_parted_in_two_by_a_later_cutter_stays_one_face() {
        let door = (3.0, 3.9, 0.0, 2.1);
        let wider = (-1.0, 7.0, 0.5, 1.5);
        assert_eq!(faces((6.0, 3.0), &[door]), 9);
        assert_eq!(faces((6.0, 3.0), &[door, wider]), 11);
        let hanging = (5.5, 6.5, 0.5, 1.5);
        assert_eq!(faces((6.0, 3.0), &[door, wider, hanging]), 11);
        let above_top = (4.6, 5.2, 2.8, 3.4);
        assert_eq!(faces((6.0, 3.0), &[door, wider, hanging, above_top]), 14);
        let window = (1.0, 2.2, 0.9, 2.1);
        assert_eq!(faces((6.0, 3.0), &[window]), 10);
        assert_eq!(faces((6.0, 3.0), &[window, door]), 13);
    }

    /// A cutter stacked on another one shares its jambs: the faces of the pair are those of the union.
    #[test]
    fn a_stacked_cutter_merges_its_jambs_into_the_one_below() {
        // [1,2] x [0,1] then [1,2] x [1,2]: left and right merge, the first one's top is swallowed, the new top is new
        assert_eq!(faces((6.0, 3.0), &[(1.0, 2.0, 0.0, 1.0), (1.0, 2.0, 1.0, 2.0)]), 6 + 3);
    }
}
