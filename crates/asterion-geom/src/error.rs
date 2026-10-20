//! Geometry failures.
//!
//! `asterion-geom` is a leaf crate (it depends on nothing but `serde`), so it
//! carries its own failure type. Codes are **ASTERION's own**; they are stable
//! strings so higher layers (`asterion-bim`, the compat harness) can map them to
//! element-level diagnostics without string-matching on messages.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable geometry failure codes.
pub mod codes {
    pub const LOOP_NOT_CLOSED: &str = "GEOM_LOOP_NOT_CLOSED";
    pub const LOOP_TOO_FEW_CURVES: &str = "GEOM_LOOP_TOO_FEW_CURVES";
    pub const CURVE_TOO_SHORT: &str = "GEOM_CURVE_TOO_SHORT";
    pub const DEGENERATE_ARC: &str = "GEOM_DEGENERATE_ARC";
    pub const NON_POSITIVE_EXTRUSION: &str = "GEOM_NON_POSITIVE_EXTRUSION";
    pub const HOLE_NOT_INSIDE_OUTER_LOOP: &str = "GEOM_HOLE_NOT_INSIDE_OUTER_LOOP";
    pub const OFFSET_COLLAPSES_CURVE: &str = "GEOM_OFFSET_COLLAPSES_CURVE";
    pub const POINTS_COLLINEAR: &str = "GEOM_POINTS_COLLINEAR";
    pub const ZERO_WIDTH_BAND: &str = "GEOM_ZERO_WIDTH_BAND";
    pub const BOOLEAN_DID_NOT_CLOSE: &str = "GEOM_BOOLEAN_DID_NOT_CLOSE";
    pub const SWEEP_SELF_INTERSECTS: &str = "GEOM_SWEEP_SELF_INTERSECTS";
    pub const SWEEP_UNSUPPORTED_CORNER: &str = "GEOM_SWEEP_UNSUPPORTED_CORNER";
    pub const SWEEP_NO_CLOSED_FORM: &str = "GEOM_SWEEP_NO_CLOSED_FORM";
    pub const SWEEP_CONVENTION_UNSUPPORTED: &str = "GEOM_SWEEP_CONVENTION_UNSUPPORTED";
    pub const ROOF_FOOTPRINT_NOT_CONVEX: &str = "GEOM_ROOF_FOOTPRINT_NOT_CONVEX";
    pub const ROOF_UNEQUAL_SLOPE_JOIN: &str = "GEOM_ROOF_UNEQUAL_SLOPE_JOIN";
    pub const ROOF_CUT_OFF_NOT_ABOVE_BOTTOM: &str = "GEOM_ROOF_CUT_OFF_NOT_ABOVE_BOTTOM";
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum GeomError {
    /// The last curve of a closed loop does not come back to the first point
    /// within the vertex tolerance. `gap` is the measured distance in metres.
    LoopNotClosed { gap: f64, tolerance: f64 },
    /// A closed loop needs at least 2 curves (two arcs can close a disc); a
    /// polygonal loop needs at least 3.
    LoopTooFewCurves { count: usize, minimum: usize },
    /// Curve shorter than `Tolerances::short_curve`.
    CurveTooShort { length: f64, tolerance: f64 },
    /// Arc with a non-positive radius or a zero sweep.
    DegenerateArc { radius: f64, sweep: f64 },
    /// Extrusion with `z1 <= z0`.
    NonPositiveExtrusion { z0: f64, z1: f64 },
    /// An inner loop (hole) is not contained in the outer loop.
    HoleNotInsideOuterLoop { hole_index: usize },
    /// Offsetting an arc by more than its radius (toward the centre) would
    /// collapse or invert it.
    OffsetCollapsesCurve { radius: f64, distance: f64 },
    /// Three points that should define an arc are collinear within tolerance.
    PointsCollinear { area2: f64 },
    /// An offset band whose two offsets do not add up to a positive width.
    ZeroWidthBand { left: f64, right: f64 },
    /// A boolean operation produced boundary pieces that do not form closed
    /// loops (degenerate or self-intersecting input).
    BooleanDidNotClose { open_pieces: usize },
    /// Some part of the profile would be swept a non-positive length along a
    /// path segment: it crosses the segment's axis of curvature (an arc) or
    /// passes through a mitre's apex (a sharp corner). The solid would
    /// self-intersect. `min_length` is the most negative swept length found.
    SweepSelfIntersects { segment: usize, min_length: f64 },
    /// A non-tangent corner involving an arc (only line-line corners can be
    /// mitred exactly).
    SweepUnsupportedCorner { index: usize, angle: f64 },
    /// The requested quantity has no elementary closed form for this sweep
    /// (on a sloped path, the skin swept by an *arc* of the profile is an
    /// elliptic integral). Reported instead of tessellating.
    SweepNoClosedForm { profile_curve: usize },
    /// The perpendicular-profile convention is only supported on a straight
    /// path; `segment` is the first segment that is not collinear.
    SweepConventionUnsupported { segment: usize },
    /// A footprint roof with sloped edges over a non-convex footprint: the
    /// lower envelope of the edge planes is not the straight-skeleton roof
    /// there, so the kernel declines instead of answering wrongly.
    RoofFootprintNotConvex,
    /// Facets of different slopes meet: the reference's join there is not
    /// resolved yet (one measured case, reproduced by neither candidate), so
    /// thickness-dependent quantities are refused rather than approximated.
    RoofUnequalSlopeJoin,
    /// A cut-off plane at or below the roof's underside (OBSERVED refused by
    /// the reference at commit, including exactly level with the base).
    RoofCutOffNotAboveBottom,
}

impl GeomError {
    pub fn code(&self) -> &'static str {
        match self {
            GeomError::LoopNotClosed { .. } => codes::LOOP_NOT_CLOSED,
            GeomError::LoopTooFewCurves { .. } => codes::LOOP_TOO_FEW_CURVES,
            GeomError::CurveTooShort { .. } => codes::CURVE_TOO_SHORT,
            GeomError::DegenerateArc { .. } => codes::DEGENERATE_ARC,
            GeomError::NonPositiveExtrusion { .. } => codes::NON_POSITIVE_EXTRUSION,
            GeomError::HoleNotInsideOuterLoop { .. } => codes::HOLE_NOT_INSIDE_OUTER_LOOP,
            GeomError::OffsetCollapsesCurve { .. } => codes::OFFSET_COLLAPSES_CURVE,
            GeomError::PointsCollinear { .. } => codes::POINTS_COLLINEAR,
            GeomError::ZeroWidthBand { .. } => codes::ZERO_WIDTH_BAND,
            GeomError::BooleanDidNotClose { .. } => codes::BOOLEAN_DID_NOT_CLOSE,
            GeomError::SweepSelfIntersects { .. } => codes::SWEEP_SELF_INTERSECTS,
            GeomError::SweepUnsupportedCorner { .. } => codes::SWEEP_UNSUPPORTED_CORNER,
            GeomError::SweepNoClosedForm { .. } => codes::SWEEP_NO_CLOSED_FORM,
            GeomError::SweepConventionUnsupported { .. } => codes::SWEEP_CONVENTION_UNSUPPORTED,
            GeomError::RoofFootprintNotConvex => codes::ROOF_FOOTPRINT_NOT_CONVEX,
            GeomError::RoofUnequalSlopeJoin => codes::ROOF_UNEQUAL_SLOPE_JOIN,
            GeomError::RoofCutOffNotAboveBottom => codes::ROOF_CUT_OFF_NOT_ABOVE_BOTTOM,
        }
    }
}

