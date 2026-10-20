//! Phases: the phase an element is created and demolished in, the phase and phase filter of a plan view, the phase of a
//! room, and what follows from them (the element's status in a phase, what a view collects, which walls bound the rooms of
//! a phase). OBSERVED through the public API and 36b.
//!
//! Rules specified: (an element's status in a phase is decided by the ORDER INDICES of its created and demolished phases
//! and the query phase in the project's phase array: [`status`]) (a phase filter is seven presentations, one per status; an
//! element is shown exactly when its status is None or its presentation is 1 ShowByCategory or 2 ShowOverridden:
//! [`PhaseFilter`], [`filter_shows`]; the filter the API creates presents Existing, Demolished and Temporary as
//! ShowOverridden, New as ShowByCategory and None, Past and Future as DontShow) (a new element takes the LAST project phase
//! when no canvas view is active: [`DEFAULT_PHASE`]) and (the definition "invalid order of phases", severity Error). The
//! room-side rules and the stock filters' tables (template file data) are ENGINE CHOICES that fit the named observation.
//!
//! The phases and the phase filters of a document come from its template: the reference product's API has no way to create
//! either. The engine carries the two phases and the five filters the template of the measurements has.

use crate::model::BimModel;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The phases of the document in sequence order (OBSERVED: "Existing" sequence 1, "New Construction" sequence 2; template
/// content). A phase is stored as its index here.
pub const PHASES: [&str; 2] = ["Existing", "New Construction"];

/// The phase new elements are created in and new views show: the last one (OBSERVED: every element made by the API reads
/// "New Construction", and a new plan shows that phase). ENGINE CHOICE: whether it is the last phase or the phase of the
/// active view is not specified (a room placed while its level's plan shows Existing is still born in New Construction).
pub const DEFAULT_PHASE: u8 = 1;

/// The phase filters of the template (OBSERVED), in the order the reference product lists them.
pub const FILTERS: [&str; 5] = ["Show All", "Show Complete", "Show Demolition", "Show New + Existing", "Show Only Current Phase"];

pub mod codes {
    pub const NO_PHASES: &str = "phase.element_has_no_phases";
    pub const BAD_ORDER: &str = "phase.invalid_order";
    pub const UNKNOWN_PHASE: &str = "phase.unknown";
    pub const UNKNOWN_FILTER: &str = "phase.unknown_filter";
    pub const VIEW_PHASE_MISMATCH: &str = "phase.view_phase_mismatch";
    pub const FILTER_NAME_IN_USE: &str = "phase.filter_name_in_use";
    pub const BAD_PRESENTATION: &str = "phase.bad_presentation";
}

/// An element's status in one phase (the reference product's enumeration names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhaseStatus {
    /// The element has no phases (a room, a datum, a view).
    None,
    /// Demolished in an earlier phase.
    Past,
    Existing,
    Demolished,
    New,
    Future,
    Temporary,
}

impl PhaseStatus {
    pub fn name(self) -> &'static str {
        match self {
            PhaseStatus::None => "None",
            PhaseStatus::Past => "Past",
            PhaseStatus::Existing => "Existing",
            PhaseStatus::Demolished => "Demolished",
            PhaseStatus::New => "New",
            PhaseStatus::Future => "Future",
            PhaseStatus::Temporary => "Temporary",
        }
    }
}

/// The phases of one element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementPhases {
    pub created: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub demolished: Option<u8>,
}

impl Default for ElementPhases {
    fn default() -> Self {
        ElementPhases { created: DEFAULT_PHASE, demolished: None }
    }
}

/// The phase and the phase filter of a view: one of the template's stock filters by index into [`FILTERS`], or a filter
/// made through the API (`custom`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewPhase {
    pub phase: u8,
    pub filter: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<ElementId>,
}

impl Default for ViewPhase {
    fn default() -> Self {
        // OBSERVED: a new plan shows New Construction with "Show All"
        ViewPhase { phase: DEFAULT_PHASE, filter: 0, custom: None }
    }
}

pub const STATUS_COUNT: usize = 7;

