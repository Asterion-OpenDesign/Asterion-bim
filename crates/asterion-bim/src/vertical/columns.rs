//! Architectural columns.
//!
//! Every rule here is OBSERVED through the public API (and `-063`, a column family made from the stock `Metric
//! Column.rft`); what the observations do not say is a named ENGINE CHOICE (question ids below). Rules specified since:
//! Spec COL-01 (the reference product: an ARCHITECTURAL column is a room-bounding element type 3, a 2D bounding type only
//! when the area settings enable it, and bounds a room only while its own Room Bounding parameter is on), Spec COL-02 (the
//! reference product: a column is a structural-type-3 instance without a curve driver; Base Offset -1001357, Top Offset
//! -1001358, base level -1001350, top level -1001351; Moves With Grids is stored for the column categories; the
//! ColumnAttachment record holds the offset, base-or-top, cut style and justification) and (the reference product: columns
//! and walls are auto-joinable; the COLUMN is the cut element and keeps the wall intact - Boolean = copy of the wall -
//! while the pair's clean step turns the overlapping edge segments into edges of the wall: its counts are the reference
//! product and not specified). The columns of the harness have no geometry (the template's family is empty: volume 0, a
//! plan-only bounding box), so the engine models the column as a prism between its base and top and the comparison records
//! its solid and box without comparing them.
//!
//! * **Type.** A column type has a width and a depth; one made from the template reads 0.6 x 0.6 m
//!   (OBSERVED: `Width` and `Depth` 600 mm). A type's size is a plain length parameter.
//! * **Instance.** A column stands at a plan point with a rotation (`LocationPoint.Rotation`,
//!   counter-clockwise, read in [0, 2 pi)), a base level and offset, a top level and offset, the
//!   flags Moves With Grids (default yes) and Room Bounding (default yes). Its location's z is the
//!   elevation of its level at the moment it is placed and does not follow later changes of the base
//!   level (OBSERVED: a column re-based on L1 keeps z = 0).
//! * **Default top.** A new column's top level is the template's "Level 2" (OBSERVED: a level the document did not define).
//!   The engine has no template levels: its default top is NO level, an unconnected column of
//!   [`DEFAULT_UNCONNECTED_HEIGHT_M`].
//! * **Transforms.** Move, rotate, mirror and copy act like any plan element: the location moves, a rotation adds its
//!   angle, a mirror about a line of direction `phi` maps the rotation `theta` to `2 phi - theta` (OBSERVED: 45 degrees
//!   about a vertical line -> 135 degrees). A grid moved under a column that moves with grids does NOT take the column with
//!   it (OBSERVED), whatever the flag says.
//! * **Attachment.** `ColumnAttachment` ties the top (or base) of a column to a floor, roof or
//!   ceiling: the top offset becomes the target's underside minus the top level's elevation plus the
//!   attachment offset (OBSERVED: a floor of 0.2 on the top level: -0.2, with offset 0.1: -0.1; the
//!   justification made no difference for the flat floor of the probe). Removing the attachment
//!   leaves the offsets as they were, and deleting the target drops the attachment and leaves the
//!   offsets (OBSERVED). The base end is symmetric and ASSUMED (not measured).
//! * **Walls.** A column standing on a wall crossing or a wall end changes nothing in the walls'
//!   area, volume or edges (OBSERVED).

use crate::model::BimModel;
use crate::spec::codes;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use asterion_geom::Vec2;
use serde::{Deserialize, Serialize};

/// OBSERVED: a column type made from the template reads Width = Depth = 600 mm.
pub const DEFAULT_COLUMN_SIZE_M: f64 = 0.6;

/// ENGINE CHOICE: the height of a column with no top level.
pub const DEFAULT_UNCONNECTED_HEIGHT_M: f64 = 4.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColumnType {
    pub id: ElementId,
    pub name: String,
    pub width: f64,
    pub depth: f64,
}

/// (OBSERVED names: `CutColumn`, `CutTarget`, `None`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachCutStyle {
    None,
    CutColumn,
    CutTarget,
}

