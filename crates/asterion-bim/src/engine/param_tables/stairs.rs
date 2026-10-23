
use super::{def, feet, text_of, Def, Dumped, Listed, Raw, Storage};
use crate::model::BimModel;
use asterion_core::ElementId as Eid;
use Listed::{Category, Class, Generic};
use Storage::{Double, ElementId, Integer, String as Str};

pub const STAIRS: &[Def] = &[
    def(-1152385, "Image", ElementId, false, Category, ""),
    def(-1151154, "Tread/Riser Start Number", Integer, false, Class { class: "StairsElement", conditional: false }, "(StairsElement)"),
    def(-1151110, "Desired Number of Risers", Integer, false, Class { class: "StairsElement", conditional: false }, "(StairsElement)"),
    def(-1151105, "Desired Stair Height", Double, false, Class { class: "StairsElement", conditional: false }, "(StairsElement)"),
    def(-1019016, "IFC Predefined Type", Str, false, Category, ""),
    def(-1019014, "Export to IFC As", Str, false, Category, ""),
    def(-1019012, "Export to IFC", Integer, false, Category, ""),
    def(-1019000, "IfcGUID", Str, false, Category, ""),
    def(-1013200, "Design Option", Str, true, Generic, ""),
    def(-1012101, "Phase Demolished", ElementId, false, Generic, ""),
    def(-1012100, "Phase Created", ElementId, false, Generic, ""),
    def(-1010106, "Comments", Str, false, Category, ""),
    def(-1007250, "Actual Tread Depth", Double, false, Class { class: "StairsElement", conditional: false }, "(StairsElement)"),
    def(-1007246, "Actual Number of Risers", Integer, false, Class { class: "StairsElement", conditional: false }, "(StairsElement)"),
    def(-1007235, "Multistory Top Level", ElementId, false, Class { class: "StairsElement", conditional: true }, "(StairsElement)"),
    def(-1007219, "Top Offset", Double, false, Class { class: "StairsElement", conditional: false }, "(StairsElement)"),
    def(-1007218, "Base Offset", Double, false, Class { class: "StairsElement", conditional: false }, "(StairsElement)"),
    def(-1007206, "Actual Riser Height", Double, false, Class { class: "StairsElement", conditional: false }, "(StairsElement)"),
    def(-1007201, "Top Level", ElementId, false, Class { class: "StairsElement", conditional: false }, "(StairsElement)"),
    def(-1007200, "Base Level", ElementId, false, Class { class: "StairsElement", conditional: false }, "(StairsElement)"),
    def(-1001203, "Mark", Str, false, Category, ""),
];

