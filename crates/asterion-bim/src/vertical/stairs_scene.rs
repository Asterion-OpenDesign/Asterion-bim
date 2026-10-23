//! Stairs through the component API: the scene around [`crate::stairs`].
//!
//! `crate::stairs` holds the rules of five specification rules (risers, run length, landing snap, railing counts, failure
//! channels).
//!
//! * The three template stairs types (OBSERVED): all with a 0.18 m maximum riser, a 0.275 m tread and a 1.0 m run width;
//!   "180mm max riser 275mm tread" is assembled with 50 mm stringers, a 50 mm tread / 12.5 mm riser run type and a
//!   non-monolithic landing; "Monolithic Stair" is cast in place, "Precast Stair" precast, both with no side supports, a
//!   monolithic run type of 0.15 / 0.19 m depth and a 0.30 / 0.22 m landing; a stairs of the last two has no supports.
//! * Setting a run's base elevation keeps its top and its end: the base snaps to a whole riser (round half away), the run
//!   keeps the risers above it, its length is `tread x (risers - 1)` and the start moves along the run (one measurement,
//!   twice).
//! * `Stairs.DesiredRisersNumber` is Spec STAIRS-RISERS-02: the setter refuses n <= 0, does nothing when n equals the
//!   current desired number, otherwise sets the riser height to `height / n` and raises the user-changed flag; the FIRST
//!   regeneration clears the flag and keeps that riser height (even above the maximum). From the next regeneration `n =
//!   max(ceil(height / max riser - 1e-9), stored actual number)`. OBSERVED: 14 on a 3 m stairs of a 180 mm type gave 17 and
//!   30 gave 30 (riser 0.1 m): it fits a second regeneration after the actual number equals the user's count. ENGINE
//!   CHOICE: the engine sets the actual number to n and regenerates a second time inside the scope.
//! * An automatic landing between two runs is refused (`CanCreateAutomaticLanding` false) in every arrangement measured: a
//!   U 1.0, 1.2 and 1.8 m apart, an L, and end to end.
//! * The scope commit gives a stairs two railings per run and one per landing; a railing set cannot be added to a stairs
//!   that has one (OBSERVED: `Railing.Create` on every stairs of the round answered "already has associated railings").
//! * A spiral run's arc is rounded to whole treads like a straight run's line (the angle per tread of the type's tread
//!   depth at the run's radius: 23 treads for half a turn at 2 m, ending 0.0234 rad short of the half turn the other way
//!   round); the run turns the way the API's `clockwise` says.
//! * A non-monolithic landing is as thick as the run type's tread (50 mm).
//! * a stairs with no run and no landing is not kept at the scope commit (OBSERVED).

use crate::model::BimModel;
use crate::spec::codes;
use crate::stairs::{self, NewStairs, NewStraightRun, StairsLanding, StairsRun, StairsTypeParams};
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use asterion_geom::Vec2;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const TEMPLATE_STAIRS_TYPE: &str = "180mm max riser 275mm tread";
pub const TEMPLATE_RUN_TYPE: &str = "50mm Tread 25mm Nosing 13mm Riser";
pub const TEMPLATE_RAILING_TYPES: [&str; 3] = ["900mm Pipe", "900mm Rectangular", "Glass Panel - Bottom Fill"];
/// ENGINE CHOICE: the run type code a component straight run gets.
const STRAIGHT_RUN_CODE: u8 = 1;

/// A run type: the thickness of treads and risers (a type of the stairs type, shared by every stairs that uses it:
/// OBSERVED, the three types showed the same riser thickness after one change).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunType {
    pub id: ElementId,
    pub name: String,
    pub tread_thickness: f64,
    pub riser_thickness: f64,
    pub nosing_length: f64,
    pub has_risers: bool,
    pub has_treads: bool,
    /// OBSERVED on, round 24: the template's run types are slanted.
    #[serde(default = "slanted_by_default")]
    pub riser_slanted: bool,
    pub is_monolithic: bool,
    pub structural_depth: f64,
    pub total_depth: f64,
}

fn slanted_by_default() -> bool {
    true
}

/// What a stairs type carries besides the parameters of [`StairsTypeParams`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StairsTypeExt {
    pub construction_method: String,
    pub left_support: Option<String>,
    pub right_support: Option<String>,
    pub run_type: ElementId,
    pub landing_type: String,
    pub landing_thickness: f64,
    pub landing_monolithic: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Justification {
    Left,
    #[default]
    Center,
    Right,
}

