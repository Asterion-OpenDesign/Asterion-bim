//! Corners with a curved (arc) wall: the forced and the stored MITRE.
//!
//! The end face of a mitred wall is the VERTICAL plane through the two points where the path curves, each offset by minus
//! and plus half of the OWNER wall's own width, meet near the control point; with two lines these points lie on the
//! bisector (`crate::joins::CornerKind::Miter`), with an arc they do not, so the corners are carried explicitly
//! ([`MiterCorners`]).
//!
//! The same corner closes the rooms of round 32: the line ends on the segment (6.0198, -0.1) - (5.9798, 0.1), the arc wall
//! starts on the same one, both with 6 faces, 10 edges and 8 vertices, the line's Area is its larger side face (6.0198 x
//! 3).
//!
//! ENGINE CHOICE: which offset curve of the owner meets which of the other is taken as left with left and right with right
//! of the travel through the corner (the pairing that makes two lines meet on the bisector).

use crate::model::Wall;
use crate::regen::{arc_from_points, Arc2};
use asterion_geom::Vec2;

/// Limit: 0.1 degrees as a sine.
const COLLINEAR_SINE: f64 = 0.0017453292519943279;

/// The path curve of a wall.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Path {
    Line { a: Vec2, b: Vec2 },
    Arc(Arc2),
}

impl Path {
    pub(crate) fn of(w: &Wall) -> Option<Path> {
        match w.through {
            None => Some(Path::Line { a: w.start, b: w.end }),
            Some(t) => arc_from_points(w.start, w.end, t).map(Path::Arc),
        }
    }

    pub(crate) fn is_line(&self) -> bool {
        matches!(self, Path::Line { .. })
    }

    /// The unit tangent at `end` (0 start, 1 finish) pointing OUT of the wall.
    pub(crate) fn outward(&self, end: usize) -> Option<Vec2> {
        let travel = match self {
            Path::Line { a, b } => (*b - *a).normalized()?,
            Path::Arc(arc) => {
                let ang = if end == 0 { arc.start_angle } else { arc.start_angle + arc.sweep };
                let radial = Vec2::new(ang.cos(), ang.sin());
                if arc.sweep >= 0.0 {
                    radial.perp_ccw()
                } else {
                    -radial.perp_ccw()
                }
            }
        };
        Some(if end == 1 { travel } else { -travel })
    }

    fn curvature(&self) -> f64 {
        match self {
            Path::Line { .. } => 0.0,
            Path::Arc(a) => 1.0 / a.radius,
        }
    }

