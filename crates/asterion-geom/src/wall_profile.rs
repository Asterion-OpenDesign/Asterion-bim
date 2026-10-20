//! The profile loop and extrusion of a STRAIGHT VERTICAL wall, in the order the reference builds it.
//!
//! Source: the specification rule (written specification only, its straight-vertical branch and the default wall elevation
//! loop):
//!
//! * the wall solid of a straight vertical wall is produced by extruding ONE closed loop of four
//!   curves, appended in this order: (1) the BOTTOM curve, the curve of the wall curve driver
//!   translated to the base height (tag 2); (2) a straight line from the end of the bottom curve to
//!   the end of the top curve; (3) the TOP curve, a copy of the bottom curve translated by the wall
//!   height and REVERSED (tag 3); (4) the closing straight line. So the FIRST curve of the loop is
//!   the wall path curve at base height;
//! * The loop is moved by `-width / 2` along the extrusion direction (the offset is `-width` for a curtain-driver wall with
//!   step version 1..3 and 0 above version 3).
//!
//! What this module does not own (documented UNKNOWN, never filled with a guess):
//!
//! * slanted and tapered walls (swept solid from a cross-section profile: not specified) and a wall with an elevation
//!   sketch (the generic extrusion builder).
//! * the end-trim of the path curve by the built-in end-parameter doubles (it changes the curve only
//!   when the element carries them; the join extension of ASTERION is applied by the caller on the
//!   path before it reaches this module).

use crate::vec::{Vec2, Vec3};

/// Which of the four loop curves a [`ProfileCurve`] is, in the order the reference appends them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileRole {
    /// (1) the wall path curve translated to the base height (curve tag 2).
    BasePath,
    /// (2) a straight line from the end of the base curve to the end of the top curve.
    EndLine,
    /// (3) the base curve translated by the wall height and reversed (curve tag 3).
    TopPath,
    /// (4) the closing straight line, from the start of the (reversed) top curve to the start of the
    /// base curve.
    ClosingLine,
}

/// One straight curve of the profile loop: its role, its end points and the curve tag the reference
/// stores on it (only the path curves carry one).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfileCurve {
    pub role: ProfileRole,
    pub from: Vec3,
    pub to: Vec3,
    pub tag: Option<u8>,
}

/// Why a profile cannot be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallProfileError {
    /// The path curve has no direction (start equals end).
    NoDirection,
    /// A non-finite coordinate, height or width.
    NotFinite,
}

impl std::fmt::Display for WallProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            WallProfileError::NoDirection => "the wall path curve has no direction",
            WallProfileError::NotFinite => "a non-finite wall profile input",
        })
    }
}

impl std::error::Error for WallProfileError {}

/// The profile loop and the extrusion of a straight vertical wall.
#[derive(Clone, Debug, PartialEq)]
pub struct StraightWallProfile {
    /// The four loop curves in the order the reference appends them (see [`ProfileRole`]), already
    /// at the base height and BEFORE the `offset` move.
    pub curves: [ProfileCurve; 4],
    /// Unit extrusion direction `t x Z = (t.y, -t.x, 0)`.
    pub direction: Vec3,
    /// The move of the loop along `direction` before the extrusion: `-width / 2`.
    pub offset: f64,
    /// The extrusion depth: the total width.
    pub depth: f64,
}

