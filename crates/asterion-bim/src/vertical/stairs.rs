//! Stair by run: riser count, run length to risers, landing elevation, railings, failure channels.
//!
//! EVERY rule here comes from one of five specification rules (the reference product's stair database, the reference
//! product, read statically):
//!
//! | row | decides |
//! |---|---|
//! | | the stored actual riser height, the desired number of risers (derived), setting it, creation, the regeneration riser step, the three warnings |
//! | | run length -> risers: location-line rounding, the riser count, the top elevation, the inverse length, the spiral count |
//! | Spec S-01c | landing base elevation snapped to whole risers, its validity, the automatic-landing pairings |
//! | | railings: one per railing path, the count check, the exclusion list, the failure |
//! | | which failure is raised at the call, at parameter set, or posted at commit |
//!
//! # Units
//!
//! The model stores metres (project rule). Every absolute tolerance the rules quote (`1e-9`, the vertex tolerance) is in
//! the reference product's internal unit, the foot (the same convention as the wall rules). Each rule below is therefore
//! evaluated in feet: operands are converted with [`FEET_PER_METRE`] on the way in and stored values are converted back
//! with `feet_to_metres`. The rules do not name the unit of the `1e-9`; that it is the internal foot is the engine's
//! convention, not a read fact (open question 1 in the report).
//!
//! # What is UNKNOWN (never filled with a guess)
//!
//! Each item is either an `Option` input that defaults to "not evaluated", an explicit argument, or
//! a refusal with [`codes::STAIRS_UNDECIDED_BRANCH`].
//!
//! * the precedence of the two special cases of the desired number of risers when both hold
//!   (riser height first, as the row lists them);
//! * the vertex tolerance: which type and version the stair operations build is NOT specified; [`vertex_eps`] returns the
//!   value of the default-constructed controller, the single place to replace.
//! * creation when the type's maximum riser height is not above 1e-9, when `n` is below 1, and the
//!   initial tread depth before it is raised to the type minimum (an explicit argument);
//! * the writer and initial value of the integer ([`Stairs::riser_count_floor`]).
//! * whether the regeneration store goes through (stored directly here).
//! * severity codes: 1 is Warning, 2 is Error, 3 is DocumentCorruption; [`RISER_HEIGHT_TOO_SMALL_SEVERITY`] (code 2, Error)
//!   is the single switch, and the three limit warnings are code 1; UNKNOWN: the default failures processor.
//! * the conditions of the top-end and bottom-end warnings, the automatic landing boundary, the
//!   proper-run-pair test, multistory landings and railings, sketched and winder runs, spiral run
//!   rounding, the railing offset, `RailingPlacementPosition`, deletion cascades, how the stairs
//!   bottom and top derive from levels and offsets (levels' elevations are used as given).

use crate::model::BimModel;
use crate::spec::{codes, FEET_PER_METRE};
use asterion_core::txn::TxnContext;
use asterion_core::units::feet_to_metres;
use asterion_core::{ElementId, Failure, Severity};
use asterion_geom::Vec2;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ------------------------------------------------------------------------------------ constants

/// Spec S-01c: the absolute guard of the stair operations, in internal units (feet).
pub const EPS: f64 = 1e-9;

/// Replace here when that call site is read.
pub const DEFAULT_VERTEX_EPS_FT: f64 = 0.0005233832795;

/// The vertex tolerance of the stair operations, in feet. See [`DEFAULT_VERTEX_EPS_FT`].
pub fn vertex_eps() -> f64 {
    DEFAULT_VERTEX_EPS_FT
}

/// The registered severity literal 2 is `Error` (0 None, 1 Warning, 2 Error, 3 DocumentCorruption), and the three limit
/// warnings carry code 1, `Warning`. With a posted error and no failures processor registered the commit rolls back.
/// UNKNOWN: what a registered failures processor does with the error.
pub const RISER_HEIGHT_TOO_SMALL_SEVERITY: Severity = Severity::Error;

fn ft(metres: f64) -> f64 {
    metres * FEET_PER_METRE
}

fn metres(feet: f64) -> f64 {
    feet_to_metres(feet)
}

// -------------------------------------------------------------------------- pure rules (feet)
//
// Every function below takes and returns internal units (feet) and is a pure function of its
// arguments, so each is tested on the exact numbers of the rule.

/// Spec S-01c: "stores the value only when it differs from the stored one by more than 1e-9 relative". UNKNOWN: the base of
/// the relative test. The stored value is used.
pub fn differs_by_more_than_relative_eps(new: f64, stored: f64) -> bool {
    (new - stored).abs() > EPS * stored.abs()
}

/// `(int)(stairs height / actual riser height + 0.5)`; 0 when the riser height is not above 1e-9; 1 when the stairs height
/// is not above 1e-9.
///
/// UNKNOWN: which special case wins when both hold. The rule lists the riser height first, and so does this function.
pub fn desired_number_of_risers(stairs_height: f64, actual_riser_height: f64) -> i32 {
    if !(actual_riser_height > EPS) {
        return 0;
    }
    if !(stairs_height > EPS) {
        return 1;
    }
    (stairs_height / actual_riser_height + 0.5) as i32
}

/// "accepted only when n > 0 and differs from the current value".
pub fn desired_number_change_applies(n: i32, current: i32) -> bool {
    n > 0 && n != current
}

/// CREATION: with the type's maximum riser height above 1e-9, `n = (int)(ceil(stairs height / max riser height) + 1e-9)`.
/// `None` when the maximum is not above 1e-9: UNKNOWN, the rule gives no other branch.
pub fn creation_riser_count(stairs_height: f64, max_riser_height: f64) -> Option<i32> {
    if !(max_riser_height > EPS) {
        return None;
    }
    let x = (stairs_height / max_riser_height).ceil() + EPS;
    if !x.is_finite() {
        return None;
    }
    Some(x as i32)
}

/// Input of the riser regeneration step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiserStepInput {
    /// Stairs height, feet.
    pub stairs_height: f64,
    /// The type's maximum riser height, feet.
    pub max_riser_height: f64,
    /// The stored actual riser height, feet.
    pub actual_riser_height: f64,
    pub user_changed_riser_height: bool,
    pub multistory_member: bool,
    /// The integer. UNKNOWN: writer and initial value.
    pub riser_count_floor: Option<i32>,
    pub vertex_eps: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RiserStepOutcome {
    /// "when the max riser height is not above 1e-9 or the stairs height is below 1e-9 nothing is done".
    NothingToDo,
    /// The user-changed flag was set on a stairs that is not a multistory member: the step ends
    /// keeping the user's riser height.
    KeptUserRiserHeight,
    /// `riser height = height / n` is stored.
    Stored { risers: i32, riser_height: f64 },
    /// `riser height = height / n` is below the vertex tolerance: the failure is posted instead.
    RiserHeightTooSmall { risers: i32, riser_height: f64 },
    Undecided,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiserStep {
    /// "when the user-changed flag is set it is cleared".
    pub clear_user_flag: bool,
    pub outcome: RiserStepOutcome,
}

/// REGENERATION (riser step): when the maximum riser height is not above 1e-9 or the stairs height is below 1e-9 nothing is
/// done; when the user-changed flag is set it is cleared and, unless the stairs is a multistory member, the step ends
/// keeping the user's riser height; otherwise `n = ceil(height / max riser height - 1e-9)` (the tolerance is subtracted
/// before the ceil); for a multistory member `n = ceil(height / current actual riser height - 1e-9)`; for a single stairs
/// `n` is the larger of that value and the integer; `riser height = height / n`; below the vertex tolerance the failure is
/// posted, else it is stored.
pub fn riser_regeneration_step(i: &RiserStepInput) -> RiserStep {
    if !(i.max_riser_height > EPS) || i.stairs_height < EPS {
        return RiserStep { clear_user_flag: false, outcome: RiserStepOutcome::NothingToDo };
    }
    let clear = i.user_changed_riser_height;
    if clear && !i.multistory_member {
        return RiserStep { clear_user_flag: true, outcome: RiserStepOutcome::KeptUserRiserHeight };
    }
    let divisor = if i.multistory_member { i.actual_riser_height } else { i.max_riser_height };
    let x = i.stairs_height / divisor;
    let undecided = RiserStep { clear_user_flag: clear, outcome: RiserStepOutcome::Undecided };
    if !x.is_finite() {
        return undecided;
    }
    let mut n = (x - EPS).ceil() as i32;
    if !i.multistory_member {
        if let Some(floor) = i.riser_count_floor {
            n = n.max(floor);
        }
    }
    if n < 1 {
        return undecided;
    }
    let riser_height = i.stairs_height / n as f64;
    let outcome = if riser_height < i.vertex_eps {
        RiserStepOutcome::RiserHeightTooSmall { risers: n, riser_height }
    } else {
        RiserStepOutcome::Stored { risers: n, riser_height }
    };
    RiserStep { clear_user_flag: clear, outcome }
}

/// WARNINGS: "actual riser height greater than max riser height + 1e-9".
pub fn riser_height_above_maximum(actual_riser_height: f64, max_riser_height: f64) -> bool {
    actual_riser_height > max_riser_height + EPS
}

/// WARNINGS: "for stairs with a component run, type minimum tread depth greater than the actual tread depth".
/// `has_component_run` is the caller's.
pub fn tread_depth_below_type_minimum(has_component_run: bool, min_tread_depth: f64, actual_tread_depth: f64) -> bool {
    has_component_run && min_tread_depth > actual_tread_depth
}

/// WARNINGS: "a run (not run type 2) whose actual width is below the type's minimum run width".
pub fn run_width_below_type_minimum(run_type_code: u8, actual_width: f64, min_run_width: f64) -> bool {
    run_type_code != 2 && actual_width < min_run_width
}

/// "Created risers = floor((top of the top component - stairs bottom) / riser height + 1e-9)".
pub fn created_risers(top_of_top_component: f64, stairs_bottom: f64, riser_height: f64) -> i32 {
    ((top_of_top_component - stairs_bottom) / riser_height + EPS).floor() as i32
}

/// "remaining risers = round(height / riser height) - created".
pub fn remaining_risers(stairs_height: f64, riser_height: f64, created: i32) -> i32 {
    (stairs_height / riser_height).round() as i32 - created
}

/// For a line: with tread depth `T` and curve length `L`, when `T <= L` the end is moved to `floor(L / T + 0.5) x T` from
/// the start, otherwise to exactly `T`. Returns the new curve length. `T` must be above 1e-9 (the caller's check).
pub fn round_run_length(length: f64, tread_depth: f64) -> f64 {
    if tread_depth <= length {
        (length / tread_depth + 0.5).floor() * tread_depth
    } else {
        tread_depth
    }
}

/// `floor(L / T + 1.5) - (1 when the run does not begin with a riser) - (1 when it does not end with a riser)`. May be
/// negative; the caller rejects that.
pub fn count_risers_for_run(length: f64, tread_depth: f64, begin_with_riser: bool, end_with_riser: bool) -> i32 {
    (length / tread_depth + 1.5).floor() as i32 - i32::from(!begin_with_riser) - i32::from(!end_with_riser)
}

/// Spiral run: "the same count formula with L replaced by the arc angle and T by the angle per tread = 2 x asin(T / (2R))
/// (T / R when T / 2 > R)". `None` when `R` is not positive.
pub fn angle_per_tread(tread_depth: f64, radius: f64) -> Option<f64> {
    if !(radius > 0.0) {
        return None;
    }
    if tread_depth / 2.0 > radius {
        Some(tread_depth / radius)
    } else {
        Some(2.0 * (tread_depth / (2.0 * radius)).asin())
    }
}

/// The spiral count, the straight formula over angles.
pub fn spiral_riser_number(arc_angle: f64, angle_per_tread: f64, begin_with_riser: bool, end_with_riser: bool) -> i32 {
    count_risers_for_run(arc_angle, angle_per_tread, begin_with_riser, end_with_riser)
}

/// "top elevation = bottom + risers x actual riser height".
pub fn run_top_elevation(bottom_elevation: f64, risers: i32, actual_riser_height: f64) -> f64 {
    bottom_elevation + risers as f64 * actual_riser_height
}

/// Inverse, for a line: "desired risers of the run = floor((stairs top elevation - run base elevation) / actual riser
/// height + 0.5) (floor(stairs height / riser height + 0.5) when that is below 2)".
pub fn desired_risers_of_run(stairs_top: f64, run_base: f64, stairs_height: f64, riser_height: f64) -> i32 {
    let n = ((stairs_top - run_base) / riser_height + 0.5).floor() as i32;
    if n < 2 {
        (stairs_height / riser_height + 0.5).floor() as i32
    } else {
        n
    }
}

/// "run length = T x (desired risers of the run + 1 per missing begin riser + 1 per missing end riser - 1) for a line".
pub fn run_total_length(tread_depth: f64, desired_risers: i32, begin_with_riser: bool, end_with_riser: bool) -> f64 {
    tread_depth * (desired_risers + i32::from(!begin_with_riser) + i32::from(!end_with_riser) - 1) as f64
}

/// Spec S-01c: "n = round half away from zero of (requested elevation - stairs bottom elevation) / riser height". `None`
/// when the riser height is not a positive number.
pub fn round_riser_number(relative_elevation: f64, riser_height: f64) -> Option<i32> {
    if !(riser_height > 0.0) || !relative_elevation.is_finite() {
        return None;
    }
    let n = (relative_elevation / riser_height).round();
    n.is_finite().then_some(n as i32)
}

/// Spec S-01c: "elevation = stairs bottom + n x riser height, clamped to the stairs bottom elevation when below it, and the
/// landing's base riser index is that n". Returns `(elevation, n)`.
pub fn snap_landing_elevation(requested: f64, stairs_bottom: f64, riser_height: f64) -> Option<(f64, i32)> {
    let n = round_riser_number(requested - stairs_bottom, riser_height)?;
    let e = stairs_bottom + n as f64 * riser_height;
    Some((if e < stairs_bottom { stairs_bottom } else { e }, n))
}

/// Spec S-01c: "a base elevation is valid when its rounded riser count (relative to the stairs) is above 0".
pub fn landing_riser_count_is_valid(relative_elevation: f64, riser_height: f64) -> bool {
    round_riser_number(relative_elevation, riser_height).map_or(false, |n| n > 0)
}

/// Spec S-01c: "the instance check also requires the elevation to be at least the run/landing tread thickness". UNKNOWN
/// (Spec S-01c): whose thickness and in which frame the elevation is taken; the caller supplies both, and `None` means the
/// check is not evaluated.
pub fn landing_elevation_meets_tread_thickness(relative_elevation: f64, tread_thickness: Option<f64>) -> bool {
    tread_thickness.map_or(true, |t| relative_elevation >= t)
}

/// An end of a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunEnd {
    End1,
    End2,
}

