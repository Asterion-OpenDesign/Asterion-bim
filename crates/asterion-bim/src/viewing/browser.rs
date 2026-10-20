//! The Project Browser's view groups (Spec BRW-01).
//!
//! Spec BRW-01 (the reference product, the static view-family map and): every view family has a singular name, a plural
//! browser name and a sort number. A view's group is labelled with its family's plural name, plus `" (" + type name + ")"`
//! when the view type's name differs from the family's singular name; the groups are ordered by the family's sort number,
//! and a family the table does not know sorts as -1.
//!
//! Not specified, so not decided here: the order of the rules inside a group and the order of two groups of the same family
//! (the shell orders them, ASSUMED).
//!
//! Organisation filters: [`view_listed`] evaluates the filters of the document's active Views organisation
//! ([`crate::document_settings::BrowserOrganisation::filter_list`]) for one view, for the parameters whose rule the rule
//! reads (the view's type -1012106 and view family -1012109); the others are `NotEvaluated` and never decide.
//!
//! Derived, never stored: [`view_group`] and [`view_listed`] are pure functions of the model.

use crate::model::{BimModel, View, ViewKind};

/// One view family of the table: its key (the view type's system family index), its singular and plural names, and its
/// browser sort number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewFamily {
    pub key: i64,
    pub singular: &'static str,
    pub plural: &'static str,
    pub sort: i32,
}

/// Spec BRW-01: the families of the table, as read (key, singular, plural, sort number).
pub const VIEW_FAMILIES: [ViewFamily; 20] = [
    ViewFamily { key: 1, singular: "Structural Plan", plural: "Structural Plans", sort: 0 },
    ViewFamily { key: 2, singular: "Floor Plan", plural: "Floor Plans", sort: 1 },
    ViewFamily { key: 3, singular: "Ceiling Plan", plural: "Ceiling Plans", sort: 2 },
    ViewFamily { key: 4, singular: "3D View", plural: "3D Views", sort: 3 },
    ViewFamily { key: 5, singular: "Elevation", plural: "Elevations", sort: 4 },
    ViewFamily { key: 6, singular: "Section", plural: "Sections", sort: 5 },
    ViewFamily { key: 7, singular: "Detail View", plural: "Detail Views", sort: 6 },
    ViewFamily { key: 8, singular: "Rendering", plural: "Renderings", sort: 7 },
    ViewFamily { key: 9, singular: "Drafting View", plural: "Drafting Views", sort: 8 },
    ViewFamily { key: 10, singular: "Walkthrough", plural: "Walkthroughs", sort: 9 },
    ViewFamily { key: 11, singular: "Area Plan", plural: "Area Plans", sort: 10 },
    ViewFamily { key: 12, singular: "Schedule", plural: "Schedules", sort: 11 },
    ViewFamily { key: 13, singular: "Cost Report", plural: "Reports", sort: 12 },
    ViewFamily { key: 14, singular: "Sheet", plural: "Sheets", sort: 13 },
    ViewFamily { key: 15, singular: "Loads Report", plural: "Loads Reports", sort: 14 },
    ViewFamily { key: 16, singular: "Pressure Loss Report", plural: "Pressure Loss", sort: 15 },
    ViewFamily { key: 17, singular: "Legend", plural: "Legends", sort: 16 },
    ViewFamily { key: 18, singular: "Graphical Column Schedule", plural: "Graphical Column Schedules", sort: 17 },
    ViewFamily { key: 19, singular: "Analysis Report", plural: "Analysis Reports", sort: 18 },
    ViewFamily { key: 20, singular: "Panel Schedule", plural: "Panel Schedules", sort: 19 },
];

/// Spec BRW-01: the sort number of an unknown family.
pub const UNKNOWN_FAMILY_SORT: i32 = -1;

/// The table entry of a family key.
pub fn family(key: i64) -> Option<&'static ViewFamily> {
    VIEW_FAMILIES.iter().find(|f| f.key == key)
}

/// The family key of an engine view kind (the engine's kinds are a subset of the table's families).
pub fn family_key_of_kind(kind: ViewKind) -> i64 {
    match kind {
        ViewKind::FloorPlan => 2,
        ViewKind::CeilingPlan => 3,
        ViewKind::ThreeD => 4,
        ViewKind::Section => 6,
        ViewKind::Elevation => 5,
        ViewKind::Drafting => 9,
        ViewKind::Schedule => 12,
        ViewKind::Sheet => 14,
    }
}

/// The browser group of one view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewGroup {
    /// The family key: the imported view type's family, else the one of the engine's kind.
    pub family: i64,
    /// The view type's name when the document stored it.
    pub type_name: Option<String>,
    /// The group's label (Spec BRW-01).
    pub label: String,
    /// The group's sort number (Spec BRW-01).
    pub sort: i32,
}

/// Spec BRW-01: the label of a family's group for a view type of name `type_name`. A view whose type name is not known (an
/// engine view has no view type) takes the family's plural alone: ASSUMED that its type is named as the family, as the
/// default types of a new project are.
pub fn group_label(f: &ViewFamily, type_name: Option<&str>) -> String {
    match type_name {
        Some(t) if t != f.singular => format!("{} ({})", f.plural, t),
        _ => f.plural.to_owned(),
    }
}

