//! Tests of the documentation rules that follow specification rules (Spec FILTER-03).

use crate::docviews::{self, FilterRule, Ogs};
use crate::model::ViewKind;
use crate::{ops, sheets, BimDocument};
use asterion_core::ElementId;
use asterion_geom::Vec2;

fn wall_doc() -> (BimDocument, ElementId, ElementId, ElementId) {
    let mut d = crate::new_document();
    let (mut plan, mut wall, mut wt) = (ElementId(0), ElementId(0), ElementId(0));
    d.transact("setup", false, |m, cx| {
        let l = ops::create_level(m, cx, "L0", 0.0)?;
        plan = ops::create_view(m, cx, "P0", ViewKind::FloorPlan, Some(l), None)?;
        let cs = crate::CompoundStructure::new(vec![crate::Layer { width: 0.2, function: crate::LayerFunction::Structure }], 0, 0)?;
        wt = ops::create_wall_type(m, cx, "T", cs)?;
        wall = ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l, start: Vec2::new(0.0, 0.0), end: Vec2::new(6.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false })?;
        Ok(())
    });
    (d, plan, wall, wt)
}

#[test]
fn the_filter_nearest_the_start_wins_the_fields_it_sets_and_the_element_overrides_last() {
    let (mut d, plan, wall, _) = wall_doc();
    let out = d.transact("filters", false, |m, cx| {
        let rule = || FilterRule { param: "type_name".into(), kind: "equals".into(), text: Some("T".into()), number: None };
        let f1 = docviews::create_filter(m, cx, "F1", vec!["Walls".into()], rule())?;
        let f2 = docviews::create_filter(m, cx, "F2", vec!["Walls".into()], rule())?;
        docviews::attach_filter(m, plan, f1, Some(true), Some(Ogs { projection_line_weight: 5, ..Ogs::default() }))?;
        docviews::attach_filter(m, plan, f2, Some(true), Some(Ogs { projection_line_weight: 9, cut_line_weight: 7, ..Ogs::default() }))?;
        docviews::set_category_overrides(m, plan, "Walls", Ogs { cut_line_weight: 3, halftone: true, ..Ogs::default() })?;
        Ok(())
    });
    assert!(out.committed(), "{:?}", out.failures);
    let o = docviews::effective_overrides(d.model(), plan, wall);
    // F1 (first in the list) wins its projection weight; F2 sets the cut weight F1 leaves unset; the filters supersede the
    // category's cut weight and halftone (halftone is always copied)
    assert_eq!((o.projection_line_weight, o.cut_line_weight, o.halftone), (5, 7, false));
    assert!(d.transact("el", false, |m, _| docviews::set_element_graphics_overrides(m, plan, wall, Ogs { projection_line_weight: 2, ..Ogs::default() })).committed());
    assert_eq!(docviews::effective_overrides(d.model(), plan, wall).projection_line_weight, 2);
}

#[test]
fn a_viewport_needs_a_sheet_first_and_revisions_number_by_position() {
    let (mut d, plan, _, _) = wall_doc();
    let bad = d.transact("vp", false, |m, cx| sheets::create_viewport(m, cx, plan, plan, [0.0, 0.0]));
    assert_eq!(bad.value, None);
    let mut revs = Vec::new();
    d.transact("revs", false, |m, cx| {
        for _ in 0..3 {
            revs.push(sheets::create_revision(m, cx, None, None, None, None, None));
        }
        Ok(())
    });
    assert_eq!(sheets::revision_sequence(d.model(), revs[2]), Some(4));
    assert!(d.transact("del", false, |m, cx| ops::delete(m, cx, revs[0])).committed());
    assert_eq!(sheets::revision_number(d.model(), revs[2]).as_deref(), Some("3"));
}

#[test]
fn moving_a_reference_plane_end_keeps_its_normal_and_refuses_a_tilt() {
    let mut d = crate::new_document();
    let p = d.transact("p", false, |m, cx| crate::datums::create_ref_plane(m, cx, [0.0; 3], [5.0, 0.0, 0.0], [0.0, 0.0, 1.0], None, Some("RP"))).value.unwrap();
    let n0 = d.model().annot.ref_planes[&p].normal;
    assert!(d.transact("b", false, |m, cx| crate::datums::set_ref_plane(m, cx, p, None, Some([0.0, 3.0, 0.0]), None)).value.is_none(), "tilted out of the plane");
    assert!(d.transact("same", false, |m, cx| crate::datums::set_ref_plane(m, cx, p, None, Some([5.0, 0.0, 0.0]), None)).value.is_none());
    assert!(d.transact("ok", false, |m, cx| crate::datums::set_ref_plane(m, cx, p, None, Some([-2.0, 0.0, 0.0]), None)).committed());
    assert_eq!(d.model().annot.ref_planes[&p].normal, n0);
}

