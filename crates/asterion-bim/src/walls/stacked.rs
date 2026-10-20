//! Stacked walls: a wall type made of member wall types stacked bottom to top, one of them of variable height.
//!
//! Spec STK-01: the members are records bottom first (height, offset, member type, member index, flip); a member whose
//! height is below -1e-9 is the VARIABLE member; the minimum height of the wall is the sum of the fixed heights; the
//! boundary at the bottom of a member is the sum of the fixed heights below it, or, when the variable member lies below,
//! the wall height less the heights of the members above. Spec STK-02: member indices are non-negative and distinct, every
//! fixed height is at least 1e-9, exactly one member is variable, and the fixed heights add up to at most the type's sample
//! height. A stacked wall never receives a join step (its members do).
//!
//! OBSERVED: the template's one stacked type "Exterior - Brick Over Block w Metal Stud" has two members, "Exterior - Block
//! on Mtl. Stud" (0.46 wide, fixed 0.9 high, offset 0) under "Exterior - Brick on Mtl. Stud" (0.35 wide, variable, offset
//! -0.055); a 6 m wall has members of 0.9 and 5.1, a 4 m one 0.9 and 3.1, 1.0 gives 0.9 and 0.1. The template's content is
//! a measurement, not a row (ENGINE CHOICE); the member walls' solids and the joins of their ends are NOT modelled: the
//! owner is a wall of the type's width.

use crate::model::{BimModel, CompoundStructure, Layer, LayerFunction, LocationLine, Wall, WallType};
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use serde::{Deserialize, Serialize};

/// One member of a stacked wall type (Spec STK-01).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StackedMember {
    /// The member's wall type.
    pub type_id: ElementId,
    /// `None`: the VARIABLE member (the reference product stores a negative height).
    pub height: Option<f64>,
    pub offset: f64,
    pub flip: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StackedWallType {
    pub id: ElementId,
    pub name: String,
    pub location_line: LocationLine,
    /// Bottom first.
    pub members: Vec<StackedMember>,
    /// The sample height of the type: the fixed heights may add up to at most this (Spec STK-02).
    pub sample_height: f64,
}

const TINY: f64 = 1e-9;

/// Spec STK-01: the sum of the fixed heights and the index of the variable member (the last one found); `None` when no
/// member is variable (Err 1).
pub fn min_height(t: &StackedWallType) -> Option<(f64, usize)> {
    let mut sum = 0.0;
    let mut var = None;
    for (i, m) in t.members.iter().enumerate() {
        match m.height {
            Some(h) if h >= -TINY => sum += h,
            _ => var = Some(i),
        }
    }
    var.map(|v| (sum, v))
}

/// Spec STK-02.
pub fn is_well_formed(t: &StackedWallType) -> bool {
    let n = t.members.len();
    if n == 0 {
        return false;
    }
    let fixed_ok = t.members.iter().all(|m| m.height.map_or(true, |h| h >= TINY));
    let variable = t.members.iter().filter(|m| m.height.is_none()).count();
    let fixed_sum: f64 = t.members.iter().filter_map(|m| m.height).sum();
    fixed_ok && variable == 1 && fixed_sum <= t.sample_height + TINY
}

/// Why a boundary cannot be placed (the error numbers of Spec STK-01).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryError {
    /// Err 1: the wall is shorter than the minimum height.
    TooShort,
    /// Err 6: the wall is as high as the minimum and the member is the variable one (it would have no height).
    NoHeightLeft,
    /// Err 9: the boundary falls outside the wall or does not rise.
    NotRising,
    /// Err 4: the bottom member has no boundary below it.
    FirstMember,
    /// Err: no such member.
    NoSuchMember,
}

/// Spec STK-01: the height, above the base of the wall, of the boundary at the BOTTOM of `member` for a wall `wall_height`
/// high.
pub fn boundary(t: &StackedWallType, wall_height: f64, member: usize) -> Result<f64, BoundaryError> {
    let (min, var) = min_height(t).ok_or(BoundaryError::TooShort)?;
    if wall_height + TINY < min {
        return Err(BoundaryError::TooShort);
    }
    if wall_height - TINY < min && member == var {
        return Err(BoundaryError::NoHeightLeft);
    }
    if member >= t.members.len() {
        return Err(BoundaryError::NoSuchMember);
    }
    if member == 0 {
        return Err(BoundaryError::FirstMember);
    }
    let b = if var < member {
        // the variable member lies below: measured from the top, walking down from the top member
        wall_height - t.members[member..].iter().map(|m| m.height.unwrap_or(0.0)).sum::<f64>()
    } else {
        t.members[..member].iter().map(|m| m.height.unwrap_or(0.0)).sum()
    };
    let previous = if member >= 2 { boundary(t, wall_height, member - 1).unwrap_or(0.0) } else { 0.0 };
    if !(b > TINY && b <= wall_height - TINY && b > previous + TINY) {
        return Err(BoundaryError::NotRising);
    }
    Ok(b)
}