/// The browser group of `view` (Spec BRW-01).
pub fn view_group(model: &BimModel, view: &View) -> ViewGroup {
    let source = model.settings.view_sources.get(&view.id);
    let key = source.and_then(|s| s.family).unwrap_or_else(|| family_key_of_kind(view.kind));
    let type_name = source.and_then(|s| s.type_name.clone());
    match family(key) {
        Some(f) => ViewGroup { family: key, label: group_label(f, type_name.as_deref()), sort: f.sort, type_name },
        // an unknown family: its sort is -1; its label is the type name, or the engine's kind
        None => ViewGroup {
            family: key,
            label: type_name.clone().unwrap_or_else(|| view.kind.name().to_owned()),
            sort: UNKNOWN_FAMILY_SORT,
            type_name,
        },
    }
}

/// The result of one organisation filter for one view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterResult {
    /// The view passes the filter.
    Satisfied,
    /// The view does not pass: it is not listed in the organisation.
    Failed,
    /// The rule for this parameter (or the data it needs) is not specified: the filter decides nothing.
    NotEvaluated,
}

/// The parameter ids of the filters whose rule is read.
pub mod filter_param {
    /// The view's type element (compared with the integer value).
    pub const VIEW_TYPE: i64 = -1012106;
    /// The view family number of the view's type.
    pub const VIEW_FAMILY: i64 = -1012109;
    /// Print set: whether a viewport of the sheets of a view-sheet set shows the view (not specified: the engine has no
    /// print sets).
    pub const PRINT_SET: i64 = -1012107;
}

/// For the rules the rule reads: id -1 is satisfied; -1012106 compares the id of the view's type element with the integer
/// value (equals / differs); -1012109 compares the view family number of the type. Every other parameter, the print set, a
/// value of another kind than int, and a view with no stored type are [`FilterResult::NotEvaluated`].
pub fn filter_satisfied(model: &BimModel, view: &View, f: &crate::document_settings::BrowserFilter) -> FilterResult {
    if f.parameter_id == -1 {
        return FilterResult::Satisfied;
    }
    let source = model.settings.view_sources.get(&view.id);
    let subject = match f.parameter_id {
        filter_param::VIEW_TYPE => source.and_then(|s| s.type_source_id),
        filter_param::VIEW_FAMILY => source.and_then(|s| s.family),
        _ => None,
    };
    let (Some(subject), 1) = (subject, f.value_kind) else { return FilterResult::NotEvaluated };
    let equal = subject == f.value_int;
    match f.relation {
        0 if equal => FilterResult::Satisfied,
        1 if !equal => FilterResult::Satisfied,
        0 | 1 => FilterResult::Failed,
        _ => FilterResult::NotEvaluated,
    }
}

