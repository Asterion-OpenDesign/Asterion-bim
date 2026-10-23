//! The Visibility/Graphics dialog's engine side: the category table, the atomic apply, and the proof that a category hidden
//! in a view leaves the plan, the section, the elevation and the 3D view alike (the drawings of one view share ONE hidden
//! set).

mod common;

use asterion_bim::docviews::{self, Ogs};
use asterion_bim::mesh3d::view_mesh;
use asterion_bim::section_graphics::section_segments;
use asterion_bim::view_categories::{self as vc, ApplyRequest, CategoryChange, FilterEntry, Tab};
use asterion_bim::view_visibility::{diagnose, plan_manifest, Status};
use asterion_core::ElementId;
use common::*;
use std::collections::BTreeSet;

fn hide(view: ElementId, s: &mut Scene, cats: &[(&str, bool)]) {
    let req = ApplyRequest {
        categories: cats.iter().map(|(n, h)| CategoryChange { name: (*n).to_owned(), hidden: Some(*h), ..Default::default() }).collect(),
        ..Default::default()
    };
    let out = s.doc.transact("Visibility/Graphics", false, |m, cx| vc::apply(m, cx, view, &req));
    assert!(out.committed(), "{:?}", out.failures);
}

/// The elements a view draws, by view kind: the plan manifest, the 3D mesh, the section's segments.
fn drawn(s: &Scene, view: ElementId) -> BTreeSet<ElementId> {
    let m = s.doc.model();
    if let Some(v) = m.views.get(&view) {
        if v.kind == asterion_bim::model::ViewKind::ThreeD {
            let v = asterion_bim::view_visibility::effective_view(m, v);
            return view_mesh(m, &v).expect("mesh").elements;
        }
        let v = asterion_bim::view_visibility::effective_view(m, v);
        return plan_manifest(m, &v).expect("manifest").keys().copied().collect();
    }
    section_segments(m, view).expect("segments").iter().map(|g| g.element).collect()
}

#[test]
fn a_hidden_category_leaves_the_plan_the_section_the_elevation_and_the_3d_view() {
    let mut s = scene();
    let cases: Vec<(&str, Vec<ElementId>)> = vec![
        ("Walls", vec![s.wall_s, s.wall_e, s.wall_n, s.wall_w]),
        ("Floors", vec![s.floor0, s.floor1]),
        ("Columns", vec![s.column_cut, s.column_behind]),
        ("Roofs", vec![s.roof]),
        ("Ceilings", vec![s.ceiling]),
        ("Toposolid", vec![s.topo]),
    ];
    let views = [s.plan0, s.plan1, s.three_d, s.section, s.elev_east, s.elev_west];
    let mut proofs = 0;
    for (cat, members) in cases {
        for view in views {
            let before = drawn(&s, view);
            let present: Vec<ElementId> = members.iter().copied().filter(|e| before.contains(e)).collect();
            if present.is_empty() {
                continue;
            }
            hide(view, &mut s, &[(cat, true)]);
            let after = drawn(&s, view);
            for e in &present {
                assert!(!after.contains(e), "{cat}: element {e:?} still drawn in view {view:?}");
                assert_eq!(diagnose(s.doc.model(), view, *e).status, Status::HiddenByVisibilityRule, "{cat} {e:?} in {view:?}");
            }
            // nothing else left with it (the other categories stay)
            let others: Vec<&ElementId> = before.iter().filter(|e| !members.contains(e) && vc::category_of(s.doc.model(), **e).map_or(false, |c| c != cat)).filter(|e| {
                // a door or a window is drawn with its host wall in the plan and the 3D view (ASSUMED engine rule, the plan keeps it with
                // the host it is cut in): hiding the host leaves it out of the drawing too
                !(cat == "Walls" && matches!(vc::category_of(s.doc.model(), **e), Some("Doors" | "Windows")))
            }).collect();
            for e in others {
                assert!(after.contains(e), "{cat}: hiding it removed {e:?} (another category) from view {view:?}");
            }
            // another view is not affected
            for other in views.iter().filter(|o| **o != view) {
                if docviews::graphics_owner(s.doc.model(), *other) != docviews::graphics_owner(s.doc.model(), view) {
                    assert!(!vc::is_hidden(&docviews::effective_graphics(s.doc.model(), *other), cat), "{cat} hidden in {other:?} too");
                }
            }
            hide(view, &mut s, &[(cat, false)]);
            assert_eq!(drawn(&s, view), before, "{cat}: showing it again restores the drawing of {view:?}");
            proofs += 1;
        }
    }
    assert!(proofs >= 20, "{proofs} proofs");
}