#[test]
fn a_duplicate_type_name_is_checked_and_trimmed() {
    let mut d = crate::new_document();
    let src = d
        .transact("t", false, |m, cx| ops::create_hosted_type(m, cx, "D1", crate::model::HostedKind::Door, 0.9, 2.1, 0.05, 1.0, 2.2, 0.0, 0.0))
        .value
        .unwrap();
    assert!(d.transact("dup", false, |m, cx| ops::duplicate_hosted_type(m, cx, src, "D1")).value.is_none());
    assert!(d.transact("bad", false, |m, cx| ops::duplicate_hosted_type(m, cx, src, "D<2>")).value.is_none());
    let c = d.transact("ok", false, |m, cx| ops::duplicate_hosted_type(m, cx, src, "  D2 ")).value.unwrap();
    assert_eq!(d.model().hosted_types[&c].name, "D2");
}

#[test]
fn a_radial_array_angle_above_pi_turns_the_other_way_and_blank_text_makes_no_note() {
    let (mut d, plan, wall, _) = wall_doc();
    // 3 pi / 2 is arrayed as -pi / 2: the copy of a wall along +x about the origin lands along -y
    let pairs = d
        .transact("arr", false, |m, cx| {
            crate::modify::array_radial(m, cx, &[wall], 3, Vec2::new(0.0, 0.0), 1.5 * std::f64::consts::PI, crate::modify::ArrayAnchor::Second)
        })
        .value
        .unwrap();
    let c = &d.model().walls[&pairs[0].1];
    assert!(c.end.x.abs() < 1e-9 && (c.end.y + 6.0).abs() < 1e-9, "{:?}", c.end);
    // An angle of 1e-12 is not valid
    assert!(d.transact("tiny", false, |m, cx| crate::modify::array_radial(m, cx, &[wall], 3, Vec2::new(0.0, 0.0), 1e-12, crate::modify::ArrayAnchor::Second)).value.is_none());
    // White space only makes no text note
    let t = d.transact("txt", false, |m, cx| crate::dims::create_text_note(m, cx, plan, [0.0; 3], "  \n ", None, None, crate::annot::HAlign::Left));
    assert!(t.value.is_none());
}

#[test]
fn the_filter_getter_drops_the_lowest_match_from_the_second_match_on() {
    // Spec FILTER-03-(4): visited from the last filter to the first; the second match replaces the first-visited settings;
    // every further match supersedes field by field; the filter nearest the start has the last word
    let (mut d, plan, wall, _) = wall_doc();
    let out = d.transact("filters", false, |m, cx| {
        let rule = || FilterRule { param: "type_name".into(), kind: "equals".into(), text: Some("T".into()), number: None };
        let mut fs = Vec::new();
        for n in ["F1", "F2", "F3"] {
            fs.push(docviews::create_filter(m, cx, n, vec!["Walls".into()], rule())?);
        }
        docviews::attach_filter(m, plan, fs[0], Some(true), Some(Ogs { projection_line_weight: 5, ..Ogs::default() }))?;
        docviews::attach_filter(m, plan, fs[1], Some(true), Some(Ogs { projection_line_weight: 9, cut_line_weight: 7, ..Ogs::default() }))?;
        Ok(fs)
    });
    let fs = out.value.unwrap();
    // two matches: F2 (last) is visited first and dropped; F1 is the result as stored (its cut weight is unset)
    let two = docviews::filter_settings_returned(d.model(), plan, wall).unwrap();
    assert_eq!((two.projection_line_weight, two.cut_line_weight), (5, -1));
    // the drawing path still draws F2's cut weight (the filter overrider pushes every match)
    assert_eq!(docviews::effective_overrides(d.model(), plan, wall).cut_line_weight, 7);
    // three matches (list F1, F2, F3): F3 is visited first and dropped, F2 is the base, F1 supersedes it where it sets a
    // value, so F3's colour never shows in the returned settings
    let red = [255, 0, 0];
    assert!(d.transact("f3", false, |m, _| docviews::attach_filter(m, plan, fs[2], Some(true), Some(Ogs { projection_line_color: Some(red), cut_line_weight: 2, ..Ogs::default() }))).committed());
    let three = docviews::filter_settings_returned(d.model(), plan, wall).unwrap();
    assert_eq!((three.projection_line_weight, three.cut_line_weight, three.projection_line_color), (5, 7, None));
    assert_eq!(docviews::effective_overrides(d.model(), plan, wall).projection_line_color, Some(red));
    // no match: nothing is returned
    let (d2, plan2, wall2, _) = wall_doc();
    assert!(docviews::filter_settings_returned(d2.model(), plan2, wall2).is_none());
}

