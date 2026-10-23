
use super::{def, feet, text_of, yes_no, Def, Dumped, Listed, Raw, Storage};
use crate::model::{BimModel, Wall, WallType};

use Listed::{Category, Class, Generic, HeightHelper};
use Storage::{Double, ElementId, Integer, String as Str};

const VWALL: Listed = Class { class: "VWall", conditional: false };
const VWALL_IF: Listed = Class { class: "VWall", conditional: true };

/// The entries below are the ones a row proves.
pub const WALL: &[Def] = &[
    def(-1019100, "Cross-Section", Integer, false, VWALL, "(VWall adds WALL_CROSS_SECTION)"),
    def(-1001119, "Structural Usage", Integer, false, VWALL, "(Structural Usage)"),
    def(-1001596, "Structural", Integer, false, VWALL, "(Structural)"),
    def(-1001713, "Related to Mass", Integer, false, VWALL, "PARAMLIST-CLASS-01"),
    def(-1001118, "Base is Attached", Integer, false, VWALL_IF, "(when the wall is neither an infill nor a host panel)"),
    def(-1001117, "Top is Attached", Integer, false, VWALL_IF, "(when the wall is neither an infill nor a host panel)"),
    def(-1001122, "Location Line", Integer, false, VWALL_IF, "(WALL_KEY_REF_PARAM)"),
    def(-1012829, "Base Extension Distance", Double, false, VWALL_IF, "(stacked sub-wall only)"),
    def(-1012828, "Top Extension Distance", Double, false, VWALL_IF, "(stacked sub-wall only)"),
    def(-1001107, "Base Constraint", ElementId, false, HeightHelper, ""),
    def(-1001108, "Base Offset", Double, false, HeightHelper, ""),
    def(-1001103, "Top Constraint", ElementId, false, HeightHelper, ""),
    def(-1001109, "Top Offset", Double, false, HeightHelper, "(Top Offset)"),
    def(-1001105, "Unconnected Height", Double, false, HeightHelper, "(Unconnected Height)"),
    def(-1152385, "Image", ElementId, false, Category, ""),
    def(-1019016, "IFC Predefined Type", Str, false, Category, ""),
    def(-1019014, "Export to IFC As", Str, false, Category, ""),
    def(-1019012, "Export to IFC", Integer, false, Category, ""),
    def(-1019000, "IfcGUID", Str, false, Category, ""),
    def(-1010106, "Comments", Str, false, Category, ""),
    def(-1001203, "Mark", Str, false, Category, ""),
    def(-1012100, "Phase Created", ElementId, false, Generic, ""),
    def(-1012101, "Phase Demolished", ElementId, false, Generic, ""),
    def(-1013200, "Design Option", Str, true, Generic, "(when design options exist); the API answers ElementId (OQ-PARAM-API-STORAGE)"),
];

pub const WALL_TYPE: &[Def] = &[
    def(-1002110, "Coarse Scale Fill Color", Integer, false, Class { class: "HostObjAttr", conditional: true }, "(HostObjAttr adds 4 ids)"),
    def(-1002106, "Coarse Scale Fill Pattern", ElementId, false, Class { class: "HostObjAttr", conditional: true }, "(HostObjAttr adds 4 ids)"),
    def(-1001206, "Fire Rating", Str, false, Category, "(Walls category entry, usage 119)"),
    def(-1001006, "Function", Integer, false, Category, "(Walls category entry, usage 71)"),
    def(-1005500, "Structural Material", ElementId, false, Category, "(Walls category entry, usage 119) (HostObjAttr: not modifiable)"),
    def(-1001405, "Type Mark", Str, false, Category, ""),
    def(-1010103, "Description", Str, false, Category, ""),
    def(-1010104, "URL", Str, false, Category, ""),
    def(-1010105, "Type Comments", Str, false, Category, ""),
    def(-1001205, "Cost", Double, false, Category, ""),
    def(-1010108, "Manufacturer", Str, false, Category, ""),
    def(-1010109, "Model", Str, false, Category, ""),
    def(-1002500, "Assembly Code", Str, false, Category, ""),
    def(-1002501, "Assembly Description", Str, true, Category, ""),
    def(-1140422, "Keynote", Str, false, Category, ""),
    def(-1019001, "Type IfcGUID", Str, false, Category, ""),
    def(-1019013, "Export Type to IFC", Integer, false, Category, ""),
    def(-1019015, "Export Type to IFC As", Str, false, Category, ""),
    def(-1019017, "Type IFC Predefined Type", Str, false, Category, ""),
    def(-1152384, "Type Image", ElementId, false, Category, ""),
    def(-1013200, "Design Option", Str, true, Generic, "(when design options exist); the API answers ElementId (OQ-PARAM-API-STORAGE)"),
];

