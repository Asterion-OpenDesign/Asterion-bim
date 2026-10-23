//! Duplicating and renaming the TYPES of the model elements.
//!
//! `ops::duplicate_hosted_type` already existed for doors and windows; the shell's dialog duplicated the other types by creating a new
//! one from the values it showed (no name check, no copy of what the dialog does not show). One entry per kind of type now:
//!
//! * a wall type (a curtain wall type is a wall type with a curtain part keyed by the same id: the part is copied too),
//! * a floor / ceiling / roof type (with its variable layer),
//! * a door / window type,
//! * a column type,
//! * a mullion type,
//! * a stairs type (the engine's own duplicate, `stairs_scene::duplicate_stairs_type`) and a railing type.
//!
//! The name checks of a duplicate are the reference product's: empty, prohibited characters, empty once trimmed, already in
//! use among the types of the same class; the copy is named exactly the trimmed name (no "copy" name is made). The class of
//! a wall type, a slab type, a column type and a mullion type is the kind itself (ENGINE CHOICE: compares "every type of
//! the same class"; which classes the reference product has for walls and slabs was not specified, so each engine kind is
//! one class).
//!
//! The duplicate's checks are the nearest reading with the type itself excluded from "in use", so keeping the name is
//! allowed.
//!
//! UNKNOWN: which properties a copy carries per class: the engine copies every stored field of the kind.

use crate::model::BimModel;
use crate::spec::codes;
use crate::type_names::check_duplicate_name;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};

fn not_a_type(id: ElementId) -> Failure {
    Failure::error(codes::NOT_FOUND, format!("Element {} is not a wall, floor, door, window, column, mullion, stairs or railing type.", id.0)).on(&[id])
}

/// The class of a type for the checks of a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeClass {
    Wall,
    Slab,
    Hosted,
    Column,
    Mullion,
    Stairs,
    Railing,
}

/// The class of `id` when it is a type this module handles.
pub fn class_of(m: &BimModel, id: ElementId) -> Option<TypeClass> {
    if m.wall_types.contains_key(&id) {
        Some(TypeClass::Wall)
    } else if m.floor_types.contains_key(&id) {
        Some(TypeClass::Slab)
    } else if m.hosted_types.contains_key(&id) {
        Some(TypeClass::Hosted)
    } else if m.column_types.contains_key(&id) {
        Some(TypeClass::Column)
    } else if m.mullion_types.contains_key(&id) {
        Some(TypeClass::Mullion)
    } else if m.stairs.types.contains_key(&id) {
        Some(TypeClass::Stairs)
    } else if m.stairs_scene.railing_types.contains_key(&id) {
        Some(TypeClass::Railing)
    } else {
        None
    }
}

fn name_taken(m: &BimModel, class: TypeClass, name: &str, except: Option<ElementId>) -> bool {
    let other = |id: ElementId| Some(id) != except;
    match class {
        TypeClass::Wall => m.wall_types.values().any(|t| other(t.id) && t.name == name),
        TypeClass::Slab => m.floor_types.values().any(|t| other(t.id) && t.name == name),
        TypeClass::Hosted => m.hosted_types.values().any(|t| other(t.id) && t.name == name),
        TypeClass::Column => m.column_types.values().any(|t| other(t.id) && t.name == name),
        TypeClass::Mullion => m.mullion_types.values().any(|t| other(t.id) && t.name == name),
        TypeClass::Stairs => m.stairs.types.values().any(|t| other(t.id) && t.name == name),
        TypeClass::Railing => m.stairs_scene.railing_types.values().any(|t| other(t.id) && t.name == name),
    }
}