#[test]
fn material_names_follow_the_create_and_duplicate_checks() {
    // A prohibited character first, then a name in use; the pipe is prohibited, the slash is not
    let mut d = crate::new_document();
    let m1 = d.transact("m", false, |m, cx| crate::docs::add_material(m, cx, "Base")).value.unwrap();
    assert!(d.transact("bar", false, |m, cx| crate::docs::add_material(m, cx, "A|B")).value.is_none());
    assert!(d.transact("slash", false, |m, cx| crate::docs::add_material(m, cx, "A/B")).committed());
    assert!(d.transact("use", false, |m, cx| crate::docs::clone_material(m, cx, m1, "Base")).value.is_none());
    assert!(d.transact("badd", false, |m, cx| crate::docs::clone_material(m, cx, m1, "{x}")).value.is_none());
    // the asset-manager unique name: free names stay, a taken one gets (n) with no space, case-insensitively
    let model = d.model();
    assert_eq!(crate::docs::unique_material_name(model, "Fresh"), "Fresh");
    assert_eq!(crate::docs::unique_material_name(model, "base"), "base(1)");
    let m2 = d.transact("m2", false, |m, cx| crate::docs::add_material(m, cx, "Base(1)")).value.unwrap();
    let _ = m2;
    assert_eq!(crate::docs::unique_material_name(d.model(), "Base"), "Base(2)");
    assert_eq!(crate::docs::unique_material_name(d.model(), "Base(1)"), "Base(2)");
}

#[test]
fn schedule_numbers_round_half_away_and_a_new_schedule_has_the_default_flags() {
    use crate::schedules::{self, RoundMethod};
    assert!((schedules::round_to_accuracy(1.875, 0.01, RoundMethod::Nearest) - 1.88).abs() < 1e-12);
    assert!((schedules::round_to_accuracy(46.5, 1.0, RoundMethod::Nearest) - 47.0).abs() < 1e-12);
    assert!((schedules::round_to_accuracy(-46.5, 1.0, RoundMethod::Nearest) + 47.0).abs() < 1e-12);
    assert!((schedules::round_to_accuracy(1.001, 0.01, RoundMethod::Up) - 1.01).abs() < 1e-12);
    assert!((schedules::round_to_accuracy(1.009, 0.01, RoundMethod::Down) - 1.0).abs() < 1e-12);
    // a value 1e-9 quanta below the half still rounds up; an accuracy below 1e-12 leaves the value alone
    assert!((schedules::round_to_accuracy(0.5 - 5e-10, 1.0, RoundMethod::Nearest) - 1.0).abs() < 1e-12);
    assert_eq!(schedules::round_to_accuracy(0.123456789, 1e-13, RoundMethod::Nearest), 0.123456789);
    // Itemized, title and headers shown, no grand total; a second default name is numbered (from 2), not refused
    let mut d = crate::new_document();
    let s1 = d.transact("s1", false, |m, cx| schedules::add_schedule_view(m, cx, Some("Walls"), None)).value.unwrap();
    let s2 = d.transact("s2", false, |m, cx| schedules::add_schedule_view(m, cx, Some("Walls"), None)).value.unwrap();
    {
        let sch = &d.model().docs.schedules;
        assert_eq!((sch[&s1].name.as_str(), sch[&s2].name.as_str()), ("Walls Schedule", "Walls Schedule 2"), "The counter starts at 2");
        let s = &sch[&s2];
        assert!(s.itemized && s.show_title && s.show_headers && !s.grand_total && !s.grand_total_title_on && !s.grand_total_count && s.fields.is_empty());
    }
    let s3 = d.transact("s3", false, |m, cx| schedules::add_schedule_view(m, cx, Some("Walls"), None)).value.unwrap();
    assert_eq!(d.model().docs.schedules[&s3].name, "Walls Schedule 3");
    assert!(d.transact("dup", false, |m, cx| schedules::add_schedule_view(m, cx, Some("Walls"), Some("Walls Schedule"))).value.is_none(), "a name the caller gives is not made unique");
    assert!(d.transact("cat", false, |m, cx| schedules::add_schedule_view(m, cx, Some("Furniture"), None)).value.is_none());
}

#[test]
fn a_dimension_text_override_keeps_above_and_below_but_drops_prefix_and_suffix() {
    use crate::annot::DimText;
    // Above, prefix, value, suffix, below; an override replaces the value and silences prefix and suffix
    let t = DimText { above: Some("A".into()), prefix: Some("P".into()), suffix: Some("S".into()), below: Some("B".into()), value_override: None };
    let (mut d, plan, wall, _) = wall_doc();
    let _ = (&mut d, plan, wall);
    assert_eq!(crate::dims::compose_for_test(&t, 4.0), "A P 4000 S B");
    let o = DimText { value_override: Some("X".into()), ..t };
    assert_eq!(crate::dims::compose_for_test(&o, 4.0), "A X B");
}

