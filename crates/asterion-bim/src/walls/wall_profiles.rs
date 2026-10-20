//! Wall profile sketches (`Wall.CreateProfileSketch` / `RemoveProfileSketch`, API of the reference product).
//!
//! * a profile is one outer loop and optional inner loops (sketched holes) in the wall's elevation plane, `u` along the
//!   location line from its start and `z` above the wall's base; the wall's body is that region extruded through the
//!   wall's width: Area = region area (22.5 for a gable with its ridge at 4.5), Volume = Area x width, the counts of a
//!   prism of the region (a pentagon: 7 faces / 15 edges / 10 vertices; a rectangle with a hole: 10 / 24 / 16);
//! * the sketch created without edits is the rectangle of the wall and changes nothing;
//! * the vertices on the TOP edge of the wall, and the ones above it, ride on the top: raising the unconnected height from
//!   3 to 5 lifts the gable's eaves to 5 and its ridge to 6.5 (Area 34.5); the same through a top constraint (L1 3.0 ->
//!   3.5: Area 22.5 -> 25.5); a vertex below the top keeps its height (which vertices are anchored to the top is fitted on
//!   these walls).
//! * a profile wider than the location line moves the line's ends to the profile's (-1 .. 7: Length 8); a narrower one
//!   does not shrink it;
//! * an inner loop that touches the outer loop (it would split the wall) discards the whole edit, silently (the commit
//!   succeeds and the previous profile stays);
//! * the box of the wall keeps the band of its location line and runs from the base to the higher of the top and the
//!   profile;
//! * an arc wall cannot have a profile ("Wall does not support profile Sketch as it is not a straight wall").

use crate::model::BimModel;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use asterion_geom::Vec2;
use serde::{Deserialize, Serialize};

pub mod codes {
    pub const NOT_STRAIGHT: &str = "wall_profile.not_straight";
    pub const BAD_LOOP: &str = "wall_profile.bad_loop";
    pub const ALREADY_HAS_SKETCH: &str = "wall_profile.already_has_sketch";
}

/// One vertex of a profile loop: `u` along the location line from its start, `z` above the base or, when `on_top`,
/// above the top edge of the wall.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileVertex {
    pub u: f64,
    pub z: f64,
    pub on_top: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WallProfile {
    /// The outer loop first, then the inner loops.
    pub loops: Vec<Vec<ProfileVertex>>,
}

impl WallProfile {
    /// The loops in `(u, z above the base)` for a wall of height `height`.
    pub fn resolved(&self, height: f64) -> Vec<Vec<Vec2>> {
        self.loops.iter().map(|l| l.iter().map(|v| Vec2::new(v.u, if v.on_top { height + v.z } else { v.z })).collect()).collect()
    }
}

fn inside(poly: &[Vec2], p: Vec2, margin: f64) -> bool {
    if !asterion_geom::region::polygon_contains_point(poly, p) {
        return false;
    }
    // strictly inside: farther than `margin` from every edge
    let n = poly.len();
    (0..n).all(|i| {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        let ab = b - a;
        let len2 = ab.dot(ab);
        let t = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
        (p - (a + ab * t)).length() > margin
    })
}

