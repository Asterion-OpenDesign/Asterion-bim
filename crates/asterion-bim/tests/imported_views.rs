//! The test imports the frame of the scene's own section and elevation as a stored box and checks that the imported view
//! draws exactly what the original does (same figures), that an unbounded elevation is framed to the model, that a view
//! with a malformed box says why it has no frame, and that the diagnosis agrees with the drawing.

mod common;

use asterion_bim::document_settings::StoredViewBox;
use asterion_bim::docviews::{self, DocViewKind};
use asterion_bim::import_ops::{import_view, ImportedView};
use asterion_bim::model::{ViewKind, ViewRange};
use asterion_bim::section_graphics;
use asterion_bim::view_visibility;
use asterion_core::ElementId;
use common::*;

/// The stored box that gives the same frame as a view made in ASTERION: basis = (right, up, away from the viewer), the
/// side-0 offset the maximum and the side-1 offset the minimum of each axis, the far bound active and the near bound not.
fn box_of(dv: &docviews::DocView, bounded_sides: bool) -> StoredViewBox {
    StoredViewBox {
        origin: dv.origin,
        basis: [dv.x, dv.y, [-dv.z[0], -dv.z[1], -dv.z[2]]],
        offset: [[dv.crop_max[0], dv.crop_min[0]], [dv.crop_max[1], dv.crop_min[1]], [-dv.crop_min[2], 0.0]],
        active: [[bounded_sides; 2], [bounded_sides; 2], [bounded_sides, false]],
        on: true,
        perspective: None,
    }
}

fn import_as(s: &mut Scene, name: &str, kind: ViewKind, stored: StoredViewBox, crop_active: bool) -> ElementId {
    let out = s.doc.transact("import", false, |m, cx| {
        let id = import_view(m, cx, ImportedView { name: name.into(), kind, scale: 100, gen_level: None, range: ViewRange::default(), crop_active, crop_visible: true })?;
        m.settings.view_boxes.insert(id, stored);
        Ok(id)
    });
    out.value.unwrap_or_else(|| panic!("not committed: {:?}", out.failures))
}

fn same_segments(a: &[section_graphics::Seg], b: &[section_graphics::Seg]) -> bool {
    let key = |s: &section_graphics::Seg| (s.element.0, [s.a[0], s.a[1], s.b[0], s.b[1]].map(|v| (v * 1e6).round() as i64), (s.class * 10.0) as i32);
    let mut ka: Vec<_> = a.iter().map(key).collect();
    let mut kb: Vec<_> = b.iter().map(key).collect();
    ka.sort();
    kb.sort();
    ka == kb
}

#[test]
fn an_imported_section_draws_the_figures_of_the_section_it_is_the_frame_of() {
    let mut s = scene();
    let original = s.doc.model().docs.views[&s.section].clone();
    let id = import_as(&mut s, "Imported section", ViewKind::Section, box_of(&original, true), true);
    let m = s.doc.model();
    let derived = docviews::doc_view(m, id).expect("the stored box gives a frame");
    assert_eq!(derived.kind, DocViewKind::Section);
    for k in 0..3 {
        assert!((derived.origin[k] - original.origin[k]).abs() < 1e-9 && (derived.x[k] - original.x[k]).abs() < 1e-9, "origin / right");
        assert!((derived.y[k] - original.y[k]).abs() < 1e-9 && (derived.z[k] - original.z[k]).abs() < 1e-9, "up / toward the viewer");
        assert!((derived.crop_min[k] - original.crop_min[k]).abs() < 1e-9 && (derived.crop_max[k] - original.crop_max[k]).abs() < 1e-9, "crop box");
    }
    let a = section_graphics::section_segments(m, s.section).unwrap();
    let b = section_graphics::section_segments(m, id).unwrap();
    assert!(a.len() > 20, "the original draws the scene");
    assert!(same_segments(&a, &b), "the imported view draws {} segments, the original {}", b.len(), a.len());
    // picking and the selection outline work in the imported view as in the original
    let wall = section_graphics::section_pick(m, id, [0.0, 1.0], 1e-3);
    assert_eq!(wall, section_graphics::section_pick(m, s.section, [0.0, 1.0], 1e-3));
    assert!(wall.is_some(), "something is under the point (0, 1)");
    assert!(!section_graphics::section_outline(m, id, s.wall_e).is_empty());
    // the diagnosis names the same status
    for e in [s.wall_e, s.wall_n, s.column_cut, s.curtain, s.stairs] {
        let (da, db) = (view_visibility::diagnose(m, s.section, e), view_visibility::diagnose(m, id, e));
        assert_eq!(da.status, db.status, "the diagnosis of {e:?}: {} vs {}", da.rule, db.rule);
    }
}

