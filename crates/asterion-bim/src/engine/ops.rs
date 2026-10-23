//! Editing operations of the BIM semantic engine.
//!
//! Every operation runs inside a transaction (`asterion_core::Document::transact`)
//! and receives the model plus a [`TxnContext`]. Two error channels mirror the
//! two behaviours observed in the reference product:
//!
//! * `Err(Failure)` â€” the request is **rejected** before any change (invalid
//!   argument, read-only parameter). The reference raises an API exception.
//! * `cx.report(Failure::error(..))` â€” the change was applied but the model cannot
//!   be made consistent; the transaction **fails at commit** and rolls back. The
//!   reference posts a failure (e.g. "Can't make Wall.") or silently rolls back.

use crate::model::{
    BimModel, CompoundStructure, CropRegion, DatumEnd, DetailLevel, Grid, HostedInstance, HostedInstanceParam,
    HostedKind, HostedType, HostedTypeParam, Level, LocationLine, PlanViewPlane, RangePlane, View, ViewKind, ViewRange,
    Wall, WallOpening, WallType,
};
use crate::regen::{host_cuts, wall_state};
use crate::spec::{codes, MAX_WALL_HEIGHT_M};
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use asterion_geom::Vec2;

pub struct NewWall {
    pub wall_type: ElementId,
    pub level: ElementId,
    pub start: Vec2,
    pub end: Vec2,
    pub height: f64,
    pub base_offset: f64,
    pub flip: bool,
    pub structural: bool,
}

/// Create a level.
///
/// A plan view of the level is a separate element, found by association with the level. At the database layer plan views of
/// levels are made by a separate step, never by the level creator. The Level TOOL (not this operation) creates the new
/// level and then, only when its "Make Plan View" check box is set, one plan view per view type of the document's new-level
/// list (for a document with an empty stored set, every plan view type of the document, in document order) - see
/// `asterion_level_create` in the FFI layer.
///
/// The text of the resource and the body of the next-number operation were not specified, so the engine makes no default
/// name - the caller passes one.
///
/// A new level has no story above and is a building story (Spec L-01: the flag is true at construction).
pub fn create_level(m: &mut BimModel, cx: &mut TxnContext<'_>, name: &str, elevation: f64) -> Result<ElementId, Failure> {
    require_numbers("The level elevation", &[elevation])?;
    // OBSERVED: a level named as another one is refused at the call ("Name must be unique"); a second level at the same
    // elevation is accepted.
    if m.levels.values().any(|l| l.name == name) {
        return Err(Failure::error(codes::LEVEL_DUPLICATE_NAME, "Name must be unique."));
    }
    let id = cx.new_id();
    m.levels.insert(id, Level { id, name: name.to_owned(), elevation, story_above: None, building_story: true });
    Ok(id)
}

/// Re-run the story-above validation of one level (Spec L-01 -> the story-above check, which runs at regeneration of the
/// level).
///
/// The story-above parameter is read; when it names an existing Level whose elevation is below this
/// level's elevation + 1e-9 (feet) the warning "story above moved" is posted on this level; when
/// that level is at or above it but is not a building story the warning "story above is not a
/// building story" is posted; in both cases the parameter is then cleared. A story above that is a
/// building story and above changes nothing; an element that is not a Level only clears the
/// parameter. Both warnings have failure severity code 1.
fn validate_story_above(m: &mut BimModel, cx: &mut TxnContext<'_>, level: ElementId) {
    let Some(this) = m.levels.get(&level) else { return };
    let Some(above_id) = this.story_above else { return };
    let elevation = this.elevation;
    let verdict = match m.levels.get(&above_id) {
        None => None,
        Some(above) => {
            if above.elevation * crate::spec::FEET_PER_METRE < elevation * crate::spec::FEET_PER_METRE + 1e-9 {
                Some(Failure::warning(codes::LEVEL_STORY_ABOVE_MOVED, "The story above is not above this level.").on(&[level]))
            } else if !above.building_story {
                Some(
                    Failure::warning(codes::LEVEL_STORY_ABOVE_NOT_A_STORY, "The story above is not a building story.")
                        .on(&[level]),
                )
            } else {
                return;
            }
        }
    };
    if let Some(f) = verdict {
        cx.report(f);
    }
    m.levels.get_mut(&level).expect("looked up above").story_above = None;
}

/// The levels that name `level` as their story above, in id order.
fn levels_with_story_above(m: &BimModel, level: ElementId) -> Vec<ElementId> {
    m.levels.values().filter(|l| l.story_above == Some(level)).map(|l| l.id).collect()
}

/// Write a level's story-above parameter. Spec L-01: the parameter is one of the level's own atoms and is validated at
/// regeneration of the level.
///
/// UNKNOWN (Spec L-01): which UI path writes the parameter and which element ids the API accepts; the id is stored as given
/// and validated, so an id that is not a level is cleared.
pub fn set_level_story_above(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    level: ElementId,
    story_above: Option<ElementId>,
) -> Result<(), Failure> {
    m.levels.get_mut(&level).ok_or_else(|| not_found(level))?.story_above = story_above;
    validate_story_above(m, cx, level);
    Ok(())
}

/// Write a level's building-story flag. Spec L-01: every level that names it as its story above is re-validated.
pub fn set_level_building_story(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    level: ElementId,
    building_story: bool,
) -> Result<(), Failure> {
    m.levels.get_mut(&level).ok_or_else(|| not_found(level))?.building_story = building_story;
    for l in levels_with_story_above(m, level) {
        validate_story_above(m, cx, l);
    }
    Ok(())
}

/// Move a level.
///
/// OBSERVED: walls follow their level. ASSUMED: a hosted instance travels with its host, and its absolute sill elevation is
/// the new level elevation plus its `Sill Height` - so the move is **committed** unless the travelled cut leaves the host,
/// which is why the instances placed on the level are revisited here in addition to the ones revisited through their host
/// by [`check_walls`]. The two sets differ exactly when an instance's level is not its host's base level.
pub fn set_level_elevation(m: &mut BimModel, cx: &mut TxnContext<'_>, level: ElementId, elevation: f64) -> Result<(), Failure> {
    require_numbers("The level elevation", &[elevation])?;
    m.levels.get_mut(&level).ok_or_else(|| not_found(level))?.elevation = elevation;
    // Spec L-01: moves the level datum by (0, 0, z - elevation) and marks its position atom touched; a level whose story
    // above is this level is re-validated (its finish step lists this level's elevation as a parent). The dependents of a
    // level in the other databases (walls, floors, roofs, hosted instances, rooms) were not specified by what follows is
    // the engine's OBSERVED behaviour.
    for l in levels_with_story_above(m, level) {
        validate_story_above(m, cx, l);
    }
    let dependents = m.dependents(level);
    check_walls(m, cx, &dependents);
    // OBSERVED: raising a roof's base level - or moving its cut-off level - can leave the cut-off at or below the
    // underside, and that is refused at commit. `dependents` already holds the roofs BASED on this level; a roof only CUT
    // by it is not a dependent, so it is added.
    let mut roofs: Vec<ElementId> = dependents.iter().copied().filter(|id| m.floors.contains_key(id)).collect();
    roofs.extend(m.floors.values().filter(|s| s.cutoff.map_or(false, |(l, _)| l == level)).map(|s| s.id));
    roofs.sort();
    roofs.dedup();
    check_slabs(m, cx, &roofs);
    let instances: Vec<ElementId> =
        dependents.iter().copied().filter(|id| m.hosted_instances.contains_key(id)).collect();
    check_inserts(m, cx, &instances);
    // The riser step of a stairs lists the stairs elevation-span atoms among its parents, and its input is the stairs
    // height (top level minus base level), so a stairs whose base or top level moved is regenerated and its failures are
    // posted for the commit (an error rolls the move back). Spec L-01: a level move marks the level's position atom touched
    // and its dependents regenerate. UNKNOWN: which atoms "the stairs elevation-span atoms" are - the rule does not say
    // they are the levels' position atoms - and how the reference product derives the stairs bottom and top from the levels
    // and the offsets (the engine uses the levels' elevations). UNKNOWN (Spec L-01): the Stair parent overrides: whether
    // the runs, landings and railings of a stairs travel with a moved base level is not specified, so only the regeneration
    // of the stairs element itself is run here.
    let stairs = m.stairs.stairs_depending_on(level);
    crate::stairs::check_stairs(m, cx, &stairs);
    Ok(())
}

pub fn create_wall_type(m: &mut BimModel, cx: &mut TxnContext<'_>, name: &str, structure: CompoundStructure) -> Result<ElementId, Failure> {
    let id = cx.new_id();
    m.wall_types.insert(id, WallType { id, name: name.to_owned(), structure });
    Ok(id)
}

/// Replace a type's layers. OBSERVED: each instance keeps its location-line reference fixed; the centre line moves by the
/// change of offset.
pub fn set_wall_type_structure(m: &mut BimModel, cx: &mut TxnContext<'_>, type_id: ElementId, structure: CompoundStructure) -> Result<(), Failure> {
    let old = m.wall_type(type_id)?.structure.clone();
    let walls: Vec<ElementId> = m.walls.values().filter(|w| w.type_id == type_id).map(|w| w.id).collect();
    for id in &walls {
        shift_for_structure_change(m, *id, &old, &structure)?;
    }
    m.wall_types.get_mut(&type_id).expect("checked").structure = structure;
    check_walls(m, cx, &walls);
    Ok(())
}

/// Change the type of a wall, keeping its location-line reference fixed (OBSERVED).
pub fn set_wall_type(m: &mut BimModel, cx: &mut TxnContext<'_>, wall: ElementId, type_id: ElementId) -> Result<(), Failure> {
    crate::groups::guard(m, cx, &[wall]);
    let new = m.wall_type(type_id)?.structure.clone();
    let old = m.wall_type(m.wall(wall)?.type_id)?.structure.clone();
    shift_for_structure_change(m, wall, &old, &new)?;
    m.wall_mut(wall)?.type_id = type_id;
    check_walls(m, cx, &[wall]);
    Ok(())
}

fn shift_for_structure_change(m: &mut BimModel, wall: ElementId, old: &CompoundStructure, new: &CompoundStructure) -> Result<(), Failure> {
    let w = m.wall(wall)?;
    let n = wall_state(m, w)?.orientation;
    let delta = n * (old.offset_for_location_line(w.location_line) - new.offset_for_location_line(w.location_line));
    let w = m.wall_mut(wall)?;
    w.start = w.start + delta;
    w.end = w.end + delta;
    Ok(())
}

/// Create a straight wall whose centre line runs from `start` to `end`.
///
/// OBSERVED rules:
/// * curves shorter than the short-curve tolerance are rejected;
/// * heights `<= 0` or `> 30000 ft` are rejected;
/// * the default location line is the wall centre line; `flip` only flips the orientation;
/// * very short walls fail with "Can't make Wall." and too-low walls are rolled back (see `check_walls`).
pub fn create_wall(m: &mut BimModel, cx: &mut TxnContext<'_>, spec: NewWall) -> Result<ElementId, Failure> {
    m.wall_type(spec.wall_type)?;
    m.level(spec.level)?;
    check_curve(spec.start, spec.end)?;
    require_numbers("The wall base offset", &[spec.base_offset])?;
    if !(spec.height > 0.0 && spec.height <= MAX_WALL_HEIGHT_M) {
        return Err(Failure::error(codes::HEIGHT_OUT_OF_RANGE, "The given value for height must be greater than 0 and no more than 30000 feet."));
    }
    // QA: a height so small that adding it to the level's elevation changes nothing (1e-308 m on a level at 20 m) has no top above its
    // base: it is as good as 0, refused like 0, instead of reaching the solid with a prism of no height
    let base_z = m.level(spec.level)?.elevation + spec.base_offset;
    if !(base_z + spec.height > base_z) {
        return Err(Failure::error(codes::HEIGHT_OUT_OF_RANGE, "The given value for height must be greater than 0 and no more than 30000 feet."));
    }
    let id = cx.new_id();
    m.walls.insert(
        id,
        Wall {
            id,
            type_id: spec.wall_type,
            base_level: spec.level,
            base_offset: spec.base_offset,
            top_level: None,
            top_offset: 0.0,
            unconnected_height: spec.height,
            start: spec.start,
            end: spec.end,
            location_line: LocationLine::WallCenterline,
            flipped: spec.flip,
            structural: spec.structural,
            join_allowed: [true, true],
            through: None,
            room_bounding: true,
        },
    );
    // OBSERVED: an end created against a host snaps onto the host's location line.
    for end_idx in 0..2 {
        snap_end_to_host(m, id, end_idx, None);
    }
    check_walls(m, cx, &[id]);
    Ok(id)
}

/// Create a wall whose location curve is the ARC through `start`, `through` and `end`.
///
/// OBSERVED: three collinear points are raised AT THE CALL (`Arc.Create` throws, parameter `end1`) and never reach the
/// model. A radius that cannot hold the wall's thickness is POSTED at commit ("Can't make Wall."): a radius of 0.05 with a
/// half thickness of 0.10. The threshold itself is one sample - ASSUMED `radius <= half thickness`.
pub fn create_arc_wall(m: &mut BimModel, cx: &mut TxnContext<'_>, spec: NewWall, through: Vec2) -> Result<ElementId, Failure> {
    if crate::regen::arc_from_points(spec.start, spec.end, through).is_none() {
        return Err(Failure::error(codes::INVALID_ARC, "Cannot create an arc from three collinear points."));
    }
    let id = create_wall(m, cx, spec)?;
    m.wall_mut(id)?.through = Some(through);
    check_walls(m, cx, &[id]);
    Ok(id)
}

/// OBSERVED: changing the location line moves neither the centre line nor the body.
pub fn set_location_line(m: &mut BimModel, cx: &mut TxnContext<'_>, wall: ElementId, line: LocationLine) -> Result<(), Failure> {
    crate::groups::guard(m, cx, &[wall]);
    m.wall_mut(wall)?.location_line = line;
    Ok(())
}

pub fn flip_wall(m: &mut BimModel, cx: &mut TxnContext<'_>, wall: ElementId) -> Result<(), Failure> {
    crate::groups::guard(m, cx, &[wall]);
    let w = m.wall(wall)?;
    let n = wall_state(m, w)?.orientation;
    let off = m.wall_type(w.type_id)?.structure.offset_for_location_line(w.location_line);
    let delta = n * (2.0 * off);
    let w = m.wall_mut(wall)?;
    let old = (w.start, w.end);
    w.start = w.start + delta;
    w.end = w.end + delta;
    w.flipped = !w.flipped;
    // The joined ends follow the drive curve that moved
    if delta.length() > 0.0 {
        crate::joins::follow_neighbours(m, wall, old);
    }
    Ok(())
}

pub fn set_wall_base_offset(m: &mut BimModel, cx: &mut TxnContext<'_>, wall: ElementId, value: f64) -> Result<(), Failure> {
    require_numbers("The wall base offset", &[value])?;
    crate::groups::guard(m, cx, &[wall]);
    m.wall_mut(wall)?.base_offset = value;
    check_walls(m, cx, &[wall]);
    Ok(())
}

/// OBSERVED: rejected as read-only while the top is attached to a level.
pub fn set_unconnected_height(m: &mut BimModel, cx: &mut TxnContext<'_>, wall: ElementId, value: f64) -> Result<(), Failure> {
    crate::groups::guard(m, cx, &[wall]);
    let w = m.wall_mut(wall)?;
    if w.top_level.is_some() {
        return Err(Failure::error(codes::READ_ONLY, "The parameter is read-only.").on(&[wall]));
    }
    if !(value > 0.0 && value <= MAX_WALL_HEIGHT_M) {
        // ASSUMED: same range rule as creation (not yet observed for the parameter path).
        return Err(Failure::error(codes::HEIGHT_OUT_OF_RANGE, "Height must be greater than 0 and no more than 30000 feet.").on(&[wall]));
    }
    w.unconnected_height = value;
    check_walls(m, cx, &[wall]);
    Ok(())
}

/// Attach the top to `level` or release it (`None`).
///
/// Attaching without an explicit offset keeps the stored offset.
pub fn set_top_constraint(m: &mut BimModel, cx: &mut TxnContext<'_>, wall: ElementId, level: Option<ElementId>, offset: Option<f64>) -> Result<(), Failure> {
    crate::groups::guard(m, cx, &[wall]);
    if let Some(l) = level {
        m.level(l)?;
    }
    let height = wall_state(m, m.wall(wall)?)?.height;
    let w = m.wall_mut(wall)?;
    match level {
        None => {
            if w.top_level.is_some() {
                w.unconnected_height = height;
            }
            w.top_level = None;
            w.top_offset = 0.0;
        }
        Some(l) => w.top_level = Some(l),
    }
    if let Some(o) = offset {
        w.top_offset = o;
    }
    check_walls(m, cx, &[wall]);
    Ok(())
}

// ------------------------------------------------------------------ join snapping and following
//
// OBSERVED: the reference does not only COMPUTE joins, it MOVES location curves to make them. Two behaviours, both new.

/// Where the infinite lines `a0-a1` and `b0-b1` meet, or `None` when they are parallel.
fn line_intersection(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2) -> Option<Vec2> {
    let d1 = a1 - a0;
    let d2 = b1 - b0;
    let denom = d1.cross(d2);
    if denom.abs() < 1e-12 {
        return None;
    }
    let t = (b0 - a0).cross(d2) / denom;
    Some(a0 + d1 * t)
}

/// Snap one end of `wall` onto the location line of a wall it lands on.
///
/// OBSERVED: a stem created at `(3, 0.1)` against a host whose centreline is `y = 0` - its end just
/// touching the host's FACE - reads back with its location start at `(3, 0)` and length 4.000, not
/// 3.900. The end is moved ALONG ITS OWN AXIS onto the host's location line. The same happened
/// when the end was set to `y = -0.1` (inside the far face) and `y = -0.5` (0.4 BEYOND it): the
/// reference still reports `(3, 0)`.
///
/// Two triggers reproduce all three:
///
/// * a **fresh** end lands within the host's half width of its location line, on its interior;
/// * an end that was ALREADY on the host's location line (a joined T) and whose curve still
///   crosses it after the edit. That is how `y = -0.5` is snapped although it is far outside the
///   host's body: the join persists while the curves still cross.
///
/// ASSUMED beyond those rows: the half-width threshold for a fresh end, which the probe only
/// brackets at exactly 0.1 (the face).
fn snap_end_to_host(m: &mut BimModel, wall: ElementId, end: usize, was_on_host: Option<ElementId>) {
    let tol = asterion_geom::Tolerances::default();
    let Ok(w) = m.wall(wall) else { return };
    let (p, other) = if end == 0 { (w.start, w.end) } else { (w.end, w.start) };
    let mut target: Option<Vec2> = None;
    // only a wall whose line passes within its half width of the end can take it (a fresh end), or the host it was
    // already on; candidates stay in ascending id because the FIRST one that takes the end wins
    let reach = m.wall_types.values().map(|t| t.structure.width() / 2.0).fold(0.0, f64::max) + 1e-6;
    let mut near: Vec<&Wall> = crate::joins::walls_in_box(m, Vec2::new(p.x - reach, p.y - reach), Vec2::new(p.x + reach, p.y + reach));
    if let Some(h) = was_on_host {
        if !near.iter().any(|q| q.id == h) {
            if let Some(q) = m.walls.get(&h) {
                near.push(q);
                near.sort_by_key(|q| q.id.0);
            }
        }
    }
    for q in near {
        if q.id == wall {
            continue;
        }
        let Some(dir) = (q.end - q.start).normalized() else { continue };
        let len = (q.end - q.start).length();
        let hw = m.wall_types.get(&q.type_id).map(|t| t.structure.width() / 2.0).unwrap_or(0.0);
        let t = (p - q.start).dot(dir);
        let off = (p - q.start).cross(dir).abs();
        let interior = t > tol.vertex && t < len - tol.vertex;
        let fresh = interior && off <= hw + 1e-9;
        let persisted = was_on_host == Some(q.id);
        if !(fresh || persisted) {
            continue;
        }
        // Not parallel to the host, or there is nothing to snap along.
        let Some(x) = line_intersection(p, other, q.start, q.end) else { continue };
        let tx = (x - q.start).dot(dir);
        if tx <= tol.vertex || tx >= len - tol.vertex {
            continue; // would land on the host's own end: that is a corner, not a T
        }
        // The axis has to run INTO the host from the free side: the intersection must not lie
        // beyond the wall's other end.
        if (x - other).dot(p - other) < 0.0 {
            continue;
        }
        target = Some(x);
        break;
    }
    if let Some(x) = target {
        if let Ok(w) = m.wall_mut(wall) {
            if end == 0 {
                w.start = x;
            } else {
                w.end = x;
            }
        }
    }
}

/// The wall whose location line the given point lies on (interior), if any.
fn host_under(m: &BimModel, wall: ElementId, p: Vec2) -> Option<ElementId> {
    let tol = asterion_geom::Tolerances::default();
    crate::joins::walls_near(m, p)
        .into_iter()
        .filter(|q| q.id != wall)
        .find(|q| {
            let Some(dir) = (q.end - q.start).normalized() else { return false };
            let len = (q.end - q.start).length();
            let t = (p - q.start).dot(dir);
            let off = (p - q.start).cross(dir).abs();
            t > tol.vertex && t < len - tol.vertex && off <= tol.vertex
        })
        .map(|q| q.id)
}

/// [`set_wall_curve`] and [`set_wall_arc`]: `through` is the new arc point (`None` = a line).
fn set_wall_curve_with(m: &mut BimModel, cx: &mut TxnContext<'_>, wall: ElementId, start: Vec2, end: Vec2, through: Option<Vec2>) -> Result<(), Failure> {
    crate::groups::guard(m, cx, &[wall]);
    check_curve(start, end)?;
    let (old_start, old_end) = {
        let w = m.wall(wall)?;
        (w.start, w.end)
    };
    // Which host each end was joined to BEFORE the edit: a joined T persists while the curves cross.
    let was_on = [host_under(m, wall, old_start), host_under(m, wall, old_end)];
    // The rectangular openings of this wall are kept by NORMALISED curve parameter, so their stations are rescaled when the
    // curve length changes (below, once the curve is final).
    let old_len = curve_length(m, wall);
    let old_radius = {
        let w = m.wall(wall)?;
        w.through.and_then(|t| crate::regen::arc_from_points(w.start, w.end, t)).map(|a| a.radius)
    };
    let w = m.wall_mut(wall)?;
    w.start = start;
    w.end = end;
    w.through = through;
    // OBSERVED (dg: the host arc replaced by a bigger one): the host parameter of an instance on an ARC is the raw
    // parameter of the arc, an ANGLE, so it stays the same angle on the new arc; the engine's station is a length, rescaled
    // by the ratio of the radii
    if let (Some(r0), Some(r1)) = (old_radius, through.and_then(|t| crate::regen::arc_from_points(start, end, t)).map(|a| a.radius)) {
        for i in m.hosted_instances.values_mut().filter(|i| i.host == wall) {
            i.host_station *= r1 / r0;
        }
    }
    // OBSERVED (j07/j08): a wall joined at a corner DRAGS its partner's end with it. Moving the corner wall 0.2 inside the
    // host shortened the host to 9.8 (location end, length parameter, area and bounding box all followed), and pushing it
    // 0.2 beyond the end lengthened the host to 10.2. The partner's end moves to the NEW point of the end that moved.
    // ASSUMED: only when the join is allowed at both ends, and only for coincident corner ends - a stem dragged by its host
    // was not probed.
    for (old, new, end_idx) in [(old_start, start, 0usize), (old_end, end, 1usize)] {
        if (old - new).length() <= 1e-9 {
            continue;
        }
        let allowed_here = m.wall(wall).map(|w| w.join_allowed[end_idx]).unwrap_or(false);
        if !allowed_here {
            continue;
        }
        // UNKNOWN: the rule says the join at a control is SEPARATED when the squared displacement of the control (projected
        // on the plane perpendicular to the driver normal) exceeds `(width / 2)^2 - 1e-9`, and kept (the neighbours
        // re-trimmed by regeneration) otherwise. The rule does not explain that observation. The engine keeps the observed
        // drag, unchanged, until those are read.
        let partners: Vec<(ElementId, usize)> = crate::joins::walls_near(m, old)
            .into_iter()
            .filter(|q| q.id != wall)
            .flat_map(|q| {
                let mut v = Vec::new();
                if (q.start - old).length() <= 1e-9 && q.join_allowed[0] {
                    v.push((q.id, 0usize));
                }
                if (q.end - old).length() <= 1e-9 && q.join_allowed[1] {
                    v.push((q.id, 1usize));
                }
                v
            })
            .collect();
        for (qid, qend) in partners {
            let partner_old_len = curve_length(m, qid);
            if let Ok(q) = m.wall_mut(qid) {
                if qend == 0 {
                    q.start = new;
                } else {
                    q.end = new;
                }
            }
            rescale_openings(m, qid, partner_old_len);
            check_walls(m, cx, &[qid]);
        }
    }
    // OBSERVED: an end that lands on a host snaps onto the host's location line.
    for end_idx in 0..2 {
        snap_end_to_host(m, wall, end_idx, was_on[end_idx]);
    }
    // The curve of this wall is final: carry its openings along it.
    rescale_openings(m, wall, old_len);
    // Spec W-10b: editing the host curve (location edit, end drag, move) neither re-hosts the instances it carries nor
    // recomputes their station, and no station is clamped: each hosted instance keeps its stored station and travels with
    // the new curve.
    //
    // Spec W-10g: an instance that no longer cuts its host wall is not refused at the edit call: the cut step of the
    // instance posts the instance-not-cutting-anything failure for the commit (the step itself does not fail), see
    // [`report_inserts_left_behind`].
    report_inserts_left_behind(m, cx, wall);
    // UNKNOWN: whether the stored station is a raw curve parameter or a normalised one. The engine keeps it as a length
    // from the curve start and does not rescale it when the curve length changes; Spec W-10b says the instance "follows the
    // new curve at the same stored station", which is what that does for a translation or a rotation. UNKNOWN: the z
    // reference of an opening corner ("curve z") and which edits re-run the corner-to-parameter derivation; the engine
    // keeps the absolute elevation.
    check_walls(m, cx, &[wall]);
    Ok(())
}

/// Commit-time failure of a host-curve change that leaves a hosted instance not cutting its host.
///
/// Spec W-10g: a door or window instance that no longer cuts its host wall is refused by the cut step of the instance on
/// the wall, which posts the instance-not-cutting-anything failure. The step does not fail: the failure is posted for the
/// commit (`cx.report`, never `Err` at the edit call), as an error (never a warning), naming the instance as the failing
/// element and carrying a "delete the instance" resolution. The edit itself is applied; the instance keeps its host and its
/// stored station (Spec W-10b).
///
/// The severity is [`asterion_core::Severity::Error`]: the failure definition is registered with the severity literal 2
/// (Spec W-10g) and literal 2 of the failure severity enumeration is Error; it is posted with the error call, never as a
/// warning. With a posted error and no failures processor registered the commit rolls back, which is the engine's outcome.
/// UNKNOWN: what a registered failures processor does with the pending error (the default processor's module was not
/// opened). TWO failing elements, the host (argument 6) and the instance (argument 5), in that order; one "delete the
/// instance" resolution built on the instance. So the failure lists the host wall AND the instance (the earlier reading of
/// Spec W-10g, "names only the instance", is superseded by that rule).
fn report_inserts_left_behind(m: &BimModel, cx: &mut TxnContext<'_>, wall: ElementId) {
    let Ok(w) = m.wall(wall) else { return };
    let new_length = (w.end - w.start).length();
    for i in m.hosted_instances.values().filter(|i| i.host == wall) {
        let Ok(t) = m.hosted_type(i.type_id) else { continue };
        let half = 0.5 * i.width_in(t);
        if i.host_station - half < 0.0 || i.host_station + half > new_length {
            cx.report(
                Failure::error(
                    codes::INSERT_NOT_CUTTING_HOST,
                    "A hosted instance is not cutting anything in its host wall after the edit. Resolution: delete the instance.",
                )
                .on(&[wall, i.id]),
            );
        }
    }
}

/// Length of the location curve of `wall`, or `None` when it cannot be evaluated.
fn curve_length(m: &BimModel, wall: ElementId) -> Option<f64> {
    let w = m.walls.get(&wall)?;
    wall_state(m, w).ok().map(|s| s.length)
}

/// A rectangular wall opening is attached to its host by two NORMALISED parameters of the host path curve (plus vertical
/// offsets), and rebuilds each corner as the curve evaluated at the stored parameter. When the host curve is edited the
/// corners are therefore re-evaluated on the new curve at the same normalised stations: they travel with the curve and
/// scale with its length. The engine stores stations as lengths from the curve start, so a change of curve length from
/// `old_len` rescales `u_min` / `u_max` by `new_len / old_len`.
///
/// Applied to the two parametrisations the engine has (a line and a circular arc), whose normalised parameter is linear in
/// length.
fn rescale_openings(m: &mut BimModel, wall: ElementId, old_len: Option<f64>) {
    let (Some(old), Some(new)) = (old_len, curve_length(m, wall)) else { return };
    if !(old > 0.0 && new > 0.0) || (new - old).abs() <= 1e-9 {
        return;
    }
    let k = new / old;
    for o in m.wall_openings.values_mut().filter(|o| o.host == wall) {
        o.u_min *= k;
        o.u_max *= k;
    }
}

pub fn set_join_allowed(m: &mut BimModel, cx: &mut TxnContext<'_>, wall: ElementId, end: usize, allowed: bool) -> Result<(), Failure> {
    crate::groups::guard(m, cx, &[wall]);
    if end > 1 {
        return Err(Failure::error(codes::BAD_END_INDEX, "Wall end index must be 0 or 1."));
    }
    m.wall_mut(wall)?.join_allowed[end] = allowed;
    Ok(())
}

/// Store the join type at a wall end (the reference product's `LocationCurve.JoinType[end]`).
///
/// The stored value is one of 0 butt / 1 miter / 2 square-off; the join is built with the type read from its FIRST wall.
/// what the reference product refuses and what each type does to the body are measured there, not decided here.
pub fn set_join_type(m: &mut BimModel, cx: &mut TxnContext<'_>, wall: ElementId, end: usize, kind: crate::model::WallJoinKind) -> Result<(), Failure> {
    crate::groups::guard(m, cx, &[wall]);
    if end > 1 {
        return Err(Failure::error(codes::BAD_END_INDEX, "Wall end index must be 0 or 1."));
    }
    let w = m.wall(wall)?.clone();
    // So the walls whose END is at the point all carry it (OBSERVED: the type set on one wall of a corner reads back on
    // both; the T host keeps Abut; a free end stays Abut).
    let tol = asterion_geom::Tolerances::default().vertex;
    let p = if end == 0 { w.start } else { w.end };
    let mut at_end: Vec<(ElementId, usize)> = vec![(wall, end)];
    let mut joined = false;
    for q in crate::joins::walls_near(m, p).into_iter().filter(|q| q.id != wall) {
        if (q.start - p).length() <= tol {
            at_end.push((q.id, 0));
            joined = true;
        } else if (q.end - p).length() <= tol {
            at_end.push((q.id, 1));
            joined = true;
        } else if crate::joins::point_on_wall_interior(q, p, tol) {
            joined = true;
        }
    }
    for (id, e) in &at_end {
        m.wall_join_types.retain(|j| !(j.wall == *id && usize::from(j.end) == *e));
    }
    if joined && kind != crate::model::WallJoinKind::Abut {
        for (id, e) in at_end {
            m.wall_join_types.push(crate::model::WallJoinTypeAtEnd { wall: id, end: e as u8, kind });
        }
    }
    Ok(())
}

