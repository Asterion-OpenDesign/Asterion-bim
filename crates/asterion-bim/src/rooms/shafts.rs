//! Shafts and openings cut in floors, ceilings and roofs.
//!
//! * a SHAFT (`NewOpening(bottom level, top level, profile)`, category "Shaft Openings") has no host and no solid; its
//!   vertical span runs from its base level + Base Offset to its top level + Top Offset (Unconnected Height 6000 mm for
//!   L0..L2); it cuts every floor and ceiling whose body it overlaps vertically (a shaft whose base, 3.2, is above the
//!   L1 floor's top face, 3.0, leaves that floor whole; one from 0 to 6 cuts the floor whose top is at 6.0);
//! * an opening in a SLOPED roof is committed only as a VERTICAL cut (flag false): its plan profile is the XY
//!   projection of the curves, the roof loses `hole area x sqrt(1 + g^2)` of Area and `hole area x t sqrt(1 + g^2)` of
//!   Volume (the vertical thickness) and gains 4 faces / 8 vertices per rectangular hole; with the flag true the
//!   commit fails ("Can't change plane of Opening Cut Sketch.");
//! * a hole of `k` sides through a slab adds `k` faces, `3k` edges and `2k` vertices (6/12/8 -> 10/24/16);
//! * deleting the host deletes its openings; deleting a shaft leaves the slabs whole again.

use crate::model::{BimModel, Slab, SlabRole};
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use asterion_geom::Vec2;
use serde::{Deserialize, Serialize};

/// ASTERION's failure codes of this slice.
pub mod codes {
    pub const BAD_PROFILE: &str = "shaft.bad_profile";
    pub const BAD_HOST: &str = "opening.bad_host";
    pub const PERPENDICULAR_ON_SLOPED_ROOF: &str = "opening.perpendicular_on_sloped_roof";
    pub const UNKNOWN_PARAMETER: &str = "shaft.unknown_parameter";
}

/// A shaft opening.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shaft {
    pub id: ElementId,
    pub base_level: ElementId,
    pub base_offset: f64,
    /// The top constraint ("Up to level"); `None` = unconnected, the height is `unconnected_height`.
    pub top_level: Option<ElementId>,
    pub top_offset: f64,
    pub unconnected_height: f64,
    /// The closed plan profile (the closing edge is implicit).
    pub boundary: Vec<Vec2>,
}

/// An opening cut in a floor, a ceiling or a roof by a sketched profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlabOpening {
    pub id: ElementId,
    pub host: ElementId,
    /// The closed plan profile (for a roof: the XY projection of the sketched curves).
    pub boundary: Vec<Vec2>,
    /// Cut perpendicular to the host face (true) or vertically (false).
    pub perpendicular: bool,
}

fn check_profile(boundary: &[Vec2]) -> Result<(), Failure> {
    let area = asterion_geom::region::polygon_signed_area(boundary).abs();
    if boundary.len() < 3 || !(area > 1e-9) || boundary.iter().any(|p| !(p.x.is_finite() && p.y.is_finite())) {
        return Err(Failure::error(codes::BAD_PROFILE, "An opening profile needs a closed loop of at least 3 points enclosing an area."));
    }
    Ok(())
}

/// Create a shaft from `base_level` up to `top_level` (OBSERVED: Base / Top Offset 0, Unconnected Height the distance
/// between the two levels).
pub fn create_shaft(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    base_level: ElementId,
    top_level: Option<ElementId>,
    boundary: Vec<Vec2>,
) -> Result<ElementId, Failure> {
    let base = m.level(base_level)?.elevation;
    let height = match top_level {
        Some(t) => m.level(t)?.elevation - base,
        // ENGINE CHOICE: an unconnected shaft without a height is not measured
        None => 3.0,
    };
    check_profile(&boundary)?;
    let id = cx.new_id();
    m.shafts.insert(id, Shaft { id, base_level, base_offset: 0.0, top_level, top_offset: 0.0, unconnected_height: height, boundary });
    Ok(id)
}

/// The vertical span `(bottom, top)` of a shaft.
pub fn shaft_span(m: &BimModel, s: &Shaft) -> Option<(f64, f64)> {
    let bottom = m.levels.get(&s.base_level)?.elevation + s.base_offset;
    let top = match s.top_level {
        Some(t) => m.levels.get(&t)?.elevation + s.top_offset,
        None => bottom + s.unconnected_height,
    };
    Some((bottom, top))
}

/// Set a shaft parameter by its product name (OBSERVED: "Top Offset" and "Base Offset" are writable lengths; "Unconnected
/// Height" applies to an unconnected shaft).
pub fn set_shaft_param(m: &mut BimModel, _cx: &mut TxnContext<'_>, id: ElementId, name: &str, value: f64) -> Result<(), Failure> {
    let s = m.shafts.get_mut(&id).ok_or_else(|| Failure::error(codes::UNKNOWN_PARAMETER, "not a shaft"))?;
    if !value.is_finite() {
        return Err(Failure::error(codes::UNKNOWN_PARAMETER, "the value is not a number"));
    }
    match name {
        "Top Offset" => s.top_offset = value,
        "Base Offset" => s.base_offset = value,
        "Unconnected Height" if value > 0.0 => s.unconnected_height = value,
        _ => return Err(Failure::error(codes::UNKNOWN_PARAMETER, format!("no shaft parameter named {name}"))),
    }
    Ok(())
}

