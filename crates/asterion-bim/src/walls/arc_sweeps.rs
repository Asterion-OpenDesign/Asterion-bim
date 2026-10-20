//! * the reference product CREATES the sweep (`WallAllowsWallSweep` is true for an arc wall) and the sweep follows the arc:
//!   an annular band 152.4 mm wide (radii `R + t/2 .. R + t/2 + 0.1524`) and 152.4 mm high over the whole arc, 0.18716 m3,
//!   6 faces (2 cylindrical), 12 edges (4 arcs), 8 vertices; its box reaches the outer radius at the arc's ends exactly,
//!   while its extreme across the chord is 0.0016 m SHORT of the outer radius (202.2508 against 202.2524): the box of a
//!   swept cylinder is taken on a tessellation of the arc (39 segments here), a rule the engine does not reproduce (the
//!   exact extent is reported and that one field is a known residual).
//! * the wall keeps its volume and its surface area, its faces stay 6, and it gains 4 LINE edges and
//!   4 vertices: the sweep's top and bottom are imprinted on the wall's two END faces, splitting the
//!   exterior vertical edge of each into three (vertices at z 0.5 and 0.6524 at both ends). A sweep on a
//!   STRAIGHT wall imprints nothing (round 28: 6 / 12 / 8 with a sweep).
//!
//! ENGINE CHOICE (fitted to that one observation. A reveal on an arc cuts the annular groove out of the wall (volume,
//! surface, the groove's 3 faces / 12 edges / 8 vertices as on a straight wall); a vertical sweep is a square prism tangent
//! to the face at its station, imprinting nothing; an offset sweep touches nothing).

use crate::annot::{SweepKind, WallSweep};
use crate::model::{BimModel, Wall};
use crate::regen::{arc_from_points, Arc2, WallState};
use crate::sweeps::{SweepState, PROFILE_M};
use asterion_geom::Vec2;

/// The plan annulus of a sweep on an arc wall, with its z range and its angular sub-range.
struct ArcStrip {
    arc: Arc2,
    r_in: f64,
    r_out: f64,
    z0: f64,
    z1: f64,
    /// Normalised parameters along the host arc the strip spans (0..1 = the whole arc).
    t0: f64,
    t1: f64,
    /// The strip stands on the convex (outward) side of the host.
    outward: bool,
    /// The strip's inner cylinder is the host's face (a sweep at offset 0, or a reveal).
    touches_face: bool,
}

fn host_of<'a>(m: &'a BimModel, sw: &WallSweep) -> Option<(&'a Wall, Arc2)> {
    let w = m.walls.get(&sw.wall)?;
    let arc = arc_from_points(w.start, w.end, w.through?)?;
    Some((w, arc))
}

fn strip(m: &BimModel, sw: &WallSweep) -> Option<ArcStrip> {
    let (w, arc) = host_of(m, sw)?;
    let width = m.wall_types.get(&w.type_id)?.structure.width();
    let (base, top) = crate::regen::wall_vertical_span(m, w).ok()?;
    if !(top > base) {
        return None;
    }
    let theta = arc.sweep.abs();
    let len = theta * arc.radius;
    let s = PROFILE_M;
    // the same position rule as a straight wall (`crate::sweeps::position_in_wall`)
    let limit = if sw.vertical { len } else { top - base };
    if !(sw.distance >= -1e-9 && sw.distance <= limit + 1e-9) {
        return None;
    }
    // the LEFT of the travel is outward on a clockwise arc (`regen::arc_override`); `side_sign` is +1 for
    // the left of the location line
    let outward = (sw.side_sign > 0) == (arc.sweep < 0.0);
    let hw = width / 2.0;
    let (r_in, r_out, touches_face) = match (sw.kind, outward) {
        (SweepKind::Sweep, true) => (arc.radius + hw + sw.offset, arc.radius + hw + sw.offset + s, sw.offset.abs() < 1e-9),
        (SweepKind::Sweep, false) => (arc.radius - hw - sw.offset - s, arc.radius - hw - sw.offset, sw.offset.abs() < 1e-9),
        (SweepKind::Reveal, true) => (arc.radius + hw - s, arc.radius + hw, true),
        (SweepKind::Reveal, false) => (arc.radius - hw, arc.radius - hw + s, true),
    };
    if r_in <= 0.0 || r_out <= r_in {
        return None;
    }
    let (t0, t1) = if sw.vertical {
        // OBSERVED (straight walls): measured from the wall's start; a flipped SWEEP and an unflipped REVEAL hang before
        // their distance
        let before = if sw.kind == SweepKind::Reveal { !sw.flipped } else { sw.flipped };
        let u0 = if before { sw.distance - s } else { sw.distance };
        (u0 / len, (u0 + s) / len)
    } else {
        (0.0, 1.0)
    };
    let (z0, z1) = if sw.vertical {
        (base, top)
    } else {
        let z0 = base + if sw.flipped { sw.distance - s } else { sw.distance };
        (z0, z0 + s)
    };
    Some(ArcStrip { arc, r_in, r_out, z0, z1, t0, t1, outward, touches_face })
}

