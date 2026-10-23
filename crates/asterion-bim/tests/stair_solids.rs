//! The solids of stairs and railings: the numbers are closed-form, the bodies reach the 3D mesh, the sections and the diagnosis, and
//! what the engine cannot build is named.
//!
//! Rows cited: Spec STAIR-TREAD-RISER-01 (treads and risers), Spec STAIR-LANDING-01 (a landing's tread), Spec
//! STAIR-SUPPORT-01 (stringers), Spec RAIL-BALUSTER-01 (rails and balusters). The numbers marked OBSERVED come from the lab
//! harness and are checks, not sources.
mod common;

use asterion_bim::element_geometry::{self, Geometry, PartRole};
use asterion_bim::stair_solids;
use asterion_bim::stairs_scene::{self, Justification, NewScene, SceneLandingSpec, SceneRun, SceneRunSpec};
use asterion_bim::{new_document, ops, BimDocument};
use asterion_core::ElementId;
use asterion_geom::Vec2;

fn v(x: f64, y: f64) -> Vec2 {
    Vec2::new(x, y)
}

struct One {
    doc: BimDocument,
    stairs: ElementId,
    runs: Vec<ElementId>,
    landings: Vec<ElementId>,
    l0: ElementId,
}

/// The template stairs of round 24 between two levels 3 m apart, with the given runs and landings.
fn template(runs: Vec<SceneRunSpec>, landings: Vec<SceneLandingSpec>, type_index: usize) -> One {
    let mut doc = new_document();
    let mut ids = None;
    let out = doc.transact("stairs", false, |m, cx| {
        let l0 = ops::create_level(m, cx, "L0", 0.0)?;
        let l1 = ops::create_level(m, cx, "L1", 3.0)?;
        let ty = stairs_scene::template_stairs_type(m, cx, type_index)?;
        let res = stairs_scene::create_scene(m, cx, NewScene { base_level: l0, top_level: l1, type_id: Some(ty), runs, landings, automatic_landings: Vec::new(), desired_risers: None })?;
        ids = Some((l0, res));
        Ok(())
    });
    assert!(out.committed(), "{:?}", out.failures);
    let (l0, res) = ids.unwrap();
    One { doc, stairs: res.stairs.expect("a stairs"), runs: res.runs.into_iter().flatten().collect(), landings: res.landings.into_iter().flatten().collect(), l0 }
}

fn straight(sx: f64, sy: f64, ex: f64, ey: f64) -> SceneRunSpec {
    SceneRunSpec { run: SceneRun::Straight { start: v(sx, sy), end: v(ex, ey), justification: Justification::Center }, base_elevation: None }
}

#[test]
fn the_template_run_is_two_n_plus_one_bodies_and_two_stringers() {
    let s = template(vec![straight(0.0, 0.0, 4.0, 0.0)], Vec::new(), 0);
    let m = s.doc.model();
    let parts: Vec<_> = stair_solids::stairs_parts(m, s.stairs);
    let all: Vec<_> = parts.iter().flat_map(|(_, p)| p.iter()).collect();
    // 15 treads and 16 risers, 2N + 1 = 31 bodies; a stringer on each side
    assert_eq!(all.iter().filter(|p| p.role == PartRole::Step).count(), 31);
    assert_eq!(all.iter().filter(|p| p.role == PartRole::Support).count(), 2);
    // every solid is closed and genus 0: Euler 2 per solid
    for p in &all {
        assert_eq!(p.solid.vertex_count() as i64 - p.solid.edge_count() as i64 + p.solid.face_count() as i64, 2);
        assert!(p.solid.volume() > 0.0);
    }
    // the harness reports the run as ONE solid and each stringer as one: 3 (round 24, s1), with the volume and area it measured to 1e-4
    let x = stair_solids::stairs_summary(m, s.stairs).unwrap();
    assert_eq!(x.solid_count, 3);
    assert!((x.volume - 0.450_827_864_194_254).abs() < 1e-9, "{x:?}");
    assert!((x.surface_area - 25.715_728_096_280_82).abs() < 1e-9, "{x:?}");
    // the box in plan is the harness's to the metre: x 0 to 4.1697 (the last riser's back corner), y +-0.55 (the stringers), z from 0 to the
    // stairs' top elevation
    let (lo, hi) = stair_solids::stairs_bbox(m, s.stairs).unwrap();
    assert!(lo[0].abs() < 1e-9 && (hi[0] - 4.169_708_144_250_509).abs() < 1e-9, "{lo:?} {hi:?}");
    assert!((lo[1] + 0.55).abs() < 1e-9 && (hi[1] - 0.55).abs() < 1e-9, "{lo:?} {hi:?}");
    assert!(lo[2].abs() < 1e-9 && (hi[2] - 3.0).abs() < 1e-9, "{lo:?} {hi:?}");
}

