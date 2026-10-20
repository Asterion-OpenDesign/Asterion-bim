//! Why an element is, or is not, in a view: the per-element visibility diagnosis.
//!
//! A view does not decide silently. For one element and one view [`diagnose`] walks the stages the element goes
//! through - it exists, its parameters and relations regenerate, the engine generates a body for it
//! ([`crate::element_geometry`]), the body reaches the view's projection, the view's range / crop / depth keep it, the
//! view's visibility rules keep it, the view hands it to the renderer - and names the first stage that stops it:
//!
//! | status | meaning |
//! |---|---|
//! | `VISIBLE` | the element is in what the view hands to the renderer |
//! | `OUTSIDE_VIEW_RANGE` | a plan view's range (cut / top / bottom / depth) leaves it out |
//! | `OUTSIDE_CROP` | the crop region or crop box leaves it out |
//! | `OUTSIDE_CLIPPING_DEPTH` | a section or elevation: it is in front of the plane or beyond the far clip |
//! | `SEEN_EDGE_ON` | the element is seen edge-on by the view (its projection has no width): a geometric fact, not a loss |
//! | `HIDDEN_BY_VISIBILITY_RULE` | hidden in the view, by category, filter, phase, isolate, or because the view's class never shows it |
//! | `GEOMETRY_NOT_GENERATED` | the element exists and should have a body and the engine has none (or failed to build it) |
//! | `PROJECTION_FAILED` | it has a body and the view collected it, and the projection produced no figure |
//! | `RENDER_SUBMISSION_FAILED` | it has a body, the view keeps it, and what reached the renderer does not contain it |
//! | `UNSUPPORTED_ELEMENT_TYPE` | the engine draws nothing of this kind of element in this kind of view (a named gap) |
//! | `ELEMENT_NOT_FOUND` / `VIEW_NOT_FOUND` | the id is in no collection of the model |
//!
//! The last-but-two statuses are the ones that must never happen in silence: a `RENDER_SUBMISSION_FAILED` is a defect
//! of the engine, not a rule. `VISIBLE` does not say the element is not occluded by another in a shaded view: occlusion
//! is the renderer's depth test, and the diagnosis says so in its `note`.
//!
//! Nothing is cached anywhere on this path: the body, the view state and the figures are recomputed on every call
//! (`derived`), which is what keeps a view from showing a representation older than the model.

use crate::docviews::{self, DocView};
use crate::element_geometry::{self, ElementClass, Geometry};
use crate::model::{BimModel, View, ViewKind};
use crate::views::{view_state, Appearance, SpanClassifier};
use asterion_core::ElementId;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Visible,
    OutsideViewRange,
    OutsideCrop,
    OutsideClippingDepth,
    /// The element is seen edge-on: its projection has no width, so nothing can be drawn of it (a legitimate geometric result).
    SeenEdgeOn,
    HiddenByVisibilityRule,
    GeometryNotGenerated,
    ProjectionFailed,
    RenderSubmissionFailed,
    UnsupportedElementType,
    ElementNotFound,
    ViewNotFound,
}

impl Status {
    pub fn code(self) -> &'static str {
        match self {
            Status::Visible => "VISIBLE",
            Status::OutsideViewRange => "OUTSIDE_VIEW_RANGE",
            Status::OutsideCrop => "OUTSIDE_CROP",
            Status::OutsideClippingDepth => "OUTSIDE_CLIPPING_DEPTH",
            Status::SeenEdgeOn => "SEEN_EDGE_ON",
            Status::HiddenByVisibilityRule => "HIDDEN_BY_VISIBILITY_RULE",
            Status::GeometryNotGenerated => "GEOMETRY_NOT_GENERATED",
            Status::ProjectionFailed => "PROJECTION_FAILED",
            Status::RenderSubmissionFailed => "RENDER_SUBMISSION_FAILED",
            Status::UnsupportedElementType => "UNSUPPORTED_ELEMENT_TYPE",
            Status::ElementNotFound => "ELEMENT_NOT_FOUND",
            Status::ViewNotFound => "VIEW_NOT_FOUND",
        }
    }

    /// A status that is a legitimate answer of the view's rules (the element is where the rules say it is), as
    /// opposed to a defect or a named gap of the engine.
    pub fn is_rule(self) -> bool {
        matches!(
            self,
            Status::Visible | Status::OutsideViewRange | Status::OutsideCrop | Status::OutsideClippingDepth | Status::SeenEdgeOn | Status::HiddenByVisibilityRule
        )
    }

    /// A statuses that is a defect: an element the engine should have drawn and did not.
    pub fn is_defect(self) -> bool {
        matches!(self, Status::ProjectionFailed | Status::RenderSubmissionFailed)
    }
}

