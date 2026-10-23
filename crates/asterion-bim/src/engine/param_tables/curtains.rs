
use super::{def, Def, Dumped, Listed, Raw, Storage};
use crate::curtains::{MullionType, ROUND_MULLION_TYPES};
use crate::model::BimModel;
use Listed::{Category, Class, Generic};
use Storage::{Double, ElementId, Integer, String as Str};

pub const MULLION_TYPE: &[Def] = &[
    def(-1152384, "Type Image", ElementId, false, Category, ""),
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
    def(-1007363, "Angle", Double, false, Class { class: "SysMullionFamSym", conditional: true }, "(SysMullionFamSym)"),
    def(-1002503, "OmniClass Title", Str, true, Class { class: "FamilySymbol", conditional: true }, "(FamilySymbol)"),
    def(-1002502, "OmniClass Number", Str, false, Class { class: "FamilySymbol", conditional: true }, "(FamilySymbol)"),
    def(-1002501, "Assembly Description", Str, true, Category, ""),
    def(-1002500, "Assembly Code", Str, false, Category, ""),
    def(-1002107, "Material", ElementId, false, Class { class: "SysMullionFamSym", conditional: false }, "(SysMullionFamSym)"),
    def(-1001405, "Type Mark", Str, false, Category, ""),
    def(-1001205, "Cost", Double, false, Category, ""),
];

/// The dump of a mullion type. Read-only: Angle is not modifiable for a circular mullion; OmniClass Number and Title are
/// never modifiable and the image-browse parameter depends on the family flag (Spec PARAM-MODIFIABLE-FAMILY-01:
/// unspecified, unknown).
pub fn mullion_type_dump(_m: &BimModel, t: &MullionType) -> Vec<Dumped> {
    MULLION_TYPE
        .iter()
        .map(|d| {
            let read_only = match d.id {
                -1007363 => Some(d.registered_read_only || ROUND_MULLION_TYPES.contains(&t.name.as_str())),
                -1002502 | -1002503 => Some(true),
                -1152384 => None,
                _ => Some(d.registered_read_only),
            };
            let raw = match d.id {
                -1152384 => Raw::Id(-1),
                -1140422 | -1019017 | -1019015 | -1010109 | -1010108 | -1010105 | -1010104 | -1010103 | -1001405 => Raw::Text(None),
                -1019013 => Raw::Int(0),
                -1001205 => Raw::Num(0.0),
                // no classification: the empty string, not the absent one
                -1002500 | -1002501 | -1002502 | -1002503 => Raw::Text(Some(String::new())),
                _ => Raw::Unknown,
            };
            Dumped { def: d, read_only, raw }
        })
        .collect()
}
