//! Schedules (OBSERVED, public API).
//!
//! The definition is measured: the category, the fields (name, heading, kind, whether it can be totalled, alignment,
//! format), filters (with the value read as a string, a double or an integer), sort / group fields with header, footer and
//! blank line, itemized, grand total and its title, headers. The table the reference product builds from it is measured in
//! round 30c as the text of every cell of the header and body sections ([`schedule_table`]):
//!
//! * the header section is one cell with the schedule's name (OBSERVED while the title is shown);
//! * the body starts with the column-heading row (gone when headers are hidden) and then ONE BLANK ROW that stays even
//!   when the headings are hidden (OBSERVED `headers-hidden`);
//! * itemized: one row per element in creation order; not itemized: one row per group, a field reading the common value or
//!   `<varies>`, Count reading the number of elements (OBSERVED `not-itemized`: `<varies>` x 6 and `5`);
//! * a sort / group field with a header puts the group value in the FIRST column of a row of its own, a footer row carries
//!   only the fields of display type Totals (blank when none), a blank line follows every group BUT THE LAST
//!   (OBSERVED: 12 rows for two groups with header + footer + blank line);
//! * the grand total row shows the title in the first column when its display is on, else nothing, and the sums of the
//!   Totals fields (OBSERVED `grand-total`, `a-totals-schedule-not-itemized`);
//! * filters drop elements before grouping (a length filter compares the reference product's internal feet; a text filter
//!   the text).
//! * the cells follow the model: a wall moved, a height edited, a wall deleted with its door, a type changed, a room that
//!   stops being enclosed reading `Not Enclosed` in Area, Volume and Perimeter (OBSERVED `a-wall-and-its-door-deleted`).
//!
//! Rows followed in this module (each cited where it decides): (a new schedule: name and default flags) (an added field's
//! column heading is the field name; whether a field can total) (filters are ANDed, in order; filter types 2..7 pass on the
//! comparison being = / != / > / >= / < / <= zero; a sort or filter is appended at the end) (the text of a body cell: an
//! itemized schedule always uses display type 0; a read error that is not a parameter error gives an empty text. UNKNOWN:
//! the natural alphanumeric order of text and the order of values that could not be read; the engine compares text by
//! code-unit order).
//!
//! A field the engine does not know is refused as "not schedulable"; the default cell formats (the numbers are those of the
//! template, measured in round 30c), the column a group header occupies (the first; only a first-column group was
//! measured), hidden fields leaving the table, the header section when the title is hidden, the scope of a schedule of all
//! categories (walls, doors, windows and slabs; OBSERVED 9 = 5 walls + 2 doors + 1 window + 1 floor, rooms excluded), where
//! the grand total count goes, the mark a door or window reads when none was set (doors numbered 1, 2 in placement order
//! match rounds 29b and 30c; the window read 19 / 6 in the reference product: UNKNOWN).

use crate::model::{BimModel, HostedKind};
use crate::spec::codes;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

pub const DEFAULT_GRAND_TOTAL_TITLE: &str = "Grand total";

/// The categories a schedule can list, by the name the API gives them.
pub const CATEGORIES: [&str; 6] = ["Walls", "Doors", "Windows", "Floors", "Rooms", "Roofs"];

