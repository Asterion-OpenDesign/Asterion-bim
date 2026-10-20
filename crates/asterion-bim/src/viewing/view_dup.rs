//! * The three options are `Duplicate`, `AsDependent` and `WithDetailing` (the name table).
//! * With detailing and as dependent also not reports (8) or schedules (5) and not walkthroughs; as dependent also not a 3D
//!   view (4), not a drafting / rendering view (10) and not a detail view, and the view must be a model view (a plan,
//!   section, elevation or 3D view).
//! * The copy takes the source's phase and phase filter and keeps its view template. A copy of a dependent view stays
//!   dependent on the same primary view; a dependent copy of a dependent view hangs on the source's primary view.
//!   `WithDetailing` is the plain duplicate plus the view-specific annotation elements of the source (text notes,
//!   dimensions, tags:), and a copy of a dependent view is standalone then.
//! * See [`crate::docviews::graphics_owner`].
//!
//! ENGINE CHOICE: every field of the view, its graphics and its hidden-element set; no annotation element. The schedule
//! views (type 5) are not views of the engine (`docs.schedules`), so a schedule is never offered.

use crate::annot::{Dimension, Tag, TextNote};
use crate::docviews::{DocView, DocViewKind};
use crate::model::{BimModel, ViewKind};
use crate::spec::codes;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuplicateOption {
    Duplicate,
    AsDependent,
    WithDetailing,
}

impl DuplicateOption {
    pub fn from_number(n: i32) -> Option<Self> {
        match n {
            0 => Some(DuplicateOption::Duplicate),
            1 => Some(DuplicateOption::AsDependent),
            2 => Some(DuplicateOption::WithDetailing),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            DuplicateOption::Duplicate => "Duplicate",
            DuplicateOption::AsDependent => "AsDependent",
            DuplicateOption::WithDetailing => "WithDetailing",
        }
    }
}

/// The suffix type: the resource template keyed by the type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameSuffix {
    /// 0: "%1 Copy %2".
    Copied,
    /// 1: "%1 - Dependent %2".
    Dependent,
    /// 2: "%1 - Callout %2".
    Callout,
    /// 3: "%1 - Section %2".
    Section,
    /// 4: "%1 - Elevation %2".
    Elevation,
}

impl NameSuffix {
    fn text(self, base: &str, n: u32) -> String {
        match self {
            NameSuffix::Copied => format!("{base} Copy {n}"),
            NameSuffix::Dependent => format!("{base} - Dependent {n}"),
            NameSuffix::Callout => format!("{base} - Callout {n}"),
            NameSuffix::Section => format!("{base} - Section {n}"),
            NameSuffix::Elevation => format!("{base} - Elevation {n}"),
        }
    }
}

/// Spec VIEWTYPE-01: the third integer of the family table, so a floor plan and an area plan are both 1 and a section, an
/// elevation and a detail view are all 3 - the API names are NOT a function of it: 1 floor, structural and area plans; 2
/// ceiling plans; 3 elevations, sections and detail views; 4 3D views and walkthroughs; 5 schedules; 6 sheets; 10
/// renderings and drafting views.
pub fn native_view_type(kind: ViewKind) -> u8 {
    match kind {
        ViewKind::FloorPlan => 1,
        ViewKind::CeilingPlan => 2,
        ViewKind::Section | ViewKind::Elevation => 3,
        ViewKind::ThreeD => 4,
        ViewKind::Schedule => 5,
        ViewKind::Sheet => 6,
        ViewKind::Drafting => 10,
    }
}

pub fn native_doc_view_type(v: &DocView) -> u8 {
    match v.kind {
        DocViewKind::CalloutPlan => 1,
        DocViewKind::Section | DocViewKind::Elevation | DocViewKind::CalloutDetail => 3,
    }
}

pub fn view_type_of(m: &BimModel, view: ElementId) -> Option<u8> {
    if let Some(v) = m.views.get(&view) {
        return Some(native_view_type(v.kind));
    }
    m.docs.views.get(&view).map(native_doc_view_type)
}

/// `View.CanBePrinted`. every other non-transient view has its default drawing and viewport, so a plan, section, elevation,
/// callout, 3D view or sheet can be printed.
pub fn can_be_printed(native_view_type: u8, is_template: bool) -> bool {
    !is_template && !matches!(native_view_type, 5 | 7 | 8 | 12 | 14)
}

