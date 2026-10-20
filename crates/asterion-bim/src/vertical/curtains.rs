//! Curtain walls: the grid, the panels and the mullion counts.
//!
//! What the observations do not say is a named ENGINE CHOICE (question ids below). Rules specified since, each cited where
//! the code follows it: Spec CW-01 (a grid line stores a direction flag and ONE surface parameter; the iso-line it draws)
//! (the layout values, the justification values, the fixed-number range, the spacing a layout switch gives), Spec CW-03
//! (the panels are the cells of the lines), Spec CW-05 (removed segments: ranges between crossing lines, normalised, split
//! by a new crossing line, the empty-line warning), Spec CW-06 (the corner-edge predicate), plus (the stored fields of the
//! lines, the skipped segments and the type: the import side).
//!
//! * **A curtain wall** is a wall of a curtain wall type: its width is 25 mm (the glass), its area
//!   `length x height`, and its orientation is the NEGATED left normal of its location line (the
//!   one observed wall read `(0, -1, 0)` for a line along +x, where a basic wall reads `(0, 1, 0)`).
//! * **The grid** has two kinds of line. A **U line** is HORIZONTAL (the API's true: a position `(x, y, 1.5 m)` makes a
//!   line at 1.5 m above the base along the whole wall), a **V line** is VERTICAL (a position `(3 m, y, z)` makes a line at
//!   3 m along the wall). The position must lie inside a cell: a point on the boundary of a panel is refused (an empty
//!   `ArgumentException`). A line at the position of an existing one of the same kind is ignored (no error, no new line).
//! * **A default grid** has no lines and one panel (glass, 25 mm thick, 24.5 mm in front of the wall's
//!   location line along its left normal).
//! * **Layouts** (the type's `Layout` per direction: 0 None, 1 Fixed Distance, 2 Fixed Number, 3
//!   Maximum Spacing; "VERT" makes V lines, "HORIZ" U lines):
//!   Fixed Number `n` makes `n` lines at `i x total / (n + 1)`; Fixed Distance `d` makes lines at
//!   `d, 2d, ...` from the start while inside the wall, the remainder at the end (justification
//!   Beginning, the default), or the remainder split to both ends (justification Center, the
//!   instance parameter's code 2); Maximum Spacing `d` makes `ceil(total / d) - 1` equal divisions
//!   (6.5 m at 2.0: lines at 1.625, 3.25, 4.875). A new layout's number of lines is 4 until set.
//!   The instance's Number is read-only unless the layout is Fixed Number; the type's Spacing is
//!   read-only while the layout is None or Fixed Number.
//! * **Panels** are the cells of the lines in both directions; a panel's area is its width x height.
//! * **Mullions** are counted per line segment: a segment is the part of a line between two crossing
//!   lines or a wall edge. The automatic ones: the type's Interior type on every automatic line of a
//!   direction (one per segment), and its Border types on the wall's two edges of that direction
//!   (41 = 16 + 15 + 10 observed for 4 + 3 lines, a 5 x 4 cell grid and two vertical borders).
//!   `AddMullions` puts a mullion on every segment of one line.
//!
//! OBSERVED: a straight mullion's `Area` is half its surface, `(perimeter x length + 2 x section) / 2`, its box is its
//! width by twice its half thickness by its segment, and a circular one's box is 0.3319 across for a radius of 1/3 m but
//! exactly 0.05 for 0.05, the other justification codes (only 2 = Center was seen to differ), the sizes of the corner
//! mullion types, the layouts other than Fixed Number with "Adjust for Mullion Size" and the default Spacing of a new type
//! (ENGINE CHOICE 1.0 m).
//!
//! * **A removed segment** (`RemoveSegment`, OBSERVED): the two panels on either side of the segment become ONE panel (area
//!   the sum of the cells, box the union of the cells' boxes) and the other panel stays in the list EMPTY (area 0, no box):
//!   the panel count does not change. The line reports one existing and one skipped segment. The next removal counts the
//!   EXISTING segments only. Mullions on a removed segment are not placed (ENGINE CHOICE: no measurement has mullions
//!   there).
//! * The template's five straight types, see [`template_size`]: a mullion has a width on each side of its line (a circular
//!   one its radius) and a half thickness across the wall. It CUTS the glass of the panels beside it, by its width on that
//!   side, only when its half thickness reaches the glass (more than 24.5 mm: the 30 mm square mullion leaves the glass
//!   whole, the others cut it); a border mullion lies inside the wall edge and cuts by its full width. With "Adjust for
//!   Mullion Size" on (the default) the automatic lines of a Fixed Number layout move so that the CLEAR width of every
//!   panel is equal: `c = (total - n w_interior - w_border1 - w_border2) / (n + 1)`, line `k` at `w_border1 + k c + (k - 1)
//!   w_interior + w_interior / 2`. Lines added through the API never move. The other layouts keep their nominal lines and a
//!   mullion type the engine has no size for has no effect.
//! * **Corner mullion types** (L, V, Quad, Trapezoid in the template) are refused by `AddMullions` on a plain grid line
//!   (OBSERVED: "Invalid curtain grid line or invalid mullion type argument").

use crate::model::BimModel;
use crate::spec::codes;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use asterion_geom::Vec2;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// OBSERVED: the glass of a curtain wall is 25 mm thick, starting 24.5 mm in front of the location line.
pub const PANEL_OFFSET_M: f64 = 0.0245;
pub const PANEL_THICKNESS_M: f64 = 0.025;
/// OBSERVED: the number of lines of a layout switched to Fixed Number and not set.
pub const DEFAULT_NUMBER: u32 = 4;
/// ENGINE CHOICE: the stored Spacing of a new curtain wall type is not specified.
pub const DEFAULT_SPACING_M: f64 = 0.0;

/// The type's `Layout`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GridLayout {
    #[default]
    None,
    FixedDistance,
    FixedNumber,
    MaxSpacing,
    /// (VALUES): layout 5. The lines of this layout are an ENGINE CHOICE, see [`layout_lines`].
    MinSpacing,
}

impl GridLayout {
    /// OBSERVED integer codes of the built-in parameter (read back as "Fixed Number" for 2 ...), and (VALUES): the stored
    /// layout is 0 None, 1 Fixed Distance, 2 Fixed Number, 3 Maximum Spacing, 5 Minimum Spacing (a value 4 is handled as a
    /// fixed distance with the padding added: not named by the enum operation, so not offered).
    pub fn from_code(c: i64) -> Option<GridLayout> {
        match c {
            0 => Some(GridLayout::None),
            1 => Some(GridLayout::FixedDistance),
            2 => Some(GridLayout::FixedNumber),
            3 => Some(GridLayout::MaxSpacing),
            5 => Some(GridLayout::MinSpacing),
            _ => None,
        }
    }

    /// The code [`GridLayout::from_code`] reads.
    pub fn code(self) -> i64 {
        match self {
            GridLayout::None => 0,
            GridLayout::FixedDistance => 1,
            GridLayout::FixedNumber => 2,
            GridLayout::MaxSpacing => 3,
            GridLayout::MinSpacing => 5,
        }
    }

    /// Whether the layout takes its lines from a spacing (Fixed Distance, Maximum and Minimum Spacing).
    pub fn uses_spacing(self) -> bool {
        matches!(self, GridLayout::FixedDistance | GridLayout::MaxSpacing | GridLayout::MinSpacing)
    }
}

/// The justification of a spacing rule instance. With the flip flag Beginning and End exchange. Fixed Distance walks from the
/// justified end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Justify {
    Beginning,
    Center,
    End,
}

impl Justify {
    /// The instance parameter `Justification` as the API shows it. OBSERVED: 2 is Center and the default reads as
    /// Beginning.
    pub fn from_api_code(c: i64) -> Justify {
        if c == 2 {
            Justify::Center
        } else {
            Justify::Beginning
        }
    }

    /// With the flip flag Beginning and End exchange.
    pub fn flipped(self) -> Justify {
        match self {
            Justify::Beginning => Justify::End,
            Justify::End => Justify::Beginning,
            Justify::Center => Justify::Center,
        }
    }
}

/// A fixed number is valid from 1 to 200.
pub const FIXED_NUMBER_RANGE: std::ops::RangeInclusive<i64> = 1..=200;

/// Switching a type to Fixed Distance, Maximum Spacing or Minimum Spacing sets the spacing to 5.0 internal length units
/// (feet) when it is zero.
pub const LAYOUT_SWITCH_SPACING_M: f64 = 5.0 * 0.3048;

/// One direction of a curtain wall type's grid settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DirectionSettings {
    pub layout: GridLayout,
    /// The type's `Spacing`, metres.
    pub spacing: f64,
    #[serde(default)]
    pub backup_layout: GridLayout,
    /// The Interior mullion type of the automatic lines of this direction.
    pub interior: Option<ElementId>,
    /// The two Border mullion types (the wall's two edges of this direction).
    pub border1: Option<ElementId>,
    pub border2: Option<ElementId>,
    /// "Adjust for Mullion Size" of this direction.
    #[serde(default = "yes")]
    pub adjust: bool,
}

fn yes() -> bool {
    true
}

impl Default for DirectionSettings {
    fn default() -> Self {
        DirectionSettings { layout: GridLayout::None, spacing: DEFAULT_SPACING_M, backup_layout: GridLayout::None, interior: None, border1: None, border2: None, adjust: true }
    }
}

/// The curtain part of a wall type (keyed by the wall type's id in `BimModel::curtain_types`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CurtainType {
    /// V lines (vertical, along the wall): the "VERT" parameters.
    pub vertical: DirectionSettings,
    /// U lines (horizontal): the "HORIZ" parameters.
    pub horizontal: DirectionSettings,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MullionType {
    pub id: ElementId,
    pub name: String,
    /// A corner mullion family (L, V, Quad, Trapezoid): it cannot go on a plain grid line.
    #[serde(default)]
    pub corner: bool,
}

/// One segment taken out of a grid line by `RemoveSegment`: the line (kind and position) and the middle of
/// the segment along it, so the segment is found again whatever lines are added later.
///
/// Spec CW-05: the reference product stores the removed stretch as the pair of crossing lines that bound it (an invalid id
/// stands for the face border), ordered by parameter; `span` is that stretch (lo, hi) along the line, so a crossing line
/// added later leaves BOTH halves removed (Spec CW-05). A segment stored without a span (a document of before) is the one
/// cell around `mid`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemovedSegment {
    pub u: bool,
    pub position: f64,
    pub mid: f64,
    #[serde(default)]
    pub span: Option<(f64, f64)>,
}

impl RemovedSegment {
    /// Whether the cell `k` of `edges` (the crossing lines along this line, border to border) is removed by this segment.
    pub fn covers_cell(&self, edges: &[f64], k: usize) -> bool {
        match self.span {
            Some((lo, hi)) => lo <= edges[k] + 1e-9 && edges[k + 1] <= hi + 1e-9,
            None => self.mid >= edges[k] && self.mid < edges[k + 1],
        }
    }
}