/// The sub-arc of `arc` between normalised parameters `t0..t1`.
fn sub_arc(arc: &Arc2, t0: f64, t1: f64) -> Arc2 {
    Arc2 { center: arc.center, radius: arc.radius, start_angle: arc.start_angle + arc.sweep * t0, sweep: arc.sweep * (t1 - t0) }
}

/// Closed-form annulus quantities of a horizontal strip: `(plan area, volume, surface)`.
fn annulus(st: &ArcStrip) -> (f64, f64, f64) {
    let theta = st.arc.sweep.abs() * (st.t1 - st.t0);
    let h = st.z1 - st.z0;
    let a = 0.5 * theta * (st.r_out * st.r_out - st.r_in * st.r_in);
    let lateral = theta * (st.r_in + st.r_out) * h + 2.0 * (st.r_out - st.r_in) * h;
    (a, a * h, 2.0 * a + lateral)
}

/// The square plan of a VERTICAL sweep: a 152.4 mm square tangent to the face at the strip's station, standing off the face
/// by the strip's radial range.
fn vertical_square(st: &ArcStrip) -> Vec<Vec2> {
    let mid = 0.5 * (st.t0 + st.t1);
    let a = st.arc.start_angle + st.arc.sweep * mid;
    let radial = Vec2::new(a.cos(), a.sin());
    let tangent = Vec2::new(-radial.y, radial.x);
    let c = st.arc.center;
    let half = 0.5 * PROFILE_M;
    let (ra, rb) = (st.r_in, st.r_out);
    vec![
        c + radial * ra - tangent * half,
        c + radial * ra + tangent * half,
        c + radial * rb + tangent * half,
        c + radial * rb - tangent * half,
    ]
}

/// The measured state of a sweep or reveal whose host is an arc wall.
pub fn sweep_state(m: &BimModel, sw: &WallSweep) -> Option<SweepState> {
    let st = strip(m, sw)?;
    let (w, _) = host_of(m, sw)?;
    let (base, top) = crate::regen::wall_vertical_span(m, w).ok()?;
    let h = st.z1 - st.z0;
    // a reveal inside the wall's height cuts it and has no solid of its own (OBSERVED, straight)
    let cutting = sw.kind == SweepKind::Reveal && sw.sweep_intersects_wall && st.z1 > base + 1e-9 && st.z0 < top - 1e-9;
    let (lo, hi, area, volume, surface) = if sw.vertical {
        let sq = vertical_square(&st);
        let lo = Vec2::new(sq.iter().map(|p| p.x).fold(f64::INFINITY, f64::min), sq.iter().map(|p| p.y).fold(f64::INFINITY, f64::min));
        let hi = Vec2::new(sq.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max), sq.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max));
        let a = PROFILE_M * (st.r_out - st.r_in);
        (lo, hi, a, a * h, 2.0 * a + 2.0 * (PROFILE_M + st.r_out - st.r_in) * h)
    } else {
        let sa = sub_arc(&st.arc, st.t0, st.t1);
        // The box of a solid with arc edges is the box of its stored edge SAMPLE points (OBSERVED: 2.25080782 for a sweep
        // whose arc reaches 2.2524)
        let tols = asterion_geom::stokes::TessTols::display_default();
        let pts: Vec<Vec2> = [st.r_in, st.r_out]
            .iter()
            .filter(|r| **r > 0.0)
            .filter_map(|r| asterion_geom::Arc::new(sa.center, *r, sa.start_angle, sa.sweep).ok().and_then(|a| asterion_geom::stokes::arc_samples_m(&a, &tols).ok()))
            .flatten()
            .collect();
        let (lo, hi) = if pts.is_empty() {
            sa.extent(st.r_in, st.r_out)
        } else {
            (
                Vec2::new(pts.iter().map(|p| p.x).fold(f64::INFINITY, f64::min), pts.iter().map(|p| p.y).fold(f64::INFINITY, f64::min)),
                Vec2::new(pts.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max), pts.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max)),
            )
        };
        let (a, v_closed, s_closed) = annulus(&st);
        // OBSERVED (`sr2`): the sweep's volume is the closed-form annulus to 5e-7 m3, while the band of the wall route
        // (`regen::arc_override`) is 7.7e-6 off it. The surface area is 1.2e-4 m2 ABOVE the closed form in the reference
        // product; the engine uses the exact band area. ENGINE CHOICE.
        let tols = asterion_geom::stokes::TessTols::display_default();
        let band = asterion_geom::stokes::ArcBand::wall(sa.center, sa.start_angle, sa.sweep, st.r_in, st.r_out, st.z0, st.z1);
        let s = asterion_geom::stokes::arc_band_surface_area(&band, &tols).unwrap_or(s_closed);
        (lo, hi, a, v_closed, s)
    };
    let bbox = ([lo.x, lo.y, st.z0], [hi.x, hi.y, st.z1]);
    // Its box is the square's. ENGINE CHOICE (why the vertical sweep has no solid is not specified)
    if cutting || sw.vertical {
        return Some(SweepState { bbox, solid_count: 0, volume: 0.0, surface_area: 0.0, faces: 0, edges: 0, endpoints: 0 });
    }
    let mut volume = volume;
    if sw.kind == SweepKind::Reveal {
        // a reveal above or below the wall: the part of its body outside the wall (as on a straight wall)
        let outside = (st.z1 - st.z0.max(top)).max(0.0) + (st.z1.min(base) - st.z0).max(0.0);
        volume = area * outside.min(h);
    }
    Some(SweepState { bbox, solid_count: 1, volume, surface_area: surface, faces: 6, edges: 12, endpoints: 8 })
}