#[test]
fn a_run_is_the_same_body_whatever_its_direction() {
    // the same run turned a quarter: same volume and surface area, the box turned with it
    let a = template(vec![straight(0.0, 0.0, 4.0, 0.0)], Vec::new(), 0);
    let b = template(vec![straight(2.0, 1.0, 2.0, 5.0)], Vec::new(), 0);
    let (xa, xb) = (stair_solids::stairs_summary(a.doc.model(), a.stairs).unwrap(), stair_solids::stairs_summary(b.doc.model(), b.stairs).unwrap());
    assert!((xa.volume - xb.volume).abs() < 1e-9 && (xa.surface_area - xb.surface_area).abs() < 1e-9);
    assert!((xb.min[0] - 1.45).abs() < 1e-9 && (xb.max[0] - 2.55).abs() < 1e-9, "{xb:?}");
}

#[test]
fn a_landing_is_one_tread_and_four_stringers() {
    // round 24, a U stairs with a sketched landing: the landing is a 0.05 m prism under its base elevation, 4 stringers around it
    let landing = SceneLandingSpec { boundary: vec![v(3.0, 0.0), v(4.2, 0.0), v(4.2, 3.0), v(3.0, 3.0)], elevation: 1.5 };
    let s = template(vec![straight(0.0, 1.0, 3.0, 1.0)], vec![landing], 0);
    let m = s.doc.model();
    let l = s.landings[0];
    let g = element_geometry::geometry_of(m, l);
    let parts = g.parts();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].role, PartRole::Landing);
    let base = m.stairs.landings[&l].base_elevation;
    let (lo, hi) = parts[0].solid.aabb();
    assert!((hi[2] - base).abs() < 1e-9 && (lo[2] - (base - 0.05)).abs() < 1e-9, "{lo:?} {hi:?} base {base}");
    assert!((parts[0].solid.volume() - 1.2 * 3.0 * 0.05).abs() < 1e-9);
    // Four mitre-joined supports, 0.4 m deep, their top (D - Dl) - t = 0.0474 m above the landing
    let sup: Vec<_> = stair_solids::support_geometry(m, s.stairs).into_iter().filter(|p| (p.solid.aabb().1[2] - p.solid.aabb().0[2] - 0.4).abs() < 1e-9).collect();
    assert_eq!(sup.len(), 4, "the landing's four stringers");
    for p in &sup {
        let (lo, hi) = p.solid.aabb();
        assert!((hi[2] - base - 0.047_393_726_959_043).abs() < 1e-9 && (hi[2] - lo[2] - 0.4).abs() < 1e-9, "{lo:?} {hi:?}");
        assert_eq!(p.solid.face_count(), 6);
    }
}

#[test]
fn a_stairs_with_no_stringers_has_only_its_steps() {
    // "Monolithic Stair" has neither stringers nor treads: the engine names the gap instead of drawing a guess
    let s = template(vec![straight(0.0, 0.0, 4.0, 0.0)], Vec::new(), 1);
    let m = s.doc.model();
    let g = element_geometry::geometry_of(m, s.runs[0]);
    assert!(matches!(&g, Geometry::NotGenerated(r) if r.contains("monolithic")), "{g:?}");
    let g = element_geometry::geometry_of(m, s.stairs);
    assert!(matches!(&g, Geometry::NotGenerated(_)), "{g:?}");
    let gaps = element_geometry::model_gaps(m);
    assert!(gaps.iter().any(|(w, n, r)| w == "stair runs" && *n == 1 && r.contains("monolithic")), "{gaps:?}");
    assert!(gaps.iter().any(|(w, _, _)| w == "stairs"), "{gaps:?}");
}