/// A copy of the type `source` under `name`; the document's defaults stay as they were.
pub fn duplicate_type(m: &mut BimModel, cx: &mut TxnContext<'_>, source: ElementId, name: &str) -> Result<ElementId, Failure> {
    let class = class_of(m, source).ok_or_else(|| not_a_type(source))?;
    let name = check_duplicate_name(name, |n| name_taken(m, class, n, None)).map_err(|f| f.on(&[source]))?;
    match class {
        TypeClass::Hosted => crate::ops::duplicate_hosted_type(m, cx, source, &name),
        TypeClass::Wall => {
            let src = m.wall_types[&source].clone();
            let id = cx.new_id();
            m.wall_types.insert(id, crate::model::WallType { id, name, structure: src.structure });
            // a curtain wall type: the grid and the mullion settings belong to the type
            if let Some(c) = m.curtain_types.get(&source).cloned() {
                m.curtain_types.insert(id, c);
            }
            Ok(id)
        }
        TypeClass::Slab => {
            let src = m.floor_types[&source].clone();
            let id = cx.new_id();
            m.floor_types.insert(id, crate::model::SlabType { id, name, ..src });
            if let Some(layer) = m.slab_variable_layers.get(&source).copied() {
                m.slab_variable_layers.insert(id, layer);
            }
            Ok(id)
        }
        TypeClass::Column => {
            let src = m.column_types[&source].clone();
            let id = cx.new_id();
            m.column_types.insert(id, crate::columns::ColumnType { id, name, ..src });
            Ok(id)
        }
        TypeClass::Mullion => {
            let src = m.mullion_types[&source].clone();
            let id = cx.new_id();
            m.mullion_types.insert(id, crate::curtains::MullionType { id, name, ..src });
            Ok(id)
        }
        TypeClass::Stairs => crate::stairs_scene::duplicate_stairs_type(m, cx, source, &name),
        TypeClass::Railing => {
            let id = cx.new_id();
            m.stairs_scene.railing_types.insert(id, crate::stairs_scene::RailingType { id, name });
            Ok(id)
        }
    }
}

/// Renames a type: the same name checks, the type itself excluded from "in use".
pub fn rename_type(m: &mut BimModel, _cx: &mut TxnContext<'_>, id: ElementId, name: &str) -> Result<(), Failure> {
    let class = class_of(m, id).ok_or_else(|| not_a_type(id))?;
    let name = check_duplicate_name(name, |n| name_taken(m, class, n, Some(id))).map_err(|f| f.on(&[id]))?;
    match class {
        TypeClass::Wall => m.wall_types.get_mut(&id).expect("checked").name = name,
        TypeClass::Slab => m.floor_types.get_mut(&id).expect("checked").name = name,
        TypeClass::Hosted => m.hosted_types.get_mut(&id).expect("checked").name = name,
        TypeClass::Column => m.column_types.get_mut(&id).expect("checked").name = name,
        TypeClass::Mullion => m.mullion_types.get_mut(&id).expect("checked").name = name,
        TypeClass::Stairs => m.stairs.types.get_mut(&id).expect("checked").name = name,
        TypeClass::Railing => m.stairs_scene.railing_types.get_mut(&id).expect("checked").name = name,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ops, CompoundStructure, Layer, LayerFunction};

    #[test]
    fn a_type_is_duplicated_and_renamed_under_the_product_name_checks() {
        let mut doc = crate::new_document();
        let (wt, ft, ct) = doc
            .transact("fx", false, |m, cx| {
                let wt = ops::create_wall_type(m, cx, "W200", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
                let ft = ops::create_floor_type(m, cx, "F300", 0.3)?;
                let ct = crate::columns::create_column_type(m, cx, "C600", Some(0.6), Some(0.6))?;
                Ok((wt, ft, ct))
            })
            .value
            .unwrap();
        let copy = doc.transact("dup", false, |m, cx| duplicate_type(m, cx, wt, "  W200 copy ")).value.unwrap();
        assert_eq!(doc.model().wall_types[&copy].name, "W200 copy", "the copy is named exactly the trimmed name");
        assert_eq!(doc.model().wall_types[&copy].structure, doc.model().wall_types[&wt].structure);
        for (src, copy_name) in [(ft, "F300 b"), (ct, "C600 b")] {
            let c = doc.transact("dup", false, |m, cx| duplicate_type(m, cx, src, copy_name)).value.unwrap();
            assert_ne!(c, src);
        }
        // the refusals, in the reference product's order
        let refused = |doc: &mut crate::BimDocument, name: &str| doc.transact("dup", false, |m, cx| duplicate_type(m, cx, wt, name)).value.is_none();
        assert!(refused(&mut doc, ""), "empty");
        assert!(refused(&mut doc, "{x}"), "prohibited characters");
        assert!(refused(&mut doc, "   "), "empty once trimmed");
        assert!(refused(&mut doc, "W200"), "in use");
        // a rename keeps its own name and refuses another type's
        assert!(doc.transact("ren", false, |m, cx| rename_type(m, cx, copy, "W200 copy")).value.is_some());
        assert!(doc.transact("ren", false, |m, cx| rename_type(m, cx, copy, "W200")).value.is_none());
        assert!(doc.transact("ren", false, |m, cx| rename_type(m, cx, copy, "Renamed")).value.is_some());
        assert_eq!(doc.model().wall_types[&copy].name, "Renamed");
        // not a type
        assert!(doc.transact("ren", false, |m, cx| rename_type(m, cx, ElementId(999_999), "x")).value.is_none());
    }
}
