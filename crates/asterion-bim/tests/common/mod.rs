//! The model every view-coherence test draws: one small building with every kind of element the engine can
//! build a body for (and the ones it cannot), on two levels, with openings, a hole in a slab, a gable roof, columns
//! that are cut by a section and behind it, a curtain wall with grid lines and mullions, a toposolid and a stairs.
//!
//! Only what the engine already supports is used. Coordinates are written down here (not derived from the
//! engine) so that a test can say where something must be drawn.
#![allow(dead_code)]

use asterion_bim::columns;
use asterion_bim::curtains;
use asterion_bim::docviews;
use asterion_bim::model::{HostedKind, SlabRole, ViewKind};
use asterion_bim::stairs::{self, NewStairs, NewStraightRun, StairsTypeParams};
use asterion_bim::{new_document, ops, BimDocument, CompoundStructure, Layer, LayerFunction};
use asterion_core::ElementId;
use asterion_geom::Vec2;

pub struct Scene {
    pub doc: BimDocument,
    pub l0: ElementId,
    pub l1: ElementId,
    pub wall_s: ElementId,
    pub wall_e: ElementId,
    pub wall_n: ElementId,
    pub wall_w: ElementId,
    pub door: ElementId,
    pub window: ElementId,
    pub floor0: ElementId,
    pub floor1: ElementId,
    /// A floor with a slope arrow, away from the building (x 11 to 13): its body is not a flat prism.
    pub floor_sloped: ElementId,
    pub ceiling: ElementId,
    pub roof: ElementId,
    pub column_cut: ElementId,
    pub column_behind: ElementId,
    pub curtain: ElementId,
    pub topo: ElementId,
    pub stairs: ElementId,
    pub stairs_run: ElementId,
    pub room: ElementId,
    pub plan0: ElementId,
    pub plan1: ElementId,
    pub three_d: ElementId,
    /// A section through y = 2, the viewer south of it, looking north.
    pub section: ElementId,
    /// Elevations of a marker at (3, 2): index 0 sees x < 3, index 2 sees x > 3.
    pub elev_west: ElementId,
    pub elev_east: ElementId,
}

fn v(x: f64, y: f64) -> Vec2 {
    Vec2::new(x, y)
}