#[test]
fn the_riser_height_moves_every_tread_top() {
    // Spec STAIR-TREAD-RISER-01: the top of tread n is at n x the riser height (n + 1 for a run that begins with a riser)
    let s = template(vec![straight(0.0, 0.0, 4.0, 0.0)], Vec::new(), 0);
    let m = s.doc.model();
    let h = m.stairs.stairs[&s.stairs].actual_riser_height;
    let p = &stair_solids::run_geometry(m, s.runs[0]).unwrap()[0];
    let tops: Vec<f64> = (0..p.solid.face_count())
        .filter(|f| p.solid.face_normal(*f).map_or(false, |n| n[2] > 0.99))
        .map(|f| p.solid.faces()[f].iter().map(|&i| p.solid.vertices()[i][2]).fold(f64::NEG_INFINITY, f64::max))
        .collect();
    assert!(!tops.is_empty());
    // the closing riser stops one tread thickness under its nosing tip: it is the highest top and no tread
    let mut tops = tops;
    tops.sort_by(|a, b| a.total_cmp(b));
    tops.pop();
    for t in tops {
        let n = (t / h).round();
        assert!((t - n * h).abs() < 1e-9, "a tread top at {t} is not a whole number of risers ({h})");
    }
    let _ = s.l0;
}

// ---------------------------------------------------------------------------------------------- views

use asterion_bim::mesh3d::view_mesh;
use asterion_bim::section_graphics::section_figures;
use asterion_bim::view_visibility::{diagnose, Status};

#[test]
fn the_scene_stairs_reach_the_3d_mesh_the_section_the_plan_and_the_diagnosis() {
    let s = common::scene();
    let m = s.doc.model();
    // 3D: the run is in the mesh under its own id, with its triangles and edges in its own range
    let mesh = view_mesh(m, m.view(s.three_d).unwrap()).unwrap();
    assert!(mesh.elements.contains(&s.stairs_run) && mesh.elements.contains(&s.stairs));
    let r = mesh.ranges.iter().find(|r| r.element == s.stairs_run).expect("a range for the run");
    assert!(r.tris.1 > r.tris.0 && r.edges.1 > r.edges.0);
    assert!(!mesh.gaps.iter().any(|(id, _)| *id == s.stairs || *id == s.stairs_run));
    assert_eq!(diagnose(m, s.three_d, s.stairs_run).status, Status::Visible);
    // the section at y = 2 crosses the run (which runs along y from 0.6 to 3.4): a cut figure with the steps' section
    let figs = section_figures(m, s.section).unwrap();
    let cut: Vec<_> = figs.iter().filter(|f| f.element == s.stairs_run && f.cut).collect();
    assert!(!cut.is_empty(), "the run is cut by the section plane");
    for f in &cut {
        assert!(f.poly.as_ref().map_or(false, |p| p.len() >= 4), "{f:?}");
    }
    assert_eq!(diagnose(m, s.section, s.stairs_run).status, Status::Visible);
    // the east elevation looks at the run from the side: its steps are projected (their outlines), nothing cut
    let east = section_figures(m, s.elev_east).unwrap();
    let seen: Vec<_> = east.iter().filter(|f| f.element == s.stairs_run).collect();
    assert!(!seen.is_empty() && seen.iter().all(|f| !f.cut), "{seen:?}");
    assert_eq!(diagnose(m, s.elev_east, s.stairs_run).status, Status::Visible);
    // plan L0: the cut plane (1.2 m) crosses the stairs: cut weight line work, and the steps below it as projection
    let ev = asterion_bim::view_visibility::effective_view(m, m.view(s.plan0).unwrap());
    let lw = stair_solids::plan_linework(m, &ev);
    assert!(lw.iter().any(|g| g.class == 2.0 && g.element == s.stairs_run), "cut line work");
    assert!(lw.iter().any(|g| g.class == 1.0 && g.element == s.stairs_run), "projection line work");
    assert_eq!(diagnose(m, s.plan0, s.stairs).status, Status::Visible);
    // plan L1: the steps lie wholly below its range
    let ev1 = asterion_bim::view_visibility::effective_view(m, m.view(s.plan1).unwrap());
    assert!(stair_solids::plan_linework(m, &ev1).iter().all(|g| g.element != s.stairs_run));
}