/// One stage of the walk: its name, whether the element passed it, and what was found.
#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnosis {
    pub view: ElementId,
    pub element: ElementId,
    pub view_kind: &'static str,
    pub class: Option<&'static str>,
    pub status: Status,
    /// The rule or stage that explains the status.
    pub rule: String,
    pub stages: Vec<Stage>,
    /// What the engine says about the element's body.
    pub geometry: String,
    /// The fingerprint of the element's reference body: equal in every view, different when the body changes.
    pub geometry_signature: Option<u64>,
    /// Always "derived on demand": no cache lies between the model and this answer.
    pub derived: &'static str,
    pub note: String,
}

fn esc(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            '\n' => vec!['\\', 'n'],
            c if (c as u32) < 0x20 => vec![' '],
            c => vec![c],
        })
        .collect()
}

impl Diagnosis {
    pub fn to_json(&self) -> String {
        let stages: Vec<String> = self
            .stages
            .iter()
            .map(|s| format!("{{\"stage\":\"{}\",\"passed\":{},\"detail\":\"{}\"}}", s.name, s.passed, esc(&s.detail)))
            .collect();
        format!(
            "{{\"view\":{},\"element\":{},\"view_kind\":\"{}\",\"class\":{},\"status\":\"{}\",\"rule\":\"{}\",\"geometry\":\"{}\",\"geometry_signature\":{},\"derived\":\"{}\",\"note\":\"{}\",\"stages\":[{}]}}",
            self.view.0,
            self.element.0,
            self.view_kind,
            self.class.map_or("null".to_owned(), |c| format!("\"{c}\"")),
            self.status.code(),
            esc(&self.rule),
            esc(&self.geometry),
            self.geometry_signature.map_or("null".to_owned(), |s| format!("\"{s:016x}\"")),
            self.derived,
            esc(&self.note),
            stages.join(",")
        )
    }

    /// One log line (the shape `ASTERION_VIEW_TRACE` prints).
    pub fn log_line(&self) -> String {
        format!(
            "view {} ({}) element {} ({}): {} - {}",
            self.view.0,
            self.view_kind,
            self.element.0,
            self.class.unwrap_or("?"),
            self.status.code(),
            self.rule
        )
    }
}

struct Walk {
    stages: Vec<Stage>,
}

impl Walk {
    fn pass(&mut self, name: &'static str, detail: impl Into<String>) {
        self.stages.push(Stage { name, passed: true, detail: detail.into() });
    }
}

/// The elements a view's own rules keep out, as a set: its hidden set, the visibility / graphics rules (category, filter,
/// template, temporary hide / isolate) and the phases. The views' drawings add this to the view's own hidden set.
pub fn hidden_set(m: &BimModel, view: ElementId) -> BTreeSet<ElementId> {
    let mut hidden = docviews::graphics_hidden_set(m, view);
    if !m.phases.is_empty() && m.views.contains_key(&view) {
        hidden.extend(
            m.walls
                .keys()
                .chain(m.hosted_instances.keys())
                .chain(m.floors.keys())
                .chain(m.columns.keys())
                .chain(m.wall_openings.keys())
                .chain(m.stairs.stairs.keys())
                .chain(m.rooms.keys())
                .copied()
                .filter(|id| !crate::phases::shown_in_view(m, view, *id)),
        );
    }
    hidden
}

/// A view with the elements its rules hide added to its own hidden set (what every drawing of the view must use).
pub fn effective_view(m: &BimModel, v: &View) -> View {
    let hidden = hidden_set(m, v.id);
    if hidden.is_empty() {
        return v.clone();
    }
    let mut v2 = v.clone();
    v2.hidden_elements.extend(hidden);
    v2
}

/// Why `element` is hidden in `view` by a rule, if it is.
fn hidden_reason(m: &BimModel, view: ElementId, element: ElementId) -> Option<String> {
    if m.views.get(&view).map_or(false, |v| v.hidden_elements.contains(&element)) {
        return Some("hidden in this view (Hide in View: the view's own set of element ids)".into());
    }
    if docviews::hidden_by_graphics(m, view, element) {
        return Some("hidden by the view's graphics: its category, a filter, the template, a temporary hide or an isolate (OBSERVED)".into());
    }
    if m.views.contains_key(&view) && !m.phases.is_empty() && !crate::phases::shown_in_view(m, view, element) {
        return Some("not shown by the view's phase and phase filter (OBSERVED)".into());
    }
    None
}

// ------------------------------------------------------------------------------------------------ plan members

/// How the plan classifies a column: by its vertical span against the view's range and crop, exactly as it classifies a
/// wall (Spec V-02 band assignment, `views::classify`). `None`: not drawn. A column used to be drawn when its BASE level
/// was the view's level and nowhere else - a column that crosses a storey vanished from the upper plans, one that ends
/// below the cut plane was drawn as cut.
pub fn column_appearance(classifier: &SpanClassifier, m: &BimModel, c: &crate::columns::Column) -> Option<Appearance> {
    let s = crate::columns::column_state(m, c).ok()?;
    if s.height <= 0.0 {
        return None;
    }
    let a = classifier.appearance(s.base_z, s.top_z, &s.footprint);
    a.is_drawn().then_some(a)
}

