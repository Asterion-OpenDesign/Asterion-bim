//! Split Element and Scale of walls.
//!
//! Written specification of the specification rules Spec SPLIT-01, SCALE-01 (the reference product, the wall's split
//! operation and). What the rules do not specified is named below with its question and the ENGINE CHOICE that stands in;
//! none of those choices is a finding.
//!
//! # Split (Spec SPLIT-01)
//!
//! * **Refusals.** The element must be a wall (Spec SPLIT-01: "error 1 when the element is not a wall" -> `WRONG_KIND`).
//!   The split point must lie on the location curve (Spec SPLIT-01): the engine takes it as a station along the curve and
//!   refuses a station outside the curve or on an end (the rule reads the "strictly inside" case; what an end does is not
//!   specified). The tolerance of the on-curve test is not specified: ENGINE CHOICE, the vertex tolerance (0.1595 mm,
//!   `spec::insert_fit_tolerance`).
//! * **The cut (Spec SPLIT-01).** `vertical = false` is the split along the location curve; the new wall's id is returned,
//!   and the original wall's id when no new wall results. The `vertical = true` cut and the walls the engine does not model
//!   (infill, curtain, tilted cross-section) are not built: refused. Which part keeps the original id is NOT specified.
//!   ENGINE CHOICE: the original id keeps the part from the start to the split point, the new wall takes the rest.
//! * **Join geometry (Spec SPLIT-01).** The join geometry is duplicated for the new wall: every geometry join of the
//!   original wall is repeated for the new one with the same cutter relation. Wall joins themselves are derived from the
//!   geometry at every regeneration, so the two parts join each other by position.
//!
//! # Scale (Spec SCALE-01)
//!
//! * A wall's location curve is scaled in the plan about the picked origin (Spec SCALE-01): a straight wall keeps its
//!   middle point scaled (`f x mid + (1 - f) x origin`) and its length multiplied by the factor, which is the uniform
//!   scaling of its two end points about the origin; a curved wall's curve is transformed by the uniform scaling about the
//!   origin (Spec SCALE-01: its three defining points are).
//! * The key reference offset of the wall's type (Spec SCALE-01: the curve is offset by it, scaled and offset back) is not
//!   specified: the engine's location curve IS the stored curve, taken at offset 0.
//! * A scaled curve shorter than the vertex tolerance posts the "curve too short" failure with the wall as the failing
//!   element and the curve is stored (Spec SCALE-01). Whether it is an error that rolls back or a warning is not specified:
//!   ENGINE CHOICE, an error at commit like every curve-too-short of the engine.
//! * The scale plane's normal is parallel to the wall's curve normal for every plan scale of a vertical wall (Spec
//!   SCALE-01), so that refusal cannot arise here.
//! * Not specified (Spec SCALE-01 not specified. Elements that are not walls are refused (`transform.not_supported`): the
//!   rule reads walls only).

//! # Trim / Extend to a boundary (Spec TRIMEXT-01)
//!
//! * **Which end moves (Spec TRIMEXT-01).** The end of the wall's curve that is FARTHER from the clicked point; the click
//!   selects the part to keep, the other end stays.
//! * **Where it goes (Spec TRIMEXT-01).** To the parameter `t` of the intersection of the wall's curve with the boundary's
//!   curve (a wall's code 2: "the moved end goes to t"). The operation that computes `t` and the fence extraction are not
//!   specified: ENGINE CHOICE, the intersection of the two location LINES (the curve extended: an extend reaches a boundary
//!   the wall does not touch); parallel lines are refused.
//! * **Allowed (Spec TRIMEXT-01):** an element with an editable curve. The engine trims and extends straight walls against
//!   straight walls; arc walls are refused.
//! * **Not specified (ENGINE CHOICE, named):** the joins of the trimmed wall (joins are derived from the geometry at every
//!   regeneration, so the moved end joins whatever it now touches); the openings and hosted elements (they keep their
//!   position in the model - their station is re-measured from the moved start - and one the wall no longer holds fails the
//!   commit-time fit check); and the Trim/Extend to Corner variant (moves both ends to their intersection; only inferred
//!   from the resource prompts, so NOT implemented:).

use crate::model::{BimModel, GeometryJoin, Wall};
use crate::spec::codes;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use asterion_geom::Vec2;

/// A wall that exists, or the failure the engine raises at the call.
fn wall_of(m: &BimModel, id: ElementId) -> Result<&Wall, Failure> {
    if let Some(w) = m.walls.get(&id) {
        return Ok(w);
    }
    match crate::transform::kind_of(m, id) {
        Err(e) if e.code == codes::NOT_FOUND => Err(e),
        _ => Err(Failure::error(codes::WRONG_KIND, format!("Element {} is not a wall.", id.0)).on(&[id])),
    }
}

fn refused(id: ElementId, why: &str) -> Failure {
    Failure::error(codes::WALL_SPLIT_REFUSED, format!("The wall cannot be split: {why}.")).on(&[id])
}