impl AttachCutStyle {
    pub fn name(&self) -> &'static str {
        match self {
            AttachCutStyle::None => "None",
            AttachCutStyle::CutColumn => "CutColumn",
            AttachCutStyle::CutTarget => "CutTarget",
        }
    }

    pub fn parse(s: &str) -> Option<AttachCutStyle> {
        match s.to_ascii_lowercase().as_str() {
            "none" => Some(AttachCutStyle::None),
            "cutcolumn" | "cut_column" => Some(AttachCutStyle::CutColumn),
            "cuttarget" | "cut_target" => Some(AttachCutStyle::CutTarget),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachJustification {
    Minimum,
    Midpoint,
    Maximum,
    Tangent,
}

impl AttachJustification {
    pub fn name(&self) -> &'static str {
        match self {
            AttachJustification::Minimum => "Minimum",
            AttachJustification::Midpoint => "Midpoint",
            AttachJustification::Maximum => "Maximum",
            AttachJustification::Tangent => "Tangent",
        }
    }

    pub fn parse(s: &str) -> Option<AttachJustification> {
        match s.to_ascii_lowercase().as_str() {
            "minimum" => Some(AttachJustification::Minimum),
            "midpoint" => Some(AttachJustification::Midpoint),
            "maximum" => Some(AttachJustification::Maximum),
            "tangent" => Some(AttachJustification::Tangent),
            _ => None,
        }
    }
}

/// Spec COL-02 (`ColumnAttachment`): the record holds the offset (a double), the base-or-top selector, the cut style and
/// the justification; the numeric meaning of the cut style and justification enums is not specified, the engine keeps the
/// names the API shows.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColumnAttachment {
    pub target: ElementId,
    pub cut_style: AttachCutStyle,
    pub justification: AttachJustification,
    /// Metres.
    pub offset: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub id: ElementId,
    pub type_id: ElementId,
    pub base_level: ElementId,
    pub base_offset: f64,
    /// `None`: no top level (unconnected, [`DEFAULT_UNCONNECTED_HEIGHT_M`] high).
    pub top_level: Option<ElementId>,
    pub top_offset: f64,
    pub location: Vec2,
    /// The z of the location point: the base level's elevation when the column was placed.
    pub location_z: f64,
    /// Counter-clockwise, in [0, 2 pi).
    pub rotation: f64,
    /// Spec COL-02: the stored flag `Moves With Grids` (the parameter exists for the column categories only; an
    /// architectural column always has it).
    pub moves_with_grids: bool,
    /// Spec COL-01: the instance parameter `Room Bounding`; a column with it off never bounds a room.
    pub room_bounding: bool,
    /// The stored HAND flip.
    #[serde(default)]
    pub hand_flipped: bool,
    #[serde(default)]
    pub facing_flipped: bool,
    pub base_attachment: Option<ColumnAttachment>,
    pub top_attachment: Option<ColumnAttachment>,
}

impl Column {
    /// The instance is mirrored exactly when ONE of the hand and facing flips holds (both flipped is a half turn, not a
    /// mirror).
    pub fn mirrored(&self) -> bool {
        self.facing_flipped ^ self.hand_flipped
    }

    /// Spec COL-01 + (3): an architectural column is a 2D room-bounding element only when its own Room Bounding parameter
    /// is on AND the document's area settings enable the column classification.
    pub fn bounds_rooms(&self, area_settings_enable_columns: bool) -> bool {
        self.room_bounding && area_settings_enable_columns
    }

    /// OBSERVED (placements incl. mirrored ones).
    pub fn hand_orientation(&self) -> Vec2 {
        let v = Vec2::new(self.rotation.cos(), self.rotation.sin());
        if self.hand_flipped {
            -v
        } else {
            v
        }
    }

    /// OBSERVED: `FacingOrientation` is the section's y axis `(-sin t, cos t)`, reversed by the facing flip: a mirrored
    /// column faces the other way and keeps its hand vector.
    pub fn facing_orientation(&self) -> Vec2 {
        let v = Vec2::new(-self.rotation.sin(), self.rotation.cos());
        if self.facing_flipped {
            -v
        } else {
            v
        }
    }
}

/// ENGINE CHOICE: the area-settings switch of Spec COL-01 for the column classification, enabled.
pub const AREA_SETTINGS_COLUMNS_ENABLED: bool = true;