/// Whether an element of `level` is drawn by a plan whose generating level is `gen` (`None`: a view without one, which
/// draws every level's).
pub fn on_level(gen: Option<ElementId>, level: ElementId) -> bool {
    gen.is_none() || gen == Some(level)
}

/// The elements a plan view draws and how, from the same rules the drawing applies: the classified walls and floors,
/// hosted symbols of drawn hosts, columns, stairs runs, rooms, room separations and visible grids.
pub fn plan_manifest(m: &BimModel, view: &View) -> Result<BTreeMap<ElementId, &'static str>, asterion_core::Failure> {
    let ev = effective_view(m, view);
    let st = view_state(m, &ev)?;
    let mut out: BTreeMap<ElementId, &'static str> = BTreeMap::new();
    for e in st.elements.iter().filter(|e| e.appearance.is_drawn()) {
        out.insert(e.id, e.appearance.name());
    }
    let gen = ev.gen_level;
    for i in m.hosted_instances.values() {
        if out.contains_key(&i.host) && !ev.hidden_elements.contains(&i.id) {
            out.insert(i.id, "hosted symbol");
        }
    }
    for o in m.wall_openings.values() {
        if out.contains_key(&o.host) && !ev.hidden_elements.contains(&o.id) {
            out.insert(o.id, "cut in its host's outline");
        }
    }
    if let Ok(cl) = SpanClassifier::new(m, &ev) {
        for c in m.columns.values().filter(|c| !ev.hidden_elements.contains(&c.id)) {
            if let Some(a) = column_appearance(&cl, m, c) {
                out.insert(c.id, a.name());
            }
        }
    }
    for r in m.stairs.runs.values() {
        if let Some(s) = m.stairs.stairs.get(&r.stairs) {
            if on_level(gen, s.base_level) && !ev.hidden_elements.contains(&s.id) {
                out.insert(r.id, "location line");
            }
        }
    }
    for r in m.rooms.values() {
        if on_level(gen, r.level) && !ev.hidden_elements.contains(&r.id) {
            out.insert(r.id, if r.tagged { "tag" } else { "seed point" });
        }
    }
    for s in m.room_separations.values() {
        if on_level(gen, s.level) && !ev.hidden_elements.contains(&s.id) {
            out.insert(s.id, "line");
        }
    }
    for g in st.grids.iter().filter(|g| g.visible) {
        if !ev.hidden_elements.contains(&g.id) {
            out.insert(g.id, "grid line");
        }
    }
    Ok(out)
}

fn kind_name(m: &BimModel, view: ElementId) -> &'static str {
    if let Some(v) = m.views.get(&view) {
        return v.kind.name();
    }
    match m.docs.views.get(&view).map(|d| d.kind) {
        Some(docviews::DocViewKind::Section) => "section",
        Some(docviews::DocViewKind::Elevation) => "elevation",
        Some(docviews::DocViewKind::CalloutPlan) => "callout_plan",
        Some(docviews::DocViewKind::CalloutDetail) => "callout_detail",
        None => "unknown",
    }
}

fn base_diagnosis(m: &BimModel, view: ElementId, element: ElementId) -> Diagnosis {
    let class = element_geometry::classify(m, element);
    let g = element_geometry::geometry_of(m, element);
    Diagnosis {
        view,
        element,
        view_kind: kind_name(m, view),
        class: class.map(ElementClass::name),
        status: Status::Visible,
        rule: String::new(),
        stages: Vec::new(),
        geometry: if class.is_some() { g.label() } else { "no such element".into() },
        geometry_signature: (class.is_some() && matches!(g, Geometry::Solid(_) | Geometry::Void(_))).then(|| element_geometry::signature(&g)),
        derived: "derived on demand: nothing between the model and this answer is cached",
        note: String::new(),
    }
}

/// The wall a door, window or opening is hosted by.
fn host_of(m: &BimModel, element: ElementId) -> Option<ElementId> {
    m.hosted_instances.get(&element).map(|i| i.host).or_else(|| m.wall_openings.get(&element).map(|o| o.host))
}

/// The classes whose kind of drawing is not model geometry: the tool does not judge them.
fn not_model_geometry(c: ElementClass) -> bool {
    matches!(c, ElementClass::Group | ElementClass::Annotation | ElementClass::View | ElementClass::Shaft)
}

/// The walk of `element` in `view`. See the module doc.
pub fn diagnose(m: &BimModel, view: ElementId, element: ElementId) -> Diagnosis {
    diagnose_with(m, view, element, &mut Cache::default())
}

/// What the diagnosis of one VIEW derives once and every element of it reuses: the plan's manifest, the 3D mesh, the section's figures.
/// [report] shares one across all the elements of a view (without it each element rebuilt the whole view).
#[derive(Default)]
pub struct Cache {
    view: Option<ElementId>,
    manifest: Option<Result<BTreeMap<ElementId, &'static str>, String>>,
    uncropped: Option<BTreeMap<ElementId, &'static str>>,
    mesh: Option<Result<crate::mesh3d::Mesh3d, String>>,
    figures: Option<Result<Vec<crate::section_graphics::Figure>, String>>,
}