/// The heights of the members for a wall `wall_height` high (base of the first member at 0), bottom first.
pub fn member_heights(t: &StackedWallType, wall_height: f64) -> Result<Vec<(f64, f64)>, BoundaryError> {
    let n = t.members.len();
    let mut bounds = vec![0.0];
    for i in 1..n {
        bounds.push(boundary(t, wall_height, i)?);
    }
    bounds.push(wall_height);
    Ok((0..n).map(|i| (bounds[i], bounds[i + 1] - bounds[i])).collect())
}

/// Spec STK-01: the horizontal span `(left, right)` of each member across the wall (from the location line), and the width
/// of the type, which is the spread of the spans.
pub fn member_spans(m: &BimModel, t: &StackedWallType) -> Option<(Vec<(f64, f64)>, f64)> {
    let mut spans = Vec::new();
    for mem in &t.members {
        let wt = m.wall_types.get(&mem.type_id)?;
        let half = wt.structure.width().abs() / 2.0;
        let k = wt.structure.offset_for_location_line(t.location_line);
        let (lo, hi) = (mem.offset - half - k, mem.offset + half - k);
        spans.push(if mem.flip { (-hi, -lo) } else { (lo, hi) });
    }
    let width = spans.iter().map(|s| s.1).fold(f64::NEG_INFINITY, f64::max) - spans.iter().map(|s| s.0).fold(f64::INFINITY, f64::min);
    (width.is_finite() && width >= 0.0).then_some((spans, width))
}

fn single_layer_type(m: &mut BimModel, cx: &mut TxnContext<'_>, name: &str, width: f64) -> Result<ElementId, Failure> {
    if let Some(t) = m.wall_types.values().find(|t| t.name == name) {
        return Ok(t.id);
    }
    let id = cx.new_id();
    let structure = CompoundStructure::new(vec![Layer { width, function: LayerFunction::Structure }], 0, 0)?;
    m.wall_types.insert(id, WallType { id, name: name.to_owned(), structure });
    Ok(id)
}

/// The `index`-th stacked wall type of the template (the harness lists them by name); only the template's one exists.
/// ENGINE CHOICE: its members are the OBSERVED ones (module docs). The owner is also a wall type of the engine, of the
/// width Spec STK-01 gives, so every wall operation applies to a stacked wall.
pub fn use_template_type(m: &mut BimModel, cx: &mut TxnContext<'_>, index: usize) -> Result<ElementId, Failure> {
    if index != 0 {
        return Err(Failure::error("stacked.no_such_type", "The template has one stacked wall type."));
    }
    const NAME: &str = "Exterior - Brick Over Block w Metal Stud";
    if let Some(t) = m.stacked_types.values().find(|t| t.name == NAME) {
        return Ok(t.id);
    }
    let block = single_layer_type(m, cx, "Exterior - Block on Mtl. Stud", 0.46)?;
    let brick = single_layer_type(m, cx, "Exterior - Brick on Mtl. Stud", 0.35)?;
    let id = cx.new_id();
    let t = StackedWallType {
        id,
        name: NAME.to_owned(),
        location_line: LocationLine::WallCenterline,
        members: vec![
            StackedMember { type_id: block, height: Some(0.9), offset: 0.0, flip: false },
            StackedMember { type_id: brick, height: None, offset: -0.055, flip: false },
        ],
        sample_height: 6.0,
    };
    let (_, width) = member_spans(m, &t).ok_or_else(|| Failure::error("stacked.invalid", "A member type is missing."))?;
    let structure = CompoundStructure::new(vec![Layer { width, function: LayerFunction::Structure }], 0, 0)?;
    m.wall_types.insert(id, WallType { id, name: NAME.to_owned(), structure });
    m.stacked_types.insert(id, t);
    Ok(id)
}