/// The regenerated column: a pure function of the model.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnState {
    pub base_z: f64,
    pub top_z: f64,
    pub height: f64,
    /// The column length parameter (-1001375) = |(top level elevation + top offset) - (base level elevation + base offset)|
    /// for a column that is not curve driven; `height` is the signed value the volume uses.
    pub length: f64,
    pub width: f64,
    pub depth: f64,
    /// The four corners of the plan rectangle, counter-clockwise, rotated about the location.
    pub footprint: Vec<Vec2>,
    pub plan_area: f64,
    /// Width x depth x height (0 when the height is not positive).
    pub volume: f64,
}

/// A rotation read in [0, 2 pi) (OBSERVED: `LocationPoint.Rotation`).
pub fn normalize_angle(a: f64) -> f64 {
    let two_pi = std::f64::consts::TAU;
    let mut r = a % two_pi;
    if r < 0.0 {
        r += two_pi;
    }
    // a rotation within rounding of a full turn is zero
    if (two_pi - r).abs() < 1e-12 {
        0.0
    } else {
        r
    }
}

fn missing(id: ElementId, what: &str) -> Failure {
    Failure::error(codes::NOT_FOUND, format!("Element {} is not a {what}.", id.0)).on(&[id])
}

fn bad_size(what: &str, v: f64) -> Failure {
    Failure::error(codes::COLUMN_BAD_SIZE, format!("The column type {what} must be a positive length, got {v}."))
}

impl BimModel {
    pub fn column(&self, id: ElementId) -> Result<&Column, Failure> {
        self.columns.get(&id).ok_or_else(|| missing(id, "column"))
    }

    pub fn column_type(&self, id: ElementId) -> Result<&ColumnType, Failure> {
        self.column_types.get(&id).ok_or_else(|| missing(id, "column type"))
    }
}

/// Create a column type (`width` and `depth` default to 0.6 m when `None`: OBSERVED for a type made
/// from the template).
pub fn create_column_type(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    name: &str,
    width: Option<f64>,
    depth: Option<f64>,
) -> Result<ElementId, Failure> {
    let (w, d) = (width.unwrap_or(DEFAULT_COLUMN_SIZE_M), depth.unwrap_or(DEFAULT_COLUMN_SIZE_M));
    if !(w.is_finite() && w > 0.0) {
        return Err(bad_size("width", w));
    }
    if !(d.is_finite() && d > 0.0) {
        return Err(bad_size("depth", d));
    }
    if m.column_types.values().any(|t| t.name == name) {
        return Err(Failure::error(codes::COLUMN_DUPLICATE_TYPE, "A column type with this name already exists."));
    }
    let id = cx.new_id();
    m.column_types.insert(id, ColumnType { id, name: name.to_owned(), width: w, depth: d });
    Ok(id)
}

/// Set the width and / or the depth of a column type.
pub fn set_column_type_size(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    id: ElementId,
    width: Option<f64>,
    depth: Option<f64>,
) -> Result<(), Failure> {
    m.column_type(id)?;
    for (label, v) in [("width", width), ("depth", depth)] {
        if let Some(v) = v {
            if !(v.is_finite() && v > 0.0) {
                return Err(bad_size(label, v));
            }
        }
    }
    let t = m.column_types.get_mut(&id).expect("checked");
    if let Some(w) = width {
        t.width = w;
    }
    if let Some(d) = depth {
        t.depth = d;
    }
    Ok(())
}

/// Place a column of `type_id` at `point` on `level`, rotated by `rotation` (radians, counter-
/// clockwise). The base level is `level` with offset 0, the top level none (see the module doc).
pub fn place_column(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    type_id: ElementId,
    level: ElementId,
    point: Vec2,
    rotation: f64,
) -> Result<ElementId, Failure> {
    m.column_type(type_id)?;
    let elevation = m.level(level)?.elevation;
    if !(point.x.is_finite() && point.y.is_finite() && rotation.is_finite()) {
        return Err(Failure::error(codes::INVALID_BOUNDARY, "A column's location and rotation must be finite numbers."));
    }
    let id = cx.new_id();
    m.columns.insert(
        id,
        Column {
            id,
            type_id,
            base_level: level,
            base_offset: 0.0,
            top_level: None,
            top_offset: 0.0,
            location: point,
            location_z: elevation,
            rotation: normalize_angle(rotation),
            moves_with_grids: true,
            room_bounding: true,
            hand_flipped: false,
            facing_flipped: false,
            base_attachment: None,
            top_attachment: None,
        },
    );
    Ok(id)
}

