//! The project base point, the survey point, the project position (shared coordinates), the Elevation Base of levels and
//! spot elevations. OBSERVED through the public API and 38b. Rules specified: (the reference product: two elements told
//! apart by kind 0 / 1; the internal position is the stored point; the SHARED position is the internal one transformed by
//! the INVERSE of the active location transform; moving a clipped point rewrites the location transform so the point keeps
//! reading the new shared coordinates; the parameters -1150191 / -1150192 / -1150193 are the Y, X and Z of the shared
//! position) and (a level's relative elevation = elevation - the Z of the location transform of the type's base elevation
//! type 0 / 1; the level type parameters). What those rules do not settle stays an ENGINE CHOICE that fits the named
//! observation and carries its question (not specified: the template's default shared coordinates, the too-far-away limit
//! of the project base point).
//!
//! Model (all fitted to the measurements, which agree with it to the last digit):
//! * internal coordinates are the engine's; the project position `(ew, ns, elev, angle)` maps an internal point `p` to
//!   shared coordinates `R(angle) p + (ew, ns, elev)` (`R` a rotation about z);
//! * a CLIPPED survey point moved by `v` moves the shared system with it: its shared position stays, the project
//!   position takes `- R v` (OBSERVED: SP (10, 20, -5) -> project position -10 / -20 / 5); an UNCLIPPED one moves within
//!   the shared system (the project position is unchanged); the project base point (unclipped) likewise;
//! * a level's `Elevation` is relative to its type's Elevation Base: the project base point's internal z, or the shared
//!   system's zero (Survey Point) - `ProjectElevation` is the internal z; writing `Elevation` writes the relative value.
//!   The engine has one level type (the template's "8mm Head"), so the base is one setting for every level (OBSERVED:
//!   writing it on L1's type changes L0 too).

use crate::model::BimModel;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use asterion_geom::Vec2;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The name of the template's level type (OBSERVED; template content).
pub const LEVEL_TYPE_NAME: &str = "8mm Head";

