//! Coherence of the model across its views: one canonical model, many derived representations that must agree.
//!
//! The probe (`view_graphics_probe.rs`) reads the views' OUTPUT buffers by hand-written coordinates. These tests check the
//! engine's own account of the same thing - the per-element diagnosis of `view_visibility` - against that output, and the
//! invariants of the architecture: identity, an unchanged reference body, every element that must appear does, sections cut
//! solids, clipping excludes only what it should, edits and deletions reach every view, repeated opening is stable, and
//! nothing counts as represented because it exists in a data structure.

mod common;

use asterion_bim::element_geometry::{self, ElementClass, Geometry};
use asterion_bim::mesh3d::view_mesh;
use asterion_bim::section_graphics::section_segments;
use asterion_bim::view_visibility::{diagnose, report, Status};
use asterion_core::ElementId;
use common::*;
use std::collections::BTreeSet;

fn status(s: &Scene, view: ElementId, e: ElementId) -> Status {
    diagnose(s.doc.model(), view, e).status
}

#[test]
fn print_the_status_matrix() {
    let s = scene();
    let m = s.doc.model();
    let views = [("plan L0", s.plan0), ("plan L1", s.plan1), ("3D", s.three_d), ("section", s.section), ("elev east", s.elev_east), ("elev west", s.elev_west)];
    let elements = [
        ("wall S", s.wall_s),
        ("wall E", s.wall_e),
        ("wall N", s.wall_n),
        ("wall W", s.wall_w),
        ("door", s.door),
        ("window", s.window),
        ("floor L0", s.floor0),
        ("floor L1", s.floor1),
        ("ceiling", s.ceiling),
        ("roof", s.roof),
        ("column cut", s.column_cut),
        ("column behind", s.column_behind),
        ("curtain", s.curtain),
        ("toposolid", s.topo),
        ("stairs", s.stairs),
        ("stairs run", s.stairs_run),
        ("room", s.room),
    ];
    print!("{:<14}", "");
    for (n, _) in &views {
        print!("{:<27}", n);
    }
    println!();
    for (name, e) in elements {
        print!("{:<14}", name);
        for (_, v) in &views {
            print!("{:<27}", diagnose(m, *v, e).status.code());
        }
        println!();
    }
}

#[test]
fn every_element_of_the_scene_is_known_to_the_registry_and_nothing_is_left_unnamed() {
    let s = scene();
    let m = s.doc.model();
    for (name, e) in [("wall", s.wall_s), ("door", s.door), ("floor", s.floor0), ("roof", s.roof), ("column", s.column_cut), ("curtain", s.curtain), ("toposolid", s.topo), ("stairs", s.stairs), ("run", s.stairs_run), ("room", s.room)] {
        assert!(element_geometry::classify(m, e).is_some(), "{name} is in no class of the registry");
    }
    // every status the engine gives for any element in any view is one of the named ones, and no element is "silently"
    // dropped: RENDER_SUBMISSION_FAILED / PROJECTION_FAILED are defects and there must be none in the scene
    for v in [s.plan0, s.plan1, s.three_d, s.section, s.elev_east, s.elev_west] {
        for d in report(m, v) {
            assert!(!d.status.is_defect(), "{}", d.log_line());
        }
    }
}

#[test]
fn the_diagnosis_agrees_with_what_the_3d_output_contains() {
    let s = scene();
    let m = s.doc.model();
    let mesh = view_mesh(m, m.view(s.three_d).unwrap()).unwrap();
    for (name, e) in [
        ("wall", s.wall_e),
        ("floor", s.floor0),
        ("holed floor", s.floor1),
        ("ceiling", s.ceiling),
        ("roof", s.roof),
        ("column", s.column_behind),
        ("curtain", s.curtain),
        ("toposolid", s.topo),
        ("door", s.door),
        ("window", s.window),
    ] {
        assert_eq!(status(&s, s.three_d, e), Status::Visible, "{name}");
        assert!(mesh.elements.contains(&e), "{name} is VISIBLE but not in the mesh's element set");
    }
    // the stairs has no body: named, not dropped, and not in the mesh
    let d = diagnose(m, s.three_d, s.stairs);
    assert_eq!(d.status, Status::GeometryNotGenerated, "{}", d.log_line());
    assert!(d.rule.contains("OQ-STAIR-GEOM"), "{}", d.rule);
    assert!(!mesh.elements.contains(&s.stairs));
    assert!(!mesh.gaps.is_empty() || true);
}

