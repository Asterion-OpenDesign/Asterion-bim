//! Reference planes, model lines and arcs, model groups, wall sweeps and reveals, dimensions, tags and text notes.
//!
//! Every rule here is OBSERVED through the public API. The operations live in [`crate::datums`], [`crate::groups`],
//! [`crate::sweeps`] and [`crate::dims`]; this module is the stored state and the small geometry they share.

use asterion_core::ElementId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type V3 = [f64; 3];

pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn length(a: V3) -> f64 {
    dot(a, a).sqrt()
}
pub fn unit(a: V3) -> V3 {
    let l = length(a);
    if l > 0.0 {
        scale(a, 1.0 / l)
    } else {
        a
    }
}

/// A reference plane (`Document.Create.NewReferencePlane`).
///
/// OBSERVED: the direction is `(bubble - free)` normalised, the normal is `direction x cut vector`, and the plane's origin
/// is the FREE end. After a mirror the `Normal` property is the reflected vector while the plane's own normal is recomputed
/// from the reflected direction, so both are kept.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RefPlane {
    pub id: ElementId,
    pub name: String,
    pub bubble: V3,
    pub free: V3,
    pub cut: V3,
    pub normal: V3,
    /// The view the plane was drawn in (a plan view).
    #[serde(default)]
    pub view: Option<ElementId>,
}

impl RefPlane {
    pub fn direction(&self) -> V3 {
        unit(sub(self.bubble, self.free))
    }

    /// The normal of the plane as `GetPlane()` reports it.
    pub fn plane_normal(&self) -> V3 {
        unit(cross(self.direction(), self.cut))
    }
}

/// The geometry of a model line or arc, in three-dimensional points.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum CurveGeom {
    Line { start: V3, end: V3 },
    /// An arc by its two ends and a point on it.
    Arc { start: V3, end: V3, mid: V3 },
}

/// A model line or arc on a sketch plane (`Document.Create.NewModelCurve`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelCurve {
    pub id: ElementId,
    pub geom: CurveGeom,
    pub plane_origin: V3,
    pub plane_normal: V3,
    /// `ChangeToReferenceLine` (round 28b measures what it changes).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reference_line: bool,
}

/// A group type: the name, and the instance whose members are the template of a placement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroupType {
    pub id: ElementId,
    pub name: String,
    pub prototype: Option<ElementId>,
    /// What the type remembers once its last instance is ungrouped (OBSERVED: a type with no instance can still be placed).
    /// ENGINE CHOICE: the freed members stand in for the type's own definition, so the placement fails if they have been
    /// deleted since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen: Option<FrozenGroup>,
}

/// The definition a group type keeps when it has no instance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrozenGroup {
    pub origin: [f64; 2],
    pub bbox_rel: Option<([f64; 3], [f64; 3])>,
    pub members: Vec<ElementId>,
}

/// A group instance: where it sits (the centre of its members' box in plan) and its member elements.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub id: ElementId,
    pub type_id: ElementId,
    pub origin: [f64; 2],
    pub members: Vec<ElementId>,
    /// The box of the instance relative to its origin in plan (z absolute), fixed when the instance is made or placed:
    /// OBSERVED, a member deleted from an instance does not change the box the instance reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox_rel: Option<([f64; 3], [f64; 3])>,
}

impl Group {
    /// The box of the instance: `(min, max)`.
    pub fn bbox(&self) -> Option<([f64; 3], [f64; 3])> {
        self.bbox_rel.map(|(lo, hi)| ([lo[0] + self.origin[0], lo[1] + self.origin[1], lo[2]], [hi[0] + self.origin[0], hi[1] + self.origin[1], hi[2]]))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SweepKind {
    Sweep,
    Reveal,
}

/// A wall sweep or reveal type. The template has one of each; their profile is a 152.4 mm square (OBSERVED).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SweepType {
    pub id: ElementId,
    pub name: String,
    pub kind: SweepKind,
    pub profile: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WallSweep {
    pub id: ElementId,
    pub type_id: ElementId,
    pub wall: ElementId,
    pub kind: SweepKind,
    pub vertical: bool,
    /// Metres from the wall's base (horizontal) or from its start (vertical).
    pub distance: f64,
    pub from_top: bool,
    pub flipped: bool,
    /// Metres away from the wall's face.
    pub offset: f64,
    /// `true` for the wall's exterior side (as it was when the sweep was made).
    pub exterior_requested: bool,
    /// `+1` for the left of the location line, `-1` for the right, fixed at creation.
    pub side_sign: i8,
    pub sweep_intersects_wall: bool,
}

/// One reference of a dimension.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum DimRef {
    /// A wall face: `side` is +1 / -1 along the wall's orientation, 0 for the location line.
    Wall { wall: ElementId, side: i8 },
    Grid { grid: ElementId },
}

impl DimRef {
    pub fn element(&self) -> ElementId {
        match self {
            DimRef::Wall { wall, .. } => *wall,
            DimRef::Grid { grid } => *grid,
        }
    }
}

/// The overrides of one segment (or of the only segment) of a dimension.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DimText {
    pub value_override: Option<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub above: Option<String>,
    pub below: Option<String>,
}