#[test]
fn doors_windows_rooms_and_stairs_leave_the_plan_by_category() {
    let mut s = scene();
    for (cat, e) in [("Doors", s.door), ("Windows", s.window), ("Rooms", s.room), ("Stairs", s.stairs)] {
        let m = s.doc.model();
        assert_eq!(vc::category_of(m, e), Some(cat));
        assert!(!vc::is_hidden(&docviews::effective_graphics(m, s.plan0), cat));
        hide(s.plan0, &mut s, &[(cat, true)]);
        assert!(docviews::hidden_by_graphics(s.doc.model(), s.plan0, e), "{cat}");
        assert!(docviews::graphics_hidden_set(s.doc.model(), s.plan0).contains(&e), "{cat} is in the set the drawings add to the hidden set");
        assert_eq!(diagnose(s.doc.model(), s.plan0, e).status, Status::HiddenByVisibilityRule);
        // plan 1 is not affected
        assert!(!docviews::hidden_by_graphics(s.doc.model(), s.plan1, e));
        hide(s.plan0, &mut s, &[(cat, false)]);
        assert!(!docviews::hidden_by_graphics(s.doc.model(), s.plan0, e));
    }
}

#[test]
fn the_parts_of_a_curtain_wall_are_two_categories() {
    let mut s = scene();
    let count = |s: &Scene, view: ElementId| view_mesh(s.doc.model(), s.doc.model().views.get(&view).unwrap()).unwrap().solids;
    let all = count(&s, s.three_d);
    hide(s.three_d, &mut s, &[("Curtain Panels", true)]);
    let no_glass = count(&s, s.three_d);
    assert!(no_glass < all, "the glass left the 3D view: {all} -> {no_glass}");
    hide(s.three_d, &mut s, &[("Curtain Wall Mullions", true)]);
    let none = count(&s, s.three_d);
    assert!(none < no_glass, "the mullions left too: {no_glass} -> {none}");
    hide(s.three_d, &mut s, &[("Curtain Panels", false), ("Curtain Wall Mullions", false)]);
    assert_eq!(count(&s, s.three_d), all);
    // the section
    let segs = |s: &Scene| section_segments(s.doc.model(), s.elev_east).unwrap().len();
    let all_e = segs(&s);
    hide(s.elev_east, &mut s, &[("Curtain Wall Mullions", true)]);
    assert!(segs(&s) < all_e, "the mullions left the elevation");
    hide(s.elev_east, &mut s, &[("Curtain Wall Mullions", false)]);
    assert_eq!(segs(&s), all_e);
    // hiding Walls hides the curtain wall as a whole, in the 3D view
    let before = drawn(&s, s.three_d);
    assert!(before.contains(&s.curtain));
    hide(s.three_d, &mut s, &[("Walls", true)]);
    assert!(!drawn(&s, s.three_d).contains(&s.curtain));
}

