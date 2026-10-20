//! The room solid trimmed by the room-bounding floors, ceilings and roofs.
//!
//! Spec M-02: a room's Volume is that of its own solid; with room volumes computed the extruded prism "is additionally cut
//! by the 3D room-bounding elements (floors, ceilings, roofs ...)", so Volume is area x height only when nothing trims it.
//! The trimming operations themselves are Spec M-02 not specified; what they produce is OBSERVED:
//!
//! * No effect at 2.6 under a top of 2.4384, at 3.5, at -0.5 or with Room Bounding off; a zero-thickness Basic Ceiling caps
//!   too; two ceilings at 2.0 and at 1.5 over half the room cap it by zones (38.76 = 44.08 - 10.64 x 0.5); the lowest one
//!   wins.
//! * round 37: the floor above caps it at its underside (22.04 x 2.8 = 61.712), and where a shaft or an opening cuts that
//!   floor the room rises to the next bounding element or its top (66.512, 67.712, 63.304).
//!
//! So the height of the room over a plan point is its top, or the underside of the lowest room-bounding slab covering that
//! point between its base and its top. ENGINE CHOICE: sloped floors, ceilings and roofs are left out (none measured), and a
//! slab that covers part of the room trims only that part (the zone rule above).

use crate::model::{BimModel, Room, SlabRole};
use asterion_geom::region::{Polycurve, Region};
use asterion_geom::{Tolerances, Vec2};

fn region_of(outer: &[Vec2], holes: &[Vec<Vec2>], tol: &Tolerances) -> Option<Region> {
    let o = Polycurve::polygon(outer, tol).ok()?;
    let h: Vec<Polycurve> = holes.iter().filter_map(|l| Polycurve::polygon(l, tol).ok()).collect();
    Region::new(o, h, tol).ok()
}

/// `(volume removed, highest height above the base)` for the room of plan region `poly` between `base` and `top`;
/// `None` when no bounding slab trims it (the caller keeps `area x height`).
pub fn trim(model: &BimModel, room: &Room, poly: &[Vec2], base: f64, top: f64) -> Option<(f64, f64)> {
    let tol = Tolerances::default();
    let mut caps: Vec<(f64, Region)> = Vec::new();
    for f in model.floors.values() {
        if !f.room_bounding || !matches!(f.role, SlabRole::Floor | SlabRole::Ceiling | SlabRole::Roof) {
            continue;
        }
        if f.slope_arrow.is_some() || f.segments.iter().any(|s| s.defines_slope) || f.boundary_arcs.iter().any(Option::is_some) {
            continue; // ENGINE CHOICE: sloped or curved slabs are not applied
        }
        let Ok(st) = crate::regen::slab_state(model, f) else { continue };
        let z = st.bottom_z;
        if !(z > base + 1e-9 && z < top - 1e-9) {
            continue;
        }
        let mut holes: Vec<Vec<Vec2>> = f.loops.clone();
        holes.extend(crate::shafts::flat_slab_holes(model, f));
        if let Some(r) = region_of(&f.boundary, &holes, &tol) {
            caps.push((z, r));
        }
    }
    if caps.is_empty() {
        return None;
    }
    let _ = room;
    caps.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut remaining: Vec<Region> = vec![region_of(poly, &[], &tol)?];
    let (mut removed, mut h_max) = (0.0, f64::NEG_INFINITY);
    for (z, cap) in &caps {
        let mut next = Vec::new();
        for r in &remaining {
            let covered: f64 = r.intersection(cap, &tol).map(|v| v.iter().map(Region::area).sum()).unwrap_or(0.0);
            if covered > 1e-12 {
                removed += covered * (top - z);
                h_max = h_max.max(z - base);
            }
            next.extend(r.difference(cap, &tol).unwrap_or_else(|_| vec![r.clone()]));
        }
        remaining = next.into_iter().filter(|r| r.area() > 1e-12).collect();
    }
    if !remaining.is_empty() {
        h_max = top - base;
    }
    (removed > 0.0).then_some((removed, h_max))
}

#[cfg(test)]
mod tests {
    use crate::{ops, CompoundStructure, Layer, LayerFunction};
    use asterion_geom::Vec2;

    /// Round 34b: the 6 x 4 ring of 0.2 walls (room 22.04 m2, top 2.4384) with a 0.2 ceiling at 2.0: Volume 44.08; a second
    /// ceiling at 1.5 over 0.1..2.9 x 0.1..3.9: 38.76; the first one not room bounding: the room goes back to the lower
    /// one's zones only (top 2.4384 elsewhere).
    #[test]
    fn the_lowest_room_bounding_ceiling_caps_the_room_by_zones() {
        let mut d = crate::new_document();
        let rect = |x0: f64, y0: f64, x1: f64, y1: f64| vec![Vec2::new(x0, y0), Vec2::new(x1, y0), Vec2::new(x1, y1), Vec2::new(x0, y1)];
        let (room, c1) = d
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let t = ops::create_wall_type(m, cx, "T", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
                let p = [(0.0, 0.0), (6.0, 0.0), (6.0, 4.0), (0.0, 4.0)];
                for i in 0..4 {
                    let (a, b) = (p[i], p[(i + 1) % 4]);
                    ops::create_wall(m, cx, ops::NewWall { wall_type: t, level: l, start: Vec2::new(a.0, a.1), end: Vec2::new(b.0, b.1), height: 3.0, base_offset: 0.0, flip: false, structural: false })?;
                }
                let ct = ops::create_slab_type(m, cx, "C200", 0.2, crate::model::SlabRole::Ceiling)?;
                let c1 = ops::create_slab_with_loops(m, cx, ct, l, rect(0.1, 0.1, 5.9, 3.9), vec![], 2.0, false)?;
                let room = ops::create_room(m, cx, l, Vec2::new(3.0, 2.0))?;
                Ok((room, c1))
            })
            .value
            .unwrap();
        let vol = |d: &crate::BimDocument| crate::regen::room_state(d.model(), d.model().rooms.get(&room).unwrap()).unwrap().volume;
        assert!((vol(&d) - 44.08).abs() < 1e-6, "{}", vol(&d));
        d.transact("c2", false, |m, cx| {
            let ct = *m.floor_types.keys().next().unwrap();
            let l = *m.levels.keys().next().unwrap();
            ops::create_slab_with_loops(m, cx, ct, l, rect(0.1, 0.1, 2.9, 3.9), vec![], 1.5, false)
        })
        .value
        .unwrap();
        assert!((vol(&d) - 38.76).abs() < 1e-6, "{}", vol(&d));
        d.transact("rb", false, |m, _| {
            m.floors.get_mut(&c1).unwrap().room_bounding = false;
            Ok(())
        });
        assert!((vol(&d) - (10.64 * 1.5 + 11.4 * 2.4384)).abs() < 1e-6, "{}", vol(&d));
    }
}
