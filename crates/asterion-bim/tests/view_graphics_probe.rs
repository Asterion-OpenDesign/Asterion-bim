//! What the views actually draw, read from their OUTPUT buffers (the 3D mesh's vertices and the section's line work),
//! independently of any bookkeeping of the engine: an element is "in" a view when something of it reaches the buffer the
//! renderer is handed. It uses only the long-standing public API (`mesh3d::view_mesh`, `section_graphics::*`), so it
//! can be run against any revision of the engine - it is how the defect was reproduced before the fix.
//!
//! Where an element is expected is written down by hand from the scene's coordinates (`common::scene`).

mod common;

use asterion_bim::section_graphics::{section_segments, CLASS_CUT, CLASS_PROJECTION};
use common::*;

#[test]
fn the_3d_view_draws_every_kind_of_element_that_has_a_body() {
    let s = scene();
    let m = s.doc.model();
    let mesh = asterion_bim::mesh3d::view_mesh(m, m.view(s.three_d).unwrap()).unwrap();
    // walls: vertices on the building's outline at the top
    assert!(mesh_has_vertex(&mesh, |p| (p[2] - 3.0).abs() < 1e-6 && p[0] > 5.8 && p[0] < 6.2), "the east wall");
    assert!(mesh_has_vertex(&mesh, |p| (p[2] - 3.0).abs() < 1e-6 && p[1] > 3.8 && p[1] < 4.2), "the north wall");
    // slabs of every role: each has a vertex at an elevation no other element has
    assert!(mesh_has_vertex(&mesh, |p| (p[2] + 0.2).abs() < 1e-6 && p[0] < 6.5), "the floor of L0 (bottom at -0.2)");
    assert!(mesh_has_vertex(&mesh, |p| (p[2] - 2.8).abs() < 1e-6), "the floor of L1 (bottom at 2.8)");
    assert!(mesh_has_vertex(&mesh, |p| (p[2] - 2.6).abs() < 1e-6), "the ceiling (bottom at 2.6)");
    assert!(mesh_has_vertex(&mesh, |p| p[2] > 3.5), "the gable roof (ridge at 4.0)");
    // columns: the one behind the section plane (the other one is cut by it, at x = 1)
    assert!(mesh_has_vertex(&mesh, |p| (p[0] - 4.5).abs() <= 0.2 + 1e-6 && (p[1] - 3.0).abs() <= 0.2 + 1e-6), "the column at (4.5, 3)");
    assert!(mesh_has_vertex(&mesh, |p| (p[0] - 1.0).abs() <= 0.2 + 1e-6 && (p[1] - 2.0).abs() <= 0.2 + 1e-6 && p[2] > 0.0), "the column at (1, 2)");
    // the curtain wall and the toposolid
    assert!(mesh_has_vertex(&mesh, |p| p[0] > 7.9), "the curtain wall at x = 8");
    assert!(mesh_has_vertex(&mesh, |p| p[0] < -2.5), "the toposolid");
}

#[test]
fn the_3d_walls_are_cut_where_a_door_and_a_window_are() {
    let s = scene();
    let m = s.doc.model();
    let mesh = asterion_bim::mesh3d::view_mesh(m, m.view(s.three_d).unwrap()).unwrap();
    // the door (1.0 x 2.1 at station 1.5 of the south wall, y = -0.1 .. 0.1): nothing of the wall covers its middle
    for y in [-0.1, 0.1] {
        assert!(!mesh_covers(&mesh, [1.5, y, 1.0]), "the door's opening is still wall on the face y = {y}");
    }
    // while the wall beside it is there
    assert!(mesh_covers(&mesh, [3.0, 0.1, 1.0]) || mesh_covers(&mesh, [3.0, -0.1, 1.0]), "the wall beside the door");
    // the window (1.2 x 1.2, sill 0.9, at station 3 of the north wall, y = 3.9 .. 4.1 - the wall runs east to west, so x = 3)
    for y in [3.9, 4.1] {
        assert!(!mesh_covers(&mesh, [3.0, y, 1.5]), "the window's opening is still wall on the face y = {y}");
    }
}

#[test]
fn a_hole_in_a_slab_is_open_in_3d() {
    let s = scene();
    let m = s.doc.model();
    let mesh = asterion_bim::mesh3d::view_mesh(m, m.view(s.three_d).unwrap()).unwrap();
    // the slab of L1: top at 3.0, hole (2, 1.5) - (3, 2.5)
    assert!(!mesh_covers(&mesh, [2.5, 2.0, 3.0]), "the top of the slab covers its hole");
    assert!(mesh_covers(&mesh, [1.0, 1.0, 3.0]), "the top of the slab beside the hole");
}

fn elements_of(segs: &[asterion_bim::section_graphics::Seg], class: f32) -> std::collections::BTreeSet<u64> {
    segs.iter().filter(|g| g.class == class).map(|g| g.element.0).collect()
}