pub fn status_number(s: PhaseStatus) -> usize {
    match s {
        PhaseStatus::None => 0,
        PhaseStatus::Past => 1,
        PhaseStatus::Existing => 2,
        PhaseStatus::Demolished => 3,
        PhaseStatus::New => 4,
        PhaseStatus::Temporary => 5,
        PhaseStatus::Future => 6,
    }
}

/// 0 DontShow, 1 ShowByCategory, 2 ShowOverridden.
pub const DONT_SHOW: u8 = 0;
pub const SHOW_BY_CATEGORY: u8 = 1;
pub const SHOW_OVERRIDDEN: u8 = 2;

/// A phase filter made through the API: seven presentations indexed by status number. The stock filters of the template are
/// not stored as elements here (their tables are template file data; [`filter_shows`] holds the measured behaviour of the
/// five).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhaseFilter {
    pub id: ElementId,
    pub name: String,
    pub presentation: [u8; STATUS_COUNT],
}

impl PhaseFilter {
    /// Whether an element of this status is shown: a status None is always visible, otherwise exactly when the presentation
    /// is ShowByCategory or ShowOverridden.
    pub fn shows(&self, s: PhaseStatus) -> bool {
        s == PhaseStatus::None || self.presentation[status_number(s)] != DONT_SHOW
    }
}

/// What the document stores about phases. Absent entries read as the defaults.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PhaseModel {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub elements: BTreeMap<ElementId, ElementPhases>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub views: BTreeMap<ElementId, ViewPhase>,
    /// The phase of a room (`ROOM_PHASE`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rooms: BTreeMap<ElementId, u8>,
}

impl PhaseModel {
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty() && self.views.is_empty() && self.rooms.is_empty()
    }
}

pub fn phase_index(name: &str) -> Option<u8> {
    PHASES.iter().position(|p| *p == name).map(|i| i as u8)
}

pub fn filter_index(name: &str) -> Option<u8> {
    FILTERS.iter().position(|p| *p == name).map(|i| i as u8)
}

/// `PhaseFilter.Create(document, name)`: refused when the name is in use ("name is already in use"); the new filter
/// presents Existing, Demolished and Temporary as ShowOverridden, New as ShowByCategory and the other statuses (None, Past,
/// Future) as DontShow. UNKNOWN: the trimming and case rules.
pub fn create_phase_filter(m: &mut BimModel, cx: &mut TxnContext<'_>, name: &str) -> Result<ElementId, Failure> {
    if filter_index(name).is_some() || m.docs.phase_filters.values().any(|f| f.name == name) {
        return Err(Failure::error(codes::FILTER_NAME_IN_USE, "name is already in use."));
    }
    let mut presentation = [DONT_SHOW; STATUS_COUNT];
    presentation[status_number(PhaseStatus::Existing)] = SHOW_OVERRIDDEN;
    presentation[status_number(PhaseStatus::Demolished)] = SHOW_OVERRIDDEN;
    presentation[status_number(PhaseStatus::Temporary)] = SHOW_OVERRIDDEN;
    presentation[status_number(PhaseStatus::New)] = SHOW_BY_CATEGORY;
    let id = cx.new_id();
    m.docs.phase_filters.insert(id, PhaseFilter { id, name: name.to_owned(), presentation });
    Ok(id)
}

/// `PhaseFilter.SetPhaseStatusPresentation`: stores the presentation of one status (a status above 6 or a presentation
/// above 2 is refused).
pub fn set_phase_status_presentation(m: &mut BimModel, filter: ElementId, status: usize, presentation: u8) -> Result<(), Failure> {
    if status >= STATUS_COUNT || presentation > SHOW_OVERRIDDEN {
        return Err(Failure::error(codes::BAD_PRESENTATION, "The status or the presentation is out of range."));
    }
    let f = m.docs.phase_filters.get_mut(&filter).ok_or_else(|| Failure::error(crate::spec::codes::NOT_FOUND, "No such phase filter.").on(&[filter]))?;
    f.presentation[status] = presentation;
    Ok(())
}