#[test]
fn a_parent_hides_its_subcategories_and_a_class_hides_a_tab() {
    let mut s = scene();
    let plan = s.plan0;
    let sep = s.doc.transact("sep", false, |m, cx| {
        let l = *m.levels.keys().next().unwrap();
        asterion_bim::ops::create_room_separation(m, cx, l, asterion_geom::Vec2::new(0.0, 2.0), asterion_geom::Vec2::new(6.0, 2.0))
    });
    let sep = sep.value.expect("a room separation line");
    assert_eq!(vc::category_of(s.doc.model(), sep), Some("<Room Separation>"));
    assert_eq!(vc::root_of("<Room Separation>"), "Lines");
    hide(plan, &mut s, &[("Lines", true)]);
    let st = vc::dialog_state(s.doc.model(), plan).unwrap();
    let row = |n: &str| st.categories.iter().find(|c| c.name == n).unwrap().clone();
    assert!(row("Lines").hidden && row("<Room Separation>").hidden, "the apply writes the parent's flag on its subcategories");
    assert!(docviews::hidden_by_graphics(s.doc.model(), plan, sep));
    hide(plan, &mut s, &[("<Room Separation>", false)]);
    // the child alone shown: the parent is still hidden, the element is hidden by it
    let st = vc::dialog_state(s.doc.model(), plan).unwrap();
    assert!(st.categories.iter().find(|c| c.name == "<Room Separation>").unwrap().hidden_by_parent);
    hide(plan, &mut s, &[("Lines", false)]);
    assert!(!docviews::hidden_by_graphics(s.doc.model(), plan, sep));
    // a class
    let req = ApplyRequest { classes: [("model".to_owned(), true)].into_iter().collect(), ..Default::default() };
    assert!(s.doc.transact("c", false, |m, cx| vc::apply(m, cx, plan, &req)).committed());
    assert!(docviews::hidden_by_graphics(s.doc.model(), plan, s.wall_s) && !docviews::hidden_by_graphics(s.doc.model(), s.plan1, s.wall_s));
    let st = vc::dialog_state(s.doc.model(), plan).unwrap();
    assert_eq!(st.hidden_classes, vec!["model"]);
    let req = ApplyRequest { classes: [("model".to_owned(), false)].into_iter().collect(), ..Default::default() };
    assert!(s.doc.transact("c", false, |m, cx| vc::apply(m, cx, plan, &req)).committed());
    assert!(!docviews::hidden_by_graphics(s.doc.model(), plan, s.wall_s));
    let _ = Tab::Model;
}

#[test]
fn the_template_defaults_can_be_shown_and_hidden_again() {
    let mut s = scene();
    let plan = s.plan0;
    let mass = |s: &Scene| vc::dialog_state(s.doc.model(), plan).unwrap().categories.iter().find(|c| c.name == "Mass").unwrap().hidden;
    assert!(mass(&s), "OBSERVED: a new view hides Mass");
    hide(plan, &mut s, &[("Mass", false)]);
    assert!(!mass(&s) && docviews::category_hidden(s.doc.model(), plan, "Mass") == false);
    hide(plan, &mut s, &[("Mass", true)]);
    assert!(mass(&s));
}

