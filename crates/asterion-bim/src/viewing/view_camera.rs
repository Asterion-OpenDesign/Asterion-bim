//! The camera of a 3D view: its orientation and projection.
//!
//! Rows followed:
//! * A default 3D view is oriented to the eye vector (-1, 1, -1). Whether that vector is the viewing direction or points at
//!   the eye is not specified by the rule.
//! * (over the viewer's bounded space): the eye is the box origin, the up vector the second basis vector, the forward
//!   vector the third, the right vector the first; the projection is perspective exactly when the viewer's projection
//!   method is 2 (1 is orthographic).
//!
//! NOT specified (named questions): the EYE POSITION a new view gets (the stored origin of the observed default view is 7.7
//! ft from the model centre along the view line, its rule is), so a new view's `eye` is `None` and the viewer frames the
//! model; how chooses the up vector: the engine projects the world Z axis perpendicular to the direction, which reproduces
//! the one observed record to 1e-4; the section box, crop, scale and detail level of the new view.

use serde::{Deserialize, Serialize};

/// The eye vector orients to (not normalised).
pub const DEFAULT_EYE_VECTOR: [f64; 3] = [-1.0, 1.0, -1.0];

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(a: [f64; 3]) -> Option<[f64; 3]> {
    let l = dot(a, a).sqrt();
    (l > 1e-12 && l.is_finite()).then(|| [a[0] / l, a[1] / l, a[2] / l])
}

/// The orientation and projection of a 3D view, in the model's coordinates (metres for the eye).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewCamera {
    /// The eye (the origin of the viewer's box); `None` = not determined, the viewer frames the model.
    pub eye: Option<[f64; 3]>,
    /// The first basis vector.
    pub right: [f64; 3],
    /// The second basis vector.
    pub up: [f64; 3],
    /// The third basis vector: the direction the camera looks along.
    pub forward: [f64; 3],
    pub perspective: bool,
}

impl ViewCamera {
    /// A camera looking along `direction`, with the world Z axis projected perpendicular to it as the up vector (ASSUMED)
    /// and the right vector `forward x up` (the observed record's third vector). `None` for a zero direction or a vertical
    /// one (no horizontal reference: not an isometric view).
    pub fn looking_along(direction: [f64; 3]) -> Option<ViewCamera> {
        let forward = unit(direction)?;
        let z = [0.0, 0.0, 1.0];
        let k = dot(z, forward);
        let up = unit([z[0] - k * forward[0], z[1] - k * forward[1], z[2] - k * forward[2]])?;
        let right = cross(forward, up);
        Some(ViewCamera { eye: None, right, up, forward, perspective: false })
    }

    /// The orientation of a default (isometric) 3D view, orthographic.
    pub fn default_isometric() -> ViewCamera {
        ViewCamera::looking_along(DEFAULT_EYE_VECTOR).expect("the default eye vector is not vertical")
    }

    /// The camera of a viewer given by its box: `origin` the box origin (the eye), `basis` its three vectors (right, up,
    /// forward), `perspective` the projection. Lengths in metres.
    pub fn from_basis(origin: [f64; 3], basis: [[f64; 3]; 3], perspective: bool) -> ViewCamera {
        ViewCamera { eye: Some(origin), right: basis[0], up: basis[1], forward: basis[2], perspective }
    }

    /// Whether the three vectors are unit length and perpendicular (a stored camera should be; the engine does not repair one).
    pub fn is_orthonormal(&self) -> bool {
        let (r, u, f) = (self.right, self.up, self.forward);
        let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
        near(dot(r, r), 1.0) && near(dot(u, u), 1.0) && near(dot(f, f), 1.0) && dot(r, u).abs() < 1e-6 && dot(r, f).abs() < 1e-6 && dot(u, f).abs() < 1e-6
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near3(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < tol)
    }