/// Replace the location curve of an existing wall by the ARC through `start`, `through` and `end` (`LocationCurve.Curve =
/// Arc.Create(...)`). The ends are carried exactly as [`set_wall_curve`] carries them (partners dragged, openings rescaled,
/// hosted instances kept by station: Spec W-10b); the arc itself is ASTERION's own three-point storage.
pub fn set_wall_arc(m: &mut BimModel, cx: &mut TxnContext<'_>, wall: ElementId, start: Vec2, end: Vec2, through: Vec2) -> Result<(), Failure> {
    if crate::regen::arc_from_points(start, end, through).is_none() {
        return Err(Failure::error(codes::INVALID_ARC, "Cannot create an arc from three collinear points."));
    }
    // the openings are rescaled against the FINAL curve length: set the arc before the curve edit
    // recomputes the lengths (`set_wall_curve` measures the old length first, on the old arc)
    set_wall_curve_with(m, cx, wall, start, end, Some(through))
}

pub fn set_wall_curve(m: &mut BimModel, cx: &mut TxnContext<'_>, wall: ElementId, start: Vec2, end: Vec2) -> Result<(), Failure> {
    set_wall_curve_with(m, cx, wall, start, end, None)
}

/// Delete an element and everything that cannot exist without it; returns all
/// deleted ids in ascending order.
///
/// Walls whose top is attached to it become unconnected with their current height and keep their stored top offset;
/// deleting a wall type deletes its walls.
///
/// ASSUMED: deleting a host wall deletes **its openings with it**, and their ids appear in the returned deleted set. The
/// reference documents that `Document.Delete` removes "the element and any elements that are totally dependent upon that
/// element" and returns the deleted id set; that a wall opening counts as totally dependent is the assumption, measured by
/// scenario step `after-delete-host-wall`. The cascade is transitive: deleting a level or a wall type deletes its walls,
/// hence their openings too. Deleting an opening alone leaves the host standing, with its Area and Volume back to gross.
/// Pin (`pinned = true`) or unpin elements (`Element.Pinned`). One transaction.
///
/// OBSERVED: walls, a level, a VIEW, a wall TYPE and a door family SYMBOL take `Pinned = true` and read it back (no error);
/// a pinned level still takes a new elevation. The engine pins walls, floors, grids, levels, views, rooms, room
/// separations, wall openings, hosted instances, stairs and the types of walls, floors, doors and windows. Pinning an
/// element that is already pinned changes nothing. UNKNOWN: that special category and the group-member rule: the engine
/// refuses what it has no collection for.
pub fn set_pinned(m: &mut BimModel, _cx: &mut TxnContext<'_>, ids: &[ElementId], pinned: bool) -> Result<(), Failure> {
    for id in ids {
        let can = m.walls.contains_key(id)
            || m.floors.contains_key(id)
            || m.grids.contains_key(id)
            || m.levels.contains_key(id)
            || m.views.contains_key(id)
            || m.wall_types.contains_key(id)
            || m.floor_types.contains_key(id)
            || m.hosted_types.contains_key(id)
            || m.rooms.contains_key(id)
            || m.room_separations.contains_key(id)
            || m.wall_openings.contains_key(id)
            || m.hosted_instances.contains_key(id)
            || m.columns.contains_key(id)
            || m.column_types.contains_key(id)
            || m.stairs.stairs.contains_key(id)
            || m.annot.ref_planes.contains_key(id)
            || m.annot.curves.contains_key(id)
            || m.annot.notes.contains_key(id)
            || m.annot.dims.contains_key(id)
            || m.annot.tags.contains_key(id)
            || m.annot.groups.contains_key(id)
            || m.annot.sweeps.contains_key(id);
        if !can {
            return Err(Failure::error(codes::NOT_FOUND, "The element cannot be pinned or unpinned.").on(&[*id]));
        }
    }
    for id in ids {
        if pinned {
            m.pinned.insert(*id);
        } else {
            m.pinned.remove(id);
        }
    }
    Ok(())
}

pub fn delete(m: &mut BimModel, cx: &mut TxnContext<'_>, id: ElementId) -> Result<Vec<ElementId>, Failure> {
    let result = delete_inner(m, id).map(|mut gone| {
        // OBSERVED: the openings of a deleted floor / roof go with it (`crate::shafts`)
        gone.extend(crate::shafts::remove_orphans(m));
        m.wall_profiles.retain(|w, _| m.walls.contains_key(w));
        m.rafter_cuts.retain(|r, _| m.floors.contains_key(r));
        m.slab_shapes.retain(|r, _| m.floors.contains_key(r));
        m.slab_variable_layers.retain(|t, _| m.floor_types.contains_key(t));
        gone
    });
    // OBSERVED: deleting a pinned wall commits with NO failure; a deleted element leaves the pinned set
    let _ = &cx;
    if let Ok(gone) = &result {
        for g in gone {
            m.pinned.remove(g);
        }
    }
    // A relation cannot outlive either of its elements.
    let alive = |m: &BimModel, e: ElementId| m.walls.contains_key(&e) || m.floors.contains_key(&e);
    let snapshot = m.geometry_joins.clone();
    m.geometry_joins = snapshot.into_iter().filter(|j| alive(m, j.a) && alive(m, j.b)).collect();
    let kept: Vec<_> = m.cross_overrides.iter().copied().filter(|o| m.walls.contains_key(&o.a) && m.walls.contains_key(&o.b)).collect();
    m.cross_overrides = kept;
    // the references of dimensions, the tags and the group memberships of what was deleted
    let result = result.map(|mut gone| {
        let listed = gone.clone();
        for g in &listed {
            gone.extend(crate::dims::element_deleted(m, *g));
            for grp in m.annot.groups.values_mut() {
                grp.members.retain(|x| x != g);
            }
        }
        gone.sort();
        gone.dedup();
        gone
    });
    // the per-view extents of a deleted datum or view go with it (`crate::datum_extents`)
    if let Ok(gone) = &result {
        crate::datum_extents::elements_deleted(m, gone);
        crate::phases::elements_deleted(m, gone);
        crate::rooms_ext::elements_deleted(m, gone);
        crate::base_points::elements_deleted(m, gone);
    }
    // a deleted element is no longer hidden anywhere
    if let Ok(gone) = &result {
        for v in m.views.values_mut() {
            for g in gone {
                v.hidden_elements.remove(g);
            }
        }
    }
    result
}

fn delete_inner(m: &mut BimModel, id: ElementId) -> Result<Vec<ElementId>, Failure> {
    let mut deleted = vec![id];
    if m.levels.contains_key(&id) {
        let based: Vec<ElementId> = m.walls.values().filter(|w| w.base_level == id).map(|w| w.id).collect();
        for w in &based {
            let gone = remove_host_inserts(m, *w);
            m.walls.remove(w);
            deleted.extend(gone);
        }
        deleted.extend(based);
        // columns: those based on the level go with it, those topped by it are left unconnected
        // (ASSUMED as for walls, not measured)
        let based_columns: Vec<ElementId> = m.columns.values().filter(|c| c.base_level == id).map(|c| c.id).collect();
        for c in &based_columns {
            m.columns.remove(c);
        }
        deleted.extend(based_columns);
        for c in m.columns.values_mut().filter(|c| c.top_level == Some(id)) {
            c.top_level = None;
        }
        let topped: Vec<ElementId> = m.walls.values().filter(|w| w.top_level == Some(id)).map(|w| w.id).collect();
        for wid in topped {
            let h = wall_state(m, m.wall(wid)?)?.height;
            let w = m.wall_mut(wid)?;
            w.unconnected_height = h;
            w.top_level = None;
        }
        // ASSUMED: a hosted instance names a level as a constraint it cannot exist without ("in relation to the level on
        // which this instance is placed"), so an instance whose level is deleted goes with it even when its HOST wall
        // survives - which happens whenever the instance's level is not the host's base level.
        let on_level: Vec<ElementId> = m.hosted_instances.values().filter(|i| i.level == id).map(|i| i.id).collect();
        for i in &on_level {
            m.hosted_instances.remove(i);
        }
        deleted.extend(on_level);
        // ASSUMED (not yet measured): floors hosted on the level go with it, as walls do.
        let hosted: Vec<ElementId> = m.floors.values().filter(|f| f.level == id).map(|f| f.id).collect();
        for f in &hosted {
            m.floors.remove(f);
        }
        deleted.extend(hosted);
        // Spec R-05: deleting a roof's CUT-OFF (up-to) level does not delete the roof. The up-to reference is rewritten to
        // the roof's OWN associated (base) level - not to a level below the deleted one - and the cut-off offset is written
        // as `z - elevation(base level)`, where `z` is the z of the start point of the first edge of the base geometry step
        // that matches the history pair (not-a-number, not-a-number), or, when the step or such an edge is missing, the top
        // (maximum z) of the geometry outline MINUS the roof's default thickness (the total width of the layers of the roof
        // type). The engine can only evaluate the second branch, and that is the one applied: `z = top of the roof's solid
        // before the rewrite - type thickness`. So `z` is the previous cut height only when that thickness term is zero.
        // UNKNOWN (Spec R-05; branch 1 edge existence): whether the base step still has an edge with that history pair
        // after the cut-off clip (a face-history question of the geometry boolean) is not modelled, so which branch a given
        // roof takes is not decided. UNKNOWN (Spec R-05): the branch for a roof with no computable outline (the up-to id
        // becomes the constant, whose value was not specified, and the offset 0): the offset 0 is applied and the engine
        // uses the roof's base level for the constant. UNKNOWN (Spec R-05): the deletion of the BASE level of a roof, the
        // same rule for a face roof and the thickness of a curtain roof (a unit-converted constant; the engine has no
        // curtain roof).
        let cut_by: Vec<ElementId> =
            m.floors.values().filter(|s| s.cutoff.map_or(false, |(l, _)| l == id)).map(|s| s.id).collect();
        for sid in cut_by {
            let base = m.floors[&sid].level;
            let base_elevation = m.levels.get(&base).map_or(0.0, |l| l.elevation);
            let z = {
                let s = &m.floors[&sid];
                let thickness = m.floor_types.get(&s.type_id).map(|t| t.thickness);
                crate::regen::roof_outline_top_z(m, s).zip(thickness).map(|(top, t)| top - t)
            };
            let offset = z.map_or(0.0, |z| z - base_elevation);
            m.floors.get_mut(&sid).expect("listed").cutoff = Some((base, offset));
        }
        // ASSUMED (not yet measured): a plan view generated by the level is deleted with it; the reference warns that the
        // views will be deleted before it removes a level.
        let plans: Vec<ElementId> = m.views.values().filter(|v| v.gen_level == Some(id)).map(|v| v.id).collect();
        for v in &plans {
            m.views.remove(v);
        }
        deleted.extend(plans);
        // ASSUMED (not measured): rooms and separation lines live on a level and go with it; a room whose UPPER LIMIT was
        // the deleted level falls back to its own level.
        let rooms: Vec<ElementId> = m.rooms.values().filter(|r| r.level == id).map(|r| r.id).collect();
        for r in &rooms {
            m.rooms.remove(r);
        }
        deleted.extend(rooms);
        let seps: Vec<ElementId> = m.room_separations.values().filter(|s| s.level == id).map(|s| s.id).collect();
        for s in &seps {
            m.room_separations.remove(s);
        }
        deleted.extend(seps);
        for r in m.rooms.values_mut().filter(|r| r.upper_limit == Some(id)) {
            r.upper_limit = None;
        }
        // Spec M-02: a room's upper limit that names an element that is missing or is not a Level reads as the room's own
        // base level - the fall-back written just above.
        m.levels.remove(&id);
        // UNKNOWN (Spec L-01): what deleting a level does to the stairs that name it as base or top level. The Stair parent
        // overrides and the deletion algorithm were not specified, so no stairs is deleted, regenerated or re-referenced
        // here and no failure is posted for one; a stairs left naming the deleted level keeps the id. Spec L-01: a level
        // whose story above is the deleted level reads the story-above parameter as an element that is not a Level, and the
        // validation only clears it.
        for l in m.levels.values_mut().filter(|l| l.story_above == Some(id)) {
            l.story_above = None;
        }
    } else if m.floor_types.contains_key(&id) {
        // OBSERVED: deleting a floor type deletes its floors.
        let instances: Vec<ElementId> = m.floors.values().filter(|f| f.type_id == id).map(|f| f.id).collect();
        for f in &instances {
            m.floors.remove(f);
        }
        deleted.extend(instances);
        m.floor_types.remove(&id);
    } else if m.floors.contains_key(&id) {
        m.floors.remove(&id);
        // OBSERVED: the attachment of a column to a deleted floor goes, the offsets stay
        for c in m.columns.values_mut() {
            if c.top_attachment.map_or(false, |a| a.target == id) {
                c.top_attachment = None;
            }
            if c.base_attachment.map_or(false, |a| a.target == id) {
                c.base_attachment = None;
            }
        }
    } else if m.column_types.contains_key(&id) {
        // a column type's columns are totally dependent on it, as every type's instances are
        let instances: Vec<ElementId> = m.columns.values().filter(|c| c.type_id == id).map(|c| c.id).collect();
        for c in &instances {
            m.columns.remove(c);
        }
        deleted.extend(instances);
        m.column_types.remove(&id);
    } else if m.columns.contains_key(&id) {
        m.columns.remove(&id);
    } else if m.wall_types.contains_key(&id) {
        let instances: Vec<ElementId> = m.walls.values().filter(|w| w.type_id == id).map(|w| w.id).collect();
        for w in &instances {
            let gone = remove_host_inserts(m, *w);
            m.walls.remove(w);
            deleted.extend(gone);
        }
        deleted.extend(instances);
        m.wall_types.remove(&id);
        m.curtain_types.remove(&id);
    } else if m.views.contains_key(&id) {
        m.views.remove(&id);
        // OBSERVED: the callouts of a plan, its viewport and its graphics go with it
        deleted.extend(crate::docviews::delete_doc_view(m, id).into_iter().filter(|g| *g != id));
    } else if m.docs.contains(id) {
        deleted = crate::docs::delete_docs_element(m, id);
    } else if m.grids.contains_key(&id) {
        m.grids.remove(&id);
    } else if m.wall_openings.contains_key(&id) {
        // An opening has no dependents of its own: it is a subtraction, nothing hosts it.
        m.wall_openings.remove(&id);
    } else if m.hosted_types.contains_key(&id) {
        // ASSUMED: a hosted type's instances are totally dependent on it, as a floor type's floors are (OBSERVED) and a
        // wall type's walls are (OBSERVED). The reference's own channel for deleting a loaded family is a warning that the
        // instances will go with it.
        let instances = m.instances_of_hosted_type(id);
        for i in &instances {
            m.hosted_instances.remove(i);
        }
        deleted.extend(instances);
        m.hosted_types.remove(&id);
    } else if m.hosted_instances.contains_key(&id) {
        // An instance has no dependents of its own: like an opening it is a subtraction plus some
        // (unmodelled) family geometry, and nothing is hosted in it.
        m.hosted_instances.remove(&id);
    } else if m.walls.contains_key(&id) {
        deleted.extend(remove_host_inserts(m, id));
        m.walls.remove(&id);
        // OBSERVED: the sweeps and reveals of a wall go with it
        deleted.extend(crate::sweeps::take_for_wall(m, id));
    } else if m.annot.groups.contains_key(&id) {
        // OBSERVED: deleting a group instance deletes its members
        for member in crate::groups::take_for_delete(m, id) {
            if let Ok(gone) = delete_inner(m, member) {
                deleted.extend(gone);
            }
        }
    } else if m.annot.ref_planes.remove(&id).is_some()
        || m.annot.curves.remove(&id).is_some()
        || m.annot.notes.remove(&id).is_some()
        || m.annot.dims.remove(&id).is_some()
        || m.annot.tags.remove(&id).is_some()
        || m.annot.sweeps.remove(&id).is_some()
    {
    } else if m.rooms.contains_key(&id) {
        // A room has no dependents. OBSERVED: deleting a bounding wall leaves the room alive with
        // Area 0, and deleting the room does not move the number cursor back.
        m.rooms.remove(&id);
    } else if m.room_separations.contains_key(&id) {
        m.room_separations.remove(&id);
    } else if crate::shafts::contains(m, id) {
        deleted = crate::shafts::remove_element_and_dependents(m, id);
    } else if m.stairs.stairs.contains_key(&id) {
        // ASSUMED (structural; see `stairs::delete_stairs`): the stairs' runs, landings and railings go with it. UNKNOWN:
        // the reference product's cascade for a stairs.
        deleted = crate::stairs::delete_stairs(m, id)?;
        crate::stairs_scene::prune(m);
    } else {
        return Err(not_found(id));
    }
    deleted.sort();
    Ok(deleted)
}

// ---------------------------------------------------------------- slabs
//
// One set of operations for all three roles.

/// Create a slab type.
///
/// OBSERVED: a thickness of 0 is rejected for a floor. DOCUMENTED (section 1a): a **ceiling** with no compound structure is
/// legal, so zero is accepted for that role only. That asymmetry is the whole reason `role` lives on the type as well as on
/// the instance.
pub fn create_slab_type(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    name: &str,
    thickness: f64,
    role: crate::model::SlabRole,
) -> Result<ElementId, Failure> {
    if !thickness.is_finite() || thickness < 0.0 || (thickness == 0.0 && !role.allows_zero_thickness()) {
        return Err(Failure::error(
            codes::INVALID_THICKNESS,
            "Slab thickness must be greater than 0 for this role.",
        ));
    }
    let id = cx.new_id();
    m.floor_types.insert(
        id,
        crate::model::SlabType {
            id,
            name: name.to_owned(),
            thickness,
            role,
            thickness_axis: role.default_thickness_axis(),
        },
    );
    Ok(id)
}

pub fn set_slab_type_thickness(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    type_id: ElementId,
    thickness: f64,
) -> Result<(), Failure> {
    let role = m.slab_type(type_id)?.role;
    if !thickness.is_finite() || thickness < 0.0 || (thickness == 0.0 && !role.allows_zero_thickness()) {
        return Err(Failure::error(
            codes::INVALID_THICKNESS,
            "Slab thickness must be greater than 0 for this role.",
        ));
    }
    m.floor_types.get_mut(&type_id).expect("checked").thickness = thickness;
    let deps = m.dependents(type_id);
    check_walls(m, cx, &deps);
    Ok(())
}

/// Create a slab from a closed boundary polygon on a level.
///
/// Area is the plan region area, Volume = Area x thickness, Perimeter is the boundary perimeter. A degenerate
/// (millimetre-scale) boundary is accepted by the reference, so ASTERION only requires a closed polygon of at least 3
/// points.
///
/// The role comes from the TYPE, not from a separate argument: a slab of a ceiling type is a
/// ceiling. `anchor` follows from the role (ASSUMED for ceiling and roof, section 2a).
pub fn create_slab(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    type_id: ElementId,
    level: ElementId,
    boundary: Vec<Vec2>,
    height_offset: f64,
    structural: bool,
) -> Result<ElementId, Failure> {
    // (`Floor::create` / `Ceiling::create`): the checks run in this order - the level id must be a Level, the type id a
    // floor / ceiling type, then the boundary must be a valid horizontal boundary (the reference product: non-empty, closed
    // loops that do not intersect each other, planar and parallel to the XY plane, no helical curve); only then is the
    // element made
    m.level(level)?;
    let role = m.slab_type(type_id)?.role;
    if boundary.len() < 3 {
        return Err(Failure::error(codes::INVALID_BOUNDARY, "A slab boundary needs at least 3 points."));
    }
    // OBSERVED: a crossing loop is raised at the CALL, not posted at commit.
    validate_slab_sketch(&boundary, &[])?;
    let id = cx.new_id();
    m.floors.insert(
        id,
        crate::model::Slab {
            id,
            type_id,
            level,
            height_offset,
            boundary,
            structural,
            role,
            anchor: role.default_anchor(),
            room_bounding: true,
            segments: Vec::new(),
            cutoff: None,
            bearing: crate::model::RoofBearing::Rafter,
            slope_arrow: None,
            loops: Vec::new(),
            boundary_arcs: Vec::new(),
            loop_arcs: Vec::new(),
            slope_edge: None,
        },
    );
    Ok(id)
}

/// Create a slab from sketch loops whose edges may be arcs: `boundary` first, then the further loops (holes or islands -
/// the parity of nesting decides).
///
/// A stored slab loop is a list of curves, each a line or an arc; the loops are validated as one sketch (first fault in
/// loop order intersect overlap) - arcs included (`crate::sketch`, "ARC EDGES"). Refusals: fewer points than the loop can
/// close with (three, or two when an arc joins them) and an arc whose three points have no circle (`slab.invalid_arc`,
/// ASTERION's input rule) are raised before the sketch check.
#[allow(clippy::too_many_arguments)]
pub fn create_slab_with_curve_loops(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    type_id: ElementId,
    level: ElementId,
    boundary: crate::model::SlabLoop,
    loops: Vec<crate::model::SlabLoop>,
    height_offset: f64,
    structural: bool,
) -> Result<ElementId, Failure> {
    let role = m.slab_type(type_id)?.role;
    m.level(level)?;
    validate_slab_curve_sketch(&boundary, &loops)?;
    let id = cx.new_id();
    let has = |l: &crate::model::SlabLoop| l.has_arcs();
    let boundary_arcs = if has(&boundary) { aligned_arcs(&boundary) } else { Vec::new() };
    let loop_arcs = if loops.iter().any(has) { loops.iter().map(aligned_arcs).collect() } else { Vec::new() };
    m.floors.insert(
        id,
        crate::model::Slab {
            id,
            type_id,
            level,
            height_offset,
            boundary: boundary.points,
            structural,
            role,
            anchor: role.default_anchor(),
            room_bounding: true,
            segments: Vec::new(),
            cutoff: None,
            bearing: crate::model::RoofBearing::Rafter,
            slope_arrow: None,
            loops: loops.into_iter().map(|l| l.points).collect(),
            boundary_arcs,
            loop_arcs,
            slope_edge: None,
        },
    );
    Ok(id)
}

/// The arcs of a loop padded (or cut) to one entry per edge.
fn aligned_arcs(l: &crate::model::SlabLoop) -> Vec<Option<Vec2>> {
    (0..l.points.len()).map(|i| l.arc_of(i)).collect()
}

/// Validate sketch loops with arc edges: point counts, degenerate arcs, then the sketch check of [`validate_slab_sketch`]
/// on the curves.
pub fn validate_slab_curve_sketch(boundary: &crate::model::SlabLoop, loops: &[crate::model::SlabLoop]) -> Result<(), Failure> {
    for (k, l) in std::iter::once(boundary).chain(loops.iter()).enumerate() {
        if l.points.len() < l.min_points() {
            return Err(Failure::error(
                codes::INVALID_BOUNDARY,
                if k == 0 { "A slab boundary needs at least 3 points (2 when an arc joins them)." } else { "A slab loop needs at least 3 points (2 when an arc joins them)." },
            ));
        }
        if l.arcs.len() > l.points.len() {
            return Err(Failure::error(codes::SLAB_INVALID_ARC, "A slab loop lists more arc edges than it has edges."));
        }
        if let Err(i) = l.curves() {
            return Err(Failure::error(
                codes::SLAB_INVALID_ARC,
                format!("Edge {i} of slab loop {k}: no arc passes through its three points (they are collinear or coincide)."),
            ));
        }
    }
    use crate::sketch::SketchLoop;
    for l in std::iter::once(boundary).chain(loops.iter()) {
        let xy: Vec<f64> = l.points.iter().flat_map(|p| [p.x, p.y]).collect();
        require_numbers("The slab sketch", &xy)?;
    }
    let mut all: Vec<SketchLoop> = vec![SketchLoop::closed_with_arcs(boundary.points.clone(), boundary.arcs.clone())];
    all.extend(loops.iter().map(|l| SketchLoop::closed_with_arcs(l.points.clone(), l.arcs.clone())));
    map_sketch_fault(crate::sketch::first_fault(&all))
}

/// Create a slab with further loops (holes, or islands - the parity of nesting decides).
///
/// OBSERVED: each extra loop is validated like the boundary, and a loop that crosses, touches or shares an edge with ANY
/// other loop is raised at the call.
#[allow(clippy::too_many_arguments)]
pub fn create_slab_with_loops(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    type_id: ElementId,
    level: ElementId,
    boundary: Vec<Vec2>,
    loops: Vec<Vec<Vec2>>,
    height_offset: f64,
    structural: bool,
) -> Result<ElementId, Failure> {
    for l in &loops {
        if l.len() < 3 {
            return Err(Failure::error(codes::INVALID_BOUNDARY, "A slab loop needs at least 3 points."));
        }
    }
    validate_slab_sketch(&boundary, &loops)?;
    let id = create_slab(m, cx, type_id, level, boundary, height_offset, structural)?;
    m.slab_mut(id)?.loops = loops;
    Ok(id)
}

/// Validate the sketch loops of a slab and map the first fault to ASTERION's failure codes. Raised AT THE CALL (OBSERVED).
///
/// The empty sketch keeps its earlier code.
pub fn validate_slab_sketch(boundary: &[Vec2], loops: &[Vec<Vec2>]) -> Result<(), Failure> {
    use crate::sketch::{first_fault, SketchLoop};
    for l in std::iter::once(boundary).chain(loops.iter().map(|l| l.as_slice())) {
        let xy: Vec<f64> = l.iter().flat_map(|p| [p.x, p.y]).collect();
        require_numbers("The slab sketch", &xy)?;
        if l.len() >= 3 && l.iter().all(|p| (*p - l[0]).length() < 1e-9) {
            return Err(Failure::error(codes::INVALID_BOUNDARY, "A slab loop has no extent: all its points coincide."));
        }
    }
    let mut all: Vec<SketchLoop> = vec![SketchLoop::closed(boundary.to_vec())];
    all.extend(loops.iter().map(|l| SketchLoop::closed(l.clone())));
    map_sketch_fault(first_fault(&all))
}

/// The failure a sketch fault maps to, shared by the straight and the curved sketch.
fn map_sketch_fault(fault: Option<crate::sketch::SketchFault>) -> Result<(), Failure> {
    use crate::sketch::SketchFault;
    match fault {
        None => Ok(()),
        Some(SketchFault::NoLoops) => Err(Failure::error(codes::INVALID_BOUNDARY, "The slab sketch has no loops.")),
        Some(SketchFault::LoopsIntersect { a, b, .. }) if a == b => Err(Failure::error(
            codes::SLAB_LOOPS_INTERSECT,
            "A slab loop crosses or touches itself.",
        )),
        Some(SketchFault::LoopsIntersect { .. }) => Err(Failure::error(
            codes::SLAB_LOOPS_INTERSECT,
            "The slab loops cannot compose a valid boundary: two loops intersect.",
        )),
        Some(SketchFault::LinesOverlap { a, b, .. }) if a == b => Err(Failure::error(
            codes::SLAB_LINES_OVERLAP,
            "A slab loop folds back over itself.",
        )),
        Some(SketchFault::LinesOverlap { .. }) => Err(Failure::error(
            codes::SLAB_LINES_OVERLAP,
            "The slab loops cannot compose a valid boundary: two loops share a segment.",
        )),
        Some(SketchFault::OpenLoop { .. }) => {
            Err(Failure::error(codes::SLAB_OPEN_LOOP, "The slab sketch has an open loop."))
        }
    }
}

/// Create a FLOOR with a slope arrow (OBSERVED).
///
/// The arrow runs from `tail` to `head`; the top surface passes through `level + height_offset` at
/// the tail and rises toward the head at `slope`, a rise/run ratio.
#[allow(clippy::too_many_arguments)]
pub fn create_sloped_slab(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    type_id: ElementId,
    level: ElementId,
    boundary: Vec<Vec2>,
    height_offset: f64,
    structural: bool,
    tail: Vec2,
    head: Vec2,
    slope: f64,
) -> Result<ElementId, Failure> {
    if !slope.is_finite() {
        return Err(Failure::error(codes::SLAB_SLOPE_NOT_FINITE, "A slab slope must be a finite number."));
    }
    if (head - tail).length() < 1e-9 {
        return Err(Failure::error(codes::INVALID_BOUNDARY, "The slope arrow has no length."));
    }
    let id = create_slab(m, cx, type_id, level, boundary, height_offset, structural)?;
    m.slab_mut(id)?.slope_arrow = Some(crate::model::SlopeArrow::with_slope(tail, head, slope));
    Ok(id)
}

/// Set the end heights of a slab's slope arrow: `tail` is the start height (`None`: the slab's base height); `head` present
/// gives the arrow by heights, absent by its stored slope. A level that does not resolve falls back to the slab's base
/// height, so it is not refused.
pub fn set_slab_slope_arrow_heights(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    id: ElementId,
    tail: Option<crate::model::ArrowHeight>,
    head: Option<crate::model::ArrowHeight>,
) -> Result<(), Failure> {
    for h in [tail, head].into_iter().flatten() {
        if !h.offset.is_finite() {
            return Err(Failure::error(codes::SLAB_SLOPE_NOT_FINITE, "A slope arrow height must be a finite number."));
        }
    }
    let s = m.slab_mut(id)?;
    let Some(a) = s.slope_arrow.as_mut() else {
        return Err(Failure::error(codes::INVALID_BOUNDARY, "The slab has no slope arrow."));
    };
    a.tail_height = tail;
    a.head_height = head;
    Ok(())
}

pub fn set_slab_height_offset(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    slab: ElementId,
    value: f64,
) -> Result<(), Failure> {
    m.slab_mut(slab)?.height_offset = value;
    check_slabs(m, _cx, &[slab]);
    check_joins(m, _cx, &[slab]);
    Ok(())
}

