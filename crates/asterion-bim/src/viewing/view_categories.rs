//! The categories the Visibility/Graphics dialog lists, which element belongs to which, and what the dialog reads and
//! writes in ONE call each (`dialog_state`, `apply`).
//!
//! # Status of every rule (founder rule 2026-10-14: nothing without its source)
//!
//! * STORAGE and the per-view rules come from specification rules: the hidden categories of a view are ONE set of category
//!   ids held by the view's exclusion manager; a category a controlling view template locks cannot be hidden or overridden.
//! * The LIST of categories and their tree is an ENGINE CHOICE: no specification rule names the categories of the reference
//!   product nor their subcategories ("the names of the category classes and of the built-in category ids"). The engine
//!   lists exactly the categories whose elements it models, plus the ones its template defaults hide (OBSERVED). The names
//!   are the labels the reference dialog shows (OBSERVED section 32: g1-vg, g2-annot). Which tab a category sits in follows
//!   those dumps for Grids, Levels, Sections and Dimensions (annotation); the placement of the others is ASSUMED. The
//!   subcategory `<Room Separation>` of `Lines` is ASSUMED.
//! * ENGINE CHOICE: four classes, one per tab; a hidden class hides every category of its tab.
//! * A category that is on by default in the template and that the person shows (`shown_categories`) is the engine's way of
//!   storing "the template's default set minus this category": the reference stores only the set. ENGINE CHOICE.

use crate::docs::Rgb;
use crate::docviews::{self, AppliedFilter, Ogs, ViewGraphics};
use crate::model::{BimModel, SlabRole};
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The four tabs of the dialog that list categories (the fifth, Filters, lists the view's filters).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tab {
    Model,
    Annotation,
    Analytical,
    Imported,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Model, Tab::Annotation, Tab::Analytical, Tab::Imported];

    pub fn code(self) -> &'static str {
        match self {
            Tab::Model => "model",
            Tab::Annotation => "annotation",
            Tab::Analytical => "analytical",
            Tab::Imported => "imported",
        }
    }

    pub fn parse(s: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.code() == s)
    }
}

/// One row of the dialog's table.
#[derive(Clone, Copy, Debug)]
pub struct CategoryDef {
    pub name: &'static str,
    pub tab: Tab,
    /// The parent row of a subcategory.
    pub parent: Option<&'static str>,
    pub discipline: &'static str,
    /// Hidden in a new view by the template (OBSERVED).
    pub default_hidden: bool,
    /// Why the category cannot be overridden, if it cannot.
    pub no_override: Option<&'static str>,
}

const ARCH: &str = "Architecture";

const fn row(name: &'static str, tab: Tab) -> CategoryDef {
    CategoryDef { name, tab, parent: None, discipline: ARCH, default_hidden: false, no_override: None }
}

const fn hidden_row(name: &'static str, tab: Tab) -> CategoryDef {
    CategoryDef { name, tab, parent: None, discipline: ARCH, default_hidden: true, no_override: None }
}