impl Cache {
    fn sync(&mut self, view: ElementId) {
        if self.view != Some(view) {
            *self = Cache { view: Some(view), ..Default::default() };
        }
    }
    fn manifest(&mut self, m: &BimModel, v: &View) -> Result<&BTreeMap<ElementId, &'static str>, String> {
        self.sync(v.id);
        let r = self.manifest.get_or_insert_with(|| plan_manifest(m, v).map_err(|e| e.message));
        r.as_ref().map_err(|e| e.clone())
    }
    fn uncropped(&mut self, m: &BimModel, v: &View) -> &BTreeMap<ElementId, &'static str> {
        self.sync(v.id);
        self.uncropped.get_or_insert_with(|| {
            let mut u = effective_view(m, v);
            u.crop_active = false;
            plan_manifest(m, &u).unwrap_or_default()
        })
    }
    fn mesh(&mut self, m: &BimModel, v: &View) -> Result<&crate::mesh3d::Mesh3d, String> {
        self.sync(v.id);
        let r = self.mesh.get_or_insert_with(|| crate::mesh3d::view_mesh(m, &effective_view(m, v)).map_err(|e| e.message));
        r.as_ref().map_err(|e| e.clone())
    }
    fn figures(&mut self, m: &BimModel, view: ElementId) -> Result<&Vec<crate::section_graphics::Figure>, String> {
        self.sync(view);
        let r = self.figures.get_or_insert_with(|| crate::section_graphics::section_figures(m, view).map_err(|e| e.message));
        r.as_ref().map_err(|e| e.clone())
    }
}

/// [diagnose] with a cache shared by the elements of one view.
pub fn diagnose_with(m: &BimModel, view: ElementId, element: ElementId, cx: &mut Cache) -> Diagnosis {
    let mut d = base_diagnosis(m, view, element);
    let mut w = Walk { stages: Vec::new() };
    let Some(class) = element_geometry::classify(m, element) else {
        d.status = Status::ElementNotFound;
        d.rule = "no collection of the model holds this element id".into();
        d.stages = vec![Stage { name: "exists", passed: false, detail: d.rule.clone() }];
        return d;
    };
    if !m.views.contains_key(&view) && !m.docs.views.contains_key(&view) {
        d.status = Status::ViewNotFound;
        d.rule = "no such view".into();
        d.stages = vec![Stage { name: "view", passed: false, detail: "no such view".into() }];
        return d;
    }
    w.pass("exists", format!("a {}", class.name()));
    let geometry = element_geometry::geometry_of(m, element);
    let diagnosed = if let Some(v) = m.views.get(&view) {
        match v.kind {
            ViewKind::FloorPlan | ViewKind::CeilingPlan => plan(m, v, element, class, &geometry, w, &mut d, cx),
            ViewKind::ThreeD => three_d(m, v, element, class, &geometry, w, &mut d, cx),
            // an imported section or elevation draws through the frame its stored viewer box gives (docviews::imported_doc_view)
            ViewKind::Section | ViewKind::Elevation if docviews::imported_doc_view(m, view).is_some() => {
                let dv = docviews::imported_doc_view(m, view).expect("checked");
                docview(m, &dv, element, class, &geometry, w, &mut d, cx)
            }
            ViewKind::Section | ViewKind::Elevation => {
                d.status = Status::UnsupportedElementType;
                d.rule = format!(
                    "this {} has no drawing frame: {}",
                    v.kind.name(),
                    docviews::imported_frame_gap(m, view).unwrap_or("its viewer box is not stored")
                );
                w.stages.push(Stage { name: "view frame", passed: false, detail: d.rule.clone() });
                w.stages
            }
            _ => {
                d.status = Status::UnsupportedElementType;
                d.rule = format!("a {} view does not draw model elements (it has its own content)", v.kind.name());
                w.stages.push(Stage { name: "view kind", passed: false, detail: d.rule.clone() });
                w.stages
            }
        }
    } else {
        let dv = &m.docs.views[&view];
        docview(m, dv, element, class, &geometry, w, &mut d, cx)
    };
    d.stages = diagnosed;
    d
}

fn geometry_gate(geometry: &Geometry, w: &mut Walk, d: &mut Diagnosis) -> bool {
    match geometry {
        Geometry::NotGenerated(r) => {
            w.stages.push(Stage { name: "geometry", passed: false, detail: r.clone() });
            d.status = Status::GeometryNotGenerated;
            d.rule = r.clone();
            false
        }
        Geometry::Failed(r) => {
            w.stages.push(Stage { name: "geometry", passed: false, detail: format!("regeneration failed: {r}") });
            d.status = Status::GeometryNotGenerated;
            d.rule = format!("regeneration of the element failed: {r}");
            false
        }
        _ => {
            w.pass("geometry", geometry.label());
            true
        }
    }
}