/// Split a straight wall at `station` metres from its start. Returns the id of the new wall.
///
/// Spec SPLIT-01; see the module documentation for what is read and the ENGINE CHOICEs.
pub fn split_wall(m: &mut BimModel, cx: &mut TxnContext<'_>, wall: ElementId, station: f64) -> Result<ElementId, Failure> {
    let w = wall_of(m, wall)?.clone();
    if w.through.is_some() {
        return Err(refused(wall, "arc walls are not split by the engine (OQ-SPLIT-ARC)"));
    }
    let length = (w.end - w.start).length();
    let tol = crate::spec::insert_fit_tolerance();
    if !station.is_finite() || station <= tol || station >= length - tol {
        return Err(refused(wall, "the split point is not strictly inside the location curve"));
    }
    let dir = (w.end - w.start).normalized().ok_or_else(|| refused(wall, "the wall has no length"))?;
    let point = w.start + dir * station;

    // the inserts hosted by the wall: those cut by the split point are refused, the others go to the part that contains
    // them (ENGINE CHOICE)
    let mut to_new_instances: Vec<ElementId> = Vec::new();
    for i in m.hosted_instances.values().filter(|i| i.host == wall) {
        let half = m.hosted_types.get(&i.type_id).map_or(0.0, |t| i.width_in(t) * 0.5);
        let (lo, hi) = (i.host_station - half, i.host_station + half);
        if lo < station - tol && hi > station + tol {
            return Err(refused(wall, "the split point falls inside a door or window"));
        }
        if i.host_station >= station {
            to_new_instances.push(i.id);
        }
    }
    let mut to_new_openings: Vec<ElementId> = Vec::new();
    for o in m.wall_openings.values().filter(|o| o.host == wall) {
        if o.u_min < station - tol && o.u_max > station + tol {
            return Err(refused(wall, "the split point falls inside an opening"));
        }
        if o.u_min >= station - tol {
            to_new_openings.push(o.id);
        }
    }

    let new_id = cx.new_id();
    let mut part = w.clone();
    part.id = new_id;
    part.start = point;
    part.join_allowed = [true, w.join_allowed[1]];
    {
        let original = m.walls.get_mut(&wall).expect("checked");
        original.end = point;
        original.join_allowed = [w.join_allowed[0], true];
    }
    m.walls.insert(new_id, part);
    for id in &to_new_instances {
        if let Some(i) = m.hosted_instances.get_mut(id) {
            i.host = new_id;
            i.host_station -= station;
        }
    }
    for id in &to_new_openings {
        if let Some(o) = m.wall_openings.get_mut(id) {
            o.host = new_id;
            o.u_min -= station;
            o.u_max -= station;
        }
    }
    // the join geometry of the original is repeated for the new wall (Spec SPLIT-01)
    let duplicated: Vec<GeometryJoin> = m
        .geometry_joins
        .iter()
        .filter(|j| j.involves(wall))
        .map(|j| GeometryJoin {
            a: if j.a == wall { new_id } else { j.a },
            b: if j.b == wall { new_id } else { j.b },
            cutter: if j.cutter == wall { new_id } else { j.cutter },
        })
        .collect();
    m.geometry_joins.extend(duplicated);

    let mut touched = vec![wall, new_id];
    touched.extend(to_new_instances);
    touched.extend(to_new_openings);
    crate::ops::check_walls(m, cx, &[wall, new_id]);
    crate::ops::check_joins(m, cx, &touched);
    let mut inserts = touched.clone();
    inserts.extend(m.dependents(wall));
    inserts.extend(m.dependents(new_id));
    crate::ops::check_inserts(m, cx, &inserts);
    Ok(new_id)
}

/// Split a straight wall at the location-curve point nearest to `point`, refused when `point` is farther from the curve
/// than the tolerance (Spec SPLIT-01: the point must be on the curve).
pub fn split_wall_at_point(m: &mut BimModel, cx: &mut TxnContext<'_>, wall: ElementId, point: Vec2) -> Result<ElementId, Failure> {
    let w = wall_of(m, wall)?;
    let (start, end) = (w.start, w.end);
    let dir = (end - start).normalized().ok_or_else(|| refused(wall, "the wall has no length"))?;
    let station = (point - start).dot(dir);
    let on_curve = start + dir * station;
    if (point - on_curve).length() > crate::spec::insert_fit_tolerance() {
        return Err(refused(wall, "the split point is not on the location curve"));
    }
    split_wall(m, cx, wall, station)
}

/// Which member of an array the translation / rotation locates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrayAnchor {
    /// The vector / angle is the step between members: the second member sits one step away.
    Second,
    /// The vector / angle locates the LAST member: the step is it divided by (count - 1).
    Last,
}

/// Largest member count of an array (`LinearArray.IsValidArraySize`: 2 to 200; radial 3 to 200).
pub const MAX_ARRAY_MEMBERS: usize = 200;

/// The first listed hosted instance whose host is not listed (it cannot be arrayed on its own).
fn lone_hosted(m: &BimModel, ids: &[ElementId]) -> Option<ElementId> {
    ids.iter().copied().find(|i| m.hosted_instances.get(i).map_or(false, |h| !ids.contains(&h.host)))
}

fn array_invalid(why: &str) -> Failure {
    Failure::error(codes::ARRAY_INVALID, format!("The array cannot be made: {why}."))
}

/// Linear array of `ids`: `count` members INCLUDING the originals, the others copies shifted by
/// `vector` (`Second`: the second member is `vector` away, each next one another `vector`) or spread
/// to put the last member `vector` away (`Last`). Returns the `(original, copy)` pairs, member by
/// member.
///
/// A view is required; the selection is grouped, copied ONCE through the paste clipboard, that first copy is MOVED by the
/// vector, the array's new-element-position field is set to 1 for the anchor `Second` and 2 for `Last`, the array is
/// brought to the requested COUNT (a count below 2 is a debug warning; the minimum size is 2) and any failure rolls the
/// sub-transaction back ("Failed to create the linear array."). UNKNOWN: how positions 1 and 2 place the members - the
/// spacing equal to the vector for `Second` and the vector spread over the count for `Last` are the OBSERVED rule below.
///
/// OBSERVED: the count is 2 to 200 ("count must be between 2 and 200"), the members INCLUDE the original (count 3 makes 2
/// copies), a zero translation COMMITS and the coincident members post one overlap warning per pair of walls, and a hosted
/// instance on its own cannot be arrayed ("An internal error has occurred": refused here). The reference product's
/// associated array makes GROUPS of the copies; the engine makes plain elements (no group, no array element) so only the
/// members' own geometry can be compared. Every member is made by [`crate::transform::copy_elements`], so the doors of a
/// copied wall come along (OBSERVED for a copy).
pub fn array_linear(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    ids: &[ElementId],
    count: usize,
    vector: Vec2,
    anchor: ArrayAnchor,
) -> Result<Vec<(ElementId, ElementId)>, Failure> {
    array_linear_with(m, cx, ids, count, vector, anchor, false)
}