/// The dialog's rows, in the order the dialog lists them (alphabetical within a tab, a subcategory right after its parent).
pub const TABLE: &[CategoryDef] = &[
    // Model Categories
    row("Ceilings", Tab::Model),
    row("Columns", Tab::Model),
    row("Curtain Panels", Tab::Model),
    row("Curtain Wall Mullions", Tab::Model),
    row("Doors", Tab::Model),
    row("Floors", Tab::Model),
    row("Lines", Tab::Model),
    CategoryDef { name: "<Room Separation>", tab: Tab::Model, parent: Some("Lines"), discipline: ARCH, default_hidden: false, no_override: None },
    hidden_row("Mass", Tab::Model),
    hidden_row("Parts", Tab::Model),
    row("Railings", Tab::Model),
    row("Roofs", Tab::Model),
    row("Rooms", Tab::Model),
    row("Shaft Openings", Tab::Model),
    row("Stairs", Tab::Model),
    row("Toposolid", Tab::Model),
    row("Walls", Tab::Model),
    row("Windows", Tab::Model),
    // Annotation Categories
    row("Callouts", Tab::Annotation),
    row("Dimensions", Tab::Annotation),
    row("Door Tags", Tab::Annotation),
    row("Elevations", Tab::Annotation),
    CategoryDef {
        name: "Grids",
        tab: Tab::Annotation,
        parent: None,
        discipline: ARCH,
        default_hidden: false,
        // OBSERVED (one case): an override of the Grids category reads back as none. ENGINE CHOICE.
        no_override: Some("the reference's override of the Grids category read back as none in the only case measured (OQ-GRID-OVERRIDE)"),
    },
    row("Levels", Tab::Annotation),
    row("Reference Planes", Tab::Annotation),
    row("Room Tags", Tab::Annotation),
    row("Sections", Tab::Annotation),
    row("Spot Elevations", Tab::Annotation),
    row("Text Notes", Tab::Annotation),
    row("Wall Tags", Tab::Annotation),
    row("Window Tags", Tab::Annotation),
    // Analytical Model Categories (the three the template hides, OBSERVED)
    hidden_row("Analytical Nodes", Tab::Analytical),
    hidden_row("Analytical Spaces", Tab::Analytical),
    hidden_row("Analytical Surfaces", Tab::Analytical),
];

/// The row of a category name.
pub fn def(name: &str) -> Option<&'static CategoryDef> {
    TABLE.iter().find(|d| d.name == name)
}

/// The parent chain of a category, nearest first.
pub fn ancestors(name: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    let mut cur = def(name).and_then(|d| d.parent);
    while let Some(p) = cur {
        out.push(p);
        cur = def(p).and_then(|d| d.parent);
        if out.len() > 8 {
            break;
        }
    }
    out
}

/// The top-level category of a row (itself when it has no parent).
pub fn root_of(name: &'static str) -> &'static str {
    ancestors(name).last().copied().unwrap_or(name)
}

/// The direct subcategories of a category.
pub fn children(name: &str) -> Vec<&'static CategoryDef> {
    TABLE.iter().filter(|d| d.parent == Some(name)).collect()
}

/// The disciplines the table covers, in order of first appearance.
pub fn disciplines() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for d in TABLE.iter() {
        if !out.contains(&d.discipline) {
            out.push(d.discipline);
        }
    }
    out
}

// ------------------------------------------------------------------------------------------------ element -> category

/// The tag category a tag of a host in `host` takes: `<host category singular> Tags`, for the hosts the table lists.
fn tag_category(host: Option<&'static str>) -> Option<&'static str> {
    Some(match host? {
        "Doors" => "Door Tags",
        "Windows" => "Window Tags",
        "Walls" => "Wall Tags",
        "Rooms" => "Room Tags",
        _ => return None,
    })
}

/// The row an element belongs to (the deepest one: a room separation is `<Room Separation>`, whose parent is `Lines`), or `None`
/// for an element that is in no listed category. A curtain wall is a wall (its panels and mullions are parts of it, see
/// [`part_filter`]).
pub fn category_of(m: &BimModel, id: ElementId) -> Option<&'static str> {
    if m.walls.contains_key(&id) {
        return Some("Walls");
    }
    if let Some(f) = m.floors.get(&id) {
        return Some(match f.role {
            SlabRole::Floor => "Floors",
            SlabRole::Ceiling => "Ceilings",
            SlabRole::Roof => "Roofs",
        });
    }
    if let Some(i) = m.hosted_instances.get(&id) {
        return Some(match m.hosted_types.get(&i.type_id).map(|t| t.kind) {
            Some(crate::model::HostedKind::Window) => "Windows",
            _ => "Doors",
        });
    }
    if m.columns.contains_key(&id) {
        return Some("Columns");
    }
    if m.rooms.contains_key(&id) {
        return Some("Rooms");
    }
    if m.room_separations.contains_key(&id) {
        return Some("<Room Separation>");
    }
    if m.stairs.stairs.contains_key(&id) || m.stairs.runs.contains_key(&id) || m.stairs.landings.contains_key(&id) {
        return Some("Stairs");
    }
    if m.stairs.railings.contains_key(&id) || m.stairs_scene.path_railings.contains_key(&id) {
        return Some("Railings");
    }
    if m.docs.toposolids.contains_key(&id) {
        return Some("Toposolid");
    }
    if m.shafts.contains_key(&id) {
        return Some("Shaft Openings");
    }
    if m.annot.curves.get(&id).map_or(false, |c| !c.reference_line) {
        return Some("Lines");
    }
    if m.grids.contains_key(&id) {
        return Some("Grids");
    }
    if m.levels.contains_key(&id) {
        return Some("Levels");
    }
    if let Some(v) = m.docs.views.get(&id) {
        return Some(match v.kind {
            docviews::DocViewKind::Section => "Sections",
            docviews::DocViewKind::Elevation => "Elevations",
            docviews::DocViewKind::CalloutPlan | docviews::DocViewKind::CalloutDetail => "Callouts",
        });
    }
    if m.docs.markers.contains_key(&id) {
        return Some("Elevations");
    }
    if m.annot.dims.contains_key(&id) {
        return Some("Dimensions");
    }
    if m.annot.notes.contains_key(&id) {
        return Some("Text Notes");
    }
    if m.annot.ref_planes.contains_key(&id) {
        return Some("Reference Planes");
    }
    if m.base_points.spots.contains_key(&id) {
        return Some("Spot Elevations");
    }
    if let Some(t) = m.annot.tags.get(&id) {
        return tag_category(category_of(m, t.host).map(root_of));
    }
    None
}