/// One way two runs can be connected: `a` is the end of run A, `b` the end of run B.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EndPairing {
    pub a: RunEnd,
    pub b: RunEnd,
}

/// Spec S-01c: the four end pairings, in the order the rule lists them: A end2 to B end1, B end2 to A end1, end1 to end1
/// "with both runs above the stairs bottom + 1e-9", end2 to end2. UNKNOWN (Spec S-01c): which run elevation "above"
/// compares (the bottom is used).
pub fn automatic_landing_pairings(run_a_bottom: f64, run_b_bottom: f64, stairs_bottom: f64) -> Vec<EndPairing> {
    let mut v = vec![
        EndPairing { a: RunEnd::End2, b: RunEnd::End1 },
        EndPairing { a: RunEnd::End1, b: RunEnd::End2 },
    ];
    if run_a_bottom > stairs_bottom + EPS && run_b_bottom > stairs_bottom + EPS {
        v.push(EndPairing { a: RunEnd::End1, b: RunEnd::End1 });
    }
    v.push(EndPairing { a: RunEnd::End2, b: RunEnd::End2 });
    v
}

/// Spec S-01c: two different runs that form a proper run pair, the landing type of run A valid, no logic joins on the ends
/// being connected and, for at least one of the end pairings, a landing boundary that is computed without error.
pub fn landing_can_be_generated(
    run_a: ElementId,
    run_b: ElementId,
    proper_run_pair: bool,
    landing_type_valid: bool,
    pairings: &[EndPairing],
    mut ends_have_no_logic_joins: impl FnMut(EndPairing) -> bool,
    mut boundary_computes: impl FnMut(EndPairing) -> bool,
) -> bool {
    run_a != run_b
        && proper_run_pair
        && landing_type_valid
        && pairings.iter().any(|p| ends_have_no_logic_joins(*p) && boundary_computes(*p))
}

// ----------------------------------------------------------- run validity
//
// "StairsRun validity (run types 1, 3 and 4)". Elevations are compared in one common frame (the rules say "relative"; the
// frame is UNKNOWN, and any common offset leaves the comparisons unchanged). Run types other than 1, 3 and 4 are not
// evaluated: their validity is not specified.

fn run_type_has_validity(run_type_code: u8) -> bool {
    matches!(run_type_code, 1 | 3 | 4)
}

fn reject(code: &str, message: &str) -> Failure {
    Failure::error(code, message)
}

/// "a new relative bottom elevation above the current top (by more than 1e-9) -> RunTopLowerThanBottomFailure, else a
/// bottom that leaves less than the minimum run height -> HeightOfRunIsTooSmall; ... a bottom under the stairs bottom ->
/// RunBottomUnderStairsBottomFailure". `min_run_height` is `None` when unknown (UNKNOWN): then that branch is not
/// evaluated. Rejection happens before any change: no commit is involved.
pub fn validate_run_bottom_elevation(
    run_type_code: u8,
    new_bottom: f64,
    current_top: f64,
    stairs_bottom: f64,
    min_run_height: Option<f64>,
) -> Result<(), Failure> {
    if !run_type_has_validity(run_type_code) {
        return Ok(());
    }
    if new_bottom > current_top + EPS {
        return Err(reject(codes::STAIRS_RUN_TOP_LOWER_THAN_BOTTOM, "The run top would be lower than its bottom."));
    }
    if let Some(min) = min_run_height {
        if current_top - new_bottom < min {
            return Err(reject(codes::STAIRS_RUN_HEIGHT_TOO_SMALL, "The run would be shorter than the minimum run height."));
        }
    }
    if new_bottom < stairs_bottom {
        return Err(reject(codes::STAIRS_RUN_BOTTOM_UNDER_STAIRS_BOTTOM, "The run bottom would be under the stairs bottom."));
    }
    Ok(())
}

/// "a new relative top elevation below the current bottom (by more than 1e-9) -> RunBottomHigherThanTopFailure, else a top
/// below the minimum run height -> HeightOfRunIsTooSmall". UNKNOWN: the formula of that second branch is not evaluated.
pub fn validate_run_top_elevation(run_type_code: u8, new_top: f64, current_bottom: f64) -> Result<(), Failure> {
    if !run_type_has_validity(run_type_code) {
        return Ok(());
    }
    if new_top < current_bottom - EPS {
        return Err(reject(codes::STAIRS_RUN_BOTTOM_HIGHER_THAN_TOP, "The run bottom would be higher than its top."));
    }
    Ok(())
}

/// "extend-below-base above the run top -> ExtendBelowBaseGreaterThanRunTopElevation".
pub fn validate_run_extend_below_base(run_type_code: u8, extend_below_base: f64, run_top: f64) -> Result<(), Failure> {
    if !run_type_has_validity(run_type_code) {
        return Ok(());
    }
    if extend_below_base > run_top {
        return Err(reject(codes::STAIRS_RUN_EXTEND_BELOW_BASE_ABOVE_TOP, "Extend-below-base is above the run top."));
    }
    Ok(())
}

// ------------------------------------------------------------------------------------- model

/// Lengths in metres.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StairsTypeParams {
    pub max_riser_height: f64,
    pub min_tread_depth: f64,
    pub min_run_width: f64,
    pub use_calculator: bool,
    /// The calculator target, metres.
    pub calculator_target: f64,
    pub calculator_riser_multiplier: f64,
    pub calculator_tread_multiplier: f64,
    /// UNKNOWN: the minimum run height.
    #[serde(default)]
    pub min_run_height: Option<f64>,
    /// UNKNOWN (Spec S-01c): the run/landing tread thickness of the instance check.
    #[serde(default)]
    pub tread_thickness: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StairsType {
    pub id: ElementId,
    pub name: String,
    pub params: StairsTypeParams,
}

/// A stairs element. Stored state only; the riser COUNT is derived ([`desired_number_of_risers`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stairs {
    pub id: ElementId,
    pub type_id: ElementId,
    pub base_level: ElementId,
    pub top_level: ElementId,
    /// "The stairs element stores the ACTUAL riser height". Metres.
    pub actual_riser_height: f64,
    /// Tread field. Metres.
    pub actual_tread_depth: f64,
    /// The user-changed-riser-height flag.
    pub user_changed_riser_height: bool,
    /// The integer used as lower bound by the regeneration step of a single stairs. UNKNOWN: writer, meaning, initial
    /// value; `None` = never written.
    #[serde(default)]
    pub riser_count_floor: Option<i32>,
    /// Membership of a multistory group. The group itself is not modelled.
    #[serde(default)]
    pub multistory_member: bool,
}

/// A straight component run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StairsRun {
    pub id: ElementId,
    pub stairs: ElementId,
    /// Location line in plan, already rounded by [`round_run_length`]. The path is planar here.
    pub start: Vec2,
    pub end: Vec2,
    /// "The run's bottom elevation is the start point z". Metres.
    pub bottom_elevation: f64,
    /// The run's actual width, metres.
    pub width: f64,
    pub begin_with_riser: bool,
    pub end_with_riser: bool,
    /// The numeric run type. UNKNOWN: the names.
    pub run_type_code: u8,
}

/// A landing (Spec S-01c). Only the base elevation rule is read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StairsLanding {
    pub id: ElementId,
    pub stairs: ElementId,
    /// Metres, always a whole number of actual riser heights above the stairs bottom.
    pub base_elevation: f64,
    /// Spec S-01c: "the landing's base riser index is that n"; `None` = unset.
    pub base_riser_index: Option<i32>,
}

/// Opaque stairs sub-element key of a railing path. The structure of a key is not specified.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StairsSubelementKey(pub u64);

/// One railing path of a stairs: the boundary curve loop, identified by its sub-element keys.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RailingPath {
    pub keys: Vec<StairsSubelementKey>,
}

/// What the stairs' path provider answers: the paths and the count it reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RailingPathSet {
    pub paths: Vec<RailingPath>,
    pub reported_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StairsRailing {
    pub id: ElementId,
    pub stairs: ElementId,
    /// Opaque: the railing type is not validated.
    pub railing_type: ElementId,
    pub path_index: usize,
    /// "stores the stairs sub-element keys of the path on the railing".
    pub subelement_keys: Vec<StairsSubelementKey>,
}

/// All stair state of a model.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StairsModel {
    #[serde(default)]
    pub types: BTreeMap<ElementId, StairsType>,
    #[serde(default)]
    pub stairs: BTreeMap<ElementId, Stairs>,
    #[serde(default)]
    pub runs: BTreeMap<ElementId, StairsRun>,
    #[serde(default)]
    pub landings: BTreeMap<ElementId, StairsLanding>,
    #[serde(default)]
    pub railings: BTreeMap<ElementId, StairsRailing>,
}

fn not_found(id: ElementId) -> Failure {
    Failure::error(codes::NOT_FOUND, format!("No element with id {id}.")).on(&[id])
}

fn undecided(why: &str) -> Failure {
    Failure::error(
        codes::STAIRS_UNDECIDED_BRANCH,
        format!("No specified rule decides this case: {why}. The engine refuses it instead of choosing."),
    )
}

// ------------------------------------------------------------------------------ derived state

/// Derived state of one run (pure function of the model).
#[derive(Clone, Debug, PartialEq)]
pub struct RunState {
    pub id: ElementId,
    /// Plan length of the (rounded) location line, metres.
    pub length: f64,
    /// Over the run's line and the stairs' actual tread depth.
    pub risers: i32,
    pub bottom_elevation: f64,
    pub top_elevation: f64,
}

/// Derived state of a stairs (pure function of the model; never stored).
#[derive(Clone, Debug, PartialEq)]
pub struct StairsState {
    /// The elevation of the base level (offsets are not modelled).
    pub bottom_elevation: f64,
    pub top_elevation: f64,
    pub height: f64,
    pub actual_riser_height: f64,
    pub actual_tread_depth: f64,
    pub desired_number_of_risers: i32,
    pub runs: Vec<RunState>,
    /// "Created risers". `None` without a run, and when a landing exists: the top of a landing component is not specified.
    pub created_risers: Option<i32>,
    pub remaining_risers: Option<i32>,
}

impl StairsModel {
    pub fn stairs(&self, id: ElementId) -> Result<&Stairs, Failure> {
        self.stairs.get(&id).ok_or_else(|| not_found(id))
    }

    pub fn stairs_type(&self, id: ElementId) -> Result<&StairsType, Failure> {
        self.types.get(&id).ok_or_else(|| not_found(id))
    }

    /// Runs of one stairs, in id order.
    pub fn runs_of(&self, stairs: ElementId) -> Vec<&StairsRun> {
        self.runs.values().filter(|r| r.stairs == stairs).collect()
    }

    /// Stairs that depend on `id`: their type, their base level or their top level (the parents of the regeneration step:
    /// "stairs elevation-span atoms" and "the type's atom").
    pub fn stairs_depending_on(&self, id: ElementId) -> Vec<ElementId> {
        self.stairs
            .values()
            .filter(|s| s.id == id || s.type_id == id || s.base_level == id || s.top_level == id)
            .map(|s| s.id)
            .collect()
    }
}

fn level_elevation(m: &BimModel, id: ElementId) -> Result<f64, Failure> {
    m.levels.get(&id).map(|l| l.elevation).ok_or_else(|| not_found(id))
}

/// `(bottom, top)` elevations in metres. UNKNOWN: how the reference product derives the stairs bottom and top (base and top
/// offsets); the levels' elevations are used.
fn stairs_span(m: &BimModel, s: &Stairs) -> Result<(f64, f64), Failure> {
    Ok((level_elevation(m, s.base_level)?, level_elevation(m, s.top_level)?))
}

fn run_state_of(s: &Stairs, run: &StairsRun) -> Result<RunState, Failure> {
    let tread = ft(s.actual_tread_depth);
    let length = run.start.distance_to(run.end);
    if !(tread > EPS) {
        return Err(undecided("the stairs' actual tread depth is not above 1e-9, so a run has no riser count"));
    }
    let risers = count_risers_for_run(ft(length), tread, run.begin_with_riser, run.end_with_riser);
    Ok(RunState {
        id: run.id,
        length,
        risers,
        bottom_elevation: run.bottom_elevation,
        top_elevation: metres(run_top_elevation(ft(run.bottom_elevation), risers, ft(s.actual_riser_height))),
    })
}

/// Derived state of a stairs: a pure function of the model, recomputed on every read.
pub fn stairs_state(m: &BimModel, id: ElementId) -> Result<StairsState, Failure> {
    let s = m.stairs.stairs(id)?;
    let (bottom, top) = stairs_span(m, s)?;
    let height = top - bottom;
    let riser_ft = ft(s.actual_riser_height);
    let mut runs = Vec::new();
    for r in m.stairs.runs_of(id) {
        runs.push(run_state_of(s, r)?);
    }
    let has_landing = m.stairs.landings.values().any(|l| l.stairs == id);
    let (created, remaining) = match runs.iter().map(|r| r.top_elevation).reduce(f64::max) {
        Some(top_of_top) if !has_landing && riser_ft > EPS => {
            let c = created_risers(ft(top_of_top), ft(bottom), riser_ft);
            (Some(c), Some(remaining_risers(ft(height), riser_ft, c)))
        }
        _ => (None, None),
    };
    Ok(StairsState {
        bottom_elevation: bottom,
        top_elevation: top,
        height,
        actual_riser_height: s.actual_riser_height,
        actual_tread_depth: s.actual_tread_depth,
        desired_number_of_risers: desired_number_of_risers(ft(height), riser_ft),
        runs,
        created_risers: created,
        remaining_risers: remaining,
    })
}