    /// (OBSERVED, the default {3D} view): the default camera looks along (-1, 1, -1), its up vector is the world Z
    /// projected perpendicular to it and its right vector is `forward x up`, orthographic.
    #[test]
    fn spec_v3d01_the_default_isometric_camera_matches_the_stored_one() {
        let c = ViewCamera::default_isometric();
        assert!(near3(c.forward, [-0.5774, 0.5774, -0.5774], 1e-4), "{:?}", c.forward);
        assert!(near3(c.up, [-0.4082, 0.4082, 0.8165], 1e-4), "{:?}", c.up);
        assert!(near3(c.right, [0.7071, 0.7071, 0.0], 1e-4), "{:?}", c.right);
        assert!(c.is_orthonormal() && !c.perspective && c.eye.is_none());
    }

    /// Eye = box origin, right / up / forward = basis 0 / 1 / 2, perspective only for projection method 2.
    #[test]
    fn spec_a_stored_viewer_gives_eye_basis_and_projection() {
        let ft = 0.3048;
        let basis = [[0.7071067811865476, 0.7071067811865476, 0.0], [-0.4082482904638631, 0.4082482904638631, 0.816496580927726], [-0.5773502691896258, 0.5773502691896258, -0.5773502691896258]];
        let c = ViewCamera::from_basis([4.4594 * ft, -4.4443 * ft, 4.4443 * ft], basis, false);
        assert_eq!(c.eye, Some([4.4594 * ft, -4.4443 * ft, 4.4443 * ft]));
        assert!(!c.perspective && c.is_orthonormal());
        let d = ViewCamera::default_isometric();
        assert!(near3(c.right, d.right, 1e-9) && near3(c.up, d.up, 1e-9) && near3(c.forward, d.forward, 1e-9), "the engine default reproduces the stored basis");
        assert!(ViewCamera::from_basis([0.0; 3], basis, true).perspective);
        assert!(!ViewCamera::from_basis([0.0; 3], basis, false).perspective);
    }

    /// Both creators orient the new view to the default eye vector; a plan has no camera; a stored camera is imported into
    /// a 3D view only, and only when its basis is orthonormal.
    #[test]
    fn spec_v3d01_the_creators_orient_the_new_view_and_a_stored_camera_is_imported_into_a_3d_view_only() {
        use crate::model::ViewKind;
        let mut d = crate::new_document();
        let (a, b, plan) = d
            .transact("v", false, |m, cx| {
                let a = crate::view_names::create_default_3d_view(m, cx)?;
                let b = crate::view_names::create_isometric_3d_view(m, cx)?;
                let l = crate::ops::create_level(m, cx, "L0", 0.0)?;
                let plan = crate::ops::create_view(m, cx, "P", ViewKind::FloorPlan, Some(l), None)?;
                Ok((a, b, plan))
            })
            .value
            .unwrap();
        assert_eq!(d.model().views[&a].camera, Some(ViewCamera::default_isometric()));
        assert_eq!(d.model().views[&b].camera, Some(ViewCamera::default_isometric()));
        assert_eq!(d.model().views[&plan].camera, None);
        let basis = [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]];
        let stored = ViewCamera::from_basis([1.0, 2.0, 3.0], basis, true);
        let s2 = stored.clone();
        assert!(d.transact("cam", false, |m, _| crate::import_ops::import_view_camera(m, a, s2)).committed());
        assert_eq!(d.model().views[&a].camera, Some(stored.clone()));
        assert!(d.transact("plan", false, |m, _| crate::import_ops::import_view_camera(m, plan, stored.clone())).value.is_none());
        let skewed = ViewCamera::from_basis([0.0; 3], [[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 0.0, 1.0]], false);
        assert!(d.transact("skew", false, |m, _| crate::import_ops::import_view_camera(m, b, skewed)).value.is_none());
        assert_eq!(d.model().views[&b].camera, Some(ViewCamera::default_isometric()), "the refused camera changed nothing");
    }

    #[test]
    fn a_vertical_or_zero_direction_has_no_horizontal_reference() {
        assert!(ViewCamera::looking_along([0.0, 0.0, -1.0]).is_none() && ViewCamera::looking_along([0.0; 3]).is_none());
        let c = ViewCamera::looking_along([0.0, 1.0, 0.0]).unwrap();
        assert!(near3(c.up, [0.0, 0.0, 1.0], 1e-12) && near3(c.right, [1.0, 0.0, 0.0], 1e-12), "{:?}", c);
    }
}