/// A work plane: a plane in space - an origin and a normal, metres - that model lines and other sketched elements lie on.
/// One made from a datum plane (a level, a grid or a reference plane) FOLLOWS the datum: its plane is read from the datum's
/// present state ([`crate::datums::sketch_plane_geometry`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SketchPlane {
    pub id: ElementId,
    pub origin: V3,
    pub normal: V3,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datum: Option<ElementId>,
}

/// A linear dimension (`Document.Create.NewDimension(view, line, references)`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dimension {
    pub id: ElementId,
    pub view: ElementId,
    pub refs: Vec<DimRef>,
    /// Two points of the dimension line (plan).
    pub line: ([f64; 2], [f64; 2]),
    pub text: DimText,
    pub segment_text: BTreeMap<usize, DimText>,
    /// Where the text was moved to, if it was, as an offset from the dimension's origin (so the text follows the
    /// references: OBSERVED).
    pub text_position: Option<V3>,
    pub locked: bool,
    /// The equality flag of the segments: one toggle for the whole dimension.
    #[serde(default)]
    pub equal: bool,
}

/// A text note type: a name and a text size (metres of paper).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextNoteType {
    pub id: ElementId,
    pub name: String,
    pub size: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

impl HAlign {
    pub fn name(self) -> &'static str {
        match self {
            HAlign::Left => "Left",
            HAlign::Center => "Center",
            HAlign::Right => "Right",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextNote {
    pub id: ElementId,
    pub view: ElementId,
    pub type_id: ElementId,
    pub coord: V3,
    pub text: String,
    /// The width given at creation or set later, metres of paper; `None` is the width the text takes.
    pub width: Option<f64>,
    pub align: HAlign,
    /// `TextNote.AddLeader`: how many leaders the note has (OBSERVED: `LeaderCount` 1 after one).
    #[serde(default, skip_serializing_if = "is_zero_count")]
    pub leaders: u32,
}

fn is_zero_count(n: &u32) -> bool {
    *n == 0
}

/// A tag on a wall or a door.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tag {
    pub id: ElementId,
    pub view: ElementId,
    pub host: ElementId,
    pub head: V3,
    pub leader: bool,
    pub vertical: bool,
    /// False for a tag made with a leader: the reference product moves the head (OBSERVED: not derived).
    #[serde(default = "yes")]
    pub head_known: bool,
}

fn yes() -> bool {
    true
}

/// All the state of this slice (one field of `BimModel`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AnnotModel {
    #[serde(default)]
    pub ref_planes: BTreeMap<ElementId, RefPlane>,
    #[serde(default)]
    pub curves: BTreeMap<ElementId, ModelCurve>,
    #[serde(default)]
    pub groups: BTreeMap<ElementId, Group>,
    #[serde(default)]
    pub group_types: BTreeMap<ElementId, GroupType>,
    #[serde(default)]
    pub sweeps: BTreeMap<ElementId, WallSweep>,
    #[serde(default)]
    pub sweep_types: BTreeMap<ElementId, SweepType>,
    #[serde(default)]
    pub dims: BTreeMap<ElementId, Dimension>,
    #[serde(default)]
    pub notes: BTreeMap<ElementId, TextNote>,
    #[serde(default)]
    pub note_types: BTreeMap<ElementId, TextNoteType>,
    #[serde(default)]
    pub tags: BTreeMap<ElementId, Tag>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sketch_planes: BTreeMap<ElementId, SketchPlane>,
}

impl AnnotModel {
    pub fn is_empty(&self) -> bool {
        *self == AnnotModel::default()
    }

    pub fn max_id(&self) -> Option<ElementId> {
        [
            self.ref_planes.keys().next_back(),
            self.curves.keys().next_back(),
            self.groups.keys().next_back(),
            self.group_types.keys().next_back(),
            self.sweeps.keys().next_back(),
            self.sweep_types.keys().next_back(),
            self.dims.keys().next_back(),
            self.notes.keys().next_back(),
            self.note_types.keys().next_back(),
            self.tags.keys().next_back(),
            self.sketch_planes.keys().next_back(),
        ]
        .into_iter()
        .flatten()
        .max()
        .copied()
    }

    pub fn contains(&self, id: ElementId) -> bool {
        self.ref_planes.contains_key(&id)
            || self.curves.contains_key(&id)
            || self.groups.contains_key(&id)
            || self.group_types.contains_key(&id)
            || self.sweeps.contains_key(&id)
            || self.sweep_types.contains_key(&id)
            || self.dims.contains_key(&id)
            || self.notes.contains_key(&id)
            || self.note_types.contains_key(&id)
            || self.tags.contains_key(&id)
            || self.sketch_planes.contains_key(&id)
    }

    /// The group instance an element is a member of.
    pub fn group_of(&self, member: ElementId) -> Option<ElementId> {
        self.groups.values().find(|g| g.members.contains(&member)).map(|g| g.id)
    }
}