/// Create or edit the profile sketch of a straight wall. `loops = None` creates the default rectangle (no change to the
/// body). Returns whether the edit was applied (`false`: an inner loop touched the outer one, the edit was discarded as the
/// reference product does).
pub fn edit_wall_profile(m: &mut BimModel, _cx: &mut TxnContext<'_>, wall: ElementId, loops: Option<Vec<Vec<Vec2>>>) -> Result<bool, Failure> {
    let w = m.wall(wall)?.clone();
    if w.through.is_some() {
        return Err(Failure::error(
            codes::NOT_STRAIGHT,
            "Wall does not support profile Sketch as it is not a straight wall; or is tapered; or it is an old curtain wall; or it is an infill wall; or it is a replacement curtain panel.",
        ));
    }
    let st = crate::regen::wall_state(m, &w)?;
    let (length, height) = (st.length, st.height);
    let loops = match loops {
        Some(l) => l,
        None => {
            if m.wall_profiles.contains_key(&wall) {
                return Err(Failure::error(codes::ALREADY_HAS_SKETCH, "Wall already has a sketch."));
            }
            {
                let rect = vec![Vec2::new(0.0, 0.0), Vec2::new(length, 0.0), Vec2::new(length, height), Vec2::new(0.0, height)];
                let v = rect.iter().map(|p| ProfileVertex { u: p.x, z: if p.y >= height - 1e-9 { p.y - height } else { p.y }, on_top: p.y >= height - 1e-9 }).collect();
                m.wall_profiles.insert(wall, WallProfile { loops: vec![v] });
            }
            return Ok(true);
        }
    };
    if loops.is_empty() || loops.iter().any(|l| l.len() < 3) || loops.iter().flatten().any(|p| !(p.x.is_finite() && p.y.is_finite())) {
        return Err(Failure::error(codes::BAD_LOOP, "A profile loop needs at least 3 points."));
    }
    // an inner loop that is not strictly inside the outer one would split the wall: the edit is discarded
    let outer = &loops[0];
    if loops[1..].iter().any(|l| l.iter().any(|p| !inside(outer, *p, 1e-6))) {
        return Ok(false);
    }
    // a profile wider than the location line moves its ends
    let u_min = loops.iter().flatten().map(|p| p.x).fold(f64::INFINITY, f64::min);
    let u_max = loops.iter().flatten().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
    let shift = if u_min < -1e-9 { -u_min } else { 0.0 };
    let dir = (w.end - w.start).normalized().ok_or_else(|| Failure::error(codes::BAD_LOOP, "The wall has no direction."))?;
    {
        let wm = m.wall_mut(wall)?;
        if shift > 0.0 {
            wm.start = wm.start - dir * shift;
        }
        if u_max > length + 1e-9 {
            wm.end = wm.end + dir * (u_max - length);
        }
    }
    let v: Vec<Vec<ProfileVertex>> = loops
        .iter()
        .map(|l| {
            l.iter()
                .map(|p| {
                    let on_top = p.y >= height - 1e-9;
                    ProfileVertex { u: p.x + shift, z: if on_top { p.y - height } else { p.y }, on_top }
                })
                .collect()
        })
        .collect();
    m.wall_profiles.insert(wall, WallProfile { loops: v });
    Ok(true)
}

/// Remove the profile sketch: the wall is the rectangle of its height and its CURRENT location line again.
pub fn remove_wall_profile(m: &mut BimModel, _cx: &mut TxnContext<'_>, wall: ElementId) -> Result<(), Failure> {
    m.wall(wall)?;
    m.wall_profiles.remove(&wall);
    Ok(())
}

