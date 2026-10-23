//! The engine's failure catalog: for every failure code the engine can produce, what the reference product does with the
//! matching failure definition, when an specification rule reads it.
//!
//! The default processor of a session, which decides what a user sees for a Warning or an Error, is NOT specified and (a
//! severity-3 definition rolls the transaction back), and of the FR rows named on each entry.
//!
//! What the engine does with the catalog: the `severity` of a posted [`asterion_core::Failure`] is the engine's own two-way
//! commit decision (Warning kept, Error / DocumentCorruption rolled back, `asterion_core::txn`); the catalog adds the
//! reference product's literal, the channel, the specification rules and the resolution so that a caller (the shell,
//! through the FFI) can show the reference's view of a failure.
//!
//! The texts themselves are resources that are not in the DLLs read.

use crate::spec::codes;

/// How the reference product delivers the failure (as read in the rows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// Posted for the commit (a posted warning object): the transaction's end decides.
    Commit,
    /// Refused at the call (an exception at the API, or a validator's return code that the caller
    /// turns into a refusal before the change).
    Call,
    /// Posted through the performance adviser, not through the document (no commit effect).
    Adviser,
}

impl Channel {
    pub fn name(self) -> &'static str {
        match self {
            Channel::Commit => "commit",
            Channel::Call => "call",
            Channel::Adviser => "adviser",
        }
    }
}

/// The reference product side of a failure, as read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reference {
    /// Literal registered for the definition: 1 Warning, 2 Error, 3 DocumentCorruption; `None` when the rule names the
    /// definition but its literal is not specified.
    pub severity: Option<u8>,
    pub channel: Channel,
    /// Resolution type the definition / message carries first (its default), `None` = none read.
    pub resolution: Option<u8>,
    /// The specification rules that read it.
    pub rows: &'static [&'static str],
}

/// One catalog entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FailureDef {
    /// ASTERION's code (`spec::codes`).
    pub code: &'static str,
    /// The element kinds it concerns.
    pub element: &'static str,
    pub reference: Option<Reference>,
}

const fn r(severity: Option<u8>, channel: Channel, resolution: Option<u8>, rows: &'static [&'static str]) -> Option<Reference> {
    Some(Reference { severity, channel, resolution, rows })
}

pub fn severity_name(literal: u8) -> &'static str {
    match literal {
        0 => "none",
        1 => "warning",
        2 => "error",
        3 => "document_corruption",
        _ => "unnamed",
    }
}

pub fn resolution_name(t: u8) -> &'static str {
    match t {
        1 => "default",
        2 => "create_elements",
        3 => "delete_elements",
        4 => "skip_elements",
        5 => "move_elements",
        6 => "fix_elements",
        7 => "detach_elements",
        8 => "quit_edit_mode",
        9 => "unlock_constraints",
        10 => "set_value",
        11 => "save_document",
        12 => "show_elements",
        _ => "invalid",
    }
}

use Channel::{Adviser, Call, Commit};