#[test]
fn a_sheet_number_is_trimmed_and_neither_empty_nor_duplicated_and_a_revision_shows_on_its_sheet() {
    let mut d = crate::new_document();
    let s1 = d.transact("s1", false, |m, cx| sheets::create_sheet(m, cx, Some("  A101 "), None, true)).value.unwrap();
    assert_eq!(d.model().docs.sheets[&s1].number, "A101");
    // Empty after trimming, then duplicated
    assert!(d.transact("e", false, |m, cx| sheets::create_sheet(m, cx, Some("   "), None, true)).value.is_none());
    assert!(d.transact("d", false, |m, cx| sheets::create_sheet(m, cx, Some("A101 "), None, true)).value.is_none());
    // a default number is counted up until it is free
    d.transact("s3", false, |m, cx| sheets::create_sheet(m, cx, Some("2"), None, true));
    let s4 = d.transact("s4", false, |m, cx| sheets::create_sheet(m, cx, None, None, true)).value.unwrap();
    assert_eq!(d.model().docs.sheets[&s4].number, "3");
    // The number a sheet shows is the project number, only for its own revisions
    let r = d.transact("r", false, |m, cx| Ok(sheets::create_revision(m, cx, None, None, None, None, None))).value.unwrap();
    assert_eq!(sheets::revision_number_on_sheet(d.model(), s1, r), None);
    d.transact("sr", false, |m, _| sheets::set_sheet_revisions(m, s1, vec![r]));
    assert_eq!(sheets::revision_number_on_sheet(d.model(), s1, r).as_deref(), Some("2"));
    assert_eq!(sheets::revision_number_on_sheet(d.model(), s4, r), None);
}

#[test]
fn templates_and_viewports_follow_the_native_view_types_and_a_revision_has_its_default_texts() {
    // : a sheet-type view takes no template, a drafting view does; a schedule or sheet is not placed in a viewport
    let mut d = crate::new_document();
    let (draft, sheet_view, plan) = d
        .transact("fx", false, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let p = ops::create_view(m, cx, "P0", ViewKind::FloorPlan, Some(l), None)?;
            let dr = ops::create_view(m, cx, "D0", ViewKind::Drafting, None, None)?;
            let sh = ops::create_view(m, cx, "S0", ViewKind::Sheet, None, None)?;
            Ok((dr, sh, p))
        })
        .value
        .unwrap();
    assert!(d.transact("t1", false, |m, cx| docviews::create_template(m, cx, draft, Some("TD"))).committed());
    assert!(d.transact("t2", false, |m, cx| docviews::create_template(m, cx, sheet_view, Some("TS"))).value.is_none());
    let sh = d.transact("s", false, |m, cx| sheets::create_sheet(m, cx, Some("A1"), None, true)).value.unwrap();
    assert!(!sheets::can_add_view(d.model(), sh, sheet_view));
    assert!(sheets::can_add_view(d.model(), sh, plan));
    // : "Revision n" and "Date n", n the sequence number
    let r = d.transact("r", false, |m, cx| Ok(sheets::create_revision(m, cx, None, None, None, None, None))).value.unwrap();
    let rev = &d.model().docs.revisions[&r];
    assert_eq!((rev.description.as_str(), rev.date.as_str()), ("Revision 2", "Date 2"));
}

#[test]
fn a_dependent_view_reads_the_hidden_categories_and_filters_of_its_primary() {
    let (mut d, plan, wall, _) = wall_doc();
    let dep = d.transact("dep", false, |m, cx| crate::view_dup::duplicate_view(m, cx, plan, crate::view_dup::DuplicateOption::AsDependent)).value.unwrap();
    assert!(d.transact("h", false, |m, _| docviews::set_category_hidden_in_view(m, dep, "Walls", true)).committed());
    assert!(docviews::effective_graphics(d.model(), plan).hidden_categories.contains("Walls"), "written to the primary");
    assert!(docviews::hidden_by_graphics(d.model(), plan, wall) && docviews::hidden_by_graphics(d.model(), dep, wall));
    d.transact("u", false, |m, _| docviews::set_category_hidden_in_view(m, plan, "Walls", false));
    assert!(!docviews::hidden_by_graphics(d.model(), dep, wall));
}