fn plan(m: &BimModel, v: &View, element: ElementId, class: ElementClass, geometry: &Geometry, mut w: Walk, d: &mut Diagnosis, cx: &mut Cache) -> Vec<Stage> {
    // what the plan has no line work for, by kind (named gaps, not rules of the view)
    let gap = match class {
        ElementClass::Ceiling | ElementClass::Roof => Some("ceilings and roofs are not classified in plan views: their appearance was never measured (Spec V-02's mode m for their categories is unspecified)"),
        ElementClass::Toposolid => Some("the plan draws no toposolid line work"),
        ElementClass::WallSweep | ElementClass::WallReveal => Some("the plan draws no wall sweep or reveal line work"),
        ElementClass::Stairs | ElementClass::StairsLanding | ElementClass::StairsRailing => Some("the plan draws only the location line of each stairs run (OQ-STAIR-GEOM)"),
        _ => None,
    };
    if let Some(r) = hidden_reason(m, v.id, element) {
        w.stages.push(Stage { name: "visibility", passed: false, detail: r.clone() });
        d.status = Status::HiddenByVisibilityRule;
        d.rule = r;
        return w.stages;
    }
    w.pass("visibility", "no hide, graphics or phase rule keeps it out");
    if not_model_geometry(class) {
        d.status = Status::UnsupportedElementType;
        d.rule = format!("a {} is not model geometry: the diagnosis does not judge it", class.name());
        w.stages.push(Stage { name: "kind", passed: false, detail: d.rule.clone() });
        return w.stages;
    }
    if class == ElementClass::Level {
        d.status = Status::HiddenByVisibilityRule;
        d.rule = "a level is a horizontal datum: perpendicular to no plan view, so a plan never shows it (Spec V-03)".into();
        w.stages.push(Stage { name: "datum", passed: false, detail: d.rule.clone() });
        return w.stages;
    }
    if let Some(r) = gap {
        d.status = Status::UnsupportedElementType;
        d.rule = r.into();
        w.stages.push(Stage { name: "kind", passed: false, detail: r.into() });
        return w.stages;
    }
    let abstract_ok = matches!(class, ElementClass::Grid | ElementClass::Room | ElementClass::RoomSeparation | ElementClass::StairsRun);
    let manifest = match cx.manifest(m, v) {
        Ok(x) => x,
        Err(e) => {
            d.status = Status::ProjectionFailed;
            d.rule = format!("the view's state could not be derived: {}", e);
            w.stages.push(Stage { name: "view state", passed: false, detail: d.rule.clone() });
            return w.stages;
        }
    };
    if let Some(how) = manifest.get(&element) {
        w.pass("projection", format!("classified and drawn as {how}"));
        w.pass("submission", "in the line work the view hands to the renderer");
        d.status = Status::Visible;
        d.rule = format!("drawn in this plan ({how})");
        return w.stages;
    }
    // not drawn: find the rule. Re-derive with the crop off to tell the crop from the range.
    if cx.uncropped(m, v).contains_key(&element) {
        d.status = Status::OutsideCrop;
        d.rule = "the plan's crop region leaves it out".into();
        w.stages.push(Stage { name: "crop", passed: false, detail: d.rule.clone() });
        return w.stages;
    }
    // the plan drew nothing of it and no crop explains it: a body the engine did not generate is the next candidate (a floor the plan
    // DOES draw needs only its outline and elevations, so this gate comes after the manifest, not before)
    if !abstract_ok && !geometry_gate(geometry, &mut w, d) {
        return w.stages;
    }
    match class {
        ElementClass::Wall | ElementClass::CurtainWall | ElementClass::Floor | ElementClass::Column => {
            d.status = Status::OutsideViewRange;
            d.rule = "the plan's view range (cut plane, top and bottom clip, view depth) leaves it out (Spec V-02 band assignment)".into();
            w.stages.push(Stage { name: "view range", passed: false, detail: d.rule.clone() });
        }
        ElementClass::Door | ElementClass::Window | ElementClass::WallOpening => {
            let hd = host_of(m, element).map(|h| diagnose_with(m, v.id, h, cx));
            d.status = hd.as_ref().map_or(Status::OutsideViewRange, |h| h.status);
            d.rule = format!("its host wall is not drawn in this plan (a hosted insert is drawn with its host): {}", hd.map_or(String::new(), |h| h.rule));
            w.stages.push(Stage { name: "host", passed: false, detail: d.rule.clone() });
        }
        ElementClass::StairsRun | ElementClass::Room | ElementClass::RoomSeparation => {
            d.status = Status::OutsideViewRange;
            d.rule = "it belongs to another level than the plan's generating level".into();
            w.stages.push(Stage { name: "level", passed: false, detail: d.rule.clone() });
        }
        ElementClass::Grid => {
            d.status = Status::OutsideViewRange;
            d.rule = "the grid's vertical extent does not meet the plan's primary range (Spec V-03)".into();
            w.stages.push(Stage { name: "datum range", passed: false, detail: d.rule.clone() });
        }
        _ => {
            d.status = Status::RenderSubmissionFailed;
            d.rule = "the element has a body and no rule leaves it out, yet the plan's line work does not carry it".into();
            w.stages.push(Stage { name: "submission", passed: false, detail: d.rule.clone() });
        }
    }
    w.stages
}