impl fmt::Display for GeomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GeomError::LoopNotClosed { gap, tolerance } => {
                write!(f, "{}: loop open by {gap} m (tolerance {tolerance} m)", self.code())
            }
            GeomError::LoopTooFewCurves { count, minimum } => {
                write!(f, "{}: {count} curves, at least {minimum} required", self.code())
            }
            GeomError::CurveTooShort { length, tolerance } => {
                write!(f, "{}: curve of {length} m is below {tolerance} m", self.code())
            }
            GeomError::DegenerateArc { radius, sweep } => {
                write!(f, "{}: radius {radius} m, sweep {sweep} rad", self.code())
            }
            GeomError::NonPositiveExtrusion { z0, z1 } => {
                write!(f, "{}: z0 {z0} m, z1 {z1} m", self.code())
            }
            GeomError::HoleNotInsideOuterLoop { hole_index } => {
                write!(f, "{}: inner loop #{hole_index}", self.code())
            }
            GeomError::OffsetCollapsesCurve { radius, distance } => {
                write!(f, "{}: radius {radius} m, offset {distance} m", self.code())
            }
            GeomError::PointsCollinear { area2 } => {
                write!(f, "{}: twice the triangle area is {area2}", self.code())
            }
            GeomError::ZeroWidthBand { left, right } => {
                write!(f, "{}: left {left} m, right {right} m", self.code())
            }
            GeomError::BooleanDidNotClose { open_pieces } => {
                write!(f, "{}: {open_pieces} boundary pieces left over", self.code())
            }
            GeomError::SweepSelfIntersects { segment, min_length } => {
                write!(f, "{}: segment {segment}, swept length {min_length} m", self.code())
            }
            GeomError::SweepUnsupportedCorner { index, angle } => {
                write!(f, "{}: corner {index}, turn {angle} rad", self.code())
            }
            GeomError::SweepNoClosedForm { profile_curve } => {
                write!(f, "{}: profile curve {profile_curve} is an arc", self.code())
            }
            GeomError::SweepConventionUnsupported { segment } => {
                write!(f, "{}: segment {segment} is not collinear", self.code())
            }
            GeomError::RoofFootprintNotConvex => write!(f, "{}", self.code()),
            GeomError::RoofUnequalSlopeJoin => write!(f, "{}", self.code()),
            GeomError::RoofCutOffNotAboveBottom => write!(f, "{}", self.code()),
        }
    }
}

impl std::error::Error for GeomError {}

pub type GeomResult<T> = Result<T, GeomError>;
