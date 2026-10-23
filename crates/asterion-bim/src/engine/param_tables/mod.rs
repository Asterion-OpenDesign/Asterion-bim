//! Per-class parameter tables: which parameters an element of a class LISTS, the storage kind of each, whether it is
//! read-only, and the value the engine can answer for it.
//!
//! The table of a class is built ONLY from specification rules, and every entry says which:
//! * `id` and `name`: (the English text of a built-in parameter id; the same text belongs to several ids, so the id comes
//!   from the listing below, never from the text).
//! * The generic ids, and the category / common-group entries whose usage mask has bit and whose kind is the element's; a
//!   row that names the id, e.g. For the wall height helper. An id that the measured dumps show and that none of these
//!   proves is NOT in the table.
//! * the element-level read-only rule: (walls, wall types, floors, ceilings, roofs, openings) (views, viewports and the
//!   base Element rule) (levels, grids, sections) (rooms) (the layers around the class rule).
//!
//! The VALUE (`raw`) is answered only where the engine already holds the value and compares it elsewhere (a wall's offsets,
//! heights, location line and structural flag; a room's name and number), or where the engine has no such feature at all
//! and the parameter reads the invalid / empty value by construction (ENGINE FACT, named at the entry). Everything else is
//! [`Raw::Unknown`]: the comparison counts it under a named rule instead of guessing.

use serde::Serialize;

pub mod columns;
pub mod curtains;
pub mod levels;
pub mod rooms;
pub mod slabs;
pub mod stairs;
pub mod views;
pub mod walls;

pub use columns::{column_dump, column_type_dump, COLUMN, COLUMN_TYPE};
pub use curtains::{mullion_type_dump, MULLION_TYPE};
pub use levels::{level_dump, LEVEL};
pub use rooms::{room_dump, ROOM};
pub use slabs::{ceiling_dump, ceiling_type_dump, roof_dump, CEILING, CEILING_TYPE, ROOF};
pub use stairs::{railing_type_dump, stairs_dump, stairs_type_dump, RAILING_TYPE, STAIRS, STAIRS_TYPE};
pub use views::{view_dump, VIEW_PLAN, VIEW_SECTION};
pub use walls::{wall_dump, wall_type_dump, WALL, WALL_TYPE};

/// The API storage kind of a parameter (`StorageType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Storage {
    None,
    Integer,
    Double,
    String,
    ElementId,
}

impl Storage {
    /// The name the API gives the kind.
    pub fn api_name(self) -> &'static str {
        match self {
            Storage::None => "None",
            Storage::Integer => "Integer",
            Storage::Double => "Double",
            Storage::String => "String",
            Storage::ElementId => "ElementId",
        }
    }
}

/// Why the reference product lists an id for a class (the proof carried by each entry).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listed {
    /// The class named is the operation that adds it. `conditional`: added under a test inside the operation (the test is
    /// not specified: the engine lists it for an ordinary element).
    Class { class: &'static str, conditional: bool },
    /// The generic ids inserts under tests of its own.
    Generic,
    /// A category entry or a common-group entry with usage bit and the element's kind.
    Category,
    /// The wall height helper: the level / offset ids of the two ends and the user height.
    HeightHelper,
}

/// One entry of a class table.
#[derive(Clone, Copy, Debug)]
pub struct Def {
    pub id: i32,
    pub name: &'static str,
    pub storage: Storage,
    pub registered_read_only: bool,
    pub listed: Listed,
    /// The specification rules behind the entry (beyond and the listing row).
    pub rows: &'static str,
}

pub(crate) const fn def(id: i32, name: &'static str, storage: Storage, registered_read_only: bool, listed: Listed, rows: &'static str) -> Def {
    Def { id, name, storage, registered_read_only, listed, rows }
}

/// The value an entry answers.
#[derive(Clone, Debug, PartialEq)]
pub enum Raw {
    Unknown,
    /// An Integer parameter (a Yes / No flag, an enumeration code).
    Int(i64),
    /// A Double parameter, in the reference product's internal units (feet, square feet, cubic feet, radians).
    Num(f64),
    /// An ElementId parameter: the raw id (-1 is the invalid id).
    Id(i64),
    /// A String parameter (`None`: the parameter holds no string).
    Text(Option<String>),
}

/// One parameter of the dump of an element.
#[derive(Clone, Debug)]
pub struct Dumped {
    pub def: &'static Def,
    pub read_only: Option<bool>,
    pub raw: Raw,
}

/// Metres to the reference product's internal length unit (feet): the stored value of a Double length parameter is in feet
/// (
/// (4) `base_offset = -1001108 x 0.3048 m`).
pub use crate::spec::FEET_PER_METRE;

pub(crate) fn feet(m: f64) -> Raw {
    Raw::Num(m * FEET_PER_METRE)
}

pub(crate) fn square_feet(m2: f64) -> Raw {
    Raw::Num(m2 * FEET_PER_METRE * FEET_PER_METRE)
}

pub(crate) fn cubic_feet(m3: f64) -> Raw {
    Raw::Num(m3 * FEET_PER_METRE * FEET_PER_METRE * FEET_PER_METRE)
}

pub(crate) fn yes_no(b: bool) -> Raw {
    Raw::Int(i64::from(b))
}

/// A text parameter: an unset text reads as no string.
pub(crate) fn text_of(s: String) -> Raw {
    Raw::Text((!s.is_empty()).then_some(s))
}