/// A stairs type (StairsType -> Element), kind 2.
pub const STAIRS_TYPE: &[Def] = &[
    def(-1152384, "Type Image", ElementId, false, Category, ""),
    def(-1151237, "Middle Support", Integer, false, Class { class: "StairsType", conditional: false }, "(StairsType)"),
    def(-1151236, "Left Lateral Offset", Double, false, Class { class: "StairsType", conditional: false }, "(StairsType)"),
    def(-1151235, "Right Lateral Offset", Double, false, Class { class: "StairsType", conditional: false }, "(StairsType)"),
    def(-1151233, "Construction Method", Integer, false, Class { class: "StairsType", conditional: true }, "(StairsType)"),
    def(-1151232, "Right Support", Integer, false, Class { class: "StairsType", conditional: false }, "(StairsType)"),
    def(-1151231, "Left Support", Integer, false, Class { class: "StairsType", conditional: false }, "(StairsType)"),
    def(-1151229, "Vertical Gap Distance", Double, false, Class { class: "StairsType", conditional: true }, "(StairsType)"),
    def(-1151228, "Horizontal Gap Distance", Double, false, Class { class: "StairsType", conditional: true }, "(StairsType)"),
    def(-1151226, "Notch Thickness", Double, false, Class { class: "StairsType", conditional: true }, "(StairsType)"),
    def(-1151225, "Notch Extension", Double, false, Class { class: "StairsType", conditional: true }, "(StairsType)"),
    def(-1151224, "Connection Method", Integer, false, Class { class: "StairsType", conditional: true }, "(StairsType)"),
    def(-1151217, "Middle Support Number", Integer, false, Class { class: "StairsType", conditional: false }, "(StairsType)"),
    def(-1151216, "Minimum Run Width", Double, false, Class { class: "StairsType", conditional: false }, "(StairsType)"),
    def(-1151206, "Calculation Rules", Storage::None, false, Class { class: "StairsType", conditional: false }, "(StairsType)"),
    def(-1140422, "Keynote", Str, false, Category, ""),
    def(-1019017, "Type IFC Predefined Type", Str, false, Category, ""),
    def(-1019015, "Export Type to IFC As", Str, false, Category, ""),
    def(-1019013, "Export Type to IFC", Integer, false, Category, ""),
    def(-1019001, "Type IfcGUID", Str, false, Category, ""),
    def(-1013200, "Design Option", Str, true, Generic, ""),
    def(-1010109, "Model", Str, false, Category, ""),
    def(-1010108, "Manufacturer", Str, false, Category, ""),
    def(-1010105, "Type Comments", Str, false, Category, ""),
    def(-1010104, "URL", Str, false, Category, ""),
    def(-1010103, "Description", Str, false, Category, ""),
    def(-1007203, "Minimum Tread Depth", Double, false, Class { class: "StairsType", conditional: false }, "(StairsType)"),
    def(-1007202, "Maximum Riser Height", Double, false, Class { class: "StairsType", conditional: false }, "(StairsType)"),
    def(-1002501, "Assembly Description", Str, true, Category, ""),
    def(-1002500, "Assembly Code", Str, false, Category, ""),
    def(-1001405, "Type Mark", Str, false, Category, ""),
    def(-1001205, "Cost", Double, false, Category, ""),
    def(-1001006, "Function", Integer, false, Category, "(category -2000120 entry, usage 69)"),
];