/// Mullions put on every segment of one line by `AddMullions`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManualMullion {
    /// True: a U (horizontal) line.
    pub u: bool,
    /// The line's position: a height above the base (U) or a station along the wall (V), metres.
    pub position: f64,
    pub mullion_type: ElementId,
}

/// The size of a straight mullion: its width on each side of its line and half its thickness across the wall
/// (metres).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MullionSize {
    pub side1: f64,
    pub side2: f64,
    pub half_thickness: f64,
    /// A circular section (the type's `Radius`): its box across the wall is the tessellated circle's, see [`MullionRec`].
    pub round: bool,
}

impl MullionSize {
    pub fn width(&self) -> f64 {
        self.side1 + self.side2
    }

    /// OBSERVED (straight and circular cases): the `Area` of a straight mullion is half its surface, `(perimeter x length +
    /// 2 x section) / 2`, with the section a rectangle `width x thickness` or a circle of the radius.
    pub fn area(&self, length: f64) -> f64 {
        if self.round {
            let r = self.half_thickness;
            std::f64::consts::PI * r * (length + r)
        } else {
            let (w, t) = (self.width(), 2.0 * self.half_thickness);
            (w + t) * length + w * t
        }
    }
}

/// The names of the template mullion types whose size is known.
pub const SIZED_MULLION_TYPES: [&str; 5] = ["25mm radius", "30mm Square", "50mm radius", "50mm x 150mm", "50mm x 65mm"];

/// The template types with a circular section.
pub const ROUND_MULLION_TYPES: [&str; 2] = ["25mm radius", "50mm radius"];

/// The size of a template mullion type, from the parameters round 23b read (OBSERVED: `Radius`, `Width on side
/// 1/2`, `Thickness`; the template's "25mm radius" type has a radius of 1/3 m, whatever its name says).
pub fn template_size(name: &str) -> Option<MullionSize> {
    let round = |r: f64| MullionSize { side1: r, side2: r, half_thickness: r, round: true };
    let rect = |side: f64, thickness: f64| MullionSize { side1: side, side2: side, half_thickness: thickness / 2.0, round: false };
    Some(match name {
        "25mm radius" => round(1.0 / 3.0),
        "30mm Square" => rect(0.015, 0.03),
        "50mm radius" => round(0.05),
        "50mm x 150mm" => rect(0.025, 0.15),
        "50mm x 65mm" => rect(0.025, 0.065),
        _ => return None,
    })
}

/// The instance part of a curtain wall (keyed by the wall's id in `BimModel::curtain_grids`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurtainGridInstance {
    /// The instance `Number` of the vertical / horizontal layout.
    pub number_vertical: u32,
    pub number_horizontal: u32,
    /// The instance `Justification` codes (2 = Center).
    pub justification_vertical: i64,
    pub justification_horizontal: i64,
    /// Lines added through the API: stations of V lines, heights of U lines (metres).
    pub manual_v: Vec<f64>,
    pub manual_u: Vec<f64>,
    pub manual_mullions: Vec<ManualMullion>,
    /// Segments removed from lines.
    #[serde(default)]
    pub removed: Vec<RemovedSegment>,
    /// Panels replaced by a family (by index in the panel order): the family type's name.
    pub panel_overrides: BTreeMap<usize, String>,
}