fn three_d(m: &BimModel, v: &View, element: ElementId, class: ElementClass, geometry: &Geometry, mut w: Walk, d: &mut Diagnosis, cx: &mut Cache) -> Vec<Stage> {
    if let Some(r) = hidden_reason(m, v.id, element) {
        w.stages.push(Stage { name: "visibility", passed: false, detail: r.clone() });
        d.status = Status::HiddenByVisibilityRule;
        d.rule = r;
        return w.stages;
    }
    w.pass("visibility", "no hide, graphics or phase rule keeps it out");
    d.note = "VISIBLE means the element is in the mesh handed to the renderer; whether another solid stands in front of it is the renderer's depth test, which this diagnosis does not evaluate".into();
    if not_model_geometry(class) {
        d.status = Status::UnsupportedElementType;
        d.rule = format!("a {} is not model geometry: the diagnosis does not judge it", class.name());
        w.stages.push(Stage { name: "kind", passed: false, detail: d.rule.clone() });
        return w.stages;
    }
    match class {
        ElementClass::Level | ElementClass::Grid => {
            d.status = Status::UnsupportedElementType;
            d.rule = "the engine's 3D view draws no datums (the reference product draws a datum in 3D only when its 3D extent is on: Spec V-03 not specified)".into();
            w.stages.push(Stage { name: "kind", passed: false, detail: d.rule.clone() });
            return w.stages;
        }
        ElementClass::Room | ElementClass::RoomSeparation => {
            d.status = Status::UnsupportedElementType;
            d.rule = "a room is a space: it has no body in a 3D view".into();
            w.stages.push(Stage { name: "kind", passed: false, detail: d.rule.clone() });
            return w.stages;
        }
        ElementClass::WallReveal => {
            d.status = Status::UnsupportedElementType;
            d.rule = "a reveal is not subtracted from the wall body in the 3D mesh".into();
            w.stages.push(Stage { name: "kind", passed: false, detail: d.rule.clone() });
            return w.stages;
        }
        _ => {}
    }
    if !geometry_gate(geometry, &mut w, d) {
        return w.stages;
    }
    w.pass("projection", "a 3D view applies no view range, cut plane or far clip: the body is projected whole (the only element test of a 3D view is its section box)");
    // an insert (door, window, opening) is drawn as the outline of its cut in its host wall: it follows its host, and a host the
    // engine cannot cut (an arc wall, a curtain wall) leaves it undrawn by a named gap, not a defect
    if let Some(host) = host_of(m, element) {
        let hd = diagnose_with(m, v.id, host, cx);
        if hd.status != Status::Visible {
            d.status = hd.status;
            d.rule = format!("its host wall is not in the 3D view (an insert is drawn with its host): {}", hd.rule);
            w.stages.push(Stage { name: "host", passed: false, detail: d.rule.clone() });
            return w.stages;
        }
        let uncut = m.walls.get(&host).map_or(true, |h| h.through.is_some()) || m.is_curtain_wall(host);
        if uncut {
            d.status = Status::UnsupportedElementType;
            d.rule = "the 3D mesh cuts the openings of straight walls only: an arc wall or a curtain wall is drawn whole, without the insert".into();
            w.stages.push(Stage { name: "kind", passed: false, detail: d.rule.clone() });
            return w.stages;
        }
    }
    match cx.mesh(m, v) {
        Ok(mesh) if mesh.elements.contains(&element) => {
            w.pass("submission", format!("in the mesh ({} solids, {} triangles)", mesh.solids, mesh.triangle_count()));
            d.status = Status::Visible;
            d.rule = "in the 3D mesh handed to the renderer".into();
        }
        Ok(mesh) => {
            let gap = mesh.gaps.iter().find(|g| g.0 == element).map(|g| g.1.clone());
            d.status = Status::RenderSubmissionFailed;
            d.rule = match gap {
                Some(r) => format!("the mesh builder recorded a gap for it: {r}"),
                None => "the element has a body and no rule hides it, yet the 3D mesh does not contain it".into(),
            };
            w.stages.push(Stage { name: "submission", passed: false, detail: d.rule.clone() });
        }
        Err(e) => {
            d.status = Status::RenderSubmissionFailed;
            d.rule = format!("the 3D mesh could not be built: {e}");
            w.stages.push(Stage { name: "submission", passed: false, detail: d.rule.clone() });
        }
    }
    w.stages
}
fn docview(m: &BimModel, dv: &DocView, element: ElementId, class: ElementClass, geometry: &Geometry, mut w: Walk, d: &mut Diagnosis, cx: &mut Cache) -> Vec<Stage> {
    // a plan callout draws what its parent plan draws, inside its crop (OBSERVED)
    if matches!(dv.kind, docviews::DocViewKind::CalloutPlan | docviews::DocViewKind::CalloutDetail) {
        let Some(parent) = dv.parent.and_then(|p| m.views.get(&p)) else {
            d.status = Status::ProjectionFailed;
            d.rule = "the callout's parent plan is gone".into();
            w.stages.push(Stage { name: "parent", passed: false, detail: d.rule.clone() });
            return w.stages;
        };
        let pd = diagnose_with(m, parent.id, element, cx);
        w.pass("parent plan", format!("{}: {}", pd.status.code(), pd.rule));
        if pd.status != Status::Visible {
            d.status = pd.status;
            d.rule = format!("in the parent plan: {}", pd.rule);
            return w.stages;
        }
        if class != ElementClass::Level && !docviews::in_section(m, dv, element) {
            d.status = Status::OutsideCrop;
            d.rule = "the callout's crop box leaves it out".into();
            w.stages.push(Stage { name: "crop", passed: false, detail: d.rule.clone() });
        } else {
            d.status = Status::Visible;
            d.rule = "drawn by the parent plan and inside the callout's crop box".into();
        }
        return w.stages;
    }
    if let Some(r) = hidden_reason(m, dv.id, element) {
        w.stages.push(Stage { name: "visibility", passed: false, detail: r.clone() });
        d.status = Status::HiddenByVisibilityRule;
        d.rule = r;
        return w.stages;
    }
    w.pass("visibility", "no hide, graphics rule keeps it out");
    if not_model_geometry(class) {
        d.status = Status::UnsupportedElementType;
        d.rule = format!("a {} is not model geometry: the diagnosis does not judge it", class.name());
        w.stages.push(Stage { name: "kind", passed: false, detail: d.rule.clone() });
        return w.stages;
    }
    match class {
        ElementClass::Grid => {
            d.status = Status::HiddenByVisibilityRule;
            d.rule = "a grid is in no section, elevation or callout collector (OBSERVED: 14 combinations)".into();
            w.stages.push(Stage { name: "collector", passed: false, detail: d.rule.clone() });
            return w.stages;
        }
        ElementClass::Level => {
            if docviews::in_section(m, dv, element) {
                w.pass("collector", "the level crosses the height of the crop box");
                d.status = Status::Visible;
                d.rule = "drawn as a level line across the view".into();
            } else {
                d.status = Status::OutsideCrop;
                d.rule = "the level does not cross the height of the crop box (OBSERVED)".into();
                w.stages.push(Stage { name: "collector", passed: false, detail: d.rule.clone() });
            }
            return w.stages;
        }
        ElementClass::Room | ElementClass::RoomSeparation => {
            d.status = Status::UnsupportedElementType;
            d.rule = "a room is collected by a section but the engine draws nothing of it there (OQ-ROOM-SECTION)".into();
            w.stages.push(Stage { name: "kind", passed: false, detail: d.rule.clone() });
            return w.stages;
        }
        ElementClass::WallReveal => {
            d.status = Status::UnsupportedElementType;
            d.rule = "a reveal is not drawn in sections or elevations".into();
            w.stages.push(Stage { name: "kind", passed: false, detail: d.rule.clone() });
            return w.stages;
        }
        _ => {}
    }
    if !geometry_gate(geometry, &mut w, d) {
        return w.stages;
    }
    // the collector: the element's box meets the crop box
    let in_collector = docviews::in_section(m, dv, element);
    let hosted = matches!(class, ElementClass::Door | ElementClass::Window | ElementClass::WallOpening);
    if !in_collector && !hosted {
        // the box against the crop box: across the view (crop) or along the depth (clip)
        let (lo, hi) = docviews::element_box(m, element).unwrap_or(([0.0; 3], [0.0; 3]));
        let pts: Vec<[f64; 3]> = [lo[0], hi[0]]
            .iter()
            .flat_map(|x| [lo[1], hi[1]].into_iter().flat_map(move |y| [lo[2], hi[2]].into_iter().map(move |z| [*x, y, z])))
            .map(|p| docviews::to_local(dv, p))
            .collect();
        let range = |a: usize| pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), p| (l.min(p[a]), h.max(p[a])));
        let across = (0..2).all(|a| range(a).1 >= dv.crop_min[a] - 1e-6 && range(a).0 <= dv.crop_max[a] + 1e-6);
        let depth_out = !(range(2).1 >= dv.crop_min[2] - 1e-6 && range(2).0 <= dv.crop_max[2] + 1e-6);
        if across && depth_out {
            d.status = Status::OutsideClippingDepth;
            d.rule = "it lies in front of the section plane or beyond the far clip (the depth of the crop box)".into();
        } else {
            d.status = Status::OutsideCrop;
            d.rule = "its box does not meet the view's crop box".into();
        }
        w.stages.push(Stage { name: "collector", passed: false, detail: d.rule.clone() });
        return w.stages;
    }
    w.pass("collector", if hosted { "a hosted insert is drawn with its host wall".to_string() } else { "its box meets the crop box".to_string() });
    let figures = match cx.figures(m, dv.id) {
        Ok(f) => f,
        Err(e) => {
            d.status = Status::ProjectionFailed;
            d.rule = format!("the view's figures could not be built: {e}");
            w.stages.push(Stage { name: "projection", passed: false, detail: d.rule.clone() });
            return w.stages;
        }
    };
    let drawn = figures.iter().any(|f| f.element == element || f.holes.iter().any(|h| h.0 == element));
    if drawn {
        let cut = figures.iter().any(|f| f.cut && (f.element == element || f.holes.iter().any(|h| h.0 == element)));
        w.pass("projection", if cut { "cut by the section plane" } else { "projected behind the plane" });
        w.pass("submission", "in the line work the view hands to the renderer");
        d.status = Status::Visible;
        d.rule = if cut { "cut by the view's plane".into() } else { "projected in the view".into() };
        return w.stages;
    }
    if hosted {
        let hd = host_of(m, element).map(|h| diagnose_with(m, dv.id, h, cx));
        match hd {
            Some(h) if h.status != Status::Visible => {
                d.status = h.status;
                d.rule = format!("its host wall is not drawn in this view (a hosted insert is drawn with its host): {}", h.rule);
            }
            _ => {
                let along = m.walls.get(&host_of(m, element).unwrap_or(ElementId::INVALID)).and_then(|w| (w.end - w.start).normalized());
                let edge_on = along.map_or(false, |a| (a.x * dv.x[0] + a.y * dv.x[1]).abs() < 1e-9);
                if edge_on {
                    d.status = Status::SeenEdgeOn;
                    d.rule = "its host wall runs along the view direction: the insert is seen edge-on and has no width to draw".into();
                } else {
                    d.status = Status::ProjectionFailed;
                    d.rule = "its host wall is drawn and the insert lies outside the host's figure".into();
                }
            }
        }
        w.stages.push(Stage { name: "host", passed: false, detail: d.rule.clone() });
        return w.stages;
    }
    // in the collector, with a body, and no figure: in front of the plane, or the projection lost it
    let corners: Vec<f64> = geometry.solids().iter().flat_map(|b| b.corners()).map(|c| docviews::to_local(dv, c)[2]).collect();
    let (lo, hi) = corners.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), z| (l.min(*z), h.max(*z)));
    if hi > 1e-9 && lo >= -1e-9 {
        d.status = Status::OutsideClippingDepth;
        d.rule = "its body lies wholly in front of the section plane (the viewer's side): a section draws what is behind its plane".into();
    } else {
        let xs: Vec<f64> = geometry.solids().iter().flat_map(|b| b.corners()).map(|c| docviews::to_local(dv, c)[0]).collect();
        let (xl, xh) = xs.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), x| (l.min(*x), h.max(*x)));
        if xh - xl < 1e-9 {
            d.status = Status::SeenEdgeOn;
            d.rule = "its body is seen edge-on by the view: its projection has no width".into();
        } else {
            d.status = Status::ProjectionFailed;
            d.rule = "the element has a body behind or across the plane and the view's projection produced no figure for it".into();
        }
    }
    w.stages.push(Stage { name: "projection", passed: false, detail: d.rule.clone() });
    w.stages
}