/// A roof is sloped when one of its edges defines a slope.
fn roof_is_sloped(f: &Slab) -> bool {
    f.segments.iter().any(|s| s.defines_slope)
}

/// Create an opening in the floor, ceiling or roof `host`.
///
/// OBSERVED: on a SLOPED roof a perpendicular cut is posted at commit ("Can't change plane of Opening Cut Sketch."), with
/// flat or with sloped curves alike; a vertical cut commits. ENGINE CHOICE: a flat roof accepts either (not measured).
pub fn create_slab_opening(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    host: ElementId,
    boundary: Vec<Vec2>,
    perpendicular: bool,
) -> Result<ElementId, Failure> {
    let f = m.floors.get(&host).ok_or_else(|| Failure::error(codes::BAD_HOST, "An opening needs a floor, a ceiling or a roof as its host."))?;
    let sloped_roof = f.role == SlabRole::Roof && roof_is_sloped(f);
    check_profile(&boundary)?;
    let id = cx.new_id();
    m.slab_openings.insert(id, SlabOpening { id, host, boundary, perpendicular });
    if sloped_roof && perpendicular {
        cx.report(Failure::error(codes::PERPENDICULAR_ON_SLOPED_ROOF, "Can't change plane of Opening Cut Sketch."));
    }
    Ok(id)
}

/// The vertical span `(bottom, top)` of a floor or a ceiling body, without regenerating it.
fn flat_span(m: &BimModel, f: &Slab) -> Option<(f64, f64)> {
    let t = m.slab_type(f.type_id).ok()?.thickness;
    let z = m.levels.get(&f.level)?.elevation + f.height_offset;
    Some(match f.anchor {
        crate::model::SlabAnchor::TopAtLevel => (z - t, z),
        crate::model::SlabAnchor::BottomAtLevel => (z, z + t),
    })
}

/// The plan profiles that cut the floor or ceiling `f`: its own openings and every shaft that overlaps its body vertically
/// by more than 1e-9 m (OBSERVED, see the module docs; ENGINE CHOICE for a shaft that only touches a face). Roofs: see
/// [`roof_holes`].
pub fn flat_slab_holes(m: &BimModel, f: &Slab) -> Vec<Vec<Vec2>> {
    if !matches!(f.role, SlabRole::Floor | SlabRole::Ceiling) {
        return Vec::new();
    }
    let mut out: Vec<Vec<Vec2>> = m.slab_openings.values().filter(|o| o.host == f.id).map(|o| o.boundary.clone()).collect();
    if let Some((b, t)) = flat_span(m, f) {
        for s in m.shafts.values() {
            // ENGINE CHOICE: a shaft cuts a slab whose outline holds the centre of the shaft's profile (round 40: a shaft
            // beside a floor leaves it whole); a partial overlap is not measured
            let c = s.boundary.iter().fold(Vec2::new(0.0, 0.0), |acc, p| acc + *p) * (1.0 / s.boundary.len().max(1) as f64);
            if !asterion_geom::region::polygon_contains_point(&f.boundary, c) {
                continue;
            }
            if let Some((sb, st)) = shaft_span(m, s) {
                if st.min(t) - sb.max(b) > 1e-9 {
                    out.push(s.boundary.clone());
                }
            }
        }
    }
    out
}

/// The vertical openings of the roof `f` (its own openings; ENGINE CHOICE: shafts are not applied to roofs, no measurement
/// has a shaft under a roof).
pub fn roof_holes(m: &BimModel, f: &Slab) -> Vec<Vec<Vec2>> {
    if f.role != SlabRole::Roof {
        return Vec::new();
    }
    m.slab_openings.values().filter(|o| o.host == f.id).map(|o| o.boundary.clone()).collect()
}

/// Whether `id` is a shaft or a slab opening.
pub fn contains(m: &BimModel, id: ElementId) -> bool {
    m.shafts.contains_key(&id) || m.slab_openings.contains_key(&id)
}

/// Delete a shaft or a slab opening; returns the ids removed.
pub fn remove_element_and_dependents(m: &mut BimModel, id: ElementId) -> Vec<ElementId> {
    if m.shafts.remove(&id).is_some() || m.slab_openings.remove(&id).is_some() {
        vec![id]
    } else {
        Vec::new()
    }
}