#[test]
fn the_revisions_dialog_rules_follow_rev_02() {
    // Issued lock, reorder is a permutation, merge needs unissued neighbours, delete needs two revisions and an unissued
    // one
    let mut d = crate::new_document();
    let s = d.transact("s", false, |m, cx| sheets::create_sheet(m, cx, Some("A1"), None, true)).value.unwrap();
    let r: Vec<ElementId> = d
        .transact("r", false, |m, cx| Ok((0..3).map(|_| sheets::create_revision(m, cx, None, None, None, None, None)).collect::<Vec<_>>()))
        .value
        .unwrap();
    // an issued revision refuses every parameter edit; the API setter of the flag is not locked
    assert!(d.transact("e", false, |m, _| sheets::edit_revision(m, r[0], sheets::RevisionEdit { description: Some("X"), ..Default::default() })).committed());
    d.transact("i", false, |m, _| sheets::set_revision_issued(m, r[0], true)).value.unwrap();
    assert!(d.transact("e2", false, |m, _| sheets::edit_revision(m, r[0], sheets::RevisionEdit { description: Some("Y"), ..Default::default() })).value.is_none());
    assert!(d.transact("v", false, |m, _| sheets::edit_revision(m, r[1], sheets::RevisionEdit { visibility: Some(3), ..Default::default() })).value.is_none(), "RevisionVisibility has 0..=2");
    assert_eq!(d.model().docs.revisions[&r[1]].visibility, 1);
    // merge: not issued, a neighbour that is not issued
    assert!(!sheets::may_merge_with_next_revision(d.model(), r[0]), "issued");
    assert!(!sheets::can_combine_with_previous(d.model(), r[1]), "its previous revision is issued");
    assert!(sheets::may_merge_with_next_revision(d.model(), r[1]) && !sheets::may_merge_with_next_revision(d.model(), r[2]), "the last has no next");
    assert!(!sheets::can_combine_with_previous(d.model(), r[0]), "the first the engine holds has no previous one");
    // a sheet that showed the merged revision shows the neighbour; the numbers close up
    d.transact("sr", false, |m, _| sheets::set_sheet_revisions(m, s, vec![r[1]])).value.unwrap();
    let into = d.transact("c", false, |m, _| sheets::combine_revision(m, r[1], true)).value.unwrap();
    assert_eq!(into, r[2]);
    assert_eq!(d.model().docs.sheets[&s].revisions, vec![r[2]]);
    assert_eq!(sheets::revision_number(d.model(), r[2]).as_deref(), Some("3"));
    // reorder: a permutation only, the issued flag is not tested; the sequence follows the order
    assert!(d.transact("bad", false, |m, _| sheets::reorder_revisions(m, &[r[0]])).value.is_none());
    assert!(d.transact("ro", false, |m, _| sheets::reorder_revisions(m, &[r[2], r[0]])).committed());
    assert_eq!((sheets::revision_sequence(d.model(), r[2]), sheets::revision_sequence(d.model(), r[0])), (Some(2), Some(3)));
    // delete: not an issued one; with the template's own, two revisions are enough
    assert!(d.transact("d1", false, |m, _| sheets::delete_revision(m, r[0])).value.is_none());
    assert!(d.transact("d2", false, |m, _| sheets::delete_revision(m, r[2])).committed());
}

#[test]
fn an_override_stores_the_fields_the_row_lists_and_a_superseding_one_copies_the_flags_always() {
    // (the stored fields) + Spec FILTER-03: background patterns and colours are copied when set; the visible flags always
    let base = Ogs { surface_background_color: Some([1, 2, 3]), cut_background_pattern: Some("Solid fill".into()), detail_level: 2, ..Ogs::default() };
    let by = Ogs { surface_background_pattern_visible: false, detail_level: 3, ..Ogs::default() };
    let out = docviews::overlay_overrides(&base, &by);
    assert_eq!(out.surface_background_color, Some([1, 2, 3]));
    assert_eq!(out.cut_background_pattern.as_deref(), Some("Solid fill"));
    assert!(!out.surface_background_pattern_visible && out.cut_foreground_pattern_visible);
    assert_eq!(out.detail_level, 2, "a superseding override does not touch the detail level");
    // a background pattern the document does not have is refused like a foreground one
    assert!(docviews::check_ogs(&Ogs { cut_background_pattern: Some("Nope".into()), ..Ogs::default() }).is_err());
}

#[test]
fn a_disabled_filter_hides_and_overrides_nothing_and_an_empty_category_set_passes_every_element() {
    // Spec FILTER-02: a hiding entry is visibility off AND enabled; Spec FILTER-03: the filter overrider skips disabled
    // entries.
    let (mut d, plan, wall, _) = wall_doc();
    let f = d
        .transact("f", false, |m, cx| {
            let f = docviews::create_filter(m, cx, "F", vec![], FilterRule { param: "type_name".into(), kind: "equals".into(), text: Some("T".into()), number: None })?;
            docviews::attach_filter(m, plan, f, Some(false), Some(Ogs { projection_line_weight: 6, ..Ogs::default() }))?;
            Ok(f)
        })
        .value
        .unwrap();
    assert!(docviews::hidden_by_graphics(d.model(), plan, wall), "no categories: the wall passes, visibility off hides it");
    assert_eq!(docviews::effective_overrides(d.model(), plan, wall).projection_line_weight, 6);
    d.transact("off", false, |m, _| docviews::set_filter_enabled(m, plan, f, false)).value.unwrap();
    assert!(!docviews::hidden_by_graphics(d.model(), plan, wall));
    assert_eq!(docviews::effective_overrides(d.model(), plan, wall).projection_line_weight, -1);
    assert_eq!(docviews::filter_settings_returned(d.model(), plan, wall).map(|o| o.projection_line_weight), Some(6));
}