pub const RAILING_TYPE: &[Def] = &[
    def(-1152384, "Type Image", ElementId, false, Category, ""),
    def(-1150380, "Use Top Rail", Integer, false, Class { class: "BaseRailingAttr", conditional: true }, "(BaseRailingAttr)"),
    def(-1140422, "Keynote", Str, false, Category, ""),
    def(-1019017, "Type IFC Predefined Type", Str, false, Category, ""),
    def(-1019015, "Export Type to IFC As", Str, false, Category, ""),
    def(-1019013, "Export Type to IFC", Integer, false, Category, ""),
    def(-1019001, "Type IfcGUID", Str, false, Category, ""),
    def(-1013200, "Design Option", Str, true, Generic, ""),
    def(-1010109, "Model", Str, false, Category, ""),
    def(-1010108, "Manufacturer", Str, false, Category, ""),
    def(-1010105, "Type Comments", Str, false, Category, ""),
    def(-1010104, "URL", Str, false, Category, ""),
    def(-1010103, "Description", Str, false, Category, ""),
    def(-1008632, "Rail Connections", Integer, false, Class { class: "BaseRailingAttr", conditional: true }, "(BaseRailingAttr)"),
    def(-1008631, "Angled Joins", Integer, false, Class { class: "BaseRailingAttr", conditional: true }, "(BaseRailingAttr)"),
    def(-1008630, "Tangent Joins", Integer, false, Class { class: "BaseRailingAttr", conditional: true }, "(BaseRailingAttr)"),
    def(-1008629, "Landing Height Adjustment", Double, false, Class { class: "BaseRailingAttr", conditional: true }, "(BaseRailingAttr)"),
    def(-1008628, "Use Landing Height Adjustment", Integer, false, Class { class: "BaseRailingAttr", conditional: true }, "(BaseRailingAttr)"),
    def(-1008626, "Baluster Placement", Storage::None, false, Class { class: "BaseRailingAttr", conditional: true }, "(BaseRailingAttr)"),
    def(-1008619, "Baluster Offset", Double, false, Class { class: "BaseRailingAttr", conditional: false }, "(BaseRailingAttr)"),
    def(-1008617, "Rail Offset", Double, false, Class { class: "BaseRailingAttr", conditional: true }, "(BaseRailingAttr)"),
    def(-1008615, "Rail Structure (Non-Continuous)", Storage::None, false, Class { class: "BaseRailingAttr", conditional: true }, "(BaseRailingAttr)"),
    def(-1008614, "Rail Shape", Integer, false, Class { class: "BaseRailingAttr", conditional: true }, "(BaseRailingAttr)"),
    def(-1008612, "Baluster Length", Double, false, Class { class: "BaseRailingAttr", conditional: true }, "(BaseRailingAttr)"),
    def(-1008611, "Baluster Width", Double, false, Class { class: "BaseRailingAttr", conditional: true }, "(BaseRailingAttr)"),
    def(-1008610, "Balusters Per Tread (Stair Railing Only)", Integer, false, Class { class: "BaseRailingAttr", conditional: true }, "(BaseRailingAttr)"),
    def(-1008609, "Baluster Separation", Double, false, Class { class: "BaseRailingAttr", conditional: true }, "(BaseRailingAttr)"),
    def(-1008608, "Baluster Spacing Type (Stair Railing Only)", Integer, false, Class { class: "BaseRailingAttr", conditional: true }, "(BaseRailingAttr)"),
    def(-1008605, "Baluster Shape", Integer, false, Class { class: "BaseRailingAttr", conditional: true }, "(BaseRailingAttr)"),
    def(-1008604, "Railing Thickness", Double, false, Class { class: "BaseRailingAttr", conditional: true }, "(BaseRailingAttr)"),
    def(-1008603, "Railing Width", Double, false, Class { class: "BaseRailingAttr", conditional: true }, "(BaseRailingAttr)"),
    def(-1008602, "Railing Height", Double, false, Class { class: "BaseRailingAttr", conditional: false }, "(BaseRailingAttr)"),
    def(-1002501, "Assembly Description", Str, true, Category, ""),
    def(-1002500, "Assembly Code", Str, false, Category, ""),
    def(-1001405, "Type Mark", Str, false, Category, ""),
    def(-1001205, "Cost", Double, false, Category, ""),
];

/// The dump of a stairs element. Read-only: Top Offset is not modifiable when the top level override is invalid and Desired
/// Stair Height when it is valid (the engine's stairs always have a top level); for a member of a multistory stairs Actual
/// Tread Depth, Base Level and Top Level are not modifiable; Actual Tread Depth also when the stairs has no component run
/// (the engine has no edit mode in a replay).
pub fn stairs_dump(m: &BimModel, id: Eid) -> Option<Vec<Dumped>> {
    let s = m.stairs.stairs.get(&id)?;
    let v = crate::stairs_scene::scene_view(m, id).ok()?;
    let text = |key: &str| text_of(crate::instance_params::element_text(m, id).get(key));
    Some(
        STAIRS
            .iter()
            .map(|d| {
                let (read_only, raw) = match d.id {
                    // Actual Tread Depth
                    -1007250 => (Some(s.multistory_member || v.runs.is_empty()), feet(v.tread_depth)),
                    -1007246 => (Some(d.registered_read_only), Raw::Int(i64::from(v.actual_risers))),
                    -1007206 => (Some(d.registered_read_only), feet(v.riser_height)),
                    // Desired Stair Height: not modifiable while the top level override is valid
                    -1151105 => (Some(true), feet(v.height)),
                    -1151110 => (Some(d.registered_read_only), Raw::Int(i64::from(v.desired_risers))),
                    // Top Offset: modifiable while the top level override is valid; the engine's stairs keep no offset (ENGINE FACT, 0)
                    -1007219 => (Some(false), Raw::Num(0.0)),
                    -1007218 => (Some(d.registered_read_only), Raw::Num(0.0)),
                    -1012101 => {
                        let none = crate::phases::element_phases(m, id).map_or(true, |p| p.demolished.is_none());
                        (Some(d.registered_read_only), if none { Raw::Id(-1) } else { Raw::Unknown })
                    }
                    -1007201 => (Some(s.multistory_member), Raw::Id(s.top_level.0 as i64)),
                    -1007200 => (Some(s.multistory_member), Raw::Id(s.base_level.0 as i64)),
                    -1010106 => (Some(d.registered_read_only), text("comments")),
                    -1001203 => (Some(d.registered_read_only), text("mark")),
                    // ENGINE FACT: no image, no IFC data
                    -1152385 => (Some(d.registered_read_only), Raw::Id(-1)),
                    -1019016 | -1019014 => (Some(d.registered_read_only), Raw::Text(None)),
                    -1019012 => (Some(d.registered_read_only), Raw::Int(0)),
                    _ => (Some(d.registered_read_only), Raw::Unknown),
                };
                Dumped { def: d, read_only, raw }
            })
            .collect(),
    )
}