/// Whether an element carries created / demolished phases. OBSERVED: walls and doors do; a room does not ("The element does
/// not have properties CreatedPhaseId and DemolishedPhaseId"). ENGINE CHOICE for the other model kinds (floors, ceilings,
/// roofs, columns, openings, stairs: yes; datums and views: no).
pub fn carries_phase_data(m: &BimModel, id: ElementId) -> bool {
    m.walls.contains_key(&id)
        || m.hosted_instances.contains_key(&id)
        || m.floors.contains_key(&id)
        || m.columns.contains_key(&id)
        || m.wall_openings.contains_key(&id)
        || m.stairs.stairs.contains_key(&id)
}

pub fn element_phases(m: &BimModel, id: ElementId) -> Option<ElementPhases> {
    carries_phase_data(m, id).then(|| m.phases.elements.get(&id).copied().unwrap_or_default())
}

pub fn view_phase(m: &BimModel, view: ElementId) -> ViewPhase {
    m.phases.views.get(&view).copied().unwrap_or_default()
}

pub fn room_phase(m: &BimModel, room: ElementId) -> u8 {
    m.phases.rooms.get(&room).copied().unwrap_or(DEFAULT_PHASE)
}

/// `Element.GetPhaseStatus(phase)`. OBSERVED: created later -> Future; created in the phase -> New, or Temporary when also
/// demolished in it; created earlier -> Existing, or Demolished in its demolition phase. ENGINE CHOICE: demolished in an
/// earlier phase -> Past (never measured: two phases cannot produce it).
pub fn status(m: &BimModel, id: ElementId, phase: u8) -> PhaseStatus {
    let Some(p) = element_phases(m, id) else { return PhaseStatus::None };
    if phase < p.created {
        return PhaseStatus::Future;
    }
    match p.demolished {
        Some(d) if d < phase => PhaseStatus::Past,
        Some(d) if d == phase && p.created == phase => PhaseStatus::Temporary,
        Some(d) if d == phase => PhaseStatus::Demolished,
        _ if p.created == phase => PhaseStatus::New,
        _ => PhaseStatus::Existing,
    }
}

/// Whether a phase filter shows an element of the given status. OBSERVED (a plan of New Construction with an existing wall,
/// three new walls and a wall demolished in New Construction): Show All shows all of them; Show Complete the new and
/// existing ones; Show Demolition the existing and the demolished one; Show New + Existing the new and existing; Show Only
/// Current Phase only the new ones; Show All shows a Temporary door. ENGINE CHOICE for Temporary under the other filters
/// (not shown) and for Past / Future (never shown).
pub fn filter_shows(filter: u8, s: PhaseStatus) -> bool {
    use PhaseStatus::*;
    match s {
        None => true,
        Past | Future => false,
        Temporary => filter == 0,
        New => matches!(filter, 0 | 1 | 3 | 4),
        Existing => matches!(filter, 0 | 1 | 2 | 3),
        Demolished => matches!(filter, 0 | 2),
    }
}

/// Whether a plan view collects an element on account of phases: an element with phases by its status in the view's phase
/// and the view's filter; a room when its phase is the view's (OBSERVED: the plan of the Existing phase does not collect
/// the rooms of New Construction; round 36b: the rooms are collected under every filter).
pub fn shown_in_view(m: &BimModel, view: ElementId, id: ElementId) -> bool {
    let vp = view_phase(m, view);
    if m.rooms.contains_key(&id) {
        return room_phase(m, id) == vp.phase;
    }
    let s = status(m, id, vp.phase);
    match vp.custom.and_then(|c| m.docs.phase_filters.get(&c)) {
        Some(f) => f.shows(s),
        None => filter_shows(vp.filter, s),
    }
}

/// Whether an element exists for the rooms of a phase: it bounds them only when it stands in that phase (status New or
/// Existing). OBSERVED: a partition demolished in New Construction does not bound the rooms of New Construction (the two
/// rooms become one region), and bounds them again once it is no longer demolished. ENGINE CHOICE for Temporary (not
/// bounding) and for an element with no phases (always).
pub fn exists_in_phase(m: &BimModel, id: ElementId, phase: u8) -> bool {
    matches!(status(m, id, phase), PhaseStatus::None | PhaseStatus::New | PhaseStatus::Existing)
}