#[test]
fn an_imported_elevation_without_a_crop_box_is_framed_to_the_model() {
    let mut s = scene();
    let original = s.doc.model().docs.views[&s.elev_west].clone();
    // no bound is active: an elevation without a crop box is unbounded; the engine frames it to the model
    let id = import_as(&mut s, "Imported elevation", ViewKind::Elevation, box_of(&original, false), false);
    let m = s.doc.model();
    let dv = docviews::doc_view(m, id).expect("a frame");
    assert_eq!(dv.kind, DocViewKind::Elevation);
    let (lo, hi) = docviews::model_extent_local(m, &dv).expect("the model has walls");
    for k in 0..3 {
        assert!(dv.crop_min[k] <= lo[k] + 1e-9 || k == 2 && dv.crop_min[2] <= lo[2] + 1e-9, "axis {k} min covers the model: {} vs {}", dv.crop_min[k], lo[k]);
        assert!(dv.crop_max[k] >= hi[k] - 1e-9 || k == 2, "axis {k} max covers the model");
    }
    // it draws: the walls that stand in front of its view direction are in the line work
    let segs = section_graphics::section_segments(m, id).unwrap();
    assert!(!segs.is_empty(), "an unbounded elevation of a building is not blank");
    assert!(segs.iter().any(|g| g.element == s.wall_w || g.element == s.wall_n || g.element == s.wall_s), "a wall of the building is drawn");
}

#[test]
fn a_view_whose_box_is_not_a_frame_says_why_it_cannot_be_drawn() {
    let mut s = scene();
    let original = s.doc.model().docs.views[&s.section].clone();
    let mut skew = box_of(&original, true);
    skew.basis[1] = skew.basis[0]; // up = right: not an orthonormal basis
    let bad = import_as(&mut s, "Skew", ViewKind::Section, skew, true);
    // and one whose file stored no box at all
    let out = s.doc.transact("import", false, |m, cx| {
        import_view(m, cx, ImportedView { name: "No box".into(), kind: ViewKind::Elevation, scale: 100, gen_level: None, range: ViewRange::default(), crop_active: false, crop_visible: false })
    });
    let none = out.value.unwrap();
    // an imported view carries the facts the file stored (the browser's family, ...): its source entry says it came from a file
    let src = asterion_bim::document_settings::ViewSource { source_id: 1, type_source_id: None, type_name: None, family: Some(5), template_source_id: None, discipline: 1, detail_level_raw: 0 };
    s.doc.transact("source", false, |m, _| {
        m.settings.view_sources.insert(none, src);
        Ok(())
    });
    let plain = s.doc.transact("plain", false, |m, cx| {
        import_view(m, cx, ImportedView { name: "Plain".into(), kind: ViewKind::Section, scale: 100, gen_level: None, range: ViewRange::default(), crop_active: false, crop_visible: false })
    });
    let plain = plain.value.unwrap();
    let m = s.doc.model();
    assert!(docviews::doc_view(m, bad).is_none() && docviews::doc_view(m, none).is_none());
    let (g1, g2) = (docviews::imported_frame_gap(m, bad).expect("a reason"), docviews::imported_frame_gap(m, none).expect("a reason"));
    assert!(g1.contains("orthonormal") && g2.contains("no readable viewer box"), "{g1} / {g2}");
    // a view the engine's New View made (no source, no box) says it was made without a frame
    assert!(docviews::imported_frame_gap(m, plain).unwrap().contains("made without a cut line"));
    // a framed view, a plan and a 3D view have no gap
    assert!(docviews::imported_frame_gap(m, s.plan0).is_none() && docviews::imported_frame_gap(m, s.three_d).is_none());
    // the diagnosis of an element in a frameless view names the gap, not "cropped" or "hidden"
    let d = view_visibility::diagnose(m, none, s.wall_e);
    assert_eq!(d.status, view_visibility::Status::UnsupportedElementType);
    assert!(d.rule.contains("no drawing frame"), "{}", d.rule);
}

#[test]
fn the_model_names_the_elements_it_has_no_body_for() {
    let s = scene();
    let gaps = asterion_bim::element_geometry::model_gaps(s.doc.model());
    // the scene's stairs and run have a body now: no gap names them; what the registry cannot build is still listed (nothing silent)
    assert!(!gaps.iter().any(|(w, _, _)| w == "stairs" || w == "stair runs"), "{gaps:?}");
}

/// The cheap prefilter of the collector (`near_view`) must never drop what the full test keeps: every view of the scene, with the filter
/// on and off, draws the same figures.
#[test]
fn the_prefilter_of_the_collector_changes_nothing() {
    let s = scene();
    let m = s.doc.model();
    for view in [s.section, s.elev_west, s.elev_east] {
        let (a, b) = (section_graphics::section_figures_with(m, view, true).unwrap(), section_graphics::section_figures_with(m, view, false).unwrap());
        assert_eq!(a.len(), b.len(), "view {view:?}");
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!((x.element, x.cut, x.rect, x.holes.len()), (y.element, y.cut, y.rect, y.holes.len()));
        }
        assert!(!a.is_empty());
    }
}
