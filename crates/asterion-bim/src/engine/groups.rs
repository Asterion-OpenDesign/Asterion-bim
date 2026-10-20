//! Model groups (OBSERVED).
//!
//! * `NewGroup(members)` makes a group TYPE (named "Group n" until renamed; the group takes its type's name) and one
//!   instance; the members that are already in a group are left out and an empty result is refused ("none of the input
//!   elements can be grouped").
//! * The members of a group are reported pinned. Editing one of them outside group edit mode does not commit ("Changes
//!   to groups are allowed only in group edit mode", an error posted at commit, `group.edit_outside_edit_mode`);
//!   moving, rotating or copying the group instance moves its members as a unit.
//! * An instance sits at the centre of the box of its members (plan); `PlaceGroup(point)` puts a new instance's centre
//!   at the point, with copies of the members of an existing instance. The box includes the door family's own geometry
//!   (ENGINE CHOICE from one door: [`DOOR_BACK_M`], [`DOOR_FRONT_M`], [`DOOR_FRAME_M`]).
//! * `UngroupMembers` frees the members (unpinned); the type keeps existing. Deleting an instance deletes its members.

use crate::annot::{FrozenGroup, Group, GroupType};
use crate::model::BimModel;
use crate::spec::codes;
use crate::transform::{self, PlanTransform};
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use asterion_geom::Vec2;

/// The template door's box beyond its opening, measured once: the frame adds 0.075 m on each side along the wall, the
/// leaf's swing reaches 0.9204 m in front of the wall axis and the frame 0.125 m behind it.
pub const DOOR_FRAME_M: f64 = 0.075;
pub const DOOR_FRONT_M: f64 = 0.9203740868164381;
pub const DOOR_BACK_M: f64 = 0.125;
/// The template window's box (round 28b, one window): no frame margin along the wall or in height, the wall's half
/// thickness behind the axis and 0.3712 m beyond the front face of the 0.2 m wall. ENGINE CHOICE: how the front depth
/// depends on the wall thickness and on the window size is not measured.
pub const WINDOW_FRONT_M: f64 = 0.471_152_665_991_514;

fn bad(code: &str, msg: &str) -> Failure {
    Failure::error(code, msg.to_owned())
}

/// Whether an element can be part of a group in this engine.
fn groupable(m: &BimModel, id: ElementId) -> bool {
    m.walls.contains_key(&id)
        || m.hosted_instances.contains_key(&id)
        || m.wall_openings.contains_key(&id)
        || m.floors.contains_key(&id)
        || m.columns.contains_key(&id)
        || m.annot.curves.contains_key(&id)
}

/// The plan box `(min, max)` and the z range of an element, for a group's origin and box.
pub fn element_box(m: &BimModel, id: ElementId) -> Option<([f64; 3], [f64; 3])> {
    if let Some(w) = m.walls.get(&id) {
        let (fp, z0, z1) = crate::regen::wall_view_body(m, w).ok()?;
        if fp.is_empty() {
            return None;
        }
        let lo = [fp.iter().map(|p| p.x).fold(f64::INFINITY, f64::min), fp.iter().map(|p| p.y).fold(f64::INFINITY, f64::min), z0];
        let hi = [fp.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max), fp.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max), z1];
        return Some((lo, hi));
    }
    if let Some(i) = m.hosted_instances.get(&id) {
        let w = m.walls.get(&i.host)?;
        let st = crate::regen::hosted_instance_state(m, i).ok()?;
        let d = (w.end - w.start).normalized()?;
        let n = Vec2::new(-d.y, d.x);
        // the facing side: the instance's facing orientation (the wall's left normal unless flipped)
        let face = if i.facing_flipped { -1.0 } else { 1.0 };
        let c = w.start + d * i.host_station;
        let z0 = st.sill_z;
        if m.hosted_types.get(&i.type_id).map_or(false, |t| t.kind == crate::model::HostedKind::Window) {
            let half = st.width / 2.0;
            let (a, b) = (c - d * half + n * (face * -0.1), c + d * half + n * (face * WINDOW_FRONT_M));
            return Some(([a.x.min(b.x), a.y.min(b.y), z0], [a.x.max(b.x), a.y.max(b.y), st.head_z]));
        }
        let half = st.width / 2.0 + DOOR_FRAME_M;
        let (a, b) = (c - d * half + n * (face * -DOOR_BACK_M), c + d * half + n * (face * DOOR_FRONT_M));
        return Some((
            [a.x.min(b.x), a.y.min(b.y), z0],
            [a.x.max(b.x), a.y.max(b.y), st.head_z + DOOR_FRAME_M],
        ));
    }
    if let Some(c) = m.annot.curves.get(&id) {
        return Some(c.geom.bbox());
    }
    None
}

/// The union box of some elements.
pub fn union_box(m: &BimModel, ids: &[ElementId]) -> Option<([f64; 3], [f64; 3])> {
    let mut acc: Option<([f64; 3], [f64; 3])> = None;
    for id in ids {
        if let Some((lo, hi)) = element_box(m, *id) {
            acc = Some(match acc {
                None => (lo, hi),
                Some((a, b)) => (
                    [a[0].min(lo[0]), a[1].min(lo[1]), a[2].min(lo[2])],
                    [b[0].max(hi[0]), b[1].max(hi[1]), b[2].max(hi[2])],
                ),
            });
        }
    }
    acc
}

