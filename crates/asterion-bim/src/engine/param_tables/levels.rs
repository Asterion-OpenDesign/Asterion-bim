
use super::{def, text_of, yes_no, Def, Dumped, Listed, Raw, Storage};
use crate::model::{BimModel, Level};
use Listed::{Category, Class, Generic};
use Storage::{ElementId, Integer, String as Str};

pub const LEVEL: &[Def] = &[
    def(-1019012, "Export to IFC", Integer, false, Category, "(category -2000240 entry, usage 101)"),
    def(-1019000, "IfcGUID", Str, false, Category, "(category -2000240 entry, usage 117)"),
    def(-1013200, "Design Option", Str, true, Generic, ""),
    def(-1012201, "Scope Box", Integer, false, Class { class: "DatumPlane", conditional: true }, "(DatumPlane)"),
    def(-1008000, "Name", Str, false, Class { class: "DatumPlane", conditional: true }, "(DatumPlane)"),
    def(-1007112, "Structural", Integer, false, Class { class: "Level", conditional: true }, "(Level)"),
    def(-1007111, "Building Story", Integer, false, Class { class: "Level", conditional: true }, "(Level)"),
    def(-1007110, "Story Above", ElementId, false, Class { class: "Level", conditional: true }, "(Level)"),
];

/// The dump of a level.
pub fn level_dump(_m: &BimModel, l: &Level) -> Vec<Dumped> {
    LEVEL
        .iter()
        .map(|d| {
            let raw = match d.id {
                -1008000 => text_of(l.name.clone()),
                -1007111 => yes_no(l.building_story),
                -1007110 => Raw::Id(l.story_above.map_or(-1, |s| s.0 as i64)),
                // ENGINE FACT: the engine's levels carry no structural flag, no scope box and no IFC export
                -1007112 => Raw::Int(0),
                -1012201 => Raw::Id(-1),
                -1019012 => Raw::Int(0),
                _ => Raw::Unknown,
            };
            Dumped { def: d, read_only: Some(d.registered_read_only), raw }
        })
        .collect()
}