impl StraightWallProfile {
    /// Build the loop of a straight wall whose path curve runs `start -> end` in plan at
    /// `base_z`, `height` tall and `width` thick (METRES).
    pub fn new(start: Vec2, end: Vec2, base_z: f64, height: f64, width: f64) -> Result<Self, WallProfileError> {
        let vals = [start.x, start.y, end.x, end.y, base_z, height, width];
        if vals.iter().any(|v| !v.is_finite()) {
            return Err(WallProfileError::NotFinite);
        }
        let t = (end - start).normalized().ok_or(WallProfileError::NoDirection)?;
        let a = Vec3::new(start.x, start.y, base_z);
        let b = Vec3::new(end.x, end.y, base_z);
        let a_top = Vec3::new(start.x, start.y, base_z + height);
        let b_top = Vec3::new(end.x, end.y, base_z + height);
        let curves = [
            // (1) the path curve at base height
            ProfileCurve { role: ProfileRole::BasePath, from: a, to: b, tag: Some(2) },
            // (2) end of the bottom curve -> end of the top curve
            ProfileCurve { role: ProfileRole::EndLine, from: b, to: b_top, tag: None },
            // (3) the top curve: the base curve translated by the height and reversed
            ProfileCurve { role: ProfileRole::TopPath, from: b_top, to: a_top, tag: Some(3) },
            // (4) the closing line back to the start of the bottom curve
            ProfileCurve { role: ProfileRole::ClosingLine, from: a_top, to: a, tag: None },
        ];
        Ok(StraightWallProfile {
            curves,
            direction: Vec3::new(t.y, -t.x, 0.0),
            offset: -width / 2.0,
            depth: width,
        })
    }

    /// The loop after the `-width / 2` move along the extrusion direction (the START face of the
    /// extrusion), same order and roles.
    pub fn moved_curves(&self) -> [ProfileCurve; 4] {
        let d = Vec3::new(self.direction.x * self.offset, self.direction.y * self.offset, 0.0);
        let mv = |p: Vec3| Vec3::new(p.x + d.x, p.y + d.y, p.z + d.z);
        let mut out = self.curves;
        for c in out.iter_mut() {
            c.from = mv(c.from);
            c.to = mv(c.to);
        }
        out
    }

    /// The plane frame the extrusion builds for its first face: origin = start point of the FIRST curve of the loop (the
    /// moved base path), `u` = its start tangent with the component along the extrusion direction removed, `v = direction x
    /// u`. Returns `(origin, u, v)`.
    pub fn first_face_frame(&self) -> (Vec3, Vec3, Vec3) {
        let first = self.moved_curves()[0];
        let t = Vec3::new(first.to.x - first.from.x, first.to.y - first.from.y, first.to.z - first.from.z);
        let len = (t.x * t.x + t.y * t.y + t.z * t.z).sqrt();
        let t = Vec3::new(t.x / len, t.y / len, t.z / len);
        let d = self.direction;
        let along = t.x * d.x + t.y * d.y + t.z * d.z;
        let u = Vec3::new(t.x - along * d.x, t.y - along * d.y, t.z - along * d.z);
        let ul = (u.x * u.x + u.y * u.y + u.z * u.z).sqrt();
        let u = Vec3::new(u.x / ul, u.y / ul, u.z / ul);
        let v = Vec3::new(
            d.y * u.z - d.z * u.y,
            d.z * u.x - d.x * u.z,
            d.x * u.y - d.y * u.x,
        );
        (Vec3::new(first.from.x, first.from.y, first.from.z), u, v)
    }