fn origin_of(m: &BimModel, members: &[ElementId]) -> [f64; 2] {
    match union_box(m, members) {
        Some((lo, hi)) => [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0],
        None => [0.0, 0.0],
    }
}

/// `NewGroup`: a type and its first instance. Returns `(instance, type)`.
pub fn create_group(m: &mut BimModel, cx: &mut TxnContext<'_>, members: &[ElementId], name: Option<&str>) -> Result<(ElementId, ElementId), Failure> {
    let ok: Vec<ElementId> = members.iter().copied().filter(|id| groupable(m, *id) && m.annot.group_of(*id).is_none()).collect();
    if ok.is_empty() {
        return Err(bad(
            codes::GROUP_CANNOT_GROUP,
            "None of the input elements can be grouped. The elements may already be grouped or may be element types that are not allowed for grouping.",
        ));
    }
    let type_id = cx.new_id();
    let id = cx.new_id();
    let name = match name {
        Some(n) => n.to_owned(),
        // OBSERVED: the smallest "Group n" not in use (a renamed first type frees its number)
        None => (1..).map(|n| format!("Group {n}")).find(|c| !m.annot.group_types.values().any(|t| &t.name == c)).expect("unbounded"),
    };
    let origin = origin_of(m, &ok);
    let bbox_rel = union_box(m, &ok).map(|(lo, hi)| ([lo[0] - origin[0], lo[1] - origin[1], lo[2]], [hi[0] - origin[0], hi[1] - origin[1], hi[2]]));
    m.annot.group_types.insert(type_id, GroupType { id: type_id, name, prototype: Some(id), frozen: None });
    m.annot.groups.insert(id, Group { id, type_id, origin, members: ok, bbox_rel });
    Ok((id, type_id))
}

/// `PlaceGroup(point, type)`: a new instance whose centre is at `point`, with copies of the prototype's members.
pub fn place_group(m: &mut BimModel, cx: &mut TxnContext<'_>, type_id: ElementId, point: [f64; 2]) -> Result<ElementId, Failure> {
    let ty = m.annot.group_types.get(&type_id).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such group type.").on(&[type_id]))?.clone();
    let (p_origin, p_rel, p_members) = match ty.prototype.and_then(|p| m.annot.groups.get(&p)) {
        Some(g) => (g.origin, g.bbox_rel, g.members.clone()),
        None => match &ty.frozen {
            Some(f) if f.members.iter().all(|i| groupable(m, *i)) => (f.origin, f.bbox_rel, f.members.clone()),
            _ => return Err(bad(codes::GROUP_CANNOT_GROUP, "The group type has no instance to copy (not modelled).")),
        },
    };
    let d = Vec2::new(point[0] - p_origin[0], point[1] - p_origin[1]);
    let pairs = transform::with_unguarded(|| transform::copy_elements_with(m, cx, &p_members, Some(PlanTransform::Translate(d)), false))?;
    let members: Vec<ElementId> = p_members.iter().filter_map(|o| pairs.iter().find(|(a, _)| a == o).map(|(_, n)| *n)).collect();
    // OBSERVED: the sketch plane of a model curve placed with a group moves with the placement
    for n in &members {
        if let Some(c) = m.annot.curves.get_mut(n) {
            c.plane_origin = [c.plane_origin[0] + d.x, c.plane_origin[1] + d.y, c.plane_origin[2]];
        }
    }
    let id = cx.new_id();
    m.annot.groups.insert(id, Group { id, type_id, origin: point, members, bbox_rel: p_rel });
    Ok(id)
}

/// `Group.UngroupMembers`: the members are free again; the group is gone and its type stays. `Group.UngroupMembers`. Every
/// member leaves the group and stays in the document as an ordinary element, the emptied group instance goes, and the group
/// TYPE is not deleted.
pub fn ungroup(m: &mut BimModel, group: ElementId) -> Result<Vec<ElementId>, Failure> {
    let g = m.annot.groups.remove(&group).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such group.").on(&[group]))?;
    reprototype(m, g.type_id, group);
    // the last instance ungrouped: the type remembers what it was made of
    if let Some(t) = m.annot.group_types.get_mut(&g.type_id) {
        if t.prototype.is_none() {
            t.frozen = Some(FrozenGroup { origin: g.origin, bbox_rel: g.bbox_rel, members: g.members.clone() });
        }
    }
    Ok(g.members)
}

fn reprototype(m: &mut BimModel, type_id: ElementId, gone: ElementId) {
    let next = m.annot.groups.values().find(|g| g.type_id == type_id).map(|g| g.id);
    if let Some(t) = m.annot.group_types.get_mut(&type_id) {
        if t.prototype == Some(gone) {
            t.prototype = next;
        }
    }
}