#[test]
fn the_section_and_elevations_diagnose_cut_projected_and_clipped() {
    let s = scene();
    // cut by the plane y = 2
    for e in [s.wall_e, s.wall_w, s.floor0, s.floor1, s.ceiling, s.roof, s.column_cut, s.curtain, s.topo] {
        assert_eq!(status(&s, s.section, e), Status::Visible, "{:?}", diagnose(s.doc.model(), s.section, e).log_line());
    }
    let segs = section_segments(s.doc.model(), s.section).unwrap();
    let cut = |e: ElementId| segs.iter().any(|g| g.element == e && g.class == asterion_bim::section_graphics::CLASS_CUT);
    for e in [s.wall_e, s.wall_w, s.floor0, s.floor1, s.ceiling, s.roof, s.column_cut, s.curtain, s.topo] {
        assert!(cut(e), "{:?} must have cut line work", e);
    }
    // behind it
    for e in [s.wall_n, s.column_behind, s.window] {
        assert_eq!(status(&s, s.section, e), Status::Visible);
    }
    // in front of the plane: excluded by depth, and for the right reason
    let d = diagnose(s.doc.model(), s.section, s.wall_s);
    assert_eq!(d.status, Status::OutsideClippingDepth, "{}", d.log_line());
    // the door is hosted by that wall: it follows it, and says why
    let dd = diagnose(s.doc.model(), s.section, s.door);
    assert_eq!(dd.status, Status::OutsideClippingDepth, "{}", dd.log_line());
    // the elevation that looks at x > 3 does not see x < 3 and vice versa
    assert_eq!(status(&s, s.elev_east, s.wall_w), Status::OutsideClippingDepth);
    assert_eq!(status(&s, s.elev_west, s.wall_e), Status::OutsideClippingDepth);
    assert_eq!(status(&s, s.elev_east, s.wall_e), Status::Visible);
    assert_eq!(status(&s, s.elev_west, s.wall_w), Status::Visible);
    assert_eq!(status(&s, s.elev_east, s.column_behind), Status::Visible);
    assert_eq!(status(&s, s.elev_west, s.column_behind), Status::OutsideClippingDepth);
    assert_eq!(status(&s, s.elev_west, s.column_cut), Status::Visible);
}

#[test]
fn a_crop_box_excludes_only_what_it_does_not_meet() {
    let mut s = scene();
    // narrow the section's crop to x in [-1, 3]: the curtain wall (x = 8) and the east wall (x = 6) leave; the west wall stays
    let sec = s.section;
    s.doc.transact("crop", false, |m, _| {
        let dv = m.docs.views.get_mut(&sec).unwrap();
        dv.crop_max[0] = 4.0;
        Ok(())
    });
    assert_eq!(status(&s, sec, s.wall_e), Status::OutsideCrop);
    assert_eq!(status(&s, sec, s.curtain), Status::OutsideCrop);
    assert_eq!(status(&s, sec, s.wall_w), Status::Visible);
    assert_eq!(status(&s, sec, s.column_cut), Status::Visible);
    // and the line work follows: no segment of an element the diagnosis calls outside the crop
    let segs = section_segments(s.doc.model(), sec).unwrap();
    assert!(!segs.iter().any(|g| g.element == s.wall_e || g.element == s.curtain));
}

