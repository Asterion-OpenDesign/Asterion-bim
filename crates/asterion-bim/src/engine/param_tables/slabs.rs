
use super::{def, feet, text_of, Def, Dumped, Listed, Raw, Storage};
use crate::model::{BimModel, Slab, SlabType};
use Listed::{Category, Class, Generic};
use Storage::{Double, ElementId, Integer, String as Str};

pub const ROOF: &[Def] = &[
    def(-1152385, "Image", ElementId, false, Category, ""),
    def(-1019016, "IFC Predefined Type", Str, false, Category, ""),
    def(-1019014, "Export to IFC As", Str, false, Category, ""),
    def(-1019012, "Export to IFC", Integer, false, Category, ""),
    def(-1019000, "IfcGUID", Str, false, Category, ""),
    def(-1013367, "Curtain version", Str, false, Class { class: "RoofBase", conditional: true }, "(RoofBase)"),
    def(-1013200, "Design Option", Str, true, Generic, ""),
    def(-1012101, "Phase Demolished", ElementId, false, Generic, ""),
    def(-1012100, "Phase Created", ElementId, false, Generic, ""),
    def(-1010106, "Comments", Str, false, Category, ""),
    def(-1001801, "Extrusion End", Double, false, Class { class: "Roof", conditional: true }, "(Roof)"),
    def(-1001800, "Extrusion Start", Double, false, Class { class: "Roof", conditional: true }, "(Roof)"),
    def(-1001713, "Related to Mass", Integer, false, Class { class: "RoofBase", conditional: true }, "(RoofBase)"),
    def(-1001652, "Level Offset", Double, false, Class { class: "Roof", conditional: true }, "(Roof)"),
    def(-1001651, "Reference Level", ElementId, false, Class { class: "Roof", conditional: true }, "(Roof)"),
    def(-1001601, "Thickness", Double, false, Class { class: "RoofBase", conditional: true }, "(RoofBase)"),
    def(-1001203, "Mark", Str, false, Category, ""),
];

pub const CEILING: &[Def] = &[
    def(-1152385, "Image", ElementId, false, Category, ""),
    def(-1019016, "IFC Predefined Type", Str, false, Category, ""),
    def(-1019014, "Export to IFC As", Str, false, Category, ""),
    def(-1019012, "Export to IFC", Integer, false, Category, ""),
    def(-1019000, "IfcGUID", Str, false, Category, ""),
    def(-1013200, "Design Option", Str, true, Generic, ""),
    def(-1012101, "Phase Demolished", ElementId, false, Generic, ""),
    def(-1012100, "Phase Created", ElementId, false, Generic, ""),
    def(-1010106, "Comments", Str, false, Category, ""),
    def(-1001952, "Level", ElementId, false, Class { class: "CeilingAndFloor", conditional: false }, "(CeilingAndFloor)"),
    def(-1001203, "Mark", Str, false, Category, ""),
];

pub const CEILING_TYPE: &[Def] = &[
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
    def(-1002501, "Assembly Description", Str, true, Category, ""),
    def(-1002500, "Assembly Code", Str, false, Category, ""),
    def(-1002110, "Coarse Scale Fill Color", Integer, false, Class { class: "HostObjAttr", conditional: true }, "(HostObjAttr)"),
    def(-1002106, "Coarse Scale Fill Pattern", ElementId, false, Class { class: "HostObjAttr", conditional: true }, "(HostObjAttr)"),
    def(-1001405, "Type Mark", Str, false, Category, ""),
    def(-1001205, "Cost", Double, false, Category, ""),
];

/// The invalid / empty value an id of a feature ASTERION does not have reads (ENGINE FACT).
fn empty_value(id: i32) -> Raw {
    match id {
        // no image
        -1152385 | -1152384 | -1002106 => Raw::Id(-1),
        // no IFC data, no keynote / manufacturer data
        -1019016 | -1019014 | -1140422 | -1019017 | -1019015 | -1010109 | -1010108 | -1010105 | -1010104 | -1010103 | -1001405 => Raw::Text(None),
        -1019013 | -1019012 | -1002110 => Raw::Int(0),
        -1001205 => Raw::Num(0.0),
        // no assembly classification: the empty string, not the absent one
        -1002500 | -1002501 => Raw::Text(Some(String::new())),
        // no mass elements
        -1001713 => Raw::Int(0),
        _ => Raw::Unknown,
    }
}

fn slab_dump(m: &BimModel, table: &'static [Def], s: &Slab, roof: bool) -> Vec<Dumped> {
    let st = crate::regen::slab_state(m, s).ok();
    let text = |key: &str| text_of(crate::instance_params::element_text(m, s.id).get(key));
    table
        .iter()
        .map(|d| {
            let (read_only, raw) = match d.id {
                -1010106 => (Some(d.registered_read_only), text("comments")),
                -1001203 => (Some(d.registered_read_only), text("mark")),
                -1001601 if roof => (None, st.as_ref().map_or(Raw::Unknown, |x| feet(x.thickness))),
                -1001952 => (Some(d.registered_read_only), Raw::Id(s.level.0 as i64)),
                -1012101 => {
                    let none = crate::phases::element_phases(m, s.id).map_or(true, |p| p.demolished.is_none());
                    (Some(d.registered_read_only), if none { Raw::Id(-1) } else { Raw::Unknown })
                }
                id => (Some(d.registered_read_only), empty_value(id)),
            };
            Dumped { def: d, read_only, raw }
        })
        .collect()
}

/// The dump of a roof.
pub fn roof_dump(m: &BimModel, s: &Slab) -> Vec<Dumped> {
    slab_dump(m, ROOF, s, true)
}

/// The dump of a ceiling.
pub fn ceiling_dump(m: &BimModel, s: &Slab) -> Vec<Dumped> {
    slab_dump(m, CEILING, s, false)
}

/// The dump of a ceiling type.
pub fn ceiling_type_dump(_m: &BimModel, _t: &SlabType) -> Vec<Dumped> {
    CEILING_TYPE.iter().map(|d| Dumped { def: d, read_only: Some(d.registered_read_only), raw: empty_value(d.id) }).collect()
}