#[test]
fn a_section_along_the_run_cuts_its_steps_into_one_sawtooth() {
    // a viewer south of a run that runs west to east, the plane through its middle: the cut is the steps' outline
    let s = template(vec![straight(0.0, 0.0, 4.0, 0.0)], Vec::new(), 0);
    let m = s.doc.model();
    let run = stair_solids::run_geometry(m, s.runs[0]).unwrap();
    let (lo, hi) = run[0].solid.aabb();
    let loops = run[0].solid.slice([0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    assert_eq!(loops.len(), 1, "one cut loop");
    // the cut polygon is the profile: its area times the width is the volume
    let l = &loops[0];
    let area: f64 = (0..l.len()).map(|i| l[i][0] * l[(i + 1) % l.len()][2] - l[(i + 1) % l.len()][0] * l[i][2]).sum::<f64>().abs() / 2.0;
    assert!((area * (hi[1] - lo[1]) - run[0].solid.volume()).abs() < 1e-9);
}

#[test]
fn the_balusters_of_a_pattern_stand_where_row_rail_baluster_01_says() {
    // Spec RAIL-BALUSTER-01: pattern length L = leading + sum of spaces; n = floor((S + 1e-9) / L); spread: n = round(S /
    // L), pitch S / n
    let st = stair_solids::baluster_stations;
    // justification 0 (start): instances from the start; 4 m of 1 m pitch: 1, 2, 3, 4 (each baluster at the end of its space)
    assert_eq!(st(4.0, 0.0, &[1.0], 0), vec![1.0, 2.0, 3.0, 4.0]);
    // 4.5 m, justification 1 (end): the used length 4 is pushed to the end: offset 0.5
    assert_eq!(st(4.5, 0.0, &[1.0], 1), vec![1.5, 2.5, 3.5, 4.5]);
    // centre: offset (4.5 - 4) / 2
    assert_eq!(st(4.5, 0.0, &[1.0], 2), vec![1.25, 2.25, 3.25, 4.25]);
    // spread: n = round(4.4 / 1) = 4, pitch 1.1
    let spread = st(4.4, 0.0, &[1.0], 3);
    assert_eq!(spread.len(), 4);
    assert!((spread[0] - 1.1).abs() < 1e-12 && (spread[3] - 4.4).abs() < 1e-12);
    // a pattern of two entries (0.3 and 0.2): L = 0.5, positions are the running sums inside each instance
    let two = st(1.0, 0.0, &[0.3, 0.2], 0);
    assert_eq!(two.len(), 4);
    assert!((two[0] - 0.3).abs() < 1e-12 && (two[1] - 0.5).abs() < 1e-12 && (two[2] - 0.8).abs() < 1e-12 && (two[3] - 1.0).abs() < 1e-12);
    // L at most 1e-9: no pattern; a path shorter than a pattern holds none (except spread, which rounds)
    assert!(st(4.0, 0.0, &[], 0).is_empty());
    assert!(st(0.4, 0.0, &[1.0], 0).is_empty());
}

#[test]
fn a_railing_on_a_path_is_one_top_rail_rails_and_balusters_inside_the_harness_box() {
    // OBSERVED: the "900mm Pipe" on a path of 4 m then 3 m reaches 0.02 m to the side, 0.9 m up, balusters every 1 m
    let mut doc = new_document();
    let mut rid = None;
    let out = doc.transact("railing", false, |m, cx| {
        let l0 = ops::create_level(m, cx, "L0", 0.0)?;
        let rt = stairs_scene::template_railing_type(m, cx, 0)?;
        rid = Some(stairs_scene::create_railing_on_path(m, cx, rt, l0, vec![(v(20.0, 0.0), v(24.0, 0.0)), (v(24.0, 0.0), v(24.0, 3.0))])?);
        Ok(())
    });
    assert!(out.committed(), "{:?}", out.failures);
    let m = doc.model();
    let id = rid.unwrap();
    assert_eq!(element_geometry::classify(m, id), Some(element_geometry::ElementClass::StairsRailing));
    let parts = element_geometry::geometry_of(m, id);
    let balusters = parts.parts().iter().filter(|p| p.role == PartRole::Baluster).count();
    assert_eq!(balusters, 8, "the harness shows balusters at 0..4 m of the first leg and 1..3 m of the second: 8");
    assert_eq!(parts.parts().iter().filter(|p| p.role == PartRole::TopRail).count(), 1, "one top rail round the corner");
    let x = stair_solids::railing_summary(m, id).unwrap();
    assert!((x.min[0] - 20.0).abs() < 1e-9 && (x.max[1] - 3.0).abs() < 1e-9, "{x:?}");
    assert!((x.min[1] + 0.02).abs() < 1e-9 && (x.max[0] - 24.02).abs() < 1e-9 && (x.max[2] - 0.9).abs() < 1e-9, "{x:?}");
}