/// Every element of the model that is in a listed category, with its row (one pass; the shell asks it once per dialog).
pub fn for_each_element(m: &BimModel, mut f: impl FnMut(ElementId, &'static str)) {
    let mut put = |id: ElementId| {
        if let Some(c) = category_of(m, id) {
            f(id, c);
        }
    };
    m.walls.keys().for_each(|i| put(*i));
    m.floors.keys().for_each(|i| put(*i));
    m.hosted_instances.keys().for_each(|i| put(*i));
    m.columns.keys().for_each(|i| put(*i));
    m.rooms.keys().for_each(|i| put(*i));
    m.room_separations.keys().for_each(|i| put(*i));
    m.stairs.stairs.keys().for_each(|i| put(*i));
    m.stairs.railings.keys().for_each(|i| put(*i));
    m.stairs_scene.path_railings.keys().for_each(|i| put(*i));
    m.docs.toposolids.keys().for_each(|i| put(*i));
    m.shafts.keys().for_each(|i| put(*i));
    m.annot.curves.keys().for_each(|i| put(*i));
    m.grids.keys().for_each(|i| put(*i));
    m.levels.keys().for_each(|i| put(*i));
    m.docs.views.keys().for_each(|i| put(*i));
    m.docs.markers.keys().for_each(|i| put(*i));
    m.annot.dims.keys().for_each(|i| put(*i));
    m.annot.notes.keys().for_each(|i| put(*i));
    m.annot.ref_planes.keys().for_each(|i| put(*i));
    m.base_points.spots.keys().for_each(|i| put(*i));
    m.annot.tags.keys().for_each(|i| put(*i));
}

/// How many elements of the document are in each listed row. `Curtain Panels` and `Curtain Wall Mullions` count the curtain walls
/// that carry them (their parts are not elements of the engine).
pub fn counts(m: &BimModel) -> BTreeMap<&'static str, usize> {
    let mut out: BTreeMap<&'static str, usize> = BTreeMap::new();
    for_each_element(m, |_, c| *out.entry(c).or_insert(0) += 1);
    let curtains = m.walls.keys().filter(|w| m.is_curtain_wall(**w)).count();
    if curtains > 0 {
        out.insert("Curtain Panels", curtains);
        out.insert("Curtain Wall Mullions", curtains);
    }
    out
}

// ------------------------------------------------------------------------------------------------ what is hidden

/// Whether the row itself is hidden in `g` (its own flag; the template's default for the categories it hides).
pub fn own_hidden(g: &ViewGraphics, name: &str) -> bool {
    if g.hidden_categories.contains(name) {
        return true;
    }
    def(name).map_or(false, |d| d.default_hidden) && !g.shown_categories.contains(name)
}