#[test]
fn the_apply_is_one_transaction_and_a_refusal_changes_nothing() {
    let mut s = scene();
    let plan = s.plan0;
    let depth = s.doc.undo_names().len();
    let req = ApplyRequest {
        categories: vec![
            CategoryChange { name: "Walls".into(), hidden: Some(true), ..Default::default() },
            CategoryChange { name: "Doors".into(), hidden: Some(true), ogs: Some(Ogs { projection_line_weight: 5, ..Ogs::default() }), ..Default::default() },
            CategoryChange { name: "Windows".into(), hidden: Some(true), ..Default::default() },
        ],
        ..Default::default()
    };
    let out = s.doc.transact("Visibility/Graphics", false, |m, cx| vc::apply(m, cx, plan, &req));
    assert!(out.committed());
    assert_eq!(s.doc.undo_names().len(), depth + 1, "the whole dialog is ONE undo step");
    let st = vc::dialog_state(s.doc.model(), plan).unwrap();
    let row = |n: &str| st.categories.iter().find(|c| c.name == n).unwrap().clone();
    assert!(row("Walls").hidden && row("Doors").hidden && row("Windows").hidden);
    assert_eq!(row("Doors").ogs.as_ref().map(|o| o.projection_line_weight), Some(5));
    // undo reverts all of it, redo restores all of it
    s.doc.undo().unwrap();
    let st = vc::dialog_state(s.doc.model(), plan).unwrap();
    assert!(st.categories.iter().all(|c| !c.hidden || c.default_hidden) && st.categories.iter().all(|c| c.ogs.is_none()));
    s.doc.redo().unwrap();
    let st = vc::dialog_state(s.doc.model(), plan).unwrap();
    assert!(st.categories.iter().find(|c| c.name == "Walls").unwrap().hidden);
    s.doc.undo().unwrap();
    // a refusal in the middle (an unknown category) rolls the whole request back
    let bad = ApplyRequest {
        categories: vec![
            CategoryChange { name: "Walls".into(), hidden: Some(true), ..Default::default() },
            CategoryChange { name: "Nonsense".into(), hidden: Some(true), ..Default::default() },
        ],
        ..Default::default()
    };
    let depth = s.doc.undo_names().len();
    let out = s.doc.transact("Visibility/Graphics", false, |m, cx| vc::apply(m, cx, plan, &bad));
    assert!(!out.committed());
    assert_eq!(s.doc.undo_names().len(), depth);
    assert!(!docviews::category_hidden(s.doc.model(), plan, "Walls"), "the first change was rolled back with the refusal");
    // a value out of range
    for ogs in [Ogs { projection_line_weight: 99, ..Ogs::default() }, Ogs { surface_transparency: 101, ..Ogs::default() }, Ogs { detail_level: 7, ..Ogs::default() }] {
        let req = ApplyRequest { categories: vec![CategoryChange { name: "Walls".into(), ogs: Some(ogs), ..Default::default() }], ..Default::default() };
        assert!(!s.doc.transact("x", false, |m, cx| vc::apply(m, cx, plan, &req)).committed());
    }
    // Grids cannot be overridden and the dialog says so
    let req = ApplyRequest { categories: vec![CategoryChange { name: "Grids".into(), ogs: Some(Ogs { projection_line_weight: 3, ..Ogs::default() }), ..Default::default() }], ..Default::default() };
    assert!(!s.doc.transact("x", false, |m, cx| vc::apply(m, cx, plan, &req)).committed());
    let st = vc::dialog_state(s.doc.model(), plan).unwrap();
    let grids = st.categories.iter().find(|c| c.name == "Grids").unwrap();
    assert!(!grids.can_override && grids.can_hide && grids.locked_reason.contains("OQ-GRID-OVERRIDE"));
}

#[test]
fn a_view_with_a_template_reads_the_templates_values_and_refuses_edits() {
    let mut s = scene();
    let plan = s.plan0;
    hide(plan, &mut s, &[("Doors", true)]);
    let t = s.doc.transact("t", false, |m, cx| docviews::create_template(m, cx, plan, Some("T")));
    let tpl = t.value.expect("template");
    assert!(s.doc.transact("a", false, |m, _| docviews::set_view_template(m, s.plan1, Some(tpl))).committed());
    let st = vc::dialog_state(s.doc.model(), s.plan1).unwrap();
    assert!(st.templated && st.template == tpl.0 && st.template_name == "T");
    assert!(st.categories.iter().all(|c| !c.can_hide && !c.can_override && !c.locked_reason.is_empty()));
    assert!(st.categories.iter().find(|c| c.name == "Doors").unwrap().hidden, "the view reads the template's graphics");
    let req = ApplyRequest { categories: vec![CategoryChange { name: "Walls".into(), hidden: Some(true), ..Default::default() }], ..Default::default() };
    assert!(!s.doc.transact("x", false, |m, cx| vc::apply(m, cx, s.plan1, &req)).committed());
    let req = ApplyRequest { categories: vec![CategoryChange { name: "Walls".into(), ogs: Some(Ogs { halftone: true, ..Ogs::default() }), ..Default::default() }], ..Default::default() };
    assert!(!s.doc.transact("x", false, |m, cx| vc::apply(m, cx, s.plan1, &req)).committed(), "overrides are controlled by the template too");
    // the template itself is edited freely (the dialog on a template)
    assert!(s.doc.transact("x", false, |m, cx| vc::apply(m, cx, tpl, &req)).committed());
    let st = vc::dialog_state(s.doc.model(), s.plan1).unwrap();
    assert!(st.categories.iter().find(|c| c.name == "Walls").unwrap().ogs.as_ref().map_or(false, |o| o.halftone), "and the view that uses it reads the change");
}

