//! Typed built-in parameters of walls.
//!
//! Parameters are a *view* over the semantic model: reading one regenerates the
//! value from the model; read-only flags follow observed reference behaviour.

use crate::model::{BimModel, Wall};
use crate::regen::wall_state;
use asterion_core::{ElementId, Failure};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "storage", content = "value", rename_all = "snake_case")]
pub enum ParamValue {
    Length(f64),
    Area(f64),
    Volume(f64),
    Integer(i64),
    Bool(bool),
    Element(Option<ElementId>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WallParam {
    Length,
    UnconnectedHeight,
    BaseOffset,
    TopOffset,
    BaseConstraint,
    TopConstraint,
    LocationLine,
    Area,
    Volume,
    Width,
    Structural,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Param {
    pub id: WallParam,
    pub read_only: bool,
    pub value: ParamValue,
}

/// All built-in parameters of a wall.
///
/// Top Offset reads the stored offset (see Wall::top_offset).
pub fn wall_params(model: &BimModel, w: &Wall) -> Result<Vec<Param>, Failure> {
    let s = wall_state(model, w)?;
    let constrained = w.top_level.is_some();
    let (top_level, top_offset) = (w.top_level, w.top_offset);
    let p = |id, read_only, value| Param { id, read_only, value };
    Ok(vec![
        p(WallParam::Length, true, ParamValue::Length(s.length)),
        p(WallParam::UnconnectedHeight, constrained, ParamValue::Length(s.height)),
        p(WallParam::BaseOffset, false, ParamValue::Length(w.base_offset)),
        p(WallParam::TopOffset, false, ParamValue::Length(top_offset)),
        p(WallParam::BaseConstraint, false, ParamValue::Element(Some(w.base_level))),
        p(WallParam::TopConstraint, false, ParamValue::Element(top_level)),
        p(WallParam::LocationLine, false, ParamValue::Integer(w.location_line.code())),
        p(WallParam::Area, true, ParamValue::Area(s.area)),
        p(WallParam::Volume, true, ParamValue::Volume(s.volume)),
        p(WallParam::Width, true, ParamValue::Length(s.width)),
        p(WallParam::Structural, false, ParamValue::Bool(w.structural)),
    ])
}

pub fn wall_param(model: &BimModel, w: &Wall, id: WallParam) -> Result<Param, Failure> {
    Ok(wall_params(model, w)?.into_iter().find(|p| p.id == id).expect("every WallParam is listed"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FloorParam {
    Level,
    HeightOffset,
    Thickness,
    Area,
    Perimeter,
    Volume,
    Structural,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FloorParamValue {
    pub id: FloorParam,
    pub read_only: bool,
    pub value: ParamValue,
}

/// Built-in parameters of a floor. Area, Perimeter, Volume and Thickness are computed/read-only; the level and height
/// offset drive the slab position.
pub fn floor_params(model: &BimModel, f: &crate::model::Floor) -> Result<Vec<FloorParamValue>, Failure> {
    let s = crate::regen::floor_state(model, f)?;
    let p = |id, read_only, value| FloorParamValue { id, read_only, value };
    Ok(vec![
        p(FloorParam::Level, false, ParamValue::Element(Some(f.level))),
        p(FloorParam::HeightOffset, false, ParamValue::Length(f.height_offset)),
        p(FloorParam::Thickness, true, ParamValue::Length(s.thickness)),
        p(FloorParam::Area, true, ParamValue::Area(s.area)),
        p(FloorParam::Perimeter, true, ParamValue::Length(s.perimeter)),
        p(FloorParam::Volume, true, ParamValue::Volume(s.volume)),
        p(FloorParam::Structural, false, ParamValue::Bool(f.structural)),
    ])
}