pub fn set_column_base_level(m: &mut BimModel, _cx: &mut TxnContext<'_>, id: ElementId, level: ElementId) -> Result<(), Failure> {
    m.column(id)?;
    m.level(level)?;
    m.columns.get_mut(&id).expect("checked").base_level = level;
    Ok(())
}

pub fn set_column_top_level(m: &mut BimModel, _cx: &mut TxnContext<'_>, id: ElementId, level: Option<ElementId>) -> Result<(), Failure> {
    m.column(id)?;
    if let Some(l) = level {
        m.level(l)?;
    }
    m.columns.get_mut(&id).expect("checked").top_level = level;
    Ok(())
}

pub fn set_column_offsets(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    id: ElementId,
    base: Option<f64>,
    top: Option<f64>,
) -> Result<(), Failure> {
    m.column(id)?;
    for v in [base, top].into_iter().flatten() {
        if !v.is_finite() {
            return Err(Failure::error(codes::COLUMN_BAD_SIZE, "A column offset must be a finite number."));
        }
    }
    let c = m.columns.get_mut(&id).expect("checked");
    if let Some(b) = base {
        c.base_offset = b;
    }
    if let Some(t) = top {
        c.top_offset = t;
    }
    Ok(())
}

/// A plan edit mirrors through the transform instead.
pub fn set_column_flips(m: &mut BimModel, id: ElementId, hand_flipped: bool, facing_flipped: bool) -> Result<(), Failure> {
    m.column(id)?;
    let c = m.columns.get_mut(&id).expect("checked");
    c.hand_flipped = hand_flipped;
    c.facing_flipped = facing_flipped;
    Ok(())
}

pub fn set_column_flags(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    id: ElementId,
    moves_with_grids: Option<bool>,
    room_bounding: Option<bool>,
) -> Result<(), Failure> {
    m.column(id)?;
    let c = m.columns.get_mut(&id).expect("checked");
    if let Some(v) = moves_with_grids {
        c.moves_with_grids = v;
    }
    if let Some(v) = room_bounding {
        c.room_bounding = v;
    }
    Ok(())
}

/// `ColumnAttachment.AddColumnAttachment`: attach the top (`top`) or the base of a column to a
/// floor, roof or ceiling. An end that already has an attachment is left alone (the API: "no
/// attachment is made"). See the module doc for the offset it writes.
pub fn attach_column(
    m: &mut BimModel,
    _cx: &mut TxnContext<'_>,
    id: ElementId,
    target: ElementId,
    top: bool,
    cut_style: AttachCutStyle,
    justification: AttachJustification,
    offset: f64,
) -> Result<(), Failure> {
    let col = m.column(id)?.clone();
    if !offset.is_finite() {
        return Err(Failure::error(codes::COLUMN_BAD_ATTACHMENT, "The attachment offset must be a finite number.").on(&[id]));
    }
    let Some(slab) = m.floors.get(&target) else {
        return Err(Failure::error(codes::COLUMN_BAD_ATTACHMENT, "A column can be attached to a floor, roof or ceiling.").on(&[id, target]));
    };
    if (top && col.top_attachment.is_some()) || (!top && col.base_attachment.is_some()) {
        return Ok(());
    }
    let state = crate::regen::slab_state(m, slab)?;
    let attachment = ColumnAttachment { target, cut_style, justification, offset };
    let slab_level = slab.level;
    let c = m.columns.get_mut(&id).expect("checked");
    if top {
        // the top level the offset is measured from: the column's own, else the target's level
        let level = c.top_level.unwrap_or(slab_level);
        c.top_level = Some(level);
        let elevation = m.levels[&level].elevation;
        let c = m.columns.get_mut(&id).expect("checked");
        c.top_offset = (state.bottom_z - elevation) + offset;
        c.top_attachment = Some(attachment);
    } else {
        let elevation = m.levels[&c.base_level].elevation;
        let c = m.columns.get_mut(&id).expect("checked");
        // ASSUMED (the base end was not measured): the column starts on the target's top face
        c.base_offset = (state.top_z - elevation) + offset;
        c.base_attachment = Some(attachment);
    }
    Ok(())
}