/// `(field name, field type, can total)` of the fields measured for a category.
pub fn schedulable(category: &str) -> &'static [(&'static str, &'static str, bool)] {
    match category {
        "Walls" => &[
            ("Type", "Instance", false),
            ("Family", "Instance", false),
            ("Family and Type", "Instance", false),
            ("Length", "Instance", true),
            ("Area", "Instance", true),
            ("Volume", "Instance", true),
            ("Unconnected Height", "Instance", true),
            ("Base Offset", "Instance", true),
            ("Top Offset", "Instance", true),
            ("Comments", "Instance", false),
            ("Mark", "Instance", false),
            ("Width", "ElementType", true),
            ("Count", "Count", true),
        ],
        "Doors" => &[
            ("Mark", "Instance", false),
            ("Type", "Instance", false),
            ("Family", "Instance", false),
            ("Family and Type", "Instance", false),
            ("Width", "ElementType", true),
            ("Height", "ElementType", true),
            ("Level", "Instance", false),
            ("Comments", "Instance", false),
            ("Count", "Count", true),
        ],
        "Windows" => &[
            ("Mark", "Instance", false),
            ("Type", "Instance", false),
            ("Family", "Instance", false),
            ("Family and Type", "Instance", false),
            ("Width", "ElementType", true),
            ("Height", "ElementType", true),
            ("Sill Height", "Instance", true),
            ("Level", "Instance", false),
            ("Comments", "Instance", false),
            ("Count", "Count", true),
        ],
        "Rooms" => &[
            ("Name", "Instance", false),
            ("Number", "Instance", false),
            ("Area", "ViewBased", true),
            ("Volume", "Instance", true),
            ("Perimeter", "ViewBased", true),
            ("Level", "Instance", false),
            ("Comments", "Instance", false),
        ],
        "Floors" => &[
            ("Type", "Instance", false),
            ("Area", "Instance", true),
            ("Perimeter", "Instance", true),
            ("Level", "Instance", false),
            ("Comments", "Instance", false),
            ("Count", "Count", true),
        ],
        _ => &[("Count", "Count", true)],
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Field {
    /// The schedulable field it was made from.
    pub source: String,
    pub heading: String,
    pub field_type: String,
    pub can_total: bool,
    pub hidden: bool,
    /// `Standard` or `Totals`.
    pub display_type: String,
    pub alignment: String,
    /// `(unit name, accuracy)` when the format is not the default.
    pub format: Option<(String, f64)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FilterValue {
    /// Metres, square metres... stored in the reference product's internal unit (feet).
    Number(f64),
    Int(i32),
    Text(String),
    None,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SchedFilter {
    pub field: usize,
    pub kind: String,
    pub value: FilterValue,
}

/// One sort / group field. a field made from a field id alone is ascending with no header, no footer and no blank line (the
/// defaults of the replay), and turning the footer off also clears its title and count bits. Two fields are equal when all
/// three agree. The engine keeps no separate footer title / count flags: the footer is one flag.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SortField {
    pub field: usize,
    pub descending: bool,
    pub header: bool,
    pub footer: bool,
    pub blank_line: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Schedule {
    pub id: ElementId,
    pub name: String,
    /// `None`: all categories.
    pub category: Option<String>,
    pub fields: Vec<Field>,
    pub filters: Vec<SchedFilter>,
    pub sort: Vec<SortField>,
    pub itemized: bool,
    pub grand_total: bool,
    pub grand_total_title_on: bool,
    pub grand_total_count: bool,
    pub grand_total_title: String,
    pub show_headers: bool,
    pub show_title: bool,
}

fn bad(msg: &str) -> Failure {
    Failure::error(codes::SCHEDULE_INVALID, msg.to_owned())
}

fn not_found(id: ElementId) -> Failure {
    Failure::error(codes::NOT_FOUND, "No such schedule.").on(&[id])
}

/// `ViewSchedule.CreateSchedule(category)`: `None` for all categories. A category that is not valid for a regular schedule
/// is refused ("The category id cannot be used for a regular schedule."); the default name is the category's schedule name
/// ("<Category> Schedule", OBSERVED; the multi-category one reads "Multi-Category Schedule") made unique in the document.
/// The FIRST schedule of a category is named exactly the category's schedule name, the second "<name> 2", the third "<name>
/// 3"
/// - the counter starts at 2, and each candidate is tested as the formatted name "Schedule: <candidate>" against the titles
///   of ALL views and view templates, exact case (one shared name space; the engine tests the plain names of its schedules
///   and views: the title format of the other view kinds is not specified). A new schedule has no field for the categories
///   of the template, is itemized, shows its title and its headers and shows no grand total, no grand total title and no
///   grand total count. A name given by the caller is NOT made unique (OBSERVED: "Name must be unique.").
pub fn add_schedule_view(m: &mut BimModel, cx: &mut TxnContext<'_>, category: Option<&str>, name: Option<&str>) -> Result<ElementId, Failure> {
    if let Some(c) = category {
        if !CATEGORIES.contains(&c) {
            return Err(bad("The category id cannot be used for a regular schedule."));
        }
    }
    let taken = |m: &BimModel, n: &str| m.docs.schedules.values().any(|s| s.name == n) || m.views.values().any(|v| v.name == n) || m.docs.views.values().any(|v| v.name == n);
    let name = match name {
        Some(n) => {
            if taken(m, n) {
                return Err(bad("Name must be unique."));
            }
            n.to_owned()
        }
        None => {
            let base = format!("{} Schedule", category.unwrap_or("Multi-Category"));
            crate::view_names::unique_default_name(&base, |c| taken(m, c))
        }
    };
    let id = cx.new_id();
    m.docs.schedules.insert(
        id,
        Schedule {
            id,
            name,
            category: category.map(str::to_owned),
            fields: Vec::new(),
            filters: Vec::new(),
            sort: Vec::new(),
            itemized: true,
            grand_total: false,
            grand_total_title_on: false,
            grand_total_count: false,
            grand_total_title: DEFAULT_GRAND_TOTAL_TITLE.into(),
            show_headers: true,
            show_title: true,
        },
    );
    Ok(id)
}

fn sched_mut(m: &mut BimModel, id: ElementId) -> Result<&mut Schedule, Failure> {
    m.docs.schedules.get_mut(&id).ok_or_else(|| not_found(id))
}

/// The field a schedule has, by the schedulable name or the heading.
pub fn field_index(s: &Schedule, name: &str) -> Option<usize> {
    s.fields.iter().position(|f| f.source == name || f.heading == name)
}

/// `ScheduleDefinition.AddField`.
pub fn add_field(
    m: &mut BimModel,
    id: ElementId,
    name: &str,
    heading: Option<&str>,
    hidden: Option<bool>,
    totals: Option<bool>,
    alignment: Option<&str>,
) -> Result<(), Failure> {
    let s = sched_mut(m, id)?;
    let cat = s.category.clone().unwrap_or_default();
    let Some((_, ft, can)) = schedulable(&cat).iter().find(|f| f.0 == name).copied() else {
        return Err(bad("There is no such schedulable field."));
    };
    if s.fields.iter().any(|f| f.source == name) {
        return Err(bad("The field is already in the schedule."));
    }
    s.fields.push(Field {
        source: name.to_owned(),
        heading: heading.unwrap_or(name).to_owned(),
        field_type: ft.to_owned(),
        can_total: can,
        hidden: hidden.unwrap_or(false),
        display_type: if totals == Some(true) { "Totals".into() } else { "Standard".into() },
        alignment: alignment.unwrap_or("Left").to_owned(),
        format: None,
    });
    Ok(())
}

pub fn set_field_format(m: &mut BimModel, id: ElementId, field: &str, unit: &str, accuracy: Option<f64>) -> Result<(), Failure> {
    let s = sched_mut(m, id)?;
    let i = field_index(s, field).ok_or_else(|| bad("No such field in the schedule."))?;
    let acc = accuracy.or_else(|| s.fields[i].format.as_ref().map(|f| f.1)).unwrap_or(0.01);
    s.fields[i].format = Some((unit.to_owned(), acc));
    Ok(())
}

pub fn attach_filter(m: &mut BimModel, id: ElementId, field: &str, kind: &str, value: FilterValue) -> Result<(), Failure> {
    let s = sched_mut(m, id)?;
    let i = field_index(s, field).ok_or_else(|| bad("No such field in the schedule."))?;
    s.filters.push(SchedFilter { field: i, kind: kind.to_owned(), value });
    Ok(())
}

pub fn add_sort(m: &mut BimModel, id: ElementId, field: &str, descending: bool, header: bool, footer: bool, blank_line: bool) -> Result<(), Failure> {
    let s = sched_mut(m, id)?;
    let i = field_index(s, field).ok_or_else(|| bad(&format!("no field {field} in the schedule")))?;
    s.sort.push(SortField { field: i, descending, header, footer, blank_line });
    Ok(())
}

/// `ScheduleDefinition` properties. Setting the grand total title while its display is off is refused (OBSERVED).
#[derive(Default)]
pub struct DefinitionEdit {
    pub name: Option<String>,
    pub itemized: Option<bool>,
    pub grand_total: Option<bool>,
    pub show_grand_total_title: Option<bool>,
    pub grand_total_title: Option<String>,
    pub grand_total_count: Option<bool>,
    pub show_headers: Option<bool>,
    pub show_title: Option<bool>,
}

pub fn edit_definition(m: &mut BimModel, id: ElementId, e: DefinitionEdit) -> Result<(), Failure> {
    if let Some(n) = &e.name {
        if m.docs.schedules.values().any(|s| s.id != id && &s.name == n) || m.views.values().any(|v| &v.name == n) || m.docs.views.values().any(|v| &v.name == n) {
            return Err(bad("Name must be unique."));
        }
    }
    let s = sched_mut(m, id)?;
    if let Some(n) = e.name {
        s.name = n;
    }
    if let Some(v) = e.itemized {
        s.itemized = v;
    }
    if let Some(v) = e.grand_total {
        s.grand_total = v;
    }
    if let Some(v) = e.show_grand_total_title {
        s.grand_total_title_on = v;
    }
    if let Some(t) = e.grand_total_title {
        // Two refusals in order - the grand total row is not shown, then its title is not shown.
        if !s.grand_total {
            return Err(bad("Display of a grand total row is not enabled."));
        }
        if !s.grand_total_title_on {
            return Err(bad("Display of grand total title is not enabled."));
        }
        s.grand_total_title = t;
    }
    if let Some(v) = e.grand_total_count {
        s.grand_total_count = v;
    }
    if let Some(v) = e.show_headers {
        s.show_headers = v;
    }
    if let Some(v) = e.show_title {
        s.show_title = v;
    }
    Ok(())
}

// ----------------------------------------------------------------------------------------------- the table

/// The text the reference product writes in a cell whose values differ across a collapsed group (OBSERVED).
pub const VARIES: &str = "<varies>";
/// The text of a room quantity when the room bounds nothing (OBSERVED `a-wall-and-its-door-deleted`).
pub const NOT_ENCLOSED: &str = "Not Enclosed";
/// The family name of a wall type, as the reference product's user interface shows it.
pub const WALL_FAMILY: &str = "Basic Wall";

/// The value of one cell before it is formatted.
#[derive(Clone, Debug, PartialEq)]
pub enum CellValue {
    Empty,
    Text(String),
    /// Metres.
    Length(f64),
    /// Square metres.
    Area(f64),
    /// Cubic metres.
    Volume(f64),
    Count(usize),
    /// A quantity the reference product cannot compute (`NOT_ENCLOSED`).
    Unavailable(&'static str),
}

impl CellValue {
    fn number(&self) -> Option<f64> {
        match self {
            CellValue::Length(v) | CellValue::Area(v) | CellValue::Volume(v) => Some(*v),
            CellValue::Count(n) => Some(*n as f64),
            _ => None,
        }
    }

    /// The same quantity kind with another number (sums keep the kind of the first term).
    fn with_number(&self, v: f64) -> CellValue {
        match self {
            CellValue::Length(_) => CellValue::Length(v),
            CellValue::Area(_) => CellValue::Area(v),
            CellValue::Volume(_) => CellValue::Volume(v),
            CellValue::Count(_) => CellValue::Count(v.round() as usize),
            other => other.clone(),
        }
    }

    fn compare(&self, other: &CellValue) -> Ordering {
        match (self.number(), other.number()) {
            (Some(a), Some(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
            _ => self.text_key().cmp(&other.text_key()),
        }
    }

    fn text_key(&self) -> String {
        match self {
            CellValue::Text(t) => t.clone(),
            CellValue::Unavailable(t) => (*t).to_owned(),
            CellValue::Empty => String::new(),
            other => other.number().map(|n| format!("{n:.9}")).unwrap_or_default(),
        }
    }
}

/// The rounding methods: 0 nearest, 1 up, 2 down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundMethod {
    Nearest,
    Up,
    Down,
}

/// The value is unchanged when the accuracy is below 1e-12; otherwise q = |value| / accuracy becomes floor(q + 0.5 + 1e-9)
/// (nearest), ceil(q - 1e-9)
/// (up) or floor(q + 1e-9) (down), and the result is q x accuracy with the sign of the value restored. A value at exactly
/// half a quantum therefore rounds away from zero (1.875 at 0.01 -> 1.88, 46.5 at 1 -> 47).
pub fn round_to_accuracy(v: f64, accuracy: f64, method: RoundMethod) -> f64 {
    if accuracy < 1e-12 {
        return v;
    }
    let q = v.abs() / accuracy;
    let q = match method {
        RoundMethod::Nearest => (q + 0.5 + 1e-9).floor(),
        RoundMethod::Up => (q - 1e-9).ceil(),
        RoundMethod::Down => (q + 1e-9).floor(),
    };
    let r = q * accuracy;
    if v < 0.0 {
        -r
    } else {
        r
    }
}

fn round_to(v: f64, accuracy: f64) -> f64 {
    round_to_accuracy(v, accuracy, RoundMethod::Nearest)
}

/// The decimals a value rounded to `accuracy` is printed with: the smallest N in 0..=12 such that the accuracy rounded to
/// 10^-N reproduces it within half of 1e-12 (0.01 -> 2, 1 -> 0, 0.25 -> 2). Trailing-zero suppression (an option; the
/// display units of feet and inches) is not used by the schedule formats the engine has.
fn decimals_of(accuracy: f64) -> usize {
    (0..=12usize)
        .find(|n| {
            let p = 10f64.powi(*n as i32);
            ((accuracy * p).round() / p - accuracy).abs() < 0.5e-12
        })
        .unwrap_or(12)
}

fn write_number(v: f64, accuracy: f64, suffix: &str) -> String {
    let r = round_to(v, accuracy);
    let r = if r == 0.0 { 0.0 } else { r };
    format!("{:.*}{}", decimals_of(accuracy), r, suffix)
}

/// A unit the format of a field names, as a multiplier from the engine's SI value: `(factor, dimension)` where the
/// dimension is 1 for lengths, 2 for areas, 3 for volumes. `None`: the unit is not known.
fn unit_factor(unit: &str) -> Option<(f64, u8)> {
    const FT: f64 = 0.3048;
    Some(match unit {
        "Millimeters" => (1000.0, 1),
        "Centimeters" => (100.0, 1),
        "Decimeters" => (10.0, 1),
        "Meters" => (1.0, 1),
        "Feet" => (1.0 / FT, 1),
        "Inches" => (12.0 / FT, 1),
        "SquareMillimeters" => (1e6, 2),
        "SquareCentimeters" => (1e4, 2),
        "SquareMeters" => (1.0, 2),
        "SquareFeet" => (1.0 / (FT * FT), 2),
        "CubicMillimeters" => (1e9, 3),
        "CubicCentimeters" => (1e6, 3),
        "CubicMeters" => (1.0, 3),
        "Liters" => (1000.0, 3),
        "CubicFeet" => (1.0 / (FT * FT * FT), 3),
        _ => return None,
    })
}

/// The text of a cell. A field format `(unit, accuracy)` writes the value in that unit to that accuracy with no symbol
/// (OBSERVED: "16.60", "3.320").
pub fn cell_text(v: &CellValue, format: Option<&(String, f64)>) -> String {
    match v {
        CellValue::Empty => String::new(),
        CellValue::Text(t) => t.clone(),
        CellValue::Unavailable(t) => (*t).to_owned(),
        CellValue::Count(n) => n.to_string(),
        CellValue::Length(x) | CellValue::Area(x) | CellValue::Volume(x) => {
            let dim = match v {
                CellValue::Length(_) => 1,
                CellValue::Area(_) => 2,
                _ => 3,
            };
            if let Some((unit, acc)) = format {
                if let Some((factor, d)) = unit_factor(unit) {
                    if d == dim {
                        return write_number(x * factor, *acc, "");
                    }
                }
            }
            match dim {
                1 => write_number(x * 1000.0, 1.0, ""),
                2 => write_number(*x, 1.0, " m²"),
                _ => write_number(*x, 0.01, " m³"),
            }
        }
    }
}

/// The category of a model element a schedule can list, or `None`.
pub fn category_of_element(m: &BimModel, id: ElementId) -> Option<&'static str> {
    if m.walls.contains_key(&id) {
        Some("Walls")
    } else if let Some(i) = m.hosted_instances.get(&id) {
        Some(match m.hosted_types.get(&i.type_id).map(|t| t.kind) {
            Some(HostedKind::Window) => "Windows",
            _ => "Doors",
        })
    } else if let Some(f) = m.floors.get(&id) {
        Some(match f.role {
            crate::model::SlabRole::Floor => "Floors",
            crate::model::SlabRole::Ceiling => "Ceilings",
            crate::model::SlabRole::Roof => "Roofs",
        })
    } else if m.rooms.contains_key(&id) {
        Some("Rooms")
    } else {
        None
    }
}

/// The elements a schedule of `category` lists, in creation order (OBSERVED: the rows of an unsorted schedule follow the
/// element ids). `None`: walls, doors, windows and slabs, never rooms.
pub fn category_elements(m: &BimModel, category: Option<&str>) -> Vec<ElementId> {
    let mut ids: Vec<ElementId> = match category {
        Some("Rooms") => m.rooms.keys().copied().collect(),
        Some(c) => m
            .walls
            .keys()
            .chain(m.hosted_instances.keys())
            .chain(m.floors.keys())
            .copied()
            .filter(|id| category_of_element(m, *id) == Some(c))
            .collect(),
        None => m.walls.keys().chain(m.hosted_instances.keys()).chain(m.floors.keys()).copied().collect(),
    };
    ids.sort();
    ids
}

/// The Comments of an element: the documentation store (what the view filters read) or the element texts.
fn comments_of(m: &BimModel, id: ElementId) -> String {
    m.docs.comments.get(&id).cloned().unwrap_or_else(|| crate::instance_params::element_text(m, id).comments)
}

/// OBSERVED: the second door reads "2" with a window placed between.
pub fn instance_mark(m: &BimModel, id: ElementId) -> String {
    let Some(i) = m.hosted_instances.get(&id) else { return String::new() };
    if !i.mark.is_empty() {
        return i.mark.clone();
    }
    let kind = m.hosted_types.get(&i.type_id).map(|t| t.kind);
    let n = m.hosted_instances.values().filter(|o| o.id <= i.id && m.hosted_types.get(&o.type_id).map(|t| t.kind) == kind).count();
    n.to_string()
}

fn level_name(m: &BimModel, id: ElementId) -> CellValue {
    m.levels.get(&id).map_or(CellValue::Empty, |l| CellValue::Text(l.name.clone()))
}

/// The value of `field` (a schedulable name) for `id`. `Empty` for a field the element has not.
pub fn cell_value(m: &BimModel, id: ElementId, field: &str) -> CellValue {
    use CellValue as C;
    if field == "Count" {
        return C::Count(1);
    }
    if field == "Comments" {
        return C::Text(comments_of(m, id));
    }
    if let Some(w) = m.walls.get(&id) {
        let st = crate::regen::wall_state(m, w).ok();
        let q = |f: fn(&crate::regen::WallState) -> f64| st.as_ref().map(f);
        let type_name = m.wall_types.get(&w.type_id).map(|t| t.name.clone()).unwrap_or_default();
        return match field {
            "Type" => C::Text(type_name),
            "Family" => C::Text(WALL_FAMILY.into()),
            "Family and Type" => C::Text(format!("{WALL_FAMILY}: {type_name}")),
            "Length" => q(|s| s.length).map_or(C::Empty, C::Length),
            "Area" => q(|s| s.area).map_or(C::Empty, C::Area),
            "Volume" => q(|s| s.volume).map_or(C::Empty, C::Volume),
            "Unconnected Height" => q(|s| s.height).map_or(C::Empty, C::Length),
            "Base Offset" => C::Length(w.base_offset),
            "Top Offset" => C::Length(w.top_offset),
            "Width" => q(|s| s.width).map_or(C::Empty, C::Length),
            "Mark" => C::Text(crate::instance_params::element_text(m, id).mark),
            "Base Constraint" | "Level" => level_name(m, w.base_level),
            _ => C::Empty,
        };
    }
    if let Some(i) = m.hosted_instances.get(&id) {
        let t = m.hosted_types.get(&i.type_id);
        let type_name = t.map(|t| t.name.clone()).unwrap_or_default();
        return match field {
            "Mark" => C::Text(instance_mark(m, id)),
            "Type" => C::Text(type_name),
            // ENGINE CHOICE: the engine has no family document; the family reads as the type.
            "Family" => C::Text(type_name),
            "Family and Type" => C::Text(format!("{type_name}: {type_name}")),
            "Width" => C::Length(i.width_override.or(t.map(|t| t.width)).unwrap_or(0.0)),
            "Height" => C::Length(i.height_override.or(t.map(|t| t.height)).unwrap_or(0.0)),
            "Sill Height" => C::Length(i.sill_height),
            "Level" => level_name(m, i.level),
            _ => C::Empty,
        };
    }
    if let Some(r) = m.rooms.get(&id) {
        let st = crate::regen::room_state(m, r).ok();
        let q = |f: fn(&crate::regen::RoomState) -> f64, mk: fn(f64) -> C| match &st {
            Some(s) if s.enclosed => mk(f(s)),
            Some(_) => C::Unavailable(NOT_ENCLOSED),
            None => C::Empty,
        };
        return match field {
            "Name" => C::Text(r.name.clone()),
            "Number" => C::Text(r.number.clone()),
            "Area" => q(|s| s.area, C::Area),
            "Volume" => q(|s| s.volume, C::Volume),
            "Perimeter" => q(|s| s.perimeter, C::Length),
            "Level" => level_name(m, r.level),
            _ => C::Empty,
        };
    }
    if let Some(f) = m.floors.get(&id) {
        let st = crate::regen::slab_state(m, f).ok();
        let q = |g: fn(&crate::regen::SlabState) -> f64| st.as_ref().map(g);
        return match field {
            "Type" => C::Text(m.floor_types.get(&f.type_id).map(|t| t.name.clone()).unwrap_or_default()),
            "Area" => q(|s| s.area).map_or(C::Empty, C::Area),
            "Perimeter" => q(|s| s.perimeter).map_or(C::Empty, C::Length),
            "Volume" => q(|s| s.volume).map_or(C::Empty, C::Volume),
            "Level" => level_name(m, f.level),
            _ => C::Empty,
        };
    }
    C::Empty
}

/// Whether `v` passes a filter of `kind` with `fv` (the reference product's filter-type names; a number is in its internal
/// feet). The types 2..7 (Equal, NotEqual, GreaterThan, GreaterThanOrEqual, LessThan, LessThanOrEqual) pass on the
/// comparison `c` of the element's value with the filter value being `c = 0`, `c != 0`, `c > 0`, `c >= 0`, `c < 0`, `c <=
/// 0`.
fn passes(v: &CellValue, kind: &str, fv: &FilterValue, format: Option<&(String, f64)>) -> bool {
    let num = match fv {
        FilterValue::Number(n) => Some(match v {
            CellValue::Length(_) => n * 0.3048,
            CellValue::Area(_) => n * 0.3048 * 0.3048,
            CellValue::Volume(_) => n * 0.3048 * 0.3048 * 0.3048,
            _ => *n,
        }),
        FilterValue::Int(i) => Some(*i as f64),
        _ => None,
    };
    let text = match fv {
        FilterValue::Text(t) => t.clone(),
        FilterValue::Number(n) => n.to_string(),
        FilterValue::Int(i) => i.to_string(),
        FilterValue::None => String::new(),
    };
    let cell = cell_text(v, format);
    let has_value = !matches!(v, CellValue::Empty) && !cell.is_empty();
    match (kind, v.number(), num) {
        ("HasValue", _, _) => has_value,
        ("HasNoValue", _, _) => !has_value,
        ("HasParameter", _, _) => !matches!(v, CellValue::Empty),
        ("HasNotParameter", _, _) => matches!(v, CellValue::Empty),
        ("Equal", Some(a), Some(b)) => (a - b).abs() < 1e-9,
        ("NotEqual", Some(a), Some(b)) => (a - b).abs() >= 1e-9,
        ("GreaterThan", Some(a), Some(b)) => a > b + 1e-9,
        ("GreaterThanOrEqual", Some(a), Some(b)) => a >= b - 1e-9,
        ("LessThan", Some(a), Some(b)) => a < b - 1e-9,
        ("LessThanOrEqual", Some(a), Some(b)) => a <= b + 1e-9,
        ("Equal", _, _) => cell == text,
        ("NotEqual", _, _) => cell != text,
        ("GreaterThan", _, _) => cell > text,
        ("GreaterThanOrEqual", _, _) => cell >= text,
        ("LessThan", _, _) => cell < text,
        ("LessThanOrEqual", _, _) => cell <= text,
        ("Contains", _, _) => cell.contains(&text),
        ("NotContains", _, _) => !cell.contains(&text),
        ("BeginsWith", _, _) => cell.starts_with(&text),
        ("NotBeginsWith", _, _) => !cell.starts_with(&text),
        ("EndsWith", _, _) => cell.ends_with(&text),
        ("NotEndsWith", _, _) => !cell.ends_with(&text),
        _ => true,
    }
}

/// What a row of the body is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowKind {
    Heading,
    Blank,
    GroupHeader,
    Data,
    GroupFooter,
    GrandTotal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableRow {
    pub kind: RowKind,
    /// The elements the row stands for (a data row: one; a collapsed group row: its members; else none).
    pub elements: Vec<ElementId>,
    pub cells: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableColumn {
    /// The schedulable field the column shows.
    pub field: String,
    pub heading: String,
    pub alignment: String,
}

/// The table of a schedule: a header section (the title) and a body section.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScheduleTable {
    /// The header section: the name of the schedule when the title is shown.
    pub title: Option<String>,
    pub columns: Vec<TableColumn>,
    pub rows: Vec<TableRow>,
}

/// One element's values for every field of the schedule (hidden ones included: filters and sorts may use them).
struct Row {
    id: ElementId,
    values: Vec<CellValue>,
}

/// The sum of a Totals field over `rows`, or the common value / `<varies>` for a standard field, or the count.
fn collapsed(s: &Schedule, rows: &[&Row], col: usize, footer: bool) -> CellValue {
    let f = &s.fields[col];
    let totals = f.display_type == "Totals";
    if f.field_type == "Count" {
        return if !footer || totals { CellValue::Count(rows.len()) } else { CellValue::Empty };
    }
    if totals && rows.iter().all(|r| r.values[col].number().is_some()) {
        let sum: f64 = rows.iter().filter_map(|r| r.values[col].number()).sum();
        return rows.first().map_or(CellValue::Empty, |r| r.values[col].with_number(sum));
    }
    if footer {
        return CellValue::Empty;
    }
    let first = rows.first().map(|r| r.values[col].clone()).unwrap_or(CellValue::Empty);
    if rows.iter().all(|r| r.values[col].text_key() == first.text_key()) {
        first
    } else {
        CellValue::Text(VARIES.into())
    }
}

/// The table the reference product builds from a schedule's definition and the model (see the module documentation).
pub fn schedule_table(m: &BimModel, id: ElementId) -> Result<ScheduleTable, Failure> {
    let s = m.docs.schedules.get(&id).ok_or_else(|| not_found(id))?;
    let visible: Vec<usize> = (0..s.fields.len()).filter(|i| !s.fields[*i].hidden).collect();
    let columns: Vec<TableColumn> = visible
        .iter()
        .map(|&i| TableColumn { field: s.fields[i].source.clone(), heading: s.fields[i].heading.clone(), alignment: s.fields[i].alignment.clone() })
        .collect();
    // the rows: every element of the category, filtered, then sorted by the sort fields in order (stable: ties keep
    // the creation order)
    let mut rows: Vec<Row> = category_elements(m, s.category.as_deref())
        .into_iter()
        .map(|e| Row { id: e, values: s.fields.iter().map(|f| cell_value(m, e, &f.source)).collect() })
        .filter(|r| s.filters.iter().all(|f| s.fields.get(f.field).map_or(true, |fd| passes(&r.values[f.field], &f.kind, &f.value, fd.format.as_ref()))))
        .collect();
    for g in s.sort.iter().rev() {
        if g.field < s.fields.len() {
            rows.sort_by(|a, b| {
                let o = a.values[g.field].compare(&b.values[g.field]);
                if g.descending {
                    o.reverse()
                } else {
                    o
                }
            });
        }
    }
    let text = |col: usize, v: &CellValue| cell_text(v, s.fields[col].format.as_ref());
    let mut out: Vec<TableRow> = Vec::new();
    if s.show_headers {
        out.push(TableRow { kind: RowKind::Heading, elements: vec![], cells: columns.iter().map(|c| c.heading.clone()).collect() });
    }
    let blank = |kind: RowKind| TableRow { kind, elements: vec![], cells: vec![String::new(); visible.len()] };
    out.push(blank(RowKind::Blank));
    let refs: Vec<&Row> = rows.iter().collect();
    emit_groups(s, &visible, &refs, 0, &mut out, &text);
    if s.grand_total {
        let mut cells = vec![String::new(); visible.len()];
        for (ci, &col) in visible.iter().enumerate() {
            let f = &s.fields[col];
            if f.display_type == "Totals" && f.field_type != "Count" {
                cells[ci] = text(col, &collapsed(s, &refs, col, true));
            }
            if f.field_type == "Count" && (s.grand_total_count || f.display_type == "Totals") {
                cells[ci] = refs.len().to_string();
            }
        }
        if s.grand_total_title_on {
            if let Some(c) = cells.first_mut() {
                *c = s.grand_total_title.clone();
            }
        }
        out.push(TableRow { kind: RowKind::GrandTotal, elements: vec![], cells });
    }
    Ok(ScheduleTable { title: s.show_title.then(|| s.name.clone()), columns, rows: out })
}

/// The rows of the sort level `level` over `rows` (already sorted): groups by the value of that sort field, each with
/// its header, its members (recursively, or one collapsed row when not itemized) and its footer, a blank line between
/// consecutive groups.
fn emit_groups(s: &Schedule, visible: &[usize], rows: &[&Row], level: usize, out: &mut Vec<TableRow>, text: &dyn Fn(usize, &CellValue) -> String) {
    let row_of = |kind: RowKind, members: &[&Row], footer: bool| TableRow {
        kind,
        elements: members.iter().map(|r| r.id).collect(),
        cells: visible.iter().map(|&col| text(col, &collapsed(s, members, col, footer))).collect(),
    };
    if level >= s.sort.len() {
        if rows.is_empty() {
            return;
        }
        if s.itemized {
            for r in rows {
                out.push(TableRow { kind: RowKind::Data, elements: vec![r.id], cells: visible.iter().map(|&col| text(col, &r.values[col])).collect() });
            }
        } else {
            out.push(row_of(RowKind::Data, rows, false));
        }
        return;
    }
    let g = &s.sort[level];
    let key = |r: &Row| r.values.get(g.field).map(CellValue::text_key).unwrap_or_default();
    let mut groups: Vec<Vec<&Row>> = Vec::new();
    for r in rows {
        match groups.last_mut() {
            Some(last) if key(last[0]) == key(r) => last.push(r),
            _ => groups.push(vec![r]),
        }
    }
    let n = groups.len();
    for (gi, members) in groups.iter().enumerate() {
        if g.header {
            let mut cells = vec![String::new(); visible.len()];
            if let Some(c) = cells.first_mut() {
                *c = members[0].values.get(g.field).map(|v| text(g.field, v)).unwrap_or_default();
            }
            out.push(TableRow { kind: RowKind::GroupHeader, elements: vec![], cells });
        }
        emit_groups(s, visible, members, level + 1, out, text);
        if g.footer {
            out.push(row_of(RowKind::GroupFooter, members, true));
        }
        if g.blank_line && gi + 1 < n {
            out.push(TableRow { kind: RowKind::Blank, elements: vec![], cells: vec![String::new(); visible.len()] });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_are_written_in_the_template_units_and_rounded_half_up() {
        assert_eq!(cell_text(&CellValue::Length(6.0), None), "6000");
        assert_eq!(cell_text(&CellValue::Area(16.600000000000016), None), "17 m²");
        assert_eq!(cell_text(&CellValue::Area(9.375), None), "9 m²");
        assert_eq!(cell_text(&CellValue::Area(46.5), None), "47 m²");
        assert_eq!(cell_text(&CellValue::Volume(1.875), None), "1.88 m³");
        assert_eq!(cell_text(&CellValue::Volume(1.6), None), "1.60 m³");
        assert_eq!(cell_text(&CellValue::Volume(8.924999999999999), None), "8.93 m³");
        let mm = ("Millimeters".to_owned(), 1.0);
        let m2 = ("SquareMeters".to_owned(), 0.01);
        let m3 = ("CubicMeters".to_owned(), 0.001);
        assert_eq!(cell_text(&CellValue::Length(6.0), Some(&mm)), "6000");
        assert_eq!(cell_text(&CellValue::Area(16.6), Some(&m2)), "16.60");
        assert_eq!(cell_text(&CellValue::Volume(3.32), Some(&m3)), "3.320");
        assert_eq!(cell_text(&CellValue::Count(5), None), "5");
        assert_eq!(cell_text(&CellValue::Unavailable(NOT_ENCLOSED), None), "Not Enclosed");
    }

    /// Two wall types, five walls: the shapes of round 30c (itemized, collapsed, grouped with header / footer / blank
    /// line, grand total with a title, a filter, a totals schedule).
    #[test]
    fn the_table_follows_the_definition_as_measured_in_round_30c() {
        let mut d = crate::new_document();
        let (l0, t200, t300) = d
            .transact("setup", false, |m, cx| {
                let l0 = crate::ops::create_level(m, cx, "L0", 0.0)?;
                let t200 = crate::ops::create_wall_type(m, cx, "W200", crate::model::CompoundStructure { layers: vec![crate::model::Layer { width: 0.2, function: crate::model::LayerFunction::Structure }], exterior_shell_layers: 0, interior_shell_layers: 0 })?;
                let t300 = crate::ops::create_wall_type(m, cx, "W300", crate::model::CompoundStructure { layers: vec![crate::model::Layer { width: 0.3, function: crate::model::LayerFunction::Structure }], exterior_shell_layers: 0, interior_shell_layers: 0 })?;
                Ok((l0, t200, t300))
            })
            .value
            .unwrap();
        let mk = |d: &mut crate::BimDocument, t, a: [f64; 2], b: [f64; 2], h| {
            d.transact("w", false, |m, cx| {
                crate::ops::create_wall(
                    m,
                    cx,
                    crate::ops::NewWall { wall_type: t, level: l0, start: asterion_geom::Vec2::new(a[0], a[1]), end: asterion_geom::Vec2::new(b[0], b[1]), height: h, base_offset: 0.0, flip: false, structural: false },
                )
            })
            .value
            .unwrap()
        };
        let w1 = mk(&mut d, t200, [0.0, 0.0], [6.0, 0.0], 3.0);
        mk(&mut d, t200, [10.0, 0.0], [14.0, 0.0], 2.0);
        mk(&mut d, t300, [20.0, 0.0], [26.0, 0.0], 3.0);
        let s = d.transact("s", false, |m, cx| add_schedule_view(m, cx, Some("Walls"), Some("W"))).value.unwrap();
        for f in ["Type", "Length", "Area", "Comments", "Count"] {
            assert!(d.transact("f", false, |m, _| add_field(m, s, f, None, None, None, None)).committed());
        }
        d.transact("c", false, |m, cx| crate::instance_params::set_element_text(m, cx, w1, "comments", "north")).value.unwrap();
        let t = schedule_table(d.model(), s).unwrap();
        assert_eq!(t.title.as_deref(), Some("W"));
        let cells: Vec<Vec<String>> = t.rows.iter().map(|r| r.cells.clone()).collect();
        assert_eq!(cells[0], vec!["Type", "Length", "Area", "Comments", "Count"]);
        assert_eq!(cells[1], vec!["", "", "", "", ""]);
        assert_eq!(cells[2], vec!["W200", "6000", "18 m²", "north", "1"]);
        assert_eq!(cells[3], vec!["W200", "4000", "8 m²", "", "1"]);
        assert_eq!(cells[4], vec!["W300", "6000", "18 m²", "", "1"]);
        assert_eq!(cells.len(), 5);
        // not itemized: one row, <varies> where the values differ, the count
        d.transact("ni", false, |m, _| edit_definition(m, s, DefinitionEdit { itemized: Some(false), ..Default::default() })).value.unwrap();
        let t = schedule_table(d.model(), s).unwrap();
        assert_eq!(t.rows[2].cells, vec![VARIES, VARIES, VARIES, VARIES, "3"]);
        assert_eq!(t.rows.len(), 3);
        // grouped by type with header, footer and blank line; a grand total with its title
        d.transact("it", false, |m, _| edit_definition(m, s, DefinitionEdit { itemized: Some(true), ..Default::default() })).value.unwrap();
        d.transact("so", false, |m, _| add_sort(m, s, "Type", false, true, true, true)).value.unwrap();
        d.transact("gt", false, |m, _| edit_definition(m, s, DefinitionEdit { grand_total: Some(true), show_grand_total_title: Some(true), grand_total_title: Some("All".into()), ..Default::default() })).value.unwrap();
        let t = schedule_table(d.model(), s).unwrap();
        let kinds: Vec<RowKind> = t.rows.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![RowKind::Heading, RowKind::Blank, RowKind::GroupHeader, RowKind::Data, RowKind::Data, RowKind::GroupFooter, RowKind::Blank, RowKind::GroupHeader, RowKind::Data, RowKind::GroupFooter, RowKind::GrandTotal]
        );
        assert_eq!(t.rows[2].cells, vec!["W200", "", "", "", ""]);
        assert_eq!(t.rows[5].cells, vec!["", "", "", "", ""]);
        assert_eq!(t.rows[10].cells, vec!["All", "", "", "", ""]);
        // a filter on the comments keeps one wall; the last group has no blank line after it
        d.transact("fi", false, |m, _| attach_filter(m, s, "Comments", "Contains", FilterValue::Text("north".into()))).value.unwrap();
        let t = schedule_table(d.model(), s).unwrap();
        assert_eq!(t.rows.len(), 6);
        assert_eq!(t.rows[3].cells[0], "W200");
        // a totals schedule, not itemized, grouped with a footer: sums per group and the grand total without a title
        let s2 = d.transact("s2", false, |m, cx| add_schedule_view(m, cx, Some("Walls"), Some("T"))).value.unwrap();
        d.transact("f", false, |m, _| add_field(m, s2, "Type", Some("TYPE"), None, Some(false), None)).value.unwrap();
        d.transact("f", false, |m, _| add_field(m, s2, "Length", None, None, Some(true), None)).value.unwrap();
        d.transact("e", false, |m, _| edit_definition(m, s2, DefinitionEdit { itemized: Some(false), grand_total: Some(true), ..Default::default() })).value.unwrap();
        d.transact("so", false, |m, _| add_sort(m, s2, "Type", false, false, true, false)).value.unwrap();
        let t = schedule_table(d.model(), s2).unwrap();
        let cells: Vec<Vec<String>> = t.rows.iter().map(|r| r.cells.clone()).collect();
        assert_eq!(cells[0], vec!["TYPE", "Length"]);
        assert_eq!(cells[2], vec!["W200", "10000"]);
        assert_eq!(cells[3], vec!["", "10000"]);
        assert_eq!(cells[4], vec!["W300", "6000"]);
        assert_eq!(cells[5], vec!["", "6000"]);
        assert_eq!(cells[6], vec!["", "16000"]);
        // headers hidden: the heading row goes, the blank row stays
        d.transact("h", false, |m, _| edit_definition(m, s2, DefinitionEdit { show_headers: Some(false), ..Default::default() })).value.unwrap();
        let t = schedule_table(d.model(), s2).unwrap();
        assert_eq!(t.rows[0].kind, RowKind::Blank);
        assert_eq!(t.rows.len(), 6);
    }

    #[test]
    fn a_schedule_takes_fields_and_refuses_what_the_product_refuses() {
        let mut d = crate::new_document();
        let s = d.transact("s", false, |m, cx| add_schedule_view(m, cx, Some("Walls"), Some("W"))).value.unwrap();
        assert!(d.transact("f", false, |m, _| add_field(m, s, "Length", None, None, Some(true), None)).committed());
        assert!(d.model().docs.schedules[&s].fields[0].can_total);
        assert!(d.transact("nf", false, |m, _| add_field(m, s, "Nonsense", None, None, None, None)).value.is_none());
        assert!(d.transact("so", false, |m, _| add_sort(m, s, "Type", false, false, false, false)).value.is_none(), "a sort needs a field of the schedule");
        let e = DefinitionEdit { grand_total_title: Some("All".into()), ..DefinitionEdit::default() };
        assert!(d.transact("t", false, |m, _| edit_definition(m, s, e)).value.is_none());
        // The title needs the grand total row AND its title display
        let e = DefinitionEdit { show_grand_total_title: Some(true), grand_total_title: Some("All".into()), ..DefinitionEdit::default() };
        assert!(d.transact("t1", false, |m, _| edit_definition(m, s, e)).value.is_none(), "the row itself is not shown");
        let e = DefinitionEdit { grand_total: Some(true), show_grand_total_title: Some(true), grand_total_title: Some("All".into()), ..DefinitionEdit::default() };
        assert!(d.transact("t2", false, |m, _| edit_definition(m, s, e)).committed());
    }
}