/// An element with no phases has the invalid id for both, so two such elements are equal.
pub fn phases_equal(m: &BimModel, a: ElementId, b: ElementId) -> bool {
    match (element_phases(m, a), element_phases(m, b)) {
        (Some(x), Some(y)) => x == y,
        (None, None) => true,
        _ => false,
    }
}

/// On the project phases: true when `x == y`, or when both are found in the phase list and the index of x is not above the
/// index of y; the invalid id of a never-demolished element is never found.
fn order_correct(x: Option<u8>, y: Option<u8>) -> bool {
    match (x, y) {
        (Some(a), Some(b)) => a == b || ((a as usize) < PHASES.len() && (b as usize) < PHASES.len() && a <= b),
        _ => false,
    }
}

/// . True when either element has no phases (the null case); false when the lifetimes `[created, demolished)` do not
/// overlap: after the guard (false only when the four ids are all equal) it asks whether the demolished phase of one
/// element is at or before the created phase of the other, for both elements. An element demolished in phase 2 and an
/// element created in phase 2 do NOT coexist; an element created in the first phase and never demolished coexists with
/// every later one. UNKNOWN: the order of the four ids inside the guard.
pub fn elems_coexist(m: &BimModel, a: ElementId, b: ElementId) -> bool {
    let (Some(x), Some(y)) = (element_phases(m, a), element_phases(m, b)) else { return true };
    if x.demolished == Some(x.created) && y.demolished == Some(y.created) && x.created == y.created {
        return true;
    }
    !(order_correct(x.demolished, Some(y.created)) || order_correct(y.demolished, Some(x.created)))
}

/// `Element.CreatedPhaseId` / `DemolishedPhaseId` writes. Refused in this order - the element has no phases ("The element
/// does not have properties CreatedPhaseId and DemolishedPhaseId."), then an order that puts the demolition before the
/// creation ("Invalid order of phases: An object cannot be demolished before it was created."; valid when the two phases
/// are equal, the demolished phase is none, or created is not after demolished); the created phase is not changed by a
/// demolish, so an element created in the phase it is demolished in is Temporary there. UNKNOWN: the Demolish tool itself
/// (which view phase it uses, what it does with an element created in that phase). `created` / `demolished`: `None` =
/// unchanged; `Some(None)` for the demolished phase = "none". OBSERVED: an element with no phases is refused at the call.
/// Created = demolished is accepted.
pub fn set_element_phases(m: &mut BimModel, _cx: &mut TxnContext<'_>, id: ElementId, created: Option<u8>, demolished: Option<Option<u8>>) -> Result<(), Failure> {
    let Some(mut p) = element_phases(m, id) else {
        return Err(Failure::error(codes::NO_PHASES, "The element does not have properties CreatedPhaseId and DemolishedPhaseId.").on(&[id]));
    };
    if let Some(c) = created {
        if c as usize >= PHASES.len() {
            return Err(Failure::error(codes::UNKNOWN_PHASE, "No such phase."));
        }
        p.created = c;
    }
    if let Some(d) = demolished {
        if d.map_or(false, |d| d as usize >= PHASES.len()) {
            return Err(Failure::error(codes::UNKNOWN_PHASE, "No such phase."));
        }
        p.demolished = d;
    }
    if p.demolished.map_or(false, |d| d < p.created) {
        return Err(Failure::error(codes::BAD_ORDER, "Invalid order of phases: An object cannot be demolished before it was created.").on(&[id]));
    }
    if p == ElementPhases::default() {
        m.phases.elements.remove(&id);
    } else {
        m.phases.elements.insert(id, p);
    }
    Ok(())
}