#[test]
fn a_schedule_instance_point_is_stored_only_when_it_differs() {
    let mut d = crate::new_document();
    let (s, sch) = d
        .transact("fx", false, |m, cx| {
            let s = sheets::create_sheet(m, cx, Some("A1"), None, true)?;
            let sc = crate::schedules::add_schedule_view(m, cx, Some("Walls"), None)?;
            Ok((s, sc))
        })
        .value
        .unwrap();
    let si = d.transact("si", false, |m, cx| sheets::create_schedule_instance(m, cx, s, sch, [0.1, 0.2])).value.unwrap();
    assert_eq!(d.transact("same", false, |m, _| sheets::set_schedule_instance_point(m, si, [0.1 + 1e-13, 0.2])).value, Some(false));
    assert_eq!(d.transact("moved", false, |m, _| sheets::set_schedule_instance_point(m, si, [0.3, 0.2])).value, Some(true));
    assert_eq!(d.model().docs.schedule_instances[&si].point, [0.3, 0.2]);
}

#[test]
fn demolishing_follows_the_order_check_of_the_row() {
    // Created == demolished is valid (Temporary), demolished before created is refused, an element without phases is
    // refused first
    let (mut d, plan, wall, _) = wall_doc();
    let _ = plan;
    assert!(d.transact("same", false, |m, cx| crate::phases::set_element_phases(m, cx, wall, Some(1), Some(Some(1)))).committed());
    assert_eq!(crate::phases::status(d.model(), wall, 1), crate::phases::PhaseStatus::Temporary);
    assert!(d.transact("before", false, |m, cx| crate::phases::set_element_phases(m, cx, wall, Some(1), Some(Some(0)))).value.is_none());
    assert!(d.transact("none", false, |m, cx| crate::phases::set_element_phases(m, cx, plan, None, Some(Some(1)))).value.is_none(), "a view has no phases");
}

#[test]
fn walls_that_do_not_coexist_are_not_joined_and_unequal_phases_are_not_tweaked() {
    use crate::phases::{elems_coexist, phases_equal, set_element_phases};
    let mut d = crate::new_document();
    let mut ids = Vec::new();
    d.transact("setup", false, |m, cx| {
        let l = ops::create_level(m, cx, "L0", 0.0)?;
        let cs = crate::CompoundStructure::new(vec![crate::Layer { width: 0.2, function: crate::LayerFunction::Structure }], 0, 0)?;
        let wt = ops::create_wall_type(m, cx, "T", cs)?;
        // a horizontal wall, a vertical wall that ends on its interior (T) and a third wall that makes an L with the first
        for (a, b) in [((0.0, 0.0), (6.0, 0.0)), ((3.0, 0.0), (3.0, 4.0)), ((6.0, 0.0), (6.0, 4.0))] {
            ids.push(ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l, start: Vec2::new(a.0, a.1), end: Vec2::new(b.0, b.1), height: 3.0, base_offset: 0.0, flip: false, structural: false })?);
        }
        Ok(())
    });
    let counts = |d: &BimDocument, i: usize| {
        let m = d.model();
        let s = crate::regen::wall_state(m, &m.walls[&ids[i]]).unwrap();
        (s.solid_edge_count, s.solid_vertex_count, s.volume)
    };
    let all_new = [counts(&d, 0), counts(&d, 1), counts(&d, 2)];
    // the stem (T) created in Existing, never demolished: coexists with the New walls, joined and trimmed, but NOT tweaked
    assert!(d.transact("p", false, |m, cx| set_element_phases(m, cx, ids[1], Some(0), None)).committed());
    let m = d.model();
    assert!(elems_coexist(m, ids[0], ids[1]) && !phases_equal(m, ids[0], ids[1]) && phases_equal(m, ids[0], ids[2]));
    assert!((counts(&d, 1).2 - all_new[1].2).abs() < 1e-12, "joined and trimmed as before");
    assert_ne!(counts(&d, 0).0, all_new[0].0, "the host loses the T-host edges of the tweak");
    assert_eq!(counts(&d, 2), all_new[2], "an untouched pair keeps its counts");
    // the stem demolished in the phase the host was created in: the two do NOT coexist, the stem is not trimmed against it
    assert!(d.transact("q", false, |m, cx| set_element_phases(m, cx, ids[1], Some(0), Some(Some(1)))).committed());
    let m = d.model();
    assert!(!elems_coexist(m, ids[0], ids[1]) && elems_coexist(m, ids[1], ids[1]));
    assert!(counts(&d, 1).2 > all_new[1].2, "an untrimmed end is longer");
    assert_eq!(counts(&d, 0).0, 12 - 1 /* the L corner of the host with wall 3 */, "the host is no T host any more");
}

