//! ASTERION BIM Semantic Engine, v0.1.
//!
//! Every rule implemented here is labelled in [`spec`] with the evidence that supports it (observed through API of the
//! reference product experiments, documented, or assumed). Wall joins and rectangular wall openings are implemented;
//! profiles, curtain walls and stacked walls are NOT.
//!
//! The opening slice comes, whose status is **SPECIFIED, NOT MEASURED**. Because the relation is shared, a host's net Area,
//! Volume, `insert_count`, face/edge counts and surface area account for either element kind with no special case at the
//! call sites.
//!
//! Deliberately NOT modelled (section 7f): the instance's own solid (leaf, frame, glass),
//! multi-layer hosts and opening wrapping, window insets moving the cut, phases, curtain walls,
//! face-hosted and work-plane-based instances, shaft and "by face" openings, stacked/slanted
//! walls, wall sweeps cut by inserts, and the auto-incrementing `Mark` sequence.


// Domains of the engine (one folder each, see STRUCTURE.md). Every module keeps its old crate-root path, so
// `asterion_bim::joins`, `crate::views`... resolve exactly as before the move.
pub mod engine;
pub mod walls;
pub mod geometry;
pub mod slabs;
pub mod rooms;
pub mod vertical;
pub mod levels_grids;
pub mod viewing;
pub mod documentation;

pub use engine::{model, ops, modify, transform, spec, params, instance_params, type_names, type_ops, quantity, failures, phases, groups, import_ops, document_settings, base_points};
pub use walls::{joins, join_end, join_order, junctions, crossings, curved_ends, curved_joins, wall_profiles, wall_vertices, stacked, sweeps, arc_sweeps, cut};
pub use geometry::{regen, element_geometry, mesh3d, boxtopo, sketch};
pub use slabs::{roof, slab_shape, rafter_cut, toposolid};
pub use rooms::{arc_rooms, room_trim, rooms_ext, shafts};
pub use vertical::{stairs, stairs_scene, columns, curtains};
pub use levels_grids::{datums, datum_extents, item_number};
pub use viewing::{views, view_camera, view_display, view_dup, view_names, view_visibility, plan_graphics, section_graphics, docviews, browser, pick3d};
pub use documentation::{annot, dims, tempdim, schedules, sheets, docs};

pub use model::{
    BimModel, CompoundStructure, CropRegion, DetailLevel, VisualStyle, Floor, FloorType, HostedInstance, HostedInstanceParam,
    HostedKind, HostedType, HostedTypeParam, InsertKind, Layer, LayerFunction, Level,
    LocationLine, DatumEnd, Grid, PlanViewPlane, RangeBase, RangePlane, View, ViewKind, ViewRange, Wall, WallOpening,
    WallType,
};
pub use regen::{CutRect, FloorState, HostOutline, HostedInstanceState, Insert, OpeningState, WallState};
pub use document_settings::DocumentSettings;
pub use views::{Appearance, ViewState, ViewedElement};

use asterion_core::{txn::IdRollbackPolicy, Document, IdAllocator};

/// A BIM document with the id policy of the reference product: ids consumed by a rolled-back or failed transaction are
/// never reused (scenario; Spec P-01: the id counter only advances, a rollback deletes the created elements and leaves the
/// counter untouched, undo and redo keep an element's id).
pub type BimDocument = Document<BimModel>;

pub fn new_document() -> BimDocument {
    Document::new(BimModel::default(), IdAllocator::default(), IdRollbackPolicy::Keep)
}

#[cfg(test)]
mod id_tests {
    //! Spec P-01: the id counter only advances; rollback and undo never give an id out again.
    use super::*;
    use asterion_core::{ElementId, Failure};

    fn level(doc: &mut BimDocument, rollback: bool, fail: bool) -> Option<ElementId> {
        let out = doc.transact("l", rollback, |m, cx| {
            // a fresh name each time: a level name must be unique (OBSERVED)
            let name = format!("L{}", m.levels.len());
            let id = ops::create_level(m, cx, &name, 0.0)?;
            if fail {
                return Err(Failure::error(spec::codes::NOT_FOUND, "forced"));
            }
            Ok(id)
        });
        out.allocated.first().copied().filter(|_| out.committed() || rollback || fail)
    }