/// The phase and / or the phase filter of a plan view.
pub fn set_view_phase(m: &mut BimModel, _cx: &mut TxnContext<'_>, view: ElementId, phase: Option<&str>, filter: Option<&str>) -> Result<(), Failure> {
    if !m.views.contains_key(&view) {
        return Err(Failure::error(crate::spec::codes::NOT_FOUND, "Only a plan view has a phase here.").on(&[view]));
    }
    let mut vp = view_phase(m, view);
    if let Some(p) = phase {
        vp.phase = phase_index(p).ok_or_else(|| Failure::error(codes::UNKNOWN_PHASE, format!("no phase named {p}")))?;
    }
    if let Some(f) = filter {
        match filter_index(f) {
            Some(i) => {
                vp.filter = i;
                vp.custom = None;
            }
            None => {
                let c = m.docs.phase_filters.values().find(|p| p.name == f).ok_or_else(|| Failure::error(codes::UNKNOWN_FILTER, format!("no phase filter named {f}")))?;
                vp.custom = Some(c.id);
            }
        }
    }
    if vp == ViewPhase::default() {
        m.phases.views.remove(&view);
    } else {
        m.phases.views.insert(view, vp);
    }
    Ok(())
}

/// `NewRooms2(level, phase)`: OBSERVED refused at the call when the plan associated with the level shows another phase
/// ("Phase associated with view does not match input phase"); accepted once that plan shows the phase, and the rooms are
/// made in the circuits of that phase, in that phase. The associated plan is the view of the level with the LOWEST view id
/// among those of the floor-plan type, else the first of the type; the engine's floor plans are the first kind (the names
/// of the two type resources are not specified).
pub fn create_rooms_in_phase(m: &mut BimModel, cx: &mut TxnContext<'_>, level: ElementId, phase: u8) -> Result<Vec<ElementId>, Failure> {
    let plan = m
        .views
        .values()
        .filter(|v| v.gen_level == Some(level) && v.kind == crate::model::ViewKind::FloorPlan)
        .map(|v| v.id)
        .min();
    let Some(plan) = plan else {
        return Err(Failure::error(crate::spec::codes::ROOM_LEVEL_HAS_NO_PLAN_VIEW, "The level has no plan view."));
    };
    if view_phase(m, plan).phase != phase {
        return Err(Failure::error(codes::VIEW_PHASE_MISMATCH, "Phase associated with view does not match input phase."));
    }
    let regions = crate::regen::enclosed_regions_full_in(m, level, phase);
    let mut ids = Vec::new();
    for (poly, islands) in &regions {
        let seed = crate::rooms_ext::auto_point(m, level, phase, poly, islands);
        let id = crate::ops::create_room(m, cx, level, seed)?;
        m.rooms.get_mut(&id).expect("just made").tagged = true;
        if phase != DEFAULT_PHASE {
            m.phases.rooms.insert(id, phase);
        }
        ids.push(id);
    }
    Ok(ids)
}

