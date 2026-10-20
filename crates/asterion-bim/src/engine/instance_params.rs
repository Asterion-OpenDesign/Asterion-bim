//! Instance parameters the element Properties palette edits that live outside the element kinds' own structs: the
//! free-text Comments and Mark of an element, a wall's base level and its Structural flag.
//!
//! * Comments and Mark, and a room's Occupancy, Department, Base / Ceiling / Wall / Floor Finish and Occupant: the
//!   reference's palette shows them in Identity Data (OBSERVED in selr-q2-selected, drr-d5-door-selected and the room
//!   tool's palette rmr-r2). They are user text with no behaviour; the engine keeps them per element in
//!   [`crate::model::BimModel::texts`] (ASSUMED representation: where the reference product stores them is not specified).
//! * Name of a level: a level's name is user text (ASSUMED: no uniqueness rule is read); its plan views keep their own
//!   names.
//! * Base level of a wall: the wall's base level is the element's associated level, and the wall's bottom is the base
//!   level's elevation plus the base offset. The engine re-bases the wall and its hosted inserts (an insert stands on its
//!   host's base level; what the reference product does with them is not specified: ASSUMED).
//! * Structural flag of a wall: a stored flag of the wall (`Wall::structural`); what turning it on changes elsewhere
//!   (Structural Usage, analytical model) is not specified.

use crate::model::BimModel;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use serde::{Deserialize, Serialize};

/// The free-text parameters of one element.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ElementText {
    pub comments: String,
    pub mark: String,
    /// The other text parameters, by key ([`TEXT_KEYS`]).
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub others: std::collections::BTreeMap<String, String>,
}

impl ElementText {
    /// The value of a text key ("" when not set).
    pub fn get(&self, key: &str) -> String {
        match key {
            "comments" => self.comments.clone(),
            "mark" => self.mark.clone(),
            _ => self.others.get(key).cloned().unwrap_or_default(),
        }
    }
}

/// The text keys an element takes.
pub const TEXT_KEYS: [&str; 9] =
    ["comments", "mark", "occupancy", "department", "base_finish", "ceiling_finish", "wall_finish", "floor_finish", "occupant"];

/// The text parameters of `id` (empty when none was set).
pub fn element_text(m: &BimModel, id: ElementId) -> ElementText {
    m.texts.get(&id).cloned().unwrap_or_default()
}

/// Write the Comments or the Mark of an element that exists. Refused: an unknown key, an unknown element.
pub fn set_element_text(m: &mut BimModel, _cx: &mut TxnContext<'_>, id: ElementId, key: &str, value: &str) -> Result<(), Failure> {
    if !m.contains(id) {
        return Err(Failure::error(crate::spec::codes::WRONG_KIND, format!("{} is not an element of the model", id.0)));
    }
    let t = m.texts.entry(id).or_default();
    match key {
        "comments" => t.comments = value.to_owned(),
        "mark" => t.mark = value.to_owned(),
        k if TEXT_KEYS.contains(&k) => {
            if value.is_empty() {
                t.others.remove(k);
            } else {
                t.others.insert(k.to_owned(), value.to_owned());
            }
        }
        _ => return Err(Failure::error(crate::spec::codes::WRONG_KIND, format!("'{key}' is not a text parameter"))),
    }
    if *t == ElementText::default() {
        m.texts.remove(&id);
    }
    Ok(())
}

/// Re-base a wall on another level: its hosted inserts move to that level with it (ASSUMED, module documentation). The
/// wall's state is regenerated in the same transaction, so a level that leaves the wall without a body is refused by the
/// regeneration's own failure.
pub fn set_wall_base_level(m: &mut BimModel, _cx: &mut TxnContext<'_>, wall: ElementId, level: ElementId) -> Result<(), Failure> {
    m.wall(wall)?;
    m.level(level)?;
    m.walls.get_mut(&wall).expect("checked").base_level = level;
    for i in m.hosted_instances.values_mut().filter(|i| i.host == wall) {
        i.level = level;
    }
    let w = m.wall(wall)?.clone();
    crate::regen::wall_state(m, &w)?;
    Ok(())
}

/// Re-level a floor, ceiling or roof (the palette's `Level`). the slab keeps its height offset, so it moves with the new
/// level (the rule a wall follows when re-based, `set_wall_base_level`), and its state is regenerated in the same
/// transaction so a level that leaves it without a body is refused by the regeneration's own failure.
pub fn set_slab_level(m: &mut BimModel, _cx: &mut TxnContext<'_>, slab: ElementId, level: ElementId) -> Result<(), Failure> {
    m.slab(slab)?;
    m.level(level)?;
    let before = m.slab(slab)?.level;
    m.slab_mut(slab)?.level = level;
    let s = m.slab(slab)?.clone();
    if let Err(f) = crate::regen::slab_state(m, &s) {
        m.slab_mut(slab)?.level = before;
        return Err(f);
    }
    Ok(())
}