    /// The path offset by `d` to the LEFT of its travel direction (start -> finish); `d < 0` is the right.
    pub(crate) fn offset(&self, d: f64) -> Curve {
        match self {
            Path::Line { a, b } => {
                let dir = (*b - *a).normalized().unwrap_or(Vec2::new(1.0, 0.0));
                Curve::Line { p: *a + dir.perp_ccw() * d, dir }
            }
            Path::Arc(arc) => {
                // the left of a counter-clockwise arc is toward its centre
                let r = if arc.sweep >= 0.0 { arc.radius - d } else { arc.radius + d };
                Curve::Circle { c: arc.center, r }
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Curve {
    Line { p: Vec2, dir: Vec2 },
    Circle { c: Vec2, r: f64 },
}

/// The intersections of two offset curves.
pub(crate) fn intersect(a: &Curve, b: &Curve) -> Vec<Vec2> {
    match (a, b) {
        (Curve::Line { p: p1, dir: d1 }, Curve::Line { p: p2, dir: d2 }) => {
            let den = d1.cross(*d2);
            if den.abs() < 1e-12 {
                return Vec::new();
            }
            let t = (*p2 - *p1).cross(*d2) / den;
            vec![*p1 + *d1 * t]
        }
        (Curve::Line { p, dir }, Curve::Circle { c, r }) | (Curve::Circle { c, r }, Curve::Line { p, dir }) => {
            // |p + t dir - c| = r
            let f = *p - *c;
            let (b2, c2) = (f.dot(*dir), f.dot(f) - r * r);
            let disc = b2 * b2 - c2;
            if disc < 0.0 {
                return Vec::new();
            }
            let s = disc.sqrt();
            vec![*p + *dir * (-b2 + s), *p + *dir * (-b2 - s)]
        }
        (Curve::Circle { c: c1, r: r1 }, Curve::Circle { c: c2, r: r2 }) => {
            let d = (*c2 - *c1).length();
            if d < 1e-12 || d > r1 + r2 + 1e-12 || d < (r1 - r2).abs() - 1e-12 {
                return Vec::new();
            }
            let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
            let h = (r1 * r1 - a * a).max(0.0).sqrt();
            let ex = (*c2 - *c1) * (1.0 / d);
            let mid = *c1 + ex * a;
            vec![mid + ex.perp_ccw() * h, mid - ex.perp_ccw() * h]
        }
    }
}

/// The join value and the two flags of the corner at `ea` of A / `eb` of B (`wa`, `wb` the walls' widths).
#[derive(Clone, Copy, Debug)]
pub(crate) struct JoinFlags {
    pub value: f64,
    pub collinear: bool,
    pub nearly_straight: bool,
}

pub(crate) fn join_flags(a: &Path, ea: usize, wa: f64, b: &Path, eb: usize, wb: f64) -> Option<JoinFlags> {
    // the first tangent is negated at a start (so it points out of A), the second at a finish (so it points into B)
    let ta = a.outward(ea)?;
    let tb = -b.outward(eb)?;
    let value = ta.cross(tb);
    let both_lines = a.is_line() && b.is_line();
    let collinear = value.abs() <= COLLINEAR_SINE && !(ta.dot(tb) < 0.1 && !both_lines);
    let straight_band = |limit_deg: f64| value.abs() <= limit_deg.to_radians().sin();
    let nearly_straight = if value.abs() <= COLLINEAR_SINE {
        true
    } else if both_lines {
        straight_band(crate::join_end::FORCED_MITER_LIMIT_LINES_DEG)
    } else {
        let limit = 30.0_f64.to_radians().sin();
        // t = sqrt(2 (1 - (1 - k w_other / 2) / (1 - k w_own / 2))), 0 when the denominator is nil or the radicand negative
        let term = |k: f64, w_other: f64, w_own: f64| {
            let den = 1.0 - k * w_own / 2.0;
            if den <= 1e-9 {
                return 0.0;
            }
            let rad = 2.0 * (1.0 - (1.0 - k * w_other / 2.0) / den);
            if rad < 0.0 {
                0.0
            } else {
                rad.sqrt()
            }
        };
        let (ka, kb) = (a.curvature(), b.curvature());
        value.abs() <= limit && value.abs() + term(ka, wb, wa) <= limit && value.abs() + term(kb, wa, wb) <= limit
    };
    Some(JoinFlags { value, collinear, nearly_straight })
}

/// The two end corners of a mitred end (plan points), the one on the LEFT of the owner's start -> finish travel first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MiterCorners {
    pub left: Vec2,
    pub right: Vec2,
}

/// The mitre corners of the owner's end `e_owner` against `other` met at its end `e_other`, `p` the control point,
/// `h_owner` / `h_other` the half widths of the two walls: each path curve is offset by minus and plus half of ITS OWN
/// width (OBSERVED e: a 0.2 line meeting a 0.3 arc at 22.6 degrees, the corners (5.9, 3.8454) and (6.1, 4.1464) are the
/// line's sides met with the arc's circles of radius 3.25 -+ 0.15; for equal widths the pairing is the same).
pub(crate) fn mitre_corners(owner: &Path, e_owner: usize, h_owner: f64, other: &Path, e_other: usize, h_other: f64, p: Vec2) -> Option<MiterCorners> {
    // sides in the sense of travel through the corner: along the owner toward p, then on along the other away from p
    let own_sign = if e_owner == 1 { 1.0 } else { -1.0 };
    let other_sign = if e_other == 0 { 1.0 } else { -1.0 };
    let pick = |side: f64| -> Option<Vec2> {
        let a = owner.offset(own_sign * side * h_owner);
        let b = other.offset(other_sign * side * h_other);
        intersect(&a, &b).into_iter().min_by(|x, y| (*x - p).length().total_cmp(&(*y - p).length()))
    };
    let through_left = pick(1.0)?;
    let through_right = pick(-1.0)?;
    // through-left of the owner is the left of start -> finish at a finish, the right at a start
    Some(if e_owner == 1 {
        MiterCorners { left: through_left, right: through_right }
    } else {
        MiterCorners { left: through_right, right: through_left }
    })
}

/// The body of an arc wall whose ends may be mitred: the band between the two concentric arcs, closed at each end by a
/// radial line or, for a mitred end, by the segment between its two corners. Closed forms (the Stokes sums of
/// `asterion_geom::stokes` describe radial ends only).
#[derive(Clone, Debug)]
pub(crate) struct SlantedBand {
    /// Plan area of the band.
    pub area: f64,
    /// Areas of the outer and the inner cylindrical face (height `h`).
    pub outer_face: f64,
    pub inner_face: f64,
    /// Lengths of the two end segments.
    pub caps: [f64; 2],
    pub lo: Vec2,
    pub hi: Vec2,
    /// The outline, outer arc then inner arc back (for a view or the viewport).
    pub outline: Vec<Vec2>,
    /// The absolute angle ranges of the outer and the inner arc, from the start end to the finish end (for the Stokes sums).
    pub outer: (f64, f64),
    pub inner: (f64, f64),
}

fn angle_of(c: Vec2, p: Vec2) -> f64 {
    (p.y - c.y).atan2(p.x - c.x)
}

/// `end_corners[e]` is the mitre of end `e` (as returned by [`mitre_corners`] for the arc wall), `None` for a radial end.
pub(crate) fn slanted_band(arc: &Arc2, r_in: f64, r_out: f64, end_corners: [Option<MiterCorners>; 2], h: f64) -> SlantedBand {
    use std::f64::consts::{PI, TAU};
    let c = arc.center;
    let dir = if arc.sweep >= 0.0 { 1.0 } else { -1.0 };
    // the point of end `e` on the circle of radius `r`
    let corner = |e: usize, r: f64| -> Vec2 {
        if let Some(m) = end_corners[e] {
            for p in [m.left, m.right] {
                if ((p - c).length() - r).abs() < 1e-6 {
                    return p;
                }
            }
        }
        let ang = if e == 0 { arc.start_angle } else { arc.start_angle + arc.sweep };
        c + Vec2::new(ang.cos(), ang.sin()) * r
    };
    let (o_s, o_e, i_s, i_e) = (corner(0, r_out), corner(1, r_out), corner(0, r_in), corner(1, r_in));
    // signed angles along the sweep
    let span = |a: Vec2, b: Vec2| dir * (dir * (angle_of(c, b) - angle_of(c, a))).rem_euclid(TAU);
    let (d_out, d_in) = (span(o_s, o_e), span(i_s, i_e));
    let rel = |p: Vec2| p - c;
    let signed = 0.5 * r_out * r_out * d_out + 0.5 * rel(o_e).cross(rel(i_e)) - 0.5 * r_in * r_in * d_in + 0.5 * rel(i_s).cross(rel(o_s));
    let area = signed.abs();
    // extent: the end points and the arcs' extreme points inside their angle ranges
    let mut lo = Vec2::new(f64::INFINITY, f64::INFINITY);
    let mut hi = Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut take = |p: Vec2| {
        lo = Vec2::new(lo.x.min(p.x), lo.y.min(p.y));
        hi = Vec2::new(hi.x.max(p.x), hi.y.max(p.y));
    };
    for p in [o_s, o_e, i_s, i_e] {
        take(p);
    }
    for (r, from, d) in [(r_out, o_s, d_out), (r_in, i_s, d_in)] {
        let a0 = angle_of(c, from);
        for k in 0..4 {
            let q = k as f64 * PI / 2.0;
            // the angle of this axis direction measured from the start along the sweep
            let along = (dir * (q - a0)).rem_euclid(TAU);
            if along <= d.abs() {
                take(c + Vec2::new(q.cos(), q.sin()) * r);
            }
        }
    }
    let sample = |r: f64, from: Vec2, d: f64| -> Vec<Vec2> {
        let n = ((d.abs() / TAU) * 96.0).ceil().max(6.0) as usize;
        let a0 = angle_of(c, from);
        (0..=n).map(|i| c + Vec2::new((a0 + d * i as f64 / n as f64).cos(), (a0 + d * i as f64 / n as f64).sin()) * r).collect()
    };
    let mut outline = sample(r_out, o_s, d_out);
    outline.extend(sample(r_in, i_s, d_in).into_iter().rev());
    // absolute angles: the arc's own start angle plus the (small) offset of each end point from it
    let wrap = |a: f64| (a + PI).rem_euclid(TAU) - PI;
    let a0 = arc.start_angle;
    let a1 = arc.start_angle + arc.sweep;
    let outer = (a0 + wrap(angle_of(c, o_s) - a0), a1 + wrap(angle_of(c, o_e) - a1));
    let inner = (a0 + wrap(angle_of(c, i_s) - a0), a1 + wrap(angle_of(c, i_e) - a1));
    SlantedBand {
        outer,
        inner,
        area,
        outer_face: r_out * d_out.abs() * h,
        inner_face: r_in * d_in.abs() * h,
        caps: [(o_s - i_s).length(), (o_e - i_e).length()],
        lo,
        hi,
        outline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arc_wall_path() -> Path {
        // round 32 a2: (6, 0) through (8, 3) to (6, 6)
        Path::Arc(arc_from_points(Vec2::new(6.0, 0.0), Vec2::new(6.0, 6.0), Vec2::new(8.0, 3.0)).unwrap())
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    /// A 0.2 line meeting a 0.2 arc at 22.6 degrees is forced to a mitre, its corners are those of round 32.
    #[test]
    fn a_line_and_an_arc_meet_on_the_measured_mitre() {
        let line = Path::Line { a: Vec2::new(0.0, 0.0), b: Vec2::new(6.0, 0.0) };
        let arc = arc_wall_path();
        let f = join_flags(&line, 1, 0.2, &arc, 0, 0.2).unwrap();
        assert!(near(f.value, 0.384615384) && !f.collinear && f.nearly_straight, "{f:?}");
        let c = mitre_corners(&line, 1, 0.1, &arc, 0, 0.1, Vec2::new(6.0, 0.0)).unwrap();
        // the line's left is y = +0.1 (inner), its right y = -0.1 (outer)
        assert!(near(c.left.x, 5.979837) && near(c.left.y, 0.1), "{c:?}");
        assert!(near(c.right.x, 6.019842) && near(c.right.y, -0.1), "{c:?}");
        // the arc wall's end 0 uses the same two points (equal widths); left of its travel is the inner side
        let c2 = mitre_corners(&arc, 0, 0.1, &line, 1, 0.1, Vec2::new(6.0, 0.0)).unwrap();
        assert!(near(c2.left.x, 5.979837) && near(c2.right.x, 6.019842), "{c2:?}");
    }

    /// Round 32 a2: the band from the mitre to the radial end: Area (the outer face) exactly, Volume within the arc tolerance.
    #[test]
    fn the_arc_wall_of_the_measured_corner_is_a_slanted_band() {
        let Path::Arc(arc) = arc_wall_path() else { unreachable!() };
        let line = Path::Line { a: Vec2::new(0.0, 0.0), b: Vec2::new(6.0, 0.0) };
        let c = mitre_corners(&Path::Arc(arc), 0, 0.1, &line, 1, 0.1, Vec2::new(6.0, 0.0)).unwrap();
        let b = slanted_band(&arc, 3.15, 3.35, [Some(c), None], 3.0);
        assert!((b.outer_face - 23.698141153249264).abs() < 1e-9, "{}", b.outer_face);
        assert!((b.area * 3.0 - 4.585858073204348).abs() / 4.585858 < 3e-4, "{}", b.area * 3.0);
        assert!((b.lo.x - 5.9615384615).abs() < 1e-6 && (b.lo.y + 0.1).abs() < 1e-9, "{:?}", b.lo);
        assert!((b.hi.x - 8.1).abs() < 1e-9 && (b.hi.y - 6.0923076923).abs() < 1e-6, "{:?}", b.hi);
    }

    /// A thick line (0.3) on a thin arc (0.2): the curvature term keeps the pair out of the forced band.
    #[test]
    fn a_thick_line_on_a_thin_arc_is_not_forced() {
        let line = Path::Line { a: Vec2::new(0.0, 0.0), b: Vec2::new(6.0, 0.0) };
        let f = join_flags(&line, 1, 0.3, &arc_wall_path(), 0, 0.2).unwrap();
        assert!(!f.nearly_straight, "{f:?}");
    }

    /// Round 32 a: three straight walls and an arc bulging out, mitred where they meet (22.6 degrees), close the room - the
    /// bodies share the segment between the two mitre corners.
    #[test]
    fn a_room_closed_by_an_arc_with_mitred_corners_is_enclosed() {
        use crate::model::{CompoundStructure, Layer, LayerFunction};
        use crate::ops;
        let mut d = crate::new_document();
        let out = d.transact("b", false, |m, cx| {
            let level = ops::create_level(m, cx, "L0", 0.0)?;
            let t = ops::create_wall_type(m, cx, "T", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
            let mk = |m: &mut crate::model::BimModel, cx: &mut asterion_core::txn::TxnContext<'_>, a: (f64, f64), b: (f64, f64), through: Option<(f64, f64)>| {
                let spec = ops::NewWall { wall_type: t, level, start: Vec2::new(a.0, a.1), end: Vec2::new(b.0, b.1), height: 3.0, base_offset: 0.0, flip: false, structural: false };
                match through {
                    Some(p) => ops::create_arc_wall(m, cx, spec, Vec2::new(p.0, p.1)),
                    None => ops::create_wall(m, cx, spec),
                }
            };
            mk(m, cx, (0.0, 0.0), (6.0, 0.0), None)?;
            mk(m, cx, (6.0, 0.0), (6.0, 4.0), None)?;
            mk(m, cx, (6.0, 4.0), (0.0, 4.0), Some((3.0, 6.0)))?;
            mk(m, cx, (0.0, 4.0), (0.0, 0.0), None)?;
            Ok(level)
        });
        let level = out.value.unwrap();
        let regions = crate::regen::enclosed_regions(d.model(), level);
        assert_eq!(regions.len(), 1);
        let room = d.transact("room", false, |m, cx| ops::create_room(m, cx, level, Vec2::new(3.0, 2.0))).value.unwrap();
        let m = d.model();
        let st = crate::regen::room_state(m, &m.rooms[&room]).unwrap();
        // the measured room (round 32 093): Area 30.54291250443718, Finish boundary length 20.92880752466245, Room.Perimeter 21.164033846118375
        let finish: f64 = st.finish.iter().map(|s| s.length()).sum();
        assert!((st.area - 30.54291250443718).abs() < 1e-9 && (finish - 20.92880752466245).abs() < 1e-9, "{} {}", st.area, finish);
        assert!((st.perimeter - 21.164033846118375).abs() < 1e-9, "{}", st.perimeter);
        assert_eq!(st.finish.len(), 4);
    }
}
