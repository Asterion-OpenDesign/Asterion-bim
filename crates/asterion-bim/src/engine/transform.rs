//! Move, Rotate, Mirror and Copy of elements: one transform mechanism, and copy as a separate one.
//!
//! Written specification of the specification rules and Spec P-01 (id allocation):
//!
//! * **One mechanism, no new ids.** Move, Rotate and Mirror build one transform and run it through the same path; the move
//!   type is 1 translation, 2 rotation, 3 mirror (0 a general transform). Every element keeps its `ElementId`: nothing is
//!   allocated. A mirror toggles the element's mirrored-geometry flag.
//! * **What is carried along.** The given ids are expanded by the per-class helpers; the overrides of the wall, floor and
//!   hosted-instance helpers are NOT specified. What the engine carries is what its model makes relative to a host: a door,
//!   window or wall opening is stored by its station along its host wall, so it follows its wall without being listed. Join
//!   geometry and wall joins are not handled by the generic operations and the engine leaves its join relations as they
//!   are.
//! * **Mirror of a wall (Spec MIR-01).** About a vertical plane the location curve is reflected WITHOUT swapping its ends
//!   (the start stays the start), the wall is flipped (its exterior side) unless it is a host panel, the sides of its joins
//!   are flipped and the materials of its two side faces are swapped. The engine: reflected curve, `flipped` toggled; it
//!   has no host panels, no stored join sides (its wall joins are derived from the geometry at every regeneration) and no
//!   per-side materials, so those parts have nothing to act on. Not specified (Spec MIR-01): non-vertical planes (the
//!   engine's mirror planes are always vertical) and a hosted instance's own mirror.
//! * **Refusals and failures.** A transform that does not decompose into simple moves posts
//!   `UnableToTransformElementError`; a transform whose Z axis is not +Z posts `NoRotateMirrorZError`. The engine's
//!   transforms are plan transforms (the Z axis is kept), so the second cannot arise; a transform that is not a finite
//!   rigid motion is refused at the call (`transform.invalid`, ASTERION's input rule). The per-class refusals (locked,
//!   pinned, hosted instances without a host) are not specified: what the engine refuses is listed on
//!   [`apply_transform_to_elements`] as ENGINE CHOICE.
//! * **Copy is a different mechanism.** The ids are deep-copied: one sequential block of new ids is reserved and handed out
//!   in the order of the copy array (Spec P-01); every id stored inside a copied element is rewritten through the old ->
//!   new map, so a copied door whose wall is copied too lands on the copy of the wall. The copies are then transformed by
//!   the same mechanism as a move.
//!
//! UNKNOWN, recorded as questions (no value is guessed for them; the engine choice is named):
//! * Which ids the per-class helpers add. OBSERVED: the hosted instances of a copied wall are carried by a copy that lists
//!   only the wall; the wall openings and the other classes are not known to be (only listed ids and hosted instances are
//!   copied).
//! * The name of a copied grid and the number of a copied room (the engine gives them the next automatic name / number, the
//!   same rule as a new one).

use crate::model::BimModel;
use crate::spec::codes;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use asterion_geom::Vec2;
use std::collections::BTreeMap;

/// A plan transform: a rigid motion of the XY plane that keeps the Z axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlanTransform {
    /// Move type 1.
    Translate(Vec2),
    /// Move type 2: about the vertical axis through `center`, `angle` in radians (counter-clockwise).
    Rotate { center: Vec2, angle: f64 },
    /// Move type 3: about the vertical plane through the line `a`-`b`.
    Mirror { a: Vec2, b: Vec2 },
}

impl PlanTransform {
    /// 1 translation, 2 rotation, 3 mirror.
    pub fn move_type(&self) -> u8 {
        match self {
            PlanTransform::Translate(_) => 1,
            PlanTransform::Rotate { .. } => 2,
            PlanTransform::Mirror { .. } => 3,
        }
    }

    pub fn is_mirror(&self) -> bool {
        matches!(self, PlanTransform::Mirror { .. })
    }

    /// The transform is a finite rigid motion (a mirror needs a line of non-zero length).
    pub fn is_well_formed(&self) -> bool {
        let f = |v: Vec2| v.x.is_finite() && v.y.is_finite();
        match *self {
            PlanTransform::Translate(d) => f(d),
            PlanTransform::Rotate { center, angle } => f(center) && angle.is_finite(),
            PlanTransform::Mirror { a, b } => f(a) && f(b) && (b - a).length() > 1e-9,
        }
    }

    /// Image of a point.
    pub fn apply(&self, p: Vec2) -> Vec2 {
        match *self {
            PlanTransform::Translate(d) => p + d,
            PlanTransform::Rotate { center, angle } => center + (p - center).rotated(angle),
            PlanTransform::Mirror { a, b } => {
                let u = (b - a).normalized().unwrap_or(Vec2::new(1.0, 0.0));
                let r = p - a;
                let along = u * r.dot(u);
                a + along * 2.0 - r
            }
        }
    }

    /// Image of a direction (the linear part).
    pub fn apply_vector(&self, v: Vec2) -> Vec2 {
        self.apply(v) - self.apply(Vec2::new(0.0, 0.0))
    }
}

fn invalid() -> Failure {
    Failure::error(codes::TRANSFORM_INVALID, "The transform is not a finite rigid motion of the plan.")
}

fn not_movable(id: ElementId, what: &str) -> Failure {
    Failure::error(codes::TRANSFORM_NOT_SUPPORTED, format!("{what} cannot be moved, rotated, mirrored or copied this way.")).on(&[id])
}

/// The element kinds a plan transform moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Wall,
    Slab,
    Grid,
    Room,
    RoomSeparation,
    Hosted,
    Opening,
    Column,
    /// A stairs with its runs, landings and railings (OBSERVED: moved and copied as one).
    Stairs,
    /// Reference plane, model line or arc, text note, dimension, tag, group instance (OBSERVED).
    RefPlane,
    Curve,
    Note,
    Dim,
    Tag,
    Group,
    /// A toposolid or a subdivision (OBSERVED: moved and copied in plan, heights kept).
    Toposolid,
}