#[test]
fn the_section_draws_every_kind_of_element_it_cuts_or_sees() {
    let s = scene();
    let m = s.doc.model();
    let segs = section_segments(m, s.section).unwrap();
    let cut = elements_of(&segs, CLASS_CUT);
    let seen = elements_of(&segs, CLASS_PROJECTION);
    let drawn = |e: asterion_core::ElementId| cut.contains(&e.0) || seen.contains(&e.0);
    // cut by the plane y = 2
    for (name, e) in [("east wall", s.wall_e), ("west wall", s.wall_w), ("floor of L0", s.floor0), ("floor of L1", s.floor1), ("ceiling", s.ceiling), ("roof", s.roof), ("column at (1, 2)", s.column_cut), ("curtain wall", s.curtain), ("toposolid", s.topo)] {
        assert!(cut.contains(&e.0), "{name} is cut by the section plane and is not in its cut line work");
    }
    // behind it
    for (name, e) in [("north wall", s.wall_n), ("column at (4.5, 3)", s.column_behind)] {
        assert!(seen.contains(&e.0), "{name} lies behind the plane and is not projected");
    }
    // the window in the north wall is drawn with its host
    assert!(drawn(s.wall_n));
    // in front of the plane: nothing of the south wall
    assert!(!drawn(s.wall_s), "the south wall is in front of the plane and must not be drawn");
}

#[test]
fn the_elevations_draw_what_is_behind_their_plane() {
    let s = scene();
    let m = s.doc.model();
    // the elevation that sees x > 3: the east wall, the curtain wall and the column at (4.5, 3) are behind it
    let east = section_segments(m, s.elev_east).unwrap();
    let all: std::collections::BTreeSet<u64> = east.iter().map(|g| g.element.0).collect();
    for (name, e) in [("east wall", s.wall_e), ("curtain wall", s.curtain), ("column at (4.5, 3)", s.column_behind), ("roof", s.roof), ("toposolid", s.topo)] {
        assert!(all.contains(&e.0), "{name} is behind the east elevation's plane and is not drawn");
    }
    // the elevation that sees x < 3: the west wall and the column at (1, 2)
    let west = section_segments(m, s.elev_west).unwrap();
    let all: std::collections::BTreeSet<u64> = west.iter().map(|g| g.element.0).collect();
    for (name, e) in [("west wall", s.wall_w), ("column at (1, 2)", s.column_cut)] {
        assert!(all.contains(&e.0), "{name} is behind the west elevation's plane and is not drawn");
    }
    assert!(!all.contains(&s.wall_e.0), "the east wall is in front of the west elevation's plane");
}

/// Prints which element reaches which view's output buffer (never fails: it is the evidence table, run with `--nocapture`).
#[test]
fn matrix_of_what_each_view_draws() {
    let s = scene();
    let m = s.doc.model();
    let mesh = asterion_bim::mesh3d::view_mesh(m, m.view(s.three_d).unwrap()).unwrap();
    let rows: Vec<(&str, asterion_core::ElementId, Box<dyn Fn([f64; 3]) -> bool>)> = vec![
        ("wall (east)", s.wall_e, Box::new(|p| p[0] > 5.8 && p[0] < 6.2 && (p[2] - 3.0).abs() < 1e-6)),
        ("floor L0", s.floor0, Box::new(|p| (p[2] + 0.2).abs() < 1e-6 && p[0] < 6.5)),
        ("floor L1 (holed)", s.floor1, Box::new(|p| (p[2] - 2.8).abs() < 1e-6)),
        ("ceiling", s.ceiling, Box::new(|p| (p[2] - 2.6).abs() < 1e-6)),
        ("roof (gable)", s.roof, Box::new(|p| p[2] > 3.5)),
        ("column (4.5, 3)", s.column_behind, Box::new(|p| (p[0] - 4.5).abs() <= 0.2 + 1e-6 && (p[1] - 3.0).abs() <= 0.2 + 1e-6 && p[2] > 0.0)),
        ("curtain wall", s.curtain, Box::new(|p| p[0] > 7.9)),
        ("toposolid", s.topo, Box::new(|p| p[0] < -2.5)),
    ];
    let segs = |v| section_segments(m, v).unwrap();
    let (sec, east, west) = (segs(s.section), segs(s.elev_east), segs(s.elev_west));
    let has = |g: &[asterion_bim::section_graphics::Seg], e: asterion_core::ElementId| g.iter().any(|x| x.element == e);
    println!("{:<20} {:>5} {:>8} {:>10} {:>10}", "element", "3D", "section", "elev-east", "elev-west");
    for (name, e, pred) in &rows {
        println!("{:<20} {:>5} {:>8} {:>10} {:>10}", name, mesh_has_vertex(&mesh, pred), has(&sec, *e), has(&east, *e), has(&west, *e));
    }
    println!("{:<20} {:>5} {:>8} {:>10} {:>10}", "column (1, 2)", mesh_has_vertex(&mesh, |p| (p[0] - 1.0).abs() <= 0.2 + 1e-6 && (p[1] - 2.0).abs() <= 0.2 + 1e-6 && p[2] > 0.0), has(&sec, s.column_cut), has(&east, s.column_cut), has(&west, s.column_cut));
    println!("door opening open in 3D: {}   window opening open in 3D: {}   slab hole open in 3D: {}", !mesh_covers(&mesh, [1.5, 0.1, 1.0]), !mesh_covers(&mesh, [3.0, 4.1, 1.5]), !mesh_covers(&mesh, [2.5, 2.0, 3.0]));
}