#[test]
fn the_box_of_an_arc_is_the_box_of_its_stored_samples() {
    // The box of an arc is the box of its display samples. The apex of a half circle of radius 3 m falls between two of its samples
    // (the sample count is odd), so the box stops short of 3 by less than the display sag of 1 mm. OBSERVED in the reference
    // product: its box of this arc stops at 2.998458648602, 1.5 mm short; the engine's samples are its own.
    let arc = asterion_geom::Arc::from_three_points(Vec2::new(-3.0, 0.0), Vec2::new(0.0, 3.0), Vec2::new(3.0, 0.0), 1e-9).unwrap();
    let c = asterion_geom::Curve::Arc(arc);
    let b = crate::regen::sampled_aabb(&c);
    assert!(b.max.y < 3.0 && 3.0 - b.max.y <= 1.0e-3 + 1e-12, "{}", b.max.y);
    assert!((b.min.x + 3.0).abs() < 1e-12 && (b.max.x - 3.0).abs() < 1e-12, "the ends of the arc are sample points");
    assert!((c.aabb().max.y - 3.0).abs() < 1e-12, "the exact box reaches the apex");
    let line = asterion_geom::Curve::line(Vec2::new(0.0, 0.0), Vec2::new(2.0, 1.0));
    assert_eq!(crate::regen::sampled_aabb(&line).max.x, 2.0);
}

#[test]
fn a_dimension_value_is_rounded_by_the_schedule_renderer_to_the_millimetre() {
    // RoundToAccuracy(nearest) = floor(|v| / acc + 0.5 + 1e-9), the sign restored; f64::round would give 1234
    assert_eq!(crate::dims::value_text(1.2345), "1235");
    assert_eq!(crate::dims::value_text(4.0), "4000");
    assert_eq!(crate::dims::value_text(-2.5), "-2500");
    assert_eq!(crate::dims::value_text(0.0004999), "0");
}

#[test]
fn a_new_view_takes_its_detail_level_from_the_scale_table() {
    // Ratio <= 1/50 coarse, <= 1/10 medium, else fine (default metric bounds); 1:100 is the default scale
    use crate::model::DetailLevel::{Coarse, Fine, Medium};
    use crate::views::detail_level_for_scale;
    assert_eq!([100, 50, 49, 20, 10, 9, 1].map(detail_level_for_scale), [Coarse, Coarse, Medium, Medium, Medium, Fine, Fine]);
    let mut d = crate::new_document();
    let v = d
        .transact("v", false, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            ops::create_view(m, cx, "P20", ViewKind::FloorPlan, Some(l), Some(20))
        })
        .value
        .unwrap();
    assert_eq!(d.model().views[&v].detail_level, Medium);
    assert_eq!(d.model().views[&v].scale, 20);
    assert!(d.transact("s", false, |m, cx| ops::set_view_scale_denominator(m, cx, v, 200)).committed());
    assert_eq!(d.model().views[&v].detail_level, Medium);
}

#[test]
fn views_can_be_printed_by_their_native_type() {
    // Plans, sections, elevations, 3D views, sheets true
    use crate::view_dup::can_be_printed;
    assert!([1u8, 2, 3, 4, 6, 10, 11].iter().all(|t| can_be_printed(*t, false)));
    assert!([5u8, 7, 8, 12, 14].iter().all(|t| !can_be_printed(*t, false)));
    assert!(!can_be_printed(1, true));
}