thread_local! {
    /// Set while a group copies or places its own members (they are not edited from outside).
    static UNGUARDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f` with the group-member guard of the copy off.
pub fn with_unguarded<R>(f: impl FnOnce() -> R) -> R {
    let before = UNGUARDED.with(|u| u.replace(true));
    let out = f();
    UNGUARDED.with(|u| u.set(before));
    out
}

fn map3(t: &PlanTransform, p: [f64; 3]) -> [f64; 3] {
    let q = t.apply(Vec2::new(p[0], p[1]));
    [q.x, q.y, p[2]]
}

fn vec3(t: &PlanTransform, v: [f64; 3]) -> [f64; 3] {
    let q = t.apply_vector(Vec2::new(v[0], v[1]));
    [q.x, q.y, v[2]]
}

pub(crate) fn kind_of(m: &BimModel, id: ElementId) -> Result<Kind, Failure> {
    if m.walls.contains_key(&id) {
        Ok(Kind::Wall)
    } else if m.floors.contains_key(&id) {
        Ok(Kind::Slab)
    } else if m.grids.contains_key(&id) {
        Ok(Kind::Grid)
    } else if m.rooms.contains_key(&id) {
        Ok(Kind::Room)
    } else if m.room_separations.contains_key(&id) {
        Ok(Kind::RoomSeparation)
    } else if m.hosted_instances.contains_key(&id) {
        Ok(Kind::Hosted)
    } else if m.wall_openings.contains_key(&id) {
        Ok(Kind::Opening)
    } else if m.columns.contains_key(&id) {
        Ok(Kind::Column)
    } else if m.levels.contains_key(&id) {
        Err(not_movable(id, "A level"))
    } else if m.views.contains_key(&id) {
        Err(not_movable(id, "A view"))
    } else if m.wall_types.contains_key(&id) || m.floor_types.contains_key(&id) || m.hosted_types.contains_key(&id) || m.column_types.contains_key(&id) {
        Err(not_movable(id, "A type"))
    } else if m.annot.ref_planes.contains_key(&id) {
        Ok(Kind::RefPlane)
    } else if m.annot.curves.contains_key(&id) {
        Ok(Kind::Curve)
    } else if m.annot.notes.contains_key(&id) {
        Ok(Kind::Note)
    } else if m.annot.dims.contains_key(&id) {
        Ok(Kind::Dim)
    } else if m.annot.tags.contains_key(&id) {
        Ok(Kind::Tag)
    } else if m.annot.groups.contains_key(&id) {
        Ok(Kind::Group)
    } else if m.docs.toposolids.contains_key(&id) {
        Ok(Kind::Toposolid)
    } else if m.annot.sweeps.contains_key(&id) {
        Err(not_movable(id, "A wall sweep (it follows its wall)"))
    } else if m.stairs.stairs.contains_key(&id) {
        Ok(Kind::Stairs)
    } else if m.stairs.runs.contains_key(&id) || m.stairs.landings.contains_key(&id) {
        Err(not_movable(id, "A stairs component on its own"))
    } else {
        Err(Failure::error(codes::NOT_FOUND, format!("Element {} does not exist.", id.0)).on(&[id]))
    }
}

/// Transform one element in place (no new id). Hosted instances and openings whose host is in
/// `set` follow it and are not touched.
fn transform_one(m: &mut BimModel, id: ElementId, t: &PlanTransform, set: &[ElementId]) -> Result<(), Failure> {
    match kind_of(m, id)? {
        Kind::Wall => {
            let w = m.walls.get_mut(&id).expect("kind");
            w.start = t.apply(w.start);
            w.end = t.apply(w.end);
            w.through = w.through.map(|p| t.apply(p));
            // Spec MIR-01: the curve is reflected with no end swap and the wall is flipped.
            if t.is_mirror() {
                w.flipped = !w.flipped;
            }
        }
        Kind::Slab => {
            let s = m.floors.get_mut(&id).expect("kind");
            for p in s.boundary.iter_mut().chain(s.loops.iter_mut().flatten()) {
                *p = t.apply(*p);
            }
            for a in s.boundary_arcs.iter_mut().chain(s.loop_arcs.iter_mut().flatten()).flatten() {
                *a = t.apply(*a);
            }
            if let Some(a) = s.slope_arrow.as_mut() {
                a.tail = t.apply(a.tail);
                a.head = t.apply(a.head);
            }
            // The edge-defined plane rises to one side of the edge's travel; a mirror swaps the sides, so the side flag
            // follows to keep the reflected plane.
            if t.is_mirror() {
                if let Some(e) = s.slope_edge.as_mut() {
                    e.reversed = !e.reversed;
                }
            }
        }
        Kind::Grid => {
            let g = m.grids.get_mut(&id).expect("kind");
            g.start = t.apply(g.start);
            g.end = t.apply(g.end);
            g.through = g.through.map(|p| t.apply(p));
        }
        Kind::Room => {
            let r = m.rooms.get_mut(&id).expect("kind");
            r.point = t.apply(r.point);
        }
        Kind::RoomSeparation => {
            let r = m.room_separations.get_mut(&id).expect("kind");
            r.start = t.apply(r.start);
            r.end = t.apply(r.end);
        }
        Kind::Hosted => {
            let host = m.hosted_instances[&id].host;
            if set.contains(&host) {
                // Stored relative to its host: it follows the wall. A MIRROR reverses the side the host's unflipped normal
                // points to (Spec MIR-01: the curve is reflected with no end swap), so the facing flag toggles to keep the
                // reflected facing; the element's mirrored flag toggles under a mirror, and `mirrored` = facing XOR hand
                // toggles with it. OBSERVED: after the mirror the door reports facing flipped, mirrored, facing (1, 0, 0).
                // ASSUMED beyond that one sample: the reference product's own mirror of an instance is not specified.
                if t.is_mirror() {
                    let h = m.hosted_instances.get_mut(&id).expect("kind");
                    h.facing_flipped = !h.facing_flipped;
                }
                return Ok(());
            }
            // Alone: a translation slides it along a straight host by the component of the move along the wall (the
            // engine's station model); a rotation or a mirror of a hosted instance alone is not specified: ENGINE CHOICE,
            // refused.
            let PlanTransform::Translate(d) = *t else {
                return Err(not_movable(id, "A hosted instance on its own"));
            };
            let w = m.walls.get(&host).ok_or_else(|| not_movable(id, "A hosted instance without its host"))?;
            if w.through.is_some() {
                return Err(not_movable(id, "An instance hosted by an arc wall"));
            }
            let dir = (w.end - w.start).normalized().ok_or_else(|| not_movable(id, "An instance on a degenerate wall"))?;
            m.hosted_instances.get_mut(&id).expect("kind").host_station += d.dot(dir);
        }
        Kind::Opening => {
            let host = m.wall_openings[&id].host;
            if set.contains(&host) {
                return Ok(());
            }
            return Err(not_movable(id, "A wall opening on its own"));
        }
        Kind::RefPlane => {
            let p = m.annot.ref_planes.get_mut(&id).expect("kind");
            p.bubble = map3(t, p.bubble);
            p.free = map3(t, p.free);
            p.cut = vec3(t, p.cut);
            p.normal = vec3(t, p.normal);
        }
        Kind::Toposolid => {
            let topo = m.docs.toposolids.get_mut(&id).expect("kind");
            crate::toposolid::map_plan(topo, |p| t.apply(p));
        }
        Kind::Curve => {
            let c = m.annot.curves.get_mut(&id).expect("kind");
            // OBSERVED: the sketch plane of a moved, rotated or mirrored curve stays where it was
            c.geom = c.geom.map_points(&|p| map3(t, p));
        }
        Kind::Note => {
            let n = m.annot.notes.get_mut(&id).expect("kind");
            n.coord = map3(t, n.coord);
            // OBSERVED: a mirror turns a right-aligned note into a left-aligned one
            if t.is_mirror() {
                n.align = match n.align {
                    crate::annot::HAlign::Left => crate::annot::HAlign::Right,
                    crate::annot::HAlign::Right => crate::annot::HAlign::Left,
                    c => c,
                };
            }
        }
        // a dimension is derived from its references: nothing to move (ENGINE CHOICE, not measured)
        Kind::Dim => {}
        Kind::Tag => {
            let g = m.annot.tags.get_mut(&id).expect("kind");
            g.head = map3(t, g.head);
        }
        Kind::Group => {
            if t.is_mirror() {
                return Err(not_movable(id, "A group instance mirrored"));
            }
            let g = m.annot.groups[&id].clone();
            for member in &g.members {
                transform_one(m, *member, t, &g.members)?;
            }
            let o = t.apply(Vec2::new(g.origin[0], g.origin[1]));
            let gm = m.annot.groups.get_mut(&id).expect("kind");
            gm.origin = [o.x, o.y];
            // a rotated instance reports the box of its members as they are now (OBSERVED)
            if !matches!(t, PlanTransform::Translate(_)) {
                gm.bbox_rel = None;
            }
        }
        Kind::Stairs => {
            // OBSERVED: a translation moves the stairs with its components; a rotation or a mirror of a stairs is not
            // measured (ENGINE CHOICE, refused)
            let PlanTransform::Translate(d) = *t else {
                return Err(not_movable(id, "A stairs rotated or mirrored"));
            };
            crate::stairs_scene::translate_stairs(m, id, d);
        }
        Kind::Column => {
            // OBSERVED: the location moves; a rotation adds its angle; a mirror about a line of direction phi maps the
            // rotation theta to 2 phi - theta
            let c = m.columns.get_mut(&id).expect("kind");
            c.location = t.apply(c.location);
            match *t {
                PlanTransform::Translate(_) => {}
                PlanTransform::Rotate { angle, .. } => c.rotation = crate::columns::normalize_angle(c.rotation + angle),
                PlanTransform::Mirror { a, b } => {
                    let d = b - a;
                    c.rotation = crate::columns::normalize_angle(2.0 * d.y.atan2(d.x) - c.rotation);
                    // A mirrored instance has exactly one of its two flips.
                    c.facing_flipped = !c.facing_flipped;
                }
            }
        }
    }
    Ok(())
}

/// Commit-time checks of the elements a transform touched (the same checks every edit runs).
fn check_after(m: &BimModel, cx: &mut TxnContext<'_>, ids: &[ElementId]) {
    let walls: Vec<ElementId> = ids.iter().copied().filter(|i| m.walls.contains_key(i)).collect();
    let slabs: Vec<ElementId> = ids.iter().copied().filter(|i| m.floors.contains_key(i)).collect();
    crate::ops::check_walls(m, cx, &walls);
    crate::ops::check_slabs(m, cx, &slabs);
    crate::ops::check_joins(m, cx, ids);
    let mut inserts: Vec<ElementId> = ids.to_vec();
    for w in &walls {
        inserts.extend(m.dependents(*w));
    }
    crate::ops::check_inserts(m, cx, &inserts);
}

fn dedup(ids: &[ElementId]) -> Vec<ElementId> {
    let mut out: Vec<ElementId> = Vec::with_capacity(ids.len());
    for id in ids {
        if !out.contains(id) {
            out.push(*id);
        }
    }
    out
}

/// Move, Rotate or Mirror elements in place (one mechanism, no new ids; hosted elements stored relative to a moved host
/// follow it).
///
/// An empty id list does nothing (an empty array returns 0). Returns the ids handled.
///
/// ENGINE CHOICE (the per-class helpers are not specified not specified): levels, views, types and stairs elements are
/// refused; a hosted instance moved without its host only slides along a straight host (a rotation or mirror of it alone is
/// refused); a wall opening moves only with its host. Code `transform.not_supported`, raised at the call.
pub fn apply_transform_to_elements(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    ids: &[ElementId],
    t: PlanTransform,
) -> Result<Vec<ElementId>, Failure> {
    if !t.is_well_formed() {
        return Err(invalid());
    }
    let ids = dedup(ids);
    for id in &ids {
        kind_of(m, *id)?;
    }
    // OBSERVED: `MoveElements` and `MirrorElements` on a pinned element raise "Unable to move element(s) due to at least
    // one of them being pinned." and change nothing; a ROTATION of a pinned wall COMMITS and the wall stays where it was
    // (nothing moves, no failure). Only the listed elements count: the wall that carries a pinned door moves. the UI also
    // posts CannotMoveLocked; with the copy flags the test is skipped.
    let rotating = matches!(t, PlanTransform::Rotate { .. });
    if !rotating {
        if let Some(p) = ids.iter().find(|i| m.pinned.contains(i)) {
            return Err(Failure::error(
                codes::ELEMENT_PINNED,
                "Unable to move element(s) due to at least one of them being pinned.",
            )
            .on(&[*p]));
        }
    }
    // OBSERVED: a member of a group cannot be moved outside group edit mode
    if !crate::groups::members_among(m, &ids).is_empty() {
        crate::groups::guard(m, cx, &ids);
        return Ok(Vec::new());
    }
    // OBSERVED: a model curve on a sketch plane that is not horizontal cannot be mirrored ("Can't mirror element into this
    // position": the transaction is refused) and a rotation about the vertical axis leaves it where it was (the step
    // commits)
    let slanted = |m: &BimModel, i: &ElementId| !plane_kept(m, i, &t);
    if t.is_mirror() {
        let bad: Vec<ElementId> = ids.iter().filter(|i| slanted(m, i)).copied().collect();
        if !bad.is_empty() {
            cx.report(Failure::error(codes::TRANSFORM_NOT_SUPPORTED, "Can't mirror element into this position.").on(&bad));
            return Ok(ids);
        }
    }
    // the ends the moved walls had, for the walls joined to them
    let before: Vec<(ElementId, Vec2, Vec2, [bool; 2])> =
        ids.iter().filter_map(|i| m.walls.get(i).map(|w| (*i, w.start, w.end, w.join_allowed))).collect();
    // OBSERVED: moving a wall that a LOCKED dimension refers to drags the other walls of the dimension along, by the part
    // of the move along the dimension line (the locked value is kept)
    let mut dragged: Vec<(ElementId, Vec2)> = Vec::new();
    if let PlanTransform::Translate(dv) = t {
        let mut moved: Vec<ElementId> = ids.clone();
        let mut again = true;
        while again {
            again = false;
            for dim in m.annot.dims.values().filter(|x| x.locked) {
                let walls: Vec<ElementId> = dim.refs.iter().filter_map(|r| if let crate::annot::DimRef::Wall { wall, .. } = r { Some(*wall) } else { None }).collect();
                let Some(&mover) = walls.iter().find(|w| moved.contains(w)) else { continue };
                let line = Vec2::new(dim.line.1[0] - dim.line.0[0], dim.line.1[1] - dim.line.0[1]);
                let Some(u) = line.normalized() else { continue };
                let along = u * dv.dot(u);
                let _ = mover;
                for w in walls {
                    if !moved.contains(&w) && !m.pinned.contains(&w) {
                        moved.push(w);
                        dragged.push((w, along));
                        again = true;
                    }
                }
            }
        }
    }
    // OBSERVED: a room bounded by a wall that is translated moves its location with it
    let follow_rooms: Vec<ElementId> = if matches!(t, PlanTransform::Translate(_)) {
        m.rooms
            .values()
            .filter(|r| {
                crate::regen::room_state(m, r)
                    .map_or(false, |s| s.finish.iter().chain(s.center.iter()).any(|seg| seg.host.map_or(false, |h| ids.contains(&h))))
            })
            .map(|r| r.id)
            .collect()
    } else {
        Vec::new()
    };
    for id in &ids {
        if rotating && (m.pinned.contains(id) || slanted(m, id)) {
            continue;
        }
        transform_one(m, *id, &t, &ids)?;
    }
    for (w, along) in &dragged {
        transform_one(m, *w, &PlanTransform::Translate(*along), &[*w])?;
    }
    if let PlanTransform::Translate(d) = t {
        follow_joined_ends(m, &ids, &before, d);
        for r in follow_rooms {
            if !ids.contains(&r) {
                if let Some(room) = m.rooms.get_mut(&r) {
                    room.point = room.point + d;
                }
            }
        }
    }
    follow_tags(m, &ids, &t);
    check_after(m, cx, &ids);
    Ok(ids)
}

/// Whether the sketch plane of a model curve maps onto itself under a rotation or a mirror about a vertical axis or plane
/// (always for a horizontal plane). OBSERVED: a curve on a vertical plane that the mirror leaves in place is mirrored; one
/// whose plane would move is refused; a rotation that would take the plane away leaves the curve as it was.
fn plane_kept(m: &BimModel, id: &ElementId, t: &PlanTransform) -> bool {
    let Some(c) = m.annot.curves.get(id) else { return true };
    let n = c.plane_normal;
    if n[2].abs() > 0.999_999 {
        return true;
    }
    let (p, q) = (Vec2::new(c.plane_origin[0], c.plane_origin[1]), Vec2::new(n[0], n[1]));
    let (p2, n2) = (t.apply(p), t.apply_vector(q));
    let same_dir = (n2.x * q.x + n2.y * q.y).abs() > q.length() * n2.length() - 1e-6;
    let on_plane = ((p2.x - p.x) * q.x + (p2.y - p.y) * q.y).abs() < 1e-6;
    same_dir && on_plane
}

/// OBSERVED: a wall moved by a translation keeps its joined neighbours attached: the end of a wall that was joined to an
/// end of the moved wall follows it (the neighbour is stretched). ENGINE CHOICE for the directions not measured (a move
/// across the neighbour tilts it) and for rotations and mirrors (no follow).
fn follow_joined_ends(m: &mut BimModel, moved: &[ElementId], before: &[(ElementId, Vec2, Vec2, [bool; 2])], d: Vec2) {
    let tol = asterion_geom::Tolerances::default().vertex;
    for (_, s0, e0, allowed) in before {
        for (k, p) in [(0usize, *s0), (1usize, *e0)] {
            if !allowed[k] {
                continue;
            }
            // only a wall with an end at p follows it; the index finds those without visiting every wall (and
            // without copying every chunk of the wall map just to look)
            let others: Vec<ElementId> = crate::joins::walls_near(m, p).into_iter().map(|w| w.id).filter(|i| !moved.contains(i)).collect();
            for n in &others {
                let Some(w) = m.walls.get_mut(n) else { continue };
                if (w.start - p).length() < tol && w.join_allowed[0] {
                    w.start = p + d;
                } else if (w.end - p).length() < tol && w.join_allowed[1] {
                    w.end = p + d;
                }
            }
        }
    }
}

/// OBSERVED: a tag follows its host when the host moves (the host's own hosted instances move with it).
fn follow_tags(m: &mut BimModel, moved: &[ElementId], t: &PlanTransform) {
    let mut hosts: Vec<ElementId> = moved.to_vec();
    hosts.extend(m.hosted_instances.values().filter(|i| moved.contains(&i.host)).map(|i| i.id));
    let tags: Vec<ElementId> = m.annot.tags.values().filter(|g| hosts.contains(&g.host) && !moved.contains(&g.id)).map(|g| g.id).collect();
    for g in tags {
        let tag = m.annot.tags.get_mut(&g).expect("listed");
        tag.head = map3(t, tag.head);
    }
}

/// Copy elements ("copy is a different mechanism") and apply `t` to the copies (`None` = copy in place). Returns the
/// `(original, copy)` pairs in the order of the copy array.
///
/// Spec P-01: the new ids are one sequential block handed out in the order of the copy array; every id stored inside a
/// copied element is rewritten through the old -> new map (a door's host, an opening's host) and ids outside the copied set
/// are kept (a type, a level, a host that is not copied -). Join relations are not copied (joins are made by regeneration
/// and auto-join, not by the copy). A copied grid takes the next automatic name and a copied room the next number; the
/// texts (comments, mark) are copied with the element. A same-document copy is a clip pasted back; the parents of a copied
/// element (host, level) are re-bound by the PARENT's paste helper (virtual, which in the base class matches only unique
/// elements). the engine keeps the original level.
pub fn copy_elements(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    ids: &[ElementId],
    t: Option<PlanTransform>,
) -> Result<Vec<(ElementId, ElementId)>, Failure> {
    copy_elements_with(m, cx, ids, t, true)
}

/// [`copy_elements`] with the choice of carrying the hosted instances of a copied wall. A COPY carries them (OBSERVED), and
/// so does an ARRAY (OBSERVED: the arrayed copy is net of the door's cut even though the group lists the wall alone).
pub fn copy_elements_with(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    ids: &[ElementId],
    t: Option<PlanTransform>,
    carry_hosted: bool,
) -> Result<Vec<(ElementId, ElementId)>, Failure> {
    if let Some(t) = t {
        if !t.is_well_formed() {
            return Err(invalid());
        }
    }
    let mut ids = dedup(ids);
    // OBSERVED: the copies are made in ascending id of their sources, whatever the order listed
    ids.sort();
    for id in &ids {
        kind_of(m, *id)?;
    }
    // OBSERVED: a tag alone cannot be copied ("some tags were not copied because they could not find a host"): a warning
    // and nothing is made
    let tags_listed = ids.iter().filter(|i| m.annot.tags.contains_key(i)).count();
    if tags_listed > 0 {
        // (the tag paste helper): a copied tag whose every reference finds no new host gets the warning CannotPasteTag
        // (literal 1, no resolution) and is dropped. The failing element of the reference product is the tag COPY it made
        // and then deleted (OBSERVED, "Elements were deleted"); the engine makes no copy, so it names none.
        cx.report(Failure::warning(codes::TAG_NOT_PASTED, "Some tags were not copied because they could not find a host."));
        ids.retain(|i| !m.annot.tags.contains_key(i));
    }
    // OBSERVED: members of a group cannot be copied outside group edit mode
    if !UNGUARDED.with(|u| u.get()) && !crate::groups::members_among(m, &ids).is_empty() {
        crate::groups::guard(m, cx, &ids);
        return Ok(Vec::new());
    }
    // OBSERVED: copying a wall ALONE copies the hosted door with it (two hosted instances after a copy of one wall that
    // listed only the wall; the copied wall is net of the door), and listing the door as well does not copy it twice. So
    // the hosted instances of every copied wall join the copy array, after the listed ids, in ascending id (their order in
    // the block is not observed). the wall openings of a copied wall are NOT carried (not observed: ENGINE CHOICE stays
    // open for them and for the other classes' helpers).
    let carried: Vec<ElementId> = if carry_hosted {
        m.hosted_instances
            .values()
            .filter(|i| ids.contains(&i.host) && m.walls.contains_key(&i.host) && !ids.contains(&i.id))
            .map(|i| i.id)
            .collect()
    } else {
        Vec::new()
    };
    ids.extend(carried);
    let mut kinds = Vec::with_capacity(ids.len());
    for id in &ids {
        kinds.push(kind_of(m, *id)?);
    }
    // one sequential block, in the order of the copy array (Spec P-01)
    let mut map: BTreeMap<ElementId, ElementId> = BTreeMap::new();
    for (id, kind) in ids.iter().zip(&kinds) {
        // a stairs is copied whole, with its own ids for the components
        let n = if *kind == Kind::Stairs { crate::stairs_scene::copy_stairs(m, cx, *id, Vec2::new(0.0, 0.0))? } else { cx.new_id() };
        map.insert(*id, n);
    }
    let remap = |e: ElementId| map.get(&e).copied().unwrap_or(e);
    for (id, kind) in ids.iter().zip(&kinds) {
        let n = map[id];
        match kind {
            Kind::Wall => {
                let mut w = m.walls[id].clone();
                w.id = n;
                m.walls.insert(n, w);
                // OBSERVED: the sweeps and reveals of a copied wall are copied with it
                let sweeps: Vec<crate::annot::WallSweep> = m.annot.sweeps.values().filter(|s| s.wall == *id).cloned().collect();
                for mut sw in sweeps {
                    sw.id = cx.new_id();
                    sw.wall = n;
                    m.annot.sweeps.insert(sw.id, sw);
                }
                // a curtain wall's copy carries its grid (OBSERVED)
                if let Some(g) = m.curtain_grids.get(id).cloned() {
                    m.curtain_grids.insert(n, g);
                }
            }
            Kind::Slab => {
                let mut s = m.floors[id].clone();
                s.id = n;
                m.floors.insert(n, s);
            }
            Kind::Grid => {
                let mut g = m.grids[id].clone();
                g.id = n;
                g.name = crate::model::Grid::next_name(m.last_grid_name.as_deref());
                while m.grids.values().any(|o| o.name == g.name) {
                    g.name = crate::model::Grid::next_name(Some(&g.name));
                }
                m.last_grid_name = Some(g.name.clone());
                m.grids.insert(n, g);
            }
            Kind::Room => {
                let mut r = m.rooms[id].clone();
                r.id = n;
                r.number = crate::ops::next_room_number(m);
                m.rooms.insert(n, r);
            }
            Kind::RoomSeparation => {
                let mut r = m.room_separations[id].clone();
                r.id = n;
                m.room_separations.insert(n, r);
            }
            Kind::Hosted => {
                let mut h = m.hosted_instances[id].clone();
                h.id = n;
                h.host = remap(h.host);
                m.hosted_instances.insert(n, h);
            }
            Kind::Opening => {
                let mut o = m.wall_openings[id].clone();
                o.id = n;
                o.host = remap(o.host);
                m.wall_openings.insert(n, o);
            }
            Kind::Stairs => {}
            Kind::RefPlane => {
                let mut p = m.annot.ref_planes[id].clone();
                p.id = n;
                if p.name != crate::datums::DEFAULT_PLANE_NAME {
                    let existing: Vec<String> = m.annot.ref_planes.values().map(|q| q.name.clone()).collect();
                    let refs: Vec<&str> = existing.iter().map(String::as_str).collect();
                    p.name = crate::datums::next_name(&refs, &p.name);
                }
                m.annot.ref_planes.insert(n, p);
            }
            Kind::Curve => {
                let mut c = m.annot.curves[id].clone();
                c.id = n;
                m.annot.curves.insert(n, c);
            }
            Kind::Toposolid => {
                let mut c = m.docs.toposolids[id].clone();
                c.id = n;
                m.docs.toposolids.insert(n, c);
                // OBSERVED: the subdivisions of a toposolid are copied with it, and the copies stay subdivisions of the
                // ORIGINAL (deleting it takes them)
                let subs: Vec<crate::toposolid::Toposolid> = m.docs.toposolids.values().filter(|s| s.host == Some(*id)).cloned().collect();
                for mut s in subs {
                    s.id = cx.new_id();
                    if let Some(tt) = t {
                        crate::toposolid::map_plan(&mut s, |p| tt.apply(p));
                    }
                    m.docs.toposolids.insert(s.id, s);
                }
            }
            Kind::Note => {
                let mut c = m.annot.notes[id].clone();
                c.id = n;
                m.annot.notes.insert(n, c);
            }
            // OBSERVED: a copied dimension is an exact duplicate (the vector is ignored)
            Kind::Dim => {
                let mut c = m.annot.dims[id].clone();
                c.id = n;
                m.annot.dims.insert(n, c);
            }
            Kind::Tag => {}
            Kind::Group => {
                let g = m.annot.groups[id].clone();
                let pairs = with_unguarded(|| copy_elements_with(m, cx, &g.members, None, false))?;
                let members: Vec<ElementId> = g.members.iter().filter_map(|o| pairs.iter().find(|(a, _)| a == o).map(|(_, x)| *x)).collect();
                m.annot.groups.insert(n, crate::annot::Group { id: n, type_id: g.type_id, origin: g.origin, members, bbox_rel: g.bbox_rel });
            }
            Kind::Column => {
                // ASSUMED (not measured): a copy keeps no attachment
                let mut c = m.columns[id].clone();
                c.id = n;
                c.top_attachment = None;
                c.base_attachment = None;
                m.columns.insert(n, c);
            }
        }
        if let Some(text) = m.texts.get(id).cloned() {
            m.texts.insert(n, text);
        }
    }
    let copies: Vec<ElementId> = ids.iter().map(|id| map[id]).collect();
    if let Some(t) = t {
        for c in &copies {
            transform_one(m, *c, &t, &copies)?;
        }
    }
    check_after(m, cx, &copies);
    Ok(ids.iter().map(|id| (*id, map[id])).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops;
    use crate::{CompoundStructure, Layer, LayerFunction};
    use std::f64::consts::FRAC_PI_2;

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

    fn near(a: Vec2, b: Vec2) {
        assert!((a - b).length() < 1e-9, "{a:?} != {b:?}");
    }

    struct Fx {
        doc: crate::BimDocument,
        wall: ElementId,
        door: ElementId,
        floor: ElementId,
        grid: ElementId,
        room: ElementId,
    }

    fn fixture() -> Fx {
        let mut doc = crate::new_document();
        let (wall, door, floor, grid, room) = tx(&mut doc, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let cs = CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?;
            let wt = ops::create_wall_type(m, cx, "W", cs)?;
            let wall = ops::create_wall(
                m,
                cx,
                ops::NewWall { wall_type: wt, level: l, start: v(0.0, 0.0), end: v(6.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false },
            )?;
            let dt = ops::create_hosted_type(m, cx, "D", crate::model::HostedKind::Door, 0.9, 2.1, 0.05, 0.9, 2.1, 0.0, 0.0)?;
            let door = ops::place_hosted_instance(
                m,
                cx,
                ops::NewHostedInstance { type_id: dt, host: wall, level: l, host_station: 2.0, sill_height: None },
            )?;
            let ft = ops::create_floor_type(m, cx, "F", 0.2)?;
            let floor = ops::create_floor(m, cx, ft, l, vec![v(0.0, 0.0), v(6.0, 0.0), v(6.0, 4.0), v(0.0, 4.0)], 0.0, false)?;
            let grid = ops::create_grid(m, cx, None, v(0.0, -2.0), v(0.0, 6.0))?;
            let room = ops::create_room(m, cx, l, v(3.0, 2.0))?;
            Ok((wall, door, floor, grid, room))
        })
        .expect("fixture");
        Fx { doc, wall, door, floor, grid, room }
    }

    /// Move, rotate and mirror keep every id and allocate none; the door follows its wall (it is stored by its station); an
    /// empty list does nothing.
    #[test]
    fn move_rotate_mirror_keep_ids_and_allocate_nothing() {
        let mut fx = fixture();
        let next = fx.doc.ids().peek();
        let all = [fx.wall, fx.door, fx.floor, fx.grid, fx.room];
        tx(&mut fx.doc, |m, cx| apply_transform_to_elements(m, cx, &all, PlanTransform::Translate(v(10.0, 5.0)))).unwrap();
        assert_eq!(fx.doc.ids().peek(), next, "no id allocated by a move");
        let m = fx.doc.model();
        near(m.walls[&fx.wall].start, v(10.0, 5.0));
        near(m.floors[&fx.floor].boundary[2], v(16.0, 9.0));
        near(m.grids[&fx.grid].end, v(10.0, 11.0));
        near(m.rooms[&fx.room].point, v(13.0, 7.0));
        assert_eq!(m.hosted_instances[&fx.door].host_station, 2.0, "the door follows its wall");

        tx(&mut fx.doc, |m, cx| apply_transform_to_elements(m, cx, &[fx.wall, fx.door], PlanTransform::Rotate { center: v(10.0, 5.0), angle: FRAC_PI_2 })).unwrap();
        near(fx.doc.model().walls[&fx.wall].end, v(10.0, 11.0));
        tx(&mut fx.doc, |m, cx| apply_transform_to_elements(m, cx, &[], PlanTransform::Translate(v(1.0, 0.0)))).unwrap();
        assert_eq!(fx.doc.ids().peek(), next);
    }

    /// Mirror (move type 3): the wall's curve is reflected and the wall flipped (Spec MIR-01), the slope-edge side follows;
    /// the operation is undoable.
    #[test]
    fn mirror_reflects_and_is_undoable() {
        let mut fx = fixture();
        let o0 = crate::regen::wall_state(fx.doc.model(), &fx.doc.model().walls[&fx.wall]).unwrap().orientation;
        tx(&mut fx.doc, |m, cx| apply_transform_to_elements(m, cx, &[fx.wall, fx.door], PlanTransform::Mirror { a: v(0.0, 1.0), b: v(1.0, 1.0) })).unwrap();
        let w = &fx.doc.model().walls[&fx.wall];
        near(w.start, v(0.0, 2.0));
        near(w.end, v(6.0, 2.0));
        let o1 = crate::regen::wall_state(fx.doc.model(), w).unwrap().orientation;
        near(o1, v(o0.x, -o0.y));
        // the door: facing reflected, flag toggled, mirrored
        let d = &fx.doc.model().hosted_instances[&fx.door];
        assert!(d.facing_flipped && d.mirrored());
        fx.doc.undo().unwrap();
        near(fx.doc.model().walls[&fx.wall].start, v(0.0, 0.0));
    }

    /// Spec MIR-01: no end swap - mirrored about the vertical plane x = -5, the wall (0,0) -> (6,0) becomes (-10,0) ->
    /// (-16,0): the image of the start is still the start; the wall is flipped.
    #[test]
    fn a_mirrored_wall_keeps_its_start_and_is_flipped() {
        let mut fx = fixture();
        let f0 = fx.doc.model().walls[&fx.wall].flipped;
        tx(&mut fx.doc, |m, cx| apply_transform_to_elements(m, cx, &[fx.wall], PlanTransform::Mirror { a: v(-5.0, 0.0), b: v(-5.0, 1.0) })).unwrap();
        let w = &fx.doc.model().walls[&fx.wall];
        near(w.start, v(-10.0, 0.0));
        near(w.end, v(-16.0, 0.0));
        assert_eq!(w.flipped, !f0);
    }

    /// The engine refusals (ENGINE CHOICE) and the invalid transform.
    #[test]
    fn refusals() {
        let mut fx = fixture();
        let e = tx(&mut fx.doc, |m, cx| apply_transform_to_elements(m, cx, &[fx.door], PlanTransform::Rotate { center: v(0.0, 0.0), angle: 1.0 })).unwrap_err();
        assert_eq!(e.code, codes::TRANSFORM_NOT_SUPPORTED);
        let e = tx(&mut fx.doc, |m, cx| apply_transform_to_elements(m, cx, &[fx.wall], PlanTransform::Mirror { a: v(1.0, 1.0), b: v(1.0, 1.0) })).unwrap_err();
        assert_eq!(e.code, codes::TRANSFORM_INVALID);
        let lvl = *fx.doc.model().levels.keys().next().unwrap();
        let e = tx(&mut fx.doc, |m, cx| apply_transform_to_elements(m, cx, &[lvl], PlanTransform::Translate(v(1.0, 0.0)))).unwrap_err();
        assert_eq!(e.code, codes::TRANSFORM_NOT_SUPPORTED);
        // a door alone slides along its straight wall
        tx(&mut fx.doc, |m, cx| apply_transform_to_elements(m, cx, &[fx.door], PlanTransform::Translate(v(1.5, 3.0)))).unwrap();
        assert_eq!(fx.doc.model().hosted_instances[&fx.door].host_station, 3.5);
    }

    /// Spec P-01: a copy reserves one sequential block of ids in the order of the copy array; the copied door is re-hosted
    /// on the copied wall; the copies are moved, the originals stay; a copied grid takes the next automatic name.
    #[test]
    fn copy_reserves_a_block_and_remaps_hosts() {
        let mut fx = fixture();
        let next = fx.doc.ids().peek().0;
        let order = [fx.door, fx.wall, fx.grid];
        let pairs = tx(&mut fx.doc, |m, cx| copy_elements(m, cx, &order, Some(PlanTransform::Translate(v(0.0, 10.0))))).unwrap();
        assert_eq!(pairs.iter().map(|p| p.1 .0).collect::<Vec<_>>(), vec![next, next + 1, next + 2]);
        // OBSERVED: the copies are made in ascending id of their sources, whatever the order listed
        assert!(pairs.windows(2).all(|w| w[0].0 < w[1].0), "{pairs:?}");
        let copy_of = |src: ElementId| pairs.iter().find(|p| p.0 == src).expect("copied").1;
        let m = fx.doc.model();
        let (door2, wall2, grid2) = (copy_of(fx.door), copy_of(fx.wall), copy_of(fx.grid));
        assert_eq!(m.hosted_instances[&door2].host, wall2, "the copy of the door is hosted by the copy of the wall");
        near(m.walls[&wall2].start, v(0.0, 10.0));
        near(m.walls[&fx.wall].start, v(0.0, 0.0));
        assert_eq!(m.grids[&grid2].name, "2");
        // a door copied alone stays on its host and slides along it
        let pairs = tx(&mut fx.doc, |m, cx| copy_elements(m, cx, &[fx.door], Some(PlanTransform::Translate(v(1.0, 0.0))))).unwrap();
        let d = &fx.doc.model().hosted_instances[&pairs[0].1];
        assert_eq!((d.host, d.host_station), (fx.wall, 3.0));
    }
}

#[cfg(test)]
mod copy_expansion_tests {
    //! OBSERVED: the hosted door of a copied wall is copied with it.
    use super::*;
    use crate::model::HostedKind;
    use crate::ops;

    #[test]
    fn copying_a_wall_alone_carries_its_door_and_listing_both_copies_it_once() {
        let mut doc = crate::new_document();
        let (wall, door, op) = doc
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L", 0.0)?;
                let cs = crate::CompoundStructure::new(vec![crate::Layer { width: 0.2, function: crate::LayerFunction::Structure }], 0, 0)?;
                let wt = ops::create_wall_type(m, cx, "W", cs)?;
                let v = Vec2::new;
                let wall = ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l, start: v(0.0, 0.0), end: v(6.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false })?;
                let dt = ops::create_hosted_type(m, cx, "D", HostedKind::Door, 1.0, 2.0, 0.05, 1.0, 2.0, 0.0, 0.0)?;
                let door = ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: dt, host: wall, level: l, host_station: 2.0, sill_height: None })?;
                let op = ops::add_rectangular_opening(m, cx, wall, 4.0, 1.0, 5.0, 2.0)?;
                Ok((wall, door, op))
            })
            .value
            .expect("fixture");
        let out = doc.transact("copy", false, |m, cx| copy_elements(m, cx, &[wall], Some(PlanTransform::Translate(Vec2::new(0.0, 10.0)))));
        let pairs = out.value.expect("copied");
        let m = doc.model();
        assert_eq!(pairs.len(), 2, "the wall and its door: {pairs:?}");
        let wall_copy = pairs.iter().find(|(o, _)| *o == wall).unwrap().1;
        let door_copy = pairs.iter().find(|(o, _)| *o == door).unwrap().1;
        assert_eq!(m.hosted_instances[&door_copy].host, wall_copy);
        assert_eq!(m.hosted_instances.len(), 2);
        // the rectangular opening is not carried (not observed: ENGINE CHOICE)
        assert!(!m.wall_openings.values().any(|o| o.host == wall_copy && o.id != op));
        // listing the door as well copies it once
        let out = doc.transact("copy2", false, |m, cx| copy_elements(m, cx, &[wall, door], Some(PlanTransform::Translate(Vec2::new(0.0, 20.0)))));
        assert_eq!(out.value.expect("copied").len(), 2);
        assert_eq!(doc.model().hosted_instances.len(), 3, "one more door, not two");
    }
}

#[cfg(test)]
mod default_type_tests {
    //! The document default type of a category.
    use crate::model::HostedKind;
    use crate::ops;

    #[test]
    fn a_load_sets_the_default_and_create_similar_replaces_it() {
        let mut doc = crate::new_document();
        let out = doc.transact("t", false, |m, cx| {
            assert_eq!(m.default_hosted_type(HostedKind::Door), None, "nothing stored, nothing invented");
            let d1 = ops::create_hosted_type(m, cx, "D1", HostedKind::Door, 0.9, 2.1, 0.05, 0.9, 2.1, 0.0, 0.0)?;
            assert_eq!(m.default_hosted_type(HostedKind::Door), Some(d1));
            let w1 = ops::create_hosted_type(m, cx, "W1", HostedKind::Window, 1.2, 1.2, 0.05, 1.2, 1.2, 0.9, 0.0)?;
            let d2 = ops::create_hosted_type(m, cx, "D2", HostedKind::Door, 0.8, 2.1, 0.05, 0.8, 2.1, 0.0, 0.0)?;
            // The next load replaces the default of its own category only
            assert_eq!((m.default_hosted_type(HostedKind::Door), m.default_hosted_type(HostedKind::Window)), (Some(d2), Some(w1)));
            let l = ops::create_level(m, cx, "L", 0.0)?;
            let cs = crate::CompoundStructure::new(vec![crate::Layer { width: 0.2, function: crate::LayerFunction::Structure }], 0, 0)?;
            let wt = ops::create_wall_type(m, cx, "W", cs)?;
            let v = asterion_geom::Vec2::new;
            let wall = ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l, start: v(0.0, 0.0), end: v(6.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false })?;
            let i = ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: d1, host: wall, level: l, host_station: 2.0, sill_height: None })?;
            // Create Similar makes the instance's type the default
            assert_eq!(ops::prepare_create_similar(m, cx, i)?, d1);
            assert_eq!(m.default_hosted_type(HostedKind::Door), Some(d1));
            // a deleted default is not replaced by a guess
            ops::delete(m, cx, i)?;
            ops::delete(m, cx, d1)?;
            assert_eq!(m.default_hosted_type(HostedKind::Door), None);
            Ok(())
        });
        assert!(out.committed(), "{:?}", out.failures);
    }
}

#[cfg(test)]
mod arc_grid_tests {
    //! Arc grids (Spec V-03, G-02).
    use crate::ops;
    use crate::model::{BimModel, ViewKind};
    use asterion_geom::Vec2;

    #[test]
    fn an_arc_grid_is_held_drawn_moved_and_saved() {
        let mut doc = crate::new_document();
        let out = doc.transact("t", false, |m: &mut BimModel, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let v = ops::create_view(m, cx, "P", ViewKind::FloorPlan, Some(l), None)?;
            let g = ops::create_arc_grid(m, cx, None, Vec2::new(5.0, 0.0), Vec2::new(-5.0, 0.0), Vec2::new(0.0, 5.0))?;
            Ok((v, g))
        });
        let (v, g) = out.value.expect("created");
        let m = doc.model();
        let grid = m.grids.get(&g).unwrap();
        assert!(grid.is_curved());
        // the drawn curve is the arc's samples; every point is 5 m from the centre
        let st = crate::views::view_state(m, m.views.get(&v).unwrap()).unwrap();
        let vg = st.grids.iter().find(|x| x.id == g).unwrap();
        assert!(vg.visible && vg.polyline.len() > 10);
        assert!(vg.polyline.iter().all(|p| (p.length() - 5.0).abs() < 1e-9));
        // stored with the ends swapped, and an arc's view curve runs forward
        assert_eq!((m.grids[&g].start, m.grids[&g].end), (Vec2::new(-5.0, 0.0), Vec2::new(5.0, 0.0)));
        assert!((vg.polyline[0] - Vec2::new(-5.0, 0.0)).length() < 1e-9);
        // the plan drawing: one segment per sample interval and one bubble (Spec G-02 default end)
        let prims = crate::plan_graphics::plan_primitives(m, m.views.get(&v).unwrap(), &st);
        let segs = prims.iter().filter(|p| p.element == g && p.role == crate::plan_graphics::Role::GridLine).count();
        assert_eq!(segs, vg.polyline.len() - 1);
        // collinear points are refused
        let e = doc.transact("bad", false, |m, cx| ops::create_arc_grid(m, cx, None, Vec2::new(0.0, 0.0), Vec2::new(2.0, 0.0), Vec2::new(1.0, 0.0)));
        assert_eq!(e.failures[0].code, crate::spec::codes::GRID_INVALID_ARC);
        // a move keeps the arc
        doc.transact("mv", false, |m, cx| crate::transform::apply_transform_to_elements(m, cx, &[g], crate::transform::PlanTransform::Translate(Vec2::new(1.0, 1.0))));
        assert_eq!(doc.model().grids[&g].through, Some(Vec2::new(1.0, 6.0)));
        doc.transact("mv2", false, |m, cx| ops::set_grid_curve(m, cx, g, Vec2::new(-5.0, 0.0), Vec2::new(5.0, 0.0)));
        assert_eq!(doc.model().grids[&g].through, Some(Vec2::new(0.0, 5.0)));
    }
}

#[cfg(test)]
mod hidden_in_view_tests {
    //! "Hide in View" (the view's own set of element ids).
    use crate::model::{BimModel, ViewKind};
    use crate::ops;
    use asterion_geom::Vec2;

    #[test]
    fn a_hidden_element_is_not_drawn_in_that_view_only() {
        let mut doc = crate::new_document();
        let out = doc.transact("t", false, |m: &mut BimModel, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let cs = crate::CompoundStructure::new(vec![crate::Layer { width: 0.2, function: crate::LayerFunction::Structure }], 0, 0)?;
            let wt = ops::create_wall_type(m, cx, "W", cs)?;
            let w = ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l, start: Vec2::new(0.0, 0.0), end: Vec2::new(6.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false })?;
            let g = ops::create_grid(m, cx, None, Vec2::new(0.0, -2.0), Vec2::new(0.0, 6.0))?;
            let p1 = ops::create_view(m, cx, "P1", ViewKind::FloorPlan, Some(l), None)?;
            let p2 = ops::create_view(m, cx, "P2", ViewKind::FloorPlan, Some(l), None)?;
            let v3 = ops::create_view(m, cx, "3D", ViewKind::ThreeD, None, None)?;
            Ok((wt, w, g, p1, p2, v3))
        });
        let (wt, w, g, p1, p2, v3) = out.value.expect("fixture");
        assert!(doc.transact("hide", false, |m, cx| ops::set_hidden_in_view(m, cx, p1, &[w, g], true)).committed());
        assert!(doc.transact("hide3d", false, |m, cx| ops::set_hidden_in_view(m, cx, v3, &[w], true)).committed());
        let m = doc.model();
        let st1 = crate::views::view_state(m, &m.views[&p1]).unwrap();
        assert!(!st1.elements.iter().find(|e| e.id == w).unwrap().appearance.is_drawn());
        assert!(st1.grids.iter().find(|x| x.id == g).unwrap().visible, "a hidden datum keeps its view curves");
        assert!(crate::plan_graphics::plan_primitives(m, &m.views[&p1], &st1).iter().all(|p| p.element != w && p.element != g));
        let st2 = crate::views::view_state(m, &m.views[&p2]).unwrap();
        assert!(st2.elements.iter().find(|e| e.id == w).unwrap().appearance.is_drawn(), "the other plan still shows it");
        assert!(crate::mesh3d::view_mesh(m, &m.views[&v3]).unwrap().triangle_count() == 0);
        // undo shows it again; a type cannot be hidden; deleting the wall forgets it
        doc.undo().unwrap();
        assert!(!doc.model().is_hidden_in_view(v3, w));
        let e = doc.transact("bad", false, |m, cx| ops::set_hidden_in_view(m, cx, p1, &[wt], true));
        assert_eq!(e.failures[0].code, crate::spec::codes::WRONG_KIND);
        doc.transact("del", false, |m, cx| ops::delete(m, cx, w));
        assert!(!doc.model().is_hidden_in_view(p1, w));
        assert!(doc.model().is_hidden_in_view(p1, g));
    }
}