/// The order the members are made in, which is the order their ids are handed out in. OBSERVED (linear and radial, counts 3
/// and 4): anchor `Second` makes member 1, 2, ... count - 1 in order (the associated array's group members and
/// `ArrayElementWithoutAssociation` alike); anchor `Last` (`ArrayElementWithoutAssociation`) makes the LAST member first,
/// then 1, 2, ... count - 2. `Last` of an associated array was not measured (the same order is kept).
fn member_order(count: usize, anchor: ArrayAnchor) -> Vec<usize> {
    match anchor {
        ArrayAnchor::Second => (1..count).collect(),
        ArrayAnchor::Last => {
            let mut v = vec![count - 1];
            v.extend(1..count - 1);
            v
        }
    }
}

/// [`array_linear`] with the association flag: `associate` is the reference product's `LinearArray.Create` (a GROUP of the
/// copies; OBSERVED: the original and the copies read `Pinned = true`), false its `ArrayElementWithoutAssociation` (plain
/// copies). The engine has no group: the copies are plain elements either way, and `associate` pins the originals and the
/// copies. The doors of an arrayed wall come along (OBSERVED: the copied wall is net of the door's cut, as for a copy; the
/// group lists the wall alone).
pub fn array_linear_with(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    ids: &[ElementId],
    count: usize,
    vector: Vec2,
    anchor: ArrayAnchor,
    associate: bool,
) -> Result<Vec<(ElementId, ElementId)>, Failure> {
    if !(2..=MAX_ARRAY_MEMBERS).contains(&count) {
        return Err(array_invalid("a linear array has 2 to 200 members"));
    }
    if !(vector.x.is_finite() && vector.y.is_finite()) {
        return Err(array_invalid("the translation must be a finite vector"));
    }
    if ids.is_empty() {
        return Err(array_invalid("there is nothing to array"));
    }
    if let Some(l) = lone_hosted(m, ids) {
        return Err(array_invalid("a door or window alone cannot be arrayed").on(&[l]));
    }
    let step = match anchor {
        ArrayAnchor::Second => vector,
        ArrayAnchor::Last => vector * (1.0 / (count - 1) as f64),
    };
    let mut pairs = Vec::new();
    for k in member_order(count, anchor) {
        let t = crate::transform::PlanTransform::Translate(step * k as f64);
        pairs.extend(crate::transform::copy_elements_with(m, cx, ids, Some(t), true)?);
    }
    if associate {
        // the originals AND the copies (the members of the groups) read pinned (OBSERVED)
        m.pinned.extend(ids.iter().copied());
        m.pinned.extend(pairs.iter().map(|(_, c)| *c));
    }
    Ok(pairs)
}

/// Radial array of `ids` about the vertical axis through `center`: `count` members including the
/// originals (3 to 200), the others copies rotated by `angle` (`Second`: the step between members)
/// or spread to put the last member `angle` away (`Last`). Returns the `(original, copy)` pairs.
///
/// OBSERVED: the count is 3 to 200 ("count must be between 3 and 200"), an angle of zero is refused ("Angle value must be
/// not zero"), and a door arrayed on its own posts "Can't cut instance of <door> out of Wall." per member (errors: the
/// transaction rolls back). The other notes are those of [`array_linear`].
pub fn array_radial(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    ids: &[ElementId],
    count: usize,
    center: Vec2,
    angle: f64,
    anchor: ArrayAnchor,
) -> Result<Vec<(ElementId, ElementId)>, Failure> {
    array_radial_with(m, cx, ids, count, center, angle, anchor, false)
}

/// [`array_radial`] with the association flag (see [`array_linear_with`]).
#[allow(clippy::too_many_arguments)]
pub fn array_radial_with(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    ids: &[ElementId],
    count: usize,
    center: Vec2,
    angle: f64,
    anchor: ArrayAnchor,
    associate: bool,
) -> Result<Vec<(ElementId, ElementId)>, Failure> {
    if !(3..=MAX_ARRAY_MEMBERS).contains(&count) {
        return Err(array_invalid("a radial array has 3 to 200 members"));
    }
    // An angle is valid when |angle| > 1e-9
    if !(center.x.is_finite() && center.y.is_finite() && angle.is_finite()) || angle.abs() <= 1e-9 {
        return Err(array_invalid("the centre must be finite and the angle a finite number that is not zero"));
    }
    if ids.is_empty() {
        return Err(array_invalid("there is nothing to array"));
    }
    // A full turn (|angle| = 2 pi, relative 1e-9) forces the anchor to Last; an angle above pi (and not pi) is replaced by
    // angle - 2 pi.
    let tau = std::f64::consts::TAU;
    let (angle, anchor) = if (angle.abs() - tau).abs() <= 1e-9 * tau {
        (angle, ArrayAnchor::Last)
    } else if angle > std::f64::consts::PI && (angle - std::f64::consts::PI).abs() > 1e-12 {
        (angle - tau, anchor)
    } else {
        (angle, anchor)
    };
    let step = match anchor {
        ArrayAnchor::Second => angle,
        ArrayAnchor::Last => angle / (count - 1) as f64,
    };
    // a door or window listed without its wall cannot follow a rotation: every member it would make cannot cut its host
    // (OBSERVED); the rest of the list is arrayed
    let lone: Vec<ElementId> = ids.iter().copied().filter(|i| m.hosted_instances.get(i).map_or(false, |h| !ids.contains(&h.host))).collect();
    let rest: Vec<ElementId> = ids.iter().copied().filter(|i| !lone.contains(i)).collect();
    for l in &lone {
        let host = m.hosted_instances[l].host;
        for _ in 1..count {
            cx.report(
                Failure::error(codes::INSERT_NOT_CUTTING_HOST, "Can't cut the instance out of its host wall.").on(&[*l, host]),
            );
        }
    }
    let mut pairs = Vec::new();
    if !rest.is_empty() {
        for k in member_order(count, anchor) {
            let t = crate::transform::PlanTransform::Rotate { center, angle: step * k as f64 };
            pairs.extend(crate::transform::copy_elements_with(m, cx, &rest, Some(t), true)?);
        }
        if associate {
            m.pinned.extend(rest.iter().copied());
            m.pinned.extend(pairs.iter().map(|(_, c)| *c));
        }
    }
    Ok(pairs)
}