/// Whether `view` is listed in the Views node under the document's active organisation (every filter must be satisfied; no
/// filters = listed). `Some(false)` when a filter that can be evaluated fails; `Some(true)` when every filter is satisfied;
/// `None` when no evaluated filter fails and at least one filter could not be evaluated (the reference product's answer is
/// not known).
pub fn view_listed(model: &BimModel, view: &View) -> Option<bool> {
    let Some(org) = &model.settings.views_browser else { return Some(true) };
    let mut unknown = false;
    for f in &org.filter_list {
        match filter_satisfied(model, view, f) {
            FilterResult::Failed => return Some(false),
            FilterResult::NotEvaluated => unknown = true,
            FilterResult::Satisfied => {}
        }
    }
    if unknown {
        None
    } else {
        Some(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The view-type and view-family filters; id -1 satisfied; any other parameter is not evaluated; an evaluated failure
    /// decides even when another filter is not evaluated.
    #[test]
    fn organisation_filters_follow_the_table() {
        use crate::document_settings::{BrowserFilter, BrowserOrganisation, ViewSource};
        let mut doc = crate::new_document();
        doc.transact("plan", false, |m, cx| {
            let l = crate::ops::create_level(m, cx, "L1", 0.0)?;
            crate::ops::create_view(m, cx, "L1", ViewKind::FloorPlan, Some(l), None)
        });
        let mut m = doc.model().clone();
        let v = m.views.values().find(|v| v.kind == ViewKind::FloorPlan).unwrap().clone();
        m.settings.view_sources.insert(
            v.id,
            ViewSource {
                source_id: 1,
                type_source_id: Some(69332),
                type_name: Some("Floor Plan".into()),
                family: Some(2),
                template_source_id: None,
                discipline: 1,
                detail_level_raw: 0,
            },
        );
        let filter = |parameter_id: i64, relation: i64, value_kind: i64, value_int: i64| BrowserFilter {
            parameter_id,
            relation,
            value_kind,
            value_int,
            value_str: String::new(),
            value_elem_id: -1,
        };
        let org = |filters: Vec<BrowserFilter>| BrowserOrganisation {
            source_id: 1,
            name: Some("o".into()),
            folders: vec![],
            filters: filters.len(),
            filter_list: filters,
            sort_parameter: None,
            sort_ascending: None,
        };
        // no organisation, and no filters: listed
        assert_eq!(view_listed(&m, &v), Some(true));
        m.settings.views_browser = Some(org(vec![]));
        assert_eq!(view_listed(&m, &v), Some(true));
        let f = |filters| {
            let mut m2 = m.clone();
            m2.settings.views_browser = Some(org(filters));
            view_listed(&m2, &v)
        };
        assert_eq!(f(vec![filter(-1, 0, 0, 0)]), Some(true), "id -1 is satisfied");
        assert_eq!(f(vec![filter(filter_param::VIEW_TYPE, 0, 1, 69332)]), Some(true));
        assert_eq!(f(vec![filter(filter_param::VIEW_TYPE, 1, 1, 69332)]), Some(false), "type differs: fails");
        assert_eq!(f(vec![filter(filter_param::VIEW_FAMILY, 0, 1, 6)]), Some(false));
        assert_eq!(f(vec![filter(filter_param::VIEW_FAMILY, 1, 1, 6)]), Some(true));
        // a parameter whose rule is not specified decides nothing; a failure elsewhere still decides
        assert_eq!(f(vec![filter(-1005207, 0, 0, 0)]), None);
        assert_eq!(f(vec![filter(-1005207, 0, 0, 0), filter(filter_param::VIEW_FAMILY, 0, 1, 6)]), Some(false));
        assert_eq!(f(vec![filter(filter_param::PRINT_SET, 0, 1, 3)]), None);
        // a view with no stored type cannot be evaluated against the type filter
        let other = View { id: asterion_core::ElementId(9999), ..v.clone() };
        let mut m3 = m.clone();
        m3.settings.views_browser = Some(org(vec![filter(filter_param::VIEW_TYPE, 0, 1, 69332)]));
        assert_eq!(view_listed(&m3, &other), None);
    }

    #[test]
    fn the_table_orders_the_families_and_labels_them_with_the_type_when_it_differs() {
        let fp = family(2).unwrap();
        assert_eq!(group_label(fp, Some("Floor Plan")), "Floor Plans");
        assert_eq!(group_label(fp, Some("Enlarged Plan")), "Floor Plans (Enlarged Plan)");
        assert_eq!(group_label(fp, None), "Floor Plans");
        let order: Vec<i32> = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
            .iter()
            .map(|k| family(*k).unwrap().sort)
            .collect();
        assert_eq!(order, (0..=10).collect::<Vec<i32>>());
        assert_eq!(family(17).unwrap().sort, 16);
        assert!(family(9999).is_none());
        // every engine kind maps to a family of the table
        for k in ViewKind::ALL {
            assert!(family(family_key_of_kind(k)).is_some(), "{k:?}");
        }
    }

    /// (the formatted titles of a floor plan and a ceiling plan differ): `create_view` accepts a ceiling plan named like a
    /// floor plan, and refuses a second floor plan of that name.
    #[test]
    fn a_ceiling_plan_may_take_a_floor_plans_name_at_creation() {
        let mut doc = crate::new_document();
        let out = doc.transact("plans", false, |m, cx| {
            let l = crate::ops::create_level(m, cx, "L1", 0.0)?;
            crate::ops::create_view(m, cx, "L1", ViewKind::FloorPlan, Some(l), None)?;
            crate::ops::create_view(m, cx, "L1", ViewKind::CeilingPlan, Some(l), None)?;
            Ok(l)
        });
        let l = out.value.expect("a ceiling plan named like the floor plan is accepted");
        let again = doc.transact("plan", false, |m, cx| crate::ops::create_view(m, cx, "L1", ViewKind::FloorPlan, Some(l), None));
        assert!(again.value.is_none(), "a second floor plan of that name is refused");
    }

    #[test]
    fn an_engine_view_takes_its_kinds_family_and_an_imported_one_its_stored_type() {
        let mut doc = crate::new_document();
        doc.transact("plan", false, |m, cx| {
            let l = crate::ops::create_level(m, cx, "L1", 0.0)?;
            crate::ops::create_view(m, cx, "L1", ViewKind::FloorPlan, Some(l), None)
        });
        let m = doc.model();
        let v = m.views.values().find(|v| v.kind == ViewKind::FloorPlan).unwrap().clone();
        let g = view_group(m, &v);
        assert_eq!((g.label.as_str(), g.sort), ("Floor Plans", 1));
        let id = v.id;
        doc.transact("source", false, |m, _| {
            m.settings.view_sources.insert(
                id,
                crate::document_settings::ViewSource {
                    source_id: 1,
                    type_source_id: None,
                    type_name: Some("Working".to_owned()),
                    family: Some(3),
                    template_source_id: None,
                    discipline: 1,
                    detail_level_raw: 0,
                },
            );
            Ok(())
        });
        let g = view_group(doc.model(), &v);
        assert_eq!((g.label.as_str(), g.sort), ("Ceiling Plans (Working)", 2));
    }
}