impl Default for CurtainGridInstance {
    fn default() -> Self {
        CurtainGridInstance {
            number_vertical: DEFAULT_NUMBER,
            number_horizontal: DEFAULT_NUMBER,
            justification_vertical: 0,
            justification_horizontal: 0,
            manual_v: Vec::new(),
            manual_u: Vec::new(),
            manual_mullions: Vec::new(),
            removed: Vec::new(),
            panel_overrides: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GridLine {
    /// True: a U (horizontal) line.
    pub u: bool,
    /// Height above the base (U) or station along the wall (V), metres.
    pub position: f64,
    pub segments: usize,
    /// Segments taken out by `RemoveSegment` (`segments` counts the existing ones).
    pub skipped: usize,
    /// Created by a layout (an automatic line) rather than by `AddGridLine`.
    pub automatic: bool,
}

/// Spec CW-01: the stored form of a grid line - one direction flag and ONE surface parameter on the reference face of its
/// wall. `const_v` is true for a line that keeps its v constant: a HORIZONTAL line along u, the API's U grid line
/// (OBSERVED: the line at z = 1.5 m stores 4.9213 ft with the flag set). For a planar wall `param` is the distance along
/// the wall path from the face origin (u) or the height above it (v), here in metres. An ARC wall stores an angle: not
/// modelled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StoredGridLine {
    pub const_v: bool,
    pub param: f64,
}

impl GridLine {
    pub fn stored(&self) -> StoredGridLine {
        StoredGridLine { const_v: self.u, param: self.position }
    }

    /// Spec CW-05: the empty-grid warning of the line - removed segments and no remaining segment.
    pub fn empty_grid_warning(&self) -> bool {
        self.skipped > 0 && self.segments == 0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Panel {
    /// The cell: stations along the wall and heights above the base, metres.
    pub v0: f64,
    pub v1: f64,
    pub u0: f64,
    pub u1: f64,
    /// `Panel` for a glass panel for one replaced by a family.
    pub class: &'static str,
    pub type_name: String,
    /// World bounding box of the glass.
    pub bbox_min: [f64; 3],
    pub bbox_max: [f64; 3],
    pub area: f64,
    /// The panel left empty by a removed segment: area 0, no box.
    pub empty: bool,
}

/// The oriented box of a mullion in the wall's frame: along the wall from `s0` to `s1` (metres from the wall start), across it by
/// `half_t` either side of the location line, and from the absolute height `z0` to `z1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MullionBox {
    pub s0: f64,
    pub s1: f64,
    pub z0: f64,
    pub z1: f64,
    pub half_t: f64,
}

/// One mullion: a segment of a grid line or border. Spec CW-04: a mullion is the swept solid of its profile along the path
/// curve of a grid edge, and an end that meets a neighbouring mullion is MITRED (one neighbour) or TRIMMED (two) to it.
/// * the VERTICAL mullion runs the whole segment between the horizontal lines (it continues through a crossing) and the
///   HORIZONTAL one is cut back to the face of the vertical mullion at each end (Spec CW-04 says only that the join pass
///   decides which continues).
/// * a BORDER mullion lies inside the wall edge: its box is `width` wide from the edge, its location is the edge itself;
/// * the location runs along the CCW loop of the face of the cell it bounds, seen from the wall's orientation: interior
///   vertical lines upward, interior horizontal ones right to left, the left border downward, the right border upward, the
///   bottom border left to right, the top border right to left (three of the six directions were measured).
/// * `bbox` / `area` only for a template type whose size is known ([`template_size`]); a circular section's box across the
///   wall is the tessellated circle's and is not derived (OBSERVED 0.33191 for a radius of 1/3 m, exactly the radius for
///   0.05 m).
#[derive(Clone, Debug, PartialEq)]
pub struct MullionRec {
    pub type_name: String,
    /// The two ends, world coordinates.
    pub start: [f64; 3],
    pub end: [f64; 3],
    /// World bounding box, when the type's size is known.
    pub bbox: Option<([f64; 3], [f64; 3])>,
    /// The `Area` parameter, when the type's size is known.
    pub area: Option<f64>,
    /// A circular section.
    pub round: bool,
    /// The oriented box, when the type's size is known.
    pub frame: Option<MullionBox>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CurtainGridState {
    pub length: f64,
    pub height: f64,
    pub u_lines: Vec<GridLine>,
    pub v_lines: Vec<GridLine>,
    pub panels: Vec<Panel>,
    pub mullions: Vec<MullionRec>,
    /// The removed segments (so a line can list its existing spans).
    pub removed: Vec<RemovedSegment>,
}

/// The default mullion types of the reference product's template, in the order of their names (what the harness's
/// `use_mullion_type` index counts). The sizes are not modelled.
pub const TEMPLATE_MULLION_TYPES: [&str; 9] = [
    "25mm radius",
    "30mm Square",
    "50mm radius",
    "50mm x 150mm",
    "50mm x 65mm",
    "L Mullion 1",
    "Quad Mullion 1",
    "Trapezoid Mullion 1",
    "V Mullion 1",
];

/// The template's corner mullion types (OBSERVED: `AddMullions` refuses them on a straight line).
pub const CORNER_MULLION_TYPES: [&str; 4] = ["L Mullion 1", "Quad Mullion 1", "Trapezoid Mullion 1", "V Mullion 1"];

fn invalid(what: &str) -> Failure {
    Failure::error(codes::CURTAIN_INVALID, format!("The curtain wall edit is refused: {what}."))
}

impl BimModel {
    pub fn is_curtain_wall(&self, wall: ElementId) -> bool {
        self.walls.get(&wall).map_or(false, |w| self.curtain_types.contains_key(&w.type_id))
    }
}

/// Make `wall_type` a curtain wall type (its settings start at no layout, no mullions).
pub fn make_curtain_type(m: &mut BimModel, wall_type: ElementId) -> Result<(), Failure> {
    m.wall_type(wall_type)?;
    m.curtain_types.entry(wall_type).or_default();
    Ok(())
}

/// Create a mullion type by name (a name that exists returns the existing type).
pub fn create_mullion_type(m: &mut BimModel, cx: &mut TxnContext<'_>, name: &str) -> ElementId {
    if let Some(t) = m.mullion_types.values().find(|t| t.name == name) {
        return t.id;
    }
    let id = cx.new_id();
    let corner = CORNER_MULLION_TYPES.contains(&name);
    m.mullion_types.insert(id, MullionType { id, name: name.to_owned(), corner });
    id
}

fn curtain_wall(m: &BimModel, wall: ElementId) -> Result<(), Failure> {
    m.wall(wall)?;
    if m.is_curtain_wall(wall) {
        Ok(())
    } else {
        Err(Failure::error(codes::WRONG_KIND, "The wall is not a curtain wall.").on(&[wall]))
    }
}

/// The wall's frame and extent for the grid: (length, base z, height, start, direction).
fn extent(m: &BimModel, wall: ElementId) -> Result<(f64, f64, f64, Vec2, Vec2), Failure> {
    let w = m.wall(wall)?;
    let (_, base, top) = crate::regen::wall_view_body(m, w)?;
    let length = (w.end - w.start).length();
    let d = (w.end - w.start).normalized().ok_or_else(|| invalid("the wall has no length"))?;
    Ok((length, base, top - base, w.start, d))
}

/// `AddGridLine`: a U (horizontal) line at the height of `z`, or a V (vertical) line at the station
/// of `point`. The position must be inside the wall's face (inside a cell); a position equal to an
/// existing line of the same kind changes nothing.
pub fn add_grid_line(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    wall: ElementId,
    u: bool,
    point: Vec2,
    z: f64,
) -> Result<(), Failure> {
    curtain_wall(m, wall)?;
    let (length, base, height, start, d) = extent(m, wall)?;
    let station = (point - start).dot(d);
    let h = z - base;
    let eps = 1e-6;
    if !(station > eps && station < length - eps && h > eps && h < height - eps) {
        return Err(invalid("the position is not inside a cell of the grid"));
    }
    let state = grid_state(m, wall)?;
    let existing = if u { &state.u_lines } else { &state.v_lines };
    let position = if u { h } else { station };
    if existing.iter().any(|l| (l.position - position).abs() < 1e-6) {
        return Ok(());
    }
    let g = m.curtain_grids.entry(wall).or_default();
    if u {
        g.manual_u.push(position);
    } else {
        g.manual_v.push(position);
    }
    Ok(())
}

/// On every segment of the `index`-th line (by position) of a direction.
pub fn add_mullions(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    wall: ElementId,
    u: bool,
    index: usize,
    mullion_type: ElementId,
) -> Result<(), Failure> {
    curtain_wall(m, wall)?;
    let Some(mt) = m.mullion_types.get(&mullion_type) else {
        return Err(invalid("the mullion type does not exist"));
    };
    if mt.corner {
        return Err(invalid("a corner mullion type does not apply to a plain grid line"));
    }
    let state = grid_state(m, wall)?;
    let lines = if u { &state.u_lines } else { &state.v_lines };
    let Some(line) = lines.get(index) else {
        return Err(invalid("the grid line does not exist"));
    };
    let position = line.position;
    let g = m.curtain_grids.entry(wall).or_default();
    // "If any segment already has a mullion, no change is made to that segment"
    if !g.manual_mullions.iter().any(|x| x.u == u && (x.position - position).abs() < 1e-6) {
        g.manual_mullions.push(ManualMullion { u, position, mullion_type });
    }
    Ok(())
}

/// On the `segment`-th EXISTING segment (along the line) of the `index`-th line (by position) of a direction.
pub fn remove_segment(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    wall: ElementId,
    u: bool,
    index: usize,
    segment: usize,
) -> Result<(), Failure> {
    curtain_wall(m, wall)?;
    let state = grid_state(m, wall)?;
    let lines = if u { &state.u_lines } else { &state.v_lines };
    let Some(line) = lines.get(index) else {
        return Err(invalid("the grid line does not exist"));
    };
    let spans = state.existing_spans(line);
    let Some(&mid) = spans.get(segment) else {
        return Err(invalid("the segment does not exist"));
    };
    // Spec CW-05: the removed stretch is bounded by the crossing lines (or the face border) either side of the segment
    let edges = state.line_edges(line);
    let span = (0..edges.len() - 1).find(|&k| edges[k] <= mid && mid < edges[k + 1]).map(|k| (edges[k], edges[k + 1]));
    let position = line.position;
    m.curtain_grids.entry(wall).or_default().removed.push(RemovedSegment { u, position, mid, span });
    Ok(())
}

/// To a family (a door panel): the `index`-th panel in panel order.
pub fn change_panel(m: &mut BimModel, _cx: &mut TxnContext<'_>, wall: ElementId, index: usize, family_type: &str) -> Result<(), Failure> {
    curtain_wall(m, wall)?;
    let state = grid_state(m, wall)?;
    if index >= state.panels.len() {
        return Err(invalid("the panel does not exist"));
    }
    m.curtain_grids.entry(wall).or_default().panel_overrides.insert(index, family_type.to_owned());
    Ok(())
}

/// `SPACING_LAYOUT_*` of a curtain wall TYPE (`vertical`: the V lines' parameters, else the U lines'). The previous
/// non-None layout is kept for the UI, and a switch to Fixed Distance, Maximum Spacing or Minimum Spacing sets the spacing
/// to 5 ft when it is zero. An unknown layout code is refused.
pub fn set_type_layout(m: &mut BimModel, wall_type: ElementId, vertical: bool, code: i64) -> Result<(), Failure> {
    let layout = GridLayout::from_code(code).ok_or_else(|| invalid("the layout code is unknown"))?;
    let t = m.curtain_types.get_mut(&wall_type).ok_or_else(|| invalid("the element is not a curtain wall type"))?;
    let d = if vertical { &mut t.vertical } else { &mut t.horizontal };
    if d.layout != GridLayout::None {
        d.backup_layout = d.layout;
    }
    d.layout = layout;
    if layout.uses_spacing() && d.spacing == 0.0 {
        d.spacing = LAYOUT_SWITCH_SPACING_M;
    }
    Ok(())
}

/// `SPACING_LENGTH_*` of a curtain wall type: read-only while the layout is None or Fixed Number (OBSERVED).
pub fn set_type_spacing(m: &mut BimModel, wall_type: ElementId, vertical: bool, spacing: f64) -> Result<(), Failure> {
    let t = m.curtain_types.get_mut(&wall_type).ok_or_else(|| invalid("the element is not a curtain wall type"))?;
    let d = if vertical { &mut t.vertical } else { &mut t.horizontal };
    if !d.layout.uses_spacing() {
        return Err(Failure::error(codes::READ_ONLY, "The parameter is read-only."));
    }
    d.spacing = spacing;
    Ok(())
}

/// `SPACING_NUM_DIVISIONS_*` of a curtain WALL: read-only unless the type's layout is Fixed Number (OBSERVED); a number
/// outside 1..=200 is not valid (the reference product warns; refusing the value is the engine's reading of that warning).
pub fn set_wall_number(m: &mut BimModel, wall: ElementId, vertical: bool, number: f64) -> Result<(), Failure> {
    curtain_wall(m, wall)?;
    let t = m.curtain_types.get(&m.wall(wall)?.type_id).cloned().unwrap_or_default();
    let layout = if vertical { t.vertical.layout } else { t.horizontal.layout };
    if layout != GridLayout::FixedNumber {
        return Err(Failure::error(codes::READ_ONLY, "The parameter is read-only."));
    }
    if !(number.is_finite() && number.fract() == 0.0 && FIXED_NUMBER_RANGE.contains(&(number as i64))) {
        return Err(invalid("a fixed number is valid from 1 to 200"));
    }
    let g = m.curtain_grids.entry(wall).or_default();
    if vertical {
        g.number_vertical = number as u32;
    } else {
        g.number_horizontal = number as u32;
    }
    Ok(())
}

/// `SPACING_JUSTIFICATION_*` of a curtain WALL, the API code ([`Justify::from_api_code`]).
pub fn set_wall_justification(m: &mut BimModel, wall: ElementId, vertical: bool, code: i64) -> Result<(), Failure> {
    curtain_wall(m, wall)?;
    let g = m.curtain_grids.entry(wall).or_default();
    if vertical {
        g.justification_vertical = code;
    } else {
        g.justification_horizontal = code;
    }
    Ok(())
}

/// Spec CW-06: an edge of the grid can take a CORNER mullion only when both values the driver answers for it (the distances
/// of the edge to the face border) are greater than -1e-9; an interior edge answers a negative value.
pub fn edge_may_take_corner_mullion(distance_a: f64, distance_b: f64) -> bool {
    distance_a > -1e-9 && distance_b > -1e-9
}

/// An arc wall stores an angle: refused.
pub fn import_grid_line(m: &mut BimModel, wall: ElementId, line: StoredGridLine) -> Result<(), Failure> {
    curtain_wall(m, wall)?;
    if m.wall(wall)?.through.is_some() {
        return Err(invalid("a curtain grid on an arc wall is not modelled"));
    }
    let (length, _, height, _, _) = extent(m, wall)?;
    let along = if line.const_v { height } else { length };
    if !(line.param.is_finite() && line.param > 1e-9 && line.param < along - 1e-9) {
        return Err(invalid("the stored line is not inside the face"));
    }
    let g = m.curtain_grids.entry(wall).or_default();
    let list = if line.const_v { &mut g.manual_u } else { &mut g.manual_v };
    if !list.iter().any(|p| (p - line.param).abs() < 1e-6) {
        list.push(line.param);
    }
    Ok(())
}

/// The pair is ordered by increasing parameter.
pub fn import_skipped_segment(m: &mut BimModel, wall: ElementId, owner: StoredGridLine, start: Option<f64>, end: Option<f64>) -> Result<(), Failure> {
    curtain_wall(m, wall)?;
    let (length, _, height, _, _) = extent(m, wall)?;
    let along = if owner.const_v { length } else { height };
    let (a, b) = (start.unwrap_or(0.0), end.unwrap_or(along));
    let (lo, hi) = (a.min(b), a.max(b));
    if !(lo.is_finite() && hi.is_finite()) || hi - lo < 1e-9 {
        return Err(invalid("the stored skipped segment has no extent"));
    }
    let g = m.curtain_grids.entry(wall).or_default();
    g.removed.push(RemovedSegment { u: owner.const_v, position: owner.param, mid: 0.5 * (lo + hi), span: Some((lo, hi)) });
    Ok(())
}

/// A layout's lines along `total` metres: positions strictly inside the wall. (CALCULATION): Fixed Distance steps by the
/// spacing from the justified end (Beginning walks forward from the start only, End backward from the end only, Center in
/// both directions - the engine centres the remainder, OBSERVED), a step is kept only while it stays inside the curve
/// within 1e-9, so the remainder is left at the far end; Fixed Number divides the curve into `number + 1` equal parts;
/// Maximum Spacing divides it into the fewest equal parts no longer than the spacing (OBSERVED: 6.5 m at 2.0 gives 1.625,
/// 3.25, 4.875). ENGINE CHOICE: Minimum Spacing divides it into the most equal parts no shorter than the spacing
/// (`floor(total / spacing)`): the rule says the curve-division virtual decides the count and does not specified its
/// rounding.
fn layout_lines(layout: GridLayout, spacing: f64, number: u32, justify: Justify, total: f64) -> Vec<f64> {
    let eps = 1e-9;
    let usable = spacing > 0.0 && spacing.is_finite();
    let mut out: Vec<f64> = match layout {
        GridLayout::None => Vec::new(),
        GridLayout::FixedNumber => (1..=number).map(|i| i as f64 * total / (number as f64 + 1.0)).collect(),
        GridLayout::FixedDistance if !usable => Vec::new(),
        GridLayout::FixedDistance => {
            let whole = (total / spacing + eps).floor();
            let remainder = total - whole * spacing;
            match justify {
                // the remainder split to both ends
                Justify::Center => (0..=whole as i64).map(|k| remainder / 2.0 + k as f64 * spacing).collect(),
                Justify::End => (1..=whole as i64).map(|k| total - k as f64 * spacing).collect(),
                Justify::Beginning => (1..=whole as i64).map(|k| k as f64 * spacing).collect(),
            }
        }
        GridLayout::MaxSpacing if !usable => Vec::new(),
        GridLayout::MaxSpacing => {
            let n = (total / spacing - eps).ceil().max(1.0);
            (1..n as i64).map(|i| i as f64 * total / n).collect()
        }
        GridLayout::MinSpacing if !usable => Vec::new(),
        GridLayout::MinSpacing => {
            let n = (total / spacing + eps).floor().max(1.0);
            (1..n as i64).map(|i| i as f64 * total / n).collect()
        }
    };
    out.retain(|p| *p > 1e-9 && *p < total - 1e-9);
    out.sort_by(|a, b| a.total_cmp(b));
    out
}

fn merge(auto: Vec<f64>, manual: &[f64]) -> Vec<(f64, bool)> {
    let mut all: Vec<(f64, bool)> = auto.into_iter().map(|p| (p, true)).collect();
    for p in manual {
        if !all.iter().any(|(q, _)| (q - p).abs() < 1e-6) {
            all.push((*p, false));
        }
    }
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    all
}

/// The regenerated grid of a curtain wall: a pure function of the model. Lines: Spec CW-01 (layouts); panels: Spec CW-03;
/// removed segments: Spec CW-05.
pub fn grid_state(m: &BimModel, wall: ElementId) -> Result<CurtainGridState, Failure> {
    curtain_wall(m, wall)?;
    let w = m.wall(wall)?;
    let ct = m.curtain_types.get(&w.type_id).cloned().unwrap_or_default();
    let g = m.curtain_grids.get(&wall).cloned().unwrap_or_default();
    let (length, base, height, start, d) = extent(m, wall)?;
    let n = d.perp_ccw();

    let size_of = |id: Option<ElementId>| id.and_then(|i| m.mullion_types.get(&i)).and_then(|t| template_size(&t.name));
    // "Adjust for Mullion Size": the automatic lines of a Fixed Number layout leave equal CLEAR panels
    let layout = |dir: &DirectionSettings, number: u32, justification: i64, total: f64| -> Vec<f64> {
        let auto = layout_lines(dir.layout, dir.spacing, number, Justify::from_api_code(justification), total);
        if dir.layout != GridLayout::FixedNumber || !dir.adjust {
            return auto;
        }
        let wi = size_of(dir.interior).map_or(0.0, |s| s.width());
        let (wb1, wb2) = (size_of(dir.border1).map_or(0.0, |s| s.width()), size_of(dir.border2).map_or(0.0, |s| s.width()));
        if wi == 0.0 && wb1 == 0.0 && wb2 == 0.0 {
            return auto;
        }
        let nn = number as f64;
        let c = (total - nn * wi - wb1 - wb2) / (nn + 1.0);
        if !(c > 0.0) {
            return auto;
        }
        (1..=number).map(|k| wb1 + k as f64 * c + (k as f64 - 1.0) * wi + wi / 2.0).collect()
    };
    let v_all = merge(layout(&ct.vertical, g.number_vertical, g.justification_vertical, length), &g.manual_v);
    let u_all = merge(layout(&ct.horizontal, g.number_horizontal, g.justification_horizontal, height), &g.manual_u);
    let mut v_edges = vec![0.0];
    v_edges.extend(v_all.iter().map(|p| p.0));
    v_edges.push(length);
    let mut u_edges = vec![0.0];
    u_edges.extend(u_all.iter().map(|p| p.0));
    u_edges.push(height);
    let (v_cells, u_cells) = (v_edges.len() - 1, u_edges.len() - 1);

    // removed segments: the cell pair each removed cell of a line separates (Spec CW-05: a removed stretch covers every
    // cell between its two bounding crossing lines, so a line added across it later leaves both halves removed)
    let mut cell_joins: Vec<((usize, usize), (usize, usize))> = Vec::new();
    // per line: (kind, position) -> the removed cells along it
    let mut skipped_of = |u: bool, position: f64| -> usize {
        let edges: &[f64] = if u { &v_edges } else { &u_edges };
        let mut cells = std::collections::BTreeSet::new();
        for r in g.removed.iter().filter(|r| r.u == u && (r.position - position).abs() < 1e-6) {
            for k in (0..edges.len() - 1).filter(|&k| r.covers_cell(edges, k)) {
                cells.insert(k);
            }
        }
        for &k in &cells {
            if u {
                let Some(ju) = u_all.iter().position(|(p, _)| (p - position).abs() < 1e-6) else { continue };
                cell_joins.push(((k, ju), (k, ju + 1)));
            } else {
                let Some(iv) = v_all.iter().position(|(p, _)| (p - position).abs() < 1e-6) else { continue };
                cell_joins.push(((iv, k), (iv + 1, k)));
            }
        }
        cells.len()
    };
    let u_lines: Vec<GridLine> = u_all
        .iter()
        .map(|(p, auto)| {
            let skipped = skipped_of(true, *p);
            GridLine { u: true, position: *p, segments: v_cells.saturating_sub(skipped), skipped, automatic: *auto }
        })
        .collect();
    let v_lines: Vec<GridLine> = v_all
        .iter()
        .map(|(p, auto)| {
            let skipped = skipped_of(false, *p);
            GridLine { u: false, position: *p, segments: u_cells.saturating_sub(skipped), skipped, automatic: *auto }
        })
        .collect();
    // whether the cell `k` along the line (kind `u`, at `position`) is removed
    let removed_here = |u: bool, position: f64, k: usize| g.removed.iter().any(|r| r.u == u && (r.position - position).abs() < 1e-6 && r.covers_cell(if u { &v_edges } else { &u_edges }, k));
    // connected cells (union-find over the joins)
    let mut parent: Vec<usize> = (0..v_cells * u_cells).collect();
    fn find(parent: &mut Vec<usize>, x: usize) -> usize {
        let mut r = x;
        while parent[r] != r {
            r = parent[r];
        }
        let mut c = x;
        while parent[c] != r {
            let n = parent[c];
            parent[c] = r;
            c = n;
        }
        r
    }
    for ((i0, j0), (i1, j1)) in &cell_joins {
        let (a, b) = (find(&mut parent, i0 * u_cells + j0), find(&mut parent, i1 * u_cells + j1));
        if a != b {
            let (lo, hi) = (a.min(b), a.max(b));
            parent[hi] = lo;
        }
    }

    // Spec CW-03: the panels are the CELLS of the lines of both directions, (n + 1)(m + 1) of them before removed segments
    // merge two cells (Spec CW-05); in the order of the bounding box minimum (x, y, z) to the millimetre (OBSERVED)
    let world = |s: f64, off: f64, z: f64| -> [f64; 3] {
        let p = start + d * s + n * off;
        [p.x, p.y, base + z]
    };
    // the mullions that cut the glass beside a cell (a mullion reaches the glass when its half thickness does)
    let cuts = |s: Option<MullionSize>| s.filter(|s| s.half_thickness > PANEL_OFFSET_M + 1e-9);
    let line_cut = |is_u: bool, pos: f64, k: usize, auto: bool, dir: &DirectionSettings| -> Option<MullionSize> {
        if removed_here(is_u, pos, k) {
            return None;
        }
        let mut best: Option<MullionSize> = None;
        let mut consider = |s: Option<MullionSize>| {
            if let Some(s) = cuts(s) {
                if best.map_or(true, |b| s.width() > b.width()) {
                    best = Some(s);
                }
            }
        };
        if auto {
            consider(size_of(dir.interior));
        }
        for mm in g.manual_mullions.iter().filter(|x| x.u == is_u && (x.position - pos).abs() < 1e-6) {
            consider(size_of(Some(mm.mullion_type)));
        }
        best
    };
    let mut panels: Vec<Panel> = Vec::new();
    for i in 0..v_cells {
        for j in 0..u_cells {
            let (c0, c1, d0, d1) = (v_edges[i], v_edges[i + 1], u_edges[j], u_edges[j + 1]);
            // along the wall: a vertical line (or the vertical borders) bounds the cell on each side
            let sl = if i == 0 {
                cuts(size_of(ct.vertical.border1)).map_or(0.0, |s| s.width())
            } else {
                line_cut(false, v_all[i - 1].0, j, v_all[i - 1].1, &ct.vertical).map_or(0.0, |s| s.side2)
            };
            let sr = if i + 1 == v_cells {
                cuts(size_of(ct.vertical.border2)).map_or(0.0, |s| s.width())
            } else {
                line_cut(false, v_all[i].0, j, v_all[i].1, &ct.vertical).map_or(0.0, |s| s.side1)
            };
            // up the wall: a horizontal line (or the horizontal borders)
            let sb = if j == 0 {
                cuts(size_of(ct.horizontal.border1)).map_or(0.0, |s| s.width())
            } else {
                line_cut(true, u_all[j - 1].0, i, u_all[j - 1].1, &ct.horizontal).map_or(0.0, |s| s.side2)
            };
            let st = if j + 1 == u_cells {
                cuts(size_of(ct.horizontal.border2)).map_or(0.0, |s| s.width())
            } else {
                line_cut(true, u_all[j].0, i, u_all[j].1, &ct.horizontal).map_or(0.0, |s| s.side1)
            };
            let (v0, v1, u0, u1) = (c0 + sl, (c1 - sr).max(c0 + sl), d0 + sb, (d1 - st).max(d0 + sb));
            let corners = [
                world(v0, PANEL_OFFSET_M, u0),
                world(v1, PANEL_OFFSET_M, u0),
                world(v0, PANEL_OFFSET_M + PANEL_THICKNESS_M, u1),
                world(v1, PANEL_OFFSET_M + PANEL_THICKNESS_M, u1),
            ];
            let mut lo = [f64::INFINITY; 3];
            let mut hi = [f64::NEG_INFINITY; 3];
            for c in corners {
                for k in 0..3 {
                    lo[k] = lo[k].min(c[k]);
                    hi[k] = hi[k].max(c[k]);
                }
            }
            panels.push(Panel {
                v0: c0,
                v1: c1,
                u0: d0,
                u1: d1,
                class: "Panel",
                type_name: "Glazed".to_owned(),
                bbox_min: lo,
                bbox_max: hi,
                area: (v1 - v0) * (u1 - u0),
                empty: false,
            });
        }
    }
    // cells joined by a removed segment form ONE panel (the first cell's); the others are left empty
    if !cell_joins.is_empty() {
        let n = panels.len();
        let roots: Vec<usize> = (0..n).map(|k| find(&mut parent, k)).collect();
        for r in 0..n {
            if roots[r] != r {
                continue;
            }
            let members: Vec<usize> = (0..n).filter(|k| roots[*k] == r).collect();
            if members.len() < 2 {
                continue;
            }
            let (mut lo, mut hi, mut area) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3], 0.0);
            for k in &members {
                for c in 0..3 {
                    lo[c] = lo[c].min(panels[*k].bbox_min[c]);
                    hi[c] = hi[c].max(panels[*k].bbox_max[c]);
                }
                area += panels[*k].area;
            }
            for k in &members {
                if *k == r {
                    panels[*k].bbox_min = lo;
                    panels[*k].bbox_max = hi;
                    panels[*k].area = area;
                } else {
                    panels[*k].area = 0.0;
                    panels[*k].empty = true;
                }
            }
        }
    }
    let key = |p: &Panel| [(p.bbox_min[0] * 1000.0).round(), (p.bbox_min[1] * 1000.0).round(), (p.bbox_min[2] * 1000.0).round()];
    panels.sort_by(|a, b| {
        let (ka, kb) = (key(a), key(b));
        // the empty panels come first (OBSERVED), then the bounding box minimum
        b.empty.cmp(&a.empty).then(ka[0].total_cmp(&kb[0])).then(ka[1].total_cmp(&kb[1])).then(ka[2].total_cmp(&kb[2]))
    });
    for (idx, name) in &g.panel_overrides {
        if let Some(p) = panels.get_mut(*idx) {
            p.class = "FamilyInstance";
            p.type_name = name.clone();
        }
    }

    // mullions: the automatic ones, then the added ones (Spec CW-04 gives no geometry: see [`MullionRec`])
    #[derive(Clone, Copy, PartialEq)]
    enum Edge {
        Interior,
        First,
        Last,
    }
    let type_name = |id: Option<ElementId>| id.and_then(|i| m.mullion_types.get(&i)).map(|t| t.name.clone());
    // the widest mullion of a line over the segments `ks` that are not removed: the automatic interior one and the added ones
    let line_size = |is_u: bool, pos: f64, ks: &[usize], auto: bool, dir: &DirectionSettings| -> Option<MullionSize> {
        let mut best: Option<MullionSize> = None;
        for &k in ks {
            if removed_here(is_u, pos, k) {
                continue;
            }
            let mut consider = |s: Option<MullionSize>| {
                if let Some(s) = s {
                    if best.map_or(true, |b| s.width() > b.width()) {
                        best = Some(s);
                    }
                }
            };
            if auto {
                consider(size_of(dir.interior));
            }
            for mm in g.manual_mullions.iter().filter(|x| x.u == is_u && (x.position - pos).abs() < 1e-6) {
                consider(size_of(Some(mm.mullion_type)));
            }
        }
        best
    };
    let record = |name: &str, size: Option<MullionSize>, s0: f64, s1: f64, z0: f64, z1: f64, start: [f64; 3], end: [f64; 3], length: f64| -> MullionRec {
        let (frame, bbox, area) = match size {
            Some(sz) => {
                let f = MullionBox { s0, s1, z0: base + z0, z1: base + z1, half_t: sz.half_thickness };
                let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                for sx in [f.s0, f.s1] {
                    for off in [-f.half_t, f.half_t] {
                        for z in [z0, z1] {
                            let c = world(sx, off, z);
                            for k in 0..3 {
                                lo[k] = lo[k].min(c[k]);
                                hi[k] = hi[k].max(c[k]);
                            }
                        }
                    }
                }
                (Some(f), Some((lo, hi)), Some(sz.area(length)))
            }
            None => (None, None, None),
        };
        MullionRec { type_name: name.to_owned(), start, end, bbox, area, round: size.map_or(false, |s| s.round), frame }
    };
    let mut mullions: Vec<MullionRec> = Vec::new();
    // a vertical mullion on every segment of the V line at `pos`
    let place_v = |pos: f64, id: Option<ElementId>, edge: Edge, mullions: &mut Vec<MullionRec>| {
        let Some(name) = type_name(id) else { return };
        let size = size_of(id);
        for j in 0..u_cells {
            if removed_here(false, pos, j) {
                continue;
            }
            let (za, zb) = (u_edges[j], u_edges[j + 1]);
            let (s0, s1, at) = match (size, edge) {
                (Some(sz), Edge::Interior) => (pos - sz.side1, pos + sz.side2, pos),
                (Some(sz), Edge::First) => (0.0, sz.width(), 0.0),
                (Some(sz), Edge::Last) => (length - sz.width(), length, length),
                (None, _) => (pos, pos, pos),
            };
            let (start, end) = if edge == Edge::First { (world(at, 0.0, zb), world(at, 0.0, za)) } else { (world(at, 0.0, za), world(at, 0.0, zb)) };
            mullions.push(record(&name, size, s0, s1, za, zb, start, end, zb - za));
        }
    };
    // a horizontal mullion on every segment of the U line at height `pos`, cut back to the vertical mullions at its ends
    let place_u = |pos: f64, id: Option<ElementId>, edge: Edge, mullions: &mut Vec<MullionRec>| {
        let Some(name) = type_name(id) else { return };
        let size = size_of(id);
        // the vertical segments next to this line
        let ks: Vec<usize> = match edge {
            Edge::First => vec![0],
            Edge::Last => vec![u_cells - 1],
            Edge::Interior => match u_all.iter().position(|(p, _)| (p - pos).abs() < 1e-6) {
                Some(mm) => vec![mm, mm + 1],
                None => Vec::new(),
            },
        };
        for i in 0..v_cells {
            if removed_here(true, pos, i) {
                continue;
            }
            let sa = if i == 0 {
                size_of(ct.vertical.border1).map_or(0.0, |s| s.width())
            } else {
                v_edges[i] + line_size(false, v_all[i - 1].0, &ks, v_all[i - 1].1, &ct.vertical).map_or(0.0, |s| s.side2)
            };
            let sb = if i + 1 == v_cells {
                length - size_of(ct.vertical.border2).map_or(0.0, |s| s.width())
            } else {
                v_edges[i + 1] - line_size(false, v_all[i].0, &ks, v_all[i].1, &ct.vertical).map_or(0.0, |s| s.side1)
            };
            let (sa, sb) = (sa.min(sb), sa.max(sb));
            let (z0, z1, at) = match (size, edge) {
                (Some(sz), Edge::Interior) => (pos - sz.side1, pos + sz.side2, pos),
                (Some(sz), Edge::First) => (0.0, sz.width(), 0.0),
                (Some(sz), Edge::Last) => (height - sz.width(), height, height),
                (None, _) => (pos, pos, pos),
            };
            let (start, end) = if edge == Edge::First { (world(sa, 0.0, at), world(sb, 0.0, at)) } else { (world(sb, 0.0, at), world(sa, 0.0, at)) };
            mullions.push(record(&name, size, sa, sb, z0, z1, start, end, sb - sa));
        }
    };
    for l in v_lines.iter().filter(|l| l.automatic) {
        place_v(l.position, ct.vertical.interior, Edge::Interior, &mut mullions);
    }
    for l in u_lines.iter().filter(|l| l.automatic) {
        place_u(l.position, ct.horizontal.interior, Edge::Interior, &mut mullions);
    }
    place_v(0.0, ct.vertical.border1, Edge::First, &mut mullions);
    place_v(length, ct.vertical.border2, Edge::Last, &mut mullions);
    place_u(0.0, ct.horizontal.border1, Edge::First, &mut mullions);
    place_u(height, ct.horizontal.border2, Edge::Last, &mut mullions);
    for mm in &g.manual_mullions {
        if mm.u {
            place_u(mm.position, Some(mm.mullion_type), Edge::Interior, &mut mullions);
        } else {
            place_v(mm.position, Some(mm.mullion_type), Edge::Interior, &mut mullions);
        }
    }
    // OBSERVED (the harness lists them so): by class, type, then the box minimum (x, y, z) to the millimetre; a mullion without a box
    // (a type of unknown size) keeps the generation order
    let mm = |v: f64| (v * 1000.0).round();
    if mullions.iter().all(|x| x.bbox.is_some()) {
        mullions.sort_by(|a, b| {
            let (ka, kb) = (a.bbox.expect("box").0, b.bbox.expect("box").0);
            a.type_name
                .cmp(&b.type_name)
                .then(mm(ka[0]).total_cmp(&mm(kb[0])))
                .then(mm(ka[1]).total_cmp(&mm(kb[1])))
                .then(mm(ka[2]).total_cmp(&mm(kb[2])))
        });
    }

    Ok(CurtainGridState { length, height, u_lines, v_lines, panels, mullions, removed: g.removed.clone() })
}

impl CurtainGridState {
    /// The middles of a line's EXISTING segments, in order along the line (what `RemoveSegment` indexes).
    pub fn existing_spans(&self, line: &GridLine) -> Vec<f64> {
        // the crossing lines of the other kind cut this line into cells; a removed one is not listed
        let edges = self.line_edges(line);
        let removed = &self.removed;
        (0..edges.len() - 1)
            .filter(|&k| !removed.iter().any(|r| r.u == line.u && (r.position - line.position).abs() < 1e-6 && r.covers_cell(&edges, k)))
            .map(|k| 0.5 * (edges[k] + edges[k + 1]))
            .collect()
    }

    /// The crossing lines of the other kind along `line`, border to border: the edges of its segments.
    pub fn line_edges(&self, line: &GridLine) -> Vec<f64> {
        let cross: Vec<f64> = if line.u { self.v_lines.iter().map(|l| l.position).collect() } else { self.u_lines.iter().map(|l| l.position).collect() };
        let extent = if line.u { self.length } else { self.height };
        let mut edges = vec![0.0];
        edges.extend(cross);
        edges.push(extent);
        edges
    }

    /// World start and end of a line: a U line runs along the whole wall at its height, a V line
    /// over the whole height at its station.
    pub fn line_curve(&self, model: &BimModel, wall: ElementId, line: &GridLine) -> Result<([f64; 3], [f64; 3]), Failure> {
        let (_, base, _, start, d) = extent(model, wall)?;
        let at = |s: f64, z: f64| {
            let p = start + d * s;
            [p.x, p.y, base + z]
        };
        Ok(if line.u {
            (at(0.0, line.position), at(self.length, line.position))
        } else {
            (at(line.position, 0.0), at(line.position, self.height))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ops, BimDocument, CompoundStructure, Layer, LayerFunction};

    struct Fx {
        doc: BimDocument,
        wall_type: ElementId,
        wall: ElementId,
    }

    fn fixture(length: f64) -> Fx {
        let mut doc = crate::new_document();
        let (wall_type, wall) = doc
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let wt = ops::create_wall_type(m, cx, "CW", CompoundStructure::new(vec![Layer { width: 0.025, function: LayerFunction::Structure }], 0, 0)?)?;
                make_curtain_type(m, wt)?;
                let wall = ops::create_wall(
                    m,
                    cx,
                    ops::NewWall { wall_type: wt, level: l, start: Vec2::new(0.0, 0.0), end: Vec2::new(length, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false },
                )?;
                Ok((wt, wall))
            })
            .value
            .expect("fixture");
        Fx { doc, wall_type, wall }
    }

    fn positions(lines: &[GridLine]) -> Vec<f64> {
        lines.iter().map(|l| (l.position * 1000.0).round() / 1000.0).collect()
    }

    #[test]
    fn a_default_curtain_wall_is_one_glass_panel_of_the_walls_area() {
        let f = fixture(6.0);
        let s = grid_state(f.doc.model(), f.wall).unwrap();
        assert_eq!((s.u_lines.len(), s.v_lines.len(), s.panels.len(), s.mullions.len()), (0, 0, 1, 0));
        assert_eq!(s.panels[0].area, 18.0);
        assert!((s.panels[0].bbox_min[1] - 0.0245).abs() < 1e-12 && (s.panels[0].bbox_max[1] - 0.0495).abs() < 1e-12);
        let w = crate::regen::wall_state(f.doc.model(), f.doc.model().wall(f.wall).unwrap()).unwrap();
        assert!((w.area - 18.0).abs() < 1e-9 && (w.volume - 0.45).abs() < 1e-9, "{} {}", w.area, w.volume);
        // the orientation is the negated left normal
        assert_eq!(w.orientation, Vec2::new(0.0, -1.0));
    }

    #[test]
    fn grid_lines_added_by_position_split_the_panels_and_a_repeat_is_ignored() {
        let mut f = fixture(6.0);
        let w = f.wall;
        // on a cell boundary: refused
        assert!(f.doc.transact("edge", false, |m, cx| add_grid_line(m, cx, w, true, Vec2::new(2.0, 0.0), 0.0)).value.is_none());
        assert!(f.doc.transact("edge", false, |m, cx| add_grid_line(m, cx, w, false, Vec2::new(0.0, 0.0), 1.5)).value.is_none());
        assert!(f.doc.transact("u", false, |m, cx| add_grid_line(m, cx, w, true, Vec2::new(2.0, 0.0), 1.5)).committed());
        let s = grid_state(f.doc.model(), w).unwrap();
        assert_eq!((positions(&s.u_lines), s.v_lines.len(), s.panels.len()), (vec![1.5], 0, 2));
        // the same height again: ignored
        assert!(f.doc.transact("u2", false, |m, cx| add_grid_line(m, cx, w, true, Vec2::new(4.0, 0.0), 1.5)).committed());
        assert_eq!(grid_state(f.doc.model(), w).unwrap().u_lines.len(), 1);
        assert!(f.doc.transact("v", false, |m, cx| add_grid_line(m, cx, w, false, Vec2::new(3.0, 0.0), 1.5)).committed());
        let s = grid_state(f.doc.model(), w).unwrap();
        assert_eq!((s.u_lines.len(), s.v_lines.len(), s.panels.len()), (1, 1, 4));
        assert_eq!((s.u_lines[0].segments, s.v_lines[0].segments), (2, 2));
        assert!(s.panels.iter().all(|p| (p.area - 4.5).abs() < 1e-9));
        let (a, b) = s.line_curve(f.doc.model(), w, &s.u_lines[0]).unwrap();
        assert_eq!((a, b), ([0.0, 0.0, 1.5], [6.0, 0.0, 1.5]));
        let (a, b) = s.line_curve(f.doc.model(), w, &s.v_lines[0]).unwrap();
        assert_eq!((a, b), ([3.0, 0.0, 0.0], [3.0, 0.0, 3.0]));
        // mullions on every segment of a line: one per segment, once
        let (mt, ) = f.doc.transact("mt", false, |m, cx| Ok((create_mullion_type(m, cx, "25mm radius"),))).value.unwrap();
        assert!(f.doc.transact("mu", false, |m, cx| add_mullions(m, cx, w, true, 0, mt)).committed());
        assert!(f.doc.transact("mu", false, |m, cx| add_mullions(m, cx, w, true, 0, mt)).committed());
        assert_eq!(grid_state(f.doc.model(), w).unwrap().mullions.len(), 2);
        assert!(f.doc.transact("mu", false, |m, cx| add_mullions(m, cx, w, false, 0, mt)).committed());
        assert_eq!(grid_state(f.doc.model(), w).unwrap().mullions.len(), 4);
        assert!(f.doc.transact("bad", false, |m, cx| add_mullions(m, cx, w, true, 5, mt)).value.is_none());
        // a panel replaced by a family
        assert!(f.doc.transact("door", false, |m, cx| change_panel(m, cx, w, 0, "AST-CW-DOOR")).committed());
        assert_eq!(grid_state(f.doc.model(), w).unwrap().panels.iter().filter(|p| p.class == "FamilyInstance").count(), 1);
    }

    fn set_layout(f: &mut Fx, vertical: bool, layout: GridLayout, spacing: f64) {
        let wt = f.wall_type;
        f.doc.transact("layout", false, |m, _| {
            let t = m.curtain_types.get_mut(&wt).unwrap();
            let d = if vertical { &mut t.vertical } else { &mut t.horizontal };
            d.layout = layout;
            d.spacing = spacing;
            Ok(())
        });
    }

    #[test]
    fn mullion_sizes_move_the_lines_and_cut_the_glass_that_they_reach() {
        // OBSERVED: Fixed Number 3 on 6 m with an interior 25 mm radius type (radius 1/3 m) leaves clear panels of 1.0 m:
        // lines at 1.3333, 3, 4.6667 and the glass between the mullion edges
        let mut f = fixture(6.0);
        set_layout(&mut f, true, GridLayout::FixedNumber, 1.0);
        let (w, wt) = (f.wall, f.wall_type);
        f.doc.transact("n", false, |m, cx| {
            m.curtain_grids.entry(w).or_default().number_vertical = 3;
            let mt = create_mullion_type(m, cx, "25mm radius");
            m.curtain_types.get_mut(&wt).unwrap().vertical.interior = Some(mt);
            Ok(())
        });
        let s = grid_state(f.doc.model(), w).unwrap();
        assert_eq!(positions(&s.v_lines), vec![1.333, 3.0, 4.667]);
        let xs: Vec<(f64, f64)> = s.panels.iter().map(|p| ((p.bbox_min[0] * 1e4).round() / 1e4, (p.bbox_max[0] * 1e4).round() / 1e4)).collect();
        assert_eq!(xs, vec![(0.0, 1.0), (1.6667, 2.6667), (3.3333, 4.3333), (5.0, 6.0)]);
        assert!(s.panels.iter().all(|p| (p.area - 3.0).abs() < 1e-9));
        // Adjust for Mullion Size off: the lines stay, the glass still stops at the mullions
        f.doc.transact("off", false, |m, _| {
            m.curtain_types.get_mut(&wt).unwrap().vertical.adjust = false;
            Ok(())
        });
        let s = grid_state(f.doc.model(), w).unwrap();
        assert_eq!(positions(&s.v_lines), vec![1.5, 3.0, 4.5]);
        assert!((s.panels[0].bbox_max[0] - (1.5 - 1.0 / 3.0)).abs() < 1e-9);
        // a 30 mm square mullion does not reach the glass; a 50 x 150 mm one cuts 25 mm a side
        let mut g = fixture(6.0);
        let gw = g.wall;
        g.doc.transact("lines", false, |m, cx| {
            add_grid_line(m, cx, gw, false, Vec2::new(3.0, 0.0), 1.5)?;
            let sq = create_mullion_type(m, cx, "30mm Square");
            add_mullions(m, cx, gw, false, 0, sq)
        });
        let s = grid_state(g.doc.model(), gw).unwrap();
        assert_eq!((s.panels[0].bbox_max[0], s.panels[0].area), (3.0, 9.0));
        let mut h = fixture(6.0);
        let hw = h.wall;
        h.doc.transact("lines", false, |m, cx| {
            add_grid_line(m, cx, hw, false, Vec2::new(3.0, 0.0), 1.5)?;
            let big = create_mullion_type(m, cx, "50mm x 150mm");
            add_mullions(m, cx, hw, false, 0, big)
        });
        let s = grid_state(h.doc.model(), hw).unwrap();
        assert!((s.panels[0].bbox_max[0] - 2.975).abs() < 1e-9 && (s.panels[0].area - 8.925).abs() < 1e-9);
    }

    #[test]
    fn a_removed_segment_merges_two_panels_and_leaves_one_empty() {
        // OBSERVED (`wr`): the panels below and above the removed half of the U line become one panel of 9 m2 and the other
        // panel stays in the list empty; the line reports one existing and one skipped segment
        let mut f = fixture(6.0);
        let w = f.wall;
        f.doc.transact("lines", false, |m, cx| {
            add_grid_line(m, cx, w, true, Vec2::new(3.0, 0.0), 1.5)?;
            add_grid_line(m, cx, w, false, Vec2::new(3.0, 0.0), 1.5)
        });
        assert!(f.doc.transact("rm", false, |m, cx| remove_segment(m, cx, w, true, 0, 0)).committed());
        let s = grid_state(f.doc.model(), w).unwrap();
        assert_eq!((s.u_lines[0].segments, s.u_lines[0].skipped), (1, 1));
        assert_eq!(s.panels.len(), 4);
        assert!(s.panels[0].empty && s.panels[0].area == 0.0);
        assert!((s.panels[1].area - 9.0).abs() < 1e-9);
        // the next removal counts the EXISTING segments only: index 1 of the V line is its upper half
        assert!(f.doc.transact("rm2", false, |m, cx| remove_segment(m, cx, w, false, 0, 1)).committed());
        let s = grid_state(f.doc.model(), w).unwrap();
        assert!(s.panels[0].empty && s.panels[1].empty && (s.panels[2].area - 13.5).abs() < 1e-9);
        assert!(f.doc.transact("bad", false, |m, cx| remove_segment(m, cx, w, true, 0, 1)).value.is_none());
        // a corner mullion type is refused on a plain line
        let corner = f.doc.transact("c", false, |m, cx| Ok(create_mullion_type(m, cx, "L Mullion 1"))).value.unwrap();
        assert!(f.doc.transact("corner", false, |m, cx| add_mullions(m, cx, w, true, 0, corner)).value.is_none());
    }

    #[test]
    fn the_layouts_make_the_observed_lines() {
        // Fixed Number 3 on 6 m: lines at 1.5, 3, 4.5
        let mut f = fixture(6.0);
        set_layout(&mut f, true, GridLayout::FixedNumber, 1.0);
        let w = f.wall;
        f.doc.transact("n", false, |m, _| {
            m.curtain_grids.entry(w).or_default().number_vertical = 3;
            Ok(())
        });
        let s = grid_state(f.doc.model(), w).unwrap();
        assert_eq!((positions(&s.v_lines), s.panels.len()), (vec![1.5, 3.0, 4.5], 4));
        // horizontal Fixed Number 2 on 3 m: U lines at 1 and 2, 12 panels with the 3 V lines
        set_layout(&mut f, false, GridLayout::FixedNumber, 1.0);
        f.doc.transact("n", false, |m, _| {
            m.curtain_grids.entry(w).or_default().number_horizontal = 2;
            Ok(())
        });
        let s = grid_state(f.doc.model(), w).unwrap();
        assert_eq!((positions(&s.u_lines), s.panels.len()), (vec![1.0, 2.0], 12));
        // Fixed Distance 1.5 on 6.5 m: 1.5, 3, 4.5, 6; the last panel 0.5 wide
        let mut g = fixture(6.5);
        set_layout(&mut g, true, GridLayout::FixedDistance, 1.5);
        let s = grid_state(g.doc.model(), g.wall).unwrap();
        assert_eq!(positions(&s.v_lines), vec![1.5, 3.0, 4.5, 6.0]);
        assert_eq!(s.panels.len(), 5);
        assert!(s.panels.iter().any(|p| (p.area - 1.5).abs() < 1e-9), "the 0.5 m x 3 m remainder");
        // Center: the remainder split, lines at 0.25, 1.75, ... 6.25
        let gw = g.wall;
        g.doc.transact("j", false, |m, _| {
            m.curtain_grids.entry(gw).or_default().justification_vertical = 2;
            Ok(())
        });
        let s = grid_state(g.doc.model(), g.wall).unwrap();
        assert_eq!(positions(&s.v_lines), vec![0.25, 1.75, 3.25, 4.75, 6.25]);
        // Maximum Spacing 2.0 on 6.5 m: four equal divisions
        let mut h = fixture(6.5);
        set_layout(&mut h, true, GridLayout::MaxSpacing, 2.0);
        let s = grid_state(h.doc.model(), h.wall).unwrap();
        assert_eq!(positions(&s.v_lines), vec![1.625, 3.25, 4.875]);
    }

    #[test]
    fn automatic_mullions_follow_the_segments_of_the_lines_and_the_borders() {
        // 3 V lines with an interior type: 3 mullions; with both borders: 5 (one segment each)
        let mut f = fixture(6.0);
        let (wt, w) = (f.wall_type, f.wall);
        let (mt0, mt1) = f
            .doc
            .transact("mt", false, |m, cx| Ok((create_mullion_type(m, cx, "25mm radius"), create_mullion_type(m, cx, "30mm Square"))))
            .value
            .unwrap();
        set_layout(&mut f, true, GridLayout::FixedNumber, 1.0);
        f.doc.transact("set", false, |m, _| {
            m.curtain_grids.entry(w).or_default().number_vertical = 3;
            let t = m.curtain_types.get_mut(&wt).unwrap();
            t.vertical.interior = Some(mt0);
            Ok(())
        });
        assert_eq!(grid_state(f.doc.model(), w).unwrap().mullions.len(), 3);
        f.doc.transact("set", false, |m, _| {
            let t = m.curtain_types.get_mut(&wt).unwrap();
            t.vertical.border1 = Some(mt1);
            t.vertical.border2 = Some(mt1);
            Ok(())
        });
        assert_eq!(grid_state(f.doc.model(), w).unwrap().mullions.len(), 5);
        // horizontal: 4 lines, interior type: 4 lines x 4 V cells + the V lines cut into 5 segments each + the
        // two borders of 5 segments: 41 (OBSERVED)
        set_layout(&mut f, false, GridLayout::FixedNumber, 1.0);
        f.doc.transact("set", false, |m, _| {
            let t = m.curtain_types.get_mut(&wt).unwrap();
            t.horizontal.interior = Some(mt0);
            Ok(())
        });
        let s = grid_state(f.doc.model(), w).unwrap();
        assert_eq!((s.u_lines.len(), s.v_lines.len(), s.panels.len()), (4, 3, 20));
        assert_eq!(s.mullions.len(), 41);
    }

    #[test]
    fn spec_cw02_the_layout_values_the_justification_the_number_range_and_the_spacing_a_switch_gives() {
        // VALUES: 0 None, 1 Fixed Distance, 2 Fixed Number, 3 Maximum Spacing, 5 Minimum Spacing (4 is not named)
        for c in [0, 1, 2, 3, 5] {
            assert_eq!(GridLayout::from_code(c).unwrap().code(), c);
        }
        assert!(GridLayout::from_code(4).is_none() && GridLayout::from_code(-1).is_none() && GridLayout::from_code(6).is_none());
        // the flip flag exchanges Beginning and End
        assert_eq!((Justify::Beginning.flipped(), Justify::End.flipped(), Justify::Center.flipped()), (Justify::End, Justify::Beginning, Justify::Center));
        // Fixed Distance walks from the justified end and leaves the remainder at the far end: 6.5 m at 1.5 m
        let lines = |l, s, j| layout_lines(l, s, 0, j, 6.5).iter().map(|p| (p * 1e6).round() / 1e6).collect::<Vec<_>>();
        assert_eq!(lines(GridLayout::FixedDistance, 1.5, Justify::Beginning), vec![1.5, 3.0, 4.5, 6.0]);
        assert_eq!(lines(GridLayout::FixedDistance, 1.5, Justify::End), vec![0.5, 2.0, 3.5, 5.0]);
        assert_eq!(lines(GridLayout::FixedDistance, 1.5, Justify::Center), vec![0.25, 1.75, 3.25, 4.75, 6.25]);
        // a step exactly on the far end is not kept (inside the curve within 1e-9)
        assert_eq!(layout_lines(GridLayout::FixedDistance, 2.0, 0, Justify::Beginning, 6.0), vec![2.0, 4.0]);
        // Maximum Spacing (OBSERVED 6.5 at 2.0) and, ENGINE CHOICE, Minimum Spacing: floor(6.5 / 2.0) = 3 parts
        assert_eq!(lines(GridLayout::MaxSpacing, 2.0, Justify::Beginning), vec![1.625, 3.25, 4.875]);
        assert_eq!(lines(GridLayout::MinSpacing, 2.0, Justify::Beginning), vec![2.166667, 4.333333]);
        assert!(layout_lines(GridLayout::MinSpacing, 9.0, 0, Justify::Beginning, 6.5).is_empty(), "a spacing longer than the wall: one part, no line");
        // Fixed Number does not touch it
        let mut f = fixture(6.0);
        let (wt, w) = (f.wall_type, f.wall);
        assert_eq!(f.doc.model().curtain_types[&wt].vertical.spacing, 0.0);
        assert!(f.doc.transact("fn", false, |m, _| set_type_layout(m, wt, true, 2)).committed());
        assert_eq!(f.doc.model().curtain_types[&wt].vertical.spacing, 0.0, "Fixed Number is not a spacing layout");
        assert!(f.doc.transact("fd", false, |m, _| set_type_layout(m, wt, true, 1)).committed());
        assert!((f.doc.model().curtain_types[&wt].vertical.spacing - 1.524).abs() < 1e-12);
        assert!(f.doc.transact("sp", false, |m, _| set_type_spacing(m, wt, true, 2.0)).committed());
        assert!(f.doc.transact("ms", false, |m, _| set_type_layout(m, wt, true, 5)).committed());
        assert_eq!(f.doc.model().curtain_types[&wt].vertical.spacing, 2.0, "a set spacing is kept");
        // the previous non-None layout is kept for the UI while the layout is None
        assert!(f.doc.transact("none", false, |m, _| set_type_layout(m, wt, true, 0)).committed());
        let d = &f.doc.model().curtain_types[&wt].vertical;
        assert_eq!((d.layout, d.backup_layout), (GridLayout::None, GridLayout::MinSpacing));
        // the spacing is read-only while the layout is None or Fixed Number; an unknown layout code is refused
        assert!(f.doc.transact("ro", false, |m, _| set_type_spacing(m, wt, true, 3.0)).value.is_none());
        assert!(f.doc.transact("bad", false, |m, _| set_type_layout(m, wt, true, 4)).value.is_none());
        // the fixed number is valid from 1 to 200 and read-only unless the layout is Fixed Number
        assert!(f.doc.transact("ro", false, |m, _| set_wall_number(m, w, true, 5.0)).value.is_none());
        assert!(f.doc.transact("fn", false, |m, _| set_type_layout(m, wt, true, 2)).committed());
        for bad in [0.0, 201.0, -3.0, 2.5, f64::NAN] {
            assert!(f.doc.transact("n", false, |m, _| set_wall_number(m, w, true, bad)).value.is_none(), "{bad}");
        }
        for ok in [1.0, 200.0] {
            assert!(f.doc.transact("n", false, |m, _| set_wall_number(m, w, true, ok)).committed(), "{ok}");
            assert_eq!(f.doc.model().curtain_grids[&w].number_vertical, ok as u32);
        }
        assert!(FIXED_NUMBER_RANGE.contains(&1) && FIXED_NUMBER_RANGE.contains(&200) && !FIXED_NUMBER_RANGE.contains(&201));
    }

    #[test]
    fn spec_cw01_a_line_is_a_direction_flag_and_one_parameter_and_a_stored_line_is_inserted_as_it_is() {
        let mut f = fixture(6.0);
        let w = f.wall;
        // the API U line is HORIZONTAL: the flag "keeps v constant" (Spec CW-01: flag 1 fixes v and runs along u), its
        // parameter the height
        assert!(f.doc.transact("u", false, |m, cx| add_grid_line(m, cx, w, true, Vec2::new(2.0, 0.0), 1.5)).committed());
        assert!(f.doc.transact("v", false, |m, cx| add_grid_line(m, cx, w, false, Vec2::new(2.0, 0.0), 1.5)).committed());
        let s = grid_state(f.doc.model(), w).unwrap();
        assert_eq!(s.u_lines[0].stored(), StoredGridLine { const_v: true, param: 1.5 });
        assert_eq!(s.v_lines[0].stored(), StoredGridLine { const_v: false, param: 2.0 });
        // import: the same lines in a fresh wall, a repeat is ignored, a parameter on or outside the face border is refused
        let mut g = fixture(6.0);
        let gw = g.wall;
        assert!(g.doc.transact("i", false, |m, _| import_grid_line(m, gw, StoredGridLine { const_v: true, param: 1.5 })).committed());
        assert!(g.doc.transact("i", false, |m, _| import_grid_line(m, gw, StoredGridLine { const_v: false, param: 2.0 })).committed());
        assert!(g.doc.transact("i", false, |m, _| import_grid_line(m, gw, StoredGridLine { const_v: true, param: 1.5 })).committed());
        let t = grid_state(g.doc.model(), gw).unwrap();
        assert_eq!((positions(&t.u_lines), positions(&t.v_lines), t.panels.len()), (vec![1.5], vec![2.0], 4));
        for (const_v, param) in [(true, 0.0), (true, 3.0), (false, 6.0), (false, -1.0), (true, f64::NAN)] {
            assert!(g.doc.transact("bad", false, |m, _| import_grid_line(m, gw, StoredGridLine { const_v, param })).value.is_none(), "{const_v} {param}");
        }
    }

    #[test]
    fn spec_cw03_the_panels_are_the_cells_of_the_lines() {
        // no line: one panel; n U lines and m V lines: (n + 1)(m + 1) cells
        let mut f = fixture(6.0);
        let w = f.wall;
        assert_eq!(grid_state(f.doc.model(), w).unwrap().panels.len(), 1);
        f.doc.transact("lines", false, |m, cx| {
            for z in [0.5, 1.0] {
                add_grid_line(m, cx, w, true, Vec2::new(1.0, 0.0), z)?;
            }
            for x in [1.0, 2.0, 4.0] {
                add_grid_line(m, cx, w, false, Vec2::new(x, 0.0), 0.7)?;
            }
            Ok(())
        });
        let s = grid_state(f.doc.model(), w).unwrap();
        assert_eq!((s.u_lines.len(), s.v_lines.len(), s.panels.len()), (2, 3, 12));
        assert!((s.panels.iter().map(|p| p.area).sum::<f64>() - 18.0).abs() < 1e-9, "the cells tile the wall");
    }

    #[test]
    fn spec_cw05_a_removed_stretch_is_bounded_by_crossing_lines_stays_removed_on_both_sides_of_a_new_line_and_ranges_merge() {
        let mut f = fixture(6.0);
        let w = f.wall;
        assert!(f.doc.transact("u", false, |m, cx| add_grid_line(m, cx, w, true, Vec2::new(1.0, 0.0), 1.5)).committed());
        // the only segment of the line, from border to border
        assert!(f.doc.transact("rm", false, |m, cx| remove_segment(m, cx, w, true, 0, 0)).committed());
        assert_eq!(f.doc.model().curtain_grids[&w].removed[0].span, Some((0.0, 6.0)));
        let s = grid_state(f.doc.model(), w).unwrap();
        assert!(s.u_lines[0].empty_grid_warning(), "(4) removed segments and no remaining segment");
        // (3) a crossing line added later splits the removed stretch in two: BOTH halves stay removed
        assert!(f.doc.transact("v", false, |m, cx| add_grid_line(m, cx, w, false, Vec2::new(3.0, 0.0), 0.5)).committed());
        let s = grid_state(f.doc.model(), w).unwrap();
        assert_eq!((s.u_lines[0].segments, s.u_lines[0].skipped), (0, 2));
        assert_eq!(s.panels.len(), 4);
        assert_eq!(s.panels.iter().filter(|p| p.empty).count(), 2);
        assert!(s.panels.iter().filter(|p| !p.empty).all(|p| (p.area - 9.0).abs() < 1e-9), "each side's two cells are one panel");
        // (2) the same stretch stored again and a stored stretch inside it merge into the one range: nothing is counted twice
        let owner = s.u_lines[0].stored();
        assert!(f.doc.transact("imp", false, |m, _| import_skipped_segment(m, w, owner, None, None)).committed());
        assert!(f.doc.transact("imp2", false, |m, _| import_skipped_segment(m, w, owner, Some(3.0), Some(0.0))).committed());
        let s = grid_state(f.doc.model(), w).unwrap();
        assert_eq!((s.u_lines[0].segments, s.u_lines[0].skipped), (0, 2));
        assert!(f.doc.transact("none", false, |m, _| import_skipped_segment(m, w, owner, Some(2.0), Some(2.0))).value.is_none());
        // a stored stretch ends at the parameters of its bounding lines, ordered: only the left cell of a fresh wall's line is removed
        let mut g = fixture(6.0);
        let gw = g.wall;
        g.doc.transact("lines", false, |m, cx| {
            add_grid_line(m, cx, gw, true, Vec2::new(1.0, 0.0), 1.5)?;
            add_grid_line(m, cx, gw, false, Vec2::new(3.0, 0.0), 0.5)
        });
        let owner = grid_state(g.doc.model(), gw).unwrap().u_lines[0].stored();
        assert!(g.doc.transact("imp", false, |m, _| import_skipped_segment(m, gw, owner, Some(3.0), None)).committed());
        let s = grid_state(g.doc.model(), gw).unwrap();
        assert_eq!((s.u_lines[0].segments, s.u_lines[0].skipped), (1, 1));
        assert!(!s.u_lines[0].empty_grid_warning());
    }

    #[test]
    fn spec_cw06_an_edge_takes_a_corner_mullion_only_when_both_distances_are_non_negative() {
        assert!(edge_may_take_corner_mullion(0.0, 0.0) && edge_may_take_corner_mullion(2.5, 0.3) && edge_may_take_corner_mullion(-1e-10, 1.0));
        assert!(!edge_may_take_corner_mullion(-0.01, 1.0) && !edge_may_take_corner_mullion(1.0, -0.01) && !edge_may_take_corner_mullion(-1.0, -1.0));
    }

    #[test]
    fn mullion_boxes_areas_directions_and_the_cut_back_of_the_horizontal_ones_follow_the_measured_cases() {
        // OBSERVED: a vertical mullion runs its whole segment, a horizontal one stops at the face of the vertical one; the
        // Area is half the surface; the left border runs downward and lies inside the wall edge
        let mut f = fixture(6.0);
        let (wt, w) = (f.wall_type, f.wall);
        let (big, sq) = f
            .doc
            .transact("mt", false, |m, cx| Ok((create_mullion_type(m, cx, "50mm x 150mm"), create_mullion_type(m, cx, "30mm Square"))))
            .value
            .unwrap();
        f.doc.transact("lines", false, |m, cx| {
            add_grid_line(m, cx, w, false, Vec2::new(3.0, 0.0), 1.5)?;
            add_grid_line(m, cx, w, true, Vec2::new(3.0, 0.0), 1.5)?;
            add_mullions(m, cx, w, false, 0, big)?;
            add_mullions(m, cx, w, true, 0, big)?;
            m.curtain_types.get_mut(&wt).unwrap().vertical.border1 = Some(sq);
            Ok(())
        });
        let s = grid_state(f.doc.model(), w).unwrap();
        let r3 = |v: f64| (v * 1e4).round() / 1e4;
        // 2 vertical halves + 2 horizontal halves + the left border in 2 pieces
        assert_eq!(s.mullions.len(), 6);
        let v = s.mullions.iter().find(|x| x.type_name == "50mm x 150mm" && x.start[2] == 0.0 && x.end[2] == 1.5 && x.start[0] == 3.0).expect("lower half of the vertical");
        let (lo, hi) = v.bbox.unwrap();
        assert_eq!((r3(lo[0]), r3(hi[0]), r3(lo[1]), r3(hi[1]), r3(lo[2]), r3(hi[2])), (2.975, 3.025, -0.075, 0.075, 0.0, 1.5));
        assert!((v.area.unwrap() - 0.3075).abs() < 1e-9, "(w + t) L + w t = 0.2 x 1.5 + 0.0075");
        let h = s.mullions.iter().find(|x| x.type_name == "50mm x 150mm" && x.start[2] == 1.5 && x.start[0] > 2.9 && x.start[0] < 3.0).expect("left half of the horizontal");
        assert_eq!((r3(h.start[0]), r3(h.end[0])), (2.975, 0.03), "right to left, from the face of the vertical mullion to the face of the border");
        let (lo, hi) = h.bbox.unwrap();
        assert_eq!((r3(lo[2]), r3(hi[2])), (1.475, 1.525));
        assert!((h.area.unwrap() - (0.2 * 2.945 + 0.0075)).abs() < 1e-9);
        let b = s.mullions.iter().find(|x| x.type_name == "30mm Square" && x.end[2] == 0.0).expect("a piece of the left border");
        assert_eq!((b.start[0], b.end[0], b.start[2] > b.end[2]), (0.0, 0.0, true), "the location is the edge, running downward");
        let (lo, hi) = b.bbox.unwrap();
        assert_eq!((r3(lo[0]), r3(hi[0])), (0.0, 0.03), "the box lies inside the wall edge");
        // the order the harness lists them in: by type, then the box minimum
        let keys: Vec<(String, i64)> = s.mullions.iter().map(|x| (x.type_name.clone(), (x.bbox.unwrap().0[0] * 1000.0).round() as i64)).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
        // a circular section: Area = pi r (L + r); the box keeps the radius (the reference product's tessellated circle is
        // not derived)
        let round = MullionSize { side1: 0.05, side2: 0.05, half_thickness: 0.05, round: true };
        assert!((round.area(3.0) - 0.4790).abs() < 1e-3, "OBSERVED 0.4791 for the 50 mm radius over 3 m");
        assert!((template_size("25mm radius").unwrap().area(3.0) - 3.4906).abs() < 1e-3);
        // a type of unknown size has no box and no area
        let odd = f.doc.transact("odd", false, |m, cx| Ok(create_mullion_type(m, cx, "AST-ODD"))).value.unwrap();
        assert!(f.doc.transact("odd", false, |m, cx| {
            add_grid_line(m, cx, w, true, Vec2::new(1.0, 0.0), 2.5)?;
            add_mullions(m, cx, w, true, 1, odd)
        })
        .committed());
        let s = grid_state(f.doc.model(), w).unwrap();
        assert!(s.mullions.iter().any(|x| x.bbox.is_none() && x.area.is_none()));
    }

    #[test]
    fn a_curtain_wall_moves_and_copies_with_its_grid_and_deleting_it_drops_the_grid() {
        use crate::transform::{copy_elements, apply_transform_to_elements, PlanTransform};
        let mut f = fixture(6.0);
        let w = f.wall;
        assert!(f.doc.transact("u", false, |m, cx| add_grid_line(m, cx, w, true, Vec2::new(2.0, 0.0), 1.2)).committed());
        let t = PlanTransform::Translate(Vec2::new(0.0, 2.0));
        assert!(f.doc.transact("move", false, |m, cx| apply_transform_to_elements(m, cx, &[w], t)).committed());
        let s = grid_state(f.doc.model(), w).unwrap();
        let (a, _) = s.line_curve(f.doc.model(), w, &s.u_lines[0]).unwrap();
        assert_eq!(a, [0.0, 2.0, 1.2]);
        let pairs = f.doc.transact("copy", false, |m, cx| copy_elements(m, cx, &[w], Some(t))).value.expect("copied");
        let copy = pairs[0].1;
        let s = grid_state(f.doc.model(), copy).unwrap();
        assert_eq!((s.u_lines.len(), s.panels.len()), (1, 2));
        assert!(f.doc.transact("del", false, |m, cx| ops::delete(m, cx, w)).committed());
        assert!(f.doc.model().curtain_grids.get(&w).is_none());
        assert!(f.doc.model().curtain_grids.get(&copy).is_some());
    }
}