    /// The four plan corners of the extruded body in the order the loop and the extrusion produce
    /// them: the start and end of the moved base curve, then the end and start of its extruded copy
    /// (`offset + depth` along the direction).
    pub fn plan_corners(&self) -> [Vec2; 4] {
        let c = self.moved_curves()[0];
        let far = Vec2::new(self.direction.x * self.depth, self.direction.y * self.depth);
        let (a, b) = (Vec2::new(c.from.x, c.from.y), Vec2::new(c.to.x, c.to.y));
        [a, b, b + far, a + far]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> StraightWallProfile {
        StraightWallProfile::new(Vec2::new(0.0, 0.0), Vec2::new(5.0, 0.0), 1.0, 3.0, 0.2).unwrap()
    }

    /// The four curves, in the read order, with the read tags and end points.
    #[test]
    fn the_loop_has_the_four_curves_in_the_read_order() {
        let l = p();
        let roles: Vec<ProfileRole> = l.curves.iter().map(|c| c.role).collect();
        assert_eq!(
            roles,
            vec![ProfileRole::BasePath, ProfileRole::EndLine, ProfileRole::TopPath, ProfileRole::ClosingLine]
        );
        // (1) base path at base height, tag 2
        assert_eq!((l.curves[0].from, l.curves[0].to, l.curves[0].tag), (Vec3::new(0.0, 0.0, 1.0), Vec3::new(5.0, 0.0, 1.0), Some(2)));
        // (2) end of the bottom curve to the end of the top curve
        assert_eq!((l.curves[1].from, l.curves[1].to), (Vec3::new(5.0, 0.0, 1.0), Vec3::new(5.0, 0.0, 4.0)));
        // (3) the top copy, translated by the height and reversed, tag 3
        assert_eq!((l.curves[2].from, l.curves[2].to, l.curves[2].tag), (Vec3::new(5.0, 0.0, 4.0), Vec3::new(0.0, 0.0, 4.0), Some(3)));
        // (4) the closing line returns to the start of the bottom curve
        assert_eq!((l.curves[3].from, l.curves[3].to), (Vec3::new(0.0, 0.0, 4.0), Vec3::new(0.0, 0.0, 1.0)));
        // closed: each curve starts where the previous one ends
        for i in 0..4 {
            assert_eq!(l.curves[i].to, l.curves[(i + 1) % 4].from);
        }
    }

    /// The direction is `t x Z = (t.y, -t.x, 0)`, the loop is moved by `-w/2` and extruded by `w`.
    #[test]
    fn the_extrusion_is_t_cross_z_moved_by_half_the_width_and_extruded_by_the_width() {
        let l = p();
        assert_eq!(l.direction, Vec3::new(0.0, -1.0, 0.0));
        assert_eq!((l.offset, l.depth), (-0.1, 0.2));
        let m = l.moved_curves();
        // -w/2 along (0,-1,0) is +0.1 in y: the LEFT face of a wall drawn along +x
        assert_eq!(m[0].from, Vec3::new(0.0, 0.1, 1.0));
        assert_eq!(m[0].to, Vec3::new(5.0, 0.1, 1.0));
        let c = l.plan_corners();
        assert_eq!(c[0], Vec2::new(0.0, 0.1));
        assert_eq!(c[1], Vec2::new(5.0, 0.1));
        assert_eq!(c[2], Vec2::new(5.0, -0.1));
        assert_eq!(c[3], Vec2::new(0.0, -0.1));
    }

    /// : the first-face frame has its origin at the start of the first (path) curve, `u` along the wall and `v = direction
    /// x u` vertical.
    #[test]
    fn the_first_face_frame_starts_at_the_path_curve() {
        let (o, u, v) = p().first_face_frame();
        assert_eq!(o, Vec3::new(0.0, 0.1, 1.0));
        assert_eq!(u, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(v, Vec3::new(0.0, 0.0, 1.0));
    }

    /// The footprint is the same set of corners for a wall drawn the other way: the solid is
    /// symmetric about the path curve, only the direction of travel changes.
    #[test]
    fn a_reversed_path_gives_the_mirrored_start_face() {
        let l = StraightWallProfile::new(Vec2::new(5.0, 0.0), Vec2::new(0.0, 0.0), 1.0, 3.0, 0.2).unwrap();
        assert_eq!(l.direction, Vec3::new(0.0, 1.0, 0.0));
        let c = l.plan_corners();
        assert_eq!(c[0], Vec2::new(5.0, -0.1));
        assert_eq!(c[2], Vec2::new(0.0, 0.1));
    }

    /// A path with no direction is an error, not a NaN frame.
    #[test]
    fn degenerate_input_is_an_error() {
        assert_eq!(
            StraightWallProfile::new(Vec2::new(1.0, 1.0), Vec2::new(1.0, 1.0), 0.0, 3.0, 0.2),
            Err(WallProfileError::NoDirection)
        );
        assert_eq!(
            StraightWallProfile::new(Vec2::new(0.0, 0.0), Vec2::new(f64::NAN, 1.0), 0.0, 3.0, 0.2),
            Err(WallProfileError::NotFinite)
        );
    }
}
