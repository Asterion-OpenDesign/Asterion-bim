
use super::{def, feet, text_of, yes_no, Def, Dumped, Listed, Raw, Storage};
use crate::columns::{Column, ColumnType};
use crate::model::BimModel;
use Listed::{Category, Class, Generic};
use Storage::{Double, ElementId, Integer, String as Str};

pub const COLUMN: &[Def] = &[
    def(-1152385, "Image", ElementId, false, Category, ""),
    def(-1150207, "Base Extension", Double, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1150206, "Top Extension", Double, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1150190, "Base Geometry Alignment", Integer, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1150189, "Top Geometry Alignment", Integer, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1150173, "Move Base With Grids", Integer, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1150172, "Move Top With Grids", Integer, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1150171, "Column Style", Integer, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1019016, "IFC Predefined Type", Str, false, Category, ""),
    def(-1019014, "Export to IFC As", Str, false, Category, ""),
    def(-1019012, "Export to IFC", Integer, false, Category, ""),
    def(-1019000, "IfcGUID", Str, false, Category, ""),
    def(-1013414, "Lateral Justification", Integer, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1013413, "z-Direction Justification", Integer, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1013200, "Design Option", Str, true, Generic, ""),
    def(-1012806, "Volume", Double, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1012101, "Phase Demolished", ElementId, false, Generic, ""),
    def(-1012100, "Phase Created", ElementId, false, Generic, ""),
    def(-1010301, "Width", Double, true, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1010300, "Height", Double, true, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1010106, "Comments", Str, false, Category, ""),
    def(-1002560, "Base is Attached", Integer, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1002559, "Top is Attached", Integer, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1002558, "Offset From Attachment At Base", Double, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1002557, "Offset From Attachment At Top", Double, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001951, "Height Offset From Level", Double, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001598, "Elevation at Top", Double, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001586, "Cross-Section Rotation", Double, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001574, "z-Direction Offset Value", Double, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001572, "End Level Offset", Double, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001571, "Start Level Offset", Double, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001561, "Elevation at Bottom", Double, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001371, "Moves With Grids", Integer, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001370, "Moves With Nearby Elements", Integer, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001365, "Schedule Level", ElementId, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001364, "Offset from Host", Double, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001363, "Host", Str, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001362, "Head Height", Double, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001361, "Sill Height", Double, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001360, "Elevation from Level", Double, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001358, "Top Offset", Double, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001357, "Base Offset", Double, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001352, "Level", ElementId, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001351, "Top Level", ElementId, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001350, "Base Level", ElementId, false, Class { class: "FamilyInstance", conditional: true }, "(FamilyInstance)"),
    def(-1001203, "Mark", Str, false, Category, ""),
];

pub const COLUMN_TYPE: &[Def] = &[
    def(-1152384, "Type Image", ElementId, false, Category, ""),
    def(-1150141, "Light Source Definition (family)", Str, false, Class { class: "FamilySymbol", conditional: true }, "(FamilySymbol)"),
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
    def(-1005431, "Thermal Resistance (R)", Double, true, Class { class: "FamilySymbol", conditional: true }, "(FamilySymbol)"),
    def(-1002503, "OmniClass Title", Str, true, Class { class: "FamilySymbol", conditional: true }, "(FamilySymbol)"),
    def(-1002502, "OmniClass Number", Str, false, Class { class: "FamilySymbol", conditional: true }, "(FamilySymbol)"),
    def(-1002501, "Assembly Description", Str, true, Category, ""),
    def(-1002500, "Assembly Code", Str, false, Category, ""),
    def(-1001405, "Type Mark", Str, false, Category, ""),
    def(-1001205, "Cost", Double, false, Category, ""),
];

/// The dump of a column. Read-only: the registered flag; Volume, Level and the attachment flags have no rule in Spec
/// PARAM-MODIFIABLE-FAMILY-01 for an ordinary column (an instance without an attachment record is modifiable at both ends).
pub fn column_dump(m: &BimModel, c: &Column) -> Vec<Dumped> {
    let text = |key: &str| text_of(crate::instance_params::element_text(m, c.id).get(key));
    COLUMN
        .iter()
        .map(|d| {
            let raw = match d.id {
                -1001350 => Raw::Id(c.base_level.0 as i64),
                -1001352 => Raw::Id(c.base_level.0 as i64),
                -1001351 => c.top_level.map_or(Raw::Unknown, |l| Raw::Id(l.0 as i64)),
                -1001357 => feet(c.base_offset),
                -1001358 => feet(c.top_offset),
                -1001371 => yes_no(c.moves_with_grids),
                -1012101 => {
                    if crate::phases::element_phases(m, c.id).map_or(true, |p| p.demolished.is_none()) {
                        Raw::Id(-1)
                    } else {
                        Raw::Unknown
                    }
                }
                -1010106 => text("comments"),
                -1001203 => text("mark"),
                // ENGINE FACT: no image, no IFC data
                -1152385 => Raw::Id(-1),
                -1019016 | -1019014 => Raw::Text(None),
                -1019012 => Raw::Int(0),
                _ => Raw::Unknown,
            };
            // Spec PARAM-MODIFIABLE-FAMILY-01: when an attachment record exists at the end, whether Base / Top Offset is
            // modifiable depends on the record flag, which the engine does not keep: unknown
            let read_only = match d.id {
                -1001357 if c.base_attachment.is_some() => None,
                -1001358 if c.top_attachment.is_some() => None,
                // Spec PARAM-MODIFIABLE-FAMILY-01: Level is modifiable "only for a stand-alone instance that is not a
                // curve-driven, mullion or curtain panel element with a valid associated level and not a group member": the
                // text does not say which side of the 'with a valid associated level' condition decides, so the state is
                // unknown
                -1001352 => None,
                _ => Some(d.registered_read_only),
            };
            Dumped { def: d, read_only, raw }
        })
        .collect()
}

/// The dump of a column type. Read-only: Spec PARAM-MODIFIABLE-FAMILY-01 - OmniClass Number and Title are never modifiable.
pub fn column_type_dump(_m: &BimModel, _t: &ColumnType) -> Vec<Dumped> {
    COLUMN_TYPE
        .iter()
        .map(|d| {
            let read_only = match d.id {
                -1002502 | -1002503 => Some(true),
                -1152384 => None,
                _ => Some(d.registered_read_only),
            };
            let raw = match d.id {
                -1152384 => Raw::Id(-1),
                -1140422 | -1019017 | -1019015 | -1010109 | -1010108 | -1010105 | -1010104 | -1010103 | -1001405 => Raw::Text(None),
                -1019013 => Raw::Int(0),
                -1001205 => Raw::Num(0.0),
                -1002500 | -1002501 => Raw::Text(Some(String::new())),
                _ => Raw::Unknown,
            };
            Dumped { def: d, read_only, raw }
        })
        .collect()
}