#[test]
fn grid_vertical_extents_are_refused_in_the_order_of_the_row() {
    // Reversed (or equal) ranges first, then the 30000 ft length check on bottom, then on top
    let mut d = crate::new_document();
    let g = d.transact("g", false, |m, cx| ops::create_grid(m, cx, Some("G"), Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0))).value.unwrap();
    let msg = |d: &mut BimDocument, b: f64, t: f64| {
        let out = d.transact("e", false, move |m, cx| ops::set_grid_vertical_extent(m, cx, g, b, t));
        assert!(out.value.is_none());
        out.failures.first().map(|f| f.message.clone()).unwrap_or_default()
    };
    assert_eq!(msg(&mut d, 1.0, 1.0), "The bottom and top ranges are reversed for the extents.");
    assert_eq!(msg(&mut d, 2.0, 1.0), "The bottom and top ranges are reversed for the extents.");
    assert_eq!(msg(&mut d, -9145.0, 1.0), "The given value for bottom must be no more than 30000 feet in absolute value.");
    assert_eq!(msg(&mut d, -1.0, 9145.0), "The given value for top must be no more than 30000 feet in absolute value.");
    assert!(d.transact("ok", false, |m, cx| ops::set_grid_vertical_extent(m, cx, g, -1.0, 4.0)).committed());
    assert_eq!(d.model().grids[&g].vertical_extent, Some((-1.0, 4.0)));
}

#[test]
fn a_filter_name_is_checked_before_its_rule_and_a_grand_total_title_needs_both_flags() {
    // Empty name, prohibited character, duplicate name, then the rule's parameter
    let mut d = crate::new_document();
    let rule = |p: &str| FilterRule { param: p.into(), kind: "equals".into(), text: Some("x".into()), number: None };
    let msg = |d: &mut BimDocument, name: &str, p: &str| {
        let out = d.transact("f", false, move |m, cx| docviews::create_filter(m, cx, name, vec!["Walls".into()], rule(p)));
        assert!(out.value.is_none());
        out.failures.first().map(|f| f.message.clone()).unwrap_or_default()
    };
    assert!(msg(&mut d, "  ", "unconnected_height").starts_with("name is an empty string"));
    assert!(msg(&mut d, "A|B", "unconnected_height").starts_with("name cannot include prohibited characters"));
    assert!(d.transact("ok", false, |m, cx| docviews::create_filter(m, cx, "AST-HIGH", vec!["Walls".into()], rule("mark"))).committed());
    assert_eq!(msg(&mut d, "AST-HIGH", "unconnected_height"), "The given value for name is already in use as a filter element name.");
    assert!(msg(&mut d, "AST-OTHER", "unconnected_height").starts_with("One of the given rules"));
    // The grand total row first, then its title
    use crate::schedules;
    let s = d.transact("s", false, |m, cx| schedules::add_schedule_view(m, cx, Some("Walls"), None)).value.unwrap();
    let title = |d: &mut BimDocument, gt: Option<bool>, on: Option<bool>| {
        let out = d.transact("t", false, move |m, _| {
            schedules::edit_definition(m, s, schedules::DefinitionEdit { grand_total: gt, show_grand_total_title: on, grand_total_title: Some("Total".into()), ..Default::default() })
        });
        out.failures.first().map(|f| f.message.clone())
    };
    assert_eq!(title(&mut d, None, None).as_deref(), Some("Display of a grand total row is not enabled."));
    assert_eq!(title(&mut d, Some(true), None).as_deref(), Some("Display of grand total title is not enabled."));
    assert_eq!(title(&mut d, Some(true), Some(true)), None);
}

#[test]
fn a_wall_profile_sketch_is_made_once_and_a_floor_checks_its_level_before_its_type() {
    // Wall.CreateProfileSketch twice -> "Wall already has a sketch."; Floor.Create checks the level, then the type
    let (mut d, _, wall, _) = wall_doc();
    assert!(d.transact("p1", false, |m, cx| crate::wall_profiles::edit_wall_profile(m, cx, wall, None)).committed());
    let again = d.transact("p2", false, |m, cx| crate::wall_profiles::edit_wall_profile(m, cx, wall, None));
    assert!(again.value.is_none());
    assert_eq!(again.failures.first().map(|f| f.message.as_str()), Some("Wall already has a sketch."));
    let bad = ElementId(999_999);
    let boundary = vec![Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0), Vec2::new(4.0, 4.0), Vec2::new(0.0, 4.0)];
    let (level, slab_type) = d
        .transact("fx", false, |m, cx| {
            let l = ops::create_level(m, cx, "L9", 9.0)?;
            let t = ops::create_slab_type(m, cx, "S", 0.2, crate::model::SlabRole::Floor)?;
            Ok((l, t))
        })
        .value
        .unwrap();
    let code = |d: &mut BimDocument, t: ElementId, l: ElementId| {
        let b = boundary.clone();
        let out = d.transact("f", false, move |m, cx| ops::create_floor(m, cx, t, l, b, 0.0, false));
        assert!(out.value.is_none());
        out.failures.first().map(|f| f.message.clone()).unwrap()
    };
    let (level_code, type_code) = (code(&mut d, slab_type, bad), code(&mut d, bad, level));
    assert_ne!(level_code, type_code, "the two refusals have their own messages");
    // the level is checked first: with both ids bad the failure is the level's
    assert_eq!(code(&mut d, bad, bad), level_code);
}