#[test]
fn filters_keep_their_order_flags_and_overrides() {
    let mut s = scene();
    let plan = s.plan0;
    let mk = |s: &mut Scene, name: &str| {
        s.doc
            .transact("f", false, |m, cx| {
                docviews::create_filter(m, cx, name, vec!["Walls".into()], docviews::FilterRule { param: "comments".into(), kind: "equals".into(), text: Some(name.into()), number: None })
            })
            .value
            .expect("filter")
    };
    let (a, b) = (mk(&mut s, "A"), mk(&mut s, "B"));
    let req = ApplyRequest {
        filters: Some(vec![
            FilterEntry { filter: b.0, visible: false, enabled: true, ogs: Ogs::default() },
            FilterEntry { filter: a.0, visible: true, enabled: false, ogs: Ogs { projection_line_weight: 4, ..Ogs::default() } },
        ]),
        ..Default::default()
    };
    assert!(s.doc.transact("Visibility/Graphics", false, |m, cx| vc::apply(m, cx, plan, &req)).committed());
    let st = vc::dialog_state(s.doc.model(), plan).unwrap();
    assert_eq!(st.filters.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["B", "A"]);
    assert!(!st.filters[0].visible && st.filters[0].enabled && st.filters[1].visible && !st.filters[1].enabled);
    assert_eq!(st.filters[1].ogs.projection_line_weight, 4);
    assert_eq!(st.document_filters.len(), 2);
    // a duplicate entry is refused, nothing changes
    let dup = ApplyRequest { filters: Some(vec![FilterEntry { filter: a.0, visible: true, enabled: true, ogs: Ogs::default() }, FilterEntry { filter: a.0, visible: true, enabled: true, ogs: Ogs::default() }]), ..Default::default() };
    assert!(!s.doc.transact("x", false, |m, cx| vc::apply(m, cx, plan, &dup)).committed());
    assert_eq!(vc::dialog_state(s.doc.model(), plan).unwrap().filters.len(), 2);
    // removing a filter is a shorter list
    let one = ApplyRequest { filters: Some(vec![]), ..Default::default() };
    assert!(s.doc.transact("x", false, |m, cx| vc::apply(m, cx, plan, &one)).committed());
    assert!(vc::dialog_state(s.doc.model(), plan).unwrap().filters.is_empty());
}

#[test]
fn the_stored_override_is_laid_over_by_a_subcategory_and_read_back_whole() {
    let mut s = scene();
    let plan = s.plan0;
    let ogs = Ogs {
        projection_line_weight: 6,
        projection_line_color: Some([255, 0, 0]),
        projection_line_pattern: Some("Dash".into()),
        surface_foreground_pattern: Some("Concrete".into()),
        surface_foreground_color: Some([10, 20, 30]),
        surface_transparency: 40,
        cut_line_weight: 9,
        cut_foreground_pattern: Some("Crosshatch".into()),
        halftone: true,
        detail_level: 3,
        ..Ogs::default()
    };
    let req = ApplyRequest { categories: vec![CategoryChange { name: "Walls".into(), ogs: Some(ogs.clone()), ..Default::default() }], ..Default::default() };
    assert!(s.doc.transact("Visibility/Graphics", false, |m, cx| vc::apply(m, cx, plan, &req)).committed());
    let st = vc::dialog_state(s.doc.model(), plan).unwrap();
    assert_eq!(st.categories.iter().find(|c| c.name == "Walls").unwrap().ogs.as_ref(), Some(&ogs));
    let eff = docviews::effective_overrides(s.doc.model(), plan, s.wall_s);
    assert_eq!((eff.projection_line_weight, eff.cut_line_weight, eff.surface_transparency, eff.detail_level, eff.halftone), (6, 9, 40, 3, true));
    // the other plan has none
    assert_eq!(docviews::effective_overrides(s.doc.model(), s.plan1, s.wall_s).projection_line_weight, -1);
    // a reset clears it
    let req = ApplyRequest { categories: vec![CategoryChange { name: "Walls".into(), reset_ogs: true, ..Default::default() }], ..Default::default() };
    assert!(s.doc.transact("x", false, |m, cx| vc::apply(m, cx, plan, &req)).committed());
    assert!(vc::dialog_state(s.doc.model(), plan).unwrap().categories.iter().find(|c| c.name == "Walls").unwrap().ogs.is_none());
}