/// The openings whose host is gone and the shafts whose base level is gone, removed (OBSERVED: deleting the floor deletes
/// its opening, deleting the roof deletes its openings). Returns their ids.
pub fn remove_orphans(m: &mut BimModel) -> Vec<ElementId> {
    let mut gone: Vec<ElementId> = m.slab_openings.values().filter(|o| !m.floors.contains_key(&o.host)).map(|o| o.id).collect();
    // ENGINE CHOICE: a shaft goes with its base level, as a wall does
    gone.extend(m.shafts.values().filter(|s| !m.levels.contains_key(&s.base_level)).map(|s| s.id));
    for id in &gone {
        m.shafts.remove(id);
        m.slab_openings.remove(id);
    }
    for s in m.shafts.values_mut() {
        if s.top_level.map_or(false, |t| !m.levels.contains_key(&t)) {
            s.top_level = None;
        }
    }
    gone
}

/// The vertical openings of a roof applied to its quantities (see the module docs). ENGINE CHOICE: the gradient is the
/// roof's common slope (0 for a flat roof); a hole across facets of different slopes is not measured.
pub fn apply_roof_holes(m: &BimModel, f: &Slab, s: &mut crate::regen::SlabState) {
    let holes = roof_holes(m, f);
    if holes.is_empty() {
        return;
    }
    let g = s.slope.unwrap_or(0.0);
    let k = (1.0 + g * g).sqrt();
    let vt = s.thickness * k;
    for h in holes {
        let a = asterion_geom::region::polygon_signed_area(&h).abs();
        let per = asterion_geom::region::polygon_perimeter(&h);
        let n = h.len();
        s.area -= a * k;
        s.volume -= a * vt;
        s.surface_area += -2.0 * a * k + per * vt;
        s.face_count += n;
        s.edge_count += 3 * n;
        s.vertex_count += 2 * n;
        // the roof's own solid (what `Solid` reports) loses the same; ENGINE CHOICE: a hole that touches a ridge imprints
        // it (round 37 measured 32 edges, not 30), not modelled
        if let Some(r) = s.sloped.as_mut() {
            r.area -= a * k;
            r.volume -= a * vt;
            r.surface_area += -2.0 * a * k + per * vt;
            r.face_count += n;
            r.edge_count += 3 * n;
            r.vertex_count += 2 * n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Vec2> {
        vec![Vec2::new(x0, y0), Vec2::new(x1, y0), Vec2::new(x1, y1), Vec2::new(x0, y1)]
    }

    /// Round 37: two 6 x 4 x 0.2 floors at L1 = 3 and L2 = 6 cut by a 2 x 2 shaft from L0 to L2 (24 -> 20 m2, 4.8 -> 4.0
    /// m3, 10 faces / 16 vertices, Perimeter stays 20); its base raised to 3.2 frees the L1 floor; a floor opening 0.5 x 3
    /// cuts only its host; deleting the host deletes the opening.
    #[test]
    fn a_shaft_cuts_the_floors_it_overlaps_and_an_opening_cuts_its_host() {
        let mut d = crate::new_document();
        let (f1, f2, sh) = d
            .transact("fx", false, |m, cx| {
                let l0 = ops::create_level(m, cx, "L0", 0.0)?;
                let l1 = ops::create_level(m, cx, "L1", 3.0)?;
                let l2 = ops::create_level(m, cx, "L2", 6.0)?;
                let ft = ops::create_floor_type(m, cx, "F200", 0.2)?;
                let f1 = ops::create_slab_with_loops(m, cx, ft, l1, rect(0.0, 0.0, 6.0, 4.0), vec![], 0.0, false)?;
                let f2 = ops::create_slab_with_loops(m, cx, ft, l2, rect(0.0, 0.0, 6.0, 4.0), vec![], 0.0, false)?;
                let sh = create_shaft(m, cx, l0, Some(l2), rect(2.0, 1.0, 4.0, 3.0))?;
                Ok((f1, f2, sh))
            })
            .value
            .unwrap();
        let st = |d: &crate::BimDocument, f| crate::regen::slab_state(d.model(), d.model().floors.get(&f).unwrap()).unwrap();
        for f in [f1, f2] {
            let s = st(&d, f);
            assert!((s.area - 20.0).abs() < 1e-9 && (s.volume - 4.0).abs() < 1e-9 && (s.perimeter - 20.0).abs() < 1e-9, "{s:?}");
            assert_eq!((s.face_count, s.vertex_count), (10, 16));
        }
        d.transact("base", false, |m, cx| set_shaft_param(m, cx, sh, "Base Offset", 3.2));
        assert!((st(&d, f1).area - 24.0).abs() < 1e-9 && (st(&d, f2).area - 20.0).abs() < 1e-9);
        let op = d.transact("op", false, |m, cx| create_slab_opening(m, cx, f2, rect(5.0, 0.5, 5.5, 3.5), true)).value.unwrap();
        assert!((st(&d, f2).area - 18.5).abs() < 1e-9 && (st(&d, f1).area - 24.0).abs() < 1e-9);
        let gone = d.transact("del", false, |m, cx| ops::delete(m, cx, f2)).value.unwrap();
        assert!(gone.contains(&op) && !d.model().slab_openings.contains_key(&op));
        d.transact("delsh", false, |m, cx| ops::delete(m, cx, sh)).value.unwrap();
        assert!((st(&d, f1).area - 24.0).abs() < 1e-9);
    }
}