const STAIRS_TYPE_STATE_DEPENDENT: &[i32] = &[-1151217, -1151211, -1151210, -1151209, -1151236, -1151235, -1151229, -1151228, -1151226, -1151225, -1151224];

/// The dump of a stairs type.
pub fn stairs_type_dump(m: &BimModel, id: Eid) -> Option<Vec<Dumped>> {
    let t = m.stairs.types.get(&id)?;
    Some(
        STAIRS_TYPE
            .iter()
            .map(|d| {
                let read_only = if STAIRS_TYPE_STATE_DEPENDENT.contains(&d.id) { None } else { Some(d.registered_read_only) };
                let raw = match d.id {
                    // the type fields the engine keeps
                    -1151216 => feet(t.params.min_run_width),
                    -1007203 => feet(t.params.min_tread_depth),
                    -1007202 => feet(t.params.max_riser_height),
                    -1152384 => Raw::Id(-1),
                    -1140422 | -1019017 | -1019015 | -1010109 | -1010108 | -1010105 | -1010104 | -1010103 | -1001405 => Raw::Text(None),
                    -1019013 => Raw::Int(0),
                    -1001205 => Raw::Num(0.0),
                    -1002500 | -1002501 => Raw::Text(Some(String::new())),
                    _ => Raw::Unknown,
                };
                Dumped { def: d, read_only, raw }
            })
            .collect(),
    )
}

/// The dump of a railing type.
pub fn railing_type_dump(m: &BimModel, id: Eid) -> Option<Vec<Dumped>> {
    m.stairs_scene.railing_types.get(&id)?;
    Some(
        RAILING_TYPE
            .iter()
            .map(|d| {
                let raw = match d.id {
                    -1152384 => Raw::Id(-1),
                    -1140422 | -1019017 | -1019015 | -1010109 | -1010108 | -1010105 | -1010104 | -1010103 | -1001405 => Raw::Text(None),
                    -1019013 => Raw::Int(0),
                    -1001205 => Raw::Num(0.0),
                    -1002500 | -1002501 => Raw::Text(Some(String::new())),
                    _ => Raw::Unknown,
                };
                // (railing attributes): these depend on the baluster mode, the height adjustment type and the rail shape of
                // the type, which the engine does not keep: unknown
                let state_dependent = ["Railing Height", "Landing Height Adjustment", "Baluster Separation", "Railing Thickness"].contains(&d.name);
                Dumped { def: d, read_only: if state_dependent { None } else { Some(d.registered_read_only) }, raw }
            })
            .collect(),
    )
}