/// The diagnosis of every element of the model in one view, and a count by status: the report a test or a person uses to see
/// at a glance what a view omits and why. Elements that are not model geometry are left out.
pub fn report(m: &BimModel, view: ElementId) -> Vec<Diagnosis> {
    let mut ids: BTreeSet<ElementId> = BTreeSet::new();
    ids.extend(m.walls.keys());
    ids.extend(m.floors.keys());
    ids.extend(m.columns.keys());
    ids.extend(m.hosted_instances.keys());
    ids.extend(m.wall_openings.keys());
    ids.extend(m.annot.sweeps.keys());
    ids.extend(m.docs.toposolids.keys());
    ids.extend(m.stairs.stairs.keys());
    ids.extend(m.stairs.runs.keys());
    ids.extend(m.stairs.landings.keys());
    ids.extend(m.stairs.railings.keys());
    ids.extend(m.rooms.keys());
    ids.extend(m.levels.keys());
    ids.extend(m.grids.keys());
    let mut cx = Cache::default();
    ids.into_iter().map(|id| diagnose_with(m, view, id, &mut cx)).collect()
}

/// Prints a diagnosis when `ASTERION_VIEW_TRACE` is set (the project's tracing convention: an environment variable, stderr).
pub fn trace(d: &Diagnosis) {
    if std::env::var_os("ASTERION_VIEW_TRACE").is_some() {
        eprintln!("view-trace: {}", d.log_line());
    }
}