/// Whether the row is hidden in `g`: itself, a parent, or its whole class.
pub fn is_hidden(g: &ViewGraphics, name: &str) -> bool {
    if let Some(d) = def(name) {
        if g.hidden_classes.contains(d.tab.code()) {
            return true;
        }
    }
    own_hidden(g, name) || ancestors(name).into_iter().any(|a| own_hidden(g, a))
}

/// Whether anything in `g` hides a category (the quick test the drawings make before walking the elements).
pub fn hides_any(g: &ViewGraphics) -> bool {
    !g.hidden_categories.is_empty() || !g.hidden_classes.is_empty()
}

/// Which parts of a curtain wall a view draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartFilter {
    pub panels: bool,
    pub mullions: bool,
}

impl PartFilter {
    pub const ALL: PartFilter = PartFilter { panels: true, mullions: true };

    pub fn is_all(self) -> bool {
        self.panels && self.mullions
    }
}

/// The parts of a curtain wall the view's categories keep.
pub fn part_filter(m: &BimModel, view: ElementId) -> PartFilter {
    let g = docviews::effective_graphics(m, view);
    PartFilter { panels: !is_hidden(&g, "Curtain Panels"), mullions: !is_hidden(&g, "Curtain Wall Mullions") }
}

// ------------------------------------------------------------------------------------------------ the dialog's read

/// One category row as the dialog reads it.
#[derive(Clone, Debug, Serialize)]
pub struct CategoryRow {
    pub name: &'static str,
    pub tab: Tab,
    pub parent: Option<&'static str>,
    pub discipline: &'static str,
    /// The row's own visibility flag is off (the template's default included).
    pub hidden: bool,
    /// Hidden because a parent or the whole class is hidden.
    pub hidden_by_parent: bool,
    pub default_hidden: bool,
    pub can_hide: bool,
    pub can_override: bool,
    /// Why the row cannot be hidden or overridden (empty when it can).
    pub locked_reason: String,
    /// Elements of the document in the row.
    pub elements: usize,
    /// The stored override; `None` when the category has none.
    pub ogs: Option<Ogs>,
}

/// One applied filter as the dialog reads it.
#[derive(Clone, Debug, Serialize)]
pub struct FilterRow {
    pub filter: u64,
    pub name: String,
    pub categories: Vec<String>,
    pub visible: bool,
    pub enabled: bool,
    pub ogs: Ogs,
}

/// Everything the dialog shows for one view, read on each call.
#[derive(Clone, Debug, Serialize)]
pub struct DialogState {
    pub view: u64,
    pub templated: bool,
    pub template: u64,
    pub template_name: String,
    /// The view whose hidden categories and filters these are (the primary view of a dependent view).
    pub owner: u64,
    /// Tabs whose whole class is hidden (`Show model categories in this view` off).
    pub hidden_classes: Vec<&'static str>,
    pub disciplines: Vec<&'static str>,
    pub categories: Vec<CategoryRow>,
    pub filters: Vec<FilterRow>,
    pub document_filters: Vec<DocFilter>,
    pub fill_patterns: Vec<&'static str>,
    pub line_patterns: Vec<&'static str>,
    pub line_weight_max: i32,
}

#[derive(Clone, Debug, Serialize)]
pub struct DocFilter {
    pub id: u64,
    pub name: String,
}

/// The pen numbers a line weight override takes (1..=16).
pub const LINE_WEIGHT_MAX: i32 = 16;