    #[test]
    fn ids_are_monotonic_across_rollback_failure_and_undo() {
        let mut doc = new_document();
        let a = level(&mut doc, false, false).unwrap();
        let rolled = level(&mut doc, true, false).unwrap();
        let failed = level(&mut doc, false, true).unwrap();
        let b = level(&mut doc, false, false).unwrap();
        assert!(rolled > a && failed > rolled && b > failed, "{a:?} {rolled:?} {failed:?} {b:?}");
        assert!(doc.model().levels.get(&rolled).is_none() && doc.model().levels.get(&failed).is_none());
        doc.undo().unwrap();
        let c = level(&mut doc, false, false).unwrap();
        assert!(c > b, "an undone creation does not hand its id back");
        doc.undo().unwrap();
        doc.redo().unwrap();
        assert!(doc.model().levels.contains_key(&c), "redo keeps the element's id");
    }

    #[test]
    fn the_counter_covers_every_element_map() {
        let mut doc = new_document();
        let out = doc.transact("f", false, |m, cx| {
            let l = ops::create_level(m, cx, "L", 0.0)?;
            let t = ops::create_floor_type(m, cx, "F", 0.2)?;
            let v = asterion_geom::Vec2::new;
            ops::create_floor(m, cx, t, l, vec![v(0.0, 0.0), v(1.0, 0.0), v(1.0, 1.0)], 0.0, false)
        });
        let f = out.value.unwrap();
        assert_eq!(doc.model().max_element_id(), Some(f));
    }
}

#[cfg(test)]
mod spec_pin_tests {
    //! Rules of specification rules that the engine already followed, pinned (Spec F-06).
    use super::*;

    /// A newly flagged slope-defining edge takes the ratio 1/12 (atan(1/12) stored).
    #[test]
    fn a_flagged_edge_starts_at_one_in_twelve() {
        let e = model::SlabSlopeEdge::with_default_slope(2);
        assert_eq!((e.edge, e.slope), (2, 1.0 / 12.0));
        assert!((e.slope.atan() - 0.08333333333333333_f64.atan()).abs() < 1e-15);
    }

    /// Spec F-06: the default floor type is got, or created when the document has none.
    #[test]
    fn the_default_floor_type_is_got_or_created() {
        let mut doc = new_document();
        let out = doc.transact("t", false, |m, cx| {
            let made = ops::default_floor_type(m, cx)?;
            assert_eq!(m.floor_types[&made].role, model::SlabRole::Floor);
            assert_eq!(ops::default_floor_type(m, cx)?, made, "got, not created twice");
            Ok(())
        });
        assert!(out.committed());
        assert_eq!(doc.model().floor_types.len(), 1);
    }

    /// A level change reaches the elements that depend on it (their derived state is recomputed from the stored level id).
    #[test]
    fn a_level_move_reaches_its_dependents() {
        let mut doc = new_document();
        let out = doc.transact("t", false, |m, cx| {
            let l = ops::create_level(m, cx, "L", 0.0)?;
            let t = ops::create_floor_type(m, cx, "F", 0.2)?;
            let v = asterion_geom::Vec2::new;
            let f = ops::create_floor(m, cx, t, l, vec![v(0.0, 0.0), v(1.0, 0.0), v(1.0, 1.0)], 0.0, false)?;
            Ok((l, f))
        });
        let (l, f) = out.value.unwrap();
        assert!(doc.model().dependents(l).contains(&f));
        doc.transact("move", false, |m, cx| ops::set_level_elevation(m, cx, l, 3.0));
        let s = regen::slab_state(doc.model(), doc.model().slab(f).unwrap()).unwrap();
        assert!((s.top_z - 3.0).abs() < 1e-12);
    }
}