pub fn scene() -> Scene {
    let mut doc = new_document();
    let out = doc.transact("scene", false, |m, cx| {
        let l0 = ops::create_level(m, cx, "L0", 0.0)?;
        let l1 = ops::create_level(m, cx, "L1", 3.0)?;
        let wt = ops::create_wall_type(m, cx, "W200", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
        let mk = |m: &mut asterion_bim::model::BimModel, cx: &mut asterion_core::txn::TxnContext<'_>, a: (f64, f64), b: (f64, f64)| {
            ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l0, start: v(a.0, a.1), end: v(b.0, b.1), height: 3.0, base_offset: 0.0, flip: false, structural: false })
        };
        let wall_s = mk(m, cx, (0.0, 0.0), (6.0, 0.0))?;
        let wall_e = mk(m, cx, (6.0, 0.0), (6.0, 4.0))?;
        let wall_n = mk(m, cx, (6.0, 4.0), (0.0, 4.0))?;
        let wall_w = mk(m, cx, (0.0, 4.0), (0.0, 0.0))?;
        let dt = ops::create_hosted_type(m, cx, "D", HostedKind::Door, 1.0, 2.1, 0.0, 0.0, 0.0, 0.0, 0.0)?;
        let door = ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: dt, host: wall_s, level: l0, host_station: 1.5, sill_height: None })?;
        let wnt = ops::create_hosted_type(m, cx, "Win", HostedKind::Window, 1.2, 1.2, 0.0, 0.0, 0.0, 0.9, 0.0)?;
        let window = ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: wnt, host: wall_n, level: l0, host_station: 3.0, sill_height: None })?;

        let floor_type = ops::create_slab_type(m, cx, "F200", 0.2, SlabRole::Floor)?;
        let rect = vec![v(0.0, 0.0), v(6.0, 0.0), v(6.0, 4.0), v(0.0, 4.0)];
        let floor0 = ops::create_slab(m, cx, floor_type, l0, rect.clone(), 0.0, false)?;
        // the slab of the upper level has a hole the section's plane passes through
        let hole = vec![v(2.0, 1.5), v(3.0, 1.5), v(3.0, 2.5), v(2.0, 2.5)];
        let floor1 = ops::create_slab_with_loops(m, cx, floor_type, l1, rect.clone(), vec![hole], 0.0, false)?;
        let floor_sloped = ops::create_sloped_slab(m, cx, floor_type, l0, vec![v(11.0, 0.0), v(13.0, 0.0), v(13.0, 2.0), v(11.0, 2.0)], 0.0, false, v(11.0, 1.0), v(13.0, 1.0), 0.05)?;
        let ceiling_type = ops::create_slab_type(m, cx, "C100", 0.1, SlabRole::Ceiling)?;
        let ceiling = ops::create_slab(m, cx, ceiling_type, l0, rect.clone(), 2.6, false)?;
        let roof_type = ops::create_slab_type(m, cx, "R200", 0.2, SlabRole::Roof)?;
        let roof = ops::create_slab(m, cx, roof_type, l1, rect.clone(), 0.0, false)?;
        // a gable: the south and north edges define a slope of 0.5, the ridge runs along y = 2 at z = 4
        ops::set_slab_segment_slope(m, cx, roof, 0, true, 0.5)?;
        ops::set_slab_segment_slope(m, cx, roof, 2, true, 0.5)?;

        let ct = columns::create_column_type(m, cx, "C400", Some(0.4), Some(0.4))?;
        let column_cut = columns::place_column(m, cx, ct, l0, v(1.0, 2.0), 0.0)?;
        columns::set_column_top_level(m, cx, column_cut, Some(l1))?;
        let column_behind = columns::place_column(m, cx, ct, l0, v(4.5, 3.0), 0.0)?;
        columns::set_column_top_level(m, cx, column_behind, Some(l1))?;

        let cwt = ops::create_wall_type(m, cx, "CW", CompoundStructure::new(vec![Layer { width: 0.025, function: LayerFunction::Structure }], 0, 0)?)?;
        curtains::make_curtain_type(m, cwt)?;
        let curtain = ops::create_wall(m, cx, ops::NewWall { wall_type: cwt, level: l0, start: v(8.0, 0.0), end: v(8.0, 4.0), height: 3.0, base_offset: 0.0, flip: false, structural: false })?;
        curtains::add_grid_line(m, cx, curtain, false, v(8.0, 2.0), 1.5)?;
        let mt = curtains::create_mullion_type(m, cx, "50mm x 150mm");
        curtains::add_mullions(m, cx, curtain, false, 0, mt)?;

        let topo = asterion_bim::toposolid::create_toposolid(m, cx, l0, Vec::new(), vec![[-3.0, -3.0], [10.0, -3.0], [10.0, 7.0], [-3.0, 7.0]])?;

        let stype = stairs::create_stairs_type(
            m,
            cx,
            "S",
            StairsTypeParams {
                max_riser_height: 0.18,
                min_tread_depth: 0.28,
                min_run_width: 1.0,
                use_calculator: false,
                calculator_target: 0.0,
                calculator_riser_multiplier: 0.0,
                calculator_tread_multiplier: 0.0,
                min_run_height: None,
                tread_thickness: None,
            },
        )?;
        let st = stairs::create_stairs(m, cx, NewStairs { type_id: stype, base_level: l0, top_level: l1, initial_tread_depth: 0.28 })?;
        let run = stairs::add_straight_stair_run(
            m,
            cx,
            NewStraightRun { stairs: st, start: v(3.5, 0.6), end: v(3.5, 3.4), bottom_elevation: 0.0, width: 1.0, begin_with_riser: true, end_with_riser: true, run_type_code: 0 },
        )?;
        let room = ops::create_room(m, cx, l0, v(3.0, 2.0))?;

        let plan0 = ops::create_view(m, cx, "Plan L0", ViewKind::FloorPlan, Some(l0), None)?;
        let plan1 = ops::create_view(m, cx, "Plan L1", ViewKind::FloorPlan, Some(l1), None)?;
        let three_d = ops::create_view(m, cx, "{3D}", ViewKind::ThreeD, None, None)?;

        let section = docviews::add_section_view(m, cx, Some("S y=2"), [-1.0, 2.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0], [0.0, -1.0, -12.0], [10.0, 6.0, 0.0])?;
        let marker = docviews::create_marker(m, cx, plan0, [3.0, 2.0, 0.0], 100)?;
        let elev_west = docviews::add_elevation_view(m, cx, marker, 0)?;
        let elev_east = docviews::add_elevation_view(m, cx, marker, 2)?;
        for e in [elev_west, elev_east] {
            let dv = m.docs.views.get_mut(&e).expect("elevation");
            dv.crop_min = [-8.0, -1.0, -20.0];
            dv.crop_max = [8.0, 6.0, 0.0];
        }
        Ok(Scene {
            doc: new_document(),
            l0,
            l1,
            wall_s,
            wall_e,
            wall_n,
            wall_w,
            door,
            window,
            floor0,
            floor1,
            floor_sloped,
            ceiling,
            roof,
            column_cut,
            column_behind,
            curtain,
            topo,
            stairs: st,
            stairs_run: run,
            room,
            plan0,
            plan1,
            three_d,
            section,
            elev_west,
            elev_east,
        })
    });
    let mut s = out.value.unwrap_or_else(|| panic!("the scene was not committed: {:?}", out.failures));
    s.doc = doc;
    s
}