// ------------------------------------------------------------------------------- regeneration

/// On a stairs, in feet: stores the value only when it differs from the stored one by more than 1e-9 relative, touches the
/// element and, when the type uses the calculator and its tread multiplier is non-zero, sets the actual tread depth to
/// `(target - riser multiplier x riser height) / tread multiplier`, never below the type's minimum tread depth.
fn set_actual_riser_height_(s: &mut Stairs, ty: &StairsTypeParams, new_riser_ft: f64) {
    if !differs_by_more_than_relative_eps(new_riser_ft, ft(s.actual_riser_height)) {
        return;
    }
    s.actual_riser_height = metres(new_riser_ft);
    if ty.use_calculator && ty.calculator_tread_multiplier != 0.0 {
        let tread = (ft(ty.calculator_target) - ty.calculator_riser_multiplier * new_riser_ft)
            / ty.calculator_tread_multiplier;
        s.actual_tread_depth = metres(tread.max(ft(ty.min_tread_depth)));
    }
}

/// Run the regeneration of one stairs and return the failures it posts (nothing is reported here).
///
/// Order: the riser step, then the warning step, which sees the stored result. UNKNOWN: the relative order of the two steps
/// is not specified.
fn regenerate_collect(m: &mut BimModel, id: ElementId) -> Result<Vec<Failure>, Failure> {
    let stairs = m.stairs.stairs(id)?.clone();
    let ty = m.stairs.stairs_type(stairs.type_id)?.params;
    let (bottom, top) = stairs_span(m, &stairs)?;
    let mut posted = Vec::new();

    let step = riser_regeneration_step(&RiserStepInput {
        stairs_height: ft(top - bottom),
        max_riser_height: ft(ty.max_riser_height),
        actual_riser_height: ft(stairs.actual_riser_height),
        user_changed_riser_height: stairs.user_changed_riser_height,
        multistory_member: stairs.multistory_member,
        riser_count_floor: stairs.riser_count_floor,
        vertex_eps: vertex_eps(),
    });
    {
        let s = m.stairs.stairs.get_mut(&id).expect("looked up above");
        if step.clear_user_flag {
            s.user_changed_riser_height = false;
        }
        match step.outcome {
            RiserStepOutcome::NothingToDo | RiserStepOutcome::KeptUserRiserHeight => {}
            // "else it is stored": written directly.
            RiserStepOutcome::Stored { riser_height, .. } => s.actual_riser_height = metres(riser_height),
            RiserStepOutcome::RiserHeightTooSmall { riser_height, .. } => {
                let mut f = Failure::error(
                    codes::STAIRS_RISER_HEIGHT_TOO_SMALL,
                    format!("The riser height {:.6} m would be below the vertex tolerance.", metres(riser_height)),
                )
                .on(&[id]);
                f.severity = RISER_HEIGHT_TOO_SMALL_SEVERITY;
                posted.push(f);
            }
            RiserStepOutcome::Undecided => {
                return Err(undecided("the riser step reached a non-finite ratio or fewer than one riser"))
            }
        }
    }

    let s = m.stairs.stairs(id)?;
    if riser_height_above_maximum(ft(s.actual_riser_height), ft(ty.max_riser_height)) {
        posted.push(
            Failure::warning(
                codes::STAIRS_RISER_HEIGHT_ABOVE_MAXIMUM,
                "The actual riser height is greater than the type's maximum riser height.",
            )
            .on(&[id]),
        );
    }
    let runs = m.stairs.runs_of(id);
    if tread_depth_below_type_minimum(!runs.is_empty(), ft(ty.min_tread_depth), ft(s.actual_tread_depth)) {
        posted.push(
            Failure::warning(
                codes::STAIRS_TREAD_DEPTH_LESS_THAN_MINIMUM,
                "The actual tread depth is less than the type's minimum tread depth.",
            )
            .on(&[id]),
        );
    }
    let narrow: Vec<ElementId> = runs
        .iter()
        .filter(|r| run_width_below_type_minimum(r.run_type_code, r.width, ty.min_run_width))
        .map(|r| r.id)
        .collect();
    if !narrow.is_empty() {
        // UNKNOWN: whether the reference product posts one warning per run or per stairs.
        let mut ids = vec![id];
        ids.extend(narrow);
        posted.push(
            Failure::warning(
                codes::STAIRS_RUN_WIDTH_LESS_THAN_MINIMUM,
                "A run's actual width is less than the type's minimum run width.",
            )
            .on(&ids),
        );
    }
    Ok(posted)
}

/// Regenerate one stairs at commit: posts its failures through the transaction (a warning keeps the
/// commit, an error rolls it back). An undecided branch is returned as `Err`.
pub fn regenerate_stairs(m: &mut BimModel, cx: &mut TxnContext<'_>, id: ElementId) -> Result<(), Failure> {
    for f in regenerate_collect(m, id)? {
        cx.report(f);
    }
    Ok(())
}

/// Regenerate every listed stairs that exists (the commit-time revisit of the dependents of an
/// edit, as `ops::check_walls` does for walls). `ops::set_level_elevation` calls it for the stairs
/// that name the moved level as base or top level; `set_stairs_type_params` for the type's stairs.
pub fn check_stairs(m: &mut BimModel, cx: &mut TxnContext<'_>, ids: &[ElementId]) {
    for id in ids {
        if m.stairs.stairs.contains_key(id) {
            if let Err(f) = regenerate_stairs(m, cx, *id) {
                cx.report(f);
            }
        }
    }
}

// ------------------------------------------------------------------------------- operations

fn require_finite(values: &[f64], what: &str) -> Result<(), Failure> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(undecided(&format!("a non-finite number was given for {what}")))
    }
}

/// Create a stairs type. No validation is read for the type's own parameters.
pub fn create_stairs_type(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    name: &str,
    params: StairsTypeParams,
) -> Result<ElementId, Failure> {
    require_finite(
        &[
            params.max_riser_height,
            params.min_tread_depth,
            params.min_run_width,
            params.calculator_target,
            params.calculator_riser_multiplier,
            params.calculator_tread_multiplier,
        ],
        "a stairs type parameter",
    )?;
    let id = cx.new_id();
    m.stairs.types.insert(id, StairsType { id, name: name.to_owned(), params });
    Ok(id)
}

/// Replace the parameters of a stairs type and regenerate the stairs of that type (the type's atom is a parent of the riser
/// step).
pub fn set_stairs_type_params(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    type_id: ElementId,
    params: StairsTypeParams,
) -> Result<(), Failure> {
    m.stairs.stairs_type(type_id)?;
    m.stairs.types.get_mut(&type_id).expect("looked up above").params = params;
    let deps = m.stairs.stairs_depending_on(type_id);
    check_stairs(m, cx, &deps);
    Ok(())
}

/// Input of [`create_stairs`].
#[derive(Clone, Copy, Debug)]
pub struct NewStairs {
    pub type_id: ElementId,
    pub base_level: ElementId,
    pub top_level: ElementId,
    /// UNKNOWN: the tread depth a new stairs holds before "the actual tread depth is raised to the type minimum when it is
    /// smaller". Metres.
    pub initial_tread_depth: f64,
}

/// CREATION, then the regeneration of the commit. With the type's maximum riser height above 1e-9: `n = (int)(ceil(stairs
/// height / max riser height) + 1e-9)`, `riser height = stairs height / n`; the riser height must be at least the vertex
/// tolerance or creation returns the invalid id; then the actual tread depth is raised to the type minimum when it is
/// smaller.
///
/// AT THE CALL: an id that is not a stairs type -> `STAIRS_TYPE_NOT_A_STAIRS_TYPE`; a riser height below the vertex
/// tolerance -> `STAIRS_CREATION_RISER_HEIGHT_TOO_SMALL`. The user flag is not raised by creation (not stated).
pub fn create_stairs(m: &mut BimModel, cx: &mut TxnContext<'_>, spec: NewStairs) -> Result<ElementId, Failure> {
    let ty = match m.stairs.types.get(&spec.type_id) {
        Some(t) => t.params,
        None => {
            return Err(Failure::error(
                codes::STAIRS_TYPE_NOT_A_STAIRS_TYPE,
                "The type id given to start a stairs is not a stairs type.",
            )
            .on(&[spec.type_id]))
        }
    };
    let bottom = level_elevation(m, spec.base_level)?;
    let top = level_elevation(m, spec.top_level)?;
    require_finite(&[spec.initial_tread_depth], "the initial tread depth")?;
    let height = ft(top - bottom);
    let n = creation_riser_count(height, ft(ty.max_riser_height)).ok_or_else(|| {
        undecided("creation with a maximum riser height not above 1e-9")
    })?;
    if n < 1 {
        return Err(undecided("creation with fewer than one riser (stairs height not above 0)"));
    }
    let riser = height / n as f64;
    if riser < vertex_eps() {
        return Err(Failure::error(
            codes::STAIRS_CREATION_RISER_HEIGHT_TOO_SMALL,
            "The creation riser height is below the vertex tolerance.",
        ));
    }
    let tread = spec.initial_tread_depth.max(ty.min_tread_depth);
    let id = cx.new_id();
    m.stairs.stairs.insert(
        id,
        Stairs {
            id,
            type_id: spec.type_id,
            base_level: spec.base_level,
            top_level: spec.top_level,
            actual_riser_height: metres(riser),
            actual_tread_depth: tread,
            user_changed_riser_height: false,
            riser_count_floor: None,
            multistory_member: false,
        },
    );
    regenerate_stairs(m, cx, id)?;
    Ok(id)
}

/// Move a stairs to other base / top levels (the elevation span is a parent of the riser step) and
/// regenerate it.
pub fn set_stairs_levels(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    stairs: ElementId,
    base_level: ElementId,
    top_level: ElementId,
) -> Result<(), Failure> {
    m.stairs.stairs(stairs)?;
    level_elevation(m, base_level)?;
    level_elevation(m, top_level)?;
    let s = m.stairs.stairs.get_mut(&stairs).expect("looked up above");
    s.base_level = base_level;
    s.top_level = top_level;
    regenerate_stairs(m, cx, stairs)
}

/// Behind the parameter validity.
///
/// AT PARAMETER SET: an integer that is not positive is rejected with `STAIRS_ILLEGAL_DESIRED_RISERS` before any change.
/// UNKNOWN: how the entry point reports it; the engine's rejection channel is `Err`.
///
/// A value equal to the current desired number is not accepted by the operation: nothing changes and the user flag is not
/// raised.
pub fn set_riser_count_target(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    stairs: ElementId,
    n: i32,
) -> Result<(), Failure> {
    let s = m.stairs.stairs(stairs)?.clone();
    if n <= 0 {
        return Err(Failure::error(
            codes::STAIRS_ILLEGAL_DESIRED_RISERS,
            "The desired number of risers must be positive.",
        )
        .on(&[stairs]));
    }
    let ty = m.stairs.stairs_type(s.type_id)?.params;
    let (bottom, top) = stairs_span(m, &s)?;
    let height = ft(top - bottom);
    let current = desired_number_of_risers(height, ft(s.actual_riser_height));
    if !desired_number_change_applies(n, current) {
        return Ok(());
    }
    let st = m.stairs.stairs.get_mut(&stairs).expect("looked up above");
    set_actual_riser_height_(st, &ty, height / n as f64);
    st.user_changed_riser_height = true;
    regenerate_stairs(m, cx, stairs)
}

/// Set the actual tread depth parameter (metres).
///
/// AT PARAMETER SET: "a double value that is not positive for the actual tread depth parameter ->
/// StairsActualTreadDepthLessThanMinimumFailure", rejected before any change.
pub fn set_actual_tread_depth(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    stairs: ElementId,
    value: f64,
) -> Result<(), Failure> {
    m.stairs.stairs(stairs)?;
    if !(ft(value) > 0.0) {
        return Err(Failure::error(
            codes::STAIRS_TREAD_DEPTH_LESS_THAN_MINIMUM,
            "The actual tread depth must be positive.",
        )
        .on(&[stairs]));
    }
    m.stairs.stairs.get_mut(&stairs).expect("looked up above").actual_tread_depth = value;
    regenerate_stairs(m, cx, stairs)
}

/// Mark a stairs as a member of a multistory group (the group is not modelled) and regenerate it.
pub fn set_multistory_member(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    stairs: ElementId,
    member: bool,
) -> Result<(), Failure> {
    m.stairs.stairs(stairs)?;
    m.stairs.stairs.get_mut(&stairs).expect("looked up above").multistory_member = member;
    regenerate_stairs(m, cx, stairs)
}

/// Input of [`add_straight_stair_run`].
#[derive(Clone, Copy, Debug)]
pub struct NewStraightRun {
    pub stairs: ElementId,
    pub start: Vec2,
    pub end: Vec2,
    /// The run's bottom elevation is the start point z. Metres.
    pub bottom_elevation: f64,
    pub width: f64,
    /// UNKNOWN: the defaults of a new run.
    pub begin_with_riser: bool,
    pub end_with_riser: bool,
    /// UNKNOWN: which run type code assigns.
    pub run_type_code: u8,
}