/// The catalog of the ARCHITECTURE elements (walls, floors, ceilings, roofs, rooms, levels, grids, doors / windows /
/// openings, joins) and of the edits on them. Every entry with a `reference` restates what the named rules specified; the
/// rest are ASTERION's own codes.
pub const CATALOG: &[FailureDef] = &[
    FailureDef { code: codes::CANNOT_MAKE_WALL, element: "wall", reference: r(Some(2), Commit, Some(3), &["FR-07", "W-03a"]) },
    FailureDef { code: codes::BELOW_MIN_HEIGHT, element: "wall", reference: r(Some(2), Commit, Some(3), &["FR-07", "W-03a"]) },
    // Its literal is not named
    FailureDef { code: codes::TOP_BELOW_BASE, element: "wall", reference: r(None, Commit, None, &["W-03a"]) },
    FailureDef { code: codes::WALL_PROFILE_CURVE_TOO_SHORT, element: "wall", reference: None },
    FailureDef { code: codes::VOLUME_INTEGRATION_FAILED, element: "wall", reference: None },
    FailureDef { code: codes::SURFACE_AREA_INTEGRATION_FAILED, element: "wall", reference: None },
    FailureDef { code: codes::INVALID_ARC, element: "wall", reference: None },
    FailureDef { code: codes::BAD_END_INDEX, element: "wall", reference: None },
    FailureDef { code: codes::HEIGHT_OUT_OF_RANGE, element: "wall", reference: None },
    FailureDef { code: codes::CURVE_TOO_SHORT, element: "wall", reference: None },
    FailureDef { code: codes::INVALID_NUMBER, element: "any", reference: None },
    // Slab sketches - (sketch validators): the definitions are registered with the literal 3.
    FailureDef { code: codes::SLAB_LOOPS_INTERSECT, element: "floor,ceiling,roof", reference: r(Some(3), Call, None, &["FR-02", "FR-03", "F-04a", "K-16", "K-28"]) },
    FailureDef { code: codes::SLAB_LINES_OVERLAP, element: "floor,ceiling,roof", reference: r(Some(3), Call, None, &["FR-02", "FR-03", "F-04a"]) },
    FailureDef { code: codes::SLAB_OPEN_LOOP, element: "floor,ceiling,roof", reference: r(Some(3), Call, None, &["FR-02", "FR-03", "F-04a"]) },
    // No loops; the engine also uses this code for its own point-count rule
    FailureDef { code: codes::INVALID_BOUNDARY, element: "floor,ceiling,roof", reference: r(None, Call, None, &["F-04a"]) },
    // Err TooManySlopes; its literal is not specified
    FailureDef { code: codes::SLAB_TOO_MANY_SLOPES, element: "floor,ceiling", reference: r(None, Call, None, &["F-03a", "F-04a"]) },
    FailureDef { code: codes::SLAB_INVALID_ARC, element: "floor,ceiling,roof", reference: None },
    FailureDef { code: codes::SLAB_SLOPE_EDGE_NOT_LINE, element: "floor,ceiling,roof", reference: None },
    FailureDef { code: codes::SLAB_INTEGRATION_FAILED, element: "floor,ceiling", reference: None },
    FailureDef { code: codes::SLAB_SEGMENT_OUT_OF_RANGE, element: "floor,ceiling,roof", reference: None },
    FailureDef { code: codes::SLAB_SLOPE_NOT_FINITE, element: "floor,ceiling,roof", reference: None },
    FailureDef { code: codes::SLAB_ROLE_HAS_NO_SLOPE, element: "floor,ceiling,roof", reference: None },
    FailureDef { code: codes::INVALID_THICKNESS, element: "floor,ceiling,roof type", reference: None },
    FailureDef { code: codes::ROOF_CUTOFF_BELOW_BASE, element: "roof", reference: r(Some(2), Commit, Some(3), &["R-03a"]) },
    FailureDef { code: codes::SLAB_SLOPE_DEGENERATE, element: "roof", reference: r(Some(2), Commit, Some(3), &["R-03b"]) },
    // joins - : the joined-but-disjoint warning, literal 1, DetachElements
    FailureDef { code: codes::JOIN_DO_NOT_INTERSECT, element: "wall,floor", reference: r(Some(1), Commit, Some(7), &["FR-10"]) },
    FailureDef { code: codes::JOIN_ALREADY_JOINED, element: "wall,floor", reference: None },
    FailureDef { code: codes::JOIN_NOT_JOINED, element: "wall,floor", reference: None },
    FailureDef { code: codes::JOIN_UNSUPPORTED_PAIR, element: "wall,floor", reference: None },
    FailureDef { code: codes::INSERT_NOT_CUTTING_HOST, element: "door,window", reference: r(Some(2), Commit, Some(3), &["W-10g", "FR-09"]) },
    FailureDef { code: codes::INSERT_WRONG_LEVEL, element: "door,window", reference: r(Some(2), Commit, Some(3), &["W-10g", "FR-09"]) },
    FailureDef { code: codes::INSERT_OVERLAPS_OR_OUT, element: "door,window", reference: r(Some(1), Commit, Some(3), &["W-12d", "K-28"]) },
    // A warning with a "delete the opening" resolution; its literal is not specified
    FailureDef { code: codes::OPENING_DOES_NOT_CUT_HOST, element: "opening", reference: r(None, Commit, Some(3), &["W-09c"]) },
    FailureDef { code: codes::INSERT_HOST_NOT_SUPPORTED, element: "door,window", reference: None },
    FailureDef { code: codes::INSERT_BAD_SIZE, element: "door,window", reference: None },
    FailureDef { code: codes::INSERT_NO_LEVEL, element: "door,window", reference: None },
    FailureDef { code: codes::INSERT_BAD_STATION, element: "door,window", reference: None },
    FailureDef { code: codes::OPENING_INVALID_BOUNDARY, element: "opening", reference: None },
    FailureDef { code: codes::HOSTED_TYPE_DUPLICATE_NAME, element: "door,window type", reference: None },
    FailureDef { code: codes::PARAM_NOT_PRESENT, element: "door,window", reference: None },
    // A host with more than 50 openings and inserts, a warning through the performance adviser ([`performance_advice`])
    FailureDef { code: codes::HOST_HAS_TOO_MANY_INSERTS, element: "wall", reference: r(Some(1), Adviser, None, &["FR-05"]) },
    // rooms - Spec M-07 (literal 2, resolution type 6), Spec M-06 (literal 1)
    FailureDef { code: codes::ROOM_BASE_ABOVE_COMPUTATION_HEIGHT, element: "room", reference: r(Some(2), Commit, Some(6), &["M-07"]) },
    FailureDef { code: codes::ROOM_NONE_CREATED_AUTOMATICALLY, element: "room", reference: r(Some(1), Commit, None, &["M-06"]) },
    FailureDef { code: codes::ROOM_NO_ENCLOSED_REGION, element: "room", reference: r(None, Call, None, &["M-06"]) },
    FailureDef { code: codes::ROOM_LEVEL_HAS_NO_PLAN_VIEW, element: "room", reference: None },
    // levels - Spec L-01 (literal 1)
    FailureDef { code: codes::LEVEL_STORY_ABOVE_MOVED, element: "level", reference: r(Some(1), Commit, None, &["L-01"]) },
    FailureDef { code: codes::LEVEL_STORY_ABOVE_NOT_A_STORY, element: "level", reference: r(None, Commit, None, &["L-01"]) },
    // OBSERVED: refused at the call.
    FailureDef { code: codes::LEVEL_DUPLICATE_NAME, element: "level", reference: None },
    // grids
    FailureDef { code: codes::GRID_DUPLICATE_NAME, element: "grid", reference: None },
    FailureDef { code: codes::GRID_BAD_EXTENT, element: "grid", reference: None },
    FailureDef { code: codes::GRID_INVALID_ARC, element: "grid", reference: None },
    // edits common to the elements
    FailureDef { code: codes::TRANSFORM_INVALID, element: "any", reference: None },
    FailureDef { code: codes::TRANSFORM_NOT_SUPPORTED, element: "any", reference: None },
    FailureDef { code: codes::WALL_SPLIT_REFUSED, element: "wall", reference: None },
    FailureDef { code: codes::WALL_TRIM_REFUSED, element: "wall", reference: None },
    FailureDef { code: codes::STAIRS_TOP_NOT_ABOVE_BASE, element: "stairs", reference: None },
    FailureDef { code: codes::REFPLANE_DUPLICATE_NAME, element: "reference plane", reference: None },
    FailureDef { code: codes::REFPLANE_INVALID, element: "reference plane", reference: None },
    FailureDef { code: codes::MODEL_LINE_INVALID, element: "model line", reference: None },
    FailureDef { code: codes::MODEL_LINE_NOT_IN_PLANE, element: "model line", reference: None },
    FailureDef { code: codes::GROUP_EDIT_OUTSIDE_EDIT_MODE, element: "group", reference: None },
    FailureDef { code: codes::GROUP_CANNOT_GROUP, element: "group", reference: None },
    FailureDef { code: codes::DIMENSION_INVALID, element: "dimension", reference: None },
    FailureDef { code: codes::TAG_INVALID, element: "tag", reference: None },
    FailureDef { code: codes::TEXT_NOTE_INVALID, element: "text note", reference: None },
    FailureDef { code: codes::SWEEP_INVALID, element: "wall sweep", reference: None },
    FailureDef { code: codes::SWEEP_OVERLAP, element: "wall sweep", reference: None },
    FailureDef { code: codes::SWEEP_OUTSIDE_WALL, element: "wall sweep", reference: None },
    FailureDef { code: codes::MATERIAL_INVALID, element: "material", reference: None },
    FailureDef { code: codes::TOPOSOLID_INVALID, element: "toposolid", reference: None },
    FailureDef { code: codes::TOPOSOLID_SUBDIVISION_FAILED, element: "toposolid", reference: None },
    FailureDef { code: codes::DOCVIEW_INVALID, element: "view", reference: None },
    FailureDef { code: codes::VIEWFILTER_INVALID, element: "view filter", reference: None },
    FailureDef { code: codes::SCHEDULE_INVALID, element: "schedule", reference: None },
    FailureDef { code: codes::SHEET_INVALID, element: "sheet", reference: None },
    // (SheetNumberEmpty / SheetNumberDuplicated, both literal 2): the number parameter's validator returns the message; the
    // reference product caller raises it (a refusal at the call), one failing element: the sheet
    FailureDef { code: codes::SHEET_NUMBER_EMPTY, element: "sheet", reference: r(Some(2), Call, None, &["FR-11"]) },
    FailureDef { code: codes::SHEET_NUMBER_DUPLICATED, element: "sheet", reference: r(Some(2), Call, None, &["FR-11"]) },
    // (CannotPasteTag / CannotPasteAssociate, literal 1, no resolution): the tag found no host to re-attach to
    FailureDef { code: codes::TAG_NOT_PASTED, element: "tag", reference: r(Some(1), Commit, None, &["FR-04"]) },
    FailureDef { code: codes::RAILING_STAIRS_HAS_RAILINGS, element: "railing", reference: None },
    FailureDef { code: codes::ELEMENT_PINNED, element: "any", reference: None },
    FailureDef { code: codes::COLUMN_BAD_SIZE, element: "column", reference: None },
    FailureDef { code: codes::COLUMN_DUPLICATE_TYPE, element: "column", reference: None },
    FailureDef { code: codes::COLUMN_BAD_ATTACHMENT, element: "column", reference: None },
    FailureDef { code: codes::CURTAIN_INVALID, element: "wall", reference: None },
    FailureDef { code: codes::ARRAY_INVALID, element: "any", reference: None },
    FailureDef { code: codes::WALL_OVERLAPS_WALL, element: "wall", reference: None },
    FailureDef { code: codes::NOT_FOUND, element: "any", reference: None },
    FailureDef { code: codes::WRONG_KIND, element: "any", reference: None },
    FailureDef { code: codes::READ_ONLY, element: "any", reference: None },
    FailureDef { code: codes::INVALID_LAYERS, element: "wall type", reference: None },
];