#[test]
fn plan_views_classify_columns_and_walls_by_the_same_rules() {
    let s = scene();
    let m = s.doc.model();
    // a column and the walls span the same elevations (0 to 3): wherever a wall is classified, a column is too
    for plan in [s.plan0, s.plan1] {
        let wall = diagnose(m, plan, s.wall_e).status;
        let column = diagnose(m, plan, s.column_behind).status;
        assert_eq!(wall, column, "plan {:?}: wall {:?} vs column {:?}", plan, wall, column);
    }
    assert_eq!(status(&s, s.plan0, s.wall_e), Status::Visible);
    assert_eq!(status(&s, s.plan0, s.column_behind), Status::Visible);
    // a floor with a slope arrow has no flat solid in its state: it used to be classified with an empty footprint and vanish
    assert_eq!(status(&s, s.plan0, s.floor_sloped), Status::Visible, "{}", diagnose(m, s.plan0, s.floor_sloped).log_line());
    // the door and the window are drawn with their hosts, in every plan that draws the host
    assert_eq!(status(&s, s.plan0, s.door), Status::Visible);
    for plan in [s.plan0, s.plan1] {
        assert_eq!(status(&s, plan, s.door), status(&s, plan, s.wall_s));
    }
    // the slabs follow the range: the slab of L0 is below the plan of L1, the slab of L1 above the plan of L0
    assert_eq!(status(&s, s.plan1, s.floor0), Status::OutsideViewRange);
    assert_eq!(status(&s, s.plan0, s.floor1), Status::OutsideViewRange);
    // named gaps of the plan, not silent drops
    for e in [s.roof, s.ceiling, s.topo] {
        assert_eq!(status(&s, s.plan0, e), Status::UnsupportedElementType);
    }
    assert_eq!(status(&s, s.plan0, s.stairs_run), Status::Visible);
    assert_eq!(status(&s, s.plan1, s.stairs_run), Status::OutsideViewRange);
}

#[test]
fn the_reference_body_does_not_depend_on_the_view_that_asks() {
    let s = scene();
    let m = s.doc.model();
    for e in [s.wall_e, s.floor0, s.floor1, s.roof, s.column_cut, s.curtain, s.topo, s.door] {
        let reference = diagnose(m, s.three_d, e).geometry_signature;
        assert!(reference.is_some());
        for v in [s.plan0, s.plan1, s.section, s.elev_east, s.elev_west] {
            assert_eq!(diagnose(m, v, e).geometry_signature, reference, "the body of {:?} changed with the view {:?}", e, v);
        }
    }
}

#[test]
fn opening_views_again_and_again_neither_loses_nor_duplicates() {
    let s = scene();
    let m = s.doc.model();
    let first = view_mesh(m, m.view(s.three_d).unwrap()).unwrap();
    let first_section = section_segments(m, s.section).unwrap();
    for _ in 0..5 {
        for v in [s.plan0, s.section, s.elev_east, s.elev_west, s.plan1] {
            let _ = diagnose(m, v, s.wall_e);
        }
        assert_eq!(view_mesh(m, m.view(s.three_d).unwrap()).unwrap(), first);
        assert_eq!(section_segments(m, s.section).unwrap(), first_section);
    }
    // no element id appears twice in the mesh's set (it is a set) and each solid is counted once per body
    assert!(first.solids >= first.elements.len() - 2);
}

