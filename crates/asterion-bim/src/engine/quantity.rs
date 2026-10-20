//! Host quantities (`Area`, `Volume`) of floors and ceilings, as one pure rule set.
//!
//! Source: the specification rules (Ceiling and Floor share), Spec F-02a (Floor Area), Spec F-02b (Floor Volume) and (a
//! slope arrow or defining edge does not move the sketch loops; the solid comes). Written specification only.
//!
//! * **Area**: two passes of the face getter, pass 1 the top faces and pass 2 the bottom faces; each pass sums over the
//!   returned faces; the Area is the LARGER of the two sums; an absolute value of 1e-9 or less is error 7 (no area is
//!   produced). The faces are the actual top and bottom faces of the solid, so for a sloped ceiling they are the inclined
//!   planar faces. There is no branch for a sloped ceiling.
//! * In every other case of the solid (a null solid is error 7). A sloped ceiling made with a slope arrow or defining edge
//!   has no shape-edit step, so its constant-offset test is TRUE and its Volume is the solid volume.
//!
//! [`GLOBAL_FLAG_CLEAR`] is that value, not a measurement of a running product; how the parameter layer outside the
//! reference product turns error 7 into the value a user sees (Spec C-02: a 2 mm sketch commits and reads Area 0); the slab
//! shape editor does not exist in ASTERION, so the constant-offset input is always `true` for the engine's own slabs and
//! the `Area x thickness` branch is reachable only through this module's tests.

/// Feet per metre (the reference's internal unit is the foot thresholds are in it).
pub const FEET_PER_METRE: f64 = 1.0 / 0.3048;

/// The smallest area / thickness (internal units, feet) that is a quantity; at or below it the operation returns error 7.
pub const MIN_QUANTITY_FT: f64 = 1e-9;

pub const GLOBAL_FLAG_CLEAR: bool = true;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuantityError {
    /// Error 7: no area produced (`|area| <= 1e-9`), no thickness (`|t| <= 1e-9`) or no solid.
    NotAvailable,
}

/// Area: the larger of the summed areas of the top faces and of the bottom faces, in SQUARE METRES in and out; error when
/// the larger sum is at or below 1e-9 square feet.
pub fn host_area(top_face_areas: &[f64], bottom_face_areas: &[f64]) -> Result<f64, QuantityError> {
    let top: f64 = top_face_areas.iter().sum();
    let bottom: f64 = bottom_face_areas.iter().sum();
    let area = if top >= bottom { top } else { bottom };
    if (area * FEET_PER_METRE * FEET_PER_METRE).abs() <= MIN_QUANTITY_FT {
        return Err(QuantityError::NotAvailable);
    }
    Ok(area)
}

/// Volume, cubic metres in and out.
///
/// * `area`: the value of [`host_area`];
/// * `solid_volume`: of the element solid, `None` for a null solid.
/// * `constant_thickness`: `Some(t)` when the type reports a constant thickness `t` (metres);
/// * `constant_offset`: the slab shape editor's constant-offset answer (`true` when the element
///   has no shape-edit step, which is always so for a slope arrow or defining edge).
pub fn host_volume(
    area: f64,
    solid_volume: Option<f64>,
    constant_thickness: Option<f64>,
    constant_offset: bool,
) -> Result<f64, QuantityError> {
    if GLOBAL_FLAG_CLEAR && !constant_offset {
        if let Some(t) = constant_thickness {
            if (t * FEET_PER_METRE).abs() <= MIN_QUANTITY_FT {
                return Err(QuantityError::NotAvailable);
            }
            return Ok(area * t);
        }
    }
    solid_volume.ok_or(QuantityError::NotAvailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pass 1 sums the top faces, pass 2 the bottom faces, the Area is the larger.
    #[test]
    fn the_area_is_the_larger_of_the_top_and_bottom_sums() {
        assert_eq!(host_area(&[10.0, 2.0], &[11.0]), Ok(12.0));
        assert_eq!(host_area(&[3.0], &[2.5, 1.0]), Ok(3.5));
        assert_eq!(host_area(&[4.0], &[4.0]), Ok(4.0));
    }

    /// At or below 1e-9 (square feet) there is no area.
    #[test]
    fn a_vanishing_area_is_error_7() {
        let at_limit_m2 = 1e-9 / (FEET_PER_METRE * FEET_PER_METRE);
        assert_eq!(host_area(&[at_limit_m2], &[]), Err(QuantityError::NotAvailable));
        assert!(host_area(&[at_limit_m2 * 1.0001], &[]).is_ok());
        assert_eq!(host_area(&[], &[]), Err(QuantityError::NotAvailable));
    }

    /// The Volume is the solid volume in every case except constant thickness AND NOT constant offset.
    #[test]
    fn the_volume_is_the_solid_volume_unless_the_offset_varies() {
        // a sloped ceiling with a slope arrow: no shape-edit step, constant offset TRUE
        assert_eq!(host_volume(20.0, Some(5.5), Some(0.25), true), Ok(5.5));
        // no constant thickness: the solid volume whatever the offsets
        assert_eq!(host_volume(20.0, Some(5.5), None, false), Ok(5.5));
        // constant thickness and non-constant offset: Area x thickness, solid volume ignored
        assert_eq!(host_volume(20.0, Some(5.5), Some(0.25), false), Ok(5.0));
        assert_eq!(host_volume(20.0, None, Some(0.25), false), Ok(5.0));
    }

    /// A thickness of 1e-9 or less in absolute value, and a null solid, are error 7.
    #[test]
    fn a_missing_solid_or_a_vanishing_thickness_is_error_7() {
        assert_eq!(host_volume(20.0, None, Some(0.25), true), Err(QuantityError::NotAvailable));
        assert_eq!(host_volume(20.0, Some(5.5), Some(0.0), false), Err(QuantityError::NotAvailable));
        assert_eq!(host_volume(20.0, Some(5.5), Some(-1e-10), false), Err(QuantityError::NotAvailable));
        assert_eq!(host_volume(20.0, Some(5.5), Some(-0.25), false), Ok(-5.0));
    }
}