#[test]
fn the_counts_name_what_the_document_holds() {
    let s = scene();
    let st = vc::dialog_state(s.doc.model(), s.plan0).unwrap();
    let n = |c: &str| st.categories.iter().find(|r| r.name == c).unwrap().elements;
    assert_eq!(n("Doors"), 1);
    assert_eq!(n("Windows"), 1);
    assert!(n("Walls") >= 5, "four walls and the curtain wall");
    assert_eq!(n("Rooms"), 1);
    assert_eq!(n("Stairs"), 1);
    assert_eq!(n("Curtain Panels"), 1);
    assert_eq!(n("Mass"), 0);
    assert_eq!(st.disciplines, vec!["Architecture"]);
    // the dialog's rows are the table's, in order
    assert_eq!(st.categories.len(), vc::TABLE.len());
}

#[test]
fn a_category_override_reaches_the_plan_the_section_and_the_3d_mesh() {
    use asterion_bim::override_style::{decode, STYLED};
    use asterion_bim::plan_graphics::{flatten, flatten_styled, plan_primitives};
    let mut s = scene();
    let plan = s.plan0;
    let view = s.doc.model().views.get(&plan).unwrap().clone();
    let ev = asterion_bim::view_visibility::effective_view(s.doc.model(), &view);
    let state = asterion_bim::views::view_state_of(&s.doc, &ev).unwrap();
    let prims = plan_primitives(s.doc.model(), &ev, &state);
    let plain = flatten(&prims);
    assert!(plain.chunks(5).all(|c| c[4] < STYLED), "an unstyled view carries no override class");
    // a view with no override has no styler at all (the quick path)
    assert!(asterion_bim::override_style::ViewStyler::new(s.doc.model(), plan).is_none());
    let req = ApplyRequest {
        categories: vec![CategoryChange {
            name: "Walls".into(),
            ogs: Some(Ogs { projection_line_weight: 6, cut_line_weight: 9, cut_line_color: Some([255, 0, 0]), cut_line_pattern: Some("Dash".into()), halftone: true, ..Ogs::default() }),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert!(s.doc.transact("Visibility/Graphics", false, |m, cx| vc::apply(m, cx, plan, &req)).committed());
    let styler = asterion_bim::override_style::ViewStyler::new(s.doc.model(), plan).expect("the view overrides now");
    let styled = flatten_styled(&prims, Some(&styler));
    assert_eq!(styled.len(), plain.len(), "styling changes classes, never the geometry");
    let mut walls = 0;
    for (a, b) in plain.chunks(5).zip(styled.chunks(5)) {
        assert_eq!(a[..4], b[..4]);
    }
    for (p, c) in prims.iter().zip(plain.chunks(5)) {
        // the walls' segments carry the override; the others (grids) are as they were
        let cat = vc::category_of(s.doc.model(), p.element);
        let sc = flatten_styled(std::slice::from_ref(p), Some(&styler));
        if cat == Some("Walls") && c[4] < 3.0 {
            let d = decode(sc[4]).expect("a wall segment is styled");
            walls += 1;
            assert!(d.halftone);
            if (c[4] - 2.0).abs() < 1e-3 {
                assert_eq!((d.pen, d.pattern, d.rgb4, d.base), (9, 1, 0xF00, 2), "the cut side");
            } else {
                assert_eq!((d.pen, d.pattern, d.rgb4), (0, 0, 0), "the projection side keeps the class's own colour");
            }
        } else if cat != Some("Walls") {
            assert_eq!(sc[4], c[4], "{cat:?} is not overridden");
        }
    }
    assert!(walls > 0, "the plan has wall segments");
    // the section: the segments of the walls carry it too, the others do not
    let segs = section_segments(s.doc.model(), s.section).unwrap();
    s.doc.transact("Visibility/Graphics", false, |m, cx| vc::apply(m, cx, s.section, &req)).value.unwrap();
    let flat = asterion_bim::section_graphics::flatten_in(s.doc.model(), s.section, &segs);
    let styled_walls = segs.iter().zip(flat.chunks(5)).filter(|(g, c)| vc::category_of(s.doc.model(), g.element) == Some("Walls") && c[4] >= STYLED).count();
    let styled_others = segs.iter().zip(flat.chunks(5)).filter(|(g, c)| vc::category_of(s.doc.model(), g.element) != Some("Walls") && c[4] >= STYLED).count();
    assert!(styled_walls > 0 && styled_others == 0, "section: walls {styled_walls}, others {styled_others}");
    // the 3D mesh: the surface colour and the halftone tint the faces
    let mesh_plain = view_mesh(s.doc.model(), s.doc.model().views.get(&s.three_d).unwrap()).unwrap();
    let req3 = ApplyRequest {
        categories: vec![CategoryChange { name: "Floors".into(), ogs: Some(Ogs { surface_foreground_color: Some([255, 0, 0]), ..Ogs::default() }), ..Default::default() }],
        ..Default::default()
    };
    s.doc.transact("Visibility/Graphics", false, |m, cx| vc::apply(m, cx, s.three_d, &req3)).value.unwrap();
    let mesh_red = view_mesh(s.doc.model(), s.doc.model().views.get(&s.three_d).unwrap()).unwrap();
    assert_eq!(mesh_plain.triangles.len(), mesh_red.triangles.len());
    let reds = |m: &asterion_bim::mesh3d::Mesh3d| m.triangles.chunks_exact(9).filter(|v| (v[6] - 1.0).abs() < 1e-6 && v[7].abs() < 1e-6 && v[8].abs() < 1e-6).count();
    assert_eq!(reds(&mesh_plain), 0);
    assert!(reds(&mesh_red) > 0, "the floor faces are drawn in the override colour");
}

#[test]
fn a_filter_that_selects_one_wall_of_a_joined_group_colours_only_that_wall() {
    use asterion_bim::override_style::{decode, STYLED};
    use asterion_bim::plan_graphics::{flatten_styled, plan_primitives};
    let mut s = scene();
    let plan = s.plan0;
    // the comments of the south wall name it for the filter; the three other walls of the joined rectangle do not match
    s.doc.transact("c", false, |m, _| {
        m.docs.comments.insert(s.wall_s, "red".into());
        Ok::<(), asterion_core::Failure>(())
    });
    let flt = s
        .doc
        .transact("f", false, |m, cx| {
            docviews::create_filter(m, cx, "RedWall", vec!["Walls".into()], docviews::FilterRule { param: "comments".into(), kind: "equals".into(), text: Some("red".into()), number: None })
        })
        .value
        .expect("filter");
    let req = ApplyRequest {
        filters: Some(vec![FilterEntry { filter: flt.0, visible: true, enabled: true, ogs: Ogs { projection_line_color: Some([255, 0, 0]), cut_line_color: Some([255, 0, 0]), ..Ogs::default() } }]),
        ..Default::default()
    };
    assert!(s.doc.transact("Visibility/Graphics", false, |m, cx| vc::apply(m, cx, plan, &req)).committed());
    let view = s.doc.model().views.get(&plan).unwrap().clone();
    let ev = asterion_bim::view_visibility::effective_view(s.doc.model(), &view);
    let state = asterion_bim::views::view_state_of(&s.doc, &ev).unwrap();
    let prims = plan_primitives(s.doc.model(), &ev, &state);
    let styler = asterion_bim::override_style::ViewStyler::new(s.doc.model(), plan).expect("the filter overrides");
    let flat = flatten_styled(&prims, Some(&styler));
    // the wall primitives come first in the list and are one segment each, so primitive i is segment i of the buffer
    let wall_prims: Vec<usize> = prims
        .iter()
        .enumerate()
        .filter(|(_, p)| matches!(p.shape, asterion_bim::plan_graphics::Shape::Segment(..)) && s.doc.model().walls.contains_key(&p.element))
        .map(|(i, _)| i)
        .collect();
    let red = wall_prims.iter().filter(|i| flat[**i * 5 + 4] >= STYLED && decode(flat[**i * 5 + 4]).unwrap().rgb4 == 0xF00).count();
    assert!(red > 0 && red < wall_prims.len(), "{red} red wall segments of {}: the wall the filter selects is coloured and the others are not", wall_prims.len());
}

#[test]
fn an_override_by_element_is_one_transaction_for_all_the_elements_and_draws_only_them() {
    use asterion_bim::override_style::{decode, STYLED};
    use asterion_bim::plan_graphics::{flatten_styled, plan_primitives};
    let mut s = scene();
    let plan = s.plan0;
    let st = vc::element_state(s.doc.model(), plan, s.wall_s).unwrap();
    assert!(st.can_override && st.ogs.is_none() && st.category == Some("Walls") && !st.hidden);
    let red = Ogs { projection_line_color: Some([255, 0, 0]), cut_line_color: Some([255, 0, 0]), cut_line_weight: 8, ..Ogs::default() };
    let depth = s.doc.undo_names().len();
    let out = s.doc.transact("Override Graphics in View", false, |m, cx| vc::apply_elements(m, cx, plan, &[s.wall_s, s.wall_n], Some(red.clone())));
    assert_eq!(out.value, Some(2));
    assert_eq!(s.doc.undo_names().len(), depth + 1, "two elements, one undo step");
    assert_eq!(vc::element_state(s.doc.model(), plan, s.wall_n).unwrap().ogs.as_ref(), Some(&red));
    // only those two walls are drawn red in the plan
    let view = s.doc.model().views.get(&plan).unwrap().clone();
    let ev = asterion_bim::view_visibility::effective_view(s.doc.model(), &view);
    let state = asterion_bim::views::view_state_of(&s.doc, &ev).unwrap();
    let prims = plan_primitives(s.doc.model(), &ev, &state);
    let styler = asterion_bim::override_style::ViewStyler::new(s.doc.model(), plan).expect("the view overrides");
    let mut red_elements = std::collections::BTreeSet::new();
    let mut plain_walls = 0;
    for p in prims.iter().filter(|p| s.doc.model().walls.contains_key(&p.element)) {
        let c = flatten_styled(std::slice::from_ref(p), Some(&styler))[4];
        if c >= STYLED && decode(c).unwrap().rgb4 == 0xF00 {
            red_elements.insert(p.element);
        } else {
            plain_walls += 1;
        }
    }
    assert!(!red_elements.is_empty() && plain_walls > 0, "red {red_elements:?}, plain {plain_walls}");
    // another view has nothing; a reset clears; a refusal changes nothing
    assert!(vc::element_state(s.doc.model(), s.plan1, s.wall_s).unwrap().ogs.is_none());
    assert!(s.doc.transact("x", false, |m, cx| vc::apply_elements(m, cx, plan, &[s.wall_s, s.wall_n], None)).committed());
    assert!(vc::element_state(s.doc.model(), plan, s.wall_s).unwrap().ogs.is_none());
    let before = s.doc.undo_names().len();
    let bad = Ogs { cut_line_weight: 99, ..Ogs::default() };
    assert!(!s.doc.transact("x", false, |m, cx| vc::apply_elements(m, cx, plan, &[s.wall_s], Some(bad))).committed());
    assert!(!s.doc.transact("x", false, |m, cx| vc::apply_elements(m, cx, plan, &[], Some(Ogs::default()))).committed());
    assert_eq!(s.doc.undo_names().len(), before);
    // an element with no listed category, and a view with a template, are refused with the reason
    let st = vc::element_state(s.doc.model(), plan, s.l0).unwrap();
    assert_eq!(st.category, Some("Levels"));
    let none = asterion_core::ElementId(999_999);
    let st = vc::element_state(s.doc.model(), plan, none).unwrap();
    assert!(!st.can_override && st.locked_reason.contains("no category"));
    let tpl = s.doc.transact("t", false, |m, cx| docviews::create_template(m, cx, plan, Some("T"))).value.unwrap();
    assert!(s.doc.transact("a", false, |m, _| docviews::set_view_template(m, s.plan1, Some(tpl))).committed());
    let st = vc::element_state(s.doc.model(), s.plan1, s.wall_s).unwrap();
    assert!(!st.can_override && st.locked_reason.contains("template"));
}