/// What the dialog shows for a view or a view template. `Err` when the id is neither.
pub fn dialog_state(m: &BimModel, view: ElementId) -> Result<DialogState, Failure> {
    if !(m.views.contains_key(&view) || m.docs.views.contains_key(&view) || m.docs.templates.contains_key(&view)) {
        return Err(Failure::error(crate::spec::codes::NOT_FOUND, format!("No such view {}.", view.0)).on(&[view]));
    }
    let g = docviews::effective_graphics(m, view);
    let templated = docviews::view_has_template(m, view);
    let tmpl = m.docs.graphics.get(&view).and_then(|g| g.template);
    let counts = counts(m);
    let categories = TABLE
        .iter()
        .map(|d| {
            let by_parent = is_hidden(&g, d.name) && !own_hidden(&g, d.name);
            let locked = if templated {
                "the view template controls the visibility and graphic overrides of the categories".to_owned()
            } else {
                String::new()
            };
            CategoryRow {
                name: d.name,
                tab: d.tab,
                parent: d.parent,
                discipline: d.discipline,
                hidden: own_hidden(&g, d.name),
                hidden_by_parent: by_parent,
                default_hidden: d.default_hidden,
                can_hide: !templated,
                can_override: !templated && d.no_override.is_none(),
                locked_reason: if !locked.is_empty() { locked } else { d.no_override.map(str::to_owned).unwrap_or_default() },
                elements: counts.get(d.name).copied().unwrap_or(0),
                ogs: g.category_overrides.get(d.name).cloned(),
            }
        })
        .collect();
    let filters = g
        .filters
        .iter()
        .map(|af| {
            let f = m.docs.filters.get(&af.filter);
            FilterRow {
                filter: af.filter.0,
                name: f.map(|f| f.name.clone()).unwrap_or_default(),
                categories: f.map(|f| f.categories.clone()).unwrap_or_default(),
                visible: af.visible,
                enabled: af.enabled,
                ogs: af.ogs.clone(),
            }
        })
        .collect();
    Ok(DialogState {
        view: view.0,
        templated,
        template: tmpl.map_or(0, |t| t.0),
        template_name: tmpl.and_then(|t| m.docs.templates.get(&t)).map(|t| t.name.clone()).unwrap_or_default(),
        owner: docviews::graphics_owner(m, view).0,
        hidden_classes: Tab::ALL.into_iter().filter(|t| g.hidden_classes.contains(t.code())).map(Tab::code).collect(),
        disciplines: disciplines(),
        categories,
        filters,
        document_filters: m.docs.filters.values().map(|f| DocFilter { id: f.id.0, name: f.name.clone() }).collect(),
        fill_patterns: crate::docs::FILL_PATTERNS.to_vec(),
        line_patterns: crate::docs::LINE_PATTERNS.to_vec(),
        line_weight_max: LINE_WEIGHT_MAX,
    })
}

// ------------------------------------------------------------------------------------------------ the dialog's write

/// One category the person changed.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CategoryChange {
    pub name: String,
    /// The new visibility flag (`true` hidden), when it changed.
    #[serde(default)]
    pub hidden: Option<bool>,
    /// The new override, when it changed; ignored when `reset_ogs`.
    #[serde(default)]
    pub ogs: Option<Ogs>,
    /// Remove the category's override.
    #[serde(default)]
    pub reset_ogs: bool,
}

/// One entry of the view's filter list after the edit.
#[derive(Clone, Debug, Deserialize)]
pub struct FilterEntry {
    pub filter: u64,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub ogs: Ogs,
}

fn yes() -> bool {
    true
}

/// What OK / Apply sends: only what changed.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ApplyRequest {
    #[serde(default)]
    pub categories: Vec<CategoryChange>,
    /// The class flags that changed: tab code -> hidden.
    #[serde(default)]
    pub classes: BTreeMap<String, bool>,
    /// The complete ordered filter list, when the Filters tab changed (`None`: untouched).
    #[serde(default)]
    pub filters: Option<Vec<FilterEntry>>,
}

impl ApplyRequest {
    pub fn is_empty(&self) -> bool {
        self.categories.is_empty() && self.classes.is_empty() && self.filters.is_none()
    }
}

fn bad(msg: &str) -> Failure {
    Failure::error(crate::spec::codes::DOCVIEW_INVALID, msg.to_owned())
}