/// Union length of a set of z intervals.
fn union_length(mut iv: Vec<(f64, f64)>) -> (f64, Vec<(f64, f64)>) {
    iv.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (a, b) in iv {
        match out.last_mut() {
            Some(last) if a <= last.1 + 1e-9 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    (out.iter().map(|(a, b)| b - a).sum(), out)
}

/// What the sweeps and reveals of an ARC wall do to its regenerated state.
pub fn apply(m: &BimModel, wall: &Wall, s: &mut WallState) {
    if wall.through.is_none() || s.solid.is_none() {
        return;
    }
    let (Some((_, arc)), Some(width)) = (
        m.annot.sweeps.values().find(|x| x.wall == wall.id).and_then(|x| host_of(m, x)),
        m.wall_types.get(&wall.type_id).map(|t| t.structure.width()),
    ) else {
        return;
    };
    let theta = arc.sweep.abs();
    let hw = width / 2.0;
    let mut imprinted_ends = 0i64;
    // the grooves of the cutting reveals, per side, as z intervals (overlapping reveals make ONE groove, OBSERVED on
    // straight walls)
    let mut grooves: [Vec<(f64, f64)>; 2] = [Vec::new(), Vec::new()];
    for sw in m.annot.sweeps.values().filter(|x| x.wall == wall.id) {
        let Some(st) = strip(m, sw) else { continue };
        match sw.kind {
            SweepKind::Sweep => {
                if !sw.vertical && st.touches_face && st.t0 <= 1e-9 && st.t1 >= 1.0 - 1e-9 && st.z0 >= s.base_z - 1e-9 && st.z1 <= s.top_z + 1e-9 {
                    imprinted_ends += 2;
                }
            }
            SweepKind::Reveal => {
                if sw.sweep_intersects_wall && !sw.vertical {
                    let (a, b) = (st.z0.max(s.base_z), st.z1.min(s.top_z));
                    if b > a + 1e-9 {
                        grooves[usize::from(st.outward)].push((a, b));
                    }
                }
            }
        }
    }
    // the sweeps' imprints: 2 edges and 2 vertices per wall end
    s.solid_edge_count += (imprinted_ends * 2).max(0) as usize;
    s.solid_vertex_count += (imprinted_ends * 2).max(0) as usize;
    // the reveals' grooves: an annular band `PROFILE_M` deep cut from the face
    for (side, iv) in grooves.iter().enumerate() {
        if iv.is_empty() {
            continue;
        }
        let outward = side == 1;
        let (r_face, r_back) = if outward { (arc.radius + hw, arc.radius + hw - PROFILE_M) } else { (arc.radius - hw, arc.radius - hw + PROFILE_M) };
        let (r_lo, r_hi) = (r_face.min(r_back), r_face.max(r_back));
        if r_lo <= 0.0 {
            continue;
        }
        let (hz, pieces) = union_length(iv.clone());
        let a = 0.5 * theta * (r_hi * r_hi - r_lo * r_lo);
        s.volume -= a * hz;
        // the face strip goes, the groove's back and its two annular faces come, the end caps lose the
        // groove's rectangle on each side
        s.surface_area += -theta * r_face * hz + theta * r_back * hz + 2.0 * a * pieces.len() as f64 - 2.0 * (r_hi - r_lo) * hz;
        s.solid_face_count += 3 * pieces.len();
        s.solid_edge_count += 12 * pieces.len();
        s.solid_vertex_count += 8 * pieces.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sweeps::{create_wall_sweep, use_type, SweepInfo};
    use crate::{ops, CompoundStructure, Layer, LayerFunction};
    use asterion_core::ElementId;

    fn fixture() -> (crate::BimDocument, ElementId) {
        let mut d = crate::new_document();
        let w = d
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let wt = ops::create_wall_type(m, cx, "W", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
                ops::create_arc_wall(
                    m,
                    cx,
                    ops::NewWall { wall_type: wt, level: l, start: Vec2::new(0.0, 200.0), end: Vec2::new(6.0, 200.0), height: 3.0, base_offset: 0.0, flip: false, structural: false },
                    Vec2::new(3.0, 202.0),
                )
            })
            .value
            .unwrap();
        (d, w)
    }

    fn make(d: &mut crate::BimDocument, wall: ElementId, kind: SweepKind, f: impl FnOnce(&mut SweepInfo)) -> ElementId {
        d.transact("s", false, |m, cx| {
            let ty = use_type(m, cx, kind, 0)?;
            let mut info = SweepInfo::new(kind, false);
            f(&mut info);
            create_wall_sweep(m, cx, wall, ty, info)
        })
        .value
        .unwrap()
    }

    #[test]
    fn round_28b_sweep_on_an_arc_wall_follows_the_arc_and_imprints_the_wall_ends() {
        // OBSERVED (`wr2` / `sr2`): sweep 0.18716389666824748 m3, 4.959056876488018 m2, 6 / 12 / 8, box x -0.2329846 ..
        // 6.2329846, y from 200.0384615, z 0.5 .. 0.6524; wall 4.585773762530564 m3, 50.12207093 m2, 6 faces, 16 edges, 12
        // vertices.
        let (mut d, w) = fixture();
        let before = crate::regen::wall_state(d.model(), &d.model().walls[&w]).unwrap();
        let sw = make(&mut d, w, SweepKind::Sweep, |i| i.distance = 0.5);
        let s = crate::sweeps::sweep_state(d.model(), sw).expect("a sweep on an arc wall has a body");
        assert!((s.volume - 0.18716389666824748).abs() < 1e-6, "{}", s.volume);
        // the engine returns the exact area; the reference product's value is 1.7e-4 above it
        assert!((s.surface_area - 4.959056876488018).abs() < 3e-4, "{}", s.surface_area);
        assert_eq!((s.faces, s.edges, s.endpoints), (6, 12, 8));
        assert!((s.bbox.0[0] + 0.2329846153846).abs() < 1e-9 && (s.bbox.1[0] - 6.2329846153846).abs() < 1e-9, "{:?}", s.bbox);
        assert!((s.bbox.0[1] - 200.0384615384615).abs() < 1e-9 && (s.bbox.0[2] - 0.5).abs() < 1e-12 && (s.bbox.1[2] - 0.6524).abs() < 1e-12);
        let after = crate::regen::wall_state(d.model(), &d.model().walls[&w]).unwrap();
        assert!((after.volume - before.volume).abs() < 1e-12 && (after.surface_area - before.surface_area).abs() < 1e-12);
        assert_eq!((after.solid_face_count, after.solid_edge_count, after.solid_vertex_count), (6, 16, 12));
    }

    #[test]
    fn a_reveal_on_an_arc_wall_cuts_an_annular_groove() {
        // ENGINE CHOICE, to be checked by round 32 `sb`
        let (mut d, w) = fixture();
        let before = crate::regen::wall_state(d.model(), &d.model().walls[&w]).unwrap();
        let rv = make(&mut d, w, SweepKind::Reveal, |i| i.distance = 1.0);
        let s = crate::sweeps::sweep_state(d.model(), rv).unwrap();
        assert_eq!(s.solid_count, 0);
        let after = crate::regen::wall_state(d.model(), &d.model().walls[&w]).unwrap();
        let theta = 7.644033846118383 / 3.25;
        let groove = 0.5 * theta * (3.35f64.powi(2) - (3.35 - PROFILE_M).powi(2)) * PROFILE_M;
        assert!((before.volume - after.volume - groove).abs() < 1e-9, "{} {}", before.volume - after.volume, groove);
        assert_eq!((after.solid_face_count, after.solid_edge_count, after.solid_vertex_count), (9, 24, 16));
        assert!((after.area - before.area).abs() < 1e-12);
    }
}
