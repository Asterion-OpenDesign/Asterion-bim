//! Offset bands: a centreline path plus a left and a right offset, closed into
//! a `Region`.
//!
//! This is the shape of a wall body in plan, and the reason the kernel needed
//! arcs: a curved wall's faces are **concentric arcs** of its centreline, not
//! chords. With `offset_band` the body of a curved wall is exact — its plan
//! area is the annular sector `|sweep| * (R_outer^2 - R_inner^2) / 2`, not a
//! polygonal approximation (see
//! `solid::tests::curved_wall_body_volume_matches_the_annular_sector`).
//!
//! Conventions:
//!
//! * `left` and `right` are **distances**, both measured outward from the
//!   centreline: `left` to the left of travel, `right` to the right. A wall of
//!   width `w` centred on its location line is `offset_band(path, w/2, w/2)`.
//!   A location line on the finish face is `offset_band(path, 0.0, w)`.
//! * At a corner where the two offset curves no longer meet, they are extended
//!   or trimmed to the intersection of their **supporting** geometry (a mitre).
//!   If the supports do not meet, a straight bevel curve is inserted instead,
//!   which is reported through `BandJoin` so callers can tell the two apart.
//! * Ends are closed with straight caps (square ends). Other end conditions are
//!   a semantic decision and belong to `asterion-bim`.

use crate::curve::Curve;
use crate::error::{GeomError, GeomResult};
use crate::intersect::intersect_curves_unbounded;
use crate::region::{Polycurve, Region};
use crate::tolerance::Tolerances;
use crate::vec::Vec2;

/// How a corner between two consecutive offset curves was resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandJoin {
    /// Already continuous within the vertex tolerance (tangent-continuous path).
    Continuous,
    /// Both curves were trimmed/extended to the mitre point.
    Mitre,
    /// Supports did not intersect; a straight bevel curve was inserted.
    Bevel,
}

/// A centreline path: consecutive curves whose ends meet within the vertex
/// tolerance. Not closed.
pub fn validate_path(path: &[Curve], tol: &Tolerances) -> GeomResult<()> {
    if path.is_empty() {
        return Err(GeomError::LoopTooFewCurves { count: 0, minimum: 1 });
    }
    for c in path {
        let len = c.length();
        if len < tol.short_curve {
            return Err(GeomError::CurveTooShort { length: len, tolerance: tol.short_curve });
        }
    }
    for w in path.windows(2) {
        let gap = w[0].end().distance_to(w[1].start());
        if gap > tol.vertex {
            return Err(GeomError::LoopNotClosed { gap, tolerance: tol.vertex });
        }
    }
    Ok(())
}

/// Offset one side of a path and join the pieces, returning the chain and how
/// each corner was resolved. `distance` is signed in the curve convention
/// (positive = to the left of travel).
pub fn offset_chain(
    path: &[Curve],
    distance: f64,
    tol: &Tolerances,
) -> GeomResult<(Vec<Curve>, Vec<BandJoin>)> {
    validate_path(path, tol)?;
    let mut chain: Vec<Curve> = Vec::with_capacity(path.len());
    for c in path {
        chain.push(c.offset(distance)?);
    }
    let mut joins: Vec<BandJoin> = Vec::new();
    let mut out: Vec<Curve> = Vec::with_capacity(chain.len());
    for (i, offset_curve) in chain.iter().copied().enumerate() {
        let mut current = offset_curve;
        if i > 0 {
            // `prev` is the previously emitted curve, already trimmed if the
            // last corner was mitred.
            let prev = out.pop().expect("i > 0 means out is non-empty");
            let gap = prev.end().distance_to(current.start());
            if gap <= tol.vertex {
                out.push(prev);
                joins.push(BandJoin::Continuous);
            } else {
                let target = prev.end().lerp(current.start(), 0.5);
                let best = intersect_curves_unbounded(&prev, &current, tol)
                    .into_iter()
                    .min_by(|a, b| a.distance_to(target).total_cmp(&b.distance_to(target)));
                match best {
                    Some(p) => {
                        out.push(trim_end(&prev, p)?);
                        current = trim_start(&current, p)?;
                        joins.push(BandJoin::Mitre);
                    }
                    None => {
                        let bevel = Curve::line(prev.end(), current.start());
                        out.push(prev);
                        out.push(bevel);
                        joins.push(BandJoin::Bevel);
                    }
                }
            }
        }
        out.push(current);
    }
    Ok((out, joins))
}

/// Closed region swept by the path between its left and right offsets, with
/// square end caps.
pub fn offset_band(path: &[Curve], left: f64, right: f64, tol: &Tolerances) -> GeomResult<Region> {
    let (band, _) = offset_band_detailed(path, left, right, tol)?;
    Ok(band)
}