/// Writes the dialog's changes into the view. Called inside ONE transaction, so the whole edit is one undo step: any refusal
/// fails the call and the transaction rolls back. Returns how many things changed.
///
/// A category's flag and override follow the engine's own rules: a view with a template refuses both; a category that is
/// not in the table is refused; a hidden flag set on a parent is also set on its subcategories, as the dialog's check box
/// does (the stored set holds the rules individually); the filter list replaces the view's list in the order given (Spec
/// FILTER-03: the order decides the override stack).
pub fn apply(m: &mut BimModel, _cx: &mut TxnContext<'_>, view: ElementId, req: &ApplyRequest) -> Result<usize, Failure> {
    let mut changed = 0usize;
    if req.is_empty() {
        return Ok(0);
    }
    if !(m.views.contains_key(&view) || m.docs.views.contains_key(&view) || m.docs.templates.contains_key(&view)) {
        return Err(Failure::error(crate::spec::codes::NOT_FOUND, format!("No such view {}.", view.0)).on(&[view]));
    }
    // parents before their subcategories: a parent's flag is written on its subcategories, which the request may then set again
    let mut order: Vec<&CategoryChange> = req.categories.iter().collect();
    order.sort_by_key(|c| ancestors(&c.name).len());
    for c in order {
        let Some(d) = def(&c.name) else {
            return Err(bad("categoryId does not correspond to a Category."));
        };
        if let Some(h) = c.hidden {
            let mut names = vec![d.name];
            names.extend(children(d.name).into_iter().map(|k| k.name));
            for n in names {
                docviews::set_category_hidden_in_view(m, view, n, h)?;
            }
            changed += 1;
        }
        if c.reset_ogs {
            docviews::set_category_overrides(m, view, d.name, Ogs::default())?;
            changed += 1;
        } else if let Some(o) = &c.ogs {
            if let Some(why) = d.no_override {
                return Err(bad(&format!("Category cannot be overridden: {why}.")));
            }
            validate_ogs(o)?;
            docviews::set_category_overrides(m, view, d.name, o.clone())?;
            changed += 1;
        }
    }
    for (tab, hidden) in &req.classes {
        let Some(t) = Tab::parse(tab) else {
            return Err(bad("Unknown category class."));
        };
        docviews::set_class_hidden(m, view, t, *hidden)?;
        changed += 1;
    }
    if let Some(list) = &req.filters {
        let mut seen = BTreeSet::new();
        let mut entries = Vec::with_capacity(list.len());
        for e in list {
            let id = ElementId(e.filter);
            if !m.docs.filters.contains_key(&id) {
                return Err(Failure::error(crate::spec::codes::NOT_FOUND, "No such filter.".to_owned()).on(&[id]));
            }
            if !seen.insert(e.filter) {
                return Err(bad("Filter is already applied to the view."));
            }
            validate_ogs(&e.ogs)?;
            entries.push(AppliedFilter { filter: id, visible: e.visible, ogs: e.ogs.clone(), enabled: e.enabled });
        }
        docviews::set_filter_list(m, view, entries)?;
        changed += 1;
    }
    Ok(changed)
}

// ------------------------------------------------------------------------------------------------ override by element

#[derive(Clone, Debug, Serialize)]
pub struct ElementState {
    pub element: u64,
    /// The row of the element's category (`None`: the element is in no listed category and cannot be overridden).
    pub category: Option<&'static str>,
    pub can_override: bool,
    /// Why it cannot be overridden (empty when it can).
    pub locked_reason: String,
    /// The stored override of the element in this view; `None` when it has none.
    pub ogs: Option<Ogs>,
    /// The view hides the element now (by Hide in View, a category, a filter ...): the `Visible` box of the dialog is a reading, not a control.
    pub hidden: bool,
    pub fill_patterns: Vec<&'static str>,
    pub line_patterns: Vec<&'static str>,
    pub line_weight_max: i32,
}

/// What the dialog shows for one element in one view.
pub fn element_state(m: &BimModel, view: ElementId, element: ElementId) -> Result<ElementState, Failure> {
    if !(m.views.contains_key(&view) || m.docs.views.contains_key(&view)) {
        return Err(Failure::error(crate::spec::codes::NOT_FOUND, format!("No such view {}.", view.0)).on(&[view]));
    }
    let category = category_of(m, element);
    let templated = docviews::view_has_template(m, view);
    let locked_reason = if category.is_none() {
        "the engine lists no category for this element".to_owned()
    } else if templated {
        "the view template's graphics rule this view; the engine reads the template's graphics while one is assigned (OQ-VIEW-TEMPLATE)".to_owned()
    } else {
        String::new()
    };
    let own = m.docs.graphics.get(&view).and_then(|g| g.element_overrides.get(&element)).cloned();
    Ok(ElementState {
        element: element.0,
        category,
        can_override: locked_reason.is_empty(),
        locked_reason,
        ogs: own,
        hidden: docviews::hidden_by_graphics(m, view, element),
        fill_patterns: crate::docs::FILL_PATTERNS.to_vec(),
        line_patterns: crate::docs::LINE_PATTERNS.to_vec(),
        line_weight_max: LINE_WEIGHT_MAX,
    })
}