/// Set a floor, ceiling or roof's Structural flag (the palette's `Structural`). ENGINE CHOICE: the flag is stored.
pub fn set_slab_structural(m: &mut BimModel, _cx: &mut TxnContext<'_>, slab: ElementId, on: bool) -> Result<(), Failure> {
    m.slab_mut(slab)?.structural = on;
    Ok(())
}

/// Rename a level (module documentation). Refused: an empty name.
pub fn rename_level(m: &mut BimModel, _cx: &mut TxnContext<'_>, level: ElementId, name: &str) -> Result<(), Failure> {
    m.level(level)?;
    if name.trim().is_empty() {
        return Err(Failure::error(crate::spec::codes::WRONG_KIND, "A level needs a name."));
    }
    // OBSERVED: a name another level carries is refused at the call ("Name must be unique").
    if m.levels.values().any(|l| l.id != level && l.name == name) {
        return Err(Failure::error(crate::spec::codes::LEVEL_DUPLICATE_NAME, "Name must be unique.").on(&[level]));
    }
    let old = std::mem::replace(&mut m.levels.get_mut(&level).expect("checked").name, name.to_owned());
    // Nothing happens when the old name is empty or equal to the new one; otherwise every view of the level whose name is
    // EQUAL to the OLD level name is renamed (after one confirmation answer per operation). OBSERVED: the plan made without
    // a name of its own (it took the level's name) is renamed with the level; a plan of the same level with its own name
    // keeps it. UNKNOWN: the default answer of the confirmation when no UI is attached: the engine answers yes, as
    // measured.
    for v in m.views.values_mut().filter(|v| v.gen_level == Some(level) && v.name == old) {
        v.name = name.to_owned();
    }
    Ok(())
}

/// Set a wall's Structural flag.
pub fn set_wall_structural(m: &mut BimModel, _cx: &mut TxnContext<'_>, wall: ElementId, on: bool) -> Result<(), Failure> {
    m.wall(wall)?;
    m.walls.get_mut(&wall).expect("checked").structural = on;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_are_stored_per_element_and_cleared_when_empty() {
        let mut doc = crate::new_document();
        let out = doc.transact("level", false, |m, cx| crate::ops::create_level(m, cx, "L1", 0.0));
        let l = out.value.unwrap();
        let o = doc.transact("text", false, |m, cx| set_element_text(m, cx, l, "comments", "west wing"));
        assert!(o.value.is_some());
        assert_eq!(element_text(doc.model(), l).comments, "west wing");
        let o = doc.transact("text", false, |m, cx| set_element_text(m, cx, l, "comments", ""));
        assert!(o.value.is_some() && doc.model().texts.is_empty());
        let o = doc.transact("text", false, |m, cx| set_element_text(m, cx, l, "colour", "red"));
        assert!(o.value.is_none());
        let o = doc.transact("text", false, |m, cx| set_element_text(m, cx, ElementId(999_999), "mark", "A"));
        assert!(o.value.is_none());
    }

    #[test]
    fn a_slab_is_re_levelled_and_marked_structural_in_one_transaction_each() {
        use asterion_geom::Vec2;
        let mut doc = crate::new_document();
        let (l0, l1, slab) = doc
            .transact("fx", false, |m, cx| {
                let l0 = crate::ops::create_level(m, cx, "L0", 0.0)?;
                let l1 = crate::ops::create_level(m, cx, "L1", 3.0)?;
                let ft = crate::ops::create_floor_type(m, cx, "F300", 0.3)?;
                let pts = vec![Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0), Vec2::new(4.0, 4.0), Vec2::new(0.0, 4.0)];
                let slab = crate::ops::create_slab(m, cx, ft, l0, pts, 0.0, false)?;
                Ok((l0, l1, slab))
            })
            .value
            .unwrap();
        let before = doc.model().slab(slab).unwrap().clone();
        assert_eq!(before.level, l0);
        assert!(doc.transact("lvl", false, |m, cx| set_slab_level(m, cx, slab, l1)).value.is_some());
        let after = doc.model().slab(slab).unwrap();
        assert_eq!(after.level, l1);
        assert_eq!(after.height_offset, before.height_offset, "the slab keeps its offset and moves with the level");
        assert!(doc.transact("st", false, |m, cx| set_slab_structural(m, cx, slab, true)).value.is_some());
        assert!(doc.model().slab(slab).unwrap().structural);
        // a level that does not exist, and an element that is not a slab, are refused and change nothing
        assert!(doc.transact("bad", false, |m, cx| set_slab_level(m, cx, slab, ElementId(999_999))).value.is_none());
        assert!(doc.transact("bad", false, |m, cx| set_slab_level(m, cx, l0, l1)).value.is_none());
        assert_eq!(doc.model().slab(slab).unwrap().level, l1);
        // one undo each
        doc.undo();
        assert!(!doc.model().slab(slab).unwrap().structural);
        doc.undo();
        assert_eq!(doc.model().slab(slab).unwrap().level, l0);
    }
}