/// The body of a wall with a profile sketch: the region of the loops (holes subtracted) less the cuts of its inserts,
/// extruded through the width (see the module docs). Nothing changes for a wall without a profile.
pub fn apply(m: &BimModel, w: &crate::model::Wall, s: &mut crate::regen::WallState) {
    use asterion_geom::boolean::{boolean, BoolOp};
    use asterion_geom::region::{Polycurve, Region};
    let Some(p) = m.wall_profiles.get(&w.id) else { return };
    let tol = asterion_geom::Tolerances::default();
    let loops = p.resolved(s.height);
    let Some(first) = loops.first() else { return };
    let Ok(outer) = Polycurve::polygon(first, &tol) else { return };
    let holes: Vec<Polycurve> = loops[1..].iter().filter_map(|l| Polycurve::polygon(l, &tol).ok()).collect();
    let Ok(region) = Region::new(outer, holes, &tol) else { return };
    let mut regions = vec![region];
    for (_, _, r) in crate::regen::host_cuts(m, w.id) {
        let (z0, z1) = (r.z_lo - s.base_z, r.z_hi - s.base_z);
        let pts = [Vec2::new(r.u_lo, z0), Vec2::new(r.u_hi, z0), Vec2::new(r.u_hi, z1), Vec2::new(r.u_lo, z1)];
        let Ok(rect) = Region::from_polygon(&pts, &tol) else { continue };
        regions = regions.iter().flat_map(|g| boolean(g, &rect, BoolOp::Difference, &tol).unwrap_or_else(|_| vec![g.clone()])).collect();
    }
    let area: f64 = regions.iter().map(Region::area).sum();
    let perimeter: f64 = regions.iter().map(Region::total_boundary_length).sum();
    let n: usize = regions.iter().map(Region::curve_count).sum();
    let gross: f64 = asterion_geom::region::polygon_signed_area(first).abs()
        - loops[1..].iter().map(|l| asterion_geom::region::polygon_signed_area(l).abs()).sum::<f64>();
    s.gross_area = gross;
    s.cut_area = (gross - area).max(0.0);
    s.area = area;
    s.volume = area * s.width;
    s.surface_area = 2.0 * area + perimeter * s.width;
    s.solid_face_count = asterion_geom::region::distinct_support_count_all(&regions, &tol) + 2;
    s.solid_edge_count = 3 * n;
    s.solid_vertex_count = 2 * n;
    // the box runs from the base to the higher of the top and the profile
    let z_top = loops.iter().flatten().map(|q| q.y).fold(f64::NEG_INFINITY, f64::max) + s.base_z;
    if let Some(u) = s.unjoined_solid.as_mut() {
        if z_top > u.z1 {
            u.z1 = z_top;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ops, CompoundStructure, Layer, LayerFunction};

    fn pts(v: &[(f64, f64)]) -> Vec<Vec2> {
        v.iter().map(|(x, y)| Vec2::new(*x, *y)).collect()
    }

    /// Round 39, a 6 x 3 wall of 0.2: the gable to 4.5 (22.5 / 4.5, 7 / 15 / 10), its top riding on the height (5 m: 34.5),
    /// a sketched hole (16.0, 10 / 24 / 16), a profile wider than the line (Length 8), an edit that would split the wall
    /// discarded, an arc wall refused.
    #[test]
    fn the_profile_reproduces_round_39() {
        let mut d = crate::new_document();
        let (l, w) = d
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let t = ops::create_wall_type(m, cx, "T", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
                let w = ops::create_wall(m, cx, ops::NewWall { wall_type: t, level: l, start: Vec2::new(0.0, 0.0), end: Vec2::new(6.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false })?;
                Ok((l, w))
            })
            .value
            .unwrap();
        let st = |d: &crate::BimDocument| crate::regen::wall_state(d.model(), d.model().wall(w).unwrap()).unwrap();
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        d.transact("p0", false, |m, cx| edit_wall_profile(m, cx, w, None)).value.unwrap();
        assert!(near(st(&d).area, 18.0));
        d.transact("p1", false, |m, cx| edit_wall_profile(m, cx, w, Some(vec![pts(&[(0.0, 0.0), (6.0, 0.0), (6.0, 3.0), (3.0, 4.5), (0.0, 3.0)])]))).value.unwrap();
        let s = st(&d);
        assert!(near(s.area, 22.5) && near(s.volume, 4.5), "{s:?}");
        assert_eq!((s.solid_face_count, s.solid_edge_count, s.solid_vertex_count), (7, 15, 10));
        d.transact("h", false, |m, cx| ops::set_unconnected_height(m, cx, w, 5.0)).value.unwrap();
        assert!(near(st(&d).area, 34.5));
        d.transact("p2", false, |m, cx| edit_wall_profile(m, cx, w, Some(vec![pts(&[(0.0, 0.0), (6.0, 0.0), (6.0, 3.0), (0.0, 3.0)]), pts(&[(2.0, 1.0), (4.0, 1.0), (4.0, 2.0), (2.0, 2.0)])]))).value.unwrap();
        let s = st(&d);
        assert!(near(s.area, 16.0) && near(s.surface_area, 36.8), "{s:?}");
        assert_eq!((s.solid_face_count, s.solid_edge_count, s.solid_vertex_count), (10, 24, 16));
        // an inner loop across the whole wall: the edit is discarded
        let applied = d.transact("split", false, |m, cx| edit_wall_profile(m, cx, w, Some(vec![pts(&[(0.0, 0.0), (6.0, 0.0), (6.0, 3.0), (0.0, 3.0)]), pts(&[(0.0, 1.0), (6.0, 1.0), (6.0, 2.0), (0.0, 2.0)])]))).value.unwrap();
        assert!(!applied && near(st(&d).area, 16.0));
        d.transact("wide", false, |m, cx| edit_wall_profile(m, cx, w, Some(vec![pts(&[(-1.0, 0.0), (7.0, 0.0), (7.0, 3.0), (-1.0, 3.0)])]))).value.unwrap();
        assert!(near(st(&d).length, 8.0) && near(st(&d).area, 24.0));
        d.transact("rm", false, |m, cx| remove_wall_profile(m, cx, w)).value.unwrap();
        assert!(near(st(&d).area, 40.0));
        let arc = d
            .transact("arc", false, |m, cx| {
                let t = *m.wall_types.keys().next().unwrap();
                let a = ops::create_wall(m, cx, ops::NewWall { wall_type: t, level: l, start: Vec2::new(20.0, 0.0), end: Vec2::new(26.0, 4.0), height: 3.0, base_offset: 0.0, flip: false, structural: false })?;
                m.wall_mut(a)?.through = Some(Vec2::new(23.0, 0.5));
                Ok(a)
            })
            .value
            .unwrap();
        assert!(d.transact("ap", false, |m, cx| edit_wall_profile(m, cx, arc, None)).value.is_none());
    }
}