pub fn set_slab_boundary(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    slab: ElementId,
    boundary: Vec<Vec2>,
) -> Result<(), Failure> {
    let s0 = m.slab(slab)?;
    // Arc edges are index-aligned with the boundary. With the same number of vertices each arc is
    // kept and its through point moves by the mean displacement of its edge's two end points -
    // exact for a rigid move, an ASTERION edit rule otherwise (how the reference edits a sketch
    // arc is not a reading). With another vertex count the arcs cannot be placed and are dropped,
    // like the per-segment slope data below.
    let arcs: Vec<Option<Vec2>> = if s0.boundary_arcs.iter().any(|a| a.is_some()) && s0.boundary.len() == boundary.len() {
        let n = boundary.len();
        (0..n)
            .map(|i| {
                s0.boundary_arcs.get(i).copied().flatten().map(|t| {
                    let j = (i + 1) % n;
                    let d0 = boundary[i] - s0.boundary[i];
                    let d1 = boundary[j] - s0.boundary[j];
                    t + (d0 + d1) * 0.5
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    let new_loop = crate::model::SlabLoop { points: boundary, arcs };
    if new_loop.points.len() < new_loop.min_points() {
        return Err(Failure::error(codes::INVALID_BOUNDARY, "A slab boundary needs at least 3 points."));
    }
    // The whole sketch is checked, so a new boundary may not cross or touch the slab's existing loops either.
    validate_slab_curve_sketch(&new_loop, &s0.extra_loops())?;
    store_boundary(m.slab_mut(slab)?, new_loop);
    Ok(())
}

/// Replace a slab's boundary with a loop whose edges may be arcs (the loops, the type and the offsets are kept). Validated
/// like a new sketch.
pub fn set_slab_boundary_curves(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    slab: ElementId,
    boundary: crate::model::SlabLoop,
) -> Result<(), Failure> {
    let extra = m.slab(slab)?.extra_loops();
    validate_slab_curve_sketch(&boundary, &extra)?;
    store_boundary(m.slab_mut(slab)?, boundary);
    check_slabs(m, cx, &[slab]);
    check_joins(m, cx, &[slab]);
    Ok(())
}

/// Store a validated boundary loop. The per-segment slope data and the slope-defining edge are
/// index-aligned with the boundary, so a boundary of a different length cannot keep them. Dropping
/// them is the only honest option: silently padding would invent flat segments the user never drew.
fn store_boundary(s: &mut crate::model::Slab, l: crate::model::SlabLoop) {
    if s.segments.len() != l.points.len() {
        s.segments.clear();
    }
    if s.boundary.len() != l.points.len() {
        s.slope_edge = None;
    }
    s.boundary_arcs = if l.has_arcs() { aligned_arcs(&l) } else { Vec::new() };
    s.boundary = l.points;
}

/// Set (or clear, `None`) the slope-defining boundary edge of a floor or a ceiling.
///
/// What an arc edge flagged slope-defining does is not specified, so the engine refuses it (`slab.slope_edge_not_line`, an
/// ENGINE CHOICE). The angle is accepted only in (-pi/2, pi/2), i.e. a finite ratio. A roof carries its slopes per segment
/// ([`set_slab_segment_slope`]).
pub fn set_slab_slope_edge(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    slab: ElementId,
    edge: Option<crate::model::SlabSlopeEdge>,
) -> Result<(), Failure> {
    let s = m.slab(slab)?;
    if let Some(e) = edge {
        if s.role == crate::model::SlabRole::Roof {
            return Err(Failure::error(
                codes::SLAB_ROLE_HAS_NO_SLOPE,
                "A roof carries its slopes per boundary segment, not a slope-defining edge.",
            ));
        }
        if !e.slope.is_finite() || !e.height.is_finite() {
            return Err(Failure::error(codes::SLAB_SLOPE_NOT_FINITE, "A slab slope must be a finite number."));
        }
        if e.edge >= s.boundary.len() {
            return Err(Failure::error(codes::SLAB_SEGMENT_OUT_OF_RANGE, "The slab has no boundary edge at that index."));
        }
        if s.boundary_arcs.get(e.edge).copied().flatten().is_some() {
            return Err(Failure::error(codes::SLAB_SLOPE_EDGE_NOT_LINE, "Only a straight boundary edge can define the slope."));
        }
        if s.slope_arrow.is_some() {
            return Err(Failure::error(codes::SLAB_TOO_MANY_SLOPES, "The slab already has a slope arrow: only one slope can be defined."));
        }
    }
    m.slab_mut(slab)?.slope_edge = edge;
    check_slabs(m, cx, &[slab]);
    check_joins(m, cx, &[slab]);
    Ok(())
}

/// Switch the type of a slab, keeping its sketch (boundary and loops), slope data, level and
/// offsets, and re-deriving the body from the new type's total layer width.
///
/// The extrusion depth of a floor (ceiling) is the total layers width of its type, with the loop plane pinned at the top
/// (bottom) elevation; every derived quantity is a pure function of the model here, so keeping the sketch and replacing
/// `type_id` IS the re-derivation.
///
/// Otherwise it touches the element in its document, stores the new type id, and marks the element's type atom touched. It
/// does not touch the sketch, the slab shape editor, joins, the cut-off or the geometry, and it has no refusal path. So
/// this operation stores `type_id` and nothing else: the sketch, the loops, the slope arrow, the segment slopes, the
/// cut-off, the level and the offsets are untouched, and every derived quantity (depth = total layers width of the new
/// type) is recomputed from the model by regeneration. What regeneration then reports (a roof whose cut-off now falls below
/// the new underside, a join that no longer intersects) is the regeneration's outcome, posted at commit - not a refusal of
/// the change.
///
/// ACCEPTANCE is the separate of the element (Spec T-02): for a Floor the candidate must pass the floor symbol test (a
/// footing slab type the footing symbol test); for a Ceiling a compound ceiling type (a soffit-type rule applies to a roof
/// soffit); anything else a generic symbol test. ASTERION models the common outcome of these tests as the ROLE of the type:
/// a type of another role (floor <-> ceiling <-> roof) is refused up front as a wrong kind.
///
/// UNKNOWN: the list of steps the dependency graph re-runs after the type atom is touched (it lives in the reference
/// product / the reference product).
pub fn set_slab_type(m: &mut BimModel, cx: &mut TxnContext<'_>, slab: ElementId, type_id: ElementId) -> Result<(), Failure> {
    let new_role = m.slab_type(type_id)?.role;
    let s = m.slab_mut(slab)?;
    if s.role != new_role {
        return Err(Failure::error(
            codes::WRONG_KIND,
            "The type belongs to another category: a floor, a ceiling and a roof keep their own types.",
        )
        .on(&[slab, type_id]));
    }
    // Spec T-02: the same id is a no-op - no touch, hence no regeneration and no new post.
    if s.type_id == type_id {
        return Ok(());
    }
    s.type_id = type_id;
    // Regeneration consequences of the touched atom (Spec T-02: the dependency graph re-runs the steps that list it; a
    // regeneration recomputes the roof cut-off): posted, not refused.
    check_slabs(m, cx, &[slab]);
    check_joins(m, cx, &[slab]);
    Ok(())
}

fn set_slab_type_of_role(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    slab: ElementId,
    type_id: ElementId,
    role: crate::model::SlabRole,
) -> Result<(), Failure> {
    if m.slab(slab)?.role != role {
        return Err(Failure::error(codes::WRONG_KIND, "The element is not of that category.").on(&[slab]));
    }
    set_slab_type(m, cx, slab, type_id)
}

/// [`set_slab_type`] for a floor.
pub fn set_floor_type(m: &mut BimModel, cx: &mut TxnContext<'_>, floor: ElementId, type_id: ElementId) -> Result<(), Failure> {
    set_slab_type_of_role(m, cx, floor, type_id, crate::model::SlabRole::Floor)
}

/// [`set_slab_type`] for a ceiling.
pub fn set_ceiling_type(m: &mut BimModel, cx: &mut TxnContext<'_>, ceiling: ElementId, type_id: ElementId) -> Result<(), Failure> {
    set_slab_type_of_role(m, cx, ceiling, type_id, crate::model::SlabRole::Ceiling)
}

/// [`set_slab_type`] for a roof.
pub fn set_roof_type(m: &mut BimModel, cx: &mut TxnContext<'_>, roof: ElementId, type_id: ElementId) -> Result<(), Failure> {
    set_slab_type_of_role(m, cx, roof, type_id, crate::model::SlabRole::Roof)
}

/// Translate a slab in plan: the boundary, every further loop and the slope arrow move together by
/// `delta` (metres), so the sketch keeps its shape, its loop nesting and its slope definition.
///
/// A rigid translation cannot create a crossing, so the loops are not revalidated. Per-segment
/// slope data is index-aligned with the boundary and is kept. Level, offsets and cut-off are not
/// moved (they are not in plan). A non-finite `delta` is refused before the change.
pub fn translate_slab(m: &mut BimModel, cx: &mut TxnContext<'_>, slab: ElementId, delta: Vec2) -> Result<(), Failure> {
    if !delta.x.is_finite() || !delta.y.is_finite() {
        return Err(Failure::error(codes::INVALID_BOUNDARY, "A slab translation must be finite."));
    }
    let s = m.slab_mut(slab)?;
    for p in s.boundary.iter_mut() {
        *p = *p + delta;
    }
    for l in s.loops.iter_mut() {
        for p in l.iter_mut() {
            *p = *p + delta;
        }
    }
    // arc through points move with their edges
    for t in s.boundary_arcs.iter_mut().chain(s.loop_arcs.iter_mut().flatten()).flatten() {
        *t = *t + delta;
    }
    if let Some(a) = s.slope_arrow.as_mut() {
        a.tail = a.tail + delta;
        a.head = a.head + delta;
    }
    check_slabs(m, cx, &[slab]);
    check_joins(m, cx, &[slab]);
    Ok(())
}

/// Set one boundary segment's slope.
///
/// ASSUMED (section 8c): a bad index or a non-finite slope is REJECTED before the change, by analogy with
/// `wall.bad_end_index`, which is OBSERVED. The Offset of one boundary segment of a roof (`FootPrintRoof.set_Offset`): its
/// plane starts that far above the roof's base (OBSERVED, see `crate::roof::RoofInput::offsets`).
pub fn set_slab_segment_offset(m: &mut BimModel, _cx: &mut TxnContext<'_>, slab: ElementId, segment_index: usize, offset: f64) -> Result<(), Failure> {
    if !offset.is_finite() {
        return Err(Failure::error(codes::SLAB_SLOPE_NOT_FINITE, "A segment offset must be a finite number."));
    }
    let s = m.slab_mut(slab)?;
    if s.role != crate::model::SlabRole::Roof {
        return Err(Failure::error(codes::SLAB_ROLE_HAS_NO_SLOPE, "Only a roof slab carries per-segment data."));
    }
    let n = s.boundary.len();
    if segment_index >= n {
        return Err(Failure::error(codes::SLAB_SEGMENT_OUT_OF_RANGE, "The slab has no boundary segment at that index."));
    }
    if s.segments.len() != n {
        s.segments = vec![crate::model::SlabSegment::default(); n];
    }
    s.segments[segment_index].offset_from_base = offset;
    Ok(())
}

pub fn set_slab_segment_slope(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    slab: ElementId,
    segment_index: usize,
    defines_slope: bool,
    slope: f64,
) -> Result<(), Failure> {
    if !slope.is_finite() {
        return Err(Failure::error(codes::SLAB_SLOPE_NOT_FINITE, "A slab slope must be a finite number."));
    }
    // OBSERVED: a slope cannot be given to a shape-edited roof (the step is not committed) (refused at the commit, not at
    // the call: the harness answers "not committed")
    if m.slab_shapes.contains_key(&slab) {
        _cx.report(Failure::error(crate::slab_shape::codes::SLOPE_ON_SHAPED, "A shape-edited roof cannot be given a slope.").on(&[slab]));
        return Ok(());
    }
    let s = m.slab_mut(slab)?;
    if s.role != crate::model::SlabRole::Roof {
        return Err(Failure::error(
            codes::SLAB_ROLE_HAS_NO_SLOPE,
            "Only a roof slab carries per-segment slope data.",
        ));
    }
    let n = s.boundary.len();
    if segment_index >= n {
        return Err(Failure::error(
            codes::SLAB_SEGMENT_OUT_OF_RANGE,
            "The slab has no boundary segment at that index.",
        ));
    }
    // How a footprint roof builds a slope on an ARC edge is not specified: ENGINE CHOICE, refused.
    if defines_slope && s.boundary_arcs.get(segment_index).copied().flatten().is_some() {
        return Err(Failure::error(
            codes::SLAB_SLOPE_EDGE_NOT_LINE,
            "Only a straight boundary edge can define the slope.",
        ));
    }
    if s.segments.len() != n {
        s.segments = vec![crate::model::SlabSegment::default(); n];
    }
    s.segments[segment_index].defines_slope = defines_slope;
    s.segments[segment_index].slope = slope;
    // A slope the roof cannot be built with is POSTED at commit, not raised (OBSERVED).
    check_slabs(m, _cx, &[slab]);
    Ok(())
}

/// Truncate a roof above a level. `None` removes the cutoff.
pub fn set_slab_cutoff(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    slab: ElementId,
    cutoff: Option<(ElementId, f64)>,
) -> Result<(), Failure> {
    if let Some((level, offset)) = cutoff {
        m.level(level)?;
        if !offset.is_finite() {
            return Err(Failure::error(codes::SLAB_SLOPE_NOT_FINITE, "A cutoff offset must be finite."));
        }
    }
    let s = m.slab_mut(slab)?;
    if s.role != crate::model::SlabRole::Roof {
        return Err(Failure::error(
            codes::SLAB_ROLE_HAS_NO_SLOPE,
            "Only a roof slab carries a cutoff level.",
        ));
    }
    s.cutoff = cutoff;
    // OBSERVED: a cut-off at or below the underside is refused AT COMMIT.
    check_slabs(m, _cx, &[slab]);
    Ok(())
}

pub fn set_slab_bearing(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    slab: ElementId,
    bearing: crate::model::RoofBearing,
) -> Result<(), Failure> {
    let s = m.slab_mut(slab)?;
    if s.role != crate::model::SlabRole::Roof {
        return Err(Failure::error(
            codes::SLAB_ROLE_HAS_NO_SLOPE,
            "Only a roof slab has a rafter/truss bearing.",
        ));
    }
    s.bearing = bearing;
    Ok(())
}

/// A pure flag: no geometry depends on it in this slice (section 2c).
pub fn set_slab_room_bounding(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    slab: ElementId,
    room_bounding: bool,
) -> Result<(), Failure> {
    m.slab_mut(slab)?.room_bounding = room_bounding;
    Ok(())
}

/// Hide `elements` in `view` ("Hide in View > Elements"); `hide = false` shows them again. One undoable transaction; hiding
/// an element that is already hidden (or showing one that is shown) changes nothing. Refused: a view or an element that
/// does not exist, a view hiding itself, and an element that is not a model element or a datum (types, levels' plan
/// views...): ENGINE CHOICE. The reference stores the id in the view's own set and its write operation refuses to hide in a
/// family document, takes the view's own viewer element to the crop box and sends link instances and stacked walls to a
/// separate branch; which categories its "can be hidden" test accepts is not specified.
pub fn set_hidden_in_view(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    view: ElementId,
    elements: &[ElementId],
    hide: bool,
) -> Result<(), Failure> {
    m.view(view)?;
    for e in elements {
        let hideable = m.walls.contains_key(e)
            || m.floors.contains_key(e)
            || m.grids.contains_key(e)
            || m.levels.contains_key(e)
            || m.hosted_instances.contains_key(e)
            || m.wall_openings.contains_key(e)
            || m.rooms.contains_key(e)
            || m.room_separations.contains_key(e)
            || m.columns.contains_key(e)
            || m.stairs.stairs.contains_key(e)
            || m.stairs.railings.contains_key(e);
        if !hideable {
            return Err(Failure::error(codes::WRONG_KIND, "Only model elements and datums can be hidden in a view.").on(&[*e]));
        }
    }
    let v = m.views.get_mut(&view).expect("checked");
    for e in elements {
        if hide {
            v.hidden_elements.insert(*e);
        } else {
            v.hidden_elements.remove(e);
        }
    }
    Ok(())
}

/// The document's default FLOOR type, got or created (Spec F-06: -> : when the document has none it is CREATED; foundation
/// slabs have a separate default, which the engine does not model).
///
/// UNKNOWN (Spec F-06; which existing type the getter picks and what it creates), questions. ENGINE CHOICE, named: the
/// first floor type by id when one exists, otherwise a new type "Generic 300mm" of 0.3 m.
pub fn default_floor_type(m: &mut BimModel, cx: &mut TxnContext<'_>) -> Result<ElementId, Failure> {
    if let Some(t) = m.floor_types.values().find(|t| t.role == crate::model::SlabRole::Floor) {
        return Ok(t.id);
    }
    create_slab_type(m, cx, "Generic 300mm", 0.3, crate::model::SlabRole::Floor)
}

/// Largest roof slope the engine builds.
///
/// ASSUMED. Round 10 measured only that 0.5 is accepted and 1e6 is refused at commit; the
/// reference's limit lies somewhere between. 1e3 is an angle of 89.94 degrees. A measurement that
/// brackets it (a slope of 10, 100, 1000) replaces this constant.
pub const MAX_ROOF_SLOPE: f64 = 1.0e3;

/// Commit-time validation of slabs: the dependent set of an elevation or slope edit.
///
/// OBSERVED: a roof's cut-off must stay above the lowest point of its underside, and a roof whose slope cannot be built is
/// refused - both POSTED, so the edit that caused them rolls back at commit instead of raising. Every op that can move a
/// roof's underside or cut-off funnels through here, which is how raising a level can be vetoed by the roof that depends on
/// it.
pub fn check_slabs(m: &BimModel, cx: &mut TxnContext<'_>, slabs: &[ElementId]) {
    for &id in slabs {
        let Ok(s) = m.slab(id) else { continue };
        if s.role != crate::model::SlabRole::Roof {
            continue;
        }
        if s.segments.iter().any(|g| g.defines_slope && g.slope.abs() > MAX_ROOF_SLOPE) {
            cx.report(Failure::error(codes::SLAB_SLOPE_DEGENERATE, "Can't make footprint roof.").on(&[id]));
            continue;
        }
        let (Some(cut), Some(lowest)) =
            (crate::regen::roof_cutoff_z(m, s), crate::regen::roof_lowest_bottom_z(m, s))
        else {
            continue;
        };
        if cut <= lowest + 1e-9 {
            cx.report(
                Failure::error(codes::ROOF_CUTOFF_BELOW_BASE, "Cutoff Level cannot be below bottom of roof.")
                    .on(&[id]),
            );
        }
    }
}

/// Instance parameters a slab can be written through (the reference's names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlabParam {
    /// The instance `Slope`. OBSERVED: a rise/run RATIO whatever its name suggests, and writing it sets the slope of every
    /// slope-defining segment.
    Slope,
    BaseOffset,
    /// 0 = rafter, 1 = truss. OBSERVED: committed, no change in quantities or bbox.
    RafterOrTruss,
    /// The same built-in parameter id the wall uses, which a ceiling and a roof share.
    RoomBounding,
}

impl SlabParam {
    pub fn from_name(name: &str) -> Option<SlabParam> {
        match name {
            "slope" => Some(SlabParam::Slope),
            "base_offset" | "height_offset" => Some(SlabParam::BaseOffset),
            "rafter_or_truss" => Some(SlabParam::RafterOrTruss),
            "room_bounding" => Some(SlabParam::RoomBounding),
            _ => None,
        }
    }
}

pub fn set_slab_param(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    slab: ElementId,
    param: SlabParam,
    value: f64,
) -> Result<(), Failure> {
    if !value.is_finite() {
        return Err(Failure::error(codes::SLAB_SLOPE_NOT_FINITE, "A slab parameter must be a finite number."));
    }
    match param {
        SlabParam::Slope => {
            let s = m.slab_mut(slab)?;
            if s.role != crate::model::SlabRole::Roof {
                return Err(Failure::error(codes::SLAB_ROLE_HAS_NO_SLOPE, "Only a roof carries a slope."));
            }
            for seg in s.segments.iter_mut().filter(|g| g.defines_slope) {
                seg.slope = value;
            }
        }
        SlabParam::BaseOffset => m.slab_mut(slab)?.height_offset = value,
        SlabParam::RafterOrTruss => {
            let s = m.slab_mut(slab)?;
            if s.role != crate::model::SlabRole::Roof {
                return Err(Failure::error(codes::SLAB_ROLE_HAS_NO_SLOPE, "Only a roof has a bearing."));
            }
            s.bearing = if value >= 0.5 { crate::model::RoofBearing::Truss } else { crate::model::RoofBearing::Rafter };
        }
        SlabParam::RoomBounding => m.slab_mut(slab)?.room_bounding = value >= 0.5,
    }
    check_slabs(m, cx, &[slab]);
    Ok(())
}

// ------------------------------------------------------------------ Join Geometry (floor - wall)

fn is_floor(m: &BimModel, id: ElementId) -> bool {
    m.floors.get(&id).map_or(false, |f| f.role == crate::model::SlabRole::Floor)
}

fn is_wall(m: &BimModel, id: ElementId) -> bool {
    m.walls.contains_key(&id)
}

/// An element as the cut-order table sees it: its category and the predicates the table asks. Only the kinds this engine
/// models are mapped.
///
/// Neither is reachable through the pairs [`join_elements`] accepts.
fn cut_order_element(m: &BimModel, id: ElementId) -> Option<crate::join_order::CutOrderElement> {
    use crate::join_order::{CutOrderElement, JoinCategory};
    if let Some(w) = m.walls.get(&id) {
        return Some(CutOrderElement { structural_wall: w.structural, ..CutOrderElement::of(JoinCategory::Wall) });
    }
    let s = m.floors.get(&id)?;
    Some(match s.role {
        crate::model::SlabRole::Floor => CutOrderElement::floor(),
        crate::model::SlabRole::Ceiling => CutOrderElement::of(JoinCategory::Ceiling),
        crate::model::SlabRole::Roof => CutOrderElement::of(JoinCategory::Roof),
    })
}

/// Join two elements' geometry: `JoinGeometry(a, b)`, `a` being the FIRST element.
///
/// For the pair this slice models (a floor and a wall) the table says a wall is cut by a floor WHICHEVER order the pair is
/// given in (rule 3, and with no rule matching the first element cuts the second), so the FLOOR is the cutter in both
/// orders - the reversed call, ASSUMED until now (only the floor-first call was measured), is what the rule reads. Only a
/// floor-wall pair is modelled: any other pair is refused with the engine's own code, which is a limit of this slice and
/// not a rule of the reference product. The two walls of `a`, `b` when they cross in an X (the implicit join of
/// `crate::crossings`).
fn crossing_pair(m: &BimModel, a: ElementId, b: ElementId) -> bool {
    match (m.walls.get(&a), m.walls.get(&b)) {
        (Some(x), Some(y)) => crate::crossings::is_crossing_pair(m, x, y),
        _ => false,
    }
}

fn set_cross_state(m: &mut BimModel, a: ElementId, b: ElementId, state: Option<crate::model::CrossState>) {
    m.cross_overrides.retain(|o| !((o.a == a && o.b == b) || (o.a == b && o.b == a)));
    if let Some(state) = state {
        m.cross_overrides.push(crate::model::CrossOverride { a, b, state });
    }
}

pub fn join_elements(m: &mut BimModel, cx: &mut TxnContext<'_>, a: ElementId, b: ElementId) -> Result<(), Failure> {
    // OBSERVED: walls that cross in an X are joined when the later one is made, so `JoinGeometry` on the pair refuses
    // ("already joined"); after `UnjoinGeometry` it joins them again (ENGINE CHOICE: not measured; the default order
    // returns).
    if crossing_pair(m, a, b) {
        return match m.cross_override(a, b) {
            Some(crate::model::CrossState::Unjoined) => {
                set_cross_state(m, a, b, None);
                Ok(())
            }
            _ => Err(Failure::error(codes::JOIN_ALREADY_JOINED, "The elements are already joined.")),
        };
    }
    let pair = (is_floor(m, a) && is_wall(m, b)) || (is_wall(m, a) && is_floor(m, b));
    if !pair {
        if !(m.walls.contains_key(&a) || m.floors.contains_key(&a)) {
            return Err(Failure::error(codes::NOT_FOUND, format!("No element with id {a}.")));
        }
        if !(m.walls.contains_key(&b) || m.floors.contains_key(&b)) {
            return Err(Failure::error(codes::NOT_FOUND, format!("No element with id {b}.")));
        }
        return Err(Failure::error(
            codes::JOIN_UNSUPPORTED_PAIR,
            "Only a floor and a wall can be joined in this slice.",
        ));
    }
    if m.geometry_joins.iter().any(|j| j.matches(a, b)) {
        return Err(Failure::error(codes::JOIN_ALREADY_JOINED, "The elements are already joined."));
    }
    // `pair` above guarantees both elements are mapped.
    let (ea, eb) = (cut_order_element(m, a), cut_order_element(m, b));
    let (Some(ea), Some(eb)) = (ea, eb) else {
        return Err(Failure::error(codes::JOIN_UNSUPPORTED_PAIR, "Only a floor and a wall can be joined in this slice."));
    };
    let order = crate::join_order::resolve_cut_order(true, &ea, &eb);
    // "no cut step" (a thin ceiling, a hosted sweep) stores only the clean-pair steps, which this
    // engine's relation cannot represent; no mapped element sets it, so it is unreachable here.
    let Some((cutter, _cut)) = order.cutter_and_cut(a, b) else {
        return Err(Failure::error(codes::JOIN_UNSUPPORTED_PAIR, "This pair adds no cut step; not modelled."));
    };
    m.geometry_joins.push(crate::model::GeometryJoin { a, b, cutter });
    check_joins(m, cx, &[a, b]);
    Ok(())
}

/// Reverse which of two joined elements cuts the other.
///
/// The reference product removes the cut-pair and clean-pair steps held by both elements and re-adds the cut step on the
/// OTHER element (now naming the former holder), so the cut element and the cutter exchange roles - exactly `cutter := the
/// other element`. It refuses (Err 1) when neither element holds a cut step, which for this engine is "not joined"
/// ([`codes::JOIN_NOT_JOINED`]).
pub fn swap_join_priority(m: &mut BimModel, cx: &mut TxnContext<'_>, a: ElementId, b: ElementId) -> Result<(), Failure> {
    // OBSERVED (step 10): on a crossing pair the cutter and the cut wall exchange
    if crossing_pair(m, a, b) {
        return match m.cross_override(a, b) {
            Some(crate::model::CrossState::Unjoined) => Err(Failure::error(codes::JOIN_NOT_JOINED, "The elements are not joined.")),
            Some(crate::model::CrossState::Swapped) => {
                set_cross_state(m, a, b, None);
                Ok(())
            }
            None => {
                set_cross_state(m, a, b, Some(crate::model::CrossState::Swapped));
                Ok(())
            }
        };
    }
    let j = m
        .geometry_joins
        .iter_mut()
        .find(|j| j.matches(a, b))
        .ok_or_else(|| Failure::error(codes::JOIN_NOT_JOINED, "The elements are not joined."))?;
    j.cutter = j.other(j.cutter);
    check_joins(m, cx, &[a, b]);
    Ok(())
}

pub fn unjoin_elements(m: &mut BimModel, _cx: &mut TxnContext<'_>, a: ElementId, b: ElementId) -> Result<(), Failure> {
    // OBSERVED (step 12): the crossing pair unjoins and neither wall is cut afterwards, whatever is edited next (location
    // line, flip, type)
    if crossing_pair(m, a, b) {
        if m.cross_override(a, b) == Some(crate::model::CrossState::Unjoined) {
            return Err(Failure::error(codes::JOIN_NOT_JOINED, "The elements are not joined."));
        }
        set_cross_state(m, a, b, Some(crate::model::CrossState::Unjoined));
        return Ok(());
    }
    let before = m.geometry_joins.len();
    m.geometry_joins.retain(|j| !j.matches(a, b));
    if m.geometry_joins.len() == before {
        return Err(Failure::error(codes::JOIN_NOT_JOINED, "The elements are not joined."));
    }
    Ok(())
}

/// POST a warning for every join whose elements no longer intersect.
///
/// OBSERVED (step 20): lowering the floor clear of the walls kept the joins and posted "Highlighted elements are joined but
/// do not intersect." once per join - as a WARNING, so the transaction commits. A join is a relation that outlives the
/// overlap that created it.
///
/// Its failing elements are the SET of the two element ids, and it carries one "uncut the pair" resolution. The cut-exists
/// variant of the same type (a different definition, also Warning) is not modelled.
pub fn check_joins(m: &BimModel, cx: &mut TxnContext<'_>, touched: &[ElementId]) {
    for j in &m.geometry_joins {
        if !touched.iter().any(|t| j.involves(*t)) {
            continue;
        }
        let (floor, wall) = if is_floor(m, j.a) { (j.a, j.b) } else { (j.b, j.a) };
        if crate::regen::floor_wall_overlap(m, floor, wall).is_none() {
            cx.report(
                Failure::warning(codes::JOIN_DO_NOT_INTERSECT, "Highlighted elements are joined but do not intersect.")
                    .on(&[floor, wall]),
            );
        }
    }
}

// ------------------------------------------------------------------ rooms

/// Take the next room number and move the cursor.
///
/// OBSERVED: the next number follows the LAST ONE ASSIGNED, not the highest that exists - after a room was renamed 101 and
/// deleted, the next came back as 102.
///
/// Spec M-05 says only where the cursor lives, not how it advances: no operation of the reference product writes a room's
/// Number; the UI placement path copies a Number remembered per category in the document's symbol-id manager (a parameter
/// value, not a counter field of the room) and writes the placed element's value back to it after a placement. UNKNOWN
/// (Spec M-05): the increment that turns the remembered Number into the next one (101 to 102), what a deleted room's number
/// (the remembered set is held by the symbol-id manager, which only stores and looks it up - no operation of it changes a
/// value - and the numeric-string incrementer has only the new-item-number operations as callers, so the 101 -> 102 step is
/// not a parameter-set operation) does to the cursor, and whether API-created rooms go through that remembered set - the
/// "last assigned + 1, starting at 1" rule here is the harness's observation and nothing else.
pub(crate) fn next_room_number(m: &mut BimModel) -> String {
    let n = m.last_room_number.map_or(1, |v| v + 1);
    m.last_room_number = Some(n);
    n.to_string()
}

/// The name a placed room takes. OBSERVED: `Room`.
pub const DEFAULT_ROOM_NAME: &str = "Room";

/// The number the next placed room will take, without moving the cursor (the same OBSERVED rule as
/// `next_room_number`; what the Room tool's tag preview shows).
pub fn peek_next_room_number(m: &BimModel) -> String {
    m.last_room_number.map_or(1, |v| v + 1).to_string()
}

/// Place a room at a point on a level. It is placed even when the point is not enclosed: its
/// `Area` is then 0 and its boundary empty (OBSERVED, P6), and it becomes enclosed the moment the
/// region closes (P7) with no action on the room.
pub fn create_room(m: &mut BimModel, cx: &mut TxnContext<'_>, level: ElementId, point: Vec2) -> Result<ElementId, Failure> {
    m.level(level)?;
    require_numbers("The room point", &[point.x, point.y])?;
    let id = cx.new_id();
    let number = next_room_number(m);
    m.rooms.insert(
        id,
        crate::model::Room {
            id,
            level,
            point,
            base_offset: 0.0,
            limit_offset: crate::model::DEFAULT_ROOM_LIMIT_OFFSET_M,
            upper_limit: None,
            name: DEFAULT_ROOM_NAME.to_owned(),
            number,
            tagged: false,
        },
    );
    Ok(id)
}

/// Create a room in every enclosed region of a level (`NewRooms2`).
///
/// OBSERVED (P12, partly falsified): raised at the call when the level has no plan view ("Level is not associated to a plan
/// view"); with one it makes a room per enclosed region; and a SECOND call makes more rooms in the same regions - it does
/// not skip occupied ones. The extra rooms are redundant (they report 0), and this op must not "fix" that.
///
/// It is [`auto_place_rooms`].
pub fn create_rooms_in_level(m: &mut BimModel, cx: &mut TxnContext<'_>, level: ElementId) -> Result<Vec<ElementId>, Failure> {
    m.level(level)?;
    if !m.views.values().any(|v| v.gen_level == Some(level)) {
        return Err(Failure::error(codes::ROOM_LEVEL_HAS_NO_PLAN_VIEW, "Level is not associated to a plan view."));
    }
    let regions = crate::regen::enclosed_regions_full_in(m, level, crate::phases::DEFAULT_PHASE);
    let mut ids = Vec::new();
    for (poly, islands) in regions {
        // OBSERVED: the room stands at the middle of the circuit's centre-line extent
        let seed = crate::rooms_ext::auto_point(m, level, crate::phases::DEFAULT_PHASE, &poly, &islands);
        let id = create_room(m, cx, level, seed)?;
        // OBSERVED: `NewRooms2` tags every room it makes (one Room Tag per room in the plan)
        m.rooms.get_mut(&id).expect("just made").tagged = true;
        ids.push(id);
    }
    Ok(ids)
}

/// The batch room creator of the placement command (Spec M-06).
///
/// The level plan has at least one candidate point, one per enclosed circuit ([`codes::ROOM_NO_ENCLOSED_REGION`] and
/// nothing is created otherwise). Per candidate point a room is created only when no placed room already holds the point -
/// a placed room is left untouched, so repeating the call creates nothing at occupied points (idempotent per point, unlike
/// the observed `NewRooms2`: [`create_rooms_in_level`]). the engine never makes an unplaced room, so "no room found or the
/// room found is unplaced" reduces to "no room found".
///
/// The room parameters copied from a template element and the limit offset passed in are not modelled: a new room has the
/// engine defaults.
pub fn auto_place_rooms(m: &mut BimModel, cx: &mut TxnContext<'_>, view: ElementId) -> Result<Vec<ElementId>, Failure> {
    let v = m.view(view)?;
    let Some(level) = v.gen_level.filter(|_| v.kind.is_plan()) else {
        return Err(Failure::error(codes::VIEW_NO_LEVEL, "A plan view must have a generating level.").on(&[view]));
    };
    m.level(level)?;
    let regions = crate::regen::enclosed_regions(m, level);
    if regions.is_empty() {
        return Err(
            Failure::error(codes::ROOM_NO_ENCLOSED_REGION, "There is no enclosed region to place a room in.").on(&[view]),
        );
    }
    let mut ids = Vec::new();
    for poly in regions {
        let seed = crate::regen::region_seed(&poly);
        let held = m.rooms.values().any(|r| r.level == level && crate::regen::point_in_region(r.point, &poly));
        if !held {
            ids.push(create_room(m, cx, level, seed)?);
        }
    }
    if ids.is_empty() {
        cx.report(Failure::warning(codes::ROOM_NONE_CREATED_AUTOMATICALLY, "No room was created automatically.").on(&[view]));
    }
    Ok(ids)
}

/// Instance parameters a room can be written through.
#[derive(Clone, Debug, PartialEq)]
pub enum RoomParam {
    LimitOffset(f64),
    BaseOffset(f64),
    /// `None` reads as the room's own level.
    UpperLimit(Option<ElementId>),
    Name(String),
    Number(String),
    /// Whether the room carries a tag (see `model::Room::tagged`).
    Tagged(bool),
}

/// Spec M-07: `base offset > computation offset + 1e-9`, in feet (internal units).
pub fn room_base_offset_above_computation_height(base_offset_m: f64, computation_offset_m: f64) -> bool {
    base_offset_m * crate::spec::FEET_PER_METRE > computation_offset_m * crate::spec::FEET_PER_METRE + 1e-9
}

pub fn set_room_param(m: &mut BimModel, cx: &mut TxnContext<'_>, room: ElementId, param: RoomParam) -> Result<(), Failure> {
    m.room(room)?;
    match param {
        RoomParam::LimitOffset(v) => m.rooms.get_mut(&room).expect("checked").limit_offset = v,
        RoomParam::BaseOffset(v) => {
            m.rooms.get_mut(&room).expect("checked").base_offset = v;
            // OBSERVED: POSTED at commit, not raised. Spec M-07: the failure is posted by the regeneration step of the room
            // (`RoomElemValidHeightStep`), not by the setter, for a placed room whose base offset is greater than the
            // level's room computation offset plus 1e-9 (feet; both relative to the level elevation), offering the
            // resolution "AdjustLimits"; severity code 2 (error). The engine posts it where the offset is written. UNKNOWN
            // (Spec M-07): the point of the transaction at which regeneration delivers it (commit versus an explicit
            // regenerate), and the writer of the level's computation offset (written per level since round 36:
            // `crate::datum_extents::computation_height`).
            let level = m.rooms[&room].level;
            if room_base_offset_above_computation_height(v, crate::datum_extents::computation_height(m, level)) {
                cx.report(
                    Failure::error(
                        codes::ROOM_BASE_ABOVE_COMPUTATION_HEIGHT,
                        "Room's lower offset is above the Computation Height.",
                    )
                    .on(&[room]),
                );
            }
        }
        RoomParam::UpperLimit(l) => {
            if let Some(l) = l {
                m.level(l)?;
            }
            m.rooms.get_mut(&room).expect("checked").upper_limit = l;
        }
        RoomParam::Name(s) => m.rooms.get_mut(&room).expect("checked").name = s,
        RoomParam::Tagged(t) => m.rooms.get_mut(&room).expect("checked").tagged = t,
        RoomParam::Number(s) => {
            // OBSERVED: a number the user assigns moves the cursor, like a grid name.
            if let Ok(n) = s.trim().parse::<u64>() {
                m.last_room_number = Some(n);
            }
            m.rooms.get_mut(&room).expect("checked").number = s;
        }
    }
    Ok(())
}

/// The project-wide Volume Computations switch. OBSERVED: ON by default; OFF gives `Volume` 0 and
/// leaves `Area` untouched.
pub fn set_volume_computation(m: &mut BimModel, _cx: &mut TxnContext<'_>, enabled: bool) -> Result<(), Failure> {
    m.volume_computation_off = !enabled;
    Ok(())
}

/// A wall's Room Bounding parameter. OBSERVED: a wall that stops bounding stops enclosing; toggling it back restores the
/// room.
pub fn set_wall_room_bounding(m: &mut BimModel, _cx: &mut TxnContext<'_>, wall: ElementId, on: bool) -> Result<(), Failure> {
    m.wall_mut(wall)?.room_bounding = on;
    Ok(())
}

/// A zero-thickness boundary that closes an open region (OBSERVED, P11: a line at x = 6 gave
/// Area 22.42 = (6.0 - 0.1) x 3.8).
pub fn create_room_separation(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    level: ElementId,
    start: Vec2,
    end: Vec2,
) -> Result<ElementId, Failure> {
    m.level(level)?;
    check_curve(start, end)?;
    let id = cx.new_id();
    m.room_separations.insert(id, crate::model::RoomSeparation { id, level, start, end });
    Ok(id)
}

// --- floor-role wrappers, so round 6 stays re-provable unchanged ------------------

pub fn create_floor_type(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    name: &str,
    thickness: f64,
) -> Result<ElementId, Failure> {
    create_slab_type(m, cx, name, thickness, crate::model::SlabRole::Floor)
}

/// The variable layer of a floor type: `layer` is the index of the layer of its structure (-1 = none). OBSERVED: under a
/// shape the bottom of the slab stays flat and the variable layer takes the difference. ENGINE CHOICE: only a floor type,
/// and the layer must exist (a floor type has one layer).
pub fn set_floor_variable_layer(m: &mut BimModel, _cx: &mut TxnContext<'_>, type_id: ElementId, layer: i64) -> Result<(), Failure> {
    let t = m.slab_type(type_id)?;
    if t.role != crate::model::SlabRole::Floor {
        return Err(Failure::error(codes::SLAB_ROLE_HAS_NO_SLOPE, "Only a floor type has a variable layer here.").on(&[type_id]));
    }
    if layer < 0 {
        m.slab_variable_layers.remove(&type_id);
    } else if layer == 0 {
        m.slab_variable_layers.insert(type_id, 0);
    } else {
        return Err(Failure::error(codes::SLAB_SEGMENT_OUT_OF_RANGE, "The structure has no layer with that index.").on(&[type_id]));
    }
    Ok(())
}

pub fn set_floor_type_thickness(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    type_id: ElementId,
    thickness: f64,
) -> Result<(), Failure> {
    set_slab_type_thickness(m, cx, type_id, thickness)
}

pub fn create_floor(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    type_id: ElementId,
    level: ElementId,
    boundary: Vec<Vec2>,
    height_offset: f64,
    structural: bool,
) -> Result<ElementId, Failure> {
    create_slab(m, cx, type_id, level, boundary, height_offset, structural)
}

pub fn set_floor_height_offset(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    floor: ElementId,
    value: f64,
) -> Result<(), Failure> {
    set_slab_height_offset(m, cx, floor, value)
}

pub fn set_floor_boundary(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    floor: ElementId,
    boundary: Vec<Vec2>,
) -> Result<(), Failure> {
    set_slab_boundary(m, cx, floor, boundary)
}

// ---------------------------------------------------------------- views

/// Create a view.
///
/// DOCUMENTED: a plan view is created from a view family type and a level (`ViewPlan.Create`), so a
/// generating level is required for plan families and rejected for the others; the new view starts
/// at the family's default scale and detail level with the crop inactive. ASSUMED: the default
/// range (see [`ViewRange::default`]) and that a duplicate view name is rejected (the reference
/// enforces unique view names).
///
/// Spec V-01: the default plan view range is filled when the plan view is constructed, by view class. For a floor plan of
/// the architectural family: cut 1.2, top 2.3, bottom 0, view depth 0, every one measured from the associated level
/// - what [`ViewRange::default`] holds. UNKNOWN (Spec V-01; open question): for a CEILING plan the same operation reads
///   "top and view depth = the level above the associated level, offset 0; cut and bottom offsets 2.3 above the associated
///   level", but the harness's ceiling plan reports the floor-plan values on the associated level, and runs after the
///   constructor and may replace the range - so the ceiling plan keeps the floor defaults here.
pub fn create_view(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    name: &str,
    kind: ViewKind,
    level: Option<ElementId>,
    scale: Option<u32>,
) -> Result<ElementId, Failure> {
    match (kind.is_plan(), level) {
        (true, None) => {
            return Err(Failure::error(codes::VIEW_NO_LEVEL, "A plan view must be created on a level."));
        }
        (true, Some(l)) => {
            m.level(l)?;
        }
        (false, Some(_)) => {
            return Err(Failure::error(codes::VIEW_NOT_PLAN, "Only plan views are generated by a level."));
        }
        (false, None) => {}
    }
    if view_title_taken(m, kind, name, None) {
        return Err(Failure::error(codes::VIEW_DUPLICATE_NAME, "The name entered is already in use. Enter a unique name."));
    }
    let scale = scale.unwrap_or(crate::spec::DEFAULT_VIEW_SCALE);
    check_scale(scale)?;
    let id = cx.new_id();
    m.views.insert(
        id,
        View {
            id,
            name: name.to_owned(),
            kind,
            scale,
            // The detail level of a new view follows the scale table of the document
            detail_level: crate::views::detail_level_for_scale(scale),
            gen_level: level,
            // Spec V-01 (see the doc comment of this function and [`ViewRange::default`]).
            range: ViewRange::default(),
            crop: None,
            // A new viewer has the crop box inactive, invisible and with no crop size set (the constructor sets none of the
            // flag bits); a view with no other information has right (1,0,0), up (0,1,0) and direction (0,0,1), the origin
            // z of a plan being its level (OBSERVED)
            crop_active: false,
            crop_visible: false,
            // Frozen at creation: see `View::origin_z`.
            origin_z: level.and_then(|l| m.levels.get(&l)).map(|l| l.elevation),
            // ASSUMED defaults: hidden line in a plan, shaded in a 3D view
            visual_style: if kind == ViewKind::ThreeD {
                crate::model::VisualStyle::Shaded
            } else {
                crate::model::VisualStyle::HiddenLine
            },
            display: Default::default(),
            // A new 3D view is oriented to the default eye vector (isometric, orthographic)
            camera: (kind == ViewKind::ThreeD).then(crate::view_camera::ViewCamera::default_isometric),
            hidden_elements: Default::default(),
        },
    );
    Ok(id)
}

/// The Discipline of a view: the value set on the view, else the one the document stored for it, else 1 (Architectural).
pub fn view_discipline(m: &BimModel, view: &View) -> i64 {
    view.display
        .discipline
        .or_else(|| m.settings.view_sources.get(&view.id).map(|s| s.discipline))
        .unwrap_or(1)
}

/// Write one display setting of a view (`crate::view_display`): a token of the setting's vocabulary, a flag, an underlay
/// level or a Discipline value the reference product names. Refused: an unknown key, a token outside the vocabulary, a
/// level that is not a level, an unnamed Discipline value.
pub fn set_view_display(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    view: ElementId,
    key: &str,
    value: crate::view_display::DisplayValue,
) -> Result<(), Failure> {
    use crate::view_display::{discipline_name, vocabulary, DisplayValue};
    m.view(view)?;
    let bad = |why: String| Failure::error(codes::VIEW_BAD_DISPLAY_SETTING, why).on(&[view]);
    if let DisplayValue::Level(Some(l)) = &value {
        m.level(*l)?;
    }
    let d = &mut m.view_mut(view)?.display;
    match (key, value) {
        ("discipline", DisplayValue::Discipline(v)) => {
            if discipline_name(v).is_none() {
                return Err(bad(format!("{v} is not a Discipline value")));
            }
            d.discipline = Some(v);
        }
        ("sun_path", DisplayValue::Flag(f)) => d.sun_path = f,
        ("annotation_crop", DisplayValue::Flag(f)) => d.annotation_crop = f,
        ("underlay_base", DisplayValue::Level(l)) => d.underlay_base = l,
        ("underlay_top", DisplayValue::Level(l)) => d.underlay_top = l,
        (k, DisplayValue::Token(t)) => {
            let Some(vocab) = vocabulary(k) else { return Err(bad(format!("'{k}' is not a display setting of a view"))) };
            if !vocab.contains(&t.as_str()) {
                return Err(bad(format!("'{t}' is not a value of {k}")));
            }
            let slot = match k {
                "display_model" => &mut d.display_model,
                "parts_visibility" => &mut d.parts_visibility,
                "orientation" => &mut d.orientation,
                "wall_join_display" => &mut d.wall_join_display,
                "show_hidden_lines" => &mut d.show_hidden_lines,
                "color_scheme_location" => &mut d.color_scheme_location,
                "underlay_orientation" => &mut d.underlay_orientation,
                _ => &mut d.depth_clipping,
            };
            *slot = t;
        }
        (k, _) => return Err(bad(format!("'{k}' does not take that kind of value"))),
    }
    Ok(())
}

pub fn set_view_scale_denominator(m: &mut BimModel, _cx: &mut TxnContext<'_>, view: ElementId, scale: u32) -> Result<(), Failure> {
    check_scale(scale)?;
    m.view_mut(view)?.scale = scale;
    Ok(())
}

pub fn set_view_visual_style(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    view: ElementId,
    style: crate::model::VisualStyle,
) -> Result<(), Failure> {
    m.view_mut(view)?.visual_style = style;
    Ok(())
}

pub fn set_view_detail_level(m: &mut BimModel, _cx: &mut TxnContext<'_>, view: ElementId, detail: DetailLevel) -> Result<(), Failure> {
    m.view_mut(view)?.detail_level = detail;
    Ok(())
}

/// Move one plane of a plan view's range.
///
/// DOCUMENTED: the reference validates the whole range before applying it
/// (`ViewPlan.CheckPlanViewRangeValidity` / `SetViewRange`), so an ordering violation is
/// **rejected** here rather than reported at commit.
pub fn set_view_range_plane(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    view: ElementId,
    plane: PlanViewPlane,
    value: RangePlane,
) -> Result<(), Failure> {
    set_view_range_planes(m, cx, view, &[(plane, value)])
}

/// Set several planes of a view range and check the ORDER of the result once (the reference product validates the range it
/// is given, not each plane on its own: OBSERVED, a range whose new top is below the old cut is valid together with its new
/// cut).
pub fn set_view_range_planes(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    view: ElementId,
    planes: &[(PlanViewPlane, RangePlane)],
) -> Result<(), Failure> {
    let offsets: Vec<f64> = planes.iter().map(|(_, p)| p.offset).collect();
    require_numbers("The view range", &offsets)?;
    let v = m.view(view)?;
    if !v.kind.is_plan() {
        return Err(Failure::error(codes::VIEW_NOT_PLAN, "Only a plan view has a view range.").on(&[view]));
    }
    let gen = v.gen_level.ok_or_else(|| Failure::error(codes::VIEW_NO_LEVEL, "A plan view must have a generating level.").on(&[view]))?;
    let mut range = v.range;
    for (plane, value) in planes {
        range.set_plane(*plane, *value);
    }
    let mut z = [None; 5];
    for p in PlanViewPlane::ALL {
        z[p as usize] = crate::views::resolve_plane(m, gen, range.plane(p))?;
    }
    if !crate::views::range_valid(z[0], z[1], z[2], z[3]) {
        return Err(Failure::error(codes::VIEW_RANGE_INVALID, "The view range is not valid.").on(&[view]));
    }
    m.view_mut(view)?.range = range;
    Ok(())
}

/// Set (or clear, with `None`) the crop rectangle. DOCUMENTED: the crop region, whether it is
/// active, and whether its outline is visible are three independent properties.
pub fn set_view_crop(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    view: ElementId,
    crop: Option<CropRegion>,
    active: Option<bool>,
    visible: Option<bool>,
) -> Result<(), Failure> {
    if let Some(c) = &crop {
        require_numbers("The crop region", &[c.min.x, c.min.y, c.max.x, c.max.y])?;
    }
    let v = m.view_mut(view)?;
    if crop.is_some() {
        v.crop = crop;
    }
    if let Some(a) = active {
        v.crop_active = a;
    }
    if let Some(vis) = visible {
        v.crop_visible = vis;
    }
    Ok(())
}

/// A view name is refused as a duplicate when the view's formatted TITLE equals, exact case, the title of ANY OTHER view,
/// view template or sheet of the document (one shared name space; the view itself may keep its own name; the store itself,
/// tests nothing); the title is the name formatted with the name format of the view's class. The formats of the classes are
/// not specified: ASSUMED here one format per view kind, so two views collide exactly when they have the same kind and the
/// same name (the corpus' floor plan and ceiling plan "Level 1" stand side by side, which a shared format would forbid).
/// The engine has no view templates.
pub fn view_title_taken(m: &BimModel, kind: ViewKind, name: &str, except: Option<ElementId>) -> bool {
    m.views.values().any(|v| Some(v.id) != except && v.kind == kind && v.name == name)
}

pub fn set_view_name(m: &mut BimModel, _cx: &mut TxnContext<'_>, view: ElementId, name: &str) -> Result<(), Failure> {
    let kind = m.view(view)?.kind;
    if view_title_taken(m, kind, name, Some(view)) {
        return Err(Failure::error(codes::VIEW_DUPLICATE_NAME, "The name entered is already in use. Enter a unique name.").on(&[view]));
    }
    m.view_mut(view)?.name = name.to_owned();
    Ok(())
}

/// DOCUMENTED: the scale is the denominator of 1:scale; 0 and values above the documented maximum
/// are rejected.
fn check_scale(scale: u32) -> Result<(), Failure> {
    if scale == 0 || scale > crate::spec::MAX_VIEW_SCALE {
        return Err(Failure::error(codes::VIEW_BAD_SCALE, "The scale value must be between 1 and 24000."));
    }
    Ok(())
}

// ---------------------------------------------------------------- grids

/// Create a straight grid.
///
/// DOCUMENTED: `Grid.Create(doc, Line)`; the name is unique among grids and, when the caller does
/// not set one, continues the sequence from the last grid created ([`Grid::next_name`]). The same
/// short-curve tolerance that rejects a wall curve applies to a grid line.
/// ASSUMED: a new grid shows a bubble at **both** ends and is vertically unbounded, so it appears
/// in every plan view until its extents are narrowed; round 8 measures both.
pub fn create_grid(m: &mut BimModel, cx: &mut TxnContext<'_>, name: Option<&str>, start: Vec2, end: Vec2) -> Result<ElementId, Failure> {
    check_curve(start, end)?;
    // OBSERVED: the automatic name continues from the last name *assigned*, which is not the same as the last grid
    // *created* - renaming a grid moves the cursor. Measured: with grids `1` and `2`, renaming the first to `A` makes the
    // next automatic name `B`, not `3`. Spec G-01: the cursor is a per-document record keyed by the grid name class and
    // overwritten by every name assigned (create and rename alike). UNKNOWN (Spec G-01): the derivation of the next name
    // from the stored one (`Grid::next_name` is the harness's observation: a trailing number or letter run incremented),
    // the text of the first name (IDS_FIRST_GRID_NAME, "1" here), and whether skips a name that is already taken (the
    // engine refuses it).
    let name = match name {
        Some(n) => n.to_owned(),
        None => Grid::next_name(m.last_grid_name.as_deref()),
    };
    if m.grids.values().any(|g| g.name == name) {
        return Err(Failure::error(codes::GRID_DUPLICATE_NAME, "The name entered is already in use. Enter a unique name."));
    }
    let id = cx.new_id();
    // OBSERVED (five grids) and Spec G-02: a new grid has no bubble flag of its own; the default comes from the grid TYPE
    // (bubble at end 1 false, at end 2 true), so a new grid shows its bubble at the second end only
    // ([`Grid::DEFAULT_BUBBLE`]. UNKNOWN (Spec G-02): the default grid type a project TEMPLATE carries is persisted data -
    // the code default applies only to a document without a default grid type).
    m.grids.insert(id, Grid { id, name: name.clone(), start, end, vertical_extent: None, bubble_in_view: Default::default(), through: None });
    m.last_grid_name = Some(name);
    Ok(id)
}

/// Rename a grid.
///
/// Otherwise the name is stored and the cursor is OVERWRITTEN with the name just assigned (no comparison, no maximum, no
/// read of other grids' names: a rename to a lower number moves the cursor down). UNKNOWN (Spec G-01): the duplicate-name
/// refusal (delegates to a operation that was not specified, so the unique-name rule stays the engine's OBSERVED /
/// DOCUMENTED one), and the record's first-write condition (the cursor is written only when the document already has a
/// stored record).
pub fn rename_grid(m: &mut BimModel, _cx: &mut TxnContext<'_>, grid: ElementId, name: &str) -> Result<(), Failure> {
    if m.grids.values().any(|g| g.id != grid && g.name == name) {
        return Err(Failure::error(codes::GRID_DUPLICATE_NAME, "The name entered is already in use. Enter a unique name.").on(&[grid]));
    }
    if m.grid(grid)?.name == name {
        return Ok(());
    }
    m.grid_mut(grid)?.name = name.to_owned();
    // OBSERVED and Spec G-01: a rename moves the name cursor, so the next automatic name follows the renamed grid rather
    // than the highest existing one.
    m.last_grid_name = Some(name.to_owned());
    Ok(())
}

pub fn set_grid_curve(m: &mut BimModel, _cx: &mut TxnContext<'_>, grid: ElementId, start: Vec2, end: Vec2) -> Result<(), Failure> {
    check_curve(start, end)?;
    let g = m.grid_mut(grid)?;
    // An arc grid keeps its arc: the through point moves by the mean displacement of the two ends
    // (exact for a rigid move; an ASTERION edit rule otherwise, as for slab arcs).
    if let Some(t) = g.through {
        let moved = t + ((start - g.start) + (end - g.end)) * 0.5;
        if asterion_geom::Arc::from_three_points(start, moved, end, crate::model::ARC_COLLINEAR_TOL_M).is_err() {
            return Err(Failure::error(codes::GRID_INVALID_ARC, "No arc passes through the grid's three points."));
        }
        g.through = Some(moved);
    }
    g.start = start;
    g.end = end;
    Ok(())
}

/// Create an ARC grid from its two ends and a point on the arc; the name follows the same rule as [`create_grid`]. Three
/// points without a circle are refused (`grid.invalid_arc`, ASTERION's input rule).
pub fn create_arc_grid(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    name: Option<&str>,
    start: Vec2,
    end: Vec2,
    through: Vec2,
) -> Result<ElementId, Failure> {
    if asterion_geom::Arc::from_three_points(start, through, end, crate::model::ARC_COLLINEAR_TOL_M).is_err() {
        return Err(Failure::error(codes::GRID_INVALID_ARC, "No arc passes through the grid's three points."));
    }
    // The engine stores the arc as the reference product reports it, so `start` / `end` and the bubble ends (Spec G-02: end
    // 1 = the curve end) agree with it. ASSUMED beyond the two samples (the operation that orders an arc grid's ends is not
    // specified).
    let id = create_grid(m, cx, name, end, start)?;
    m.grid_mut(id)?.through = Some(through);
    Ok(id)
}

/// The envelope along the vertical axis becomes `bottom - z0 .. top - z0`, `z0` the surface origin height (0 for the
/// engine's grids, whose surface passes through the level of their line).
pub fn set_grid_vertical_extent(m: &mut BimModel, _cx: &mut TxnContext<'_>, grid: ElementId, bottom: f64, top: f64) -> Result<(), Failure> {
    const MAX_LENGTH_M: f64 = 30000.0 * 0.3048;
    if !(bottom.is_finite() && top.is_finite() && bottom <= top - 1e-9) {
        return Err(Failure::error(codes::GRID_BAD_EXTENT, "The bottom and top ranges are reversed for the extents.").on(&[grid]));
    }
    for (what, v) in [("bottom", bottom), ("top", top)] {
        if v.abs() > MAX_LENGTH_M {
            return Err(Failure::error(codes::GRID_BAD_EXTENT, format!("The given value for {what} must be no more than 30000 feet in absolute value.")).on(&[grid]));
        }
    }
    m.grid_mut(grid)?.vertical_extent = Some((bottom, top));
    Ok(())
}

/// DOCUMENTED: `Grid.ShowBubbleInView` / `HideBubbleInView` per `DatumEnds`. v0.1 keeps one model
/// extent, so bubble visibility is stored per grid rather than per view (recorded limitation).
/// Show or hide a grid's bubble **in one view**.
///
/// OBSERVED: the reference stores this per view, so hiding a bubble in one plan leaves every other plan untouched. ASTERION
/// stored one flag per grid until the measurement showed the lower and upper plans disagreeing about the same grid's End0.
///
/// Spec G-02: the type default is evaluated WITHOUT overrides first; "show" records a per-view override only when that
/// default is hidden, "hide" only when it is shown, and otherwise both override bits of the end are cleared - overrides
/// exist only where they differ from the type default. The engine stores the resulting bubble pair per view, which is the
/// same function of (grid, view, end) while the type default is the constant [`Grid::DEFAULT_BUBBLE`] (no grid types are
/// modelled). UNKNOWN (Spec G-02): which view types count as "extent view types" for the override to apply, and how a grid
/// without a per-view record appears in plan views of other classes.
pub fn set_grid_bubble(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    grid: ElementId,
    view: ElementId,
    end: DatumEnd,
    visible: bool,
) -> Result<(), Failure> {
    if !m.views.contains_key(&view) {
        return Err(Failure::error(codes::NOT_FOUND, "A bubble is shown or hidden in a view.").on(&[view]));
    }
    let g = m.grid_mut(grid)?;
    let mut b = g.bubble_in_view.get(&view).copied().unwrap_or(crate::model::Grid::DEFAULT_BUBBLE);
    b[end.index()] = visible;
    g.bubble_in_view.insert(view, b);
    Ok(())
}

// ---------------------------------------------------------------- wall openings
//
// SPECIFIED, NOT MEASURED: the validation split below is ASTERION's prediction of the reference's two channels, not an
// observation.

/// Create a rectangular opening in a wall from two opposite corners.
///
/// `u0`/`u1` are stations along the host's location curve (metres from its start); `z0`/`z1` are
/// absolute elevations. Corner order carries no information and is normalised.
///
/// Channel split - **rejected before the change**:
/// * the host does not exist, or is not a wall (`insert.host_not_supported`). DOCUMENTED: "The
///   host object must be one that supports insertion of instances otherwise this method will
///   fail", and `NewOpening` documents its own rejection "or if the wall doesn't support
///   rectangular openings".
/// * a non-finite corner (`opening.invalid_boundary`).
/// * a zero or negative extent in either direction (`insert.bad_size`).
///
/// **Reported at commit** (the transaction rolls back) - the three geometric failures, see
/// [`check_inserts`].
///
/// ASTERION splits them so each is reachable and they never both apply: a **non-finite** corner means the two points do not
/// describe a rectangle at all -> `opening.invalid_boundary`; a finite but **zero-extent** rectangle means the cut has no
/// size -> `insert.bad_size`. An inverted-but-finite pair is normalised and commits, because the reference's contract is
/// "two opposite corners". A positive but tiny rectangle commits (section 5: no threshold is invented, the measurement
/// decides).
pub fn add_rectangular_opening(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    host: ElementId,
    u0: f64,
    z0: f64,
    u1: f64,
    z1: f64,
) -> Result<ElementId, Failure> {
    check_host_supports_inserts(m, host)?;
    let (u_min, u_max, z_min, z_max) = check_rect(host, u0, z0, u1, z1)?;
    let id = cx.new_id();
    m.wall_openings.insert(id, WallOpening { id, host, u_min, u_max, z_min, z_max });
    check_inserts(m, cx, &[id]);
    report_opening_does_not_cut(m, cx, id);
    Ok(id)
}

/// The step of a rectangular opening whose boolean succeeded but removed nothing from the host posts a WARNING on the
/// opening ("did cut something" false; a delete-the-opening resolution) and still returns success, so the transaction
/// COMMITS. The geometric condition is [`crate::regen::opening_cuts_something`]: the opening adds no area to what the
/// earlier openings of its host removed (entirely off the wall, nested inside an existing cut, or lying in a band that is
/// already open).
///
/// The harness shows the same warning at exactly those steps in rounds 9, 10 and 12 (a check, not the source: the rule is
/// the rule). It is posted when the opening is created or re-sketched; whether a later edit of the host or of an EARLIER
/// opening re-posts it is UNKNOWN, so no other edit does.
///
/// The does-not-cut-anything outcome is not routed through it; it is the posted-warning object. it is NOT this one and must
/// not be copied here.
fn report_opening_does_not_cut(m: &BimModel, cx: &mut TxnContext<'_>, opening: ElementId) {
    if crate::regen::opening_cuts_something(m, opening) == Some(false) {
        cx.report(
            Failure::warning(
                codes::OPENING_DOES_NOT_CUT_HOST,
                "The rectangular opening does not cut its host. Resolution: delete the opening.",
            )
            .on(&[opening]),
        );
    }
}

/// Move or resize an existing opening, from two opposite corners. Same channels as
/// [`add_rectangular_opening`].
pub fn set_rect_opening(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    opening: ElementId,
    u0: f64,
    z0: f64,
    u1: f64,
    z1: f64,
) -> Result<(), Failure> {
    let host = m.opening(opening)?.host;
    let (u_min, u_max, z_min, z_max) = check_rect(host, u0, z0, u1, z1)?;
    let o = m.opening_mut(opening)?;
    o.u_min = u_min;
    o.u_max = u_max;
    o.z_min = z_min;
    o.z_max = z_max;
    check_inserts(m, cx, &[opening]);
    report_opening_does_not_cut(m, cx, opening);
    Ok(())
}

/// Reject a host that cannot carry an insert.
///
/// ASTERION v0.1 models no slanted or stacked wall, so the only reachable rejection is "not a wall" - which covers naming a
/// level, a floor, a grid, a type or a missing id as the host.
fn check_host_supports_inserts(m: &BimModel, host: ElementId) -> Result<(), Failure> {
    if m.walls.contains_key(&host) {
        return Ok(());
    }
    Err(Failure::error(
        codes::INSERT_HOST_NOT_SUPPORTED,
        "The host element does not support the insertion of instances or openings.",
    )
    .on(&[host]))
}

/// Normalise and validate the two corners. See [`add_rectangular_opening`] for the channel reasoning.
fn check_rect(host: ElementId, u0: f64, z0: f64, u1: f64, z1: f64) -> Result<(f64, f64, f64, f64), Failure> {
    if ![u0, z0, u1, z1].iter().all(|v| v.is_finite()) {
        return Err(Failure::error(
            codes::OPENING_INVALID_BOUNDARY,
            "The two corners given do not describe a rectangular opening boundary.",
        )
        .on(&[host]));
    }
    let (u_min, u_max) = (u0.min(u1), u0.max(u1));
    let (z_min, z_max) = (z0.min(z1), z0.max(z1));
    if !(u_max - u_min > crate::spec::MIN_OPENING_EXTENT_M && z_max - z_min > crate::spec::MIN_OPENING_EXTENT_M) {
        return Err(Failure::error(
            codes::INSERT_BAD_SIZE,
            "The opening must have a positive width and a positive height.",
        )
        .on(&[host]));
    }
    Ok((u_min, u_max, z_min, z_max))
}

/// Post-edit consistency checks for every insert kind; a report here makes the transaction roll
/// back.
///
/// DOCUMENTED (sections 2a, 6c): the three checks below are expressed on the *derived* cut
/// rectangle, so a [`crate::model::HostedInstance`] and a [`WallOpening`] are validated by the
/// same code and the host's quantities cannot diverge between the two element kinds. The
/// reference's own argument for this shape is that it retrieves both through one query with one
/// native binding behind it.
///
/// ASSUMED channel: *argument* problems are rejected up front while *geometric* problems - the cut no longer fits in the
/// host - are commit-time failures. ASTERION reports its own codes:
pub fn check_inserts(m: &BimModel, cx: &mut TxnContext<'_>, elements: &[ElementId]) {
    // A hosted door/window instance whose station is at or beyond an end of the host curve and whose parametric half extent
    // does not reach back into it is NOT refused. The reference product posts a WARNING on the instance and its host
    // (resolution: delete the instance) and the transaction commits. Nothing here changes a quantity: the cut stays clipped
    // to the host, so the fourteen measured areas are untouched.
    //
    // The same instance can be reported twice when several ops of one transaction reach this check; the reference product
    // keeps one posted warning per instance.
    for &id in elements {
        let Some(inst) = m.hosted_instances.get(&id) else { continue };
        let (Ok(host), Ok(ty)) = (m.wall(inst.host), m.hosted_type(inst.type_id)) else { continue };
        let Ok(hs) = wall_state(m, host) else { continue };
        if !crate::spec::insert_overlaps_wall_curve(inst.host_station, inst.width_in(ty) / 2.0, 0.0, hs.length) {
            cx.report(
                Failure::warning(
                    codes::INSERT_OVERLAPS_OR_OUT,
                    "The instance lies outside the host wall curve or overlaps its end. Resolution: delete the instance.",
                )
                .on(&[inst.host, id]),
            );
        }
    }
    // MEASURED (round 9, 2026-10-13): there is NOTHING TO REFUSE here. Three rules this function used to enforce were
    // falsified by the reference, all three in the same direction - the reference ACCEPTS and clips where ASTERION
    // rejected:
    //
    //   * an insert wider than its host:  an opening spanning -1.000 to 7.000 on a 6 m wall
    //     committed and removed 4.600 m2, the part inside the wall;
    //   * an insert leaving the host vertically: 2.800 to 3.400 on a 3 m wall committed and removed
    //     exactly 0.600 x 0.200 = 0.120 m2; a sill of 2.000 pushing the head to 4.100 committed and
    //     left 18.000 - 0.900 = 17.100 m2; shortening the host to 1.500 under a 2.100 head
    //     committed and left 9.000 - 1.350 = 7.650 m2;
    //   * two inserts overlapping: 3.000-3.900 and 3.500-4.400 took 1.890 + 1.050, the UNION, not
    //     the sum - two 1.890 m2 rectangles overlapping removed 2.940 m2, never 3.780.
    //
    // So the geometry is not a constraint to enforce but a quantity to derive: see `crate::cut`, whose `net_face_area`
    // reproduces all fourteen measured areas of the three round-9 scenarios. The one step of that round the reference did
    // not commit - moving a host curve out from under its insert - is a rejection at the operation and lives with it
    // (`report_inserts_left_behind`, Spec W-10g: the failure is posted for the commit, not raised at the call; the severity
    // name is Error (the registered literal 2).
}

/// Remove every insert hosted by `wall` - rectangular openings **and** hosted door/window
/// instances - and return their ids.
///
/// ASSUMED (section 4): both kinds are "totally dependent" on the host in the sense of `Document.Delete`, which removes
/// "the element and any elements that are totally dependent upon that element" and returns the deleted id set. Scenario
/// step `after-delete-host-wall-door-must-go` is what breaks or confirms it for the instance half.
fn remove_host_inserts(m: &mut BimModel, wall: ElementId) -> Vec<ElementId> {
    // a curtain wall's grid goes with the wall
    m.curtain_grids.remove(&wall);
    let mut gone = m.openings_of(wall);
    gone.extend(m.hosted_instances_of(wall));
    for id in &gone {
        m.wall_openings.remove(id);
        m.hosted_instances.remove(id);
    }
    gone
}

/// The largest magnitude, in metres, of a coordinate, an elevation or an offset the engine accepts. ASSUMED (numeric
/// safety): far beyond any building, far below the point where areas and volumes overflow to infinity.
pub const NUMERIC_LIMIT_M: f64 = 1.0e9;

/// Every number must be finite and at most [`NUMERIC_LIMIT_M`] in magnitude, else `input.invalid_number` at the call.
pub(crate) fn require_numbers(what: &str, values: &[f64]) -> Result<(), Failure> {
    if values.iter().all(|v| v.is_finite() && v.abs() <= NUMERIC_LIMIT_M) {
        return Ok(());
    }
    Err(Failure::error(codes::INVALID_NUMBER, format!("{what}: every number must be finite and no larger than 1e9 metres.")))
}

fn check_curve(start: Vec2, end: Vec2) -> Result<(), Failure> {
    require_numbers("The curve", &[start.x, start.y, end.x, end.y])?;
    if (end - start).length() < crate::spec::creation_curve_floor() {
        return Err(Failure::error(codes::CURVE_TOO_SHORT, "Curve length is too small for the reference product's tolerance (as identified by Application.ShortCurveTolerance)."));
    }
    Ok(())
}

/// Post-edit consistency checks; failures make the transaction roll back.
///
/// The height must be `>= 0.1 ft`, INCLUSIVE, compared in feet ([`crate::spec::wall_height_is_valid`]; the boundary
/// observation, 0.03048 m rolled back, is the float value 0.09999999999999999 ft). A wall is too short when NOT `width /
/// 100 < length`, in feet ([`crate::spec::wall_is_too_short`]). A level move that drops a level-constrained top below its
/// base reports the same "top lower than base" failure as the reference.
///
/// That is why every report below is `cx.report(Failure::error(..).on(&[wall]))` and why the transaction rolls back. The
/// guard of that rule is: the wall is not valid (width, driver curve, parameter range, height `>= 0.1 ft`) or is too short,
/// or the geometry regeneration result is an error. UNKNOWN: the producers of that (tag, code) result - which geometric
/// condition yields each code - so the arc rules below (`radius <= half thickness`, the thickened-profile curve too short)
/// are the engine's earlier OBSERVED behaviour and the rule does not decide them; and bodies.
///
/// Post the overlap WARNING for every pair of walls, one of them in `walls`, whose bodies overlap: parallel straight walls
/// on the same base level whose axes are closer than half the sum of their widths and whose extents along the axis overlap.
/// OBSERVED (: by an array, by a wall created over another, by a copy - also by a zero vector - and by a move onto another
/// wall; "Highlighted walls overlap. One of them may be ignored when the reference product finds room boundaries", the
/// element ids of the pair). Arc walls are not tested (not measured). Posted once per pair per transaction.
pub fn report_wall_overlaps(m: &BimModel, cx: &mut TxnContext<'_>, walls: &[ElementId]) {
    let tol = crate::spec::insert_fit_tolerance();
    let width = |w: &Wall| m.wall_types.get(&w.type_id).map_or(0.0, |t| t.structure.width());
    let overlap = |a: &Wall, b: &Wall| -> bool {
        if a.base_level != b.base_level || a.through.is_some() || b.through.is_some() {
            return false;
        }
        let (Some(da), Some(db)) = ((a.end - a.start).normalized(), (b.end - b.start).normalized()) else { return false };
        if da.cross(db).abs() > 1e-6 {
            return false;
        }
        if (b.start - a.start).cross(da).abs() >= (width(a) + width(b)) * 0.5 - tol {
            return false;
        }
        let (b0, b1) = ((b.start - a.start).dot(da), (b.end - a.start).dot(da));
        let (lo, hi) = (b0.min(b1), b0.max(b1));
        hi.min((a.end - a.start).length()) - lo.max(0.0) > tol
    };
    let widest = m.wall_types.values().map(|t| t.structure.width()).fold(0.0, f64::max);
    for id in walls {
        let Some(a) = m.walls.get(id) else { continue };
        for q in crate::joins::walls_around(m, a, widest) {
            if q.id == a.id || !overlap(a, q) {
                continue;
            }
            // a pair of two listed walls is posted from its lower id only
            if walls.contains(&q.id) && q.id.0 < a.id.0 {
                continue;
            }
            let pair = if a.id.0 < q.id.0 { [a.id, q.id] } else { [q.id, a.id] };
            if cx.has_failure(codes::WALL_OVERLAPS_WALL, &pair) {
                continue;
            }
            cx.report(
                Failure::warning(
                    codes::WALL_OVERLAPS_WALL,
                    "Highlighted walls overlap. One of them may be ignored when the reference product finds room boundaries.",
                )
                .on(&pair),
            );
        }
    }
}

pub fn check_walls(m: &BimModel, cx: &mut TxnContext<'_>, walls: &[ElementId]) {
    report_wall_overlaps(m, cx, walls);
    for &id in walls {
        let Ok(w) = m.wall(id) else { continue };
        let s = match wall_state(m, w) {
            Ok(s) => s,
            Err(f) => {
                // Err 9 of the thickening route is a regeneration failure of the wall, posted at commit like the other wall
                // failures (the engine's own code).
                if f.code == codes::WALL_PROFILE_CURVE_TOO_SHORT {
                    cx.report(f.on(&[id]));
                }
                continue;
            }
        };
        if !crate::spec::wall_height_is_valid(s.height) {
            let (code, msg) = if w.top_level.is_some() {
                (codes::TOP_BELOW_BASE, "The top of the Wall is lower than the base of the Wall.")
            } else {
                (codes::BELOW_MIN_HEIGHT, "The wall is below the minimum wall height.")
            };
            cx.report(Failure::error(code, msg).on(&[id]));
        }
        let arc = w.through.and_then(|t| crate::regen::arc_from_points(w.start, w.end, t));
        let layer_widths: Vec<f64> = m
            .wall_type(w.type_id)
            .map(|t| t.structure.layers.iter().map(|l| l.width).collect())
            .unwrap_or_default();
        let too_short = if arc.is_some() {
            crate::spec::wall_is_too_short(&layer_widths, s.length, arc.map(|a| a.radius))
        } else {
            // length from the end points converted to feet, as the reference builds its curve
            let f = crate::spec::FEET_PER_METRE;
            let len_ft = ((w.end.x * f - w.start.x * f).powi(2) + (w.end.y * f - w.start.y * f).powi(2)).sqrt();
            crate::spec::wall_is_too_short_ft(&layer_widths, len_ft, None)
        };
        // The absolute floor is the engine's earlier OBSERVED behaviour (for very thin walls the short-curve tolerance
        // dominates). Same literal as (strict there; equality was never observed, and which operation this floor models is
        // not specified, so the `<=` is kept).
        let below_floor = s.length <= asterion_geom::Tolerances::default().short_curve;
        if too_short || below_floor || !crate::spec::wall_width_is_valid(s.width) {
            cx.report(Failure::error(codes::CANNOT_MAKE_WALL, "Can't make Wall.").on(&[id]));
        }
        // OBSERVED (one sample): an arc whose radius cannot hold the wall - 0.05 against a half thickness of 0.10 - is
        // posted at commit as "Can't make Wall.".
        if let Some(arc) = w.through.and_then(|t| crate::regen::arc_from_points(w.start, w.end, t)) {
            if arc.radius <= s.width / 2.0 {
                cx.report(Failure::error(codes::CANNOT_MAKE_WALL, "Can't make Wall.").on(&[id]));
            }
        }
        // ASSUMED: moving or shortening a host so that one of its inserts no longer fits is a **commit-time** failure, not
        // a silent re-cut. Every op that can move a host already funnels through here, so the openings are revisited with
        // it.
        let inserts: Vec<ElementId> = host_cuts(m, id).into_iter().map(|(e, _, _)| e).collect();
        if !inserts.is_empty() {
            check_inserts(m, cx, &inserts);
        }
    }
}

// ------------------------------------------------- hosted instances: doors and windows
//
// SPECIFIED, NOT MEASURED: every channel decision below is ASTERION's falsifiable prediction, not an observation.
//
// The validation split follows section 4's pattern verbatim: *argument* problems (a missing or
// unsupported host, a missing type or level, a size that is not a size, a write to a read-only
// parameter) are rejected UP FRONT with `Err(Failure)`, because the reference documents them as
// exceptions on the creation overload; *geometric* problems (the cut no longer fits the host) are
// COMMIT-TIME failures through `cx.report`, because the reference posts "Can't cut instance of
// <family> out of wall" after the edit. Routing one of them through the wrong channel makes the
// `status` field diverge even when the geometry matches.

/// Everything [`place_hosted_instance`] needs. Mirrors [`NewWall`] and section 7c's own
/// `NewHostedInstance { type_id, host, level, host_station, sill_height }`.
pub struct NewHostedInstance {
    pub type_id: ElementId,
    /// Must be a wall (section 1a: v0.1 hosts only in a wall).
    pub host: ElementId,
    pub level: ElementId,
    /// Metres along the host's location curve from its start (section 1c).
    pub host_station: f64,
    /// `None` takes the kind's placement default: DOCUMENTED (section 3c) that a window's initial sill is the type's
    /// *Default Sill Height*, ASSUMED that a door's is 0. A scenario that merely places an instance therefore does not have
    /// to invent a sill, which is exactly how step `window-placed-default-sill` is written.
    pub sill_height: Option<f64>,
}

/// Create a door or window **type**.
///
/// Rejections, all up front:
/// * `hosted_type.duplicate_name` - another hosted type already has this name (section 7a).
/// * `insert.bad_size` - `width` or `height` is not strictly positive and finite (section 5:
///   ASTERION rejects non-finite and non-positive outright and accepts anything positive, adopting
///   whatever threshold the measurement shows rather than inventing one), or one of the recorded
///   lengths (`thickness`, `rough_width`, `rough_height`, `inset`) is negative or non-finite, or
///   `default_sill_height` is non-finite.
///
/// ASSUMED (sections 1c, 3b): a **door** type's `inset` is pinned to 0 rather than rejected,
/// because `WINDOW_INSET` is documented for windows only and "the door template has no inset".
#[allow(clippy::too_many_arguments)]
pub fn create_hosted_type(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    name: &str,
    kind: HostedKind,
    width: f64,
    height: f64,
    thickness: f64,
    rough_width: f64,
    rough_height: f64,
    default_sill_height: f64,
    inset: f64,
) -> Result<ElementId, Failure> {
    if m.hosted_type_name_taken(name, None) {
        return Err(Failure::error(
            codes::HOSTED_TYPE_DUPLICATE_NAME,
            format!("A hosted type named {name:?} already exists."),
        ));
    }
    check_cut_extent("width", width)?;
    check_cut_extent("height", height)?;
    for (label, v) in [("thickness", thickness), ("rough width", rough_width), ("rough height", rough_height), ("inset", inset)] {
        check_recorded_length(label, v)?;
    }
    check_finite_length("default sill height", default_sill_height)?;
    let id = cx.new_id();
    m.hosted_types.insert(
        id,
        HostedType {
            id,
            name: name.to_owned(),
            kind,
            width,
            height,
            thickness,
            rough_width,
            rough_height,
            default_sill_height,
            inset: if kind.has_inset() { inset } else { 0.0 },
        },
    );
    // Loading a family makes its first type the document default of its category, replacing the previous default. ASTERION
    // has no family files: creating a door / window type is the engine's load of a one-type family (ASSUMED mapping;
    // whether a type DUPLICATE writes the default is not specified).
    m.settings.default_hosted_types.insert(kind, id);
    Ok(id)
}

/// Give a hosted instance its OWN width and / or height (`Some`), or take it back to its type's
/// (`None`). One transaction; the host's cut, the plan symbol and the fit checks follow.
///
/// ENGINE CHOICE (: where the reference product stores a family instance's size override and how the opening cut takes it
/// is not specified yet): a size must be a finite length above zero.
pub fn set_hosted_instance_size(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    id: ElementId,
    width: Option<f64>,
    height: Option<f64>,
) -> Result<(), Failure> {
    crate::groups::guard(m, cx, &[id]);
    for (label, v) in [("width", width), ("height", height)] {
        if let Some(v) = v {
            if !(v.is_finite() && v > 0.0) {
                return Err(Failure::error(
                    codes::INSERT_BAD_SIZE,
                    format!("The instance {label} must be a positive length, got {v}."),
                )
                .on(&[id]));
            }
        }
    }
    let host = m.hosted_instances.get(&id).ok_or_else(|| Failure::error(codes::NOT_FOUND, format!("Element {} is not a hosted instance.", id.0)).on(&[id]))?.host;
    {
        let i = m.hosted_instances.get_mut(&id).expect("checked");
        i.width_override = width;
        i.height_override = height;
    }
    check_walls(m, cx, &[host]);
    check_inserts(m, cx, &[id, host]);
    Ok(())
}

/// Give a hosted instance its own rectangular CUT LOOP, relative to its station and sill (`Some`),
/// or take it back to the width x height rectangle (`None`). One transaction; the host's cut and
/// the fit checks follow.
///
/// ENGINE CHOICE (: how the reference product stores the cut loop of a family for an instance and how the host's opening
/// takes it is not specified yet). A loop must be finite with `min < max` on both axes.
pub fn set_hosted_instance_cut_loop(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    id: ElementId,
    cut_loop: Option<crate::model::InstanceCutLoop>,
) -> Result<(), Failure> {
    crate::groups::guard(m, cx, &[id]);
    if let Some(c) = cut_loop {
        let ok = [c.u_min, c.u_max, c.z_min, c.z_max].iter().all(|v| v.is_finite()) && c.u_min < c.u_max && c.z_min < c.z_max;
        if !ok {
            return Err(Failure::error(codes::INSERT_BAD_SIZE, "An instance cut loop must be finite with min below max on both axes.").on(&[id]));
        }
    }
    let host = m
        .hosted_instances
        .get(&id)
        .ok_or_else(|| Failure::error(codes::NOT_FOUND, format!("Element {} is not a hosted instance.", id.0)).on(&[id]))?
        .host;
    m.hosted_instances.get_mut(&id).expect("checked").cut_loop = cut_loop;
    check_walls(m, cx, &[host]);
    check_inserts(m, cx, &[id, host]);
    Ok(())
}

/// Duplicate a door / window type (Edit Type > Duplicate): a copy of `source` under `name`.
///
/// The writers of the document default of a category are the family LOAD, the Create Similar preparation and the
/// first-element callbacks; a duplicate is not among them. OBSERVED: after a duplicate the default stays the loaded type.
pub fn duplicate_hosted_type(m: &mut BimModel, cx: &mut TxnContext<'_>, source: ElementId, name: &str) -> Result<ElementId, Failure> {
    let src = m.hosted_type(source)?.clone();
    // The duplicate's name checks, compared with every family type of the class (doors and windows)
    let name = &crate::type_names::check_duplicate_name(name, |n| m.hosted_types.values().any(|t| t.name == n))?;
    let before = m.settings.default_hosted_types.get(&src.kind).copied();
    let id = create_hosted_type(
        m,
        cx,
        name,
        src.kind,
        src.width,
        src.height,
        src.thickness,
        src.rough_width,
        src.rough_height,
        src.default_sill_height,
        src.inset,
    )?;
    match before {
        Some(t) => {
            m.settings.default_hosted_types.insert(src.kind, t);
        }
        None => {
            m.settings.default_hosted_types.remove(&src.kind);
        }
    }
    Ok(id)
}

/// Create Similar preparation: the type of the instance used becomes the document default of its category; returns that
/// type. The command that triggers it is outside the reference product.
pub fn prepare_create_similar(m: &mut BimModel, _cx: &mut TxnContext<'_>, instance: ElementId) -> Result<ElementId, Failure> {
    let h = m
        .hosted_instances
        .get(&instance)
        .ok_or_else(|| Failure::error(codes::NOT_FOUND, format!("Element {} is not a door or window.", instance.0)).on(&[instance]))?;
    let t = h.type_id;
    let kind = m.hosted_type(t)?.kind;
    m.settings.default_hosted_types.insert(kind, t);
    Ok(t)
}

/// Write one type parameter, in ASTERION's own vocabulary (section 8's `set_hosted_type_param`).
///
/// Changing `width` or `height` **re-cuts every host** that carries an instance of this type, so
/// the three geometric commit-time failures are re-evaluated for all of them - that is how a type
/// whose door no longer fits its wall becomes a rolled-back transaction rather than a silently
/// wrong Area.
///
/// Changing `rough_width` / `rough_height` / `thickness` / `inset` re-cuts **nothing**.
///
/// Changing `default_sill_height` does **not** move existing instances either: DOCUMENTED (section
/// 3c) that the type value supplies the *initial* instance sill, not a live constraint.
pub fn set_hosted_type_param(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    type_id: ElementId,
    param: HostedTypeParam,
    value: f64,
) -> Result<(), Failure> {
    let kind = m.hosted_type(type_id)?.kind;
    // OBSERVED (steps 17 and 18): `rough_width` and `rough_height` DO NOT EXIST on a family built from the shipped `Metric
    // Door.rft` / `Metric Window.rft` template - writing either one is an error, raised before the change. the measurement
    // says they are not there at all.
    //
    // Scoped to what was measured: this installation ships no door or window family, so the
    // harness builds one from the template and every round-9 parameter result is for such a
    // family. A library family may well carry them, which is an untested assumption, not a fact.
    if matches!(param, HostedTypeParam::RoughWidth | HostedTypeParam::RoughHeight) {
        return Err(Failure::error(
            codes::PARAM_NOT_PRESENT,
            "The parameter does not exist on this family.",
        )
        .on(&[type_id]));
    }
    if param.must_be_positive() {
        check_cut_extent(param.name(), value)?;
    } else if param == HostedTypeParam::DefaultSillHeight {
        check_finite_length(param.name(), value)?;
    } else {
        check_recorded_length(param.name(), value)?;
    }
    let t = m.hosted_type_mut(type_id)?;
    match param {
        HostedTypeParam::Width => t.width = value,
        HostedTypeParam::Height => t.height = value,
        HostedTypeParam::Thickness => t.thickness = value,
        HostedTypeParam::RoughWidth => t.rough_width = value,
        HostedTypeParam::RoughHeight => t.rough_height = value,
        // ASSUMED (sections 1c, 3b): a door has no inset, so the write is pinned to 0.
        HostedTypeParam::Inset => t.inset = if kind.has_inset() { value } else { 0.0 },
        HostedTypeParam::DefaultSillHeight => t.default_sill_height = value,
    }
    if param.moves_the_cut() {
        let affected = m.instances_of_hosted_type(type_id);
        check_inserts(m, cx, &affected);
    }
    Ok(())
}

/// Section 7c's `set_hosted_type_size`: the two cut-driving parameters at once.
pub fn set_hosted_type_size(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    type_id: ElementId,
    width: f64,
    height: f64,
) -> Result<(), Failure> {
    // Both are validated before either is written, so a rejected height cannot leave a written
    // width behind: the reject channel must not mutate (section 4).
    check_cut_extent("width", width)?;
    check_cut_extent("height", height)?;
    set_hosted_type_param(m, cx, type_id, HostedTypeParam::Width, width)?;
    set_hosted_type_param(m, cx, type_id, HostedTypeParam::Height, height)
}

/// Place a door or window instance in a wall.
///
/// Mirrors the reference's hosted creation overload (point + symbol + host + level, section 1b).
/// Rejections, all up front, each with the documented exception it mirrors:
/// * `insert.host_not_supported` - the host is missing or is not a wall. DOCUMENTED: "The host
///   object must be one that supports insertion of instances otherwise this method will fail",
///   plus the documented "host not in document" exception.
/// * `element.not_found` - the type does not exist (section 7c: "type not found"). The reference's nearest documented
///   exception is "symbol not in document"; the inactive-symbol rejection has no ASTERION counterpart, because ASTERION has
///   no lazily activated symbol geometry - recorded as out of scope, not silently merged into this code.
/// * `insert.no_level` - the level does not exist. DOCUMENTED: "level not in document".
/// * `insert.bad_station` - the station or the sill is not a finite number.
///
/// Then [`check_inserts`] runs at commit. It refuses nothing: a station at or beyond an end of the host curve whose extent
/// does not reach back only posts a warning, and a cut that leaves the host or overlaps another insert is clipped / united.
/// The one commit-time error left on this path is the level rule below, whose basis is UNKNOWN.
///
/// The station is stored as given (no clamp) and the level is the one passed in; a placement point is turned into a station
/// by the caller (the reference product projects it on the host curve).
pub fn place_hosted_instance(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    spec: NewHostedInstance,
) -> Result<ElementId, Failure> {
    check_host_supports_inserts(m, spec.host)?;
    let kind = m.hosted_type(spec.type_id)?.kind;
    let type_default_sill = m.hosted_type(spec.type_id)?.default_sill_height;
    if !m.levels.contains_key(&spec.level) {
        return Err(Failure::error(codes::INSERT_NO_LEVEL, "The level the instance is placed on does not exist.")
            .on(&[spec.level]));
    }
    // OBSERVED (step 32): placing an instance on a level ABOVE its host's base level does not commit. An instance is
    // anchored to its host, and the host's own base level is the only one it can belong to. UNKNOWN: the placement path
    // read there stores the level passed in and refuses nothing, and derives the position from that level's elevation.
    // UNKNOWN (Spec W-10g): why an instance placed on another level stops cutting (the reference product predicates and the
    // extent test were not specified). The rule below is kept as the engine's earlier behaviour. The channel is
    // commit-time, not a refusal: round 9 reported `not_committed` for this step, so the reference created the instance and
    // then rolled the transaction back. The report is raised after the element exists, below.
    let host_level = m.wall(spec.host)?.base_level;
    let wrong_level = spec.level != host_level;
    let sill = spec.sill_height.unwrap_or_else(|| kind.initial_sill_height(type_default_sill));
    check_station("host station", spec.host_station)?;
    check_station("sill height", sill)?;
    let id = cx.new_id();
    m.hosted_instances.insert(
        id,
        HostedInstance {
            id,
            type_id: spec.type_id,
            host: spec.host,
            level: spec.level,
            host_station: spec.host_station,
            sill_height: sill,
            // ASSUMED (section 1d prediction 1): a freshly placed instance is unflipped on both
            // axes, so its facing equals the host's orientation and it is not mirrored.
            facing_flipped: false,
            hand_flipped: false,
            // Section 7f puts the auto-incrementing Mark sequence out of v0.1 scope, so a placed
            // instance starts unmarked rather than guessing the reference's counter.
            mark: String::new(),
            width_override: None,
            height_override: None,
            cut_loop: None,
        },
    );
    if wrong_level {
        // The failure observed at this step carries the text of the instance-not-cutting-anything definition (Spec W-10g),
        // and the one operation that posts that definition (the shared cut-step reporter) names TWO failing elements, the
        // host and the instance, builds one "delete the instance" resolution on the instance, and posts it with the error
        // call for the commit. So the elements are {host, instance}, not {host, level}. UNKNOWN (Spec W-10g): why an
        // instance on another level stops cutting.
        cx.report(
            Failure::error(
                codes::INSERT_WRONG_LEVEL,
                "The element must be placed on the level its host is based on.",
            )
            .on(&[spec.host, id]),
        );
    }
    check_inserts(m, cx, &[id]);
    Ok(id)
}

/// Move an instance along its host (section 7c's `set_host_station`).
pub fn set_host_station(m: &mut BimModel, cx: &mut TxnContext<'_>, instance: ElementId, value: f64) -> Result<(), Failure> {
    check_station("host station", value)?;
    m.hosted_instance_mut(instance)?.host_station = value;
    check_inserts(m, cx, &[instance]);
    Ok(())
}

/// Write the instance's `Sill Height`; the head follows.
///
/// ASSUMED (section 3c): sill and head are **one relationship seen from two sides**,
/// `head = sill + type.height`, maintained in both directions. ASTERION stores only the sill, so
/// writing it moves the head by construction and the two can never disagree. DOCUMENTED (section
/// 3a): changing the sill does **not** resize the instance, which is why the type height is left
/// alone here.
pub fn set_sill_height(m: &mut BimModel, cx: &mut TxnContext<'_>, instance: ElementId, value: f64) -> Result<(), Failure> {
    check_station("sill height", value)?;
    m.hosted_instance_mut(instance)?.sill_height = value;
    check_inserts(m, cx, &[instance]);
    Ok(())
}

/// Write the instance's `Head Height`; the **sill** follows and the type height is unchanged.
///
/// The other half of the one relationship in [`set_sill_height`]: `sill = head - type.height`.
pub fn set_head_height(m: &mut BimModel, cx: &mut TxnContext<'_>, instance: ElementId, value: f64) -> Result<(), Failure> {
    check_station("head height", value)?;
    let type_id = m.hosted_instance(instance)?.type_id;
    let height = m.hosted_type(type_id)?.height;
    m.hosted_instance_mut(instance)?.sill_height = value - height;
    check_inserts(m, cx, &[instance]);
    Ok(())
}

/// Retype an instance. The cut follows the new type's `width`/`height` immediately, so the three
/// geometric checks run again.
pub fn set_hosted_instance_type(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    instance: ElementId,
    type_id: ElementId,
) -> Result<(), Failure> {
    m.hosted_type(type_id)?;
    m.hosted_instance_mut(instance)?.type_id = type_id;
    check_inserts(m, cx, &[instance]);
    Ok(())
}

/// Write the instance's `Mark`.
///
/// DOCUMENTED (section 3a): `ALL_MODEL_MARK` "identifies or enumerates a particular instance as
/// specified by the user", duplicates warn but are allowed - so no uniqueness check here. The
/// auto-incrementing sequence and the second built-in whose display name is also "Mark" are both
/// out of v0.1 scope (sections 3a, 7f), which is why section 7e records `params.mark` instead of
/// comparing it.
pub fn set_instance_mark(m: &mut BimModel, _cx: &mut TxnContext<'_>, instance: ElementId, mark: &str) -> Result<(), Failure> {
    m.hosted_instance_mut(instance)?.mark = mark.to_owned();
    Ok(())
}

/// Flip the instance's facing axis.
///
/// ASTERION mirrors that exactly: a refused flip is a **no-op with a `false` result**, never a transaction failure, and
/// section 8 routes the boolean to the step's `flip_result` field rather than to `status`.
///
/// `Err` is therefore reserved for the one thing that is not a refusal: there is no such instance.
pub fn flip_facing(m: &mut BimModel, cx: &mut TxnContext<'_>, instance: ElementId) -> Result<bool, Failure> {
    crate::groups::guard(m, cx, &[instance]);
    let i = m.hosted_instance_mut(instance)?;
    if !HostedInstance::CAN_FLIP_FACING {
        return Ok(false);
    }
    i.facing_flipped = !i.facing_flipped;
    Ok(true)
}

/// Flip the instance's hand axis. Same boolean channel as [`flip_facing`].
///
/// Neither flip moves the cut: the cut is centred on the station and spans the full host
/// thickness, so flipping is a pure orientation change (ASSUMED, section 7b) and no geometric
/// check is re-run. The *derived* `mirrored` flag changes with either flip
/// ([`HostedInstance::mirrored`]).
pub fn flip_hand(m: &mut BimModel, cx: &mut TxnContext<'_>, instance: ElementId) -> Result<bool, Failure> {
    crate::groups::guard(m, cx, &[instance]);
    // OBSERVED: the hand of a window on an ARC host does not flip (committed, nothing changes); ENGINE CHOICE for every
    // instance on an arc
    let on_arc = m.hosted_instances.get(&instance).and_then(|i| m.walls.get(&i.host)).map_or(false, |w| w.through.is_some());
    let i = m.hosted_instance_mut(instance)?;
    if !HostedInstance::CAN_FLIP_HAND || on_arc {
        return Ok(false);
    }
    i.hand_flipped = !i.hand_flipped;
    Ok(true)
}

/// A value written to an instance parameter. The reference's parameters are typed
/// (`AsDouble`/`AsString`/`AsElementId`), so writing the wrong storage is a rejection, not a
/// coercion.
#[derive(Clone, Debug, PartialEq)]
pub enum HostedParamValue {
    Length(f64),
    Text(String),
    Element(ElementId),
}

/// Write one instance parameter by name, which is the shape section 8's
/// `set_hosted_instance_param` op contract takes.
///
/// Routes `sill_height` / `head_height` / `mark` to the typed ops above, and **rejects** a write
/// to `level` or `host_id` with `param.read_only` (section 7d: "writing `Host Id`, `Level` after
/// placement, or a computed value -> reject"). ASSUMED (section 3a): both are read-only after
/// placement; a write of the value the instance already carries is accepted as a no-op, so the
/// rejection is about *changing* the host or level, not about touching the parameter.
///
/// `element.wrong_kind` rejects a value of the wrong storage type (a string for the sill, a number
/// for the mark).
pub fn set_hosted_instance_param(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    instance: ElementId,
    param: HostedInstanceParam,
    value: HostedParamValue,
) -> Result<(), Failure> {
    crate::groups::guard(m, cx, &[instance]);
    let current = m.hosted_instance(instance)?.clone();
    // OBSERVED (step 30): `mark` DOES NOT EXIST on an instance of a family built from the shipped template - writing it is
    // an error, raised before the change. Same scope note as `rough_width`/`rough_height` on the type: measured for a
    // template-built family, untested for a library one.
    if param == HostedInstanceParam::Mark {
        return Err(Failure::error(codes::PARAM_NOT_PRESENT, "The parameter does not exist on this family.")
            .on(&[instance]));
    }
    if param.read_only() {
        let unchanged = match (param, &value) {
            (HostedInstanceParam::Level, HostedParamValue::Element(id)) => *id == current.level,
            (HostedInstanceParam::HostId, HostedParamValue::Element(id)) => *id == current.host,
            _ => false,
        };
        if unchanged {
            return Ok(());
        }
        return Err(Failure::error(
            codes::READ_ONLY,
            format!("The instance parameter {:?} is read-only after placement.", param.name()),
        )
        .on(&[instance]));
    }
    match (param, value) {
        (HostedInstanceParam::SillHeight, HostedParamValue::Length(v)) => set_sill_height(m, cx, instance, v),
        (HostedInstanceParam::HeadHeight, HostedParamValue::Length(v)) => set_head_height(m, cx, instance, v),
        (HostedInstanceParam::Mark, HostedParamValue::Text(v)) => set_instance_mark(m, cx, instance, &v),
        (p, v) => Err(Failure::error(
            codes::WRONG_KIND,
            format!("The instance parameter {:?} does not take {v:?}.", p.name()),
        )
        .on(&[instance])),
    }
}

/// An extent that drives the **cut**: must be finite and strictly positive.
///
/// ASSUMED (section 5): "reject non-finite and non-positive width/height outright
/// (`insert.bad_size`), accept anything positive, and adopt whatever threshold the measurement
/// shows rather than inventing one" - so [`crate::spec::MIN_OPENING_EXTENT_M`] is 0 and a 2 mm
/// door is accepted, exactly as a 2 mm rectangular opening is.
fn check_cut_extent(label: &str, v: f64) -> Result<(), Failure> {
    if v.is_finite() && v > crate::spec::MIN_OPENING_EXTENT_M {
        return Ok(());
    }
    Err(Failure::error(codes::INSERT_BAD_SIZE, format!("The hosted type {label} must be a positive length, got {v}.")))
}

/// A length that is **recorded only** (`thickness`, the rough opening, the inset): finite and
/// non-negative. ASSUMED: 0 is legitimate - it is what an unauthored rough dimension looks like
/// (section 3b), and it is the only value a door's inset ever takes.
fn check_recorded_length(label: &str, v: f64) -> Result<(), Failure> {
    if v.is_finite() && v >= 0.0 {
        return Ok(());
    }
    Err(Failure::error(codes::INSERT_BAD_SIZE, format!("The hosted type {label} must be a non-negative length, got {v}.")))
}

/// A signed length that only has to be a number (`default_sill_height`).
fn check_finite_length(label: &str, v: f64) -> Result<(), Failure> {
    if v.is_finite() {
        return Ok(());
    }
    Err(Failure::error(codes::INSERT_BAD_SIZE, format!("The hosted type {label} must be a finite length, got {v}.")))
}

/// A station or elevation along/above the host: finite, and otherwise unconstrained.
///
/// A station outside the host is **not** rejected here on purpose, and not clamped either (the setter stores the value as
/// given): it is a *geometric* matter, and an instance whose station lies at or beyond an end of the host curve only raises
/// the posted warning of [`check_inserts`]. A host-curve edit that leaves an instance off its host is the separate
/// commit-time failure posted by `set_wall_curve` (Spec W-10g).
fn check_station(label: &str, v: f64) -> Result<(), Failure> {
    if v.is_finite() {
        return Ok(());
    }
    Err(Failure::error(codes::INSERT_BAD_STATION, format!("The instance {label} must be a finite length, got {v}.")))
}

fn not_found(id: ElementId) -> Failure {
    Failure::error(codes::NOT_FOUND, format!("No element with id {id}."))
}

#[cfg(test)]
mod opening_tests {
    //! Unit tests of the `WallOpening` slice.

    use super::*;
    use crate::model::{Layer, LayerFunction};
    use crate::regen::opening_state;
    use crate::spec::codes;
    use asterion_core::ElementId;

    struct Fx {
        doc: crate::BimDocument,
        level: ElementId,
        wall_type: ElementId,
        /// 6.0 m long, 3.0 m high, 0.20 m thick, base at z = 0, unjoined.
        wall: ElementId,
    }

    fn fixture() -> Fx {
        let mut doc = crate::new_document();
        let out = doc.transact("fx", false, |m, cx| {
            let level = create_level(m, cx, "AST-L0", 0.0)?;
            let wall_type = create_wall_type(
                m,
                cx,
                "AST-WALL-200",
                CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?,
            )?;
            let wall = create_wall(
                m,
                cx,
                NewWall {
                    wall_type,
                    level,
                    start: Vec2::new(0.0, 0.0),
                    end: Vec2::new(6.0, 0.0),
                    height: 3.0,
                    base_offset: 0.0,
                    flip: false,
                    structural: false,
                },
            )?;
            Ok((level, wall_type, wall))
        });
        assert!(out.committed(), "fixture failed: {:?}", out.failures);
        let (level, wall_type, wall) = out.value.expect("fixture committed");
        Fx { doc, level, wall_type, wall }
    }

    impl Fx {
        fn host(&self) -> crate::WallState {
            let m = self.doc.model();
            wall_state(m, m.wall(self.wall).expect("host wall")).expect("host state")
        }

        /// Add an opening and assert it commits; returns its id.
        fn open(&mut self, u0: f64, z0: f64, u1: f64, z1: f64) -> ElementId {
            let wall = self.wall;
            let out = self.doc.transact("open", false, |m, cx| add_rectangular_opening(m, cx, wall, u0, z0, u1, z1));
            assert!(out.committed(), "opening {u0}..{u1} / {z0}..{z1} should commit: {:?}", out.failures);
            out.value.expect("committed")
        }

        /// Try to add an opening; returns the outcome without asserting anything.
        fn try_open(&mut self, u0: f64, z0: f64, u1: f64, z1: f64) -> asterion_core::txn::Outcome<ElementId> {
            let wall = self.wall;
            self.doc.transact("try-open", false, |m, cx| add_rectangular_opening(m, cx, wall, u0, z0, u1, z1))
        }
    }

    fn codes_of<T>(out: &asterion_core::txn::Outcome<T>) -> Vec<&str> {
        out.failures.iter().map(|f| f.code.as_str()).collect()
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    // ------------------------------------------------------------------ creation

    #[test]
    fn a_wall_opening_is_created_from_two_opposite_corners() {
        let mut fx = fixture();
        let o = fx.open(1.0, 0.9, 2.2, 2.1);
        let m = fx.doc.model();
        let op = m.opening(o).expect("opening exists");
        assert_eq!((op.u_min, op.u_max, op.z_min, op.z_max), (1.0, 2.2, 0.9, 2.1));
        assert_eq!(op.host, fx.wall);
        assert!(near(op.width(), 1.2) && near(op.height(), 1.2) && near(op.cut_area(), 1.44));
        assert!(m.contains(o), "contains() must know the opening");
        assert_eq!(m.openings_of(fx.wall), vec![o]);
        assert_eq!(m.dependents(fx.wall), vec![o], "the host's dependents must include its opening");

        let st = opening_state(m, op).expect("opening state");
        assert!(st.is_rect_boundary);
        assert!(near(st.cut_area, 1.44) && near(st.clipped_cut_area, 1.44));
        assert!(near(st.thickness, 0.2), "a wall opening cuts the full wall thickness");
        assert!(near(st.cut_volume, 0.288));
        assert!(st.inside_host_u && st.inside_host_z);
        assert!(st.solid.is_some(), "the cut prism is derived, never stored");
    }

    /// The reference's contract is "two opposite corners", so corner order carries no information.
    #[test]
    fn inverted_corners_are_normalised_rather_than_rejected() {
        let mut fx = fixture();
        let o = fx.open(2.2, 2.1, 1.0, 0.9);
        let op = fx.doc.model().opening(o).expect("opening").clone();
        assert_eq!((op.u_min, op.u_max, op.z_min, op.z_max), (1.0, 2.2, 0.9, 2.1));
    }

    /// DOCUMENTED (section 2a / 6c): one host-side relation covers everything inserted into a
    /// host, and section 7b orders it by the cut's lower station.
    #[test]
    fn the_insert_relation_lists_a_hosts_openings_sorted_by_station() {
        let mut fx = fixture();
        let far = fx.open(3.0, 0.0, 3.9, 2.1);
        let near_ = fx.open(1.0, 0.9, 2.2, 2.1);
        let rel = crate::regen::inserts(fx.doc.model(), fx.wall).expect("insert relation");
        assert_eq!(rel.iter().map(|i| i.element).collect::<Vec<_>>(), vec![near_, far]);
        assert!(rel.iter().all(|i| i.kind == crate::InsertKind::RectOpening && i.host == fx.wall));
        assert!(near(rel[0].cut.expect("clipped").area(), 1.44));
        assert!(near(rel[1].cut.expect("clipped").area(), 1.89));
    }

    // ------------------------------------------------- the net Area / Volume numbers

    /// For a 6.0 x 3.0 m, 0.20 m wall: baseline Area 18.0 / Volume 3.6; with one interior 1.2 x 1.2 opening Area 16.56 /
    /// Volume 3.312; adding a 0.9 x 2.1 opening at z = 0 Area 14.67 / Volume 2.934. If the reference product's
    /// `HOST_AREA_COMPUTED` turns out to be gross, this test is the thing that is wrong, and that is the point of writing
    /// it.
    #[test]
    fn the_host_area_and_volume_lose_the_opening_cut() {
        let mut fx = fixture();
        let base = fx.host();
        assert!(near(base.gross_area, 18.0) && near(base.area, 18.0) && near(base.cut_area, 0.0));
        assert!(near(base.volume, 3.6));

        fx.open(1.0, 0.9, 2.2, 2.1); // 1.2 x 1.2 = 1.44 m2, strictly inside the outline
        let one = fx.host();
        assert!(near(one.gross_area, 18.0), "the gross outline never moves: {}", one.gross_area);
        assert!(near(one.cut_area, 1.44), "cut area {}", one.cut_area);
        assert!(near(one.area, 16.56), "net area {}", one.area);
        assert!(near(one.volume, 3.312), "net volume {}", one.volume);

        fx.open(3.0, 0.0, 3.9, 2.1); // 0.9 x 2.1 = 1.89 m2, touching the host base
        let two = fx.host();
        assert!(near(two.cut_area, 3.33), "cut area {}", two.cut_area);
        assert!(near(two.area, 14.67), "net area {}", two.area);
        assert!(near(two.volume, 2.934), "net volume {}", two.volume);

        assert!(near(two.volume, two.area * two.width));
    }

    /// ASSUMED (section 2c): `surface_area` **grows** while `volume` shrinks - the cheap check
    /// that the cut is a real subtraction and not a graphical override.
    #[test]
    fn the_host_surface_area_grows_while_its_volume_shrinks() {
        let mut fx = fixture();
        let before = fx.host();
        // box: perimeter 18 x 0.2 + 2 x 18 = 3.6 + 36 = 39.6
        assert!(near(before.surface_area, 39.6), "{}", before.surface_area);
        fx.open(1.0, 0.9, 2.2, 2.1);
        let after = fx.host();
        // 2 x 16.56 + 3.6 + reveals 2 x (1.2 + 1.2) x 0.2 = 33.12 + 3.6 + 0.96 = 37.68
        assert!(near(after.surface_area, 37.68), "{}", after.surface_area);
        assert!(after.surface_area > 2.0 * after.area, "the reveal faces must be counted");
        assert!(after.volume < before.volume);
        assert!(after.surface_area - 2.0 * after.area > before.surface_area - 2.0 * before.area);
    }

    /// Faces now count **distinct supporting planes**, not boundary curves.
    ///
    /// OBSERVED: the reference counts coplanar side faces as one, which is the only thing that explains the asymmetry it
    /// measured — a floating opening adds **4** faces, one flush with the base adds **3**, because a flush opening is a
    /// *notch* whose two bottom pieces share the `z = 0` plane. it survives only as the fallback when the region cannot be
    /// formed.
    ///
    /// Edges are still the old `12 + 12n` prediction and are still WRONG — that is the project's
    /// last unexplained wall quantity and it needs a measurement, not a kernel. See
    /// `cut::tests::face_and_edge_counts_track_brep_history_not_geometry`.
    #[test]
    fn faces_count_distinct_planes_not_boundary_curves() {
        let mut fx = fixture();
        assert_eq!(fx.host().solid_face_count, 6, "a plain box has four side planes");
        fx.open(1.0, 0.9, 2.2, 2.1);
        assert_eq!(fx.host().solid_face_count, 10, "a floating hole adds four planes");
        fx.open(3.0, 0.0, 3.9, 2.1);
        // A notch adds three planes, not four: its two bottom pieces are one plane. And the
        // merge is per LOOP, not per solid - the door's head and the window's both sit at
        // `z = 2.100`, and the reference keeps them apart. 7 planes in the notched outer loop +
        // 4 in the window's hole + the two large faces = the measured 13.
        assert_eq!(fx.host().solid_face_count, 13);
    }

    // ------------------------------------------------------------ rejection channel

    /// `insert.host_not_supported`: the host is missing, or is not an element that hosts inserts.
    #[test]
    fn a_host_that_cannot_host_an_insert_is_rejected_before_the_change() {
        let mut fx = fixture();
        let level = fx.level;
        let wall_type = fx.wall_type;
        for (label, host) in [("level", level), ("wall type", wall_type), ("unknown id", ElementId(9999))] {
            let out = fx.doc.transact("bad-host", false, |m, cx| add_rectangular_opening(m, cx, host, 1.0, 0.9, 2.2, 2.1));
            assert!(!out.committed(), "{label} must not host an opening");
            assert_eq!(codes_of(&out), vec![codes::INSERT_HOST_NOT_SUPPORTED], "{label}");
        }
        assert!(fx.doc.model().wall_openings.is_empty());
    }

    /// `insert.bad_size`: a finite but degenerate rectangle. ASSUMED (section 5): anything
    /// positive is accepted, including the 2 mm rectangle of step `degenerate-2mm-opening`, so no
    /// minimum size is invented here.
    #[test]
    fn a_degenerate_opening_is_rejected_but_a_two_millimetre_one_is_accepted() {
        let mut fx = fixture();
        for (u0, z0, u1, z1) in [(1.0, 1.0, 1.0, 2.0), (1.0, 1.0, 2.0, 1.0), (1.0, 1.0, 1.0, 1.0)] {
            let out = fx.try_open(u0, z0, u1, z1);
            assert!(!out.committed(), "zero extent must be rejected");
            assert_eq!(codes_of(&out), vec![codes::INSERT_BAD_SIZE]);
        }
        let tiny = fx.open(5.0, 1.0, 5.002, 1.002);
        let m = fx.doc.model();
        assert!(near(m.opening(tiny).expect("tiny opening").cut_area(), 4e-6));
        assert!(near(wall_state(m, m.wall(fx.wall).unwrap()).unwrap().area, 18.0 - 4e-6));
    }

    /// `opening.invalid_boundary`: the two points do not describe a rectangle at all.
    #[test]
    fn a_non_finite_corner_is_an_invalid_boundary() {
        let mut fx = fixture();
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let out = fx.try_open(1.0, 0.9, bad, 2.1);
            assert!(!out.committed());
            assert_eq!(codes_of(&out), vec![codes::OPENING_INVALID_BOUNDARY]);
            let out = fx.try_open(1.0, bad, 2.2, 2.1);
            assert_eq!(codes_of(&out), vec![codes::OPENING_INVALID_BOUNDARY]);
        }
    }

    // --------------------------------------------------------- commit-failure channel

    /// "Committed - everything else, including a cut that exactly touches the host's edges"
    /// (section 7c). Two cuts that only touch each other are not an overlap either.
    #[test]
    fn cuts_that_only_touch_the_host_edges_or_each_other_commit() {
        let mut fx = fixture();
        fx.open(0.0, 0.0, 1.0, 3.0); // flush with the wall start, base and top
        fx.open(1.0, 0.0, 2.0, 3.0); // shares the u = 1.0 edge with the first
        let s = fx.host();
        assert!(near(s.cut_area, 6.0), "cut area {}", s.cut_area);
        assert!(near(s.area, 12.0) && near(s.volume, 2.4));
    }

    // --------------------------------------------------------------------- cascades

    /// ASSUMED (section 4): a host wall's openings are "totally dependent" on it, so they are
    /// deleted with it and their ids appear in the returned deleted set.
    #[test]
    fn deleting_the_host_wall_deletes_its_openings() {
        let mut fx = fixture();
        let a = fx.open(1.0, 0.9, 2.2, 2.1);
        let b = fx.open(3.0, 0.0, 3.9, 2.1);
        let wall = fx.wall;
        let out = fx.doc.transact("delete-host", false, |m, cx| delete(m, cx, wall));
        assert!(out.committed(), "{:?}", out.failures);
        let mut expected = vec![wall, a, b];
        expected.sort();
        assert_eq!(out.value.expect("deleted set"), expected);
        let m = fx.doc.model();
        assert!(m.wall_openings.is_empty() && m.walls.is_empty());
    }

    /// The cascade is transitive: level -> walls -> their openings, and type -> walls -> openings.
    #[test]
    fn the_host_cascade_is_transitive_through_levels_and_wall_types() {
        for via_level in [true, false] {
            let mut fx = fixture();
            let o = fx.open(1.0, 0.9, 2.2, 2.1);
            let target = if via_level { fx.level } else { fx.wall_type };
            let out = fx.doc.transact("delete-root", false, |m, cx| delete(m, cx, target));
            assert!(out.committed(), "via_level={via_level}: {:?}", out.failures);
            assert!(out.value.expect("deleted set").contains(&o), "via_level={via_level}");
            assert!(fx.doc.model().wall_openings.is_empty(), "via_level={via_level}");
        }
    }

    /// Deleting an opening leaves the host standing with its quantities back to gross, and the
    /// opening itself has no dependents.
    #[test]
    fn deleting_an_opening_restores_the_hosts_gross_quantities() {
        let mut fx = fixture();
        let a = fx.open(1.0, 0.9, 2.2, 2.1);
        let b = fx.open(3.0, 0.0, 3.9, 2.1);
        assert!(fx.doc.model().dependents(a).is_empty(), "nothing depends on an opening");
        let out = fx.doc.transact("delete-opening", false, |m, cx| delete(m, cx, a));
        assert!(out.committed(), "{:?}", out.failures);
        assert_eq!(out.value.expect("deleted set"), vec![a]);
        let m = fx.doc.model();
        assert_eq!(m.openings_of(fx.wall), vec![b]);
        assert!(m.walls.contains_key(&fx.wall), "the host survives");
        let s = wall_state(m, m.wall(fx.wall).unwrap()).unwrap();
        assert!(near(s.area, 18.0 - 1.89) && near(s.volume, (18.0 - 1.89) * 0.2));

        let out = fx.doc.transact("delete-last", false, |m, cx| delete(m, cx, b));
        assert!(out.committed());
        let s = fx.host();
        assert!(near(s.area, 18.0) && near(s.volume, 3.6) && near(s.cut_area, 0.0));
        assert_eq!((s.solid_face_count, s.solid_edge_count), (6, 12));
    }

    /// Undo must restore the host's net quantities, because they are derived and never stored.
    #[test]
    fn undo_restores_the_hosts_net_quantities_because_they_are_derived() {
        let mut fx = fixture();
        fx.open(1.0, 0.9, 2.2, 2.1);
        assert!(near(fx.host().area, 16.56));
        assert!(fx.doc.undo().is_ok(), "the opening transaction is undoable");
        assert!(near(fx.host().area, 18.0), "after undo: {}", fx.host().area);
        assert!(fx.doc.redo().is_ok(), "and redoable");
        assert!(near(fx.host().area, 16.56), "after redo: {}", fx.host().area);
    }

    /// The thickness of the cut follows the host type, so a type change re-derives the volume
    /// without touching the stored opening (section 4: "the cut depth follows the new width").
    #[test]
    fn the_cut_depth_follows_the_host_type_width() {
        let mut fx = fixture();
        let o = fx.open(1.0, 0.9, 2.2, 2.1);
        let wall_type = fx.wall_type;
        let out = fx.doc.transact("thicken", false, |m, cx| {
            set_wall_type_structure(
                m,
                cx,
                wall_type,
                CompoundStructure::new(vec![Layer { width: 0.3, function: LayerFunction::Structure }], 0, 0)?,
            )
        });
        assert!(out.committed(), "{:?}", out.failures);
        let m = fx.doc.model();
        let s = wall_state(m, m.wall(fx.wall).unwrap()).unwrap();
        assert!(near(s.area, 16.56), "the elevation cut is unchanged: {}", s.area);
        assert!(near(s.volume, 16.56 * 0.3), "volume follows the new width: {}", s.volume);
        let st = opening_state(m, m.opening(o).unwrap()).unwrap();
        assert!(near(st.thickness, 0.3) && near(st.cut_volume, 1.44 * 0.3));
        let op = m.opening(o).unwrap();
        assert_eq!((op.u_min, op.u_max, op.z_min, op.z_max), (1.0, 2.2, 0.9, 2.1));
    }

    /// An opening step whose boolean succeeds but removes nothing posts a WARNING on the opening and the transaction
    /// COMMITS. The cases are the ones the rule decides: entirely off the host, nested inside an earlier cut, and lying in
    /// a band that is already open. An opening that opens anything new - even partly - is silent, and an EARLIER opening is
    /// never reported because of a later one (steps run in creation order: one boolean after another on the wall geometry
    /// the previous ones modified).
    #[test]
    fn spec_w09c_an_opening_that_cuts_nothing_warns_and_commits() {
        let mut fx = fixture();
        // the first opening always cuts: no warning
        let first = fx.try_open(1.0, 0.9, 2.2, 2.1);
        assert!(first.committed() && first.failures.is_empty(), "{:?}", first.failures);
        let first = first.value.expect("opening");

        // nested strictly inside the first cut: cuts nothing new
        let nested = fx.try_open(1.2, 1.0, 1.8, 1.8);
        assert!(nested.committed(), "a warning does not roll back: {:?}", nested.failures);
        assert_eq!(codes_of(&nested), vec![codes::OPENING_DOES_NOT_CUT_HOST]);
        assert_eq!(nested.failures[0].severity, asterion_core::Severity::Warning);
        assert_eq!(nested.failures[0].elements, vec![nested.value.expect("opening")]);

        // identical to the first one: all four edges coincident, still cuts nothing
        let same = fx.try_open(1.0, 0.9, 2.2, 2.1);
        assert!(same.committed());
        assert_eq!(codes_of(&same), vec![codes::OPENING_DOES_NOT_CUT_HOST]);

        // entirely beyond the end of the wall: clipped to nothing, accepted, warned
        let off = fx.try_open(7.0, 0.5, 8.0, 1.5);
        assert!(off.committed());
        assert_eq!(codes_of(&off), vec![codes::OPENING_DOES_NOT_CUT_HOST]);

        // overlapping partly: adds area, silent
        let partial = fx.try_open(2.0, 0.9, 3.0, 2.1);
        assert!(partial.committed() && partial.failures.is_empty(), "{:?}", partial.failures);

        // the warnings changed no quantity: 1.44 + the 0.8 x 1.2 part of the partial one
        assert!(near(fx.host().area, 18.0 - 1.44 - 0.8 * 1.2), "{}", fx.host().area);

        // a later opening never makes an EARLIER one warn at its own creation: the first still
        // reports no failure when it is re-sketched over open air
        let re = fx.doc.transact("resketch", false, |m, cx| set_rect_opening(m, cx, first, 4.0, 0.9, 5.0, 2.1));
        assert!(re.committed() && re.failures.is_empty(), "{:?}", re.failures);
        // and moving it back onto the area the later ones already opened makes it warn
        let back = fx.doc.transact("back", false, |m, cx| set_rect_opening(m, cx, first, 2.2, 1.0, 2.8, 2.0));
        assert!(back.committed());
        assert!(back.failures.is_empty(), "the first opening is the earliest step: {:?}", back.failures);
    }
}

#[cfg(test)]
mod hosted_tests {
    //! Unit tests of the hosted-instance slice - doors and windows.
    //!
    //! The named sizes every number below is built from:
    //! * host `AST-WALL-200`: a **6.000 x 3.000 m** wall, **0.200 m** thick, single layer,
    //!   unjoined, base level at z = 0 -> gross Area **18.000 m2**, Volume **3.600 m3**.
    //! * door type `AST-DOOR-900x2100`: **0.900 x 2.100 m** -> a cut of **1.890 m2**.
    //! * window type `AST-WINDOW-1200x1200`: **1.200 x 1.200 m**, default sill **0.900 m** ->
    //!   a cut of **1.440 m2**.
    //!
    //! The resulting host quantities this engine commits to:
    //! | host carries | Area (m2) | Volume (m3) | faces | edges |
    //! |---|---|---|---|---|
    //! | nothing | 18.000 | 3.600 | 6 | 12 |
    //! | the door | **16.110** | **3.222** | 10 | 24 |
    //! | door + window | **14.670** | **2.934** | 14 | 36 |

    use super::*;
    use crate::model::{HostedKind, HostedTypeParam, Layer, LayerFunction};
    use crate::regen::{hosted_instance_state, inserts};
    use crate::spec::codes;
    use asterion_core::ElementId;

    struct Fx {
        doc: crate::BimDocument,
        level: ElementId,
        /// A second level, 3.0 m up, used for the level-cascade and sill-datum tests.
        upper: ElementId,
        wall_type: ElementId,
        /// 6.0 m long, 3.0 m high, 0.20 m thick, base at z = 0, unjoined.
        wall: ElementId,
        door_type: ElementId,
        window_type: ElementId,
    }

    fn fixture() -> Fx {
        let mut doc = crate::new_document();
        let out = doc.transact("fx", false, |m, cx| {
            let level = create_level(m, cx, "AST-L0", 0.0)?;
            let upper = create_level(m, cx, "AST-L1", 3.0)?;
            let wall_type = create_wall_type(
                m,
                cx,
                "AST-WALL-200",
                CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?,
            )?;
            let wall = create_wall(
                m,
                cx,
                NewWall {
                    wall_type,
                    level,
                    start: Vec2::new(0.0, 0.0),
                    end: Vec2::new(6.0, 0.0),
                    height: 3.0,
                    base_offset: 0.0,
                    flip: false,
                    structural: false,
                },
            )?;
            // kind, width, height, thickness, rough_w, rough_h, default_sill, inset
            let door_type =
                create_hosted_type(m, cx, "AST-DOOR-900x2100", HostedKind::Door, 0.9, 2.1, 0.05, 0.95, 2.15, 0.0, 0.0)?;
            let window_type = create_hosted_type(
                m,
                cx,
                "AST-WINDOW-1200x1200",
                HostedKind::Window,
                1.2,
                1.2,
                0.06,
                1.25,
                1.25,
                0.9,
                0.02,
            )?;
            Ok((level, upper, wall_type, wall, door_type, window_type))
        });
        assert!(out.committed(), "fixture failed: {:?}", out.failures);
        let (level, upper, wall_type, wall, door_type, window_type) = out.value.expect("fixture committed");
        Fx { doc, level, upper, wall_type, wall, door_type, window_type }
    }

    impl Fx {
        fn host(&self) -> crate::WallState {
            let m = self.doc.model();
            wall_state(m, m.wall(self.wall).expect("host wall")).expect("host state")
        }

        fn state(&self, i: ElementId) -> crate::HostedInstanceState {
            let m = self.doc.model();
            hosted_instance_state(m, m.hosted_instance(i).expect("instance")).expect("instance state")
        }

        /// Place an instance and assert it commits; returns its id.
        fn place(&mut self, type_id: ElementId, station: f64) -> ElementId {
            let out = self.try_place(type_id, station, None);
            assert!(out.committed(), "placement at {station} should commit: {:?}", out.failures);
            out.value.expect("committed")
        }

        fn try_place(
            &mut self,
            type_id: ElementId,
            station: f64,
            sill: Option<f64>,
        ) -> asterion_core::txn::Outcome<ElementId> {
            let (host, level) = (self.wall, self.level);
            self.doc.transact("place", false, |m, cx| {
                place_hosted_instance(m, cx, NewHostedInstance { type_id, host, level, host_station: station, sill_height: sill })
            })
        }
    }

    fn codes_of<T>(out: &asterion_core::txn::Outcome<T>) -> Vec<&str> {
        out.failures.iter().map(|f| f.code.as_str()).collect()
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    /// A door wholly off the end of its host is NOT refused; the transaction commits and a warning on the instance and the
    /// host is posted. A door that still reaches the wall is silent, and no quantity changes (round 9: clipped to the
    /// host).
    #[test]
    fn an_instance_off_the_host_curve_commits_with_a_warning() {
        let mut fx = fixture();
        // station 7.0 on a 6.0 m wall, half extent 0.45: the extent does not reach back
        let out = fx.try_place(fx.door_type, 7.0, None);
        assert!(out.committed(), "{:?}", out.failures);
        assert_eq!(codes_of(&out), vec![codes::INSERT_OVERLAPS_OR_OUT]);
        let w = &out.failures[0];
        assert_eq!(w.severity, asterion_core::diag::Severity::Warning);
        assert_eq!(w.elements.len(), 2, "the instance and its host");
        assert!(near(fx.host().area, 18.0), "nothing was cut out of the host");

        // station 6.2: the door's half still lies on the wall -> no warning
        let ok = fx.try_place(fx.door_type, 6.2, None);
        assert!(ok.committed());
        assert!(ok.failures.is_empty(), "{:?}", ok.failures);
        // an in-range door never warns
        let inside = fx.try_place(fx.door_type, 3.0, None);
        assert!(inside.failures.is_empty());
    }

    // ------------------------------------------- host curve edits

    /// Run `set_wall_curve` on the fixture wall.
    fn edit_curve(fx: &mut Fx, start: (f64, f64), end: (f64, f64)) -> asterion_core::txn::Outcome<()> {
        let wall = fx.wall;
        fx.doc.transact("edit-curve", false, |m, cx| {
            set_wall_curve(m, cx, wall, Vec2::new(start.0, start.1), Vec2::new(end.0, end.1))
        })
    }

    /// Its transform is re-derived from the stored station on the new curve. A translation of the host therefore commits
    /// with no failure and the instance travels with it.
    #[test]
    fn moving_the_host_carries_the_instance_by_its_stored_station() {
        let mut fx = fixture();
        let d = fx.place(fx.door_type, 2.0);
        let before = fx.state(d).location_point;
        let out = edit_curve(&mut fx, (10.0, 5.0), (16.0, 5.0));
        assert!(out.committed(), "{:?}", out.failures);
        assert!(out.failures.is_empty(), "{:?}", out.failures);
        let m = fx.doc.model();
        let inst = m.hosted_instance(d).expect("instance");
        assert_eq!(inst.host, fx.wall, "Spec W-10b: not re-hosted");
        assert_eq!(inst.host_station, 2.0, "Spec W-10b: the station is not recomputed");
        let after = fx.state(d).location_point;
        assert!(near(after.x - before.x, 10.0) && near(after.y - before.y, 5.0), "{before:?} -> {after:?}");
        assert!(near(fx.host().area, 16.11), "the cut follows the host: {}", fx.host().area);
    }

    /// Spec W-10b: shortening the host keeps the stored station (no clamp, no recompute, no rescale) while the extent stays
    /// on the curve. UNKNOWN: raw versus normalised station, so the value is pinned only as "unchanged".
    #[test]
    fn a_host_edit_keeps_the_stored_station_without_clamping_or_rehosting() {
        let mut fx = fixture();
        let d = fx.place(fx.door_type, 2.0);
        let out = edit_curve(&mut fx, (0.0, 0.0), (3.0, 0.0));
        assert!(out.committed(), "{:?}", out.failures);
        assert!(out.failures.is_empty(), "{:?}", out.failures);
        let inst = fx.doc.model().hosted_instance(d).expect("instance").clone();
        assert_eq!((inst.host, inst.host_station), (fx.wall, 2.0));
    }

    /// Spec W-10g: the edit is applied and the instance-not-cutting failure is posted for the commit as an error (the step
    /// itself does not fail; severity literal 2 is Error). UNKNOWN (Spec W-10g): the geometric extent test; the engine
    /// keeps its own threshold. What the rules decide is pinned: the failure names the host and the instance, and the model
    /// still has the instance on its host at its stored station (no re-host, no clamp, no deletion, Spec W-10b).
    #[test]
    fn a_host_edit_that_leaves_the_instance_off_the_curve_keeps_it_on_its_station() {
        let mut fx = fixture();
        let d = fx.place(fx.door_type, 4.0);
        let out = edit_curve(&mut fx, (0.0, 0.0), (1.0, 0.0));
        // Spec W-10g: the instance-not-cutting failure, posted for the commit as an error.
        assert!(codes_of(&out).contains(&codes::INSERT_NOT_CUTTING_HOST), "{:?}", out.failures);
        assert!(
            out.failures
                .iter()
                .filter(|f| f.code == codes::INSERT_NOT_CUTTING_HOST)
                .all(|f| f.severity == asterion_core::Severity::Error),
            "{:?}",
            out.failures
        );
        assert!(out.failures.iter().all(|f| f.elements == vec![fx.wall, d]), "{:?}", out.failures);
        let m = fx.doc.model();
        let inst = m.hosted_instance(d).expect("instance still exists");
        assert_eq!((inst.host, inst.host_station), (fx.wall, 4.0));
    }

    /// Its corners are re-evaluated on the new curve at the same stations, so they scale with its length. A translation
    /// changes nothing.
    #[test]
    fn a_rectangular_opening_keeps_its_normalised_stations_when_the_host_curve_changes() {
        let mut fx = fixture();
        let wall = fx.wall;
        let out = fx.doc.transact("open", false, |m, cx| add_rectangular_opening(m, cx, wall, 1.5, 0.9, 3.0, 2.1));
        assert!(out.committed(), "{:?}", out.failures);
        let o = out.value.expect("opening");
        let stations = |fx: &Fx| {
            let op = fx.doc.model().opening(o).expect("opening");
            (op.u_min, op.u_max, op.z_min, op.z_max)
        };
        assert_eq!(stations(&fx), (1.5, 3.0, 0.9, 2.1));

        // translation: same length, same stations
        assert!(edit_curve(&mut fx, (2.0, 2.0), (8.0, 2.0)).committed());
        assert_eq!(stations(&fx), (1.5, 3.0, 0.9, 2.1));

        // end drag, 6 m -> 12 m: normalised stations 0.25 and 0.5 -> 3.0 and 6.0
        assert!(edit_curve(&mut fx, (2.0, 2.0), (14.0, 2.0)).committed());
        let (u0, u1, z0, z1) = stations(&fx);
        assert!(near(u0, 3.0) && near(u1, 6.0), "{u0} {u1}");
        // UNKNOWN: the z reference of the corners; absolute z is kept.
        assert_eq!((z0, z1), (0.9, 2.1));

        // start drag, 12 m -> 4 m: the stations are measured from the NEW start
        assert!(edit_curve(&mut fx, (10.0, 2.0), (14.0, 2.0)).committed());
        let (u0, u1, _, _) = stations(&fx);
        assert!(near(u0, 1.0) && near(u1, 2.0), "{u0} {u1}");
    }

    /// Together on one host: the door keeps its stored station, the rectangular opening beside it is carried by normalised
    /// parameter (halved with the curve).
    #[test]
    fn a_host_edit_treats_instances_and_openings_by_their_own_rule() {
        let mut fx = fixture();
        let wall = fx.wall;
        let d = fx.place(fx.door_type, 2.0);
        let out = fx.doc.transact("open", false, |m, cx| add_rectangular_opening(m, cx, wall, 0.5, 0.9, 1.5, 2.1));
        assert!(out.committed());
        let o = out.value.expect("opening");
        let out = edit_curve(&mut fx, (0.0, 0.0), (3.0, 0.0));
        assert!(out.committed(), "{:?}", out.failures);
        assert!(out.failures.is_empty(), "{:?}", out.failures);
        assert_eq!(fx.doc.model().hosted_instance(d).expect("instance").host_station, 2.0);
        let op = fx.doc.model().opening(o).expect("opening").clone();
        assert!(near(op.u_min, 0.25) && near(op.u_max, 0.75));
    }

    // ------------------------------------------------------------------ types

    /// A hosted type is a flat record with the section 3b parameters, modelled like `WallType`.
    #[test]
    fn a_hosted_type_carries_the_size_and_the_reporting_only_rough_opening() {
        let fx = fixture();
        let m = fx.doc.model();
        let d = m.hosted_type(fx.door_type).expect("door type");
        assert_eq!(d.kind, HostedKind::Door);
        assert!(near(d.width, 0.9) && near(d.height, 2.1) && near(d.thickness, 0.05));
        assert!(near(d.rough_width, 0.95) && near(d.rough_height, 2.15));
        // ASSUMED (sections 1c, 3b): a door has no inset, so the value given is pinned to 0.
        assert!(near(d.inset, 0.0), "a door type must not carry an inset");
        assert!(m.contains(fx.door_type), "contains() must know a hosted type");

        let w = m.hosted_type(fx.window_type).expect("window type");
        assert_eq!(w.kind, HostedKind::Window);
        assert!(near(w.inset, 0.02), "a window type keeps its inset");
        assert!(near(w.default_sill_height, 0.9));
    }

    /// `hosted_type.duplicate_name` (ASSUMED, section 7a: "name unique among hosted types").
    #[test]
    fn a_duplicate_hosted_type_name_is_rejected_before_the_change() {
        let mut fx = fixture();
        let out = fx.doc.transact("dup", false, |m, cx| {
            create_hosted_type(m, cx, "AST-DOOR-900x2100", HostedKind::Door, 1.0, 2.0, 0.05, 1.0, 2.0, 0.0, 0.0)
        });
        assert!(!out.committed());
        assert_eq!(codes_of(&out), vec![codes::HOSTED_TYPE_DUPLICATE_NAME]);
        assert_eq!(fx.doc.model().hosted_types.len(), 2, "no third type was created");
    }

    /// `insert.bad_size` on every length a type carries. ASSUMED (section 5): the two cut-driving
    /// extents must be strictly positive, the recorded ones merely non-negative, and no minimum is
    /// invented - a 2 mm door is accepted exactly as a 2 mm rectangular opening is.
    #[test]
    fn a_hosted_type_with_a_bad_length_is_rejected_before_the_change() {
        let mut fx = fixture();
        let bad: [(&str, [f64; 7]); 9] = [
            ("zero width", [0.0, 2.1, 0.05, 0.9, 2.1, 0.0, 0.0]),
            ("negative width", [-0.9, 2.1, 0.05, 0.9, 2.1, 0.0, 0.0]),
            ("nan width", [f64::NAN, 2.1, 0.05, 0.9, 2.1, 0.0, 0.0]),
            ("zero height", [0.9, 0.0, 0.05, 0.9, 2.1, 0.0, 0.0]),
            ("infinite height", [0.9, f64::INFINITY, 0.05, 0.9, 2.1, 0.0, 0.0]),
            ("negative thickness", [0.9, 2.1, -0.05, 0.9, 2.1, 0.0, 0.0]),
            ("nan rough width", [0.9, 2.1, 0.05, f64::NAN, 2.1, 0.0, 0.0]),
            ("negative inset", [0.9, 2.1, 0.05, 0.9, 2.1, 0.0, -0.01]),
            ("nan default sill", [0.9, 2.1, 0.05, 0.9, 2.1, f64::NAN, 0.0]),
        ];
        for (label, v) in bad {
            let out = fx.doc.transact("bad-type", false, |m, cx| {
                create_hosted_type(m, cx, "AST-BAD", HostedKind::Window, v[0], v[1], v[2], v[3], v[4], v[5], v[6])
            });
            assert!(!out.committed(), "{label} must be rejected");
            assert_eq!(codes_of(&out), vec![codes::INSERT_BAD_SIZE], "{label}");
        }
        // And a 2 mm door commits, with the cut area to prove it is real.
        let out = fx.doc.transact("tiny", false, |m, cx| {
            create_hosted_type(m, cx, "AST-TINY", HostedKind::Door, 0.002, 0.002, 0.001, 0.0, 0.0, 0.0, 0.0)
        });
        assert!(out.committed(), "{:?}", out.failures);
        let tiny = out.value.unwrap();
        let i = fx.place(tiny, 5.5);
        assert!(near(fx.state(i).cut_area, 4e-6));
    }

    // ---------------------------------------------------- placement and the host's numbers

    /// Placement: section 1b's hosted overload, section 1c's station, section 1d's initial flags.
    #[test]
    fn a_hosted_instance_knows_its_host_type_level_and_station() {
        let mut fx = fixture();
        let d = fx.place(fx.door_type, 2.0);
        let m = fx.doc.model();
        let i = m.hosted_instance(d).expect("instance exists");
        assert_eq!((i.host, i.type_id, i.level), (fx.wall, fx.door_type, fx.level));
        assert!(near(i.host_station, 2.0));
        // ASSUMED (section 3c): a door defaults to Sill Height = 0.
        assert!(near(i.sill_height, 0.0));
        assert!(!i.facing_flipped && !i.hand_flipped, "a fresh placement is unflipped on both axes");
        assert_eq!(i.mark, "", "the auto-increment Mark sequence is out of v0.1 scope");
        assert!(m.contains(d), "contains() must know the instance");
        assert_eq!(m.hosted_instances_of(fx.wall), vec![d]);
        assert_eq!(m.instances_of_hosted_type(fx.door_type), vec![d]);
        assert!(m.dependents(fx.wall).contains(&d), "the host's dependents must include its instance");
        assert!(m.dependents(fx.level).contains(&d), "the level's dependents must include the instance");
        assert!(m.dependents(fx.door_type).contains(&d), "the type's dependents must include its instance");

        // The cut rectangle, derived: centred on the station, from the level plane upwards.
        let st = fx.state(d);
        assert!(near(st.rect.u_lo, 1.55) && near(st.rect.u_hi, 2.45), "{:?}", st.rect);
        assert!(near(st.rect.z_lo, 0.0) && near(st.rect.z_hi, 2.1), "{:?}", st.rect);
        assert!(near(st.cut_area, 1.89) && near(st.clipped_cut_area, 1.89));
        assert!(near(st.thickness, 0.2), "a hosted instance cuts the full host thickness in v0.1");
        assert!(near(st.cut_volume, 0.378));
        assert!(st.inside_host_u && st.inside_host_z);
        assert!(st.solid.is_some(), "the cut prism is derived, never stored");
        // Section 1c's prediction for `LocationPoint.Point`: on the location curve, at the level.
        assert_eq!(st.location_point.to_array(), [2.0, 0.0, 0.0]);
        assert!(near(st.rotation, 0.0));
    }

    /// DOCUMENTED (section 3c): a window's initial sill is the type's *Default Sill Height*;
    /// ASSUMED: a door's is 0. Placement therefore needs no sill argument.
    #[test]
    fn the_initial_sill_comes_from_the_type_for_a_window_and_is_zero_for_a_door() {
        let mut fx = fixture();
        let d = fx.place(fx.door_type, 2.0);
        let w = fx.place(fx.window_type, 4.5);
        assert!(near(fx.state(d).sill_height, 0.0));
        assert!(near(fx.state(w).sill_height, 0.9), "the window takes the type default");
        assert!(near(fx.state(w).rect.z_lo, 0.9) && near(fx.state(w).rect.z_hi, 2.1));
        // An explicit sill overrides the default. Station 0.900 keeps the cut (u 0.300 .. 1.500)
        // clear of the door at station 2.000 (u 1.550 .. 2.450), which would otherwise be an
        // overlap and therefore a commit failure.
        let out = fx.try_place(fx.window_type, 0.9, Some(0.4));
        assert!(out.committed(), "{:?}", out.failures);
        assert!(near(fx.state(out.value.unwrap()).sill_height, 0.4));
    }

    /// **The numbers this engine commits to.** A 6.000 x 3.000 m, 0.200 m host loses
    /// 0.900 x 2.100 m to a door and 1.200 x 1.200 m to a window:
    /// 18.000 -> 16.110 -> 14.670 m2, and 3.600 -> 3.222 -> 2.934 m3.
    ///
    /// If that is false, this test is what is wrong, and writing it down is the point.
    #[test]
    fn the_host_area_and_volume_lose_a_door_and_then_a_window() {
        let mut fx = fixture();
        let base = fx.host();
        assert!(near(base.gross_area, 18.0) && near(base.area, 18.0) && near(base.volume, 3.6));
        assert_eq!(base.insert_count, 0);

        fx.place(fx.door_type, 2.0); // 0.9 x 2.1 = 1.89 m2, sill 0 -> touches the host base
        let one = fx.host();
        assert!(near(one.gross_area, 18.0), "the gross outline never moves: {}", one.gross_area);
        assert!(near(one.cut_area, 1.89), "cut area {}", one.cut_area);
        assert!(near(one.area, 16.11), "net area {}", one.area);
        assert!(near(one.volume, 3.222), "net volume {}", one.volume);
        assert_eq!(one.insert_count, 1);

        fx.place(fx.window_type, 4.5); // 1.2 x 1.2 = 1.44 m2, sill 0.9 -> strictly inside
        let two = fx.host();
        assert!(near(two.cut_area, 3.33), "cut area {}", two.cut_area);
        assert!(near(two.area, 14.67), "net area {}", two.area);
        assert!(near(two.volume, 2.934), "net volume {}", two.volume);
        assert_eq!(two.insert_count, 2);
        // The identity section 7b commits to: Volume = Area x width.
        assert!(near(two.volume, two.area * two.width));
        // Faces count distinct supporting planes, merged per LOOP: the measured 13. Edges are still the falsified `12 +
        // 12n` and are the project's last open wall quantity.
        assert_eq!((two.solid_face_count, two.solid_edge_count), (13, 36));
        // The measured model is `2 x net area + perimeter of the net region x thickness` = 2 x 14.670 + 27.000 x 0.200. The
        // old term billed reveal for the door's jambs where they coincide with the wall's own base edge, which is not
        // interior surface at all - a 0.360 m2 overcount that only a measurement would ever have caught.
        assert!(near(two.surface_area, 34.74), "surface area {}", two.surface_area);
    }

    /// **The whole point of section 6c.** A rectangular opening and a hosted door of the same rectangle must move the
    /// host's quantities *identically*, because they go through one relation.
    #[test]
    fn a_door_and_an_equivalent_rect_opening_cut_the_host_identically() {
        let as_instance = {
            let mut fx = fixture();
            fx.place(fx.door_type, 2.0);
            fx.host()
        };
        let as_opening = {
            let mut fx = fixture();
            let wall = fx.wall;
            let out = fx.doc.transact("open", false, |m, cx| add_rectangular_opening(m, cx, wall, 1.55, 0.0, 2.45, 2.1));
            assert!(out.committed(), "{:?}", out.failures);
            fx.host()
        };
        assert!(near(as_instance.area, as_opening.area), "{} vs {}", as_instance.area, as_opening.area);
        assert!(near(as_instance.volume, as_opening.volume));
        assert!(near(as_instance.cut_area, as_opening.cut_area));
        assert!(near(as_instance.surface_area, as_opening.surface_area));
        assert_eq!(as_instance.insert_count, as_opening.insert_count);
        assert_eq!(as_instance.solid_face_count, as_opening.solid_face_count);
        assert_eq!(as_instance.solid_edge_count, as_opening.solid_edge_count);
    }

    /// DOCUMENTED (sections 2a, 6c): one host-side relation covers **both** element kinds, sorted
    /// by the cut's lower station (section 7b).
    #[test]
    fn the_insert_relation_mixes_both_element_kinds_sorted_by_station() {
        let mut fx = fixture();
        let window = fx.place(fx.window_type, 4.5); // u 3.9 .. 5.1
        let door = fx.place(fx.door_type, 2.0); // u 1.55 .. 2.45
        let wall = fx.wall;
        let out = fx.doc.transact("open", false, |m, cx| add_rectangular_opening(m, cx, wall, 0.2, 1.0, 0.8, 2.0));
        assert!(out.committed(), "{:?}", out.failures);
        let opening = out.value.unwrap();

        let rel = inserts(fx.doc.model(), fx.wall).expect("insert relation");
        assert_eq!(rel.iter().map(|i| i.element).collect::<Vec<_>>(), vec![opening, door, window]);
        assert_eq!(
            rel.iter().map(|i| i.kind).collect::<Vec<_>>(),
            vec![crate::InsertKind::RectOpening, crate::InsertKind::HostedInstance, crate::InsertKind::HostedInstance]
        );
        assert!(rel.iter().all(|i| i.host == fx.wall && i.cut.is_some()));
        assert_eq!(fx.host().insert_count, 3);
        assert!(near(fx.host().cut_area, 1.89 + 1.44 + 0.6));
    }

    // ------------------------------------------------------- the sill / head relationship

    /// ASSUMED (section 3c): sill and head are **two views of one relationship**,
    /// `head = sill + type.height`, maintained in both directions with the type height unchanged.
    #[test]
    fn sill_and_head_are_two_views_of_one_relationship() {
        let mut fx = fixture();
        let d = fx.place(fx.door_type, 2.0);
        // door type height 2.100, sill 0 -> head 2.100
        assert!(near(fx.state(d).head_height, 2.1));

        let out = fx.doc.transact("sill", false, |m, cx| set_sill_height(m, cx, d, 0.15));
        assert!(out.committed(), "{:?}", out.failures);
        let st = fx.state(d);
        assert!(near(st.sill_height, 0.15) && near(st.head_height, 2.25), "head must follow the sill");
        assert!(near(st.height, 2.1), "the instance is NOT resized by a sill change");

        let out = fx.doc.transact("head", false, |m, cx| set_head_height(m, cx, d, 2.4));
        assert!(out.committed(), "{:?}", out.failures);
        let st = fx.state(d);
        assert!(near(st.sill_height, 0.3), "sill must follow the head: {}", st.sill_height);
        assert!(near(st.head_height, 2.4) && near(st.height, 2.1), "the type height is unchanged");

        // Only the sill is stored, so the two can never disagree - that is what makes this one
        // relationship rather than two parameters kept in sync.
        assert!(near(fx.doc.model().hosted_instance(d).unwrap().sill_height, 0.3));
        let st = fx.state(d);
        assert!(near(st.head_height, st.sill_height + st.height));
    }

    /// The sill datum is the **instance's level**, so the absolute sill elevation follows a level
    /// move (ASSUMED, section 4: "the insert travels with the wall; its absolute z = new level
    /// elevation + Sill Height").
    #[test]
    fn the_absolute_sill_elevation_follows_the_level() {
        let mut fx = fixture();
        let d = fx.place(fx.door_type, 2.0);
        let out = fx.doc.transact("sill", false, |m, cx| set_sill_height(m, cx, d, 0.2));
        assert!(out.committed(), "{:?}", out.failures);
        assert!(near(fx.state(d).sill_z, 0.2) && near(fx.state(d).head_z, 2.3));
        let level = fx.level;
        let out = fx.doc.transact("raise-level", false, |m, cx| set_level_elevation(m, cx, level, 0.5));
        assert!(out.committed(), "the level move must COMMIT: {:?}", out.failures);
        let st = fx.state(d);
        assert!(near(st.sill_height, 0.2), "the stored sill is relative and does not move");
        assert!(near(st.sill_z, 0.7) && near(st.head_z, 2.8), "the absolute elevations do move");
        // The host travelled with its level too, so the cut is still inside it.
        assert!(st.inside_host_z);
        assert!(near(fx.host().area, 16.11));
    }

    // ----------------------------------------------------------- facing / hand / mirrored

    /// ASSUMED (section 1d): facing equals the host's orientation when unflipped, the hand axis is
    /// the host's unit tangent, `hand x facing = +-Z`, and `mirrored = facing ^ hand` - so flipping
    /// **both** axes is a 180 degree rotation and is *not* mirrored.
    #[test]
    fn flipping_facing_and_hand_drives_the_derived_mirrored_flag() {
        let mut fx = fixture();
        let d = fx.place(fx.door_type, 2.0);
        let host = fx.host();

        let st = fx.state(d);
        assert_eq!(st.facing_orientation, host.orientation, "an unflipped instance faces the host's exterior");
        assert_eq!(st.hand_orientation, host.direction, "the hand axis is the host's unit tangent");
        // hand x facing (3D) must be +-Z: in plan that is the 2D cross product being +-1.
        let cross = st.hand_orientation.x * st.facing_orientation.y - st.hand_orientation.y * st.facing_orientation.x;
        assert!(near(cross.abs(), 1.0), "hand x facing must be +-Z, got {cross}");
        assert!(!st.mirrored);
        assert!(st.can_flip_facing && st.can_flip_hand);

        // flip facing -> one axis flipped -> MIRRORED
        let out = fx.doc.transact("flip-facing", false, |m, cx| flip_facing(m, cx, d));
        assert!(out.committed() && out.value == Some(true), "a flip returns a bool, never a failure");
        let st = fx.state(d);
        assert_eq!(st.facing_orientation, -host.orientation);
        assert_eq!(st.hand_orientation, host.direction);
        assert!(st.mirrored, "one axis flipped is mirrored");

        // flip hand as well -> BOTH axes flipped -> a 180 degree rotation, NOT mirrored
        let out = fx.doc.transact("flip-hand", false, |m, cx| flip_hand(m, cx, d));
        assert!(out.committed() && out.value == Some(true));
        let st = fx.state(d);
        assert_eq!(st.facing_orientation, -host.orientation);
        assert_eq!(st.hand_orientation, -host.direction);
        assert!(!st.mirrored, "both axes flipped is a rotation, not a mirror");
        assert!(near(st.rotation, std::f64::consts::PI));

        // unflip facing -> hand only -> mirrored again
        let out = fx.doc.transact("unflip-facing", false, |m, cx| flip_facing(m, cx, d));
        assert!(out.committed() && out.value == Some(true));
        let st = fx.state(d);
        assert!(st.mirrored);
        let i = fx.doc.model().hosted_instance(d).unwrap();
        assert!(!i.facing_flipped && i.hand_flipped, "the two flags are stored, `mirrored` is not");
        assert_eq!(i.mirrored(), st.mirrored, "mirrored is derived from the two flags, never stored");

        // Neither flip moves the cut, so the host's quantities are untouched.
        assert!(near(fx.host().area, 16.11));
    }

    /// A flip of an instance that does not exist is the one thing that is **not** a refusal, so it
    /// is the only `Err` this op can produce (section 1d: a refused flip is a no-op returning
    /// false, never a transaction failure).
    #[test]
    fn flipping_an_unknown_instance_is_an_error_not_a_false_result() {
        let mut fx = fixture();
        let out = fx.doc.transact("flip-nothing", false, |m, cx| flip_facing(m, cx, ElementId(9999)));
        assert!(!out.committed());
        assert_eq!(codes_of(&out), vec![codes::NOT_FOUND]);
    }

    // ----------------------------------------------------------- the type drives the cut

    /// ASSUMED (section 2a): the cut is the type's `Width x Height`.
    #[test]
    fn the_rough_opening_never_moves_the_host_area_but_the_width_does() {
        let mut fx = fixture();
        fx.place(fx.door_type, 2.0);
        assert!(near(fx.host().area, 16.11));

        let t = fx.door_type;
        // OBSERVED: `rough_width` and `rough_height` DO NOT EXIST on a family built from the shipped template - writing
        // either is refused before the change.
        for (param, value) in [(HostedTypeParam::RoughWidth, 1.1), (HostedTypeParam::RoughHeight, 2.3)] {
            let out = fx.doc.transact("rough", false, |m, cx| set_hosted_type_param(m, cx, t, param, value));
            assert!(!out.committed(), "{} does not exist and must be refused", param.name());
            assert_eq!(codes_of(&out), vec![codes::PARAM_NOT_PRESENT]);
            assert!(near(fx.host().area, 16.11), "a refused write cannot move the host Area");
        }
        // Thickness does exist, and writing it still must not move the host's Area - the cut is a
        // through-cut, so the host's face loses the same rectangle whatever the leaf is like.
        let out = fx.doc.transact("thick", false, |m, cx| {
            set_hosted_type_param(m, cx, t, HostedTypeParam::Thickness, 0.08)
        });
        assert!(out.committed(), "{:?}", out.failures);
        assert!(near(fx.host().area, 16.11));

        // Width 0.900 -> 1.200 raises the cut from 1.890 to 2.520 m2: Area 18.000 - 2.520 = 15.480.
        let out = fx.doc.transact("widen", false, |m, cx| set_hosted_type_size(m, cx, t, 1.2, 2.1));
        assert!(out.committed(), "{:?}", out.failures);
        let s = fx.host();
        assert!(near(s.cut_area, 2.52), "cut area {}", s.cut_area);
        assert!(near(s.area, 15.48) && near(s.volume, 3.096), "area {} volume {}", s.area, s.volume);
    }

    /// DOCUMENTED (section 3c): the type's *Default Sill Height* supplies the **initial** instance
    /// sill, not a live constraint, so changing it leaves placed instances where they are.
    #[test]
    fn changing_the_type_default_sill_leaves_placed_instances_alone() {
        let mut fx = fixture();
        let w = fx.place(fx.window_type, 4.5);
        assert!(near(fx.state(w).sill_height, 0.9));
        let t = fx.window_type;
        let out = fx.doc.transact("default-sill", false, |m, cx| {
            set_hosted_type_param(m, cx, t, HostedTypeParam::DefaultSillHeight, 1.4)
        });
        assert!(out.committed(), "{:?}", out.failures);
        assert!(near(fx.state(w).sill_height, 0.9), "the placed instance does not move");
        // ...but the next placement takes the new default.
        let w2 = fx.place(fx.window_type, 1.0);
        assert!(near(fx.state(w2).sill_height, 1.4));
    }

    /// Section 4: "the cut depth follows the new width, Width/Height/Sill Height unchanged, host
    /// Volume recomputed as Area x new width."
    #[test]
    fn the_cut_depth_follows_the_host_type_width() {
        let mut fx = fixture();
        let d = fx.place(fx.door_type, 2.0);
        let wall_type = fx.wall_type;
        let out = fx.doc.transact("thicken", false, |m, cx| {
            set_wall_type_structure(
                m,
                cx,
                wall_type,
                CompoundStructure::new(vec![Layer { width: 0.35, function: LayerFunction::Structure }], 0, 0)?,
            )
        });
        assert!(out.committed(), "{:?}", out.failures);
        let s = fx.host();
        assert!(near(s.area, 16.11), "the elevation cut is unchanged: {}", s.area);
        assert!(near(s.volume, 16.11 * 0.35), "volume follows the new width: {}", s.volume);
        let st = fx.state(d);
        assert!(near(st.thickness, 0.35) && near(st.cut_volume, 1.89 * 0.35));
        assert!(near(st.sill_height, 0.0) && near(st.width, 0.9) && near(st.height, 2.1));
    }

    /// Retyping an instance re-cuts the host immediately.
    #[test]
    fn retyping_an_instance_re_cuts_its_host() {
        let mut fx = fixture();
        let d = fx.place(fx.door_type, 2.0);
        assert!(near(fx.host().area, 16.11));
        let window_type = fx.window_type;
        let out = fx.doc.transact("retype", false, |m, cx| set_hosted_instance_type(m, cx, d, window_type));
        assert!(out.committed(), "{:?}", out.failures);
        let st = fx.state(d);
        assert_eq!(st.kind, HostedKind::Window);
        assert!(near(st.cut_area, 1.44), "the new type's rectangle: {}", st.cut_area);
        // The sill is the instance's own stored value (0 from the door placement), NOT the new
        // type's default: retyping is not a re-placement (ASSUMED, section 3c).
        assert!(near(st.sill_height, 0.0) && near(st.head_height, 1.2));
        assert!(near(fx.host().area, 18.0 - 1.44));
        // Retyping to an id that is not a hosted type rejects before the change.
        let level = fx.level;
        let out = fx.doc.transact("retype-bad", false, |m, cx| set_hosted_instance_type(m, cx, d, level));
        assert!(!out.committed());
        assert_eq!(codes_of(&out), vec![codes::NOT_FOUND]);
    }

    // ------------------------------------------------------------ reject channel

    /// `insert.host_not_supported` / `element.not_found` / `insert.no_level`: section 4's
    /// *argument* problems, every one rejected before the change.
    #[test]
    fn a_bad_host_type_or_level_is_rejected_before_the_change() {
        let mut fx = fixture();
        let (level, wall_type, door_type, wall) = (fx.level, fx.wall_type, fx.door_type, fx.wall);
        for (label, host) in [("level", level), ("wall type", wall_type), ("unknown id", ElementId(9999))] {
            let out = fx.doc.transact("bad-host", false, |m, cx| {
                place_hosted_instance(
                    m,
                    cx,
                    NewHostedInstance { type_id: door_type, host, level, host_station: 2.0, sill_height: None },
                )
            });
            assert!(!out.committed(), "{label} must not host an instance");
            assert_eq!(codes_of(&out), vec![codes::INSERT_HOST_NOT_SUPPORTED], "{label}");
        }
        for (label, type_id) in [("a level", level), ("a wall type", wall_type), ("an unknown id", ElementId(9999))] {
            let out = fx.doc.transact("bad-type", false, |m, cx| {
                place_hosted_instance(
                    m,
                    cx,
                    NewHostedInstance { type_id, host: wall, level, host_station: 2.0, sill_height: None },
                )
            });
            assert!(!out.committed(), "{label} is not a hosted type");
            assert_eq!(codes_of(&out), vec![codes::NOT_FOUND], "{label}");
        }
        for (label, lvl) in [("a wall", wall), ("an unknown id", ElementId(9999))] {
            let out = fx.doc.transact("bad-level", false, |m, cx| {
                place_hosted_instance(
                    m,
                    cx,
                    NewHostedInstance { type_id: door_type, host: wall, level: lvl, host_station: 2.0, sill_height: None },
                )
            });
            assert!(!out.committed(), "{label} is not a level");
            assert_eq!(codes_of(&out), vec![codes::INSERT_NO_LEVEL], "{label}");
        }
        assert!(fx.doc.model().hosted_instances.is_empty(), "no instance survived a rejection");
    }

    /// `insert.bad_station`: a station or elevation that is not a number. ASTERION's own addition
    /// to the section 7d table, because a hosted instance's rectangle comes from a *station* and a
    /// *sill*, neither of which is a size.
    #[test]
    fn a_non_finite_station_or_sill_is_rejected_before_the_change() {
        let mut fx = fixture();
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let out = fx.try_place(fx.door_type, bad, None);
            assert!(!out.committed());
            assert_eq!(codes_of(&out), vec![codes::INSERT_BAD_STATION]);
            let out = fx.try_place(fx.door_type, 2.0, Some(bad));
            assert_eq!(codes_of(&out), vec![codes::INSERT_BAD_STATION]);
        }
        let d = fx.place(fx.door_type, 2.0);
        for (label, out) in [
            ("sill", fx.doc.transact("sill", false, |m, cx| set_sill_height(m, cx, d, f64::NAN))),
            ("head", fx.doc.transact("head", false, |m, cx| set_head_height(m, cx, d, f64::NAN))),
            ("station", fx.doc.transact("station", false, |m, cx| set_host_station(m, cx, d, f64::NAN))),
        ] {
            assert_eq!(codes_of(&out), vec![codes::INSERT_BAD_STATION], "{label}");
        }
        assert!(near(fx.state(d).sill_height, 0.0), "nothing was written");
    }

    /// `param.read_only`: ASSUMED (sections 3a, 7d) that `Level` and `Host Id` are read-only after
    /// placement, while sill, head and mark are writable. Writing the value already stored is a
    /// no-op, so the rejection is about *changing* the host or level.
    #[test]
    fn level_and_host_id_are_read_only_after_placement() {
        use crate::ops::HostedParamValue;
        let mut fx = fixture();
        let d = fx.place(fx.door_type, 2.0);
        let (level, upper, wall) = (fx.level, fx.upper, fx.wall);

        for (label, param, value) in [
            ("level", HostedInstanceParam::Level, HostedParamValue::Element(upper)),
            ("host", HostedInstanceParam::HostId, HostedParamValue::Element(ElementId(9999))),
        ] {
            let v = value.clone();
            let out = fx.doc.transact("ro", false, |m, cx| set_hosted_instance_param(m, cx, d, param, v.clone()));
            assert!(!out.committed(), "{label} must be read-only");
            assert_eq!(codes_of(&out), vec![codes::READ_ONLY], "{label}");
        }
        // Writing the stored value back is accepted as a no-op.
        for (param, value) in [
            (HostedInstanceParam::Level, HostedParamValue::Element(level)),
            (HostedInstanceParam::HostId, HostedParamValue::Element(wall)),
        ] {
            let out = fx.doc.transact("noop", false, |m, cx| set_hosted_instance_param(m, cx, d, param, value.clone()));
            assert!(out.committed(), "{:?}", out.failures);
        }
        let m = fx.doc.model();
        assert_eq!((m.hosted_instance(d).unwrap().level, m.hosted_instance(d).unwrap().host), (level, wall));
        assert!(HostedInstanceParam::Level.read_only() && HostedInstanceParam::HostId.read_only());
        assert!(!HostedInstanceParam::SillHeight.read_only() && !HostedInstanceParam::HeadHeight.read_only());
        assert!(!HostedInstanceParam::Mark.read_only());
    }

    /// `element.wrong_kind`: the reference's parameters are typed (`AsDouble`/`AsString`), so
    /// writing the wrong storage is a rejection rather than a coercion.
    #[test]
    fn writing_a_parameter_with_the_wrong_storage_type_is_rejected() {
        use crate::ops::HostedParamValue;
        let mut fx = fixture();
        let d = fx.place(fx.door_type, 2.0);
        let out = fx.doc.transact("wrong", false, |m, cx| {
            set_hosted_instance_param(m, cx, d, HostedInstanceParam::SillHeight, HostedParamValue::Text("x".into()))
        });
        assert!(!out.committed());
        assert_eq!(codes_of(&out), vec![codes::WRONG_KIND]);
        // OBSERVED: `mark` DOES NOT EXIST on an instance of a template-built family, so it is refused whatever storage it
        // is written with - the parameter check comes first.
        for v in [HostedParamValue::Length(3.0), HostedParamValue::Text("AST-D-01".into())] {
            let out = fx.doc.transact("mark", false, |m, cx| {
                set_hosted_instance_param(m, cx, d, HostedInstanceParam::Mark, v.clone())
            });
            assert!(!out.committed(), "mark does not exist on this family");
            assert_eq!(codes_of(&out), vec![codes::PARAM_NOT_PRESENT]);
        }
        assert_eq!(fx.doc.model().hosted_instance(d).unwrap().mark, "");
    }

    // ------------------------------------------------------- commit-failure channel

    // --------------------------------------------------------------------- cascades

    /// ASSUMED (section 4): a wall-hosted instance is "totally dependent" on its host, so deleting
    /// the host deletes it and its id appears in the returned deleted set - alongside the host's
    /// rectangular openings, through one cascade.
    #[test]
    fn deleting_the_host_wall_deletes_its_instances_and_its_openings() {
        let mut fx = fixture();
        let door = fx.place(fx.door_type, 2.0);
        let window = fx.place(fx.window_type, 4.5);
        let wall = fx.wall;
        let out = fx.doc.transact("open", false, |m, cx| add_rectangular_opening(m, cx, wall, 0.2, 1.0, 0.8, 2.0));
        assert!(out.committed(), "{:?}", out.failures);
        let opening = out.value.unwrap();

        let out = fx.doc.transact("delete-host", false, |m, cx| delete(m, cx, wall));
        assert!(out.committed(), "{:?}", out.failures);
        let mut expected = vec![wall, door, window, opening];
        expected.sort();
        assert_eq!(out.value.expect("deleted set"), expected);
        let m = fx.doc.model();
        assert!(m.hosted_instances.is_empty() && m.wall_openings.is_empty() && m.walls.is_empty());
        // The types survive - only their instances were dependent.
        assert_eq!(m.hosted_types.len(), 2);
    }

    /// The cascade is transitive through the level and the wall type, and a hosted **type** deletion takes its instances
    /// the way a floor type takes its floors (OBSERVED) - here ASSUMED for a loadable family.
    #[test]
    fn the_cascade_reaches_instances_through_the_level_the_wall_type_and_the_hosted_type() {
        for route in ["level", "wall_type", "hosted_type"] {
            let mut fx = fixture();
            let d = fx.place(fx.door_type, 2.0);
            let target = match route {
                "level" => fx.level,
                "wall_type" => fx.wall_type,
                _ => fx.door_type,
            };
            let out = fx.doc.transact("delete-root", false, |m, cx| delete(m, cx, target));
            assert!(out.committed(), "{route}: {:?}", out.failures);
            assert!(out.value.expect("deleted set").contains(&d), "{route}: the instance must be in the deleted set");
            assert!(fx.doc.model().hosted_instances.is_empty(), "{route}");
            // deleting the hosted type leaves the host wall standing, back to gross
            if route == "hosted_type" {
                let s = fx.host();
                assert!(near(s.area, 18.0) && near(s.volume, 3.6) && s.insert_count == 0);
            }
        }
    }

    /// An instance whose **level** is deleted goes with it even when its host survives - the two OBSERVED (step 32): an
    /// instance belongs to its HOST'S BASE LEVEL and no other. Placing one on a different level does not commit, so the two
    /// element sets can never differ
    /// - which makes the case this test was written for unreachable, and that is the finding.
    #[test]
    fn an_instance_may_only_be_placed_on_its_hosts_base_level() {
        let mut fx = fixture();
        let (wall, upper, door_type) = (fx.wall, fx.upper, fx.door_type);
        let out = fx.doc.transact("taller", false, |m, cx| set_unconnected_height(m, cx, wall, 6.0));
        assert!(out.committed(), "{:?}", out.failures);
        let out = fx.doc.transact("place-upper", false, |m, cx| {
            place_hosted_instance(
                m,
                cx,
                NewHostedInstance { type_id: door_type, host: wall, level: upper, host_station: 2.0, sill_height: Some(0.5) },
            )
        });
        assert!(!out.committed(), "a level above the host's base must be refused");
        assert_eq!(codes_of(&out), vec![codes::INSERT_WRONG_LEVEL]);

        // Deleting that level therefore cannot orphan an instance, because none can be on it.
        let out = fx.doc.transact("delete-upper-level", false, |m, cx| delete(m, cx, upper));
        assert!(out.committed(), "{:?}", out.failures);
        let m = fx.doc.model();
        assert!(m.hosted_instances.is_empty(), "nothing was ever placed on the upper level");
        assert!(m.walls.contains_key(&wall), "the host wall, based on the lower level, survives");

        // And the case that IS reachable: an instance on the host's own level goes with the host.
        let d = fx.place(fx.door_type, 2.0);
        let out = fx.doc.transact("delete-host", false, |m, cx| delete(m, cx, wall));
        assert!(out.committed(), "{:?}", out.failures);
        assert!(out.value.expect("deleted set").contains(&d), "the host carries its instance away");
    }

    /// Deleting an instance leaves the host standing with its quantities back to gross, and the
    /// instance itself has no dependents.
    #[test]
    fn deleting_an_instance_restores_the_hosts_gross_quantities() {
        let mut fx = fixture();
        let door = fx.place(fx.door_type, 2.0);
        let window = fx.place(fx.window_type, 4.5);
        assert!(fx.doc.model().dependents(door).is_empty(), "nothing depends on a hosted instance");

        let out = fx.doc.transact("delete-door", false, |m, cx| delete(m, cx, door));
        assert!(out.committed(), "{:?}", out.failures);
        assert_eq!(out.value.expect("deleted set"), vec![door]);
        let s = fx.host();
        assert!(near(s.area, 18.0 - 1.44) && near(s.volume, (18.0 - 1.44) * 0.2));
        assert_eq!(s.insert_count, 1);
        assert_eq!(fx.doc.model().hosted_instances_of(fx.wall), vec![window]);

        let out = fx.doc.transact("delete-window", false, |m, cx| delete(m, cx, window));
        assert!(out.committed());
        let s = fx.host();
        assert!(near(s.area, 18.0) && near(s.volume, 3.6) && near(s.cut_area, 0.0));
        assert_eq!((s.solid_face_count, s.solid_edge_count, s.insert_count), (6, 12, 0));
    }

    /// Undo must restore the host's net quantities, because they are derived and never stored.
    #[test]
    fn undo_restores_the_hosts_net_quantities_because_they_are_derived() {
        let mut fx = fixture();
        fx.place(fx.door_type, 2.0);
        assert!(near(fx.host().area, 16.11));
        assert!(fx.doc.undo().is_ok(), "the placement transaction is undoable");
        assert!(near(fx.host().area, 18.0), "after undo: {}", fx.host().area);
        assert!(fx.doc.redo().is_ok(), "and redoable");
        assert!(near(fx.host().area, 16.11), "after redo: {}", fx.host().area);
    }

    /// Section 7e compares the read-only flags and `host_face_null` as exact values, so they are
    /// pinned here rather than left implicit.
    #[test]
    fn the_documented_instance_flags_are_pinned() {
        assert!(HostedInstance::HOST_FACE_NULL, "a wall-hosted instance has no host FACE (section 1a)");
        assert!(HostedInstance::CAN_FLIP_FACING && HostedInstance::CAN_FLIP_HAND, "true for both kinds in v0.1");
        assert_eq!(HostedKind::from_name("door"), Some(HostedKind::Door));
        assert_eq!(HostedKind::from_name("window"), Some(HostedKind::Window));
        assert_eq!(HostedKind::from_name("curtain_panel"), None);
        for p in HostedTypeParam::ALL {
            assert_eq!(HostedTypeParam::from_name(p.name()), Some(p));
        }
        for p in HostedInstanceParam::ALL {
            assert_eq!(HostedInstanceParam::from_name(p.name()), Some(p));
        }
        // Both insert kinds are now really implemented; the other two stay reserved.
        assert!(crate::InsertKind::RectOpening.is_implemented());
        assert!(crate::InsertKind::HostedInstance.is_implemented());
        assert!(!crate::InsertKind::ShaftOpening.is_implemented());
        assert!(!crate::InsertKind::FaceOpening.is_implemented());
    }
}

#[cfg(test)]
mod level_rea_tests {
    use super::*;
    use asterion_core::Severity;

    fn three_levels() -> (crate::BimDocument, ElementId, ElementId, ElementId) {
        let mut doc = crate::new_document();
        let out = doc.transact("levels", false, |m, cx| {
            let a = create_level(m, cx, "A", 0.0)?;
            let b = create_level(m, cx, "B", 3.0)?;
            let c = create_level(m, cx, "C", 6.0)?;
            Ok((a, b, c))
        });
        let (a, b, c) = out.value.unwrap();
        (doc, a, b, c)
    }

    /// Spec L-02: creating a Level creates no plan view, and a new level has no story above and is a building story (Spec
    /// L-01: the flag is true at construction).
    #[test]
    fn l02_creating_a_level_makes_no_view() {
        let (doc, a, _, _) = three_levels();
        assert!(doc.model().views.is_empty());
        let l = doc.model().level(a).unwrap();
        assert_eq!(l.story_above, None);
        assert!(l.building_story);
    }

    /// Spec L-01: a story above that is not above the level (elevation below this level's + 1e-9) posts a warning on this
    /// level and the parameter is cleared.
    #[test]
    fn l01_a_story_above_that_moves_below_its_level_warns_and_is_cleared() {
        let (mut doc, a, b, _) = three_levels();
        assert!(doc.transact("set", false, |m, cx| set_level_story_above(m, cx, a, Some(b))).committed());
        assert_eq!(doc.model().level(a).unwrap().story_above, Some(b), "above and a building story: kept");
        // Move B to A's elevation: "story above elevation < elevation + 1e-9" holds on equality.
        let out = doc.transact("move", false, |m, cx| set_level_elevation(m, cx, b, 0.0));
        assert!(out.committed(), "the failure is a warning, the move commits");
        let f = out.failures.iter().find(|f| f.code == codes::LEVEL_STORY_ABOVE_MOVED).expect("warning posted");
        assert_eq!(f.severity, Severity::Warning);
        assert_eq!(f.elements, vec![a]);
        assert_eq!(doc.model().level(a).unwrap().story_above, None);
        // Nothing left to validate: a further move posts nothing.
        let out = doc.transact("again", false, |m, cx| set_level_elevation(m, cx, b, -1.0));
        assert!(out.failures.iter().all(|f| f.code != codes::LEVEL_STORY_ABOVE_MOVED));
    }

    /// Spec L-01: above but not a building story posts the other warning; a story above that is a building story and above
    /// changes nothing.
    #[test]
    fn l01_a_story_above_that_is_not_a_building_story_warns_and_is_cleared() {
        let (mut doc, a, b, _) = three_levels();
        assert!(doc.transact("set", false, |m, cx| set_level_story_above(m, cx, a, Some(b))).committed());
        // Changing the flag re-validates the levels that name B as their story above.
        let out = doc.transact("flag", false, |m, cx| set_level_building_story(m, cx, b, false));
        assert!(out.committed());
        assert!(out.failures.iter().any(|f| f.code == codes::LEVEL_STORY_ABOVE_NOT_A_STORY));
        assert_eq!(doc.model().level(a).unwrap().story_above, None);
        // Writing the parameter directly validates at once as well.
        let out = doc.transact("set again", false, |m, cx| set_level_story_above(m, cx, a, Some(b)));
        assert!(out.failures.iter().any(|f| f.code == codes::LEVEL_STORY_ABOVE_NOT_A_STORY));
        assert_eq!(doc.model().level(a).unwrap().story_above, None);
    }

    /// Spec L-01: an element that is not a Level only clears the parameter - and so does the deletion of the story-above
    /// level (UNKNOWN (Spec L-01): whether a warning is posted there).
    #[test]
    fn l01_a_story_above_that_is_not_a_level_or_is_deleted_is_just_cleared() {
        let (mut doc, a, b, c) = three_levels();
        let out = doc.transact("set", false, |m, cx| set_level_story_above(m, cx, a, Some(ElementId(999_999))));
        assert!(out.committed());
        assert!(out.failures.is_empty());
        assert_eq!(doc.model().level(a).unwrap().story_above, None);
        assert!(doc.transact("set", false, |m, cx| set_level_story_above(m, cx, b, Some(c))).committed());
        assert_eq!(doc.model().level(b).unwrap().story_above, Some(c));
        let out = doc.transact("del", false, |m, cx| delete(m, cx, c));
        assert!(out.committed());
        assert_eq!(doc.model().level(b).unwrap().story_above, None);
    }

    /// The moved level's own validation is not re-run by its own position change (UNKNOWN): only the levels that name it as
    /// their story above are.
    #[test]
    fn k27_a_move_revalidates_the_levels_that_name_the_moved_level() {
        let (mut doc, a, b, _) = three_levels();
        assert!(doc.transact("set", false, |m, cx| set_level_story_above(m, cx, a, Some(b))).committed());
        // Moving A itself (above B) re-validates nothing: the parameter is kept.
        let out = doc.transact("move a", false, |m, cx| set_level_elevation(m, cx, a, 10.0));
        assert!(out.committed());
        assert!(out.failures.is_empty());
        assert_eq!(doc.model().level(a).unwrap().story_above, Some(b));
    }
}

#[cfg(test)]
mod failure_row_tests {
    //! The CHANNEL, SEVERITY and FAILING ELEMENTS of the failure codes this module's operations raise, as the reference
    //! product failure-definition rows (`FR-<addr>`, specification) decide them. Each test names its rule.
    //!
    //! The two channels: `Err(Failure)` from the edit closure is "raised at the call"
    //! (`Outcome::value == None`); `cx.report` is "posted for the commit"
    //! (`Outcome::value == Some(..)` and the transaction rolled back when the failure is an Error).

    use super::*;
    use crate::model::{Layer, LayerFunction};
    use crate::spec::codes;
    use asterion_core::txn::TransactionStatus;
    use asterion_core::{ElementId, Severity};

    struct World {
        doc: crate::BimDocument,
        level: ElementId,
        upper: ElementId,
        wall_type: ElementId,
        wall: ElementId,
    }

    fn new_wall(wall_type: ElementId, level: ElementId, end_x: f64, height: f64) -> NewWall {
        NewWall {
            wall_type,
            level,
            start: Vec2::new(0.0, 0.0),
            end: Vec2::new(end_x, 0.0),
            height,
            base_offset: 0.0,
            flip: false,
            structural: false,
        }
    }

    /// Two levels (0 m, 3 m), a 0.2 m single-layer wall type and a 6 x 3 m wall on the lower one.
    fn world() -> World {
        let mut doc = crate::new_document();
        let out = doc.transact("fx", false, |m, cx| {
            let level = create_level(m, cx, "L0", 0.0)?;
            let upper = create_level(m, cx, "L1", 3.0)?;
            let wall_type = create_wall_type(
                m,
                cx,
                "WT-200",
                CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?,
            )?;
            let wall = create_wall(m, cx, new_wall(wall_type, level, 6.0, 3.0))?;
            Ok((level, upper, wall_type, wall))
        });
        assert!(out.committed(), "{:?}", out.failures);
        let (level, upper, wall_type, wall) = out.value.expect("committed");
        World { doc, level, upper, wall_type, wall }
    }

    fn door_type(w: &mut World) -> ElementId {
        let out = w.doc.transact("type", false, |m, cx| {
            create_hosted_type(
                m,
                cx,
                "AST-DOOR-900x2100",
                crate::model::HostedKind::Door,
                0.9,
                2.1,
                0.05,
                0.95,
                2.15,
                0.0,
                0.0,
            )
        });
        assert!(out.committed(), "{:?}", out.failures);
        out.value.expect("type")
    }

    /// The edit itself did not raise (the closure returned `Ok`), yet the transaction did not
    /// commit: the failure was posted for the commit.
    fn assert_posted_for_commit<T: std::fmt::Debug>(out: &asterion_core::txn::Outcome<T>) {
        assert!(out.value.is_some(), "the edit call must not raise: {:?}", out.failures);
        assert_eq!(out.status, TransactionStatus::RolledBack, "an Error posted at commit rolls back");
    }

    /// "every definition of this operation is posted for the commit ... names the wall's own element id as the failing
    /// element ... so each post uses the Error variant". A wall that is not valid or is too short is therefore a posted
    /// Error naming the wall, never an exception at the creation call.
    #[test]
    fn fr_07_a_wall_that_cannot_be_drawn_is_posted_for_commit_as_an_error_naming_the_wall() {
        let mut w = world();
        let (wt, l) = (w.wall_type, w.level);
        // 0.0019 m is above the short-curve tolerance of the creation call but not above width / 100 = 0.002 m (too short
        // iff NOT (w / 100 < L)).
        let out = w.doc.transact("short", false, |m, cx| create_wall(m, cx, new_wall(wt, l, 0.0019, 3.0)));
        assert_posted_for_commit(&out);
        let id = out.value.expect("the call returned the id before the commit failed");
        // the fixture's wall stands where the new one is: the overlap WARNING is posted too (OBSERVED)
        let real: Vec<&Failure> = out.failures.iter().filter(|f| f.code != codes::WALL_OVERLAPS_WALL).collect();
        assert_eq!(real.len(), 1, "{:?}", out.failures);
        let f = real[0];
        assert_eq!(f.code, codes::CANNOT_MAKE_WALL);
        assert_eq!(f.severity, Severity::Error);
        assert_eq!(f.elements, vec![id], "the wall's own id is the one failing element");
        assert!(!w.doc.model().walls.contains_key(&id), "rolled back: the wall does not exist");
    }

    /// : a wall with no top level whose height is below 0.1 ft fails and the same regeneration posts the draw failure as an
    /// Error for the commit, naming the wall. The creation CALL does not raise (the 30 000 ft range check is a separate,
    /// call-time rule).
    #[test]
    fn fr_07_a_wall_below_the_minimum_height_is_posted_for_commit_as_an_error() {
        let mut w = world();
        let (wt, l) = (w.wall_type, w.level);
        // 0.03 m is 0.0984 ft, under the inclusive 0.1 ft minimum.
        let out = w.doc.transact("low", false, |m, cx| create_wall(m, cx, new_wall(wt, l, 6.0, 0.03)));
        assert_posted_for_commit(&out);
        let id = out.value.expect("id");
        let f = out.failures.iter().find(|f| f.code != codes::WALL_OVERLAPS_WALL).expect("the posted error");
        assert_eq!((f.code.as_str(), f.severity), (codes::BELOW_MIN_HEIGHT, Severity::Error));
        assert_eq!(f.elements, vec![id]);
    }

    /// : the height derived from a top constraint that lands below the base is the negative-height failure, posted for the
    /// commit as an Error naming the wall; the edit call returns normally and the previous top is kept by the rollback.
    #[test]
    fn fr_07_a_top_constraint_below_the_base_is_posted_for_commit_and_rolls_back() {
        let mut w = world();
        let (wall, upper) = (w.wall, w.upper);
        let attach = w.doc.transact("attach", false, |m, cx| set_top_constraint(m, cx, wall, Some(upper), Some(0.0)));
        assert!(attach.committed(), "{:?}", attach.failures);
        // an offset of -3.5 m puts the top at -0.5 m, under the base at 0
        let out = w.doc.transact("below", false, |m, cx| set_top_constraint(m, cx, wall, Some(upper), Some(-3.5)));
        assert_posted_for_commit(&out);
        let f = &out.failures[0];
        assert_eq!((f.code.as_str(), f.severity), (codes::TOP_BELOW_BASE, Severity::Error));
        assert_eq!(f.elements, vec![wall]);
        assert_eq!(w.doc.model().wall(wall).unwrap().top_offset, 0.0, "the rollback kept the previous offset");
    }

    /// The message names two failing elements, the host id and the instance id (arguments 6 and 5)". A hosted instance left
    /// off its host by an edit of the host curve is a posted Error naming the host and then the instance.
    #[test]
    fn fr_09_an_instance_that_stops_cutting_is_posted_as_an_error_naming_host_then_instance() {
        let mut w = world();
        let (wall, level) = (w.wall, w.level);
        let type_id = door_type(&mut w);
        let placed = w.doc.transact("place", false, |m, cx| {
            place_hosted_instance(
                m,
                cx,
                NewHostedInstance { type_id, host: wall, level, host_station: 4.0, sill_height: None },
            )
        });
        assert!(placed.committed(), "{:?}", placed.failures);
        let door = placed.value.expect("placed");
        // shorten the host to 1 m: the door at station 4.0 no longer touches it
        let out = w.doc.transact("shorten", false, |m, cx| {
            set_wall_curve(m, cx, wall, Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0))
        });
        assert_posted_for_commit(&out);
        let f = out.failures.iter().find(|f| f.code == codes::INSERT_NOT_CUTTING_HOST).expect("posted");
        assert_eq!(f.severity, Severity::Error);
        assert_eq!(f.elements, vec![wall, door], "host first, instance second");
    }

    /// With Spec W-10g: the instance placed on a level that is not its host's base level is reported with the
    /// instance-not-cutting definition, whose one poster names the host and the instance - not the level.
    #[test]
    fn fr_09_a_wrong_level_instance_names_the_host_and_the_instance_not_the_level() {
        let mut w = world();
        let (wall, upper) = (w.wall, w.upper);
        let type_id = door_type(&mut w);
        let out = w.doc.transact("place", false, |m, cx| {
            place_hosted_instance(
                m,
                cx,
                NewHostedInstance { type_id, host: wall, level: upper, host_station: 2.0, sill_height: Some(0.5) },
            )
        });
        assert_posted_for_commit(&out);
        let id = out.value.expect("the instance exists until the rollback");
        let f = out.failures.iter().find(|f| f.code == codes::INSERT_WRONG_LEVEL).expect("posted");
        assert_eq!(f.severity, Severity::Error);
        assert_eq!(f.elements, vec![wall, id]);
        assert!(!f.elements.contains(&upper), "the level is not a failing element");
    }

    /// (the posted-warning class of the joined-elements-do-not-intersect message): the definition carries the Warning
    /// literal (1) and names the SET of the two element ids; a Warning does not roll the transaction back. The edit that
    /// moves the floor clear of the wall is applied, and the join is kept.
    #[test]
    fn fr_10_joined_elements_that_stop_intersecting_warn_and_the_transaction_commits() {
        let mut w = world();
        let (wall, level) = (w.wall, w.level);
        let out = w.doc.transact("floor", false, |m, cx| {
            let ft = create_floor_type(m, cx, "F-200", 0.2)?;
            let floor = create_floor(
                m,
                cx,
                ft,
                level,
                vec![Vec2::new(-1.0, -1.0), Vec2::new(7.0, -1.0), Vec2::new(7.0, 5.0), Vec2::new(-1.0, 5.0)],
                0.5,
                false,
            )?;
            join_elements(m, cx, floor, wall)?;
            Ok(floor)
        });
        assert!(out.committed() && out.failures.is_empty(), "{:?}", out.failures);
        let floor = out.value.expect("floor");
        // 1 m below the level: the 0.2 m slab spans z -1.2 .. -1.0, clear of the wall (0 .. 3)
        let out = w.doc.transact("lower", false, |m, cx| set_floor_height_offset(m, cx, floor, -1.0));
        assert!(out.committed(), "a Warning is kept: {:?}", out.failures);
        let warn: Vec<_> = out.failures.iter().filter(|f| f.code == codes::JOIN_DO_NOT_INTERSECT).collect();
        assert_eq!(warn.len(), 1, "{:?}", out.failures);
        assert_eq!(warn[0].severity, Severity::Warning);
        let mut named = warn[0].elements.clone();
        named.sort();
        let mut both = vec![floor, wall];
        both.sort();
        assert_eq!(named, both, "the failing elements are the set of the two joined ids");
        assert_eq!(w.doc.model().geometry_joins.len(), 1, "the join outlives the overlap");
    }

    /// The sketch validators post nothing and their poster is outside the reference product, so the rules do NOT decide the
    /// channel of a refused floor sketch. This test pins the engine's OBSERVED channel (rounds 10 and 13: raised at the
    /// call, nothing created) so that a future row that decides it differently shows up as a deliberate change.
    #[test]
    fn fr_02_the_sketch_fault_channel_is_the_observed_one_not_a_row_decision() {
        let mut w = world();
        let level = w.level;
        let out = w.doc.transact("bowtie", false, |m, cx| {
            let ft = create_floor_type(m, cx, "F-200", 0.2)?;
            create_floor(
                m,
                cx,
                ft,
                level,
                vec![Vec2::new(0.0, 0.0), Vec2::new(4.0, 4.0), Vec2::new(4.0, 0.0), Vec2::new(0.0, 4.0)],
                0.0,
                false,
            )
        });
        assert!(out.value.is_none(), "raised at the call (OBSERVED), not posted");
        assert_eq!(out.failures.len(), 1);
        assert_eq!(out.failures[0].code, codes::SLAB_LOOPS_INTERSECT);
    }
}

#[cfg(test)]
mod view_name_tests {
    //! A view name is a duplicate only when the formatted titles are equal (one format per view kind, ASSUMED while the
    //! formats are not specified).
    use super::*;
    use crate::spec::codes;

    #[test]
    fn a_floor_plan_and_a_ceiling_plan_may_share_a_name_two_floor_plans_may_not() {
        let mut doc = crate::new_document();
        let out = doc.transact("views", false, |m, cx| {
            let l = create_level(m, cx, "L-V4", 0.0)?;
            let f = create_view(m, cx, "Shared", ViewKind::FloorPlan, Some(l), None)?;
            let c = create_view(m, cx, "Shared", ViewKind::CeilingPlan, Some(l), None)?;
            Ok((l, f, c))
        });
        let (l, f, c) = out.value.expect("a floor plan and a ceiling plan of one name are accepted");
        assert_ne!(f, c);
        let dup = doc.transact("dup", false, |m, cx| create_view(m, cx, "Shared", ViewKind::FloorPlan, Some(l), None));
        assert!(!dup.committed());
        assert_eq!(dup.failures[0].code, codes::VIEW_DUPLICATE_NAME);
        // a rename follows the same rule: the ceiling plan may take a floor plan's name, not another ceiling plan's
        let other = doc.transact("c2", false, |m, cx| create_view(m, cx, "Other", ViewKind::CeilingPlan, Some(l), None)).value.unwrap();
        let ok = doc.transact("rename", false, |m, cx| {
            create_view(m, cx, "Plan Only", ViewKind::FloorPlan, Some(l), None)?;
            set_view_name(m, cx, other, "Plan Only")
        });
        assert!(ok.committed());
        let refused = doc.transact("rename2", false, |m, cx| set_view_name(m, cx, other, "Shared"));
        assert!(!refused.committed());
        assert_eq!(refused.failures[0].code, codes::VIEW_DUPLICATE_NAME);
    }
}