/// As [`offset_band`], also reporting how each side's corners were resolved.
pub fn offset_band_detailed(
    path: &[Curve],
    left: f64,
    right: f64,
    tol: &Tolerances,
) -> GeomResult<(Region, Vec<BandJoin>)> {
    validate_path(path, tol)?;
    if left + right <= 0.0 {
        return Err(GeomError::ZeroWidthBand { left, right });
    }
    let (left_chain, mut joins) = offset_chain(path, left, tol)?;
    let (right_chain, right_joins) = offset_chain(path, -right, tol)?;
    joins.extend(right_joins);

    let mut curves: Vec<Curve> = Vec::with_capacity(left_chain.len() + right_chain.len() + 2);
    curves.extend(left_chain.iter().copied());
    let left_end = left_chain.last().expect("non-empty").end();
    let right_end = right_chain.last().expect("non-empty").end();
    if left_end.distance_to(right_end) > tol.vertex {
        curves.push(Curve::line(left_end, right_end));
    }
    curves.extend(right_chain.iter().rev().map(|c| c.reversed()));
    let right_start = right_chain.first().expect("non-empty").start();
    let left_start = left_chain.first().expect("non-empty").start();
    if right_start.distance_to(left_start) > tol.vertex {
        curves.push(Curve::line(right_start, left_start));
    }
    let lp = Polycurve::closed(curves, tol)?;
    Ok((Region::from_loop(lp), joins))
}

/// Convenience: a band of total width `width` centred on a single curve.
pub fn centred_band(centreline: Curve, width: f64, tol: &Tolerances) -> GeomResult<Region> {
    offset_band(&[centreline], width / 2.0, width / 2.0, tol)
}

fn trim_end(c: &Curve, p: Vec2) -> GeomResult<Curve> {
    Ok(match c {
        Curve::Line(l) => Curve::Line(l.with_end(p)),
        Curve::Arc(a) => Curve::Arc(a.with_end(p)?),
    })
}

