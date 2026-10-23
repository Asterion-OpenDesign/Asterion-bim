
use super::{cubic_feet, def, feet, square_feet, text_of, Def, Dumped, Listed, Raw, Storage};
use crate::model::{BimModel, Room};
use Listed::{Category, Class, Generic};
use Storage::{Double, ElementId, Integer, String as Str};

pub const ROOM: &[Def] = &[
    def(-1154671, "Air Changes per Hour", Double, false, Class { class: "RoomElem", conditional: true }, "(RoomElem)"),
    def(-1154668, "Outdoor Air per Area", Double, false, Class { class: "RoomElem", conditional: true }, "(RoomElem)"),
    def(-1154665, "Outdoor Air per Person", Double, false, Class { class: "RoomElem", conditional: true }, "(RoomElem)"),
    def(-1154662, "Outdoor Air Information", Str, false, Class { class: "RoomElem", conditional: true }, "(RoomElem)"),
    def(-1152385, "Image", ElementId, false, Category, ""),
    def(-1114384, "Zone", Str, false, Class { class: "RoomElem", conditional: true }, "(RoomElem)"),
    def(-1114284, "Electrical Loads", Storage::None, false, Class { class: "RoomElem", conditional: true }, "(RoomElem)"),
    def(-1114283, "People", Storage::None, false, Class { class: "RoomElem", conditional: true }, "(RoomElem)"),
    def(-1114221, "Design HVAC Load per area", Double, false, Class { class: "RoomElem", conditional: true }, "(RoomElem)"),
    def(-1114176, "Specified Supply Airflow", Double, false, Class { class: "RoomElem", conditional: true }, "(RoomElem)"),
    def(-1114173, "Occupancy Unit", Integer, false, Class { class: "RoomElem", conditional: true }, "(RoomElem)"),
    def(-1019016, "IFC Predefined Type", Str, false, Category, ""),
    def(-1019014, "Export to IFC As", Str, false, Category, ""),
    def(-1019012, "Export to IFC", Integer, false, Category, ""),
    def(-1019000, "IfcGUID", Str, false, Category, ""),
    def(-1013200, "Design Option", Str, true, Generic, ""),
    def(-1012701, "Area Type", Integer, false, Class { class: "RoomElem", conditional: true }, "(RoomElem)"),
    def(-1012113, "Phase", ElementId, false, Category, "(category -2000160 entry, usage 1)"),
    def(-1010106, "Comments", Str, false, Category, ""),
    def(-1006925, "Base Offset", Double, false, Category, "(category -2000160 entry, usage 69)"),
    def(-1006924, "Limit Offset", Double, false, Category, "(category -2000160 entry, usage 69)"),
    def(-1006922, "Upper Limit", ElementId, false, Category, "(category -2000160 entry, usage 69)"),
    def(-1006921, "Volume", Double, false, Category, "(category -2000160 entry, usage 117)"),
    def(-1006920, "Unbounded Height", Double, false, Category, "(category -2000160 entry, usage 85)"),
    def(-1006917, "Perimeter", Double, false, Category, "(category -2000160 entry, usage 113)"),
    def(-1006916, "Level", ElementId, false, Category, "(category -2000160 entry, usage 101)"),
    def(-1006909, "Occupancy", Str, false, Category, "(category -2000160 entry, usage 125)"),
    def(-1006907, "Department", Str, false, Category, "(category -2000160 entry, usage 125)"),
    def(-1006906, "Base Finish", Str, false, Category, "(category -2000160 entry, usage 125)"),
    def(-1006905, "Ceiling Finish", Str, false, Category, "(category -2000160 entry, usage 125)"),
    def(-1006904, "Wall Finish", Str, false, Category, "(category -2000160 entry, usage 125)"),
    def(-1006903, "Floor Finish", Str, false, Category, "(category -2000160 entry, usage 125)"),
    def(-1006902, "Area", Double, false, Category, "(category -2000160 entry, usage 113)"),
    def(-1006901, "Number", Str, false, Category, "(category -2000160 entry, usage 117)"),
    def(-1006900, "Name", Str, false, Category, "(category -2000160 entry, usage 125)"),
    def(-1002053, "Workset", Integer, false, Class { class: "RoomElem", conditional: true }, "(RoomElem)"),
    def(-1001203, "Mark", Str, false, Category, ""),
];

/// The dump of a room. the Area, Perimeter, Volume, Level, Number and Name of an architectural room are read-only only
/// through layers the rule did not specified: unknown for all rooms.
pub fn room_dump(m: &BimModel, r: &Room) -> Vec<Dumped> {
    let st = crate::regen::room_state(m, r).ok();
    let placed = crate::rooms_ext::is_placed(m, r.id);
    let has_level = crate::rooms_ext::has_level(m, r.id);
    let flagged = !placed || st.as_ref().map_or(true, |s| !s.enclosed);
    let text = |key: &str| text_of(crate::instance_params::element_text(m, r.id).get(key));
    ROOM.iter()
        .map(|d| {
            // No rule of the rule reaches these ids (layers it did not specified decide)
            let layered = matches!(d.id, -1006902 | -1006917 | -1006921 | -1006916 | -1006901 | -1006900);
            let read_only = if layered || flagged { None } else { Some(d.registered_read_only) };
            let raw = match d.id {
                -1006900 => Raw::Text(Some(r.name.clone())),
                -1006901 => Raw::Text(Some(r.number.clone())),
                -1006902 => st.as_ref().map_or(Raw::Unknown, |s| square_feet(s.area)),
                -1006917 => st.as_ref().map_or(Raw::Unknown, |s| feet(s.perimeter)),
                -1006921 => st.as_ref().map_or(Raw::Unknown, |s| cubic_feet(s.volume)),
                -1006920 => st.as_ref().map_or(Raw::Unknown, |s| feet(s.unbounded_height)),
                -1006924 => feet(r.limit_offset),
                -1006925 => feet(r.base_offset),
                // OBSERVED (the `upper_limit` of rooms/<r>, compared): the parameter holds the room's own level when none
                // was set; an unplaced room reads the invalid id (not specified, ROOM_UPPER_LEVEL -1006922)
                -1006922 if has_level && placed => Raw::Id(r.upper_limit.map_or(r.level.0 as i64, |l| l.0 as i64)),
                -1006922 if has_level => Raw::Id(-1),
                -1006916 if has_level => Raw::Id(r.level.0 as i64),
                -1006909 => text("occupancy"),
                -1006907 => text("department"),
                -1006906 => text("base_finish"),
                -1006905 => text("ceiling_finish"),
                -1006904 => text("wall_finish"),
                -1006903 => text("floor_finish"),
                -1010106 => text("comments"),
                -1001203 => text("mark"),
                // ENGINE FACT: no image, no IFC data
                -1152385 => Raw::Id(-1),
                -1019016 | -1019014 => Raw::Text(None),
                -1019012 => Raw::Int(0),
                _ => Raw::Unknown,
            };
            Dumped { def: d, read_only, raw }
        })
        .collect()
}