/// Whether a triangle (three `[x, y, z]` points) covers the point, which must lie on its plane (to 1e-6).
pub fn triangle_covers(t: [[f64; 3]; 3], p: [f64; 3]) -> bool {
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let (e1, e2, w) = (sub(t[1], t[0]), sub(t[2], t[0]), sub(p, t[0]));
    let n = cross(e1, e2);
    let nl = dot(n, n).sqrt();
    if nl < 1e-12 || (dot(n, w) / nl).abs() > 1e-6 {
        return false;
    }
    let (d00, d01, d11, d20, d21) = (dot(e1, e1), dot(e1, e2), dot(e2, e2), dot(w, e1), dot(w, e2));
    let den = d00 * d11 - d01 * d01;
    let (a, b) = ((d11 * d20 - d01 * d21) / den, (d00 * d21 - d01 * d20) / den);
    a >= -1e-9 && b >= -1e-9 && a + b <= 1.0 + 1e-9
}

/// The triangles of a mesh as point triples.
pub fn triangles(mesh: &asterion_bim::mesh3d::Mesh3d) -> Vec<[[f64; 3]; 3]> {
    mesh.triangles
        .chunks_exact(27)
        .map(|t| {
            let p = |i: usize| [t[i * 9] as f64, t[i * 9 + 1] as f64, t[i * 9 + 2] as f64];
            [p(0), p(1), p(2)]
        })
        .collect()
}

/// Whether the mesh has a vertex that satisfies `pred`.
pub fn mesh_has_vertex(mesh: &asterion_bim::mesh3d::Mesh3d, pred: impl Fn([f64; 3]) -> bool) -> bool {
    mesh.triangles.chunks_exact(9).any(|v| pred([v[0] as f64, v[1] as f64, v[2] as f64]))
}

/// Whether any triangle of the mesh covers `p`.
pub fn mesh_covers(mesh: &asterion_bim::mesh3d::Mesh3d, p: [f64; 3]) -> bool {
    triangles(mesh).into_iter().any(|t| triangle_covers(t, p))
}