/// Writes (`Some`) or clears (`None`) the graphic override of every element in `view`: one call inside ONE transaction, any
/// refusal fails it all. Settings equal to the default clear the element's entry. The engine only overrides elements of a
/// listed category and never in a view with a template (ENGINE CHOICE, see [`element_state`]).
pub fn apply_elements(m: &mut BimModel, _cx: &mut TxnContext<'_>, view: ElementId, elements: &[ElementId], ogs: Option<Ogs>) -> Result<usize, Failure> {
    if elements.is_empty() {
        return Err(bad("No element to override."));
    }
    if let Some(o) = &ogs {
        validate_ogs(o)?;
    }
    for e in elements {
        let st = element_state(m, view, *e)?;
        if !st.can_override {
            return Err(bad(&format!("Element cannot be overridden: {}.", st.locked_reason)));
        }
    }
    for e in elements {
        docviews::set_element_graphics_overrides(m, view, *e, ogs.clone().unwrap_or_default())?;
    }
    Ok(elements.len())
}

/// The value ranges an override takes: pen numbers 1..=16 or -1 (not overridden), transparency 0..=100, detail level 0..=3, and
/// the patterns of the document (checked by [`docviews::check_ogs`]).
pub fn validate_ogs(o: &Ogs) -> Result<(), Failure> {
    for w in [o.projection_line_weight, o.cut_line_weight] {
        if w != -1 && !(1..=LINE_WEIGHT_MAX).contains(&w) {
            return Err(bad(&format!("A line weight is a pen number 1 to {LINE_WEIGHT_MAX}, or -1 for none.")));
        }
    }
    if !(0..=100).contains(&o.surface_transparency) {
        return Err(bad("A surface transparency is 0 to 100."));
    }
    if !(0..=3).contains(&o.detail_level) {
        return Err(bad("A detail level override is 0 (by view), 1 coarse, 2 medium or 3 fine."));
    }
    docviews::check_ogs(o)
}

/// A colour as the dialog writes it.
pub type Color = Rgb;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_consistent() {
        let mut seen = BTreeSet::new();
        for d in TABLE.iter() {
            assert!(seen.insert(d.name), "{} appears twice", d.name);
            if let Some(p) = d.parent {
                let pd = def(p).expect("a parent is a row");
                assert_eq!(pd.tab, d.tab, "a subcategory sits in its parent's tab");
                assert!(pd.parent.is_none(), "one level of subcategories");
            }
        }
        // every legacy harness category is a row
        for c in docviews::GRAPHICS_CATEGORIES.iter().chain(docviews::DEFAULT_HIDDEN_CATEGORIES.iter()) {
            assert!(def(c).is_some(), "{c} is listed");
        }
    }

    #[test]
    fn a_subcategory_is_hidden_with_its_parent_and_a_class_with_all_its_rows() {
        let mut g = ViewGraphics::default();
        assert!(!is_hidden(&g, "<Room Separation>"));
        g.hidden_categories.insert("Lines".to_owned());
        assert!(is_hidden(&g, "<Room Separation>") && !own_hidden(&g, "<Room Separation>"));
        g.hidden_categories.clear();
        g.hidden_classes.insert("model".to_owned());
        assert!(is_hidden(&g, "Walls") && !is_hidden(&g, "Grids"));
        // the template's defaults
        let g = ViewGraphics::default();
        assert!(is_hidden(&g, "Mass") && is_hidden(&g, "Analytical Nodes") && !is_hidden(&g, "Walls"));
    }
}