/// (component straight run).
///
/// AT THE CALL, each rolling back the sub-transaction:
/// * the stairs id is not a stairs -> `STAIRS_RUN_NEEDS_STAIRS`;
/// * the stairs' actual tread depth is not above 1e-9 -> `STAIRS_RUN_TREAD_DEPTH_NOT_POSITIVE`;
/// * the computed riser count is negative -> `STAIRS_RUN_NEGATIVE_RISER_COUNT`;
/// * the regeneration after creation posts an error -> `STAIRS_REGENERATION_FAILED`.
///
/// The location line is first rounded by [`round_run_length`]; the count is
/// [`count_risers_for_run`] of the rounded line; the top elevation is derived
/// ([`run_top_elevation`]) and never stored.
pub fn add_straight_stair_run(m: &mut BimModel, cx: &mut TxnContext<'_>, spec: NewStraightRun) -> Result<ElementId, Failure> {
    let s = match m.stairs.stairs.get(&spec.stairs) {
        Some(s) => s.clone(),
        None => {
            return Err(Failure::error(codes::STAIRS_RUN_NEEDS_STAIRS, "The id given is not a stairs.")
                .on(&[spec.stairs]))
        }
    };
    let tread = ft(s.actual_tread_depth);
    if !(tread > EPS) {
        return Err(Failure::error(
            codes::STAIRS_RUN_TREAD_DEPTH_NOT_POSITIVE,
            "The stairs' actual tread depth is not above 1e-9.",
        )
        .on(&[spec.stairs]));
    }
    require_finite(
        &[spec.start.x, spec.start.y, spec.end.x, spec.end.y, spec.bottom_elevation, spec.width],
        "a run",
    )?;
    let l0 = ft(spec.start.distance_to(spec.end));
    let dir = (spec.end.x - spec.start.x, spec.end.y - spec.start.y);
    if !(l0 > 0.0) {
        return Err(undecided("a run whose location line has no length has no direction to extend along"));
    }
    let length = round_run_length(l0, tread);
    let unit = (dir.0 / spec.start.distance_to(spec.end), dir.1 / spec.start.distance_to(spec.end));
    let rounded_m = metres(length);
    let end = Vec2 { x: spec.start.x + unit.0 * rounded_m, y: spec.start.y + unit.1 * rounded_m };

    let risers = count_risers_for_run(length, tread, spec.begin_with_riser, spec.end_with_riser);
    if risers < 0 {
        return Err(Failure::error(codes::STAIRS_RUN_NEGATIVE_RISER_COUNT, "The run's riser count is negative.")
            .on(&[spec.stairs]));
    }
    let id = cx.new_id();
    m.stairs.runs.insert(
        id,
        StairsRun {
            id,
            stairs: spec.stairs,
            start: spec.start,
            end,
            bottom_elevation: spec.bottom_elevation,
            width: spec.width,
            begin_with_riser: spec.begin_with_riser,
            end_with_riser: spec.end_with_riser,
            run_type_code: spec.run_type_code,
        },
    );
    let posted = regenerate_collect(m, spec.stairs)?;
    if let Some(bad) = posted.iter().find(|f| f.severity == Severity::Error) {
        return Err(Failure::error(
            codes::STAIRS_REGENERATION_FAILED,
            format!("The regeneration after the run was created failed: {}", bad.message),
        )
        .on(&[spec.stairs, id]));
    }
    for f in posted {
        cx.report(f);
    }
    Ok(id)
}

/// Committing the stairs edit scope with unresolved errors raises. The stairs is regenerated; an error among its posted
/// failures -> `STAIRS_SCOPE_COMMIT_UNRESOLVED_ERRORS`. UNKNOWN: which failures count as "unresolved"; error severity is
/// used.
pub fn commit_stairs_edit_scope(m: &mut BimModel, cx: &mut TxnContext<'_>, stairs: ElementId) -> Result<(), Failure> {
    let posted = regenerate_collect(m, stairs)?;
    if posted.iter().any(|f| f.severity == Severity::Error) {
        return Err(Failure::error(
            codes::STAIRS_SCOPE_COMMIT_UNRESOLVED_ERRORS,
            "Cannot commit the stairs edit scope due to unresolved errors.",
        )
        .on(&[stairs]));
    }
    for f in posted {
        cx.report(f);
    }
    Ok(())
}

/// Add a landing whose base elevation is `requested` (metres), set through the snapping rule of Spec S-01c.
///
/// UNKNOWN (Spec S-01c): how a landing element is created (sketched and automatic landing creation are not specified). The
/// parameter validity is the parameter-set path, not applied here; use [`set_landing_base_elevation`] for that.
pub fn add_landing(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    stairs: ElementId,
    requested: f64,
) -> Result<ElementId, Failure> {
    let s = m.stairs.stairs(stairs)?.clone();
    require_finite(&[requested], "a landing elevation")?;
    let (bottom, _) = stairs_span(m, &s)?;
    let (e, n) = snap_landing_elevation(ft(requested), ft(bottom), ft(s.actual_riser_height))
        .ok_or_else(|| undecided("a landing needs a positive actual riser height to snap to"))?;
    let id = cx.new_id();
    m.stairs.landings.insert(
        id,
        StairsLanding { id, stairs, base_elevation: metres(e), base_riser_index: Some(n) },
    );
    regenerate_stairs(m, cx, stairs)?;
    Ok(id)
}

/// Set a landing's base elevation parameter (metres).
///
/// AT PARAMETER SET (Spec S-01c): a base elevation whose rounded riser count relative to the stairs is not above 0 is
/// rejected with `STAIRS_LANDING_HEIGHT_UNDER_STAIRS_BOTTOM` before any change; the instance check also requires the
/// elevation to be at least the tread thickness (when the type states one). Then the value is snapped
/// ([`snap_landing_elevation`]) and re-stored "only when it changes by more than 1e-9 relative or the index is unset".
pub fn set_landing_base_elevation(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    landing: ElementId,
    requested: f64,
) -> Result<(), Failure> {
    let l = m.stairs.landings.get(&landing).ok_or_else(|| not_found(landing))?.clone();
    let s = m.stairs.stairs(l.stairs)?.clone();
    let ty = m.stairs.stairs_type(s.type_id)?.params;
    require_finite(&[requested], "a landing elevation")?;
    let (bottom, _) = stairs_span(m, &s)?;
    let relative = ft(requested) - ft(bottom);
    let riser = ft(s.actual_riser_height);
    if !landing_riser_count_is_valid(relative, riser)
        || !landing_elevation_meets_tread_thickness(relative, ty.tread_thickness.map(ft))
    {
        return Err(Failure::error(
            codes::STAIRS_LANDING_HEIGHT_UNDER_STAIRS_BOTTOM,
            "The landing base elevation rounds to no riser above the stairs bottom.",
        )
        .on(&[landing]));
    }
    let (e, n) = snap_landing_elevation(ft(requested), ft(bottom), riser)
        .ok_or_else(|| undecided("a landing needs a positive actual riser height to snap to"))?;
    let slot = m.stairs.landings.get_mut(&landing).expect("looked up above");
    if slot.base_riser_index.is_none() || differs_by_more_than_relative_eps(e, ft(slot.base_elevation)) {
        slot.base_elevation = metres(e);
        slot.base_riser_index = Some(n);
    }
    regenerate_stairs(m, cx, l.stairs)
}

/// "one per railing path of the stairs".
///
/// The provider's answer is an input ([`RailingPathSet`]): the boundary curve loops come from run
/// and landing geometry that this engine does not model. Rules: the reported count must agree with
/// the number of boundaries returned, otherwise the call fails with error 1; for each path index
/// not in `excluded` a railing is created, the path's sub-element keys are stored on it and its id
/// is appended to the output; the call fails (error 1) when no railing results.
///
/// A multistory-group stairs first looks for an existing railing at that location and reuses it: refused as undecided.
pub fn generate_stair_railings(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    stairs: ElementId,
    railing_type: ElementId,
    paths: &RailingPathSet,
    excluded: &[usize],
) -> Result<Vec<ElementId>, Failure> {
    let s = m.stairs.stairs(stairs)?;
    if s.multistory_member {
        return Err(undecided(
            "a multistory-group stairs reuses 'the best' existing railing at its location (an engine choice)",
        ));
    }
    if paths.reported_count != paths.paths.len() {
        return Err(Failure::error(
            codes::STAIRS_RAILING_CREATION_FAILED,
            "The railing paths' reported count disagrees with the boundaries returned (error 1).",
        )
        .on(&[stairs]));
    }
    let mut out = Vec::new();
    for (i, path) in paths.paths.iter().enumerate() {
        if excluded.contains(&i) {
            continue;
        }
        let id = cx.new_id();
        m.stairs.railings.insert(
            id,
            StairsRailing { id, stairs, railing_type, path_index: i, subelement_keys: path.keys.clone() },
        );
        out.push(id);
    }
    if out.is_empty() {
        return Err(Failure::error(codes::STAIRS_RAILING_CREATION_FAILED, "No railing resulted (error 1).")
            .on(&[stairs]));
    }
    Ok(out)
}

/// Remove a stairs element together with the components that name it (its runs, landings and
/// railings) and return every removed id, the stairs first.
///
/// ASSUMED (structural, NOT a read rule): a run, landing or railing here holds the id of its stairs and has no meaning
/// without it (`add_straight_stair_run` refuses a missing stairs with `STAIRS_RUN_NEEDS_STAIRS`), so they leave with it rather
/// than dangle. UNKNOWN (Spec L-01): which dependents the reference product's deletion algorithm removes for a stairs and
/// which failures it posts.
pub fn delete_stairs(m: &mut BimModel, id: ElementId) -> Result<Vec<ElementId>, Failure> {
    m.stairs.stairs(id)?;
    let mut gone = vec![id];
    let runs: Vec<ElementId> = m.stairs.runs.values().filter(|r| r.stairs == id).map(|r| r.id).collect();
    let landings: Vec<ElementId> = m.stairs.landings.values().filter(|l| l.stairs == id).map(|l| l.id).collect();
    let railings: Vec<ElementId> = m.stairs.railings.values().filter(|r| r.stairs == id).map(|r| r.id).collect();
    for r in &runs {
        m.stairs.runs.remove(r);
    }
    for l in &landings {
        m.stairs.landings.remove(l);
    }
    for r in &railings {
        m.stairs.railings.remove(r);
    }
    m.stairs.stairs.remove(&id);
    gone.extend(runs);
    gone.extend(landings);
    gone.extend(railings);
    Ok(gone)
}