impl Justification {
    pub fn from_name(n: &str) -> Option<Self> {
        match n {
            "left" => Some(Justification::Left),
            "center" => Some(Justification::Center),
            "right" => Some(Justification::Right),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Justification::Left => "Left",
            Justification::Center => "Center",
            Justification::Right => "Right",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStyle {
    #[default]
    Straight,
    Sketched,
    Spiral,
}

impl RunStyle {
    pub fn name(self) -> &'static str {
        match self {
            RunStyle::Straight => "Straight",
            RunStyle::Sketched => "Sketched",
            RunStyle::Spiral => "Spiral",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpiralGeom {
    pub center: Vec2,
    pub radius: f64,
    pub start_angle: f64,
    pub included_angle: f64,
    /// The API's `clockwise` argument (round 24 passed true and the run turned clockwise).
    pub clockwise: bool,
}

/// What a run has beyond the stored [`StairsRun`] (keyed by the run's id).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RunExt {
    pub style: RunStyle,
    pub justification: Justification,
    /// Sketched runs: the riser count is the number of riser lines.
    #[serde(default)]
    pub fixed_risers: Option<i32>,
    /// Sketched runs: the walk path.
    #[serde(default)]
    pub path: Option<(Vec2, Vec2)>,
    /// Sketched runs: the two boundary lines.
    #[serde(default)]
    pub boundary: Vec<(Vec2, Vec2)>,
    #[serde(default)]
    pub spiral: Option<SpiralGeom>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LandingExt {
    pub boundary: Vec<Vec2>,
    pub automatic: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RailingType {
    pub id: ElementId,
    pub name: String,
}

/// A railing drawn on a path (not on a stairs).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathRailing {
    pub id: ElementId,
    pub railing_type: ElementId,
    pub level: ElementId,
    pub path: Vec<(Vec2, Vec2)>,
}

/// One stored baluster: where it stands and which family symbol it is (the importer's data; the symbol is opaque).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BalusterInstance {
    /// The centre of its base, metres.
    pub position: [f64; 3],
    /// Rotation about the vertical axis, radians.
    pub rotation: f64,
    pub symbol: u64,
}

/// All the scene state of the stairs slice (one field of `BimModel`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StairsScene {
    #[serde(default)]
    pub run_types: BTreeMap<ElementId, RunType>,
    #[serde(default)]
    pub types: BTreeMap<ElementId, StairsTypeExt>,
    #[serde(default)]
    pub runs: BTreeMap<ElementId, RunExt>,
    #[serde(default)]
    pub landings: BTreeMap<ElementId, LandingExt>,
    #[serde(default)]
    pub railing_types: BTreeMap<ElementId, RailingType>,
    #[serde(default)]
    pub path_railings: BTreeMap<ElementId, PathRailing>,
    /// The balusters a stored railing carries (one placement per baluster). A railing with an entry here is drawn with
    /// exactly these; one without is laid out by Spec RAIL-BALUSTER-01.
    #[serde(default)]
    pub baluster_instances: BTreeMap<ElementId, Vec<BalusterInstance>>,
    /// The stairs type a new stairs gets when none is given: the one the last stairs was changed to (OBSERVED, two stairs
    /// made without a type after one made with a wide type took the wide type; it could as well be the last type
    /// duplicated).
    #[serde(default)]
    pub default_type: Option<ElementId>,
}

impl StairsScene {
    pub fn is_empty(&self) -> bool {
        self.run_types.is_empty()
            && self.types.is_empty()
            && self.runs.is_empty()
            && self.landings.is_empty()
            && self.railing_types.is_empty()
            && self.path_railings.is_empty()
            && self.baluster_instances.is_empty()
            && self.default_type.is_none()
    }

    /// The highest id this scene holds.
    pub fn max_id(&self) -> Option<ElementId> {
        [
            self.run_types.keys().next_back(),
            self.runs.keys().next_back(),
            self.landings.keys().next_back(),
            self.railing_types.keys().next_back(),
            self.path_railings.keys().next_back(),
        ]
        .into_iter()
        .flatten()
        .max()
        .copied()
    }
}

fn refused(msg: &str) -> Failure {
    Failure::error(codes::STAIRS_UNDECIDED_BRANCH, format!("Not modelled: {msg}."))
}

// ------------------------------------------------------------------ types

/// One of the template's three stairs types with its run and landing types (OBSERVED: the three share a 0.18 m maximum
/// riser, a 0.275 m tread and a 1.0 m run width).
struct TemplateType {
    name: &'static str,
    construction: &'static str,
    supports: Option<&'static str>,
    run_type: &'static str,
    tread_thickness: f64,
    riser_thickness: f64,
    nosing: f64,
    treads_and_risers: bool,
    monolithic_run: bool,
    structural_depth: f64,
    landing_type: &'static str,
    landing_thickness: f64,
    landing_monolithic: bool,
}

const TEMPLATE_TYPES: [TemplateType; 3] = [
    TemplateType {
        name: TEMPLATE_STAIRS_TYPE,
        construction: "Assembled",
        supports: Some("Stringer - 50mm Width"),
        run_type: TEMPLATE_RUN_TYPE,
        tread_thickness: 0.05,
        riser_thickness: 0.0125,
        nosing: 0.025,
        treads_and_risers: true,
        monolithic_run: false,
        structural_depth: 0.0,
        landing_type: "Non-Monolithic Landing",
        landing_thickness: 0.0,
        landing_monolithic: false,
    },
    TemplateType {
        name: "Monolithic Stair",
        construction: "CastInPlace",
        supports: None,
        run_type: "150mm Depth",
        tread_thickness: 0.0,
        riser_thickness: 0.0,
        nosing: 0.0,
        treads_and_risers: false,
        monolithic_run: true,
        structural_depth: 0.15,
        landing_type: "300mm Thickness",
        landing_thickness: 0.3,
        landing_monolithic: true,
    },
    TemplateType {
        name: "Precast Stair",
        construction: "Precast",
        supports: None,
        run_type: "190mm Depth",
        tread_thickness: 0.0,
        riser_thickness: 0.0,
        nosing: 0.0,
        treads_and_risers: false,
        monolithic_run: true,
        structural_depth: 0.19,
        landing_type: "220mm Thickness",
        landing_thickness: 0.22,
        landing_monolithic: true,
    },
];

/// The assembled template run type's `(tread thickness, riser thickness, nosing length)` in metres (OBSERVED): what a
/// stairs whose type has no run type of its own is built with.
pub fn template_run_dims() -> (f64, f64, f64) {
    let t = &TEMPLATE_TYPES[0];
    (t.tread_thickness, t.riser_thickness, t.nosing)
}

fn ensure_run_type(m: &mut BimModel, cx: &mut TxnContext<'_>, t: &TemplateType) -> ElementId {
    if let Some(r) = m.stairs_scene.run_types.values().find(|r| r.name == t.run_type) {
        return r.id;
    }
    let id = cx.new_id();
    m.stairs_scene.run_types.insert(
        id,
        RunType {
            id,
            name: t.run_type.to_owned(),
            tread_thickness: t.tread_thickness,
            riser_thickness: t.riser_thickness,
            nosing_length: t.nosing,
            has_risers: t.treads_and_risers,
            has_treads: t.treads_and_risers,
            riser_slanted: true,
            is_monolithic: t.monolithic_run,
            structural_depth: t.structural_depth,
            total_depth: t.structural_depth,
        },
    );
    id
}

/// The template's stairs type of that index (name order: the 180 mm type, "Monolithic Stair", "Precast Stair"), created on
/// first use. A type without side supports (the two monolithic ones) has no supports; its landing is as thick as its
/// landing type says.
pub fn template_stairs_type(m: &mut BimModel, cx: &mut TxnContext<'_>, index: usize) -> Result<ElementId, Failure> {
    let t = TEMPLATE_TYPES.get(index).ok_or_else(|| refused("no such template stairs type"))?;
    if let Some(existing) = m.stairs.types.values().find(|x| x.name == t.name) {
        return Ok(existing.id);
    }
    let run_type = ensure_run_type(m, cx, t);
    let id = stairs::create_stairs_type(
        m,
        cx,
        t.name,
        StairsTypeParams {
            max_riser_height: 0.18,
            min_tread_depth: 0.275,
            min_run_width: 1.0,
            use_calculator: false,
            calculator_target: 0.0,
            calculator_riser_multiplier: 0.0,
            calculator_tread_multiplier: 0.0,
            min_run_height: None,
            tread_thickness: None,
        },
    )?;
    m.stairs_scene.types.insert(
        id,
        StairsTypeExt {
            construction_method: t.construction.to_owned(),
            left_support: t.supports.map(str::to_owned),
            right_support: t.supports.map(str::to_owned),
            run_type,
            landing_type: t.landing_type.to_owned(),
            landing_thickness: t.landing_thickness,
            landing_monolithic: t.landing_monolithic,
        },
    );
    Ok(id)
}

/// `Duplicate` of a stairs type: the same parameters, the same run type (shared).
pub fn duplicate_stairs_type(m: &mut BimModel, cx: &mut TxnContext<'_>, source: ElementId, name: &str) -> Result<ElementId, Failure> {
    let params = m.stairs.stairs_type(source)?.params;
    let ext = m.stairs_scene.types.get(&source).cloned();
    // The duplicate's name checks
    let name = &crate::type_names::check_duplicate_name(name, |n| m.stairs.types.values().any(|t| t.name == n)).map_err(|f| f.on(&[source]))?;
    let id = stairs::create_stairs_type(m, cx, name, params)?;
    if let Some(e) = ext {
        m.stairs_scene.types.insert(id, e);
    }
    Ok(id)
}

/// The run type of a stairs type (`StairsType.RunType`).
pub fn run_type_of(m: &BimModel, stairs_type: ElementId) -> Result<ElementId, Failure> {
    m.stairs_scene.types.get(&stairs_type).map(|e| e.run_type).ok_or_else(|| refused("the stairs type has no run type"))
}

/// Set a named length property of a stairs type or of a run type (metres).
pub fn set_property(m: &mut BimModel, cx: &mut TxnContext<'_>, target: ElementId, property: &str, value: f64) -> Result<(), Failure> {
    if let Some(t) = m.stairs.types.get(&target) {
        let mut p = t.params;
        match property {
            "MaxRiserHeight" => p.max_riser_height = value,
            "MinTreadDepth" => p.min_tread_depth = value,
            "MinRunWidth" => p.min_run_width = value,
            _ => return Err(refused(&format!("the stairs type property {property}"))),
        }
        return stairs::set_stairs_type_params(m, cx, target, p);
    }
    if let Some(r) = m.stairs_scene.run_types.get_mut(&target) {
        match property {
            "TreadThickness" => r.tread_thickness = value,
            "RiserThickness" => r.riser_thickness = value,
            "NosingLength" => r.nosing_length = value,
            _ => return Err(refused(&format!("the run type property {property}"))),
        }
        return Ok(());
    }
    Err(Failure::error(codes::NOT_FOUND, format!("No stairs type or run type with id {target}.")).on(&[target]))
}

/// The template's railing type of that index (name order); created on first use.
pub fn template_railing_type(m: &mut BimModel, cx: &mut TxnContext<'_>, index: usize) -> Result<ElementId, Failure> {
    let name = TEMPLATE_RAILING_TYPES.get(index).ok_or_else(|| refused("no such railing type"))?;
    if let Some(t) = m.stairs_scene.railing_types.values().find(|t| t.name == *name) {
        return Ok(t.id);
    }
    let id = cx.new_id();
    m.stairs_scene.railing_types.insert(id, RailingType { id, name: (*name).to_owned() });
    Ok(id)
}

// ------------------------------------------------------------------ creation

#[derive(Clone, Debug)]
pub enum SceneRun {
    Straight { start: Vec2, end: Vec2, justification: Justification },
    Sketched { risers: usize, path: (Vec2, Vec2), boundary: Vec<(Vec2, Vec2)> },
    Spiral { geom: SpiralGeom, justification: Justification },
}

#[derive(Clone, Debug)]
pub struct SceneRunSpec {
    pub run: SceneRun,
    pub base_elevation: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct SceneLandingSpec {
    pub boundary: Vec<Vec2>,
    pub elevation: f64,
}

#[derive(Clone, Debug)]
pub struct NewScene {
    pub base_level: ElementId,
    pub top_level: ElementId,
    pub type_id: Option<ElementId>,
    pub runs: Vec<SceneRunSpec>,
    pub landings: Vec<SceneLandingSpec>,
    pub automatic_landings: Vec<(usize, usize)>,
    pub desired_risers: Option<i32>,
}

#[derive(Clone, Debug)]
pub struct SceneResult {
    /// `None` when the scope committed nothing (no run and no landing).
    pub stairs: Option<ElementId>,
    pub stairs_id: ElementId,
    pub runs: Vec<Option<ElementId>>,
    pub landings: Vec<Option<ElementId>>,
    pub component_errors: Vec<String>,
    pub can_create_automatic: Option<bool>,
}

/// The `StairsEditScope` workflow of the harness: start, make the components, commit.
pub fn create_scene(m: &mut BimModel, cx: &mut TxnContext<'_>, spec: NewScene) -> Result<SceneResult, Failure> {
    let base = m.level(spec.base_level)?.elevation;
    let top = m.level(spec.top_level)?.elevation;
    if !(top > base) {
        // OBSERVED: `Start` raises when the top level is not higher than the base level
        return Err(Failure::error(codes::STAIRS_TOP_NOT_ABOVE_BASE, "The top level must be higher than the base level.")
            .on(&[spec.base_level, spec.top_level]));
    }
    let type_id = match spec.type_id {
        Some(t) => {
            m.stairs_scene.default_type = Some(t);
            t
        }
        None => match m.stairs_scene.default_type.filter(|t| m.stairs.types.contains_key(t)) {
            Some(t) => t,
            None => template_stairs_type(m, cx, 0)?,
        },
    };
    let ty = m.stairs.stairs_type(type_id)?.params;
    let id = stairs::create_stairs(
        m,
        cx,
        NewStairs { type_id, base_level: spec.base_level, top_level: spec.top_level, initial_tread_depth: ty.min_tread_depth },
    )?;
    let mut out = SceneResult { stairs: None, stairs_id: id, runs: Vec::new(), landings: Vec::new(), component_errors: Vec::new(), can_create_automatic: None };
    for r in &spec.runs {
        match make_run(m, cx, id, r, base, &ty) {
            Ok(run) => out.runs.push(Some(run)),
            Err(f) => {
                out.component_errors.push(f.message.clone());
                out.runs.push(None);
            }
        }
    }
    for l in &spec.landings {
        match stairs::add_landing(m, cx, id, l.elevation) {
            Ok(lid) => {
                m.stairs_scene.landings.insert(lid, LandingExt { boundary: l.boundary.clone(), automatic: false });
                out.landings.push(Some(lid));
            }
            Err(f) => {
                out.component_errors.push(f.message.clone());
                out.landings.push(None);
            }
        }
    }
    for (a, b) in &spec.automatic_landings {
        // OBSERVED (two parallel runs 1.8 m apart): the reference product answers that it cannot create it
        out.can_create_automatic = Some(false);
        let _ = (a, b);
        out.component_errors.push("automatic landing: the landing cannot be created between these runs".to_owned());
    }
    if let Some(n) = spec.desired_risers {
        if let Err(f) = set_desired_risers(m, cx, id, n) {
            out.component_errors.push(f.message);
        }
    }
    // the commit of the scope: nothing is kept without a component
    let has_component = m.stairs.runs.values().any(|r| r.stairs == id) || m.stairs.landings.values().any(|l| l.stairs == id);
    if !has_component {
        stairs::delete_stairs(m, id)?;
        return Ok(out);
    }
    out.stairs = Some(id);
    stairs::regenerate_stairs(m, cx, id)?;
    make_railings(m, cx, id, None)?;
    Ok(out)
}

fn make_run(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    stairs_id: ElementId,
    r: &SceneRunSpec,
    base_level_elevation: f64,
    ty: &StairsTypeParams,
) -> Result<ElementId, Failure> {
    let run_id = match &r.run {
        SceneRun::Straight { start, end, justification } => {
            let id = stairs::add_straight_stair_run(
                m,
                cx,
                NewStraightRun {
                    stairs: stairs_id,
                    start: *start,
                    end: *end,
                    bottom_elevation: base_level_elevation,
                    width: ty.min_run_width,
                    begin_with_riser: true,
                    end_with_riser: true,
                    run_type_code: STRAIGHT_RUN_CODE,
                },
            )?;
            m.stairs_scene.runs.insert(id, RunExt { style: RunStyle::Straight, justification: *justification, ..Default::default() });
            id
        }
        SceneRun::Sketched { risers, path, boundary } => {
            let id = cx.new_id();
            m.stairs.runs.insert(
                id,
                StairsRun {
                    id,
                    stairs: stairs_id,
                    start: path.0,
                    end: path.1,
                    bottom_elevation: base_level_elevation,
                    width: 0.0,
                    begin_with_riser: true,
                    end_with_riser: true,
                    run_type_code: STRAIGHT_RUN_CODE,
                },
            );
            m.stairs_scene.runs.insert(
                id,
                RunExt { style: RunStyle::Sketched, fixed_risers: Some(*risers as i32), path: Some(*path), boundary: boundary.clone(), ..Default::default() },
            );
            id
        }
        SceneRun::Spiral { geom, justification } => {
            let id = cx.new_id();
            let at = |a: f64| geom.center + Vec2::new(a.cos(), a.sin()) * geom.radius;
            let tread = m.stairs.stairs(stairs_id)?.actual_tread_depth;
            let rounded = spiral_sweep(geom, tread).unwrap_or(geom.included_angle);
            let sweep = if geom.clockwise { -rounded } else { rounded };
            m.stairs.runs.insert(
                id,
                StairsRun {
                    id,
                    stairs: stairs_id,
                    start: at(geom.start_angle),
                    end: at(geom.start_angle + sweep),
                    bottom_elevation: base_level_elevation,
                    width: ty.min_run_width,
                    begin_with_riser: true,
                    end_with_riser: true,
                    run_type_code: STRAIGHT_RUN_CODE,
                },
            );
            m.stairs_scene.runs.insert(id, RunExt { style: RunStyle::Spiral, justification: *justification, spiral: Some(*geom), ..Default::default() });
            id
        }
    };
    if let Some(e) = r.base_elevation {
        set_run_base_elevation(m, cx, run_id, e)?;
    }
    Ok(run_id)
}

/// The arc a spiral run spans once rounded to whole treads: the straight rule over angles.
fn spiral_sweep(g: &SpiralGeom, tread_depth: f64) -> Option<f64> {
    let apt = stairs::angle_per_tread(tread_depth, g.radius)?;
    Some(if apt <= g.included_angle { (g.included_angle / apt + 0.5).floor() * apt } else { apt })
}

/// `StairsRun.BaseElevation = e`: the base snaps to a whole riser, the top and the END of the run stay, the run keeps the
/// risers above the new base.
pub fn set_run_base_elevation(m: &mut BimModel, _cx: &mut TxnContext<'_>, run: ElementId, new_base: f64) -> Result<(), Failure> {
    let r = m.stairs.runs.get(&run).ok_or_else(|| refused("no such run"))?.clone();
    if m.stairs_scene.runs.get(&run).map_or(false, |e| e.style != RunStyle::Straight) {
        return Err(refused("the base elevation of a sketched or spiral run"));
    }
    let s = m.stairs.stairs(r.stairs)?.clone();
    let st = stairs::stairs_state(m, r.stairs)?;
    let rs = st.runs.iter().find(|x| x.id == run).ok_or_else(|| refused("the run has no state"))?;
    let rh = s.actual_riser_height;
    let (e, _n) = stairs::snap_landing_elevation(new_base, st.bottom_elevation, rh).ok_or_else(|| refused("no riser height"))?;
    stairs::validate_run_bottom_elevation(r.run_type_code, e, rs.top_elevation, st.bottom_elevation, None)?;
    let risers = ((rs.top_elevation - e) / rh).round() as i32;
    if risers < 1 {
        return Err(refused("a run with no riser above the new base"));
    }
    let length = stairs::run_total_length(s.actual_tread_depth, risers, r.begin_with_riser, r.end_with_riser);
    let dir = (r.end - r.start).normalized().ok_or_else(|| refused("a run with no direction"))?;
    let run_mut = m.stairs.runs.get_mut(&run).expect("looked up above");
    run_mut.start = r.end - dir * length;
    run_mut.bottom_elevation = e;
    Ok(())
}

/// The desired number of risers inside an edit scope (Spec STAIRS-RISERS-02): the rule's setter and first regeneration (the
/// user's riser height is kept and the flag cleared), then the stored actual number of risers becomes n (ENGINE CHOICE: the
/// caller is not specified) and the second regeneration takes `n = max(ceil(height / max riser - 1e-9), stored actual
/// number)`. A count that is not positive is refused.
pub fn set_desired_risers(m: &mut BimModel, cx: &mut TxnContext<'_>, id: ElementId, n: i32) -> Result<(), Failure> {
    m.stairs.stairs(id)?;
    if n <= 0 {
        return Err(Failure::error(codes::STAIRS_ILLEGAL_DESIRED_RISERS, "The desired number of risers must be positive.").on(&[id]));
    }
    let current = stairs::stairs_state(m, id)?.desired_number_of_risers;
    if !stairs::desired_number_change_applies(n, current) {
        return Ok(());
    }
    stairs::set_riser_count_target(m, cx, id, n)?;
    m.stairs.stairs.get_mut(&id).expect("looked up above").riser_count_floor = Some(n);
    stairs::regenerate_stairs(m, cx, id)
}

/// The stored actual number of risers of a stairs (offset; `None` until something writes it).
pub fn stored_actual_number_of_risers(m: &BimModel, id: ElementId) -> Option<i32> {
    m.stairs.stairs.get(&id).and_then(|s| s.riser_count_floor)
}

// ------------------------------------------------------------------ railings

/// The railings a stairs gets at the commit of its edit scope: two per run and one per landing, through the path provider
/// rule.
fn make_railings(m: &mut BimModel, cx: &mut TxnContext<'_>, stairs_id: ElementId, railing_type: Option<ElementId>) -> Result<Vec<ElementId>, Failure> {
    let rt = match railing_type {
        Some(t) => t,
        None => template_railing_type(m, cx, 0)?,
    };
    let mut paths = Vec::new();
    for r in m.stairs.runs_of(stairs_id) {
        for side in 0..2u64 {
            paths.push(stairs::RailingPath { keys: vec![stairs::StairsSubelementKey(r.id.0 * 2 + side)] });
        }
    }
    for l in m.stairs.landings.values().filter(|l| l.stairs == stairs_id) {
        paths.push(stairs::RailingPath { keys: vec![stairs::StairsSubelementKey(l.id.0 * 2)] });
    }
    let set = stairs::RailingPathSet { reported_count: paths.len(), paths };
    stairs::generate_stair_railings(m, cx, stairs_id, rt, &set, &[])
}

/// `Railing.Create(stairs, type, position)`. OBSERVED: refused on every stairs of the round, each of which already had the
/// railings its edit scope made.
pub fn create_railing_on_stairs(m: &mut BimModel, cx: &mut TxnContext<'_>, stairs_id: ElementId, railing_type: ElementId) -> Result<Vec<ElementId>, Failure> {
    m.stairs.stairs(stairs_id)?;
    if m.stairs.railings.values().any(|r| r.stairs == stairs_id) {
        return Err(Failure::error(
            codes::RAILING_STAIRS_HAS_RAILINGS,
            "The stairs already have associated railings (or are in editing mode).",
        )
        .on(&[stairs_id]));
    }
    make_railings(m, cx, stairs_id, Some(railing_type))
}

pub fn create_railing_on_path(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    railing_type: ElementId,
    level: ElementId,
    path: Vec<(Vec2, Vec2)>,
) -> Result<ElementId, Failure> {
    m.level(level)?;
    if !m.stairs_scene.railing_types.contains_key(&railing_type) {
        return Err(Failure::error(codes::NOT_FOUND, format!("No railing type with id {railing_type}.")).on(&[railing_type]));
    }
    if path.is_empty() {
        return Err(refused("a railing path with no curve"));
    }
    let id = cx.new_id();
    m.stairs_scene.path_railings.insert(id, PathRailing { id, railing_type, level, path });
    Ok(id)
}

// ------------------------------------------------------------------ views

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathCurve {
    pub arc: bool,
    pub start: [f64; 3],
    pub end: [f64; 3],
}

#[derive(Clone, Debug)]
pub struct RunView {
    pub id: ElementId,
    pub base: f64,
    pub top: f64,
    pub risers: i32,
    pub treads: i32,
    pub width: f64,
    pub begins_with_riser: bool,
    pub ends_with_riser: bool,
    pub justification: Justification,
    pub style: RunStyle,
    pub path: Vec<PathCurve>,
    pub footprint: Vec<PathCurve>,
}

#[derive(Clone, Debug)]
pub struct LandingView {
    pub id: ElementId,
    pub base: f64,
    pub thickness: f64,
    pub automatic: bool,
}

#[derive(Clone, Debug)]
pub struct SceneView {
    pub bottom: f64,
    pub top: f64,
    pub height: f64,
    pub desired_risers: i32,
    pub actual_risers: i32,
    pub actual_treads: i32,
    pub riser_height: f64,
    pub tread_depth: f64,
    pub runs: Vec<RunView>,
    pub landings: Vec<LandingView>,
    pub supports: usize,
    pub railings: Vec<ElementId>,
}

fn line3(a: Vec2, b: Vec2, z: f64) -> PathCurve {
    PathCurve { arc: false, start: [a.x, a.y, z], end: [b.x, b.y, z] }
}

/// The derived state of a stairs as the harness reports it: a pure function of the model.
pub fn scene_view(m: &BimModel, id: ElementId) -> Result<SceneView, Failure> {
    let s = m.stairs.stairs(id)?;
    let st = stairs::stairs_state(m, id)?;
    let rh = s.actual_riser_height;
    let z = st.bottom_elevation;
    let mut runs = Vec::new();
    for r in m.stairs.runs_of(id) {
        let ext = m.stairs_scene.runs.get(&r.id).cloned().unwrap_or_default();
        let rs = st.runs.iter().find(|x| x.id == r.id);
        let (risers, base) = match ext.style {
            RunStyle::Straight => (rs.map_or(0, |x| x.risers), r.bottom_elevation),
            RunStyle::Sketched => (ext.fixed_risers.unwrap_or(0), r.bottom_elevation),
            RunStyle::Spiral => {
                let g = ext.spiral.ok_or_else(|| refused("a spiral run without its geometry"))?;
                let apt = stairs::angle_per_tread(s.actual_tread_depth, g.radius).ok_or_else(|| refused("a spiral run without a radius"))?;
                let sweep = spiral_sweep(&g, s.actual_tread_depth).unwrap_or(g.included_angle);
                (stairs::spiral_riser_number(sweep, apt, r.begin_with_riser, r.end_with_riser), r.bottom_elevation)
            }
        };
        let treads = risers - i32::from(r.begin_with_riser) - i32::from(r.end_with_riser) + 1;
        let top = base + risers as f64 * rh;
        let (path, footprint) = match ext.style {
            RunStyle::Straight => {
                let d = (r.end - r.start).normalized().unwrap_or(Vec2::new(1.0, 0.0));
                let left = Vec2::new(-d.y, d.x);
                let shift = match ext.justification {
                    Justification::Left => left * (-r.width / 2.0),
                    Justification::Center => Vec2::new(0.0, 0.0),
                    Justification::Right => left * (r.width / 2.0),
                };
                let (c0, c1) = (r.start + shift, r.end + shift);
                let h = left * (r.width / 2.0);
                (
                    vec![line3(c0, c1, z)],
                    vec![line3(c1 + h, c0 + h, z), line3(c0 + h, c0 - h, z), line3(c0 - h, c1 - h, z), line3(c1 - h, c1 + h, z)],
                )
            }
            RunStyle::Sketched => {
                let p = ext.path.unwrap_or((r.start, r.end));
                // the loop of the two boundary lines: the second reversed, across to the first's start, the first, across
                // to the second's end (OBSERVED `sk`)
                let footprint = match ext.boundary.as_slice() {
                    [b0, b1] => vec![line3(b1.1, b1.0, z), line3(b1.0, b0.0, z), line3(b0.0, b0.1, z), line3(b0.1, b1.1, z)],
                    _ => Vec::new(),
                };
                (vec![line3(p.0, p.1, z)], footprint)
            }
            RunStyle::Spiral => {
                let g = ext.spiral.expect("checked above");
                let rounded = spiral_sweep(&g, s.actual_tread_depth).unwrap_or(g.included_angle);
                let sweep = if g.clockwise { -rounded } else { rounded };
                let (a0, a1) = (g.start_angle, g.start_angle + sweep);
                let at = |a: f64, rad: f64| g.center + Vec2::new(a.cos(), a.sin()) * rad;
                let arc = |p: Vec2, q: Vec2| PathCurve { arc: true, start: [p.x, p.y, z], end: [q.x, q.y, z] };
                // the left of the travel is outside when the run turns clockwise
                let (rl, rr) = if g.clockwise { (g.radius + r.width / 2.0, g.radius - r.width / 2.0) } else { (g.radius - r.width / 2.0, g.radius + r.width / 2.0) };
                (
                    vec![arc(at(a0, g.radius), at(a1, g.radius))],
                    vec![
                        arc(at(a1, rl), at(a0, rl)),
                        line3(at(a0, rl), at(a0, rr), z),
                        arc(at(a0, rr), at(a1, rr)),
                        line3(at(a1, rr), at(a1, rl), z),
                    ],
                )
            }
        };
        runs.push(RunView {
            id: r.id,
            base,
            top,
            risers,
            treads,
            width: r.width,
            begins_with_riser: r.begin_with_riser,
            ends_with_riser: r.end_with_riser,
            justification: ext.justification,
            style: ext.style,
            path,
            footprint,
        });
    }
    let ext = m.stairs_scene.types.get(&s.type_id);
    let tread_thickness = ext.and_then(|e| m.stairs_scene.run_types.get(&e.run_type)).map_or(0.0, |t| t.tread_thickness);
    // a landing is as thick as its type says when the type is monolithic, else as thick as the run type's tread
    let landing_thickness = match ext {
        Some(e) if e.landing_monolithic => e.landing_thickness,
        _ => tread_thickness,
    };
    let has_supports = ext.map_or(true, |e| e.left_support.is_some());
    let landings: Vec<LandingView> = m
        .stairs
        .landings
        .values()
        .filter(|l| l.stairs == id)
        .map(|l: &StairsLanding| LandingView {
            id: l.id,
            base: l.base_elevation,
            thickness: landing_thickness,
            automatic: m.stairs_scene.landings.get(&l.id).map_or(false, |e| e.automatic),
        })
        .collect();
    // the highest component: the risers the stairs has made
    let top_of_top = runs.iter().map(|r| r.top).chain(landings.iter().map(|l| l.base)).fold(f64::NEG_INFINITY, f64::max);
    let actual_risers = if top_of_top.is_finite() && rh > 0.0 { stairs::created_risers(top_of_top, z, rh) } else { 0 };
    let actual_treads = runs.iter().map(|r| r.treads).sum();
    let supports = if has_supports { 2 * runs.len() + 4 * landings.len() } else { 0 };
    let railings = m.stairs.railings.values().filter(|r| r.stairs == id).map(|r| r.id).collect();
    Ok(SceneView {
        bottom: z,
        top: st.top_elevation,
        height: st.height,
        desired_risers: st.desired_number_of_risers,
        actual_risers,
        actual_treads,
        riser_height: rh,
        tread_depth: s.actual_tread_depth,
        runs,
        landings,
        supports,
        railings,
    })
}

// ------------------------------------------------------------------ transforms and deletion

/// Translate a stairs and everything it owns.
pub fn translate_stairs(m: &mut BimModel, id: ElementId, d: Vec2) {
    for r in m.stairs.runs.values_mut().filter(|r| r.stairs == id) {
        r.start = r.start + d;
        r.end = r.end + d;
        if let Some(e) = m.stairs_scene.runs.get_mut(&r.id) {
            if let Some(p) = e.path.as_mut() {
                *p = (p.0 + d, p.1 + d);
            }
            if let Some(g) = e.spiral.as_mut() {
                g.center = g.center + d;
            }
        }
    }
    let landing_ids: Vec<ElementId> = m.stairs.landings.values().filter(|l| l.stairs == id).map(|l| l.id).collect();
    for l in landing_ids {
        if let Some(e) = m.stairs_scene.landings.get_mut(&l) {
            for p in e.boundary.iter_mut() {
                *p = *p + d;
            }
        }
    }
}

/// Copy a stairs with its runs, landings and railings (new ids), translated by `d`.
pub fn copy_stairs(m: &mut BimModel, cx: &mut TxnContext<'_>, id: ElementId, d: Vec2) -> Result<ElementId, Failure> {
    let s = m.stairs.stairs(id)?.clone();
    let new_id = cx.new_id();
    let mut ns = s.clone();
    ns.id = new_id;
    m.stairs.stairs.insert(new_id, ns);
    let runs: Vec<StairsRun> = m.stairs.runs_of(id).into_iter().cloned().collect();
    for mut r in runs {
        let old = r.id;
        r.id = cx.new_id();
        r.stairs = new_id;
        if let Some(e) = m.stairs_scene.runs.get(&old).cloned() {
            m.stairs_scene.runs.insert(r.id, e);
        }
        m.stairs.runs.insert(r.id, r);
    }
    let landings: Vec<StairsLanding> = m.stairs.landings.values().filter(|l| l.stairs == id).cloned().collect();
    for mut l in landings {
        let old = l.id;
        l.id = cx.new_id();
        l.stairs = new_id;
        if let Some(e) = m.stairs_scene.landings.get(&old).cloned() {
            m.stairs_scene.landings.insert(l.id, e);
        }
        m.stairs.landings.insert(l.id, l);
    }
    let railings: Vec<stairs::StairsRailing> = m.stairs.railings.values().filter(|r| r.stairs == id).cloned().collect();
    for mut r in railings {
        r.id = cx.new_id();
        r.stairs = new_id;
        m.stairs.railings.insert(r.id, r);
    }
    translate_stairs(m, new_id, d);
    Ok(new_id)
}

/// Drop the scene records of elements that no longer exist (after a deletion).
pub fn prune(m: &mut BimModel) {
    let runs: Vec<ElementId> = m.stairs_scene.runs.keys().copied().filter(|r| !m.stairs.runs.contains_key(r)).collect();
    for r in runs {
        m.stairs_scene.runs.remove(&r);
    }
    let landings: Vec<ElementId> = m.stairs_scene.landings.keys().copied().filter(|l| !m.stairs.landings.contains_key(l)).collect();
    for l in landings {
        m.stairs_scene.landings.remove(&l);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops;

    struct Fx {
        doc: crate::BimDocument,
        l0: ElementId,
        l1: ElementId,
    }

    fn fixture() -> Fx {
        let mut doc = crate::new_document();
        let (l0, l1) = doc
            .transact("fx", false, |m, cx| Ok((ops::create_level(m, cx, "L0", 0.0)?, ops::create_level(m, cx, "L1", 3.0)?)))
            .value
            .expect("fixture");
        Fx { doc, l0, l1 }
    }

    fn straight(l0: ElementId, l1: ElementId, a: (f64, f64), b: (f64, f64), j: Justification) -> NewScene {
        NewScene {
            base_level: l0,
            top_level: l1,
            type_id: None,
            runs: vec![SceneRunSpec { run: SceneRun::Straight { start: Vec2::new(a.0, a.1), end: Vec2::new(b.0, b.1), justification: j }, base_elevation: None }],
            landings: Vec::new(),
            automatic_landings: Vec::new(),
            desired_risers: None,
        }
    }

    #[test]
    fn a_straight_run_makes_the_observed_counts_and_two_railings() {
        // OBSERVED (`s1`): 3 m high, 4 m long: 17 desired risers, 16 made, 15 treads, riser 3/17, the run top 16 risers up,
        // the line rounded to 15 treads (4.125 m), two railings
        let mut f = fixture();
        let (l0, l1) = (f.l0, f.l1);
        let r = f.doc.transact("s", false, |m, cx| create_scene(m, cx, straight(l0, l1, (0.0, 0.0), (4.0, 0.0), Justification::Center))).value.unwrap();
        let v = scene_view(f.doc.model(), r.stairs.unwrap()).unwrap();
        assert_eq!((v.desired_risers, v.actual_risers, v.actual_treads, v.railings.len(), v.supports), (17, 16, 15, 2, 2));
        assert!((v.riser_height - 3.0 / 17.0).abs() < 1e-12 && (v.tread_depth - 0.275).abs() < 1e-12);
        let run = &v.runs[0];
        assert!((run.top - 16.0 * 3.0 / 17.0).abs() < 1e-12 && (run.path[0].end[0] - 4.125).abs() < 1e-12);
        assert_eq!(run.footprint.len(), 4);
        assert_eq!((run.footprint[0].start, run.footprint[0].end), ([4.125, 0.5, 0.0], [0.0, 0.5, 0.0]));
    }

    #[test]
    fn left_and_right_justification_move_the_path_by_half_the_width() {
        let mut f = fixture();
        let (l0, l1) = (f.l0, f.l1);
        let a = f.doc.transact("l", false, |m, cx| create_scene(m, cx, straight(l0, l1, (40.0, 0.0), (44.0, 0.0), Justification::Left))).value.unwrap();
        let b = f.doc.transact("r", false, |m, cx| create_scene(m, cx, straight(l0, l1, (50.0, 0.0), (54.0, 0.0), Justification::Right))).value.unwrap();
        assert_eq!(scene_view(f.doc.model(), a.stairs.unwrap()).unwrap().runs[0].path[0].start, [40.0, -0.5, 0.0]);
        assert_eq!(scene_view(f.doc.model(), b.stairs.unwrap()).unwrap().runs[0].path[0].start, [50.0, 0.5, 0.0]);
    }

    #[test]
    fn a_top_level_not_above_the_base_is_refused_and_no_component_means_no_stairs() {
        let mut f = fixture();
        let (l0, l1) = (f.l0, f.l1);
        let bad = f.doc.transact("bad", false, |m, cx| create_scene(m, cx, straight(l1, l0, (0.0, 0.0), (4.0, 0.0), Justification::Center)));
        assert!(bad.value.is_none());
        let none = f
            .doc
            .transact("none", false, |m, cx| create_scene(m, cx, NewScene { runs: Vec::new(), ..straight(l0, l1, (0.0, 0.0), (4.0, 0.0), Justification::Center) }))
            .value
            .unwrap();
        assert!(none.stairs.is_none() && f.doc.model().stairs.stairs.is_empty());
    }

    #[test]
    fn a_second_railing_set_on_a_stairs_with_railings_is_refused() {
        let mut f = fixture();
        let (l0, l1) = (f.l0, f.l1);
        let r = f.doc.transact("s", false, |m, cx| create_scene(m, cx, straight(l0, l1, (0.0, 0.0), (4.0, 0.0), Justification::Center))).value.unwrap();
        let s = r.stairs.unwrap();
        let out = f.doc.transact("rail", false, |m, cx| {
            let rt = template_railing_type(m, cx, 0)?;
            create_railing_on_stairs(m, cx, s, rt)
        });
        assert!(out.value.is_none());
        assert_eq!(out.failures[0].code, codes::RAILING_STAIRS_HAS_RAILINGS);
    }

    #[test]
    fn the_base_elevation_of_a_run_snaps_to_a_riser_and_keeps_its_top() {
        // OBSERVED (`u1b`): base 1.5 snapped to 9 risers (1.588), top kept at 12 risers, 3 risers, 2 treads: a 0.55 m run
        // that ends where it ended
        let mut f = fixture();
        let (l0, l1) = (f.l0, f.l1);
        let mut spec = straight(l0, l1, (3.0, 1.8), (0.0, 1.8), Justification::Center);
        spec.runs[0].base_elevation = Some(1.5);
        let r = f.doc.transact("u", false, |m, cx| create_scene(m, cx, spec)).value.unwrap();
        let v = scene_view(f.doc.model(), r.stairs.unwrap()).unwrap();
        let run = &v.runs[0];
        assert!((run.base - 9.0 * 3.0 / 17.0).abs() < 1e-12 && (run.top - 12.0 * 3.0 / 17.0).abs() < 1e-9, "{} {}", run.base, run.top);
        assert_eq!((run.risers, run.treads), (3, 2));
        assert!((run.path[0].end[0] - (-0.025)).abs() < 1e-9 && (run.path[0].start[0] - 0.525).abs() < 1e-9, "{:?}", run.path);
    }

    #[test]
    fn a_desired_number_is_a_lower_bound_of_the_riser_count() {
        let mut f = fixture();
        let (l0, l1) = (f.l0, f.l1);
        let r = f.doc.transact("s", false, |m, cx| create_scene(m, cx, straight(l0, l1, (0.0, 0.0), (4.0, 0.0), Justification::Center))).value.unwrap();
        let s = r.stairs.unwrap();
        f.doc.transact("14", false, |m, cx| set_desired_risers(m, cx, s, 14));
        assert_eq!(scene_view(f.doc.model(), s).unwrap().desired_risers, 17);
        f.doc.transact("30", false, |m, cx| set_desired_risers(m, cx, s, 30));
        let v = scene_view(f.doc.model(), s).unwrap();
        assert_eq!((v.desired_risers, v.actual_risers), (30, 16));
        assert!((v.runs[0].top - 1.6).abs() < 1e-9);
        // a count below ceil(height / max riser) is raised to it again; the flag is cleared by the first regeneration
        // and the stored actual number of risers is the user's count
        f.doc.transact("14b", false, |m, cx| set_desired_risers(m, cx, s, 14));
        assert_eq!(scene_view(f.doc.model(), s).unwrap().desired_risers, 17);
        assert!(!f.doc.model().stairs.stairs[&s].user_changed_riser_height);
        assert_eq!(stored_actual_number_of_risers(f.doc.model(), s), Some(14));
        assert!(f.doc.transact("0", false, |m, cx| set_desired_risers(m, cx, s, 0)).value.is_none());
    }

    #[test]
    fn the_monolithic_template_types_have_no_supports_and_their_own_run_and_landing_types() {
        // OBSERVED: stairs of "Monolithic Stair" and "Precast Stair" have 2 railings and no supports
        let mut f = fixture();
        let (l0, l1) = (f.l0, f.l1);
        for (index, depth, landing) in [(1usize, 0.15, 0.3), (2, 0.19, 0.22)] {
            let ty = f.doc.transact("t", false, |m, cx| template_stairs_type(m, cx, index)).value.unwrap();
            let spec = NewScene { type_id: Some(ty), ..straight(l0, l1, (0.0, 0.0), (4.0, 0.0), Justification::Center) };
            let r = f.doc.transact("s", false, |m, cx| create_scene(m, cx, spec)).value.unwrap();
            let v = scene_view(f.doc.model(), r.stairs.unwrap()).unwrap();
            assert_eq!((v.supports, v.railings.len(), v.actual_risers), (0, 2, 16));
            let ext = &f.doc.model().stairs_scene.types[&ty];
            let run_type = &f.doc.model().stairs_scene.run_types[&ext.run_type];
            assert!((run_type.structural_depth - depth).abs() < 1e-12 && (ext.landing_thickness - landing).abs() < 1e-12 && run_type.is_monolithic);
        }
        assert!(f.doc.transact("t", false, |m, cx| template_stairs_type(m, cx, 3)).value.is_none());
    }

    #[test]
    fn a_spiral_run_is_rounded_to_whole_treads_and_turns_the_way_the_api_says() {
        // OBSERVED (`sp`): half a turn at radius 2.0 with `clockwise` true: 24 risers, 23 treads, and the arc ends 0.0234
        // rad before the half turn, reached clockwise
        let mut f = fixture();
        let (l0, l1) = (f.l0, f.l1);
        let spec = NewScene {
            runs: vec![SceneRunSpec {
                run: SceneRun::Spiral {
                    geom: SpiralGeom { center: Vec2::new(45.0, 0.0), radius: 2.0, start_angle: 0.0, included_angle: std::f64::consts::PI, clockwise: true },
                    justification: Justification::Center,
                },
                base_elevation: None,
            }],
            ..straight(l0, l1, (0.0, 0.0), (4.0, 0.0), Justification::Center)
        };
        let r = f.doc.transact("sp", false, |m, cx| create_scene(m, cx, spec)).value.unwrap();
        let v = scene_view(f.doc.model(), r.stairs.unwrap()).unwrap();
        let run = &v.runs[0];
        assert_eq!((run.risers, run.treads), (24, 23));
        assert!((run.path[0].end[0] - 43.000547719993186).abs() < 1e-9 && (run.path[0].end[1] - 0.04680363207639597).abs() < 1e-9, "{:?}", run.path);
        assert_eq!(run.footprint.len(), 4);
        assert!((run.footprint[0].start[0] - 42.50068464999148).abs() < 1e-9 && (run.footprint[2].end[0] - 43.50041078999489).abs() < 1e-9);
    }

    #[test]
    fn a_copy_carries_runs_and_railings_and_a_move_translates_them() {
        let mut f = fixture();
        let (l0, l1) = (f.l0, f.l1);
        let r = f.doc.transact("s", false, |m, cx| create_scene(m, cx, straight(l0, l1, (0.0, 0.0), (4.0, 0.0), Justification::Center))).value.unwrap();
        let s = r.stairs.unwrap();
        let c = f.doc.transact("copy", false, |m, cx| copy_stairs(m, cx, s, Vec2::new(0.0, 12.0))).value.unwrap();
        let v = scene_view(f.doc.model(), c).unwrap();
        assert_eq!((v.runs.len(), v.railings.len()), (1, 2));
        assert!((v.runs[0].path[0].start[1] - 12.0).abs() < 1e-12);
        assert_eq!(scene_view(f.doc.model(), s).unwrap().runs[0].path[0].start[1], 0.0);
    }
}