pub fn template_capable(native_type: u8) -> bool {
    matches!(native_type, 1 | 2 | 3 | 4 | 5 | 10 | 11 | 13)
}

/// The view types a viewport cannot show (not 5, 6, 7, 8, 12, 14).
pub fn viewport_capable(native_type: u8) -> bool {
    !matches!(native_type, 5 | 6 | 7 | 8 | 12 | 14)
}

/// Whether the view is a model view (it shows the model: plans, ceiling plans, sections, elevations, 3D views).
fn is_model_view(native_type: u8) -> bool {
    matches!(native_type, 1 | 2 | 3 | 4)
}

fn refuse(msg: &str, view: ElementId) -> Failure {
    Failure::error(codes::DOCVIEW_INVALID, msg.to_owned()).on(&[view])
}

pub fn can_duplicate(m: &BimModel, view: ElementId, option: DuplicateOption) -> bool {
    let Some(t) = view_type_of(m, view) else { return false };
    let (no_reports, no_3d, dependent) = match option {
        DuplicateOption::Duplicate => (false, false, false),
        DuplicateOption::WithDetailing => (true, false, false),
        DuplicateOption::AsDependent => (true, true, true),
    };
    if matches!(t, 7 | 12) || t == 6 {
        return false;
    }
    if no_reports && matches!(t, 5 | 8) {
        return false;
    }
    if no_3d && t == 4 {
        return false;
    }
    if dependent {
        let detail_view = m.docs.views.get(&view).map_or(false, |v| v.kind == DocViewKind::CalloutDetail);
        if !is_model_view(t) || t == 10 || detail_view {
            return false;
        }
    }
    true
}

/// The primary view of a dependent view, or `None`.
pub fn primary_of(m: &BimModel, view: ElementId) -> Option<ElementId> {
    m.docs.primary_views.get(&view).copied()
}

/// The views that depend on `primary`, in id order.
pub fn dependents_of(m: &BimModel, primary: ElementId) -> Vec<ElementId> {
    m.docs.primary_views.iter().filter(|(_, p)| **p == primary).map(|(d, _)| *d).collect()
}

fn view_name(m: &BimModel, view: ElementId) -> Option<String> {
    m.views.get(&view).map(|v| v.name.clone()).or_else(|| m.docs.views.get(&view).map(|v| v.name.clone()))
}

fn name_taken(m: &BimModel, name: &str) -> bool {
    m.views.values().any(|v| v.name == name)
        || m.docs.views.values().any(|v| v.name == name)
        || m.docs.templates.values().any(|v| v.name == name)
        || m.docs.schedules.values().any(|v| v.name == name)
        || m.docs.sheets.values().any(|s| s.name == name)
}

/// The base is the source's name (the PRIMARY view's name for the dependent suffix when the source has a primary view); the
/// first n from 1 whose candidate is unused.
pub fn name_for_copy(m: &BimModel, view: ElementId, suffix: NameSuffix) -> Option<String> {
    let base = match (suffix, primary_of(m, view)) {
        (NameSuffix::Dependent, Some(p)) => view_name(m, p)?,
        _ => view_name(m, view)?,
    };
    (1u32..).map(|n| suffix.text(&base, n)).find(|c| !name_taken(m, c))
}