// ----------------------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::create_level;
    use crate::{new_document, BimDocument};
    use asterion_core::TransactionStatus;

    // ---- helpers -------------------------------------------------------------------------

    const FT: f64 = 0.3048;

    /// Feet to metres for building model inputs.
    fn m_(feet: f64) -> f64 {
        feet * FT
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1.0)
    }

    fn codes_of(out: &asterion_core::txn::Outcome<impl std::fmt::Debug>) -> Vec<String> {
        out.failures.iter().map(|f| f.code.clone()).collect()
    }

    /// A type with a maximum riser of 0.5 ft, a minimum tread of 1.0 ft and a minimum run width of
    /// 3 ft; no calculator.
    fn plain_type() -> StairsTypeParams {
        StairsTypeParams {
            max_riser_height: m_(0.5),
            min_tread_depth: m_(1.0),
            min_run_width: m_(3.0),
            use_calculator: false,
            calculator_target: 0.0,
            calculator_riser_multiplier: 0.0,
            calculator_tread_multiplier: 0.0,
            min_run_height: None,
            tread_thickness: None,
        }
    }

    struct Fx {
        doc: BimDocument,
        ty: ElementId,
        base: ElementId,
        top: ElementId,
    }

    /// Levels at 0 ft and 10 ft (stairs height 10 ft), the plain type above.
    fn fx() -> Fx {
        let mut doc = new_document();
        let (base, top, ty) = {
            let o = doc.transact("setup", false, |m, cx| {
                let b = create_level(m, cx, "L0", 0.0)?;
                let t = create_level(m, cx, "L1", m_(10.0))?;
                let ty = create_stairs_type(m, cx, "T", plain_type())?;
                Ok((b, t, ty))
            });
            assert!(o.committed(), "{:?}", o.failures);
            o.value.unwrap()
        };
        Fx { doc, ty, base, top }
    }

    impl Fx {
        fn make(&mut self, tread_ft: f64) -> ElementId {
            let (ty, base, top) = (self.ty, self.base, self.top);
            let o = self.doc.transact("stairs", false, |m, cx| {
                create_stairs(m, cx, NewStairs { type_id: ty, base_level: base, top_level: top, initial_tread_depth: m_(tread_ft) })
            });
            assert!(o.committed(), "{:?}", o.failures);
            o.value.unwrap()
        }

        fn stairs(&self, id: ElementId) -> &Stairs {
            self.doc.model().stairs.stairs.get(&id).unwrap()
        }

        fn riser_ft(&self, id: ElementId) -> f64 {
            self.stairs(id).actual_riser_height / FT
        }
    }


    /// "desired number of risers = (int)(stairs height / actual riser height + 0.5)".
    #[test]
    fn s01a_desired_number_rounds_height_over_riser_half_up() {
        assert_eq!(desired_number_of_risers(10.0, 0.5), 20);
        // 9.75 / 0.5 = 19.5 -> (int)(19.5 + 0.5) = 20: the half rounds up
        assert_eq!(desired_number_of_risers(9.75, 0.5), 20);
        // 9.7 / 0.5 = 19.4 -> (int)19.9 = 19
        assert_eq!(desired_number_of_risers(9.7, 0.5), 19);
        // just under the half
        assert_eq!(desired_number_of_risers(9.749, 0.5), 19);
    }

    /// "(0 when the riser height is not above 1e-9, 1 when the stairs height is not above 1e-9)" - boundaries exactly at
    /// 1e-9, "not above" is inclusive.
    #[test]
    fn s01a_desired_number_special_cases_at_1e_minus_9() {
        assert_eq!(desired_number_of_risers(10.0, 1e-9), 0, "riser height exactly 1e-9 is not above");
        assert_eq!(desired_number_of_risers(10.0, 0.0), 0);
        // a hair above 1e-9 the formula applies: 1e-8 / 1.000001e-9 = 9.99999 -> (int)10.49999 = 10
        assert_eq!(desired_number_of_risers(1e-8, 1.000001e-9), 10);
        assert_eq!(desired_number_of_risers(1e-9, 0.5), 1, "stairs height exactly 1e-9 is not above");
        assert_eq!(desired_number_of_risers(0.0, 0.5), 1);
        // a hair above 1e-9 the formula applies: 2e-9 / 1.0000001e-9 = 1.9999998 -> (int)2.4999998 = 2
        assert_eq!(desired_number_of_risers(2e-9, 1.0000001e-9), 2);
        // both hold: the row lists the riser case first (precedence UNKNOWN, see module docs)
        assert_eq!(desired_number_of_risers(1e-9, 1e-9), 0);
    }

    /// "Setting the desired number n (accepted only when n > 0 and differs from the current value) stores actual riser
    /// height = stairs height / n ... and raises the user-changed-riser-height flag".
    #[test]
    fn s01a_setting_the_desired_number_stores_height_over_n_and_raises_the_flag() {
        let mut f = fx();
        let id = f.make(1.0);
        // creation: n = ceil(10 / 0.5) = 20 -> riser 0.5 ft
        assert!(near(f.riser_ft(id), 0.5));
        assert!(!f.stairs(id).user_changed_riser_height);

        let out = f.doc.transact("n", false, |m, cx| set_riser_count_target(m, cx, id, 25));
        assert!(out.committed(), "{:?}", out.failures);
        // 10 / 25 = 0.4 ft; the regeneration cleared the flag again and kept the user's value
        assert!(near(f.riser_ft(id), 0.4), "{}", f.riser_ft(id));
        assert!(!f.stairs(id).user_changed_riser_height, "the regeneration step clears the flag");
        let st = stairs_state(f.doc.model(), id).unwrap();
        assert_eq!(st.desired_number_of_risers, 25);
    }

    /// "accepted only when n > 0 and differs from the current value" - equal is not accepted: nothing changes and the flag
    /// is not raised.
    #[test]
    fn s01a_a_desired_number_equal_to_the_current_is_not_accepted() {
        assert!(!desired_number_change_applies(20, 20));
        assert!(!desired_number_change_applies(0, 20));
        assert!(!desired_number_change_applies(-1, 20));
        assert!(desired_number_change_applies(21, 20));
        let mut f = fx();
        let id = f.make(1.0);
        let before = f.doc.model().clone();
        let out = f.doc.transact("same", false, |m, cx| set_riser_count_target(m, cx, id, 20));
        assert!(out.committed());
        assert_eq!(f.doc.model(), &before);
    }

    #[test]
    fn s01a_the_riser_height_is_stored_only_beyond_1e_minus_9_relative() {
        assert!(!differs_by_more_than_relative_eps(0.5 * (1.0 + 5e-10), 0.5));
        assert!(differs_by_more_than_relative_eps(0.5 * (1.0 + 2e-9), 0.5));
        assert!(!differs_by_more_than_relative_eps(0.5, 0.5));
        let ty = plain_type();
        let mut s = Stairs {
            id: ElementId(1),
            type_id: ElementId(2),
            base_level: ElementId(3),
            top_level: ElementId(4),
            actual_riser_height: m_(0.5),
            actual_tread_depth: m_(1.0),
            user_changed_riser_height: false,
            riser_count_floor: None,
            multistory_member: false,
        };
        let stored = s.actual_riser_height;
        set_actual_riser_height_(&mut s, &ty, ft(stored) * (1.0 + 5e-10));
        assert_eq!(s.actual_riser_height, stored, "within 1e-9 relative: not stored");
        set_actual_riser_height_(&mut s, &ty, ft(stored) * (1.0 + 2e-9));
        assert_ne!(s.actual_riser_height, stored, "beyond 1e-9 relative: stored");
    }

    /// "when the type uses the calculator and its tread multiplier is non-zero, sets the actual tread depth to (target -
    /// riser multiplier x riser height) / tread multiplier, never below the type's minimum tread depth".
    #[test]
    fn s01a_the_tread_calculator_follows_the_riser_height() {
        let mut ty = plain_type();
        ty.use_calculator = true;
        ty.calculator_target = m_(2.0);
        ty.calculator_riser_multiplier = 2.0;
        ty.calculator_tread_multiplier = 1.0;
        ty.min_tread_depth = m_(0.8);
        let base = Stairs {
            id: ElementId(1),
            type_id: ElementId(2),
            base_level: ElementId(3),
            top_level: ElementId(4),
            actual_riser_height: m_(0.4),
            actual_tread_depth: m_(5.0),
            user_changed_riser_height: false,
            riser_count_floor: None,
            multistory_member: false,
        };
        // riser 0.5 ft: (2.0 - 2 x 0.5) / 1 = 1.0 ft
        let mut s = base.clone();
        set_actual_riser_height_(&mut s, &ty, 0.5);
        assert!(near(ft(s.actual_tread_depth), 1.0), "{}", ft(s.actual_tread_depth));
        // riser 0.6 ft: (2.0 - 1.2) = 0.8 ft, exactly the minimum
        let mut s = base.clone();
        set_actual_riser_height_(&mut s, &ty, 0.6);
        assert!(near(ft(s.actual_tread_depth), 0.8));
        // riser 0.7 ft: 0.6 ft < 0.8 ft minimum -> never below the minimum
        let mut s = base.clone();
        set_actual_riser_height_(&mut s, &ty, 0.7);
        assert!(near(ft(s.actual_tread_depth), 0.8));
        // tread multiplier zero: the tread depth is untouched
        let mut z = ty;
        z.calculator_tread_multiplier = 0.0;
        let mut s = base.clone();
        set_actual_riser_height_(&mut s, &z, 0.5);
        assert_eq!(s.actual_tread_depth, base.actual_tread_depth);
        // calculator not used: untouched
        let mut u = ty;
        u.use_calculator = false;
        let mut s = base.clone();
        set_actual_riser_height_(&mut s, &u, 0.5);
        assert_eq!(s.actual_tread_depth, base.actual_tread_depth);
    }

    /// CREATION: "n = (int)(ceil(stairs height / max riser height) + 1e-9), riser height = stairs height / n".
    #[test]
    fn s01a_creation_riser_count_is_the_ceiling_of_height_over_maximum() {
        assert_eq!(creation_riser_count(10.0, 0.5), Some(20));
        assert_eq!(creation_riser_count(10.1, 0.5), Some(21));
        // the tolerance is ADDED after the ceil at creation: 20.0000000002 -> ceil 21
        assert_eq!(creation_riser_count(10.0 + 1e-10, 0.5), Some(21));
        // The maximum riser height must be above 1e-9.
        assert_eq!(creation_riser_count(10.0, 1e-9), None);
        assert_eq!(creation_riser_count(10.0, 1.0000001e-9).is_some(), true);
    }

    /// CREATION: "then the actual tread depth is raised to the type minimum when it is smaller", and the element keeps a
    /// larger one.
    #[test]
    fn s01a_creation_raises_the_tread_depth_to_the_type_minimum() {
        let mut f = fx();
        let small = f.make(0.4); // minimum is 1.0 ft
        assert!(near(f.stairs(small).actual_tread_depth, m_(1.0)));
        let large = f.make(1.5);
        assert!(near(f.stairs(large).actual_tread_depth, m_(1.5)));
    }

    /// CREATION: "the riser height must be at least the vertex tolerance ... or creation returns the invalid id" - at the
    /// call: nothing is created.
    #[test]
    fn s01a_creation_refuses_a_riser_below_the_vertex_tolerance() {
        let mut f = fx();
        // a height of 0.0005 ft over the creation count 1 gives a riser of 0.0005 < 0.00052338
        let tiny = f.doc.transact("l", false, |m, cx| create_level(m, cx, "tiny", m_(0.0005))).value.unwrap();
        let (ty, base) = (f.ty, f.base);
        let out = f.doc.transact("c", false, |m, cx| {
            create_stairs(m, cx, NewStairs { type_id: ty, base_level: base, top_level: tiny, initial_tread_depth: m_(1.0) })
        });
        assert_eq!(out.status, TransactionStatus::RolledBack);
        assert_eq!(codes_of(&out), vec![codes::STAIRS_CREATION_RISER_HEIGHT_TOO_SMALL]);
        assert!(f.doc.model().stairs.stairs.is_empty());
        // a height just above the tolerance (0.00053 ft) is created
        let ok = f.doc.transact("l", false, |m, cx| create_level(m, cx, "ok", m_(0.00053))).value.unwrap();
        let out = f.doc.transact("c", false, |m, cx| {
            create_stairs(m, cx, NewStairs { type_id: ty, base_level: base, top_level: ok, initial_tread_depth: m_(1.0) })
        });
        assert!(out.committed(), "{:?}", out.failures);
    }

    fn step(height: f64, max: f64, riser: f64, flag: bool, member: bool, floor: Option<i32>) -> RiserStep {
        riser_regeneration_step(&RiserStepInput {
            stairs_height: height,
            max_riser_height: max,
            actual_riser_height: riser,
            user_changed_riser_height: flag,
            multistory_member: member,
            riser_count_floor: floor,
            vertex_eps: DEFAULT_VERTEX_EPS_FT,
        })
    }

    /// REGENERATION: "when the max riser height is not above 1e-9 or the stairs height is below 1e-9 nothing is done" - the
    /// flag is not cleared either.
    #[test]
    fn s01a_regeneration_does_nothing_at_its_guards() {
        let s = step(10.0, 1e-9, 0.4, true, false, None);
        assert_eq!(s, RiserStep { clear_user_flag: false, outcome: RiserStepOutcome::NothingToDo });
        let s = step(1e-9 - 1e-12, 0.5, 0.4, true, false, None);
        assert_eq!(s.outcome, RiserStepOutcome::NothingToDo, "height below 1e-9");
        // height exactly 1e-9 is NOT below: the step runs (n = ceil(2e-9 - 1e-9) = 1, riser 1e-9)
        let s = step(1e-9, 0.5, 0.4, false, false, None);
        assert!(matches!(s.outcome, RiserStepOutcome::RiserHeightTooSmall { risers: 1, .. }), "{s:?}");
    }

    /// REGENERATION: "when the user-changed flag is set it is cleared and, unless the stairs is a multistory member, the
    /// step ends keeping the user's riser height".
    #[test]
    fn s01a_regeneration_keeps_the_user_riser_height_and_clears_the_flag() {
        let s = step(10.0, 0.5, 0.4, true, false, None);
        assert_eq!(s, RiserStep { clear_user_flag: true, outcome: RiserStepOutcome::KeptUserRiserHeight });
    }

    /// REGENERATION: "otherwise n = ceil(height / max riser height - 1e-9) (the tolerance is subtracted before the ceil)" -
    /// the boundary at 1e-9 is the reverse of creation's.
    #[test]
    fn s01a_regeneration_subtracts_the_tolerance_before_the_ceil() {
        // 10 / 0.5 = 20 exactly -> 20 risers of 0.5
        let s = step(10.0, 0.5, 0.4, false, false, None);
        assert_eq!(s.outcome, RiserStepOutcome::Stored { risers: 20, riser_height: 0.5 });
        // 20.0000000002 - 1e-9 = 19.9999999992 -> ceil 20 (creation gave 21 for this height)
        let s = step(10.0 + 1e-10, 0.5, 0.4, false, false, None);
        assert!(matches!(s.outcome, RiserStepOutcome::Stored { risers: 20, .. }), "{s:?}");
        // 20.000000004 - 1e-9 = 20.000000003 -> ceil 21
        let s = step(10.0 + 2e-9, 0.5, 0.4, false, false, None);
        assert!(matches!(s.outcome, RiserStepOutcome::Stored { risers: 21, .. }), "{s:?}");
    }

    /// REGENERATION: "for a single stairs n is the larger of that value and the integer stored ".
    #[test]
    fn s01a_a_single_stairs_takes_the_larger_of_the_count_and_the_stored_integer() {
        let s = step(10.0, 0.5, 0.4, false, false, Some(25));
        assert_eq!(s.outcome, RiserStepOutcome::Stored { risers: 25, riser_height: 0.4 });
        let s = step(10.0, 0.5, 0.4, false, false, Some(12));
        assert_eq!(s.outcome, RiserStepOutcome::Stored { risers: 20, riser_height: 0.5 });
    }

    /// REGENERATION: "for a multistory member n = ceil(height / current actual riser height - 1e-9)"; a set flag is cleared
    /// and the step continues for a member.
    #[test]
    fn s01a_a_multistory_member_derives_the_count_from_its_own_riser_height() {
        // 10 / 0.4 = 25 exactly -> 25 risers of 0.4, not the 20 the maximum would give
        let s = step(10.0, 0.5, 0.4, false, true, Some(40));
        assert_eq!(s.outcome, RiserStepOutcome::Stored { risers: 25, riser_height: 0.4 });
        // flag set + member: cleared, and the step goes on
        let s = step(10.0, 0.5, 0.4, true, true, None);
        assert!(s.clear_user_flag);
        assert_eq!(s.outcome, RiserStepOutcome::Stored { risers: 25, riser_height: 0.4 });
        // 10 / 0.3999 = 25.006 -> 26
        let s = step(10.0, 0.5, 0.3999, false, true, None);
        assert!(matches!(s.outcome, RiserStepOutcome::Stored { risers: 26, .. }), "{s:?}");
    }

    /// REGENERATION: "if it is below the vertex tolerance the failure RiserHeightIsTooSmall is posted ..., else it is
    /// stored" - the boundary is the vertex tolerance, strict.
    #[test]
    fn s01a_a_riser_below_the_vertex_tolerance_is_posted_not_stored() {
        // height 0.001, max 0.5 -> ceil(0.002 - 1e-9) = 1 ; floor 4 -> n = 4 -> 0.00025 < 0.00052338
        let s = step(0.001, 0.5, 0.4, false, false, Some(4));
        assert!(matches!(s.outcome, RiserStepOutcome::RiserHeightTooSmall { risers: 4, .. }), "{s:?}");
        // height 0.002094 / 4 = 0.0005235 >= 0.0005233832795 -> stored
        let s = step(0.002094, 0.5, 0.4, false, false, Some(4));
        assert!(matches!(s.outcome, RiserStepOutcome::Stored { risers: 4, .. }), "{s:?}");
        // exactly the tolerance: height = 4 x eps -> NOT below -> stored
        let s = step(4.0 * DEFAULT_VERTEX_EPS_FT, 0.5, 0.4, false, false, Some(4));
        assert!(matches!(s.outcome, RiserStepOutcome::Stored { .. }), "{s:?}");
    }

    /// REGENERATION at commit: the failure is POSTED at commit, not raised at the call; the engine's severity for code 2 is
    /// the single switch.
    #[test]
    fn s01a_s01e_riser_too_small_is_posted_at_commit_and_rolls_back() {
        let mut f = fx();
        let id = f.make(1.0);
        let out = f.doc.transact("floor", false, |m, cx| {
            m.stairs.stairs.get_mut(&id).unwrap().riser_count_floor = Some(100_000);
            regenerate_stairs(m, cx, id)
        });
        assert_eq!(out.status, TransactionStatus::RolledBack);
        assert_eq!(codes_of(&out), vec![codes::STAIRS_RISER_HEIGHT_TOO_SMALL]);
        assert_eq!(out.failures[0].severity, RISER_HEIGHT_TOO_SMALL_SEVERITY);
        assert_eq!(f.stairs(id).riser_count_floor, None, "rolled back");
    }

    #[test]
    fn s01a_the_riser_height_warning_boundary_is_max_plus_1e_minus_9() {
        assert!(!riser_height_above_maximum(0.5, 0.5));
        assert!(!riser_height_above_maximum(0.5 + 1e-9, 0.5), "exactly max + 1e-9 is not greater");
        assert!(riser_height_above_maximum(0.5 + 2e-9, 0.5));
    }

    /// WARNINGS: "for stairs with a component run, type minimum tread depth greater than the actual tread depth raises
    /// StairsActualTreadDepthLessThanMinimumFailure".
    #[test]
    fn s01a_the_tread_warning_needs_a_component_run_and_is_strict() {
        assert!(tread_depth_below_type_minimum(true, 1.0, 0.99));
        assert!(!tread_depth_below_type_minimum(true, 1.0, 1.0));
        assert!(!tread_depth_below_type_minimum(false, 1.0, 0.5), "no component run: no warning");
    }

    /// WARNINGS: "a run (not run type 2) whose actual width is below the type's minimum run width raises
    /// StairsActualRunWidthLessThanMinimumFailure".
    #[test]
    fn s01a_the_run_width_warning_excludes_run_type_2() {
        assert!(run_width_below_type_minimum(1, 2.9, 3.0));
        assert!(!run_width_below_type_minimum(1, 3.0, 3.0), "equal is not below");
        assert!(!run_width_below_type_minimum(2, 1.0, 3.0), "run type 2 is excluded");
        assert!(run_width_below_type_minimum(0, 1.0, 3.0));
    }

    /// "Created risers = floor((top of the top component - stairs bottom) / riser height
    /// + 1e-9); remaining risers = round(height / riser height) - created."
    #[test]
    fn s01a_created_and_remaining_risers() {
        assert_eq!(created_risers(5.0, 0.0, 0.5), 10);
        // 9.9999999999 / 0.5 = 19.9999999998 + 1e-9 = 20.0000000008 -> floor 20
        assert_eq!(created_risers(10.0 - 1e-10, 0.0, 0.5), 20);
        assert_eq!(created_risers(10.0 - 1e-8, 0.0, 0.5), 19, "20 - 2e-8 + 1e-9 < 20");
        assert_eq!(remaining_risers(10.0, 0.5, 10), 10);
        assert_eq!(remaining_risers(9.75, 0.5, 4), 16, "round(19.5) = 20");
    }

    /// The regeneration warnings are posted through the transaction and COMMIT.
    #[test]
    fn s01a_s01e_the_limit_warnings_are_posted_at_commit_and_keep_the_commit() {
        let mut f = fx();
        let id = f.make(1.0);
        // a tread below the type minimum (1.0 ft) with a run on the stairs -> tread warning
        let out = f.doc.transact("run", false, |m, cx| {
            add_straight_stair_run(
                m,
                cx,
                NewStraightRun {
                    stairs: id,
                    start: Vec2 { x: 0.0, y: 0.0 },
                    end: Vec2 { x: m_(5.0), y: 0.0 },
                    bottom_elevation: 0.0,
                    width: m_(2.0), // below the 3 ft minimum width
                    begin_with_riser: true,
                    end_with_riser: true,
                    run_type_code: 1,
                },
            )
        });
        assert!(out.committed(), "{:?}", out.failures);
        assert_eq!(codes_of(&out), vec![codes::STAIRS_RUN_WIDTH_LESS_THAN_MINIMUM]);
        assert_eq!(out.failures[0].severity, Severity::Warning);
        let out = f.doc.transact("tread", false, |m, cx| set_actual_tread_depth(m, cx, id, m_(0.9)));
        assert!(out.committed(), "{:?}", out.failures);
        let c = codes_of(&out);
        assert!(c.contains(&codes::STAIRS_TREAD_DEPTH_LESS_THAN_MINIMUM.to_owned()), "{c:?}");
        // a type edit regenerates the stairs of that type (the type's atom is a parent of the
        // step): the maximum is lowered to 0.45 ft and the step recomputes n = ceil(10 / 0.45 - 1e-9)
        // = 23, riser 0.4348 ft <= 0.45 ft, so no riser warning is posted
        let (ty, mut p) = (f.ty, plain_type());
        p.max_riser_height = m_(0.45);
        let out = f.doc.transact("type", false, |m, cx| set_stairs_type_params(m, cx, ty, p));
        assert!(out.committed(), "{:?}", out.failures);
        assert!(!codes_of(&out).contains(&codes::STAIRS_RISER_HEIGHT_ABOVE_MAXIMUM.to_owned()));
        assert!(near(f.riser_ft(id), 10.0 / 23.0), "{}", f.riser_ft(id));
    }

    /// The user's riser height survives regeneration even above the maximum - that is exactly when "actual riser height
    /// greater than max riser height + 1e-9" is raised.
    #[test]
    fn s01a_a_user_riser_height_above_the_maximum_raises_the_warning() {
        let mut f = fx();
        let id = f.make(1.0);
        // 10 ft / 15 risers = 0.6667 ft > 0.5 ft maximum
        let out = f.doc.transact("n", false, |m, cx| set_riser_count_target(m, cx, id, 15));
        assert!(out.committed(), "{:?}", out.failures);
        assert_eq!(codes_of(&out), vec![codes::STAIRS_RISER_HEIGHT_ABOVE_MAXIMUM]);
        assert!(near(f.riser_ft(id), 10.0 / 15.0));
        assert_eq!(f.doc.model().stairs.stairs[&id].user_changed_riser_height, false);
        // the next regeneration without the flag recomputes from the maximum: back to 20 risers
        let out = f.doc.transact("regen", false, |m, cx| regenerate_stairs(m, cx, id));
        assert!(out.committed());
        assert!(near(f.riser_ft(id), 0.5), "{}", f.riser_ft(id));
        assert!(out.failures.is_empty());
    }


    /// "when T <= L the end is moved to floor(L / T + 0.5) x T from the start, otherwise to exactly T".
    #[test]
    fn s01b_the_location_line_is_rounded_to_whole_treads() {
        assert!(near(round_run_length(5.4, 1.0), 5.0));
        assert!(near(round_run_length(5.5, 1.0), 6.0), "the half goes up: floor(6.0)");
        assert!(near(round_run_length(5.4999, 1.0), 5.0));
        assert!(near(round_run_length(1.0, 1.0), 1.0), "T <= L includes equality");
        assert!(near(round_run_length(0.99, 1.0), 1.0), "L < T: exactly T");
        assert!(near(round_run_length(0.1, 1.0), 1.0));
        // 10 / 0.9 = 11.11 -> floor(11.61) = 11 treads of 0.9
        assert!(near(round_run_length(10.0, 0.9), 9.9));
    }

    /// - (1 when the run does not begin with a riser) - (1 when it does not end with a riser), so a
    /// run that begins and ends with a riser has round(L / T) + 1 risers".
    #[test]
    fn s01b_riser_number_formula() {
        assert_eq!(count_risers_for_run(5.0, 1.0, true, true), 6, "round(5) + 1");
        assert_eq!(count_risers_for_run(5.0, 1.0, false, true), 5);
        assert_eq!(count_risers_for_run(5.0, 1.0, true, false), 5);
        assert_eq!(count_risers_for_run(5.0, 1.0, false, false), 4);
        // floor(L/T + 1.5) at the half: L/T = 4.5 -> floor(6.0) = 6 ; 4.4999 -> floor(5.9999) = 5
        assert_eq!(count_risers_for_run(4.5, 1.0, true, true), 6);
        assert_eq!(count_risers_for_run(4.4999, 1.0, true, true), 5);
        // a run of exactly one tread, with neither riser: floor(2.5) - 2 = 0
        assert_eq!(count_risers_for_run(1.0, 1.0, false, false), 0);
        // negative on an UNROUNDED short line: floor(0.2 + 1.5) - 2 = -1
        assert_eq!(count_risers_for_run(0.2, 1.0, false, false), -1);
    }

    /// "The run's bottom elevation is the start point z, its top elevation = bottom + risers x actual riser height".
    #[test]
    fn s01b_top_elevation_is_bottom_plus_risers_times_riser_height() {
        assert!(near(run_top_elevation(2.0, 6, 0.5), 5.0));
        assert!(near(run_top_elevation(2.0, 0, 0.5), 2.0));
    }

    /// The line is rounded first, then the count is taken over the rounded line; the top elevation is derived from it; the
    /// regeneration then runs.
    #[test]
    fn s01b_create_straight_run_rounds_the_line_and_counts_risers() {
        let mut f = fx();
        let id = f.make(1.0);
        // L = 5.4 ft, T = 1.0 ft -> end moved to 5.0 ft; begin and end with riser -> 6 risers
        let out = f.doc.transact("run", false, |m, cx| {
            add_straight_stair_run(
                m,
                cx,
                NewStraightRun {
                    stairs: id,
                    start: Vec2 { x: 0.0, y: 0.0 },
                    end: Vec2 { x: m_(5.4), y: 0.0 },
                    bottom_elevation: m_(1.0),
                    width: m_(4.0),
                    begin_with_riser: true,
                    end_with_riser: true,
                    run_type_code: 1,
                },
            )
        });
        assert!(out.committed(), "{:?}", out.failures);
        let run = out.value.unwrap();
        let r = &f.doc.model().stairs.runs[&run];
        assert!(near(r.end.x, m_(5.0)), "{}", r.end.x / FT);
        let st = stairs_state(f.doc.model(), id).unwrap();
        assert_eq!(st.runs.len(), 1);
        assert_eq!(st.runs[0].risers, 6);
        // top = 1.0 + 6 x 0.5 = 4.0 ft
        assert!(near(st.runs[0].top_elevation, m_(4.0)), "{}", st.runs[0].top_elevation / FT);
        // created risers = floor((4.0 - 0) / 0.5 + 1e-9) = 8 ; remaining = round(10 / 0.5) - 8 = 12
        assert_eq!(st.created_risers, Some(8));
        assert_eq!(st.remaining_risers, Some(12));
        // a line shorter than one tread is moved to exactly one tread
        let out = f.doc.transact("short", false, |m, cx| {
            add_straight_stair_run(
                m,
                cx,
                NewStraightRun {
                    stairs: id,
                    start: Vec2 { x: 0.0, y: 1.0 },
                    end: Vec2 { x: 0.0, y: 1.0 + m_(0.3) },
                    bottom_elevation: 0.0,
                    width: m_(4.0),
                    begin_with_riser: true,
                    end_with_riser: false,
                    run_type_code: 1,
                },
            )
        });
        assert!(out.committed(), "{:?}", out.failures);
        let r = &f.doc.model().stairs.runs[&out.value.unwrap()];
        assert!(near(r.end.y - r.start.y, m_(1.0)));
    }

    /// "InternalException when the stairs id is not a stairs or its actual tread depth is not above 1e-9" - at the call,
    /// nothing is created.
    #[test]
    fn s01b_s01e_create_straight_run_internal_failures_at_the_call() {
        let mut f = fx();
        let id = f.make(1.0);
        let new_run = |stairs: ElementId| NewStraightRun {
            stairs,
            start: Vec2 { x: 0.0, y: 0.0 },
            end: Vec2 { x: 1.0, y: 0.0 },
            bottom_elevation: 0.0,
            width: 1.0,
            begin_with_riser: true,
            end_with_riser: true,
            run_type_code: 1,
        };
        let level = f.base;
        let out = f.doc.transact("not-stairs", false, |m, cx| add_straight_stair_run(m, cx, new_run(level)));
        assert_eq!(out.status, TransactionStatus::RolledBack);
        assert_eq!(codes_of(&out), vec![codes::STAIRS_RUN_NEEDS_STAIRS]);
        // tread depth exactly 1e-9 ft is not above: set it directly (the parameter path would
        // accept it, it is positive), then the call refuses
        let out = f.doc.transact("t", false, |m, cx| {
            m.stairs.stairs.get_mut(&id).unwrap().actual_tread_depth = 1e-9 * FT;
            add_straight_stair_run(m, cx, new_run(id))
        });
        assert_eq!(codes_of(&out), vec![codes::STAIRS_RUN_TREAD_DEPTH_NOT_POSITIVE]);
        assert!(f.doc.model().stairs.runs.is_empty());
    }

    /// "a negative result is rejected with an ArgumentException" - the engine checks the count after rounding, which can
    /// never be below zero for a line (L >= T), so the refusal is reachable only through [`count_risers_for_run`] on an
    /// unrounded line.
    #[test]
    fn s01b_a_rounded_line_never_gives_a_negative_count() {
        for l0 in [0.001, 0.2, 0.99, 1.0, 1.4, 7.7] {
            let l = round_run_length(l0, 1.0);
            assert!(count_risers_for_run(l, 1.0, false, false) >= 0, "L0 = {l0}");
        }
    }

    /// "run length = T x (desired risers of the run + 1 per missing begin riser + 1 per missing end riser - 1) for a line,
    /// with desired risers of the run = floor((stairs top elevation - run base elevation) / actual riser height + 0.5)
    /// (floor(stairs height / riser height + 0.5) when that is below 2)".
    #[test]
    fn s01b_the_inverse_run_length() {
        // top 10, base 4, riser 0.5 -> floor(12.5) = 12 ; both risers: T x (12 - 1)
        let n = desired_risers_of_run(10.0, 4.0, 10.0, 0.5);
        assert_eq!(n, 12);
        assert!(near(run_total_length(1.0, n, true, true), 11.0));
        assert!(near(run_total_length(1.0, n, false, true), 12.0));
        assert!(near(run_total_length(1.0, n, false, false), 13.0));
        assert_eq!(count_risers_for_run(11.0, 1.0, true, true), n);
        // below 2: floor(stairs height / riser + 0.5) = floor(20.5) = 20
        assert_eq!(desired_risers_of_run(10.0, 9.8, 10.0, 0.5), 20);
        // exactly 2 is not below 2
        assert_eq!(desired_risers_of_run(10.0, 9.0, 10.0, 0.5), 2);
    }

    /// Spiral: "the same count formula with L replaced by the arc angle and T by the angle per tread = 2 x asin(T / (2R))
    /// (T / R when T / 2 > R)".
    #[test]
    fn s01b_spiral_count_uses_the_angle_per_tread() {
        let a = angle_per_tread(1.0, 2.0).unwrap();
        assert!((a - 2.0 * (0.25f64).asin()).abs() < 1e-15);
        // T / 2 > R: T / R
        assert_eq!(angle_per_tread(1.0, 0.4), Some(2.5));
        // T / 2 == R is NOT greater: the asin branch, asin(1) = pi / 2
        assert!((angle_per_tread(1.0, 0.5).unwrap() - std::f64::consts::PI).abs() < 1e-15);
        assert_eq!(angle_per_tread(1.0, 0.0), None);
        // a quarter circle (pi / 2) at R = 2, T = 1: floor(1.5708 / 0.5054 + 1.5) = 4 risers with both
        let n = spiral_riser_number(std::f64::consts::FRAC_PI_2, a, true, true);
        assert_eq!(n, ((std::f64::consts::FRAC_PI_2 / a) + 1.5).floor() as i32);
        assert_eq!(n, 4);
    }

    // ================================================================ Spec S-01c

    /// Spec S-01c: "n = round half away from zero of (requested elevation - stairs bottom elevation) / riser height,
    /// elevation = stairs bottom + n x riser height".
    #[test]
    fn s01c_landing_elevation_snaps_to_whole_risers() {
        assert_eq!(snap_landing_elevation(1.26, 0.0, 0.5), Some((1.5, 3)));
        assert_eq!(snap_landing_elevation(1.24, 0.0, 0.5), Some((1.0, 2)));
        // the half goes away from zero: 1.25 / 0.5 = 2.5 -> 3
        assert_eq!(snap_landing_elevation(1.25, 0.0, 0.5), Some((1.5, 3)));
        // a bottom that is not zero
        assert_eq!(snap_landing_elevation(3.0, 1.0, 0.5), Some((3.0, 4)));
        assert_eq!(snap_landing_elevation(3.0, 1.0, 0.0), None);
    }

    /// Spec S-01c: "clamped to the stairs bottom elevation when below it, and the landing's base riser index is that n"
    /// (round half away from zero is the negative half too: -0.5 -> -1).
    #[test]
    fn s01c_landing_elevation_is_clamped_to_the_stairs_bottom() {
        assert_eq!(snap_landing_elevation(-0.25, 0.0, 0.5), Some((0.0, -1)), "-0.5 rounds away from zero to -1");
        assert_eq!(snap_landing_elevation(-0.2, 0.0, 0.5), Some((0.0, 0)));
        assert_eq!(snap_landing_elevation(-3.0, 1.0, 0.5), Some((1.0, -8)));
    }

    /// Spec S-01c: "A base elevation is valid when its rounded riser count (relative to the stairs) is above 0" - the
    /// boundary is the half riser: 0.25 / 0.5 = 0.5 rounds to 1, 0.249 to 0.
    #[test]
    fn s01c_landing_validity_is_a_riser_count_above_zero() {
        assert!(landing_riser_count_is_valid(0.25, 0.5));
        assert!(!landing_riser_count_is_valid(0.249, 0.5));
        assert!(!landing_riser_count_is_valid(0.0, 0.5));
        assert!(!landing_riser_count_is_valid(-1.0, 0.5));
        assert!(landing_riser_count_is_valid(1.0, 0.5));
        // "the instance check also requires the elevation to be at least the tread thickness"
        assert!(landing_elevation_meets_tread_thickness(0.3, Some(0.3)));
        assert!(!landing_elevation_meets_tread_thickness(0.29, Some(0.3)));
        assert!(landing_elevation_meets_tread_thickness(0.0, None), "not evaluated");
    }

    /// "re-stored only when it changes by more than 1e-9 relative or the index is unset".
    #[test]
    fn s01c_s01e_landing_parameter_set() {
        let mut f = fx();
        let id = f.make(1.0);
        let out = f.doc.transact("landing", false, |m, cx| add_landing(m, cx, id, m_(2.26)));
        assert!(out.committed(), "{:?}", out.failures);
        let l = out.value.unwrap();
        // 2.26 / 0.5 = 4.52 -> 5 -> 2.5 ft
        let lan = f.doc.model().stairs.landings[&l].clone();
        assert_eq!(lan.base_riser_index, Some(5));
        assert!(near(lan.base_elevation, m_(2.5)), "{}", lan.base_elevation / FT);

        let before = f.doc.model().clone();
        let out = f.doc.transact("bad", false, |m, cx| set_landing_base_elevation(m, cx, l, m_(0.1)));
        assert_eq!(out.status, TransactionStatus::RolledBack);
        assert_eq!(codes_of(&out), vec![codes::STAIRS_LANDING_HEIGHT_UNDER_STAIRS_BOTTOM]);
        assert_eq!(f.doc.model(), &before);

        let out = f.doc.transact("good", false, |m, cx| set_landing_base_elevation(m, cx, l, m_(3.9)));
        assert!(out.committed(), "{:?}", out.failures);
        let lan = &f.doc.model().stairs.landings[&l];
        assert_eq!(lan.base_riser_index, Some(8));
        assert!(near(lan.base_elevation, m_(4.0)));
    }

    /// Spec S-01c: the instance check against the tread thickness (when the type states one).
    #[test]
    fn s01c_landing_instance_check_uses_the_type_tread_thickness() {
        let mut f = fx();
        let ty = f.ty;
        let mut p = plain_type();
        p.tread_thickness = Some(m_(0.9));
        f.doc.transact("t", false, |m, cx| set_stairs_type_params(m, cx, ty, p));
        let id = f.make(1.0);
        let l = f.doc.transact("a", false, |m, cx| add_landing(m, cx, id, m_(2.0))).value.unwrap();
        // 0.5 ft rounds to one riser (valid) but is below the 0.9 ft tread thickness
        let out = f.doc.transact("low", false, |m, cx| set_landing_base_elevation(m, cx, l, m_(0.5)));
        assert_eq!(codes_of(&out), vec![codes::STAIRS_LANDING_HEIGHT_UNDER_STAIRS_BOTTOM]);
        let out = f.doc.transact("ok", false, |m, cx| set_landing_base_elevation(m, cx, l, m_(1.0)));
        assert!(out.committed(), "{:?}", out.failures);
    }

    /// Spec S-01c: the four end pairings, in the rule's order, and "end1 to end1 with both runs above the stairs bottom +
    /// 1e-9" - the boundary at + 1e-9.
    #[test]
    fn s01c_automatic_landing_pairings() {
        use RunEnd::*;
        let all = automatic_landing_pairings(1.0, 2.0, 0.0);
        assert_eq!(
            all,
            vec![
                EndPairing { a: End2, b: End1 },
                EndPairing { a: End1, b: End2 },
                EndPairing { a: End1, b: End1 },
                EndPairing { a: End2, b: End2 },
            ]
        );
        // a run exactly at stairs bottom + 1e-9 is not above: the end1-end1 pairing is absent
        let three = automatic_landing_pairings(1e-9, 2.0, 0.0);
        assert_eq!(three.len(), 3);
        assert!(!three.contains(&EndPairing { a: End1, b: End1 }));
        let four = automatic_landing_pairings(2e-9, 2.0, 0.0);
        assert_eq!(four.len(), 4);
    }

    /// Spec S-01c: two different runs that form a proper pair, a valid landing type and, for at least one end pairing, a
    /// boundary that computes. The unspecified predicates are injected.
    #[test]
    fn s01c_automatic_landing_needs_one_computable_pairing() {
        let (a, b) = (ElementId(1), ElementId(2));
        let p = automatic_landing_pairings(1.0, 2.0, 0.0);
        let ok_all = |_p: EndPairing| true;
        assert!(landing_can_be_generated(a, b, true, true, &p, ok_all, ok_all));
        assert!(!landing_can_be_generated(a, a, true, true, &p, ok_all, ok_all), "two different runs");
        assert!(!landing_can_be_generated(a, b, false, true, &p, ok_all, ok_all), "proper pair");
        assert!(!landing_can_be_generated(a, b, true, false, &p, ok_all, ok_all), "landing type");
        // only the last pairing computes: still "at least one"
        let last = p[p.len() - 1];
        assert!(landing_can_be_generated(a, b, true, true, &p, ok_all, |q| q == last));
        assert!(!landing_can_be_generated(a, b, true, true, &p, ok_all, |_| false));
    }


    fn paths(n: usize) -> RailingPathSet {
        RailingPathSet {
            paths: (0..n).map(|i| RailingPath { keys: vec![StairsSubelementKey(100 + i as u64)] }).collect(),
            reported_count: n,
        }
    }

    /// "creates a railing per railing path ... for each path index not excluded by the caller's index list ..., stores the
    /// stairs sub-element keys of the path on the railing and appends its id to the output array".
    #[test]
    fn s01d_one_railing_per_path_minus_the_excluded_indices() {
        let mut f = fx();
        let id = f.make(1.0);
        let rtype = ElementId(9999);
        let ps = paths(4);
        let out = f.doc.transact("rail", false, |m, cx| generate_stair_railings(m, cx, id, rtype, &ps, &[1, 3]));
        assert!(out.committed(), "{:?}", out.failures);
        let ids = out.value.unwrap();
        assert_eq!(ids.len(), 2);
        let m = f.doc.model();
        assert_eq!(m.stairs.railings[&ids[0]].path_index, 0);
        assert_eq!(m.stairs.railings[&ids[1]].path_index, 2);
        assert_eq!(m.stairs.railings[&ids[1]].subelement_keys, vec![StairsSubelementKey(102)]);
        assert_eq!(m.stairs.railings[&ids[0]].railing_type, rtype);
        assert_eq!(m.stairs.railings[&ids[0]].stairs, id);
    }

    /// Otherwise it fails with error 1" and "The call fails (error 1) when no railing results".
    #[test]
    fn s01d_the_count_must_agree_and_a_railing_must_result() {
        let mut f = fx();
        let id = f.make(1.0);
        let mut ps = paths(3);
        ps.reported_count = 2;
        let out = f.doc.transact("rail", false, |m, cx| generate_stair_railings(m, cx, id, ElementId(1), &ps, &[]));
        assert_eq!(out.status, TransactionStatus::RolledBack);
        assert_eq!(codes_of(&out), vec![codes::STAIRS_RAILING_CREATION_FAILED]);
        // every path excluded: no railing results
        let ps = paths(2);
        let out = f.doc.transact("rail", false, |m, cx| generate_stair_railings(m, cx, id, ElementId(1), &ps, &[0, 1]));
        assert_eq!(codes_of(&out), vec![codes::STAIRS_RAILING_CREATION_FAILED]);
        // no paths at all
        let ps = paths(0);
        let out = f.doc.transact("rail", false, |m, cx| generate_stair_railings(m, cx, id, ElementId(1), &ps, &[]));
        assert_eq!(codes_of(&out), vec![codes::STAIRS_RAILING_CREATION_FAILED]);
        assert!(f.doc.model().stairs.railings.is_empty());
    }

    /// A stairs of a multistory group "first looks for an existing railing at that location and reuses it" - the choice is
    /// not specified, the engine refuses.
    #[test]
    fn s01d_a_multistory_member_is_refused_as_undecided() {
        let mut f = fx();
        let id = f.make(1.0);
        f.doc.transact("m", false, |m, cx| set_multistory_member(m, cx, id, true));
        let ps = paths(2);
        let out = f.doc.transact("rail", false, |m, cx| generate_stair_railings(m, cx, id, ElementId(1), &ps, &[]));
        assert_eq!(codes_of(&out), vec![codes::STAIRS_UNDECIDED_BRANCH]);
    }


    /// "an integer value that is not positive for the desired number of risers parameter -> StairsIllegalDesiredNumRisers"
    /// - rejected before any change.
    #[test]
    fn s01e_a_non_positive_desired_number_is_rejected_at_parameter_set() {
        let mut f = fx();
        let id = f.make(1.0);
        let before = f.doc.model().clone();
        for n in [0, -1, -20] {
            let out = f.doc.transact("n", false, |m, cx| set_riser_count_target(m, cx, id, n));
            assert_eq!(out.status, TransactionStatus::RolledBack, "n = {n}");
            assert_eq!(codes_of(&out), vec![codes::STAIRS_ILLEGAL_DESIRED_RISERS]);
        }
        assert_eq!(f.doc.model(), &before);
    }

    /// "a double value that is not positive for the actual tread depth parameter ->
    /// StairsActualTreadDepthLessThanMinimumFailure" - the same failure definition the regeneration warning uses, here
    /// rejected at parameter set (zero is not positive; a tiny positive is).
    #[test]
    fn s01e_a_non_positive_tread_depth_is_rejected_at_parameter_set() {
        let mut f = fx();
        let id = f.make(1.0);
        for v in [0.0, -0.1] {
            let out = f.doc.transact("t", false, |m, cx| set_actual_tread_depth(m, cx, id, v));
            assert_eq!(out.status, TransactionStatus::RolledBack, "v = {v}");
            assert_eq!(codes_of(&out), vec![codes::STAIRS_TREAD_DEPTH_LESS_THAN_MINIMUM]);
            assert_eq!(out.failures[0].severity, Severity::Error);
        }
        assert!(near(f.stairs(id).actual_tread_depth, m_(1.0)));
        // positive but below the type minimum: accepted, the warning is posted at commit
        let out = f.doc.transact("t", false, |m, cx| set_actual_tread_depth(m, cx, id, m_(0.0001)));
        assert!(out.committed());
        assert!(out.failures.is_empty(), "no component run yet, so no tread warning");
    }

    /// StairsRun validity: "a new relative bottom elevation above the current top (by more than 1e-9) ->
    /// RunTopLowerThanBottomFailure".
    #[test]
    fn s01e_run_bottom_above_the_current_top() {
        let c = |bottom: f64| validate_run_bottom_elevation(1, bottom, 5.0, 0.0, None).err().map(|f| f.code);
        assert_eq!(c(5.0 + 2e-9).as_deref(), Some(codes::STAIRS_RUN_TOP_LOWER_THAN_BOTTOM));
        assert_eq!(c(5.0 + 1e-9), None, "exactly 1e-9 above is not more than 1e-9");
        assert_eq!(c(5.0), None);
        assert_eq!(c(4.0), None);
    }

    /// "... the minimum is UNKNOWN, so the branch exists only when it is supplied.
    #[test]
    fn s01e_run_bottom_leaving_less_than_the_minimum_height() {
        let c = |bottom: f64, min: Option<f64>| validate_run_bottom_elevation(3, bottom, 5.0, 0.0, min).err().map(|f| f.code);
        assert_eq!(c(4.9, Some(0.5)).as_deref(), Some(codes::STAIRS_RUN_HEIGHT_TOO_SMALL));
        assert_eq!(c(4.5, Some(0.5)), None, "exactly the minimum leaves enough");
        assert_eq!(c(4.9, None), None, "unknown minimum: not evaluated");
        // the cross check comes first
        assert_eq!(
            c(5.5, Some(0.5)).as_deref(),
            Some(codes::STAIRS_RUN_TOP_LOWER_THAN_BOTTOM),
        );
    }

    /// "a new relative top elevation below the current bottom (by more than 1e-9) -> RunBottomHigherThanTopFailure".
    #[test]
    fn s01e_run_top_below_the_current_bottom() {
        let c = |top: f64| validate_run_top_elevation(4, top, 2.0).err().map(|f| f.code);
        assert_eq!(c(2.0 - 2e-9).as_deref(), Some(codes::STAIRS_RUN_BOTTOM_HIGHER_THAN_TOP));
        assert_eq!(c(2.0 - 1e-9), None);
        assert_eq!(c(3.0), None);
    }

    /// "a bottom under the stairs bottom -> RunBottomUnderStairsBottomFailure; extend-below-base above the run top ->
    /// ExtendBelowBaseGreaterThanRunTopElevation".
    #[test]
    fn s01e_run_bottom_under_the_stairs_bottom_and_extend_below_base() {
        let e = validate_run_bottom_elevation(1, -0.1, 5.0, 0.0, None).unwrap_err();
        assert_eq!(e.code, codes::STAIRS_RUN_BOTTOM_UNDER_STAIRS_BOTTOM);
        assert!(validate_run_bottom_elevation(1, 0.0, 5.0, 0.0, None).is_ok());
        let e = validate_run_extend_below_base(1, 5.1, 5.0).unwrap_err();
        assert_eq!(e.code, codes::STAIRS_RUN_EXTEND_BELOW_BASE_ABOVE_TOP);
        assert!(validate_run_extend_below_base(1, 5.0, 5.0).is_ok());
    }

    /// The validity is stated for "run types 1, 3 and 4" only; the others are not evaluated here.
    #[test]
    fn s01e_run_validity_applies_to_run_types_1_3_and_4() {
        for t in [1u8, 3, 4] {
            assert!(validate_run_bottom_elevation(t, 6.0, 5.0, 0.0, None).is_err(), "type {t}");
            assert!(validate_run_top_elevation(t, 1.0, 2.0).is_err(), "type {t}");
        }
        for t in [0u8, 2, 5] {
            assert!(validate_run_bottom_elevation(t, 6.0, 5.0, 0.0, None).is_ok(), "type {t}");
            assert!(validate_run_top_elevation(t, 1.0, 2.0).is_ok(), "type {t}");
        }
    }

    /// The edit-scope start "throws ArgumentException for an id that is not a StairsType" - at the call.
    #[test]
    fn s01e_a_stairs_cannot_start_from_an_id_that_is_not_a_stairs_type() {
        let mut f = fx();
        let (base, top) = (f.base, f.top);
        let out = f.doc.transact("c", false, |m, cx| {
            create_stairs(m, cx, NewStairs { type_id: base, base_level: base, top_level: top, initial_tread_depth: 0.3 })
        });
        assert_eq!(codes_of(&out), vec![codes::STAIRS_TYPE_NOT_A_STAIRS_TYPE]);
        assert_eq!(out.status, TransactionStatus::RolledBack);
    }

    /// "InvalidOperationException 'Cannot commit StairsEditModeScope due to unresolved errors'" - raised when the scope
    /// commits with an error posted; a scope with only warnings commits.
    #[test]
    fn s01e_the_edit_scope_refuses_to_commit_with_unresolved_errors() {
        let mut f = fx();
        let id = f.make(1.0);
        let out = f.doc.transact("scope", false, |m, cx| commit_stairs_edit_scope(m, cx, id));
        assert!(out.committed(), "{:?}", out.failures);
        let out = f.doc.transact("scope", false, |m, cx| {
            m.stairs.stairs.get_mut(&id).unwrap().riser_count_floor = Some(100_000);
            commit_stairs_edit_scope(m, cx, id)
        });
        assert_eq!(out.status, TransactionStatus::RolledBack);
        assert_eq!(codes_of(&out), vec![codes::STAIRS_SCOPE_COMMIT_UNRESOLVED_ERRORS]);
    }

    /// "RegenerationFailedException when the regeneration after creation fails (each after rolling back its
    /// sub-transaction)" - the run is not kept.
    #[test]
    fn s01e_a_failing_regeneration_after_run_creation_rolls_the_run_back() {
        let mut f = fx();
        let id = f.make(1.0);
        let out = f.doc.transact("run", false, |m, cx| {
            m.stairs.stairs.get_mut(&id).unwrap().riser_count_floor = Some(100_000);
            add_straight_stair_run(
                m,
                cx,
                NewStraightRun {
                    stairs: id,
                    start: Vec2 { x: 0.0, y: 0.0 },
                    end: Vec2 { x: m_(3.0), y: 0.0 },
                    bottom_elevation: 0.0,
                    width: m_(4.0),
                    begin_with_riser: true,
                    end_with_riser: true,
                    run_type_code: 1,
                },
            )
        });
        assert_eq!(out.status, TransactionStatus::RolledBack);
        assert_eq!(codes_of(&out), vec![codes::STAIRS_REGENERATION_FAILED]);
        assert!(f.doc.model().stairs.runs.is_empty());
    }

    /// Derived state is a pure function of the model: a level move re-derives the desired number
    /// with no stored riser count, and `check_stairs` runs the riser step for the dependents.
    #[test]
    fn derived_state_follows_the_levels_and_regeneration_runs_for_dependents() {
        let mut f = fx();
        let id = f.make(1.0);
        let top = f.top;
        assert_eq!(stairs_state(f.doc.model(), id).unwrap().desired_number_of_risers, 20);
        let out = f.doc.transact("move", false, |m, cx| {
            m.levels.get_mut(&top).unwrap().elevation = m_(12.0);
            let deps = m.stairs.stairs_depending_on(top);
            check_stairs(m, cx, &deps);
            Ok(())
        });
        assert!(out.committed(), "{:?}", out.failures);
        // regeneration: n = ceil(12 / 0.5 - 1e-9) = 24 -> riser 0.5 ft
        assert!(near(f.riser_ft(id), 0.5));
        assert_eq!(stairs_state(f.doc.model(), id).unwrap().desired_number_of_risers, 24);
        // 13 ft: n = ceil(26 - 1e-9) = 26 -> still 0.5; 12.9 -> ceil(25.8) = 26 -> riser 0.4962
        f.doc.transact("move", false, |m, cx| {
            m.levels.get_mut(&top).unwrap().elevation = m_(12.9);
            check_stairs(m, cx, &[id]);
            Ok(())
        });
        assert!(near(f.riser_ft(id), 12.9 / 26.0), "{}", f.riser_ft(id));
    }

    /// Spec L-01, through the real level operation: moving the top level of a stairs re-runs its riser step in the same
    /// transaction (no direct call of `check_stairs`). Without the wiring the riser height would stay at 0.5 ft (stale:
    /// 12.9 ft over 20 risers).
    #[test]
    fn moving_a_level_regenerates_the_stairs_that_name_it() {
        let mut f = fx();
        let id = f.make(1.0);
        let (base, top) = (f.base, f.top);
        assert!(near(f.riser_ft(id), 0.5));

        // an unrelated level: the stairs is not a dependent of it and nothing changes
        let other = f
            .doc
            .transact("other level", false, |m, cx| create_level(m, cx, "L-other", m_(30.0)))
            .value
            .unwrap();
        let before = f.stairs(id).clone();
        let out = f.doc.transact("move other", false, |m, cx| crate::ops::set_level_elevation(m, cx, other, m_(31.0)));
        assert!(out.committed(), "{:?}", out.failures);
        assert_eq!(f.stairs(id), &before, "a stairs is regenerated only for its own levels");

        // the top level: 12.9 ft -> n = ceil(12.9 / 0.5 - 1e-9) = 26 -> riser 12.9 / 26 ft
        let out = f.doc.transact("raise top", false, |m, cx| crate::ops::set_level_elevation(m, cx, top, m_(12.9)));
        assert!(out.committed(), "{:?}", out.failures);
        assert!(near(f.riser_ft(id), 12.9 / 26.0), "{}", f.riser_ft(id));
        assert_eq!(stairs_state(f.doc.model(), id).unwrap().desired_number_of_risers, 26);

        // the base level is a dependent too: base at 2.9 ft -> height 10 ft -> n = 20 -> 0.5 ft
        let out = f.doc.transact("raise base", false, |m, cx| crate::ops::set_level_elevation(m, cx, base, m_(2.9)));
        assert!(out.committed(), "{:?}", out.failures);
        assert!(near(f.riser_ft(id), 0.5), "{}", f.riser_ft(id));

        // a failure posted by the regeneration reaches the commit and rolls the level move back: height 0.0004 ft -> one
        // riser of 0.0004 ft, below the vertex tolerance.
        let out = f.doc.transact("collapse", false, |m, cx| crate::ops::set_level_elevation(m, cx, top, m_(2.9004)));
        assert_eq!(out.status, TransactionStatus::RolledBack);
        assert_eq!(codes_of(&out), vec![codes::STAIRS_RISER_HEIGHT_TOO_SMALL]);
        assert!(near(f.doc.model().levels[&top].elevation, m_(12.9)), "the level move rolled back");
        assert!(near(f.riser_ft(id), 0.5), "and so did the regeneration");
    }

    /// An unknown id is the engine's ordinary not-found, never a panic.
    #[test]
    fn unknown_ids_are_not_found() {
        let mut f = fx();
        let out = f.doc.transact("n", false, |m, cx| set_riser_count_target(m, cx, ElementId(777), 3));
        assert_eq!(codes_of(&out), vec![codes::NOT_FOUND]);
        assert!(stairs_state(f.doc.model(), ElementId(777)).is_err());
    }

    /// ASSUMED (structural, see `delete_stairs`): the generic delete reaches a stairs and removes the
    /// components that name it; the type and the levels stay; undo restores everything.
    #[test]
    fn deleting_a_stairs_removes_its_components_and_undo_restores_them() {
        let mut f = fx();
        let id = f.make(1.0);
        let run = f
            .doc
            .transact("run", false, |m, cx| {
                add_straight_stair_run(
                    m,
                    cx,
                    NewStraightRun {
                        stairs: id,
                        start: Vec2 { x: 0.0, y: 0.0 },
                        end: Vec2 { x: m_(10.0), y: 0.0 },
                        bottom_elevation: 0.0,
                        width: m_(4.0),
                        begin_with_riser: true,
                        end_with_riser: true,
                        run_type_code: 1,
                    },
                )
            })
            .value
            .unwrap();
        let landing = f.doc.transact("landing", false, |m, cx| add_landing(m, cx, id, m_(5.0))).value.unwrap();
        let out = f.doc.transact("delete", false, |m, cx| crate::ops::delete(m, cx, id));
        assert!(out.committed(), "{:?}", out.failures);
        let mut want = vec![id, run, landing];
        want.sort();
        assert_eq!(out.value.unwrap(), want);
        let m = f.doc.model();
        assert!(m.stairs.stairs.is_empty() && m.stairs.runs.is_empty() && m.stairs.landings.is_empty());
        assert!(m.stairs.types.contains_key(&f.ty) && m.levels.contains_key(&f.base));
        // an unknown id keeps the engine's ordinary not-found
        let out = f.doc.transact("delete", false, |m, cx| crate::ops::delete(m, cx, id));
        assert_eq!(codes_of(&out), vec![codes::NOT_FOUND]);
        assert!(f.doc.undo().is_ok());
        assert!(f.doc.model().stairs.stairs.contains_key(&id));
    }
}
