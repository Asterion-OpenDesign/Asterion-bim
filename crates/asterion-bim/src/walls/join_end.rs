//! This module is a written specification, one function per row, with no measurement behind it:
//!
//! | row | what | here |
//! |---|---|---|
//! | | the join angle value and its flags, the integer-to-name map of join types | [`join_angle_flags`], [`WallJoinType`] |
//! | Spec W-05a | which target the end of the first or second wall gets | [`first_or_second_wall_target`] |
//! | | the third and later walls of a join | [`later_wall_target`] and friends |
//! | Spec W-07c | the routing priority of a layer function | [`routing_priority`] |
//!
//! It is deliberately pure: no model, no geometry.
//!
//! # What the engine does with it
//!
//! `crate::joins` classifies a corner as collinear with [`corner_flags`]. The forced-miter flag is exposed by the same
//! function, but the engine does NOT act on it: see [`FORCED_MITER_LIMIT_LINES_DEG`].

use asterion_geom::Vec2;

/// ("collinear") limit on `|value|`, the value being the signed sine of the angle between the two oriented tangents:
/// `0.0017453292519943279`, 0.1 degrees. The same limit decides "the tangents are parallel within 0.1 degrees" for.
pub const COLLINEAR_SINE_LIMIT: f64 = 0.0017453292519943279;

/// The first limit the join builders pass, degrees.
///
/// So two straight walls whose axes meet at less than 20 degrees from collinear (or from folded
/// back) get join type 1 (miter) whatever type is stored, and the support check is skipped.
///
/// UNKNOWN: the engine does NOT act on this flag. A miter changes the solid's end planes, and the engine's reported `Area`,
/// `Volume` and surface-area conventions were fitted on butt joins at 45 and 135 degrees - which are outside this band - so
/// the miter numbers cannot be produced without inventing the convention. The engine keeps its butt formula in the band;
/// that is the known divergence recorded for this rule.
pub const FORCED_MITER_LIMIT_LINES_DEG: f64 = 20.0;

/// The second limit (degrees) used when at least one path curve is not a line. Not used by the engine, which models
/// straight walls and circular arcs and has no curvature rule of this rule.
pub const FORCED_MITER_LIMIT_CURVES_DEG: f64 = 30.0;

/// 0 is the default and the type for which the abut-face operation runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallJoinType {
    /// 0: the end is bounded by a side face of the neighbour.
    Butt,
    /// 1: the end plane passes through the intersections of the offset side curves.
    Miter,
    /// 2: a plane square to the wall through the offset-curve intersections.
    SquareOff,
}

impl WallJoinType {
    /// The type for a stored integer, `None` for any other value ("Unimplemented wall join type").
    pub fn from_code(code: i32) -> Option<WallJoinType> {
        match code {
            0 => Some(WallJoinType::Butt),
            1 => Some(WallJoinType::Miter),
            2 => Some(WallJoinType::SquareOff),
            _ => None,
        }
    }

    pub fn code(self) -> i32 {
        match self {
            WallJoinType::Butt => 0,
            WallJoinType::Miter => 1,
            WallJoinType::SquareOff => 2,
        }
    }
}