/// Returns the id of the copy. Refused: "View cannot be duplicated" when the option's flags refuse the view.
pub fn duplicate_view(m: &mut BimModel, cx: &mut TxnContext<'_>, view: ElementId, option: DuplicateOption) -> Result<ElementId, Failure> {
    if view_type_of(m, view).is_none() {
        return Err(Failure::error(codes::NOT_FOUND, "No such view.").on(&[view]));
    }
    if !can_duplicate(m, view, option) {
        return Err(refuse("View cannot be duplicated.", view));
    }
    let source_primary = primary_of(m, view);
    let suffix = if option == DuplicateOption::AsDependent { NameSuffix::Dependent } else { NameSuffix::Copied };
    let name = name_for_copy(m, view, suffix).ok_or_else(|| refuse("View cannot be duplicated.", view))?;
    let id = cx.new_id();
    // the clone: the view itself, its graphics (with its template) and its phase settings
    if let Some(v) = m.views.get(&view).cloned() {
        let mut c = v;
        c.id = id;
        c.name = name;
        m.views.insert(id, c);
    } else if let Some(v) = m.docs.views.get(&view).cloned() {
        let mut c = v;
        c.id = id;
        c.name = name;
        // a copy of a section is another view of the same marker / parent; it is not one of the marker's slots
        m.docs.views.insert(id, c);
    }
    if let Some(g) = m.docs.graphics.get(&view).cloned() {
        m.docs.graphics.insert(id, g);
    }
    if let Some(p) = m.phases.views.get(&view).copied() {
        m.phases.views.insert(id, p);
    }
    // dependence: as dependent, the copy hangs on the source's primary view, else on the source; a plain duplicate of a
    // dependent view stays dependent on the same primary view; with detailing it becomes standalone
    match (option, source_primary) {
        (DuplicateOption::AsDependent, Some(p)) => {
            m.docs.primary_views.insert(id, p);
        }
        (DuplicateOption::AsDependent, None) => {
            m.docs.primary_views.insert(id, view);
        }
        (DuplicateOption::Duplicate, Some(p)) => {
            m.docs.primary_views.insert(id, p);
        }
        _ => {}
    }
    if option == DuplicateOption::WithDetailing {
        copy_view_details(m, cx, view, id);
    }
    Ok(id)
}

/// The view-specific annotation elements of the source are copied to the new view.
fn copy_view_details(m: &mut BimModel, cx: &mut TxnContext<'_>, from: ElementId, to: ElementId) {
    let notes: Vec<TextNote> = m.annot.notes.values().filter(|n| n.view == from).cloned().collect();
    for mut n in notes {
        n.id = cx.new_id();
        n.view = to;
        m.annot.notes.insert(n.id, n);
    }
    let dims: Vec<Dimension> = m.annot.dims.values().filter(|d| d.view == from).cloned().collect();
    for mut d in dims {
        d.id = cx.new_id();
        d.view = to;
        m.annot.dims.insert(d.id, d);
    }
    let tags: Vec<Tag> = m.annot.tags.values().filter(|t| t.view == from).cloned().collect();
    for mut t in tags {
        t.id = cx.new_id();
        t.view = to;
        m.annot.tags.insert(t.id, t);
    }
}

