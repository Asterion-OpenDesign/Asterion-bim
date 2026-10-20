//! `views::view_state_of` reuses the bodies of the walls and floors while a document's generation is unchanged. It must give
//! exactly what `views::view_state` gives, after every kind of change: an edit, a rollback, an undo, a redo, another document.

mod common;

use asterion_bim::model::ViewKind;
use asterion_bim::views::{view_state, view_state_of, Appearance};
use asterion_bim::{new_document, ops, BimDocument, CompoundStructure, Layer, LayerFunction};
use asterion_core::ElementId;
use asterion_geom::Vec2;

fn wall(wall_type: ElementId, level: ElementId, x0: f64, x1: f64) -> ops::NewWall {
    ops::NewWall { wall_type, level, start: Vec2::new(x0, 0.0), end: Vec2::new(x1, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false }
}

fn drawn(doc: &BimDocument, view: ElementId) -> Vec<(u64, Vec<(i64, i64)>)> {
    let st = view_state_of(doc, doc.model().view(view).unwrap()).unwrap();
    st.elements
        .iter()
        .filter(|e| e.appearance != Appearance::Hidden)
        .map(|e| (e.id.0, e.footprint.iter().map(|p| ((p.x * 1e6).round() as i64, (p.y * 1e6).round() as i64)).collect()))
        .collect()
}

fn fresh(doc: &BimDocument, view: ElementId) -> Vec<(u64, Vec<(i64, i64)>)> {
    let st = view_state(doc.model(), doc.model().view(view).unwrap()).unwrap();
    st.elements
        .iter()
        .filter(|e| e.appearance != Appearance::Hidden)
        .map(|e| (e.id.0, e.footprint.iter().map(|p| ((p.x * 1e6).round() as i64, (p.y * 1e6).round() as i64)).collect()))
        .collect()
}

#[test]
fn the_cached_state_is_the_fresh_state_after_every_kind_of_change() {
    let mut s = common::scene();
    let plan = s.plan0;
    assert_eq!(drawn(&s.doc, plan), fresh(&s.doc, plan), "first read");
    // a second read with no change in between is the same
    assert_eq!(drawn(&s.doc, plan), fresh(&s.doc, plan), "second read");

    let before = s.doc.generation();
    let wt = s.doc.model().wall_types.keys().next().copied().expect("a wall type");
    let level = s.l0;
    let out = s.doc.transact("add wall", false, |m, cx| ops::create_wall(m, cx, wall(wt, level, 20.0, 26.0)));
    assert!(out.value.is_some(), "the wall is made");
    assert_ne!(s.doc.generation(), before, "a transaction renews the generation");
    let with_wall = fresh(&s.doc, plan);
    assert_eq!(drawn(&s.doc, plan), with_wall, "after an edit the cached state follows the model");
    assert!(with_wall.len() > 1);

    let g1 = s.doc.generation();
    s.doc.undo().expect("undo");
    assert_ne!(s.doc.generation(), g1, "undo renews the generation");
    assert_eq!(drawn(&s.doc, plan), fresh(&s.doc, plan), "after undo");
    assert!(drawn(&s.doc, plan).len() < with_wall.len(), "the wall is gone");

    s.doc.redo().expect("redo");
    assert_eq!(drawn(&s.doc, plan), with_wall, "after redo");

    // a rolled-back transaction leaves the model as it was and may leave the generation changed, never equal-and-wrong
    let g2 = s.doc.generation();
    let rolled = s.doc.transact("rolled back", true, |m, cx| ops::create_wall(m, cx, wall(wt, level, 30.0, 36.0)));
    assert!(rolled.value.is_some());
    assert_ne!(s.doc.generation(), g2);
    assert_eq!(drawn(&s.doc, plan), with_wall, "a rolled-back edit changes nothing");
}

#[test]
fn another_document_never_reads_the_state_of_this_one() {
    let a = common::scene();
    let plan_a = a.plan0;
    let read_a = drawn(&a.doc, plan_a);
    // a different, empty document on the same thread, then the first again
    let mut b = new_document();
    let view = {
        let out = b.transact("level", false, |m, cx| {
            let l = ops::create_level(m, cx, "L", 0.0)?;
            let wt = ops::create_wall_type(m, cx, "W", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
            ops::create_wall(m, cx, wall(wt, l, 0.0, 5.0))?;
            ops::create_view(m, cx, "plan", ViewKind::FloorPlan, Some(l), Some(100))
        });
        out.value.expect("a plan view")
    };
    let read_b = drawn(&b, view);
    assert_eq!(read_b, fresh(&b, view));
    assert_ne!(read_a, read_b);
    assert_eq!(drawn(&a.doc, plan_a), read_a, "the first document still reads its own state");
}

#[test]
fn an_elevation_framed_to_the_model_shows_the_walls_it_looks_at() {
    use asterion_bim::{docviews, section_graphics};
    let mut s = common::scene();
    let plan = s.plan0;
    // a marker well south of the building, looking north (index 1: the viewer side is -Y)
    let made = s.doc.transact("elevation", false, |m, cx| {
        let mk = docviews::create_marker(m, cx, plan, [3.0, -8.0, 0.0], 100)?;
        let v = docviews::add_elevation_view(m, cx, mk, 1)?;
        Ok::<ElementId, asterion_core::Failure>(v)
    });
    let v = made.value.expect("the elevation");
    // as made by the harness' operation: a 1 m box at the marker, which sees nothing of the building
    assert!(!docviews::in_section(s.doc.model(), s.doc.model().docs.views.get(&v).unwrap(), s.wall_s));
    let framed = s.doc.transact("frame", false, |m, _| docviews::frame_to_model(m, v, 1.0));
    assert!(framed.value.is_some());
    let m = s.doc.model();
    let dv = m.docs.views.get(&v).unwrap();
    assert!(docviews::in_section(m, dv, s.wall_s), "the south wall is in the framed elevation");
    assert!(dv.crop_max[2] <= 1e-9, "the box stays canonical (nothing in front of the cut plane)");
    assert!(docviews::far_clip_offset(dv) > 8.0, "the depth reaches across the model: {}", docviews::far_clip_offset(dv));
    let segs = section_graphics::section_segments(m, v).expect("figures");
    assert!(segs.len() > 8, "the elevation draws the building: {} segments", segs.len());
}