/// Rename a group type (every instance takes the name).
pub fn set_type_name(m: &mut BimModel, type_id: ElementId, name: &str) -> Result<(), Failure> {
    let t = m.annot.group_types.get_mut(&type_id).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such group type.").on(&[type_id]))?;
    t.name = name.to_owned();
    Ok(())
}

/// Deleting an instance deletes its members; returns the ids to delete after the group is dropped.
pub fn take_for_delete(m: &mut BimModel, group: ElementId) -> Vec<ElementId> {
    match m.annot.groups.remove(&group) {
        Some(g) => {
            reprototype(m, g.type_id, group);
            g.members
        }
        None => Vec::new(),
    }
}

/// Post the failure of an edit of group members outside group edit mode (the transaction does not commit).
pub fn guard(m: &BimModel, cx: &mut TxnContext<'_>, ids: &[ElementId]) {
    let mut hit: Vec<ElementId> = Vec::new();
    for id in ids {
        if let Some(g) = m.annot.group_of(*id) {
            hit.push(g);
            hit.push(*id);
        }
    }
    if !hit.is_empty() {
        cx.report(
            Failure::error(
                codes::GROUP_EDIT_OUTSIDE_EDIT_MODE,
                "Changes to groups are allowed only in group edit mode. Ungroup the instance to make the change.",
            )
            .on(&hit),
        );
    }
}

/// The ids among `ids` that are members of a group.
pub fn members_among(m: &BimModel, ids: &[ElementId]) -> Vec<ElementId> {
    ids.iter().copied().filter(|i| m.annot.group_of(*i).is_some()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ops, CompoundStructure, Layer, LayerFunction};

    #[test]
    fn a_group_is_placed_by_the_centre_of_its_members_and_edits_of_members_do_not_commit() {
        let mut d = crate::new_document();
        let (w, ty) = d
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let wt = ops::create_wall_type(m, cx, "W", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
                let w = ops::create_wall(
                    m,
                    cx,
                    ops::NewWall { wall_type: wt, level: l, start: Vec2::new(0.0, 0.0), end: Vec2::new(5.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false },
                )?;
                let (_, ty) = create_group(m, cx, &[w], Some("G"))?;
                Ok((w, ty))
            })
            .value
            .unwrap();
        let g1 = d.model().annot.groups.values().next().unwrap().clone();
        assert_eq!(g1.origin, [2.5, 0.0]);
        let g2 = d.transact("p", false, |m, cx| place_group(m, cx, ty, [10.0, 3.0])).value.unwrap();
        assert_eq!(d.model().annot.group_types[&ty].name, "G");
        let w2 = d.model().annot.groups[&g2].members[0];
        let wall2 = d.model().walls[&w2].clone();
        assert!((wall2.start.x - 7.5).abs() < 1e-9 && (wall2.start.y - 3.0).abs() < 1e-9);
        // a member edited outside group edit mode: the transaction does not commit
        let out = d.transact("e", false, |m, cx| ops::set_unconnected_height(m, cx, w2, 4.0));
        assert!(!out.committed());
        assert!(d.model().is_pinned(w));
        // already in a group: refused
        assert!(d.transact("g", false, |m, cx| create_group(m, cx, &[w], None)).value.is_none());
        // ungroup frees the members
        d.transact("u", false, |m, _| ungroup(m, g2));
        assert!(!d.model().is_pinned(w2));
        assert!(d.transact("e2", false, |m, cx| ops::set_unconnected_height(m, cx, w2, 4.0)).committed());
    }

    #[test]
    fn a_default_group_name_is_the_smallest_unused_and_a_type_with_no_instance_can_be_placed() {
        // OBSERVED
        let mut d = crate::new_document();
        let (a, b) = d
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let wt = ops::create_wall_type(m, cx, "W", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
                let mk = |m: &mut crate::BimModel, cx: &mut TxnContext<'_>, y: f64| {
                    ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l, start: Vec2::new(0.0, y), end: Vec2::new(4.0, y), height: 3.0, base_offset: 0.0, flip: false, structural: false })
                };
                Ok((mk(m, cx, 0.0)?, mk(m, cx, 10.0)?))
            })
            .value
            .unwrap();
        let (_, t1) = d.transact("g1", false, |m, cx| create_group(m, cx, &[a], None)).value.unwrap();
        assert_eq!(d.model().annot.group_types[&t1].name, "Group 1");
        d.transact("r", false, |m, _| set_type_name(m, t1, "Renamed"));
        let (g2, t2) = d.transact("g2", false, |m, cx| create_group(m, cx, &[b], None)).value.unwrap();
        assert_eq!(d.model().annot.group_types[&t2].name, "Group 1", "the number the rename freed is used again");
        // the only instance of the first type ungrouped: the type still places
        let inst = d.model().annot.groups.values().find(|g| g.type_id == t1).unwrap().id;
        d.transact("u", false, |m, _| ungroup(m, inst));
        let placed = d.transact("p", false, |m, cx| place_group(m, cx, t1, [20.0, 20.0])).value.unwrap();
        assert_eq!(d.model().annot.groups[&placed].members.len(), 1);
        let _ = g2;
    }
}