/// `ColumnAttachment.RemoveColumnAttachment` at the top or the base: the attachment goes, the
/// offsets stay (OBSERVED).
pub fn remove_column_attachment(m: &mut BimModel, _cx: &mut TxnContext<'_>, id: ElementId, top: bool) -> Result<(), Failure> {
    m.column(id)?;
    let c = m.columns.get_mut(&id).expect("checked");
    if top {
        c.top_attachment = None;
    } else {
        c.base_attachment = None;
    }
    Ok(())
}

/// The regenerated column (see the module doc). Spec COL-02: the base elevation is the elevation of the base level plus the
/// Base Offset (reference 0) and the top elevation the elevation of the top level plus the Top Offset (reference 1); both
/// references may name the SAME level (OBSERVED: base and top level equal, offsets -0.656 ft and 1.640 ft).
pub fn column_state(model: &BimModel, c: &Column) -> Result<ColumnState, Failure> {
    let t = model.column_type(c.type_id)?;
    let base_z = model.level(c.base_level)?.elevation + c.base_offset;
    let top_z = match c.top_level {
        Some(l) => model.level(l)?.elevation + c.top_offset,
        None => base_z + DEFAULT_UNCONNECTED_HEIGHT_M + c.top_offset,
    };
    let height = top_z - base_z;
    let (hw, hd) = (t.width / 2.0, t.depth / 2.0);
    let (cos, sin) = (c.rotation.cos(), c.rotation.sin());
    let corner = |x: f64, y: f64| c.location + Vec2::new(x * cos - y * sin, x * sin + y * cos);
    let footprint = vec![corner(-hw, -hd), corner(hw, -hd), corner(hw, hd), corner(-hw, hd)];
    Ok(ColumnState {
        base_z,
        top_z,
        height,
        length: height.abs(),
        width: t.width,
        depth: t.depth,
        footprint,
        plan_area: t.width * t.depth,
        volume: if height > 0.0 { t.width * t.depth * height } else { 0.0 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ops, BimDocument, CompoundStructure, Layer, LayerFunction};

    struct Fx {
        doc: BimDocument,
        l0: ElementId,
        l1: ElementId,
        ty: ElementId,
    }

    fn fixture() -> Fx {
        let mut doc = crate::new_document();
        let (l0, l1, ty) = doc
            .transact("fx", false, |m, cx| {
                let l0 = ops::create_level(m, cx, "L0", 0.0)?;
                let l1 = ops::create_level(m, cx, "L1", 3.0)?;
                let ty = create_column_type(m, cx, "C", None, None)?;
                Ok((l0, l1, ty))
            })
            .value
            .expect("fixture");
        Fx { doc, l0, l1, ty }
    }

    #[test]
    fn a_column_type_defaults_to_600_and_takes_its_size_by_parameter() {
        let mut f = fixture();
        assert_eq!((f.doc.model().column_types[&f.ty].width, f.doc.model().column_types[&f.ty].depth), (0.6, 0.6));
        let out = f.doc.transact("size", false, |m, cx| set_column_type_size(m, cx, f.ty, Some(0.4), Some(0.3)));
        assert!(out.committed());
        let ty = f.ty;
        for (w, d) in [(Some(0.0), None), (None, Some(-1.0)), (Some(f64::NAN), None)] {
            assert!(f.doc.transact("bad", false, |m, cx| set_column_type_size(m, cx, ty, w, d)).value.is_none());
        }
        assert!(f.doc.transact("dup", false, |m, cx| create_column_type(m, cx, "C", None, None)).value.is_none());
    }

    #[test]
    fn a_placed_column_has_the_observed_defaults_and_a_rotated_footprint() {
        let mut f = fixture();
        let (l0, ty) = (f.l0, f.ty);
        let id = f
            .doc
            .transact("c", false, |m, cx| {
                set_column_type_size(m, cx, ty, Some(0.4), Some(0.3))?;
                place_column(m, cx, ty, l0, Vec2::new(2.0, 2.0), std::f64::consts::FRAC_PI_4 + std::f64::consts::TAU)
            })
            .value
            .expect("placed");
        let c = f.doc.model().column(id).unwrap().clone();
        assert!(c.moves_with_grids && c.room_bounding && c.top_level.is_none());
        assert!((c.rotation - std::f64::consts::FRAC_PI_4).abs() < 1e-12, "{}", c.rotation);
        let s = column_state(f.doc.model(), &c).unwrap();
        assert_eq!(s.plan_area, 0.4 * 0.3);
        // the rectangle's centre is the location, whatever the rotation
        let mid = s.footprint.iter().fold(Vec2::new(0.0, 0.0), |a, p| a + *p) * 0.25;
        assert!((mid - Vec2::new(2.0, 2.0)).length() < 1e-12);
        assert_eq!(s.height, DEFAULT_UNCONNECTED_HEIGHT_M);
        // a top level and offsets move the top and the base
        let l1 = f.l1;
        assert!(f.doc.transact("top", false, |m, cx| set_column_top_level(m, cx, id, Some(l1))).committed());
        assert!(f.doc.transact("off", false, |m, cx| set_column_offsets(m, cx, id, Some(-0.2), Some(0.5))).committed());
        let s = column_state(f.doc.model(), f.doc.model().column(id).unwrap()).unwrap();
        assert_eq!((s.base_z, s.top_z), (-0.2, 3.5));
        assert!((s.volume - 0.4 * 0.3 * 3.7).abs() < 1e-12);
        // re-basing keeps the location's z (OBSERVED)
        assert!(f.doc.transact("base", false, |m, cx| set_column_base_level(m, cx, id, l1)).committed());
        assert_eq!(f.doc.model().column(id).unwrap().location_z, 0.0);
    }

    #[test]
    fn an_attachment_writes_the_offset_from_the_underside_and_removal_keeps_it() {
        let mut f = fixture();
        let (l0, l1, ty) = (f.l0, f.l1, f.ty);
        let (a, b, floor) = f
            .doc
            .transact("c", false, |m, cx| {
                let ft = ops::create_floor_type(m, cx, "F", 0.2)?;
                let v = Vec2::new;
                let floor = ops::create_slab(m, cx, ft, l1, vec![v(6.0, 6.0), v(12.0, 6.0), v(12.0, 12.0), v(6.0, 12.0)], 0.0, false)?;
                let a = place_column(m, cx, ty, l0, v(8.0, 8.0), 0.0)?;
                let b = place_column(m, cx, ty, l0, v(10.0, 10.0), 0.0)?;
                set_column_top_level(m, cx, a, Some(l1))?;
                set_column_top_level(m, cx, b, Some(l1))?;
                Ok((a, b, floor))
            })
            .value
            .expect("fixture");
        let out = f.doc.transact("att", false, |m, cx| attach_column(m, cx, a, floor, true, AttachCutStyle::None, AttachJustification::Minimum, 0.0));
        assert!(out.committed(), "{:?}", out.failures);
        assert!((f.doc.model().column(a).unwrap().top_offset + 0.2).abs() < 1e-12);
        let out = f.doc.transact("att", false, |m, cx| attach_column(m, cx, b, floor, true, AttachCutStyle::None, AttachJustification::Maximum, 0.1));
        assert!(out.committed());
        assert!((f.doc.model().column(b).unwrap().top_offset + 0.1).abs() < 1e-12);
        // a second attachment at the same end changes nothing
        assert!(f.doc.transact("again", false, |m, cx| attach_column(m, cx, a, floor, true, AttachCutStyle::None, AttachJustification::Minimum, 0.5)).committed());
        assert!((f.doc.model().column(a).unwrap().top_offset + 0.2).abs() < 1e-12);
        // removal leaves the offset; deleting the target drops the attachment and leaves the offset
        assert!(f.doc.transact("rm", false, |m, cx| remove_column_attachment(m, cx, a, true)).committed());
        assert!(f.doc.model().column(a).unwrap().top_attachment.is_none());
        assert!((f.doc.model().column(a).unwrap().top_offset + 0.2).abs() < 1e-12);
        assert!(f.doc.transact("del", false, |m, cx| ops::delete(m, cx, floor)).committed());
        assert!(f.doc.model().column(b).unwrap().top_attachment.is_none());
        assert!((f.doc.model().column(b).unwrap().top_offset + 0.1).abs() < 1e-12);
        // a wall is not a target
        let bad = f.doc.transact("bad", false, |m, cx| attach_column(m, cx, a, ElementId(9_999_999), true, AttachCutStyle::None, AttachJustification::Minimum, 0.0));
        assert!(bad.value.is_none());
    }

    #[test]
    fn move_rotate_mirror_and_copy_act_on_the_location_and_the_rotation() {
        use crate::transform::{copy_elements, apply_transform_to_elements, PlanTransform};
        let mut f = fixture();
        let (l0, ty) = (f.l0, f.ty);
        let id = f
            .doc
            .transact("c", false, |m, cx| place_column(m, cx, ty, l0, Vec2::new(6.0, 2.0), std::f64::consts::FRAC_PI_4))
            .value
            .expect("placed");
        // a mirror about the vertical line x = 10: 45 degrees -> 135 degrees, 6 -> 14 (OBSERVED)
        let mirror = PlanTransform::Mirror { a: Vec2::new(10.0, -5.0), b: Vec2::new(10.0, 5.0) };
        assert!(f.doc.transact("mirror", false, |m, cx| apply_transform_to_elements(m, cx, &[id], mirror)).committed());
        let c = f.doc.model().column(id).unwrap();
        assert!((c.location - Vec2::new(14.0, 2.0)).length() < 1e-9);
        assert!((c.rotation - 3.0 * std::f64::consts::FRAC_PI_4).abs() < 1e-9, "{}", c.rotation);
        // a quarter turn about (10, 0) adds its angle
        let rot = PlanTransform::Rotate { center: Vec2::new(10.0, 0.0), angle: std::f64::consts::FRAC_PI_2 };
        assert!(f.doc.transact("rotate", false, |m, cx| apply_transform_to_elements(m, cx, &[id], rot)).committed());
        let c = f.doc.model().column(id).unwrap();
        assert!((c.location - Vec2::new(8.0, 4.0)).length() < 1e-9, "{:?}", c.location);
        assert!((c.rotation - 5.0 * std::f64::consts::FRAC_PI_4).abs() < 1e-9);
        // a copy is a column of its own at the moved place; a pinned column is not moved
        let t = PlanTransform::Translate(Vec2::new(0.0, 5.0));
        let pairs = f.doc.transact("copy", false, |m, cx| copy_elements(m, cx, &[id], Some(t))).value.expect("copied");
        let copy = pairs[0].1;
        assert!((f.doc.model().column(copy).unwrap().location - Vec2::new(8.0, 9.0)).length() < 1e-9);
        assert!(f.doc.transact("pin", false, |m, cx| ops::set_pinned(m, cx, &[id], true)).committed());
        assert!(f.doc.transact("move", false, |m, cx| apply_transform_to_elements(m, cx, &[id], t)).value.is_none());
        assert!(!f.doc.model().is_pinned(copy));
    }

    #[test]
    fn rows_col_01_col_02_the_references_the_length_the_flags_and_the_mirror_state() {
        use crate::transform::{apply_transform_to_elements, PlanTransform};
        // Spec COL-02: base = base level + Base Offset, top = top level + Top Offset, the two may be the SAME level (the
        // stored column: both references on one level, offsets -0.656168 ft and 1.640420 ft)
        let mut f = fixture();
        let (l0, ty) = (f.l0, f.ty);
        let ft = 0.3048;
        let id = f
            .doc
            .transact("c", false, |m, cx| {
                let c = place_column(m, cx, ty, l0, Vec2::new(0.0, 0.0), 0.0)?;
                set_column_top_level(m, cx, c, Some(l0))?;
                set_column_offsets(m, cx, c, Some(-0.656167979 * ft), Some(1.640419948 * ft))?;
                Ok(c)
            })
            .value
            .expect("placed");
        let s = column_state(f.doc.model(), f.doc.model().column(id).unwrap()).unwrap();
        assert!((s.base_z + 0.2).abs() < 1e-8 && (s.top_z - 0.5).abs() < 1e-8, "{} {}", s.base_z, s.top_z);
        assert!((s.length - 0.7).abs() < 1e-8 && (s.height - 0.7).abs() < 1e-8);
        // The length parameter is the ABSOLUTE difference; the volume of an inverted column is 0
        assert!(f.doc.transact("inv", false, |m, cx| set_column_offsets(m, cx, id, Some(0.5), Some(-0.2))).committed());
        let s = column_state(f.doc.model(), f.doc.model().column(id).unwrap()).unwrap();
        assert!((s.length - 0.7).abs() < 1e-9 && s.height < 0.0 && s.volume == 0.0);
        // Spec COL-02: Moves With Grids is a stored flag, on for a new column, edited through the flags
        assert!(f.doc.model().column(id).unwrap().moves_with_grids);
        assert!(f.doc.transact("flag", false, |m, cx| set_column_flags(m, cx, id, Some(false), None)).committed());
        assert!(!f.doc.model().column(id).unwrap().moves_with_grids && f.doc.model().column(id).unwrap().room_bounding);
        // Spec COL-01: a column with Room Bounding off never bounds a room; with it on it does when the area settings
        // enable columns
        let c = f.doc.model().column(id).unwrap().clone();
        assert!(c.bounds_rooms(AREA_SETTINGS_COLUMNS_ENABLED) && !c.bounds_rooms(false));
        assert!(f.doc.transact("rb", false, |m, cx| set_column_flags(m, cx, id, None, Some(false))).committed());
        assert!(!f.doc.model().column(id).unwrap().bounds_rooms(true));
        // One mirror makes the column mirrored, a second one undoes it (hand and facing flips XOR)
        let mirror = PlanTransform::Mirror { a: Vec2::new(10.0, -5.0), b: Vec2::new(10.0, 5.0) };
        assert!(!f.doc.model().column(id).unwrap().mirrored());
        assert!(f.doc.transact("m1", false, |m, cx| apply_transform_to_elements(m, cx, &[id], mirror)).committed());
        assert!(f.doc.model().column(id).unwrap().mirrored());
        assert!(f.doc.transact("m2", false, |m, cx| apply_transform_to_elements(m, cx, &[id], mirror)).committed());
        assert!(!f.doc.model().column(id).unwrap().mirrored());
        // the vectors OBSERVED: after the first mirror (rotation 0 -> pi) the hand vector is (cos, sin) and the facing
        // reversed
        let c = f.doc.model().column(id).unwrap().clone();
        assert!((c.hand_orientation() - Vec2::new(c.rotation.cos(), c.rotation.sin())).length() < 1e-12);
        assert!((c.facing_orientation() - Vec2::new(-c.rotation.sin(), c.rotation.cos())).length() < 1e-12);
        let mut flipped = c.clone();
        flipped.facing_flipped = true;
        assert!((flipped.facing_orientation() + c.facing_orientation()).length() < 1e-12 && (flipped.hand_orientation() - c.hand_orientation()).length() < 1e-12);
        let mut both = f.doc.model().column(id).unwrap().clone();
        both.hand_flipped = true;
        both.facing_flipped = true;
        assert!(!both.mirrored(), "both flips is a half turn, not a mirror");
    }

    #[test]
    fn deleting_a_level_or_a_type_takes_its_columns_and_a_top_level_leaves_them_unconnected() {
        let mut f = fixture();
        let (l0, l1, ty) = (f.l0, f.l1, f.ty);
        let (a, b) = f
            .doc
            .transact("c", false, |m, cx| {
                let a = place_column(m, cx, ty, l0, Vec2::new(0.0, 0.0), 0.0)?;
                let b = place_column(m, cx, ty, l1, Vec2::new(5.0, 0.0), 0.0)?;
                set_column_top_level(m, cx, a, Some(l1))?;
                Ok((a, b))
            })
            .value
            .expect("fixture");
        assert!(f.doc.transact("del", false, |m, cx| ops::delete(m, cx, l1)).committed());
        assert!(f.doc.model().columns.get(&b).is_none(), "its base level is gone");
        assert_eq!(f.doc.model().column(a).unwrap().top_level, None, "its top level is gone: unconnected");
        assert!(f.doc.transact("type", false, |m, cx| ops::delete(m, cx, ty)).committed());
        assert!(f.doc.model().columns.is_empty());
    }
}
