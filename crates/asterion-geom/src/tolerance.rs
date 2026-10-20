//! Modelling tolerances.
//!
//! The defaults reproduce the values *observed* through the public API of the vendor (run 2026-10-12):
//!
//! | quantity | internal (ft) | metres |
//! |---|---|---|
//! | ShortCurveTolerance | 0.0025602645572916664 | 0.0007803686370625 |
//! | VertexTolerance | 0.0005233832795 | 0.0001595272235916 |
//! | AngleTolerance | — | 0.0017453292519943279 rad (0.1°) |
//!
//! They are stored in metres here; the foot values are kept for traceability.

use serde::{Deserialize, Serialize};

/// The short-curve tolerance, in the unit of the curve coordinates (FEET): the same literal for every controller type and
/// version.
pub const SHORT_CURVE_TOL_FT: f64 = 0.002_560_264_557_291_666_4;

/// A curve is too short exactly when its length, in the unit of the coordinates (feet), is STRICTLY less than
/// [`SHORT_CURVE_TOL_FT`]. (The length of a bounded arc is `(t1 - t0) R`, 0 when the span is under 1e-9; of a bounded line
/// `|t1 - t0|`: see [`arc_curve_length_ft`].) This is the predicate the thicken-and-extrude route answers Err 9 with.
pub fn is_too_short_ft(length_ft: f64) -> bool {
    length_ft < SHORT_CURVE_TOL_FT
}

/// For a bounded arc: `(t1 - t0) R`, and 0 when `t1 - t0 < 1e-9`.
pub fn arc_curve_length_ft(span: f64, radius_ft: f64) -> f64 {
    if span < 1e-9 {
        0.0
    } else {
        span * radius_ft
    }
}

/// (C) (3D, double): two points are equal when `S = |a|^2 + |b|^2` is below 1e-18 or `|a - b|^2 <= 1e-18 S` - a RELATIVE
/// tolerance that grows with the distance from the origin (`|a - b| <= 1e-9 sqrt(S)`). The unit is that of the coordinates
/// passed.
pub fn vec_almost_equal(a: crate::vec::Vec3, b: crate::vec::Vec3) -> bool {
    let s = a.x * a.x + a.y * a.y + a.z * a.z + b.x * b.x + b.y * b.y + b.z * b.z;
    if s < 1e-18 {
        return true;
    }
    let (dx, dy, dz) = (a.x - b.x, a.y - b.y, a.z - b.z);
    dx * dx + dy * dy + dz * dz <= 1e-18 * s
}

/// (C) : per component, equal when `|x| + |y| < 1e-9` or `|x - y| <= 1e-9 (|x| + |y|)`.
pub fn vec_almost_equal_2d(a: crate::vec::Vec2, b: crate::vec::Vec2) -> bool {
    relatively_equal(a.x, b.x) && relatively_equal(a.y, b.y)
}

pub fn relatively_equal(x: f64, y: f64) -> bool {
    let s = x.abs() + y.abs();
    s < 1e-9 || (x - y).abs() <= 1e-9 * s
}

/// (C) : different when `S >= 1e-18` and `|a - b|^2 > 1e-18 S`.
pub fn points_are_different_2d(a: crate::vec::Vec2, b: crate::vec::Vec2) -> bool {
    let s = a.dot(a) + b.dot(b);
    let d = a - b;
    s >= 1e-18 && d.dot(d) > 1e-18 * s
}

/// (G) : a finite value with `|x| <= 30000.000000001` (FEET, the reference product's internal unit).
pub fn is_length_ft(x: f64) -> bool {
    x.is_finite() && x.abs() <= 30000.000000001
}

/// (G) : `1e-9 <= x <= 30000.000000001` (feet).
pub fn is_positive_length_ft(x: f64) -> bool {
    (1e-9..=30000.000000001).contains(&x)
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tolerances {
    /// Curves shorter than this are rejected as degenerate.
    pub short_curve: f64,
    /// Points closer than this are considered coincident.
    pub vertex: f64,
    /// Directions within this angle (radians) are considered parallel.
    pub angle: f64,
}

impl Tolerances {
    pub const REFERENCE_COMPATIBLE: Tolerances = Tolerances {
        short_curve: SHORT_CURVE_TOL_FT * 0.3048,
        vertex: 0.000_523_383_279_5 * 0.3048,
        angle: 0.001_745_329_251_994_327_9,
    };
}

impl Default for Tolerances {
    fn default() -> Self {
        Tolerances::REFERENCE_COMPATIBLE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Strictly less than 0.0025602645572916664 ft; equal is NOT too short.
    #[test]
    fn too_short_is_a_strict_comparison_in_feet() {
        assert_eq!(SHORT_CURVE_TOL_FT, 0.0025602645572916664);
        assert!(is_too_short_ft(0.0025602645572916660));
        assert!(!is_too_short_ft(SHORT_CURVE_TOL_FT));
        assert!(!is_too_short_ft(0.0025602645572916668));
        assert!(is_too_short_ft(0.0));
        // arcs: (t1 - t0) R; a span under 1e-9 has length 0 whatever the radius
        assert!((arc_curve_length_ft(1.0, 0.002) - 0.002).abs() < 1e-18);
        assert_eq!(arc_curve_length_ft(5e-10, 1.0e6), 0.0);
        assert!(is_too_short_ft(arc_curve_length_ft(5e-10, 1.0e6)));
        // 0.78 mm: the same literal in metres
        assert!((Tolerances::default().short_curve - SHORT_CURVE_TOL_FT * 0.3048).abs() < 1e-18);
    }

    #[test]
    fn defaults_match_observed_values_in_metres() {
        let t = Tolerances::default();
        assert!((t.short_curve - 0.000_780_368_637_062_5).abs() < 1e-15);
        assert!((t.vertex - 0.000_159_527_223_591_6).abs() < 1e-15);
        assert!((t.angle - 0.1_f64.to_radians()).abs() < 1e-15);
    }

    /// (C)/(D)/(G): the relative point test grows with the distance from the origin, the absolute floor 1e-18 on the
    /// squared lengths, the length bounds in feet.
    #[test]
    fn k13_relative_equality_and_length_bounds() {
        use crate::vec::{Vec2, Vec3};
        let far = Vec3::new(100.0, 0.0, 0.0);
        assert!(vec_almost_equal(far, Vec3::new(100.0 + 1.0e-7, 0.0, 0.0)), "1e-7 at 100 units coincides");
        assert!(!vec_almost_equal(far, Vec3::new(100.0 + 1.0e-6, 0.0, 0.0)));
        assert!(vec_almost_equal(Vec3::new(1e-10, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)), "both near the origin");
        assert!(!vec_almost_equal(Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0 + 1e-8, 0.0, 0.0)));
        assert!(vec_almost_equal_2d(Vec2::new(1.0, 2.0), Vec2::new(1.0 + 5e-10, 2.0)));
        assert!(!points_are_different_2d(Vec2::new(0.0, 0.0), Vec2::new(1e-10, 0.0)));
        assert!(points_are_different_2d(Vec2::new(1.0, 0.0), Vec2::new(1.0 + 1e-6, 0.0)));
        assert!(is_length_ft(30000.0) && !is_length_ft(30000.1) && !is_length_ft(f64::NAN));
        assert!(is_positive_length_ft(1e-9) && !is_positive_length_ft(5e-10));
    }
}