pub mod codes {
    pub const BAD_POINT: &str = "base_point.unknown";
    pub const SPOT_HOST: &str = "spot_elevation.bad_host";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Which {
    Project,
    Survey,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpotElevation {
    pub id: ElementId,
    pub view: ElementId,
    pub host: ElementId,
    pub point: Vec2,
    pub leader: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BasePoints {
    pub project: [f64; 3],
    pub survey: [f64; 3],
    /// OBSERVED defaults: the survey point clipped, the project base point not.
    pub survey_clipped: bool,
    pub project_clipped: bool,
    /// East/west, north/south, elevation (metres) and the angle to true north (radians).
    pub position: [f64; 4],
    /// The Elevation Base of the level type: true = Survey Point (raw 1), false = Project Base Point (raw 0, default).
    pub level_base_survey: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub spots: BTreeMap<ElementId, SpotElevation>,
}

impl Default for BasePoints {
    fn default() -> Self {
        BasePoints { project: [0.0; 3], survey: [0.0; 3], survey_clipped: true, project_clipped: false, position: [0.0; 4], level_base_survey: false, spots: BTreeMap::new() }
    }
}

impl BasePoints {
    pub fn is_default(&self) -> bool {
        *self == BasePoints::default()
    }
}

fn rot(angle: f64, v: [f64; 3]) -> [f64; 3] {
    let (s, c) = angle.sin_cos();
    [c * v[0] - s * v[1], s * v[0] + c * v[1], v[2]]
}

/// The shared coordinates of an internal point.
pub fn to_shared(m: &BimModel, p: [f64; 3]) -> [f64; 3] {
    let pp = m.base_points.position;
    let r = rot(pp[3], p);
    [r[0] + pp[0], r[1] + pp[1], r[2] + pp[2]]
}

/// The offset `Elevation = internal z - offset` of the levels' Elevation Base.
pub fn level_base_offset(m: &BimModel) -> f64 {
    if m.base_points.level_base_survey {
        -m.base_points.position[2]
    } else {
        m.base_points.project[2]
    }
}

/// `Level.Elevation`: the level's elevation relative to its Elevation Base.
pub fn relative_elevation(m: &BimModel, level: ElementId) -> Option<f64> {
    m.levels.get(&level).map(|l| l.elevation - level_base_offset(m))
}

/// The internal elevation that a written `Level.Elevation` stands for.
pub fn internal_from_relative(m: &BimModel, value: f64) -> f64 {
    value + level_base_offset(m)
}

/// Move a base point.
pub fn move_point(m: &mut BimModel, _cx: &mut TxnContext<'_>, which: Which, v: [f64; 3]) -> Result<(), Failure> {
    let bp = &mut m.base_points;
    let clipped = match which {
        Which::Project => bp.project_clipped,
        Which::Survey => bp.survey_clipped,
    };
    let p = match which {
        Which::Project => &mut bp.project,
        Which::Survey => &mut bp.survey,
    };
    for i in 0..3 {
        p[i] += v[i];
    }
    // a clipped point keeps its shared position: the shared system moves with it (ENGINE CHOICE for the project base
    // point, never measured clipped)
    if clipped {
        let r = rot(bp.position[3], v);
        for i in 0..3 {
            bp.position[i] -= r[i];
        }
    }
    Ok(())
}

pub fn set_clipped(m: &mut BimModel, _cx: &mut TxnContext<'_>, which: Which, value: bool) -> Result<(), Failure> {
    match which {
        Which::Project => m.base_points.project_clipped = value,
        Which::Survey => m.base_points.survey_clipped = value,
    }
    Ok(())
}

/// `SetProjectPosition`: the given components replace the stored ones (the harness reads the others first).
pub fn set_position(m: &mut BimModel, _cx: &mut TxnContext<'_>, ew: Option<f64>, ns: Option<f64>, elev: Option<f64>, angle: Option<f64>) -> Result<(), Failure> {
    let p = &mut m.base_points.position;
    for (i, v) in [ew, ns, elev, angle].into_iter().enumerate() {
        if let Some(v) = v {
            p[i] = v;
        }
    }
    Ok(())
}

/// The Elevation Base of the level type (raw 1 = Survey Point).
pub fn set_level_base(m: &mut BimModel, _cx: &mut TxnContext<'_>, survey: bool) -> Result<(), Failure> {
    m.base_points.level_base_survey = survey;
    Ok(())
}

/// A spot elevation on the top face of a floor in a plan (OBSERVED: committed, one top face; its origin is the point on the
/// floor's top and follows the floor when its level moves).
pub fn create_spot(m: &mut BimModel, cx: &mut TxnContext<'_>, view: ElementId, host: ElementId, point: Vec2, leader: bool) -> Result<ElementId, Failure> {
    m.view(view)?;
    if !m.floors.get(&host).map_or(false, |f| f.role == crate::model::SlabRole::Floor) {
        return Err(Failure::error(codes::SPOT_HOST, "A spot elevation is placed on the top face of a floor here.").on(&[host]));
    }
    let id = cx.new_id();
    m.base_points.spots.insert(id, SpotElevation { id, view, host, point, leader });
    Ok(id)
}

/// The origin of a spot elevation: its point on the host's top face.
pub fn spot_origin(m: &BimModel, s: &SpotElevation) -> Option<[f64; 3]> {
    let f = m.floors.get(&s.host)?;
    let z = crate::regen::slab_reference_z_at(m, f, s.point)?;
    Some([s.point.x, s.point.y, z])
}

/// The value a spot elevation shows, in metres: relative to the project base point (OBSERVED: 3000, then 2500 once the
/// project base point is 0.5 m up, and -47000 for a top at -46.5; independent of the levels' Elevation Base).
pub fn spot_value(m: &BimModel, s: &SpotElevation) -> Option<f64> {
    spot_origin(m, s).map(|o| o[2] - m.base_points.project[2])
}

/// Forget the spot elevations of deleted hosts and views (ENGINE CHOICE: they go with them).
pub fn elements_deleted(m: &mut BimModel, gone: &[ElementId]) {
    m.base_points.spots.retain(|id, s| !gone.contains(id) && !gone.contains(&s.host) && !gone.contains(&s.view));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_measured_sequence_of_round_38b() {
        let mut d = crate::new_document();
        let mut ls = [ElementId(0); 2];
        d.transact("l", false, |m, cx| {
            ls[0] = crate::ops::create_level(m, cx, "L0", 0.0)?;
            ls[1] = crate::ops::create_level(m, cx, "L1", 3.0)?;
            Ok(())
        });
        assert!(d.transact("mv", false, |m, cx| move_point(m, cx, Which::Survey, [10.0, 20.0, -5.0])).committed());
        assert_eq!(d.model().base_points.position, [-10.0, -20.0, 5.0, 0.0]);
        assert!(d.transact("b", false, |m, cx| set_level_base(m, cx, true)).committed());
        assert!((relative_elevation(d.model(), ls[1]).unwrap() - 8.0).abs() < 1e-12);
        assert!(d.transact("b", false, |m, cx| set_level_base(m, cx, false)).committed());
        assert!(d.transact("c", false, |m, cx| set_clipped(m, cx, Which::Survey, false)).committed());
        assert!(d.transact("mv2", false, |m, cx| move_point(m, cx, Which::Survey, [0.0, 0.0, 2.0])).committed());
        assert_eq!(d.model().base_points.position, [-10.0, -20.0, 5.0, 0.0]);
        assert!(d.transact("pbp", false, |m, cx| move_point(m, cx, Which::Project, [1.0, 2.0, 0.5])).committed());
        assert!((relative_elevation(d.model(), ls[0]).unwrap() + 0.5).abs() < 1e-12);
        assert!(d.transact("pp", false, |m, cx| set_position(m, cx, Some(100.0), Some(200.0), Some(50.0), Some(0.5))).committed());
        let s = to_shared(d.model(), d.model().base_points.project);
        assert!((s[0] - 99.91873148468194).abs() < 1e-9 && (s[1] - 202.2345906623849).abs() < 1e-9 && (s[2] - 50.5).abs() < 1e-12, "{s:?}");
        assert!(d.transact("b", false, |m, cx| set_level_base(m, cx, true)).committed());
        assert!((internal_from_relative(d.model(), 3.5) + 46.5).abs() < 1e-12);
    }
}