/// Trim or extend `wall` to the location line of `boundary` (Spec TRIMEXT-01). The end of the wall's curve farther from
/// `click` moves to the intersection of the two lines.
pub fn trim_extend_wall(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    wall: ElementId,
    click: Vec2,
    boundary: ElementId,
) -> Result<(), Failure> {
    let refuse = |id: ElementId, why: &str| Failure::error(codes::WALL_TRIM_REFUSED, format!("Trim/Extend refused: {why}.")).on(&[id]);
    let w = wall_of(m, wall)?.clone();
    let b = wall_of(m, boundary)?.clone();
    if wall == boundary {
        return Err(refuse(wall, "the boundary is the wall itself"));
    }
    if w.through.is_some() || b.through.is_some() {
        return Err(refuse(wall, "an arc wall is not trimmed or extended by the engine (OQ-TRIMEXT-ARC)"));
    }
    let length = (w.end - w.start).length();
    let d = (w.end - w.start).normalized().ok_or_else(|| refuse(wall, "the wall has no length"))?;
    let e = (b.end - b.start).normalized().ok_or_else(|| refuse(boundary, "the boundary has no length"))?;
    if !(click.x.is_finite() && click.y.is_finite()) {
        return Err(refuse(wall, "the clicked point is not a number"));
    }
    // start + d t on the boundary line: (start + d t - b.start) x e = 0
    let denom = d.cross(e);
    if denom.abs() < 1e-9 {
        return Err(refuse(wall, "the wall and the boundary are parallel"));
    }
    let t = (b.start - w.start).cross(e) / denom;
    let tol = crate::spec::insert_fit_tolerance();
    // the end farther from the click moves (Spec TRIMEXT-01); a tie moves the end
    let move_end = (click - w.end).length() >= (click - w.start).length();
    let (new_start, new_end, shift) = if move_end {
        if t <= tol {
            return Err(refuse(wall, "the moved end would cross the other end"));
        }
        (w.start, w.start + d * t, 0.0)
    } else {
        if t >= length - tol {
            return Err(refuse(wall, "the moved end would cross the other end"));
        }
        (w.start + d * t, w.end, t)
    };
    {
        let target = m.walls.get_mut(&wall).expect("checked");
        target.start = new_start;
        target.end = new_end;
    }
    // inserts keep their place in the model: the stations are measured from the (moved) start
    let mut touched = vec![wall];
    if shift != 0.0 {
        for i in m.hosted_instances.values_mut().filter(|i| i.host == wall) {
            i.host_station -= shift;
            touched.push(i.id);
        }
        for o in m.wall_openings.values_mut().filter(|o| o.host == wall) {
            o.u_min -= shift;
            o.u_max -= shift;
            touched.push(o.id);
        }
    } else {
        touched.extend(m.dependents(wall));
    }
    crate::ops::check_walls(m, cx, &[wall]);
    crate::ops::check_joins(m, cx, &touched);
    let mut inserts = touched.clone();
    inserts.extend(m.dependents(wall));
    crate::ops::check_inserts(m, cx, &inserts);
    Ok(())
}

