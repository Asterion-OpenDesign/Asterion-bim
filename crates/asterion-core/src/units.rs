//! Units. ASTERION stores lengths in metres, areas in m², volumes in m³ and angles
//! in radians, as `f64`. Foreign unit systems are converted only at adapter
//! boundaries.

/// International foot, exact by definition (1959 agreement).
pub const METRES_PER_FOOT: f64 = 0.3048;

pub fn feet_to_metres(ft: f64) -> f64 {
    ft * METRES_PER_FOOT
}

pub fn metres_to_feet(m: f64) -> f64 {
    m / METRES_PER_FOOT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foot_conversion_round_trips() {
        for v in [0.0, 1.0, 3.5, -0.25, 1e-6, 12345.678] {
            assert!((feet_to_metres(metres_to_feet(v)) - v).abs() <= 1e-12 * v.abs().max(1.0));
        }
        assert_eq!(feet_to_metres(1.0), 0.3048);
    }
}