/// The catalog entry of a code.
pub fn definition(code: &str) -> Option<&'static FailureDef> {
    CATALOG.iter().find(|d| d.code == code)
}

/// The number of openings and inserts above which the performance adviser warns about a host (the test is `count > 50`).
pub const MAX_HOST_INSERTS: usize = 50;

/// The performance adviser's warnings for the document: every wall that hosts more than [`MAX_HOST_INSERTS`] openings and
/// inserts, as a Warning naming the wall. These are not posted to a transaction (the adviser channel has no commit effect):
/// a caller asks for them.
///
/// UNKNOWN: which inserts counts (here: the wall's doors, windows and rectangular openings) and when the adviser runs its
/// rules.
pub fn performance_advice(m: &crate::model::BimModel) -> Vec<asterion_core::Failure> {
    let mut out = Vec::new();
    for w in m.walls.keys() {
        let n = m.hosted_instances.values().filter(|h| h.host == *w).count()
            + m.wall_openings.values().filter(|o| o.host == *w).count();
        if n > MAX_HOST_INSERTS {
            out.push(
                asterion_core::Failure::warning(
                    codes::HOST_HAS_TOO_MANY_INSERTS,
                    format!("This element hosts {n} openings and inserts; a large number of them can slow the model down."),
                )
                .on(&[*w]),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every code of the catalog is a declared code, appears once, and every reference literal is a named value other than
    /// None (a definition is never registered with 0).
    #[test]
    fn the_catalog_is_consistent() {
        let mut seen = std::collections::BTreeSet::new();
        for d in CATALOG {
            assert!(codes::ALL.contains(&d.code), "{} is not in codes::ALL", d.code);
            assert!(seen.insert(d.code), "{} twice", d.code);
            if let Some(r) = d.reference {
                if let Some(s) = r.severity {
                    assert!((1..=3).contains(&s), "{}", d.code);
                }
                assert!(!r.rows.is_empty(), "{}", d.code);
                if let Some(t) = r.resolution {
                    assert_ne!(resolution_name(t), "invalid", "{}", d.code);
                }
            }
        }
        assert_eq!(severity_name(3), "document_corruption");
        assert_eq!(definition(codes::JOIN_DO_NOT_INTERSECT).unwrap().reference.unwrap().resolution, Some(7));
    }

    /// Every failure code an architecture element can produce has a catalog entry.
    #[test]
    fn every_architecture_code_is_catalogued() {
        for c in codes::ALL.iter().filter(|c| !c.starts_with("stairs.") && !c.starts_with("view.")) {
            assert!(definition(c).is_some(), "{c} has no catalog entry");
        }
    }

    /// The adviser warns for MORE than 50 openings and inserts on one host.
    #[test]
    fn the_adviser_warns_above_fifty_inserts() {
        use crate::ops;
        let mut doc = crate::new_document();
        let out = doc.transact("t", false, |m, cx| {
            let l = ops::create_level(m, cx, "L", 0.0)?;
            let cs = crate::CompoundStructure::new(vec![crate::Layer { width: 0.2, function: crate::LayerFunction::Structure }], 0, 0)?;
            let wt = ops::create_wall_type(m, cx, "W", cs)?;
            let v = asterion_geom::Vec2::new;
            let w = ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l, start: v(0.0, 0.0), end: v(200.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false })?;
            let t = ops::create_hosted_type(m, cx, "Win", crate::model::HostedKind::Window, 0.5, 0.5, 0.05, 0.5, 0.5, 1.0, 0.0)?;
            for i in 0..51 {
                ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: t, host: w, level: l, host_station: 1.0 + 3.0 * i as f64, sill_height: None })?;
            }
            Ok(w)
        });
        let w = out.value.expect("51 windows placed");
        let advice = performance_advice(doc.model());
        assert_eq!(advice.len(), 1);
        assert_eq!((advice[0].code.as_str(), advice[0].elements.clone()), (codes::HOST_HAS_TOO_MANY_INSERTS, vec![w]));
        let last = *doc.model().hosted_instances.keys().next_back().unwrap();
        doc.transact("d", false, |m, cx| ops::delete(m, cx, last));
        assert!(performance_advice(doc.model()).is_empty(), "exactly 50 is not more than 50");
    }
}