/// Scale walls in the plan about `origin` by `factor` (Spec SCALE-01). Returns the ids handled.
///
/// An element that is not a wall is refused (`transform.not_supported`). A wall left shorter than the vertex tolerance
/// posts the `geom.curve_too_short` failure on it and keeps the scaled curve (Spec SCALE-01).
pub fn scale_elements(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    ids: &[ElementId],
    origin: Vec2,
    factor: f64,
) -> Result<Vec<ElementId>, Failure> {
    if !(factor.is_finite() && factor > 0.0 && origin.x.is_finite() && origin.y.is_finite()) {
        return Err(Failure::error(codes::TRANSFORM_INVALID, "A scale needs a finite origin and a finite factor above zero."));
    }
    let mut list: Vec<ElementId> = Vec::new();
    for id in ids {
        if !list.contains(id) {
            list.push(*id);
        }
    }
    for id in &list {
        if m.walls.contains_key(id) {
            continue;
        }
        return Err(match crate::transform::kind_of(m, *id) {
            Err(e) if e.code == codes::NOT_FOUND => e,
            _ => Failure::error(codes::TRANSFORM_NOT_SUPPORTED, "Only walls are scaled (Spec SCALE-01 states the wall's scale).").on(&[*id]),
        });
    }
    let tol = crate::spec::insert_fit_tolerance();
    for id in &list {
        let w = m.walls.get_mut(id).expect("checked");
        let scale = |p: Vec2| origin + (p - origin) * factor;
        w.start = scale(w.start);
        w.end = scale(w.end);
        w.through = w.through.map(scale);
        if (w.end - w.start).length() < tol {
            cx.report(Failure::error(codes::CURVE_TOO_SHORT, "The scaled wall curve is too short.").on(&[*id]));
        }
    }
    crate::ops::check_walls(m, cx, &list);
    crate::ops::check_joins(m, cx, &list);
    let mut inserts = list.clone();
    for w in &list {
        inserts.extend(m.dependents(*w));
    }
    crate::ops::check_inserts(m, cx, &inserts);
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HostedKind;
    use crate::{ops, BimDocument, CompoundStructure, Layer, LayerFunction};

    struct Fx {
        doc: BimDocument,
        wall: ElementId,
        door: ElementId,
        window: ElementId,
        opening: ElementId,
    }

    fn fixture() -> Fx {
        let mut doc = crate::new_document();
        let out = doc.transact("fx", false, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let cs = CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?;
            let wt = ops::create_wall_type(m, cx, "W", cs)?;
            let v = Vec2::new;
            let wall = ops::create_wall(
                m,
                cx,
                ops::NewWall { wall_type: wt, level: l, start: v(0.0, 0.0), end: v(10.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false },
            )?;
            let dt = ops::create_hosted_type(m, cx, "D", HostedKind::Door, 1.0, 2.0, 0.05, 1.0, 2.0, 0.0, 0.0)?;
            let wnt = ops::create_hosted_type(m, cx, "Wn", HostedKind::Window, 1.0, 1.0, 0.05, 1.0, 1.0, 0.9, 0.0)?;
            let door = ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: dt, host: wall, level: l, host_station: 2.0, sill_height: None })?;
            let window = ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: wnt, host: wall, level: l, host_station: 8.0, sill_height: None })?;
            let opening = ops::add_rectangular_opening(m, cx, wall, 6.0, 1.0, 7.0, 2.0)?;
            Ok((wall, door, window, opening))
        });
        let (wall, door, window, opening) = out.value.expect("fixture");
        Fx { doc, wall, door, window, opening }
    }

    #[test]
    fn a_split_makes_two_walls_and_rehosts_the_inserts_by_station() {
        let mut f = fixture();
        let out = f.doc.transact("split", false, |m, cx| split_wall(m, cx, f.wall, 5.0));
        assert!(out.committed(), "{:?}", out.failures);
        let new = out.value.unwrap();
        let m = f.doc.model();
        assert_ne!(new, f.wall);
        assert_eq!((m.walls[&f.wall].start, m.walls[&f.wall].end), (Vec2::new(0.0, 0.0), Vec2::new(5.0, 0.0)));
        assert_eq!((m.walls[&new].start, m.walls[&new].end), (Vec2::new(5.0, 0.0), Vec2::new(10.0, 0.0)));
        assert_eq!(m.walls[&new].type_id, m.walls[&f.wall].type_id);
        // the door stays with the first part, the window moves with its station kept from the new start
        assert_eq!((m.hosted_instances[&f.door].host, m.hosted_instances[&f.door].host_station), (f.wall, 2.0));
        assert_eq!((m.hosted_instances[&f.window].host, m.hosted_instances[&f.window].host_station), (new, 3.0));
        // the opening 6..7 lies on the second part: its range is kept from the new start
        let o = &m.wall_openings[&f.opening];
        assert_eq!((o.host, o.u_min, o.u_max), (new, 1.0, 2.0));
    }

    #[test]
    fn a_split_through_an_insert_or_outside_the_curve_is_refused_and_changes_nothing() {
        let mut f = fixture();
        let before = f.doc.model().clone();
        // 6.5 falls inside the opening 6..7, 2.0 is the door's centre, 8.2 is inside the window
        for station in [0.0, 10.0, -1.0, 11.0, f64::NAN, 2.0, 6.5, 8.2] {
            let out = f.doc.transact("split", false, |m, cx| split_wall(m, cx, f.wall, station));
            assert!(out.value.is_none(), "{station}");
        }
        assert_eq!(f.doc.model(), &before);
        // not a wall / not found
        let e = f.doc.transact("x", false, |m, cx| split_wall(m, cx, f.door, 1.0));
        assert!(e.value.is_none());
        let e = f.doc.transact("x", false, |m, cx| split_wall(m, cx, ElementId(9_999_999), 1.0));
        assert!(e.value.is_none());
        assert_eq!(f.doc.model(), &before);
    }

    #[test]
    fn a_split_at_a_point_off_the_curve_is_refused() {
        let mut f = fixture();
        let out = f.doc.transact("p", false, |m, cx| split_wall_at_point(m, cx, f.wall, Vec2::new(7.0, 0.5)));
        assert!(out.value.is_none());
        let out = f.doc.transact("p", false, |m, cx| split_wall_at_point(m, cx, f.wall, Vec2::new(7.0, 0.0)));
        assert!(out.committed(), "{:?}", out.failures);
    }

    #[test]
    fn a_split_undoes_in_one_step_and_repeats_the_join_geometry() {
        let mut f = fixture();
        let slab = f
            .doc
            .transact("slab", false, |m, cx| {
                let ft = ops::create_floor_type(m, cx, "F", 0.2)?;
                let l = *m.levels.keys().next().unwrap();
                let v = Vec2::new;
                let s = ops::create_slab(m, cx, ft, l, vec![v(-1.0, -1.0), v(11.0, -1.0), v(11.0, 1.0), v(-1.0, 1.0)], 0.0, false)?;
                ops::join_elements(m, cx, s, f.wall)?;
                Ok(s)
            })
            .value
            .expect("slab");
        let before = f.doc.model().clone();
        let new = f.doc.transact("split", false, |m, cx| split_wall(m, cx, f.wall, 3.0)).value.expect("split");
        let m = f.doc.model();
        assert!(m.geometry_joins.iter().any(|j| j.involves(new) && j.involves(slab)), "{:?}", m.geometry_joins);
        assert!(m.geometry_joins.iter().any(|j| j.involves(f.wall) && j.involves(slab)));
        assert!(f.doc.undo().is_ok());
        assert_eq!(f.doc.model(), &before);
    }

    #[test]
    fn scaling_a_wall_scales_its_curve_about_the_origin() {
        let mut f = fixture();
        // about another origin: the middle point is f x mid + (1 - f) x origin, the length x f
        let out = f.doc.transact("scale", false, |m, cx| scale_elements(m, cx, &[f.wall], Vec2::new(5.0, 3.0), 0.5));
        let w = &f.doc.model().walls[&f.wall];
        let mid = (w.start + w.end) * 0.5;
        let expected_mid = Vec2::new(5.0, 0.0) * 0.5 + Vec2::new(5.0, 3.0) * 0.5;
        assert!((mid - expected_mid).length() < 1e-12, "{mid:?} {expected_mid:?} {:?}", out.failures);
        assert!(((w.end - w.start).length() - 5.0).abs() < 1e-12);
        // the stored curve is the same one whether or not the commit-time fit checks object
        assert_eq!(w.start, Vec2::new(2.5, 1.5));
    }

    fn nw_fixture(
        m: &mut BimModel,
        cx: &mut TxnContext<'_>,
    ) -> Result<(ElementId, impl Fn(Vec2, Vec2) -> ops::NewWall), Failure> {
        let l = ops::create_level(m, cx, "L0", 0.0)?;
        let cs = CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?;
        let wt = ops::create_wall_type(m, cx, "W", cs)?;
        Ok((l, move |a: Vec2, b: Vec2| ops::NewWall { wall_type: wt, level: l, start: a, end: b, height: 3.0, base_offset: 0.0, flip: false, structural: false }))
    }

    #[test]
    fn trim_and_extend_move_the_end_farther_from_the_click_to_the_boundary_line() {
        let mut doc = crate::new_document();
        let ids = doc
            .transact("fx", false, |m, cx| {
                let (_, nw) = nw_fixture(m, cx)?;
                let v = Vec2::new;
                let long = ops::create_wall(m, cx, nw(v(0.0, 0.0), v(10.0, 0.0)))?;
                let short = ops::create_wall(m, cx, nw(v(0.0, 4.0), v(4.0, 4.0)))?;
                let par = ops::create_wall(m, cx, nw(v(0.0, 6.0), v(4.0, 6.0)))?;
                let bound = ops::create_wall(m, cx, nw(v(6.0, -2.0), v(6.0, 8.0)))?;
                Ok((long, short, par, bound))
            })
            .value
            .expect("fixture");
        let (long, short, par, bound) = ids;
        // trim: the click near the start keeps the start part, the end moves back to x = 6
        let out = doc.transact("trim", false, |m, cx| trim_extend_wall(m, cx, long, Vec2::new(1.0, 0.0), bound));
        assert!(out.value.is_some(), "{:?}", out.failures);
        let w = &doc.model().walls[&long];
        assert_eq!((w.start, w.end), (Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0)));
        // extend: the click near the start, the end reaches the boundary
        let out = doc.transact("extend", false, |m, cx| trim_extend_wall(m, cx, short, Vec2::new(0.5, 4.0), bound));
        assert!(out.value.is_some(), "{:?}", out.failures);
        let w = &doc.model().walls[&short];
        assert_eq!((w.start, w.end), (Vec2::new(0.0, 4.0), Vec2::new(6.0, 4.0)));
        // parallel and itself are refused and change nothing
        let before = doc.model().clone();
        let out = doc.transact("p", false, |m, cx| trim_extend_wall(m, cx, par, Vec2::new(1.0, 6.0), long));
        assert!(out.value.is_none());
        let out = doc.transact("s", false, |m, cx| trim_extend_wall(m, cx, bound, Vec2::new(6.0, 0.0), bound));
        assert!(out.value.is_none());
        assert_eq!(doc.model(), &before);
    }

    #[test]
    fn trimming_the_start_moves_the_stations_so_the_inserts_keep_their_place() {
        let mut doc = crate::new_document();
        let (wall, bound, door) = doc
            .transact("fx", false, |m, cx| {
                let (l, nw) = nw_fixture(m, cx)?;
                let v = Vec2::new;
                let wall = ops::create_wall(m, cx, nw(v(0.0, 0.0), v(10.0, 0.0)))?;
                let bound = ops::create_wall(m, cx, nw(v(3.0, -2.0), v(3.0, 2.0)))?;
                let dt = ops::create_hosted_type(m, cx, "D", HostedKind::Door, 1.0, 2.0, 0.05, 1.0, 2.0, 0.0, 0.0)?;
                let door = ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: dt, host: wall, level: l, host_station: 8.0, sill_height: None })?;
                Ok((wall, bound, door))
            })
            .value
            .expect("fixture");
        // the click near the end: the start moves from x = 0 to x = 3
        let out = doc.transact("trim", false, |m, cx| trim_extend_wall(m, cx, wall, Vec2::new(9.0, 0.0), bound));
        assert!(out.committed(), "{:?}", out.failures);
        let m = doc.model();
        assert_eq!((m.walls[&wall].start, m.walls[&wall].end), (Vec2::new(3.0, 0.0), Vec2::new(10.0, 0.0)));
        // the door at x = 8 is at station 5 from the new start
        assert!((m.hosted_instances[&door].host_station - 5.0).abs() < 1e-12);
    }

    #[test]
    fn a_trim_that_leaves_a_door_outside_the_wall_posts_a_failure() {
        let mut doc = crate::new_document();
        let (wall, bound) = doc
            .transact("fx", false, |m, cx| {
                let (l, nw) = nw_fixture(m, cx)?;
                let v = Vec2::new;
                let wall = ops::create_wall(m, cx, nw(v(0.0, 0.0), v(10.0, 0.0)))?;
                let bound = ops::create_wall(m, cx, nw(v(6.0, -2.0), v(6.0, 2.0)))?;
                let dt = ops::create_hosted_type(m, cx, "D", HostedKind::Door, 1.0, 2.0, 0.05, 1.0, 2.0, 0.0, 0.0)?;
                ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: dt, host: wall, level: l, host_station: 8.0, sill_height: None })?;
                Ok((wall, bound))
            })
            .value
            .expect("fixture");
        let out = doc.transact("trim", false, |m, cx| trim_extend_wall(m, cx, wall, Vec2::new(1.0, 0.0), bound));
        assert!(!out.failures.is_empty(), "the door at station 8 no longer fits a wall of 6 m");
    }

    #[test]
    fn linear_and_radial_arrays_copy_the_members_and_refuse_bad_counts() {
        let mut f = fixture();
        let out = f.doc.transact("a", false, |m, cx| array_linear(m, cx, &[f.wall], 3, Vec2::new(0.0, 3.0), ArrayAnchor::Second));
        let pairs = out.value.expect("array");
        assert_eq!(pairs.iter().filter(|(o, _)| *o == f.wall).count(), 2);
        let m = f.doc.model();
        let mut by_id: Vec<(ElementId, f64)> = pairs.iter().filter(|(o, _)| *o == f.wall).map(|(_, c)| (*c, m.walls[c].start.y)).collect();
        by_id.sort_by_key(|(c, _)| *c);
        // anchor Second makes member 1 then 2 (OBSERVED): ids ascending give 3 then 6
        assert_eq!(by_id.iter().map(|(_, y)| *y).collect::<Vec<_>>(), vec![3.0, 6.0]);
        assert_eq!(m.hosted_instances.len(), 2 * 3, "the two hosted instances come with every copy (OBSERVED: copies are net of the door)");
        // anchor Last: the last member is 8 m away, 4 members -> step 8/3
        let out = f.doc.transact("b", false, |m, cx| array_linear(m, cx, &[f.wall], 4, Vec2::new(8.0, 0.0), ArrayAnchor::Last));
        let pairs = out.value.expect("array");
        let m = f.doc.model();
        let mut xs: Vec<(ElementId, f64)> = pairs.iter().filter(|(o, _)| *o == f.wall).map(|(_, c)| (*c, m.walls[c].start.x)).collect();
        xs.sort_by_key(|(c, _)| *c);
        let xs: Vec<f64> = xs.into_iter().map(|(_, x)| x).collect();
        // ids ascending: the last member (8), then member 1 (8/3) and member 2 (16/3)
        assert!((xs[0] - 8.0).abs() < 1e-12 && (xs[1] - 8.0 / 3.0).abs() < 1e-12 && (xs[2] - 16.0 / 3.0).abs() < 1e-12, "{xs:?}");
        // radial: 4 members, 90 degrees apart about the origin
        let out = f.doc.transact("c", false, |m, cx| array_radial(m, cx, &[f.wall], 4, Vec2::new(0.0, 0.0), std::f64::consts::FRAC_PI_2, ArrayAnchor::Second));
        let pairs = out.value.expect("radial");
        let m = f.doc.model();
        let mut c: Vec<(ElementId, Vec2)> = pairs.iter().filter(|(o, _)| *o == f.wall).map(|(_, c)| (*c, m.walls[c].end)).collect();
        c.sort_by_key(|(id, _)| *id);
        // anchor Second: ids ascending give 90, 180 and 270 degrees
        assert!((c[0].1 - Vec2::new(0.0, 10.0)).length() < 1e-9 && (c[1].1 - Vec2::new(-10.0, 0.0)).length() < 1e-9, "{c:?}");
        assert!((c[2].1 - Vec2::new(0.0, -10.0)).length() < 1e-9, "{c:?}");
        // an associated array pins the originals (OBSERVED)
        let before_pin = f.doc.model().is_pinned(f.wall);
        assert!(!before_pin);
        let out = f.doc.transact("assoc", false, |m, cx| array_linear_with(m, cx, &[f.wall], 2, Vec2::new(0.0, 50.0), ArrayAnchor::Second, true));
        assert!(out.value.is_some());
        assert!(f.doc.model().is_pinned(f.wall));
        // bad counts, vectors and empty lists are refused and change nothing
        let before = f.doc.model().clone();
        for (n, v) in [(1usize, Vec2::new(1.0, 0.0)), (201, Vec2::new(1.0, 0.0)), (3, Vec2::new(f64::NAN, 0.0))] {
            let out = f.doc.transact("x", false, |m, cx| array_linear(m, cx, &[f.wall], n, v, ArrayAnchor::Second));
            assert!(out.value.is_none());
        }
        for n in [2usize, 201] {
            let out = f.doc.transact("x", false, |m, cx| array_radial(m, cx, &[f.wall], n, Vec2::new(0.0, 0.0), 1.0, ArrayAnchor::Second));
            assert!(out.value.is_none(), "{n}");
        }
        let out = f.doc.transact("x", false, |m, cx| array_linear(m, cx, &[], 3, Vec2::new(1.0, 0.0), ArrayAnchor::Second));
        assert!(out.value.is_none());
        // a door on its own cannot be arrayed linearly
        let out = f.doc.transact("x", false, |m, cx| array_linear(m, cx, &[f.door], 3, Vec2::new(1.0, 0.0), ArrayAnchor::Second));
        assert!(out.value.is_none());
        // a zero translation COMMITS and the coincident walls post one overlap warning per pair
        let _ = before;
        let mut z = fixture();
        let out = z.doc.transact("zero", false, |m, cx| array_linear(m, cx, &[z.wall], 3, Vec2::new(0.0, 0.0), ArrayAnchor::Second));
        assert!(out.committed(), "{:?}", out.failures);
        assert_eq!(out.failures.iter().filter(|x| x.code == codes::WALL_OVERLAPS_WALL).count(), 3, "{:?}", out.failures);
        // a door on its own, radially: one error per member, the transaction does not commit
        let out = f.doc.transact("radial door", false, |m, cx| array_radial(m, cx, &[f.door], 3, Vec2::new(0.0, 0.0), 1.0, ArrayAnchor::Second));
        assert!(!out.committed());
        assert_eq!(out.failures.iter().filter(|x| x.code == codes::INSERT_NOT_CUTTING_HOST).count(), 2);
    }

    #[test]
    fn a_pinned_element_is_not_moved_or_mirrored_but_rotates_and_deletes_without_failure() {
        let mut f = fixture();
        assert!(f.doc.transact("pin", false, |m, cx| ops::set_pinned(m, cx, &[f.wall], true)).committed());
        assert!(f.doc.model().is_pinned(f.wall));
        let before = f.doc.model().clone();
        let t = crate::transform::PlanTransform::Translate(Vec2::new(0.0, 1.0));
        let out = f.doc.transact("move", false, |m, cx| crate::transform::apply_transform_to_elements(m, cx, &[f.wall], t));
        assert!(out.value.is_none() && out.failures.iter().any(|x| x.code == codes::ELEMENT_PINNED), "{:?}", out.failures);
        assert_eq!(f.doc.model(), &before);
        let mirror = crate::transform::PlanTransform::Mirror { a: Vec2::new(5.0, 0.0), b: Vec2::new(5.0, 1.0) };
        let out = f.doc.transact("mirror", false, |m, cx| crate::transform::apply_transform_to_elements(m, cx, &[f.wall], mirror));
        assert!(out.value.is_none());
        // a rotation of a pinned wall commits (OBSERVED)
        let rot = crate::transform::PlanTransform::Rotate { center: Vec2::new(0.0, 0.0), angle: 0.5 };
        let before_rot = f.doc.model().walls[&f.wall].clone();
        let out = f.doc.transact("rotate", false, |m, cx| crate::transform::apply_transform_to_elements(m, cx, &[f.wall], rot));
        assert!(out.committed(), "{:?}", out.failures);
        assert!(f.doc.model().is_pinned(f.wall));
        assert_eq!(f.doc.model().walls[&f.wall], before_rot, "the pinned wall stays where it was (OBSERVED)");
        let before = f.doc.model().clone();
        // a copy of a pinned element is allowed and is not pinned
        let out = f.doc.transact("copy", false, |m, cx| crate::transform::copy_elements(m, cx, &[f.wall], Some(t)));
        let copy = out.value.expect("copied")[0].1;
        assert!(!f.doc.model().is_pinned(copy));
        // unpinning allows the move; a type and a view can be pinned (OBSERVED)
        assert!(f.doc.transact("unpin", false, |m, cx| ops::set_pinned(m, cx, &[f.wall], false)).committed());
        assert!(f.doc.transact("move", false, |m, cx| crate::transform::apply_transform_to_elements(m, cx, &[f.wall], t)).value.is_some());
        // a wall type can be pinned too (OBSERVED); a missing element cannot
        let ty = f.doc.model().walls[&f.wall].type_id;
        assert!(f.doc.transact("pin", false, |m, cx| ops::set_pinned(m, cx, &[ty], true)).committed());
        assert!(f.doc.transact("pin", false, |m, cx| ops::set_pinned(m, cx, &[ElementId(9_999_999)], true)).value.is_none());
        let view = f
            .doc
            .transact("view", false, |m, cx| {
                let l = *m.levels.keys().next().unwrap();
                ops::create_view(m, cx, "V", crate::model::ViewKind::FloorPlan, Some(l), None)
            })
            .value
            .expect("a view");
        assert!(f.doc.transact("pin view", false, |m, cx| ops::set_pinned(m, cx, &[view], true)).committed());
        // the wall that carries a pinned door moves; the door alone does not
        assert!(f.doc.transact("pin", false, |m, cx| ops::set_pinned(m, cx, &[f.door], true)).committed());
        assert!(f.doc.transact("move wall", false, |m, cx| crate::transform::apply_transform_to_elements(m, cx, &[f.wall], t)).value.is_some());
        assert!(f.doc.transact("move door", false, |m, cx| crate::transform::apply_transform_to_elements(m, cx, &[f.door], t)).value.is_none());
        // deleting a pinned element commits with no failure, and it leaves the pinned set
        let out = f.doc.transact("del", false, |m, cx| ops::delete(m, cx, f.door));
        assert!(out.committed() && out.failures.is_empty(), "{:?}", out.failures);
        assert!(!f.doc.model().is_pinned(f.door) && !f.doc.model().hosted_instances.contains_key(&f.door));
    }

    #[test]
    fn an_arc_wall_scales_its_three_points() {
        let mut doc = crate::new_document();
        let w = doc
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let cs = CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?;
                let wt = ops::create_wall_type(m, cx, "W", cs)?;
                let v = Vec2::new;
                ops::create_arc_wall(m, cx, ops::NewWall { wall_type: wt, level: l, start: v(0.0, 0.0), end: v(4.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false }, v(2.0, 1.0))
            })
            .value
            .expect("arc wall");
        let out = doc.transact("scale", false, |m, cx| scale_elements(m, cx, &[w], Vec2::new(0.0, 0.0), 2.0));
        assert!(out.value.is_some(), "{:?}", out.failures);
        let wall = &doc.model().walls[&w];
        assert_eq!((wall.start, wall.end, wall.through), (Vec2::new(0.0, 0.0), Vec2::new(8.0, 0.0), Some(Vec2::new(4.0, 2.0))));
    }

    #[test]
    fn a_scale_refuses_bad_factors_and_non_walls_and_a_collapsed_wall_posts_curve_too_short() {
        let mut f = fixture();
        let before = f.doc.model().clone();
        for factor in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let out = f.doc.transact("s", false, |m, cx| scale_elements(m, cx, &[f.wall], Vec2::new(0.0, 0.0), factor));
            assert!(out.value.is_none(), "{factor}");
        }
        let out = f.doc.transact("s", false, |m, cx| scale_elements(m, cx, &[f.door], Vec2::new(0.0, 0.0), 2.0));
        assert!(out.value.is_none());
        let out = f.doc.transact("s", false, |m, cx| scale_elements(m, cx, &[ElementId(9_999_999)], Vec2::new(0.0, 0.0), 2.0));
        assert!(out.value.is_none());
        assert_eq!(f.doc.model(), &before);
        // 10 m scaled by 1e-6 is 0.01 mm, below the vertex tolerance: the failure is posted
        let out = f.doc.transact("s", false, |m, cx| scale_elements(m, cx, &[f.wall], Vec2::new(0.0, 0.0), 1e-6));
        assert!(out.failures.iter().any(|x| x.code == codes::CURVE_TOO_SHORT), "{:?}", out.failures);
    }
}