/// Forget the phases of deleted elements.
pub fn elements_deleted(m: &mut BimModel, gone: &[ElementId]) {
    for g in gone {
        m.phases.elements.remove(g);
        m.phases.views.remove(g);
        m.phases.rooms.remove(g);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filter_made_through_the_api_presents_the_statuses_the_row_reads() {
        // Existing, Demolished, Temporary ShowOverridden; New ShowByCategory; None, Past, Future DontShow
        let mut d = crate::new_document();
        let f = d.transact("f", false, |m, cx| create_phase_filter(m, cx, "Mine")).value.unwrap();
        let p = &d.model().docs.phase_filters[&f];
        assert_eq!(p.presentation, [0, 0, 2, 2, 1, 2, 0]);
        assert!(p.shows(PhaseStatus::None) && p.shows(PhaseStatus::Existing) && p.shows(PhaseStatus::New) && p.shows(PhaseStatus::Temporary));
        assert!(!p.shows(PhaseStatus::Past) && !p.shows(PhaseStatus::Future));
        // a name in use (a stock filter's, or its own) is refused; a presentation edit is stored; out-of-range values are refused
        assert!(d.transact("dup", false, |m, cx| create_phase_filter(m, cx, "Mine")).value.is_none());
        assert!(d.transact("dup2", false, |m, cx| create_phase_filter(m, cx, "Show All")).value.is_none());
        assert!(d.transact("e", false, |m, _| set_phase_status_presentation(m, f, status_number(PhaseStatus::New), DONT_SHOW)).committed());
        assert!(!d.model().docs.phase_filters[&f].shows(PhaseStatus::New));
        assert!(d.transact("bad", false, |m, _| set_phase_status_presentation(m, f, 7, 1)).value.is_none());
        assert!(d.transact("bad2", false, |m, _| set_phase_status_presentation(m, f, 2, 3)).value.is_none());
    }

    #[test]
    fn a_view_with_an_api_filter_shows_by_its_presentations() {
        let mut d = crate::new_document();
        let (v, w) = d
            .transact("fx", false, |m, cx| {
                let l = crate::ops::create_level(m, cx, "L0", 0.0)?;
                let cs = crate::CompoundStructure::new(vec![crate::Layer { width: 0.2, function: crate::LayerFunction::Structure }], 0, 0)?;
                let t = crate::ops::create_wall_type(m, cx, "T", cs)?;
                let w = crate::ops::create_wall(m, cx, crate::ops::NewWall { wall_type: t, level: l, start: asterion_geom::Vec2::new(0.0, 0.0), end: asterion_geom::Vec2::new(6.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false })?;
                let v = crate::ops::create_view(m, cx, "P", crate::model::ViewKind::FloorPlan, Some(l), None)?;
                Ok((v, w))
            })
            .value
            .unwrap();
        let f = d.transact("f", false, |m, cx| create_phase_filter(m, cx, "Mine")).value.unwrap();
        d.transact("sv", false, |m, cx| set_view_phase(m, cx, v, None, Some("Mine"))).value.unwrap();
        assert!(shown_in_view(d.model(), v, w), "a new wall is New: ShowByCategory");
        d.transact("e", false, |m, _| set_phase_status_presentation(m, f, status_number(PhaseStatus::New), DONT_SHOW)).value.unwrap();
        assert!(!shown_in_view(d.model(), v, w));
        d.transact("back", false, |m, cx| set_view_phase(m, cx, v, None, Some("Show All"))).value.unwrap();
        assert!(shown_in_view(d.model(), v, w) && view_phase(d.model(), v).custom.is_none());
        assert!(d.transact("nf", false, |m, cx| set_view_phase(m, cx, v, None, Some("Nope"))).value.is_none());
    }

    #[test]
    fn the_status_of_an_element_follows_its_phases() {
        let mut d = crate::new_document();
        let mut w = ElementId(0);
        d.transact("w", false, |m, cx| {
            let l = crate::ops::create_level(m, cx, "L0", 0.0)?;
            let cs = crate::CompoundStructure::new(vec![crate::Layer { width: 0.2, function: crate::LayerFunction::Structure }], 0, 0)?;
            let t = crate::ops::create_wall_type(m, cx, "T", cs)?;
            w = crate::ops::create_wall(
                m,
                cx,
                crate::ops::NewWall { wall_type: t, level: l, start: asterion_geom::Vec2::new(0.0, 0.0), end: asterion_geom::Vec2::new(6.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false },
            )?;
            Ok(())
        });
        assert_eq!(status(d.model(), w, 0), PhaseStatus::Future);
        assert_eq!(status(d.model(), w, 1), PhaseStatus::New);
        assert!(d.transact("p", false, |m, cx| set_element_phases(m, cx, w, Some(0), Some(Some(1)))).committed());
        assert_eq!(status(d.model(), w, 0), PhaseStatus::New);
        assert_eq!(status(d.model(), w, 1), PhaseStatus::Demolished);
        assert!(!exists_in_phase(d.model(), w, 1));
        // demolished before created is refused
        assert!(d.transact("bad", false, |m, cx| set_element_phases(m, cx, w, Some(1), Some(Some(0)))).value.is_none());
        assert!(d.transact("tmp", false, |m, cx| set_element_phases(m, cx, w, Some(1), Some(Some(1)))).committed());
        assert_eq!(status(d.model(), w, 1), PhaseStatus::Temporary);
        assert!(filter_shows(0, PhaseStatus::Temporary) && !filter_shows(4, PhaseStatus::Existing) && filter_shows(2, PhaseStatus::Demolished));
    }
}