fn trim_start(c: &Curve, p: Vec2) -> GeomResult<Curve> {
    Ok(match c {
        Curve::Line(l) => Curve::Line(l.with_start(p)),
        Curve::Arc(a) => Curve::Arc(a.with_start(p)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::Arc;
    use crate::solid::ExtrudedRegion;
    use std::f64::consts::PI;

    fn tol() -> Tolerances {
        Tolerances::default()
    }
    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-12, "{a} != {b}");
    }

    #[test]
    fn straight_band_is_the_rectangle_it_should_be() {
        // Wall centreline 5 m long, width 0.3 m: 1.5 m2, perimeter 10.6 m.
        let path = [Curve::line(Vec2::new(0.0, 0.0), Vec2::new(5.0, 0.0))];
        let r = offset_band(&path, 0.15, 0.15, &tol()).unwrap();
        near(r.area(), 1.5);
        near(r.outer_perimeter(), 2.0 * 5.0 + 2.0 * 0.3);
        assert_eq!(r.curve_count(), 4);
        // The same numbers the v0.1 prism produced for a wall body.
        let s = ExtrudedRegion::new(r, 0.0, 3.0).unwrap();
        near(s.volume(), 1.5 * 3.0);
        assert_eq!((s.vertex_count(), s.face_count(), s.edge_count()), (8, 6, 12));
    }

    #[test]
    fn asymmetric_offsets_place_the_band_off_the_centreline() {
        // Location line on one face: all 0.3 m to the left.
        let path = [Curve::line(Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0))];
        let r = offset_band(&path, 0.3, 0.0, &tol()).unwrap();
        near(r.area(), 1.2);
        let b = r.aabb();
        near(b.min.y, 0.0);
        near(b.max.y, 0.3);
    }

    #[test]
    fn curved_band_area_is_the_annular_sector() {
        // 90-degree curved wall: centreline radius 5 CCW, width 0.3.
        // Left of travel on a CCW arc is the centre, so the inner face is at
        // 4.85 and the outer at 5.15.
        let arc = Arc::from_center_angles(Vec2::new(0.0, 0.0), 5.0, 0.0, PI / 2.0, true).unwrap();
        let r = offset_band(&[Curve::Arc(arc)], 0.15, 0.15, &tol()).unwrap();
        let expected = 0.5 * (PI / 2.0) * (5.15 * 5.15 - 4.85 * 4.85);
        near(r.area(), expected);
        near(r.outer_perimeter(), (PI / 2.0) * (4.85 + 5.15) + 0.6);
        assert_eq!(r.curve_count(), 4);
        // Exactly two of the four boundary curves are arcs.
        assert_eq!(r.outer().arc_count(), 2);
        let b = r.aabb();
        near(b.max.x, 5.15);
        near(b.max.y, 5.15);
    }

    #[test]
    fn curved_band_matches_the_annulus_when_the_centreline_closes() {
        // A full circular wall of radius 5, width 0.4: area = pi * (5.2^2 - 4.8^2).
        let upper = Arc::new(Vec2::new(0.0, 0.0), 5.0, 0.0, PI).unwrap();
        let lower = Arc::new(Vec2::new(0.0, 0.0), 5.0, PI, PI).unwrap();
        let path = [Curve::Arc(upper), Curve::Arc(lower)];
        let r = offset_band(&path, 0.2, 0.2, &tol()).unwrap();
        near(r.area(), PI * (5.2 * 5.2 - 4.8 * 4.8));
    }

    #[test]
    fn right_angle_corner_is_mitred_and_the_area_is_exact() {
        // Centreline (0,0)->(4,0)->(4,3), width 0.4. At a right angle the outer
        // mitre gain and the inner mitre loss cancel, so the area is exactly
        // (4 + 3) * 0.4 = 2.8. Perimeter: inner side 3.8 + 2.8, outer side
        // 4.2 + 3.2, two 0.4 caps => 14.8.
        let path = [
            Curve::line(Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0)),
            Curve::line(Vec2::new(4.0, 0.0), Vec2::new(4.0, 3.0)),
        ];
        let (r, joins) = offset_band_detailed(&path, 0.2, 0.2, &tol()).unwrap();
        assert_eq!(joins, vec![BandJoin::Mitre, BandJoin::Mitre]);
        near(r.area(), 2.8);
        near(r.outer_perimeter(), 3.8 + 2.8 + 4.2 + 3.2 + 0.8);
        assert_eq!(r.curve_count(), 6);
        let b = r.aabb();
        near(b.min.x, 0.0);
        near(b.max.x, 4.2);
        near(b.min.y, -0.2);
        near(b.max.y, 3.0);
    }

    #[test]
    fn tangent_line_to_arc_needs_no_join() {
        // A straight run that continues into a tangent arc: both offsets stay
        // continuous, so no mitre and no bevel is inserted.
        let line = Curve::line(Vec2::new(-3.0, 0.0), Vec2::new(0.0, 0.0));
        let arc = Arc::from_center_angles(Vec2::new(0.0, 2.0), 2.0, -PI / 2.0, 0.0, true).unwrap();
        let path = [line, Curve::Arc(arc)];
        let (r, joins) = offset_band_detailed(&path, 0.1, 0.1, &tol()).unwrap();
        assert_eq!(joins, vec![BandJoin::Continuous, BandJoin::Continuous]);
        // Area = straight part + annular quarter sector.
        let straight = 3.0 * 0.2;
        let sector = 0.5 * (PI / 2.0) * (2.1 * 2.1 - 1.9 * 1.9);
        near(r.area(), straight + sector);
        assert_eq!(r.curve_count(), 6);
    }

    #[test]
    fn a_band_wider_than_an_arc_radius_is_rejected() {
        let arc = Arc::from_center_angles(Vec2::new(0.0, 0.0), 1.0, 0.0, PI / 2.0, true).unwrap();
        let e = offset_band(&[Curve::Arc(arc)], 1.5, 0.5, &tol()).unwrap_err();
        assert_eq!(e.code(), crate::error::codes::OFFSET_COLLAPSES_CURVE);
    }

    #[test]
    fn a_disconnected_path_is_rejected() {
        let path = [
            Curve::line(Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0)),
            Curve::line(Vec2::new(1.5, 0.0), Vec2::new(2.0, 0.0)),
        ];
        let e = offset_band(&path, 0.1, 0.1, &tol()).unwrap_err();
        assert_eq!(e.code(), crate::error::codes::LOOP_NOT_CLOSED);
    }

    #[test]
    fn zero_width_is_rejected() {
        let path = [Curve::line(Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0))];
        assert!(offset_band(&path, 0.0, 0.0, &tol()).is_err());
    }

    #[test]
    fn band_is_bit_reproducible() {
        let arc = Arc::from_center_angles(Vec2::new(0.3, -1.2), 7.5, 0.4, 2.1, true).unwrap();
        let path = [Curve::Arc(arc)];
        let first = offset_band(&path, 0.17, 0.08, &tol()).unwrap();
        for _ in 0..8 {
            let again = offset_band(&path, 0.17, 0.08, &tol()).unwrap();
            assert_eq!(again.area().to_bits(), first.area().to_bits());
            assert_eq!(
                again.total_boundary_length().to_bits(),
                first.total_boundary_length().to_bits()
            );
        }
    }
}