/// The dump of a wall.
pub fn wall_dump(m: &BimModel, w: &Wall) -> Vec<Dumped> {
    let state = crate::regen::wall_state(m, w).ok();
    let constrained = w.top_level.is_some();
    // the element-text keys the Properties palette edits (instance_params): an unset text reads as no string
    let text = |key: &str| text_of(crate::instance_params::element_text(m, w.id).get(key));
    WALL.iter()
        .map(|d| {
            let (read_only, raw) = match d.id {
                // Editable only on an unconnected wall (top constraint set and base level valid => not modifiable)
                -1001105 => (Some(d.registered_read_only || constrained), state.as_ref().map_or(Raw::Unknown, |s| feet(s.height))),
                // The top offset is editable only with a top level
                -1001109 => (Some(d.registered_read_only || !constrained), feet(w.top_offset)),
                -1001108 => (Some(d.registered_read_only), feet(w.base_offset)),
                -1001107 => (Some(d.registered_read_only), Raw::Id(w.base_level.0 as i64)),
                -1001103 => (Some(d.registered_read_only), Raw::Id(w.top_level.map_or(-1, |l| l.0 as i64))),
                // Structural Usage is modifiable only for a structurally significant wall that is not an infill (the engine
                // has no infill walls); the code of the enumeration is not specified
                -1001119 => (Some(d.registered_read_only || !w.structural), Raw::Unknown),
                -1001596 => (Some(d.registered_read_only), yes_no(w.structural)),
                // The extension distances are modifiable only on the top / bottom sub-wall of a stacked wall whose type
                // accepts them; no wall of the model is such a sub-wall (the members of a stacked wall are derived,
                // `crate::stacked::members`), and the engine has no extension of a wall end: ENGINE FACT, 0. The value of a
                // sub-wall comes from the record and is not modelled here.
                -1012829 | -1012828 => (Some(true), Raw::Num(0.0)),
                // OBSERVED code (the engine's Location Line enumeration, compared in walls/params): the code table of the
                // reference product's WALL_KEY_REF_PARAM is not specified
                -1001122 => (Some(d.registered_read_only), Raw::Int(w.location_line.code())),
                // ENGINE FACT: ASTERION has no mass elements (a wall cannot be made from a mass) and no attachment of a
                // wall to a floor or a roof, so the three flags read No
                -1001713 | -1001118 | -1001117 => (Some(d.registered_read_only), Raw::Int(0)),
                -1019100 => (Some(d.registered_read_only), Raw::Unknown),
                // ENGINE FACT: no image, no IFC data, no design options: the reference product's invalid / empty values
                -1152385 => (Some(d.registered_read_only), Raw::Id(-1)),
                -1019016 | -1019014 => (Some(d.registered_read_only), Raw::Text(None)),
                -1019012 => (Some(d.registered_read_only), Raw::Int(0)),
                -1019000 => (Some(d.registered_read_only), Raw::Unknown),
                -1010106 => (Some(d.registered_read_only), text("comments")),
                -1001203 => (Some(d.registered_read_only), text("mark")),
                -1012101 => {
                    let none = crate::phases::element_phases(m, w.id).map_or(true, |p| p.demolished.is_none());
                    (Some(d.registered_read_only), if none { Raw::Id(-1) } else { Raw::Unknown })
                }
                _ => (Some(d.registered_read_only), Raw::Unknown),
            };
            Dumped { def: d, read_only, raw }
        })
        .collect()
}

/// The dump of a wall type. The values the engine holds: none of the type's own parameters except the ones below.
pub fn wall_type_dump(_m: &BimModel, _t: &WallType) -> Vec<Dumped> {
    WALL_TYPE
        .iter()
        .map(|d| {
            let raw = match d.id {
                // no image, no keynote, no IFC data, no manufacturer data, no cost, no material, no fill pattern or colour override
                -1152384 | -1002106 | -1005500 => Raw::Id(-1),
                -1140422 | -1019015 | -1019017 | -1010108 | -1010109 | -1010105 | -1010104 | -1010103 | -1001405 | -1001206 => Raw::Text(None),
                -1019013 | -1002110 => Raw::Int(0),
                -1001205 => Raw::Num(0.0),
                // no assembly classification: the empty string, not the absent one (the reference product reads "" on every
                // type)
                -1002500 | -1002501 => Raw::Text(Some(String::new())),
                _ => Raw::Unknown,
            };
            Dumped { def: d, read_only: Some(d.registered_read_only || d.id == -1005500), raw }
        })
        .collect()
}
