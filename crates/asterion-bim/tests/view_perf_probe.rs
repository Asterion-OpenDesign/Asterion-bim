//! Cost of the 3D mesh and of a section on a medium model (about 1 000 walls with doors, 100 slabs, 100 columns).
//! Ignored by default (it measures, it does not assert a speed): run with
//! `cargo test -p asterion-bim --release --test view_perf_probe -- --ignored --nocapture`.
//! Only the long-standing public API is used, so the same file runs against any revision of the engine.

use asterion_bim::columns;
use asterion_bim::docviews;
use asterion_bim::model::{HostedKind, SlabRole, ViewKind};
use asterion_bim::{new_document, ops, CompoundStructure, Layer, LayerFunction};
use asterion_geom::Vec2;
use std::time::Instant;

#[test]
#[ignore]
fn mesh_and_section_cost_on_a_medium_model() {
    let mut doc = new_document();
    let n = 20; // 20 x 20 rooms of 5 m: 2 x 20 x 21 = 840 walls
    let out = doc.transact("grid", false, |m, cx| {
        let l0 = ops::create_level(m, cx, "L0", 0.0)?;
        let wt = ops::create_wall_type(m, cx, "W", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
        let dt = ops::create_hosted_type(m, cx, "D", HostedKind::Door, 0.9, 2.1, 0.0, 0.0, 0.0, 0.0, 0.0)?;
        let ft = ops::create_slab_type(m, cx, "F", 0.2, SlabRole::Floor)?;
        let ct = columns::create_column_type(m, cx, "C", Some(0.3), Some(0.3))?;
        let mut first_wall = None;
        for i in 0..=n {
            for j in 0..n {
                let (a, b) = (Vec2::new(i as f64 * 5.0, j as f64 * 5.0), Vec2::new(i as f64 * 5.0, (j + 1) as f64 * 5.0));
                let w = ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l0, start: a, end: b, height: 3.0, base_offset: 0.0, flip: false, structural: false })?;
                first_wall.get_or_insert(w);
                if (i + j) % 3 == 0 {
                    ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: dt, host: w, level: l0, host_station: 2.5, sill_height: None })?;
                }
                let (c, d) = (Vec2::new(j as f64 * 5.0, i as f64 * 5.0), Vec2::new((j + 1) as f64 * 5.0, i as f64 * 5.0));
                ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l0, start: c, end: d, height: 3.0, base_offset: 0.0, flip: false, structural: false })?;
            }
        }
        for k in 0..100 {
            let (x, y) = ((k % 10) as f64 * 10.0, (k / 10) as f64 * 10.0);
            ops::create_slab(m, cx, ft, l0, vec![Vec2::new(x, y), Vec2::new(x + 9.0, y), Vec2::new(x + 9.0, y + 9.0), Vec2::new(x, y + 9.0)], 0.0, false)?;
            columns::place_column(m, cx, ct, l0, Vec2::new(x + 2.5, y + 2.5), 0.0)?;
        }
        let three = ops::create_view(m, cx, "{3D}", ViewKind::ThreeD, None, None)?;
        let sec = docviews::add_section_view(m, cx, Some("S"), [-5.0, 50.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0], [0.0, -1.0, -60.0], [110.0, 6.0, 0.0])?;
        Ok((three, sec))
    });
    let (three, sec) = out.value.expect("scene");
    let m = doc.model();
    println!("walls {}, slabs {}, columns {}", m.walls.len(), m.floors.len(), m.columns.len());
    for run in 0..3 {
        let t = Instant::now();
        let mesh = asterion_bim::mesh3d::view_mesh(m, m.view(three).unwrap()).unwrap();
        let t_mesh = t.elapsed();
        let t = Instant::now();
        let segs = asterion_bim::section_graphics::section_segments(m, sec).unwrap();
        let t_sec = t.elapsed();
        println!("run {run}: mesh {:>7.1} ms ({} solids, {} triangles)   section {:>7.1} ms ({} segments)", t_mesh.as_secs_f64() * 1e3, mesh.solids, mesh.triangle_count(), t_sec.as_secs_f64() * 1e3, segs.len());
    }
}