/// Forget the dependence of views that went (a deleted primary leaves its dependents standalone here).
pub fn views_deleted(m: &mut BimModel, gone: &[ElementId]) {
    for g in gone {
        m.docs.primary_views.remove(g);
    }
    m.docs.primary_views.retain(|_, p| !gone.contains(p));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops;

    fn doc() -> (crate::BimDocument, ElementId, ElementId, ElementId) {
        let mut d = crate::new_document();
        let (plan, three, sheet) = d
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let p = ops::create_view(m, cx, "P0", ViewKind::FloorPlan, Some(l), None)?;
                let t = ops::create_view(m, cx, "{3D}", ViewKind::ThreeD, None, None)?;
                let s = crate::sheets::create_sheet(m, cx, Some("A1"), None, true)?;
                Ok((p, t, s))
            })
            .value
            .unwrap();
        (d, plan, three, sheet)
    }

    #[test]
    fn the_flags_of_each_option_refuse_what_the_row_lists() {
        let (d, plan, three, sheet) = doc();
        let m = d.model();
        for o in [DuplicateOption::Duplicate, DuplicateOption::WithDetailing, DuplicateOption::AsDependent] {
            assert!(!can_duplicate(m, sheet, o), "a sheet is never duplicated ({})", o.name());
            assert!(can_duplicate(m, plan, o), "a plan is ({})", o.name());
        }
        assert!(can_duplicate(m, three, DuplicateOption::Duplicate) && can_duplicate(m, three, DuplicateOption::WithDetailing));
        assert!(!can_duplicate(m, three, DuplicateOption::AsDependent), "no dependent of a 3D view");
        assert_eq!(DuplicateOption::from_number(3), None);
    }

    #[test]
    fn a_copy_is_named_copy_n_keeps_phase_and_template_and_a_dependent_hangs_on_its_primary() {
        let (mut d, plan, _, _) = doc();
        d.transact("ph", false, |m, cx| crate::phases::set_view_phase(m, cx, plan, Some("Existing"), Some("Show Complete"))).value.unwrap();
        let t = d.transact("t", false, |m, cx| crate::docviews::create_template(m, cx, plan, Some("T"))).value.unwrap();
        d.transact("ap", false, |m, _| crate::docviews::set_view_template(m, plan, Some(t))).value.unwrap();
        let c1 = d.transact("c1", false, |m, cx| duplicate_view(m, cx, plan, DuplicateOption::Duplicate)).value.unwrap();
        let c2 = d.transact("c2", false, |m, cx| duplicate_view(m, cx, plan, DuplicateOption::Duplicate)).value.unwrap();
        assert_eq!((d.model().views[&c1].name.as_str(), d.model().views[&c2].name.as_str()), ("P0 Copy 1", "P0 Copy 2"));
        assert_eq!(crate::phases::view_phase(d.model(), c1), crate::phases::view_phase(d.model(), plan));
        assert_eq!(d.model().docs.graphics[&c1].template, Some(t));
        // as dependent: "<primary> - Dependent n"; the copy of a dependent view stays dependent on the same primary
        let dep = d.transact("dep", false, |m, cx| duplicate_view(m, cx, plan, DuplicateOption::AsDependent)).value.unwrap();
        assert_eq!(d.model().views[&dep].name, "P0 - Dependent 1");
        assert_eq!(primary_of(d.model(), dep), Some(plan));
        let dep2 = d.transact("dep2", false, |m, cx| duplicate_view(m, cx, dep, DuplicateOption::AsDependent)).value.unwrap();
        assert_eq!((d.model().views[&dep2].name.as_str(), primary_of(d.model(), dep2)), ("P0 - Dependent 2", Some(plan)));
        let dup_dep = d.transact("dd", false, |m, cx| duplicate_view(m, cx, dep, DuplicateOption::Duplicate)).value.unwrap();
        assert_eq!((d.model().views[&dup_dep].name.as_str(), primary_of(d.model(), dup_dep)), ("P0 - Dependent 1 Copy 1", Some(plan)));
        assert_eq!(dependents_of(d.model(), plan), vec![dep, dep2, dup_dep]);
        // with detailing the copy of a dependent view is standalone
        let det = d.transact("det", false, |m, cx| duplicate_view(m, cx, dep, DuplicateOption::WithDetailing)).value.unwrap();
        assert_eq!(primary_of(d.model(), det), None);
        // the refusals
        let s = d.model().docs.sheets.keys().copied().next().unwrap();
        assert!(d.transact("sheet", false, |m, cx| duplicate_view(m, cx, s, DuplicateOption::Duplicate)).value.is_none());
    }

    #[test]
    fn with_detailing_copies_the_annotations_of_the_view_and_a_plain_duplicate_does_not() {
        let (mut d, plan, _, _) = doc();
        d.transact("n", false, |m, cx| crate::dims::create_text_note(m, cx, plan, [1.0, 1.0, 0.0], "note", None, None, crate::annot::HAlign::Left)).value.unwrap();
        let plain = d.transact("p", false, |m, cx| duplicate_view(m, cx, plan, DuplicateOption::Duplicate)).value.unwrap();
        let det = d.transact("d", false, |m, cx| duplicate_view(m, cx, plan, DuplicateOption::WithDetailing)).value.unwrap();
        let on = |v: ElementId| d.model().annot.notes.values().filter(|n| n.view == v).count();
        assert_eq!((on(plan), on(plain), on(det)), (1, 0, 1));
    }

    #[test]
    fn view_type_numbers_gate_templates_and_viewports() {
        // 1, 2, 3, 4, 5, 10, 11, 13 take a template; not 5, 6, 7, 8, 12, 14 on a sheet
        assert!([1, 2, 3, 4, 5, 10, 11, 13].iter().all(|t| template_capable(*t)));
        assert!([6, 7, 8, 12, 14].iter().all(|t| !template_capable(*t)));
        assert!([1, 2, 3, 4, 10, 11, 13].iter().all(|t| viewport_capable(*t)));
        assert!([5, 6, 7, 8, 12, 14].iter().all(|t| !viewport_capable(*t)));
        assert_eq!((native_view_type(ViewKind::FloorPlan), native_view_type(ViewKind::Sheet), native_view_type(ViewKind::Drafting)), (1, 6, 10));
    }
}