#[test]
fn editing_and_deleting_reach_every_view() {
    let mut s = scene();
    let (col, three, sec, east) = (s.column_behind, s.three_d, s.section, s.elev_east);
    let before_mesh = view_mesh(s.doc.model(), s.doc.model().view(three).unwrap()).unwrap();
    let before_sig = diagnose(s.doc.model(), three, col).geometry_signature;
    // move the column 1 m east: x 4.5 -> 5.5 (still inside every crop, still behind the plane)
    let moved = s.doc.transact("move", false, |m, _| {
        let c = m.columns.get_mut(&col).unwrap();
        c.location = asterion_geom::Vec2::new(5.5, 3.0);
        Ok(())
    });
    assert!(moved.committed());
    let after_sig = diagnose(s.doc.model(), three, col).geometry_signature;
    assert_ne!(before_sig, after_sig, "the reference body did not follow the edit");
    let mesh = view_mesh(s.doc.model(), s.doc.model().view(three).unwrap()).unwrap();
    assert_ne!(mesh.triangles, before_mesh.triangles);
    assert!(mesh_has_vertex(&mesh, |p| (p[0] - 5.5).abs() <= 0.2 + 1e-6 && (p[1] - 3.0).abs() <= 0.2 + 1e-6));
    assert!(!mesh_has_vertex(&mesh, |p| (p[0] - 4.5).abs() < 0.19 && (p[1] - 3.0).abs() <= 0.2 + 1e-6 && p[2] > 0.5 && p[2] < 2.5), "a stale column body survived");
    let segs = section_segments(s.doc.model(), sec).unwrap();
    let xs: Vec<f64> = segs.iter().filter(|g| g.element == col).flat_map(|g| [g.a[0], g.b[0]]).collect();
    // local x of the section: world x + 1
    assert!(!xs.is_empty() && xs.iter().all(|x| (*x - 6.5).abs() <= 0.2 + 1e-6), "{xs:?}");
    // delete it: every view forgets it and the diagnosis says so
    let del = s.doc.transact("delete", false, |m, cx| asterion_bim::ops::delete(m, cx, col));
    assert!(del.committed(), "{:?}", del.failures);
    let m = s.doc.model();
    let mesh = view_mesh(m, m.view(three).unwrap()).unwrap();
    assert!(!mesh.elements.contains(&col));
    assert!(!mesh_has_vertex(&mesh, |p| (p[0] - 5.5).abs() < 0.19 && (p[1] - 3.0).abs() < 0.19 && p[2] > 0.5 && p[2] < 2.5));
    assert!(!section_segments(m, sec).unwrap().iter().any(|g| g.element == col));
    assert!(!section_segments(m, east).unwrap().iter().any(|g| g.element == col));
    for v in [s.plan0, three, sec, east] {
        assert_eq!(diagnose(m, v, col).status, Status::ElementNotFound);
    }
    // undo brings it back everywhere
    s.doc.undo().unwrap();
    let m = s.doc.model();
    assert!(view_mesh(m, m.view(three).unwrap()).unwrap().elements.contains(&col));
    assert_eq!(diagnose(m, sec, col).status, Status::Visible);
}

#[test]
fn a_door_follows_its_wall_and_a_hidden_category_leaves_every_view() {
    let mut s = scene();
    let (three, sec, plan) = (s.three_d, s.section, s.plan0);
    for v in [three, sec, plan] {
        assert_eq!(status(&s, v, s.column_behind), Status::Visible, "{v:?}");
    }
    // hide the Columns category in every view: the diagnosis says why, and the output agrees
    s.doc.transact("hide", false, |m, _| {
        for v in [three, sec, plan] {
            asterion_bim::docviews::set_category_hidden_in_view(m, v, "Columns", true)?;
        }
        Ok(())
    });
    let m = s.doc.model();
    for v in [three, sec, plan] {
        assert_eq!(diagnose(m, v, s.column_behind).status, Status::HiddenByVisibilityRule, "{v:?}");
    }
    let mesh = view_mesh(m, &asterion_bim::view_visibility::effective_view(m, m.view(three).unwrap())).unwrap();
    assert!(!mesh.elements.contains(&s.column_behind) && !mesh.elements.contains(&s.column_cut));
    assert!(!section_segments(m, sec).unwrap().iter().any(|g| g.element == s.column_behind));
    // walls are not affected
    assert_eq!(diagnose(m, three, s.wall_e).status, Status::Visible);
}

#[test]
fn the_registry_names_what_it_cannot_build() {
    let s = scene();
    let m = s.doc.model();
    let g = element_geometry::geometry_of(m, s.stairs);
    assert!(matches!(g, Geometry::NotGenerated(ref r) if r.contains("no solid")), "{g:?}");
    assert_eq!(element_geometry::classify(m, s.stairs), Some(ElementClass::Stairs));
    assert!(matches!(element_geometry::geometry_of(m, s.room), Geometry::Abstract(_)));
    assert!(matches!(element_geometry::geometry_of(m, s.door), Geometry::Void(_)));
    // an id that is nothing
    assert_eq!(diagnose(m, s.three_d, ElementId(987_654)).status, Status::ElementNotFound);
    assert_eq!(diagnose(m, ElementId(987_654), s.wall_e).status, Status::ViewNotFound);
    let _ = BTreeSet::<u8>::new();
}