/// A member wall as the reference product reports it: type, height, base offset from the owner's base and the location line
/// shifted by the member offset.
#[derive(Clone, Debug, PartialEq)]
pub struct MemberState {
    pub type_id: ElementId,
    pub width: f64,
    pub height: f64,
    pub base_offset: f64,
    pub start: asterion_geom::Vec2,
    pub end: asterion_geom::Vec2,
}

/// The members of a stacked wall, bottom first; empty for any other wall, and for a stacked wall that is too low (Spec
/// STK-01).
pub fn members(m: &BimModel, w: &Wall) -> Vec<MemberState> {
    let Some(t) = m.stacked_types.get(&w.type_id) else { return Vec::new() };
    let Ok((base_z, top_z)) = crate::regen::wall_vertical_span(m, w) else { return Vec::new() };
    let Ok(heights) = member_heights(t, top_z - base_z) else { return Vec::new() };
    let Some((spans, _)) = member_spans(m, t) else { return Vec::new() };
    let Some(dir) = (w.end - w.start).normalized() else { return Vec::new() };
    let left = if w.flipped { -dir.perp_ccw() } else { dir.perp_ccw() };
    t.members
        .iter()
        .zip(heights.iter().zip(spans.iter()))
        .filter_map(|(mem, ((b, h), (lo, hi)))| {
            let wt = m.wall_types.get(&mem.type_id)?;
            // the member's location line is the middle of its span
            let shift = left * ((lo + hi) / 2.0);
            Some(MemberState { type_id: mem.type_id, width: wt.structure.width(), height: *h, base_offset: *b, start: w.start + shift, end: w.end + shift })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template() -> (crate::BimDocument, ElementId) {
        let mut d = crate::new_document();
        let t = d.transact("t", false, |m, cx| use_template_type(m, cx, 0)).value.unwrap();
        (d, t)
    }

    /// Spec STK-01, STK-02 on the template type; round 38: 0.9 + 5.1, 0.9 + 3.1, 0.9 + 0.1; width 0.46; spans [-0.23, 0.23]
    /// and [-0.23, 0.12].
    #[test]
    fn the_template_type_follows_the_rows_and_the_measured_members() {
        let (d, t) = template();
        let m = d.model();
        let st = &m.stacked_types[&t];
        assert!(is_well_formed(st));
        assert_eq!(min_height(st).map(|x| x.1), Some(1));
        assert!((min_height(st).unwrap().0 - 0.9).abs() < 1e-12);
        for (h, top) in [(6.0, 5.1), (4.0, 3.1), (1.0, 0.1)] {
            let hs = member_heights(st, h).unwrap();
            assert!((hs[0].1 - 0.9).abs() < 1e-9 && (hs[1].1 - top).abs() < 1e-9 && (hs[1].0 - 0.9).abs() < 1e-9, "{hs:?}");
        }
        assert_eq!(boundary(st, 0.5, 1), Err(BoundaryError::TooShort));
        assert_eq!(boundary(st, 0.9, 1), Err(BoundaryError::NoHeightLeft));
        let (spans, width) = member_spans(m, st).unwrap();
        assert!((width - 0.46).abs() < 1e-9);
        assert!((spans[0].0 + 0.23).abs() < 1e-9 && (spans[1].0 + 0.23).abs() < 1e-9 && (spans[1].1 - 0.12).abs() < 1e-9, "{spans:?}");
    }

    /// Spec STK-02: two variable members, a fixed height under 1e-9, a repeated... are not valid.
    #[test]
    fn a_stacked_type_with_two_variable_members_is_not_valid() {
        let (d, t) = template();
        let mut st = d.model().stacked_types[&t].clone();
        st.members[0].height = None;
        assert!(!is_well_formed(&st));
        st.members[0].height = Some(0.0);
        assert!(!is_well_formed(&st));
        st.members[0].height = Some(7.0);
        assert!(!is_well_formed(&st), "the fixed heights exceed the sample height");
    }

    /// A variable member below: the boundary is measured from the top.
    #[test]
    fn a_variable_member_below_is_measured_from_the_top() {
        let (d, t) = template();
        let mut st = d.model().stacked_types[&t].clone();
        st.members[0].height = None;
        st.members[1].height = Some(1.0);
        let hs = member_heights(&st, 4.0).unwrap();
        assert!((hs[0].1 - 3.0).abs() < 1e-9 && (hs[1].0 - 3.0).abs() < 1e-9 && (hs[1].1 - 1.0).abs() < 1e-9, "{hs:?}");
    }
}