/// A single-slanted or tapered wall supports only type 0, every other wall supports every type.
pub fn driver_supports_join_type(single_slanted_or_tapered: bool, join_type_code: i32) -> bool {
    !single_slanted_or_tapered || join_type_code == 0
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JoinAngleFlags {
    /// "angle": NOT an angle, the z component of the cross product of the two oriented tangents
    /// (the signed sine).
    pub value: f64,
    /// Collinear: `|value| <= 0.0017453292519943279`. The caller reports the neighbour as the joined element and builds no
    /// target.
    pub collinear: bool,
    /// Nearly straight: forces join type 1 (miter) and skips the support check.
    pub nearly_straight: bool,
    pub flag_c: bool,
    /// = NOT.
    pub flag_d: bool,
}

/// For two walls whose path curves are lines.
///
/// The first is negated when, the second when NOT.
///
/// `limit_low_deg` is the first limit the join builders pass (20.0, [`FORCED_MITER_LIMIT_LINES_DEG`]).
///
/// `collinear` and `nearly_straight` depend on `|value|` only and are independent of that convention; the sign is what
/// feeds the side choice of [`first_or_second_wall_target`].
pub fn join_angle_flags(
    tangent_a: Vec2,
    end_a: usize,
    tangent_b: Vec2,
    end_b: usize,
    limit_low_deg: f64,
) -> JoinAngleFlags {
    let flag_c = end_a == 0;
    let flag_d = end_b == 1;
    let ta = if flag_c { Vec2::new(-tangent_a.x, -tangent_a.y) } else { tangent_a };
    let tb = if flag_d { Vec2::new(-tangent_b.x, -tangent_b.y) } else { tangent_b };
    let value = ta.cross(tb);
    let collinear = value.abs() <= COLLINEAR_SINE_LIMIT;
    // "parallel within 0.1 degrees" is the same test; for two lines the other clause is the limit.
    let nearly_straight = collinear || value.abs() <= limit_low_deg.to_radians().sin();
    JoinAngleFlags { value, collinear, nearly_straight, flag_c, flag_d }
}

/// The two flags of [`join_angle_flags`] that do not depend on the orientation convention, for two
/// unit directions that both point from the shared corner INTO their wall's body (the engine's own
/// convention). `|cross|` of two unit vectors is the same number as `|value|` whichever way each
/// tangent is oriented, so this is exact, not an approximation.
///
/// Returns `(collinear, nearly_straight)`.
pub fn corner_flags(dir_into_a: Vec2, dir_into_b: Vec2) -> (bool, bool) {
    let v = dir_into_a.cross(dir_into_b).abs();
    (v <= COLLINEAR_SINE_LIMIT, v <= FORCED_MITER_LIMIT_LINES_DEG.to_radians().sin())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndTarget {
    /// Type 0: a SIDE face of the neighbour, wrapped as the target. `side_before_flip` is the side index the rule states;
    /// UNKNOWN: the helper that may flip it to the opposite side was not specified.
    AbutFace { side_before_flip: u8 },
    /// Type 1: the plane through the intersections of the offset side curves.
    MiterPlane,
    /// Type 2: a plane square to the wall, through the offset-curve intersections.
    SquareOffPlane,
    /// The neighbour is reported as the joined element, the call returns 0 and no target is built (7 when no output id is
    /// requested).
    NeighbourIsJoinedElement,
    /// Err 5: no target (a side attach with a type the side attach does not allow).
    NoTarget,
    /// Err 1: failed.
    AngleError,
    /// Err: the join type is not supported by BOTH wall drivers.
    UnsupportedJoinType,
    /// Err 1 (debug "Unimplemented wall join type"): the stored value is not 0, 1 or 2.
    UnimplementedJoinType,
}

/// How the other wall is attached at the join.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OtherAttach {
    /// At an end of the other wall (end index 0 or 1), with the result; `None` is an error.
    AtEnd { angle: Option<JoinAngleFlags> },
    /// At a side index (end index 2 or 3) of the other wall.
    AtSide { end_index: u8 },
}

/// * `wall_index`: 0 for the first wall of the join, 1 for the second;
/// * `stored_type_code`: the join type read from the FIRST wall at the first wall's end index, used
///   for both walls of the pair;
/// * `supports`: of (first wall, second wall) for the type tested.
///
/// 1. for the second wall a type value 2 is replaced by 0;
/// 2. a side attach: for the second wall type 1, and for the first wall any non-zero type, answer
///    Err 5 (no target);
/// 5. type 0 gives the abut face with side index for an end attach, or `end_index - 2` for a side attach; type 1 the miter
///    plane; type 2 the square-off plane; any other value is "Unimplemented wall join type".
pub fn first_or_second_wall_target(
    wall_index: usize,
    stored_type_code: i32,
    other: OtherAttach,
    supports: impl Fn(i32) -> bool,
) -> EndTarget {
    let mut code = stored_type_code;
    if wall_index == 1 && code == 2 {
        code = 0;
    }
    let mut forced_miter = false;
    let side: u8;
    match other {
        OtherAttach::AtSide { end_index } => {
            let refused = (wall_index == 1 && code == 1) || (wall_index == 0 && code != 0);
            if refused {
                return EndTarget::NoTarget;
            }
            side = end_index.saturating_sub(2);
        }
        OtherAttach::AtEnd { angle } => {
            let Some(a) = angle else {
                return EndTarget::AngleError;
            };
            if a.collinear {
                return EndTarget::NeighbourIsJoinedElement;
            }
            if a.nearly_straight {
                forced_miter = true;
                code = 1;
            }
            let positive = a.value > 0.0;
            side = u8::from(a.flag_d ^ positive ^ (wall_index == 1));
        }
    }
    if !forced_miter && !supports(code) {
        return EndTarget::UnsupportedJoinType;
    }
    match WallJoinType::from_code(code) {
        Some(WallJoinType::Butt) => EndTarget::AbutFace { side_before_flip: side },
        Some(WallJoinType::Miter) => EndTarget::MiterPlane,
        Some(WallJoinType::SquareOff) => EndTarget::SquareOffPlane,
        None => EndTarget::UnimplementedJoinType,
    }
}

/// A neighbour counts as parallel / collinear to the wall (and is then NOT used as a target) when the angle between the
/// tangents is within 5 degrees of 0 or 180, for the current step version. UNKNOWN: the rule's legacy clause "(10 degrees
/// and exactly 180 for step version 0)" is for steps loaded from old files; this engine creates only current steps.
pub const LATER_WALL_PARALLEL_LIMIT_DEG: f64 = 5.0;

/// Is the neighbour with tangent angle `angle_rad` (0..=pi, between the tangents of the wall and the neighbour) parallel /
/// collinear to the wall?
pub fn later_wall_neighbour_is_parallel(angle_rad: f64) -> bool {
    let limit = LATER_WALL_PARALLEL_LIMIT_DEG.to_radians();
    angle_rad <= limit || (std::f64::consts::PI - angle_rad) <= limit
}

/// The closed curve loop of the two neighbour offset curves is extruded through the wall Z span with a margin of `0.05 x
/// height` above and below, a total extrusion length of `1.1 x height`. Returns `(margin, total_length)`.
pub fn later_wall_loop_extrusion(height: f64) -> (f64, f64) {
    (0.05 * height, 1.1 * height)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaterWallTarget {
    /// Exactly one neighbour contributes: its side face (side picked by the geometry and the sign
    /// of the join angle).
    NeighbourSideFace,
    /// Two contribute: a closed loop of the two offset curves bounded at their intersection,
    /// extruded ([`later_wall_loop_extrusion`]) into a shell.
    OffsetLoopShell,
    /// Both neighbours are the same wall attached at the same side index: delegated to
    /// [`first_or_second_wall_target`] for the second wall with a two-entry list.
    DelegatedToSecondWall,
    /// Err 5: the wall index is below 2, or the wall / target geometry is null.
    Refused,
    /// Err 1: a neighbour is not a wall with a centre curve and a curve driver.
    InvalidNeighbour,
    /// Err 7: none contributes and both neighbours are the same element.
    SameElementNoTarget,
    /// None contributes and the neighbours differ: the rule does not say. UNKNOWN (the branch structure after the neighbour
    /// selection).
    Undecided,
}

/// Decision for the third and later walls.
///
/// * `contributing`: how many of the two neighbours are not parallel / collinear
///   ([`later_wall_neighbour_is_parallel`]) and so are used;
/// * `same_wall_same_side`: both neighbours are the same wall attached at the same side index;
/// * `same_element`: both neighbours are the same element.
pub fn later_wall_target(
    wall_index: usize,
    neighbours_valid: bool,
    same_wall_same_side: bool,
    contributing: usize,
    same_element: bool,
) -> LaterWallTarget {
    if wall_index < 2 {
        return LaterWallTarget::Refused;
    }
    if !neighbours_valid {
        return LaterWallTarget::InvalidNeighbour;
    }
    if same_wall_same_side {
        return LaterWallTarget::DelegatedToSecondWall;
    }
    match contributing {
        1 => LaterWallTarget::NeighbourSideFace,
        2 => LaterWallTarget::OffsetLoopShell,
        _ if same_element => LaterWallTarget::SameElementNoTarget,
        _ => LaterWallTarget::Undecided,
    }
}

/// The routing priority of a layer (Spec W-07c), and whether the function code was one the operation does not know (a debug
/// warning in the reference product).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoutingPriority {
    pub value: u8,
    pub unknown_code: bool,
}

/// Spec W-07c: layer function 100 gives 0; function 200 is treated as 1; functions 1, 2, 3, 4, 5 give 5, 4, 3, 2, 1; 0
/// gives 0; any other value is a debug warning and 0. The code is the reference product's INTERNAL layer-function integer;
/// UNKNOWN (Spec W-07c): which API function name each of 1..5 is, so this engine's [`crate::LayerFunction`] is not mapped
/// here.
pub fn routing_priority(function_code: i32) -> RoutingPriority {
    let known = |value: u8| RoutingPriority { value, unknown_code: false };
    match function_code {
        100 => known(0),
        // "function 200 treated as 1", and 1 gives 5
        200 | 1 => known(5),
        2 => known(4),
        3 => known(3),
        4 => known(2),
        5 => known(1),
        0 => known(0),
        _ => RoutingPriority { value: 0, unknown_code: true },
    }
}

/// Spec W-07c: only a layer with width `>= 1e-9` (internal units, feet) takes part in the matching and gets a routing
/// priority.
pub const LAYER_MIN_WIDTH_FT: f64 = 1e-9;

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f64, y: f64) -> Vec2 {
        Vec2::new(x, y)
    }

    /// The numeric-to-name map of the join type, and the refusal of every other value.
    #[test]
    fn spec_w05b2_join_type_integers_are_butt_miter_square_off() {
        assert_eq!(WallJoinType::from_code(0), Some(WallJoinType::Butt));
        assert_eq!(WallJoinType::from_code(1), Some(WallJoinType::Miter));
        assert_eq!(WallJoinType::from_code(2), Some(WallJoinType::SquareOff));
        assert_eq!(WallJoinType::from_code(3), None);
        assert_eq!(WallJoinType::from_code(-1), None);
        for t in [WallJoinType::Butt, WallJoinType::Miter, WallJoinType::SquareOff] {
            assert_eq!(WallJoinType::from_code(t.code()), Some(t));
        }
    }

    /// A slanted or tapered wall supports only type 0.
    #[test]
    fn spec_w05b1_a_slanted_or_tapered_wall_supports_only_butt() {
        assert!(driver_supports_join_type(true, 0));
        assert!(!driver_supports_join_type(true, 1));
        assert!(!driver_supports_join_type(true, 2));
        for t in 0..3 {
            assert!(driver_supports_join_type(false, t));
        }
    }

    /// The constants are exactly the ones the rule quotes.
    #[test]
    fn spec_w05b2_the_join_angle_flags_for_two_lines() {
        assert_eq!(COLLINEAR_SINE_LIMIT, 0.0017453292519943279);
        // perpendicular walls: neither flag, |value| = 1
        let f = join_angle_flags(v(1.0, 0.0), 1, v(0.0, 1.0), 0, 20.0);
        assert!((f.value.abs() - 1.0).abs() < 1e-12);
        assert!(!f.collinear && !f.nearly_straight);
        assert!(!f.flag_c && !f.flag_d);
        let g = join_angle_flags(v(1.0, 0.0), 0, v(0.0, 1.0), 1, 20.0);
        assert!(g.flag_c && g.flag_d);
        // 19 degrees from straight: forced miter; 21: not
        let at = |deg: f64| {
            let r = deg.to_radians();
            join_angle_flags(v(1.0, 0.0), 1, v(r.cos(), r.sin()), 0, 20.0)
        };
        assert!(at(19.0).nearly_straight && !at(19.0).collinear);
        assert!(!at(21.0).nearly_straight);
        // 0.09 degrees: collinear; 0.2: not
        assert!(at(0.09).collinear && at(0.09).nearly_straight);
        assert!(!at(0.2).collinear && at(0.2).nearly_straight);
        // the flags depend on |value| only: folded back (180 - 19) is the same as 19
        assert!(at(180.0 - 19.0).nearly_straight);
    }

    /// Via the convention-free helper the engine uses.
    #[test]
    fn spec_w05b2_corner_flags_agree_with_the_oriented_form() {
        for deg in [0.05, 0.5, 10.0, 19.9, 20.1, 45.0, 90.0, 135.0, 179.0, 179.95] {
            let r = f64::to_radians(deg);
            let (a, b) = (v(1.0, 0.0), v(r.cos(), r.sin()));
            let (c, n) = corner_flags(a, b);
            let f = join_angle_flags(a, 1, b, 0, 20.0);
            assert_eq!((c, n), (f.collinear, f.nearly_straight), "{deg}");
        }
    }

    /// Call-site rules: demotion of type 2 for the second wall, side attach refusals.
    #[test]
    fn spec_w05b1_demotion_and_side_attach_refusals() {
        let all = |_t: i32| true;
        let end = |value: f64| OtherAttach::AtEnd {
            angle: Some(JoinAngleFlags { value, collinear: false, nearly_straight: false, flag_c: false, flag_d: false }),
        };
        // type 2: the first wall squares off, the second is demoted to butt
        assert_eq!(first_or_second_wall_target(0, 2, end(1.0), all), EndTarget::SquareOffPlane);
        assert!(matches!(first_or_second_wall_target(1, 2, end(1.0), all), EndTarget::AbutFace { .. }));
        // type 1 miters both
        assert_eq!(first_or_second_wall_target(0, 1, end(1.0), all), EndTarget::MiterPlane);
        assert_eq!(first_or_second_wall_target(1, 1, end(1.0), all), EndTarget::MiterPlane);
        // side attach: the first wall refuses any non-zero type, the second refuses type 1 only
        let side = OtherAttach::AtSide { end_index: 3 };
        assert_eq!(first_or_second_wall_target(0, 1, side, all), EndTarget::NoTarget);
        assert_eq!(first_or_second_wall_target(0, 2, side, all), EndTarget::NoTarget);
        assert_eq!(first_or_second_wall_target(1, 1, side, all), EndTarget::NoTarget);
        assert_eq!(first_or_second_wall_target(0, 0, side, all), EndTarget::AbutFace { side_before_flip: 1 });
        // the second wall's type 2 is demoted to 0 BEFORE the side-attach test, so it is accepted
        assert_eq!(first_or_second_wall_target(1, 2, side, all), EndTarget::AbutFace { side_before_flip: 1 });
        assert_eq!(
            first_or_second_wall_target(0, 0, OtherAttach::AtSide { end_index: 2 }, all),
            EndTarget::AbutFace { side_before_flip: 0 }
        );
        // an unknown stored value
        assert_eq!(first_or_second_wall_target(0, 7, end(1.0), |t| t == 0), EndTarget::UnsupportedJoinType);
        assert_eq!(first_or_second_wall_target(0, 7, end(1.0), all), EndTarget::UnimplementedJoinType);
    }

    #[test]
    fn spec_w05b1_flags_decide_before_the_type() {
        let flags = |collinear, nearly_straight| OtherAttach::AtEnd {
            angle: Some(JoinAngleFlags { value: 0.2, collinear, nearly_straight, flag_c: false, flag_d: false }),
        };
        let only_butt = |t: i32| t == 0;
        assert_eq!(first_or_second_wall_target(0, 0, flags(true, true), only_butt), EndTarget::NeighbourIsJoinedElement);
        // nearly straight: type 1 although the stored type is 0 and the drivers support only 0
        assert_eq!(first_or_second_wall_target(0, 0, flags(false, true), only_butt), EndTarget::MiterPlane);
        assert_eq!(first_or_second_wall_target(1, 2, flags(false, true), only_butt), EndTarget::MiterPlane);
        // otherwise the support check applies
        assert_eq!(first_or_second_wall_target(0, 1, flags(false, false), only_butt), EndTarget::UnsupportedJoinType);
        assert_eq!(
            first_or_second_wall_target(0, 0, OtherAttach::AtEnd { angle: None }, only_butt),
            EndTarget::AngleError
        );
    }

    #[test]
    fn spec_w05b1_the_abut_side_is_a_three_way_xor() {
        for flag_d in [false, true] {
            for positive in [false, true] {
                for wall_index in [0usize, 1] {
                    let angle = JoinAngleFlags {
                        value: if positive { 0.5 } else { -0.5 },
                        collinear: false,
                        nearly_straight: false,
                        flag_c: false,
                        flag_d,
                    };
                    let got = first_or_second_wall_target(wall_index, 0, OtherAttach::AtEnd { angle: Some(angle) }, |_| true);
                    let want = u8::from(flag_d ^ positive ^ (wall_index == 1));
                    assert_eq!(got, EndTarget::AbutFace { side_before_flip: want });
                }
            }
        }
    }

    /// The 5 degree discard, the 1.1 x height loop extrusion and the outcome table.
    #[test]
    fn spec_w06_later_wall_rules() {
        let deg = f64::to_radians;
        assert!(later_wall_neighbour_is_parallel(deg(4.9)));
        assert!(later_wall_neighbour_is_parallel(deg(175.1)));
        assert!(later_wall_neighbour_is_parallel(0.0));
        assert!(later_wall_neighbour_is_parallel(std::f64::consts::PI));
        assert!(!later_wall_neighbour_is_parallel(deg(5.1)));
        assert!(!later_wall_neighbour_is_parallel(deg(90.0)));
        assert!(!later_wall_neighbour_is_parallel(deg(174.9)));

        let (margin, total) = later_wall_loop_extrusion(3.0);
        assert!((margin - 0.15).abs() < 1e-12 && (total - 3.3).abs() < 1e-12);
        assert!((total - (3.0 + 2.0 * margin)).abs() < 1e-12, "1.1 x height = height + 2 x 0.05 x height");

        assert_eq!(later_wall_target(1, true, false, 2, false), LaterWallTarget::Refused);
        assert_eq!(later_wall_target(2, false, false, 2, false), LaterWallTarget::InvalidNeighbour);
        assert_eq!(later_wall_target(2, true, true, 0, true), LaterWallTarget::DelegatedToSecondWall);
        assert_eq!(later_wall_target(2, true, false, 1, false), LaterWallTarget::NeighbourSideFace);
        assert_eq!(later_wall_target(3, true, false, 2, false), LaterWallTarget::OffsetLoopShell);
        assert_eq!(later_wall_target(2, true, false, 0, true), LaterWallTarget::SameElementNoTarget);
        assert_eq!(later_wall_target(2, true, false, 0, false), LaterWallTarget::Undecided);
    }

    /// Spec W-07c: function 1..5 give 5..1, 0 and 100 give 0, 200 is treated as 1, anything else is a warning and 0.
    #[test]
    fn spec_w07c_routing_priority_by_layer_function() {
        let p = |c| routing_priority(c);
        assert_eq!((1..=5).map(|c| p(c).value).collect::<Vec<_>>(), vec![5, 4, 3, 2, 1]);
        assert_eq!(p(0).value, 0);
        assert_eq!(p(100).value, 0);
        assert_eq!(p(200).value, p(1).value, "function 200 is treated as 1");
        for c in [6, 7, 50, -1, 101, 201] {
            assert_eq!(p(c), RoutingPriority { value: 0, unknown_code: true }, "{c}");
        }
        assert!(!p(100).unknown_code && !p(0).unknown_code && !p(200).unknown_code);
        assert_eq!(LAYER_MIN_WIDTH_FT, 1e-9);
    }
}
