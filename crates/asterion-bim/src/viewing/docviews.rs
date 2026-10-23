//! Section, elevation and callout views, view graphics, view templates and filters (OBSERVED, public API).
//!
//! * `ViewSection.CreateSection(box)`: the reference product keeps the frame in a canonical form. A box that extends to the
//!   positive side of its Z axis is turned half a turn about the up axis (X and Z reversed, the Z range negated), then the
//!   origin moves to the corner of the box (lowest X and Y, Z = 0): the reported crop box runs from `(0, 0, -depth)` to
//!   `(width, height, 0)`. The reported `ViewDirection` is that Z axis (it points from the model to the viewer).
//! * The far clip offset is the depth of the crop box (`min z = -offset`); the crop box setter does not move `min z`.
//! * The category visibility of a view is kept as the set of hidden categories, on top of the template's defaults
//!   (Analytical Nodes, Analytical Spaces, Analytical Surfaces, Mass, Parts); a hidden category leaves the view's
//!   collector, the element is not flagged hidden (`Element.IsHidden` is the explicit hide).
//! * A view with a template reads its graphics from the template; a category cannot be hidden in it.
//!
//! ENGINE CHOICES are named at each rule.

use crate::docs::Rgb;
use crate::model::BimModel;
use crate::spec::codes;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type V3 = [f64; 3];

/// The categories hidden in a new view by the template (OBSERVED).
pub const DEFAULT_HIDDEN_CATEGORIES: [&str; 5] = ["Analytical Nodes", "Analytical Spaces", "Analytical Surfaces", "Mass", "Parts"];
/// The categories whose graphics the harness reads (the keys of the observation).
pub const GRAPHICS_CATEGORIES: [&str; 12] =
    ["Walls", "Doors", "Windows", "Floors", "Rooms", "Grids", "Levels", "Roofs", "Columns", "Stairs", "Sections", "Dimensions"];

fn bad(msg: &str) -> Failure {
    Failure::error(codes::DOCVIEW_INVALID, msg.to_owned())
}

fn not_found(id: ElementId, what: &str) -> Failure {
    Failure::error(codes::NOT_FOUND, format!("No such {what}.")).on(&[id])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocViewKind {
    Section,
    Elevation,
    /// A callout of a plan that is itself a plan.
    CalloutPlan,
    /// A detail callout.
    CalloutDetail,
}

impl DocViewKind {
    pub fn class(self) -> &'static str {
        match self {
            DocViewKind::CalloutPlan => "ViewPlan",
            _ => "ViewSection",
        }
    }
    pub fn view_type(self) -> &'static str {
        match self {
            DocViewKind::Section => "Section",
            DocViewKind::Elevation => "Elevation",
            DocViewKind::CalloutPlan => "FloorPlan",
            DocViewKind::CalloutDetail => "Detail",
        }
    }
}

/// A section, elevation or callout view and the frame of its crop box.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocView {
    pub id: ElementId,
    pub kind: DocViewKind,
    pub name: String,
    pub scale: u32,
    pub detail_level: String,
    pub display_style: String,
    pub crop_active: bool,
    pub crop_visible: bool,
    /// The frame: origin, X (right), Y (up), Z (toward the viewer: the view direction the API reports).
    pub origin: V3,
    pub x: V3,
    pub y: V3,
    pub z: V3,
    pub crop_min: V3,
    pub crop_max: V3,
    /// `Far Clipping`: 0 no clip, 1 clip without lines, 2 clip with lines (OBSERVED default 2). The volume a section shows
    /// has its far (depth) bound always on, the near bound following the viewer bound and the sides following the crop
    /// flag; an ELEVATION without an active crop box is unbounded. The engine's section volume is the crop box (`crop_min`
    /// / `crop_max`) with the far side clipped at the depth; the elevation case is not modelled.
    pub far_clip: i32,
    pub marker: Option<ElementId>,
    pub parent: Option<ElementId>,
    /// The level of a plan callout.
    pub level: Option<ElementId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
}

/// An elevation marker: up to four elevations around a point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ElevationMarker {
    pub id: ElementId,
    pub point: V3,
    pub scale: u32,
    pub plan: ElementId,
    /// The elevations by index 0..=3.
    pub views: BTreeMap<u8, ElementId>,
}

pub const ELEVATION_MARKER_MAX_VIEWS: usize = 4;

/// (OBSERVED defaults: weights -1, no colour, no pattern, transparency 0, no halftone).
///
/// The harness reads the twelve fields above the line `/` only; the fields below it have no observation (the engine
/// defaults the four visibility flags to visible, ASSUMED, and the detail level to 0 = not overridden).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ogs {
    pub projection_line_weight: i32,
    pub projection_line_color: Option<Rgb>,
    pub projection_line_pattern: Option<String>,
    pub cut_line_weight: i32,
    pub cut_line_color: Option<Rgb>,
    pub cut_line_pattern: Option<String>,
    pub surface_transparency: i32,
    pub halftone: bool,
    pub surface_foreground_pattern: Option<String>,
    pub surface_foreground_color: Option<Rgb>,
    pub cut_foreground_pattern: Option<String>,
    pub cut_foreground_color: Option<Rgb>,
    /// 0 not overridden, 1 coarse, 2 medium, 3 fine.
    #[serde(default)]
    pub detail_level: i32,
    #[serde(default = "visible")]
    pub surface_foreground_pattern_visible: bool,
    #[serde(default = "visible")]
    pub surface_background_pattern_visible: bool,
    #[serde(default = "visible")]
    pub cut_foreground_pattern_visible: bool,
    #[serde(default = "visible")]
    pub cut_background_pattern_visible: bool,
    #[serde(default)]
    pub surface_background_pattern: Option<String>,
    #[serde(default)]
    pub surface_background_color: Option<Rgb>,
    #[serde(default)]
    pub cut_background_pattern: Option<String>,
    #[serde(default)]
    pub cut_background_color: Option<Rgb>,
}

fn visible() -> bool {
    true
}

impl Default for Ogs {
    fn default() -> Self {
        Ogs {
            projection_line_weight: -1,
            projection_line_color: None,
            projection_line_pattern: None,
            cut_line_weight: -1,
            cut_line_color: None,
            cut_line_pattern: None,
            surface_transparency: 0,
            halftone: false,
            surface_foreground_pattern: None,
            surface_foreground_color: None,
            cut_foreground_pattern: None,
            cut_foreground_color: None,
            detail_level: 0,
            surface_foreground_pattern_visible: true,
            surface_background_pattern_visible: true,
            cut_foreground_pattern_visible: true,
            cut_background_pattern_visible: true,
            surface_background_pattern: None,
            surface_background_color: None,
            cut_background_pattern: None,
            cut_background_color: None,
        }
    }
}

/// A filter applied to a view with its visibility and overrides.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppliedFilter {
    pub filter: ElementId,
    /// An entry with visibility on never hides.
    pub visible: bool,
    pub ogs: Ogs,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
}

fn enabled_default() -> bool {
    true
}

/// The graphics state of a view.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewGraphics {
    pub hidden_categories: BTreeSet<String>,
    /// The categories the template hides by default (`DEFAULT_HIDDEN_CATEGORIES`) that the person has shown. ENGINE CHOICE:
    /// the reference stores one set of hidden categories that a new view starts with the template's categories in; the
    /// engine keeps that start as a rule and stores the departures from it.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub shown_categories: BTreeSet<String>,
    /// The category classes (tabs of the dialog: `model`, `annotation`, `analytical`, `imported`) the view hides as a
    /// whole. ENGINE CHOICE (see `view_categories`).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub hidden_classes: BTreeSet<String>,
    pub category_overrides: BTreeMap<String, Ogs>,
    pub element_overrides: BTreeMap<ElementId, Ogs>,
    pub filters: Vec<AppliedFilter>,
    pub template: Option<ElementId>,
    /// Elements hidden temporarily, and the isolated set when isolating.
    pub temp_hidden: BTreeSet<ElementId>,
    pub isolated: Option<BTreeSet<ElementId>>,
    /// Elements hidden permanently ("Hide in View") in a section, elevation or callout: the view's own set, as a plan's
    /// `View::hidden_elements` (the view's own object, never the template's). ENGINE CHOICE until round 30d measures a hide
    /// in a section; documents written before the field read it empty.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub hidden: BTreeSet<ElementId>,
}

/// A view template: its own graphics, scale and detail level.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewTemplate {
    pub id: ElementId,
    pub name: String,
    pub scale: u32,
    pub detail_level: String,
    pub graphics: ViewGraphics,
}

/// The rule of a parameter filter (one rule).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FilterRule {
    pub param: String,
    /// `equals`, `greater`, `less`, `contains`, `begins`.
    pub kind: String,
    pub text: Option<String>,
    pub number: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewFilter {
    pub id: ElementId,
    pub name: String,
    pub categories: Vec<String>,
    pub rule: FilterRule,
}

// ------------------------------------------------------------------------------------------------ frames

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn neg(a: V3) -> V3 {
    scale(a, -1.0)
}

/// World to local: the coordinates of a world point in a view's frame.
pub fn to_local(v: &DocView, p: V3) -> V3 {
    let d = sub(p, v.origin);
    [dot(d, v.x), dot(d, v.y), dot(d, v.z)]
}

/// The section or elevation a view id names, for drawing, picking and diagnosis: the stored [`DocView`] of a view made in
/// ASTERION, or the one derived (on every call, never stored) from the viewer box an IMPORTED section or elevation kept in
/// the document settings ([`imported_doc_view`]). `None` for anything else.
pub fn doc_view<'a>(m: &'a BimModel, id: ElementId) -> Option<std::borrow::Cow<'a, DocView>> {
    if let Some(v) = m.docs.views.get(&id) {
        return Some(std::borrow::Cow::Borrowed(v));
    }
    imported_doc_view(m, id).map(std::borrow::Cow::Owned)
}

/// Why an imported section or elevation has no drawing frame, or `None` when it has one (or is not such a view). The window shows
/// it in place of the drawing: an imported view is never silently blank.
pub fn imported_frame_gap(m: &BimModel, id: ElementId) -> Option<&'static str> {
    let v = m.views.get(&id)?;
    if !matches!(v.kind, crate::model::ViewKind::Section | crate::model::ViewKind::Elevation) {
        return None;
    }
    if imported_doc_view(m, id).is_some() {
        return None;
    }
    Some(match (m.settings.view_boxes.get(&id), m.settings.view_sources.contains_key(&id)) {
        (Some(_), _) => "the stored viewer frame is not an orthonormal basis",
        (None, true) => "the imported document holds no readable viewer box for this view (frame fields unreadable)",
        (None, false) => "it was made without a cut line, direction or depth: make sections and elevations with the View > Section and Elevation tools",
    })
}

/// The frame a stored viewer box gives an imported section or elevation, as a [`DocView`].
///
/// * Frame: the viewer's basis 0 is the view's right vector, basis 1 its up vector and basis 2 the view direction (the
///   engine's `z` points toward the viewer, so it is basis 2 reversed); the origin is the box origin.
/// * Crop: the crop rectangle is the axis-0 and axis-1 bound offsets (side 0 the maximum, side 1 the minimum, in the
///   viewer's own axes) that are active; the depth is the axis-2 side-0 offset (the far clip), the near side is the
///   viewer's origin plane unless the near bound is active. A bound applies only when the box is on.
/// * A side with no active bound (an elevation without a crop box is unbounded) takes the extent of the model's walls,
///   floors and columns in the view's frame plus a metre: ENGINE CHOICE, the same as an elevation made in ASTERION.
/// * Name, scale, detail level, visual style and the crop flags are the plain view's own.
pub fn imported_doc_view(m: &BimModel, id: ElementId) -> Option<DocView> {
    use crate::model::ViewKind;
    let view = m.views.get(&id)?;
    let kind = match view.kind {
        ViewKind::Section => DocViewKind::Section,
        ViewKind::Elevation => DocViewKind::Elevation,
        _ => return None,
    };
    let b = m.settings.view_boxes.get(&id)?;
    let len = |a: V3| dot(a, a).sqrt();
    let unit = b.basis.iter().all(|a| (len(*a) - 1.0).abs() < 1e-6);
    let square = dot(b.basis[0], b.basis[1]).abs() < 1e-6 && dot(b.basis[0], b.basis[2]).abs() < 1e-6 && dot(b.basis[1], b.basis[2]).abs() < 1e-6;
    if !unit || !square {
        return None;
    }
    let (x, y, z) = (b.basis[0], b.basis[1], neg(b.basis[2]));
    let mut dv = DocView {
        id,
        kind,
        name: view.name.clone(),
        scale: view.scale,
        detail_level: match view.detail_level {
            crate::model::DetailLevel::Coarse => "Coarse",
            crate::model::DetailLevel::Medium => "Medium",
            crate::model::DetailLevel::Fine => "Fine",
        }
        .into(),
        display_style: "HLR".into(),
        crop_active: view.crop_active,
        crop_visible: view.crop_visible,
        origin: b.origin,
        x,
        y,
        z,
        crop_min: [f64::NEG_INFINITY; 3],
        crop_max: [f64::INFINITY; 3],
        far_clip: 2,
        marker: None,
        parent: None,
        level: None,
        params: BTreeMap::new(),
    };
    // the bounds the box holds (an axis in the viewer's own axes; the depth axis points away from the viewer, the engine's z toward it)
    let bound = |axis: usize, side: usize| -> Option<f64> { (b.on && b.active[axis][side]).then_some(b.offset[axis][side]) };
    if let Some(v) = bound(0, 1) {
        dv.crop_min[0] = v;
    }
    if let Some(v) = bound(0, 0) {
        dv.crop_max[0] = v;
    }
    if let Some(v) = bound(1, 1) {
        dv.crop_min[1] = v;
    }
    if let Some(v) = bound(1, 0) {
        dv.crop_max[1] = v;
    }
    if let Some(v) = bound(2, 0) {
        dv.crop_min[2] = -v;
    }
    dv.crop_max[2] = bound(2, 1).map_or(0.0, |v| -v);
    // a side with no bound: the model's extent in this frame
    if dv.crop_min.iter().chain(dv.crop_max.iter()).any(|v| !v.is_finite()) {
        let (lo, hi) = model_extent_local(m, &dv).unwrap_or(([-1.0; 3], [1.0; 3]));
        for a in 0..3 {
            if !dv.crop_min[a].is_finite() {
                dv.crop_min[a] = lo[a] - 1.0;
            }
            if !dv.crop_max[a].is_finite() {
                dv.crop_max[a] = hi[a] + 1.0;
            }
        }
    }
    Some(dv)
}

/// The extent of the model's walls, floors and columns in a view's frame (`None` when the model has none), metres. Cheap on
/// purpose (it runs on every draw and pick of a view with an unbounded side): the corners of the stored footprints between the
/// elevations of their levels, no regeneration. A wall's thickness is not added; the caller's margin covers it.
pub fn model_extent_local(m: &BimModel, v: &DocView) -> Option<(V3, V3)> {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    let mut take = |x: f64, y: f64, z: f64| {
        let p = to_local(v, [x, y, z]);
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    };
    let elev = |l: ElementId| m.levels.get(&l).map_or(0.0, |l| l.elevation);
    for w in m.walls.values() {
        let z0 = elev(w.base_level) + w.base_offset;
        let z1 = match w.top_level {
            Some(t) => elev(t) + w.top_offset,
            None => z0 + w.unconnected_height,
        };
        let mut pts = vec![w.start, w.end];
        pts.extend(w.through);
        for p in pts {
            take(p.x, p.y, z0);
            take(p.x, p.y, z1);
        }
    }
    for f in m.floors.values() {
        let z = elev(f.level);
        for p in &f.boundary {
            take(p.x, p.y, z);
        }
    }
    for c in m.columns.values() {
        let z0 = elev(c.base_level) + c.base_offset;
        let z1 = c.top_level.map_or(z0 + crate::columns::DEFAULT_UNCONNECTED_HEIGHT_M, |t| elev(t) + c.top_offset);
        take(c.location.x, c.location.y, z0);
        take(c.location.x, c.location.y, z1);
    }
    lo[0].is_finite().then_some((lo, hi))
}

/// One side of the crop rectangle of a section, elevation or callout marker: its ends in world coordinates, its midpoint
/// and whether the bound it lies on is active.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CropLine {
    pub a: V3,
    pub b: V3,
    pub mid: V3,
    pub active: bool,
}

/// The crop region of a section, elevation or callout marker is built from the plane through the viewer origin with the
/// viewer's normal, cut by the four bound planes of the viewer's bounded space - axis 0 side 0 (the maximum), axis 0 side 1
/// (the minimum), axis 1 side 0, axis 1 side 1 - giving four lines, each bounded between its intersections with its two
/// neighbours: the sides of the rectangle `[crop_min.x, crop_max.x] x [crop_min.y, crop_max.y]` of the view's own frame, in
/// that order (x max, x min, y max, y min). The activity flag of a bound is the viewer's; the engine keeps one crop flag
/// for the box, so the four share it. A box with no extent has no lines.
pub fn crop_box_lines(v: &DocView) -> Option<[CropLine; 4]> {
    let ([x0, y0], [x1, y1]) = ([v.crop_min[0], v.crop_min[1]], [v.crop_max[0], v.crop_max[1]]);
    if !(x1 > x0 + 1e-9 && y1 > y0 + 1e-9) {
        return None;
    }
    let at = |x: f64, y: f64| -> V3 { [v.origin[0] + x * v.x[0] + y * v.y[0], v.origin[1] + x * v.x[1] + y * v.y[1], v.origin[2] + x * v.x[2] + y * v.y[2]] };
    let line = |a: V3, b: V3| CropLine { a, b, mid: [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1]), 0.5 * (a[2] + b[2])], active: v.crop_active };
    Some([line(at(x1, y0), at(x1, y1)), line(at(x0, y0), at(x0, y1)), line(at(x0, y1), at(x1, y1)), line(at(x0, y0), at(x1, y0))])
}

/// The canonical form of a box and frame as the reference product reports it (see the module doc).
pub fn canonical(origin: V3, x: V3, y: V3, z: V3, min: V3, max: V3) -> (V3, V3, V3, V3, V3, V3) {
    let (mut x, y, mut z, mut min, mut max) = (x, y, z, min, max);
    if max[2] > 1e-9 {
        // half a turn about the up axis: X and Z reversed, the ranges mirrored
        x = neg(x);
        z = neg(z);
        let (lo_x, hi_x) = (-max[0], -min[0]);
        let (lo_z, hi_z) = (-max[2], -min[2]);
        min = [lo_x, min[1], lo_z];
        max = [hi_x, max[1], hi_z];
    }
    // the origin moves to the corner (lowest X and Y, Z = 0)
    let o = add(add(add(origin, scale(x, min[0])), scale(y, min[1])), scale(z, max[2]));
    let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    (o, x, y, z, [0.0, 0.0, -size[2]], [size[0], size[1], 0.0])
}

fn views_with_name<'a>(m: &'a BimModel, name: &str, except: ElementId) -> bool {
    m.views.values().any(|v| v.id != except && v.name == name)
        || m.docs.views.values().any(|v| v.id != except && v.name == name)
        || m.docs.templates.values().any(|v| v.id != except && v.name == name)
        || m.docs.schedules.values().any(|v| v.id != except && v.name == name)
}

fn next_name(m: &BimModel, prefix: &str, start: u32) -> String {
    (start..).map(|n| format!("{prefix} {n}")).find(|c| !views_with_name(m, c, ElementId(0))).expect("unbounded")
}

/// The box basis must be orthonormal (1e-9) and every axis of the box must have positive extent ("All offsets must be
/// active and must have positive volume"); the view keeps a far clip at the box's far side and no near clip (the far clip
/// offset is the depth; the near side of the depth axis is inactive); the new section is named. The viewer basis is basis 0
/// = unit(Z x Y) of the box (the section line runs along it), basis 1 = the box Y, basis 2 = (basis 1) x (basis 0) = the
/// box Z; the crop offsets are the 8 box corners in the NEW viewer's coordinates (side 0 = the maximum, side 1 = the
/// minimum of each axis) and the near side of the depth axis is made inactive again. The canonical frame below is OBSERVED
/// (round
/// 30) and agrees with that basis for a box whose far side is positive (x' = Z x Y = -x for x = Y x Z; the API getters
///     apply the managed hop leaves open). no measurement covers it.
pub fn add_section_view(m: &mut BimModel, cx: &mut TxnContext<'_>, name: Option<&str>, origin: V3, x: V3, y: V3, z: V3, min: V3, max: V3) -> Result<ElementId, Failure> {
    // QA: numbers the file format cannot hold are refused at the call (`input.invalid_number`)
    crate::ops::require_numbers("The section", &[origin, x, y, z, min, max].iter().flat_map(|p| p.iter().copied()).collect::<Vec<f64>>())?;
    // OBSERVED: a box with no width, or a vertical view direction, does not make a valid frame: the basis vectors have unit
    // length within 1e-9 and are pairwise perpendicular within 1e-9
    let orthonormal = |a: V3, b: V3| (dot(a, a).sqrt() - 1.0).abs() < 1e-9 && dot(a, b).abs() < 1e-9;
    if max[0] - min[0] < 1e-9 || max[1] - min[1] < 1e-9 || (max[2] - min[2]).abs() < 1e-9 || !orthonormal(x, y) || !orthonormal(z, x) || !orthonormal(z, y) {
        return Err(bad("The BoundingBoxXYZ is not appropriate for detail views. The basis vectors of must be unit length and orthonormal. The near and far bound offsets cannot be reversed or too close to each other."));
    }
    let name = match name {
        Some(n) => {
            if views_with_name(m, n, ElementId(0)) {
                return Err(bad("Name must be unique."));
            }
            n.to_owned()
        }
        // The next free number of the series - the LOWEST free one
        None => next_name(m, "Section", 1),
    };
    let (o, x, y, z, lo, hi) = canonical(origin, x, y, z, min, max);
    let id = cx.new_id();
    m.docs.views.insert(
        id,
        DocView {
            id,
            kind: DocViewKind::Section,
            name,
            scale: 100,
            detail_level: "Coarse".into(),
            display_style: "HLR".into(),
            crop_active: true,
            crop_visible: true,
            origin: o,
            x,
            y,
            z,
            crop_min: lo,
            crop_max: hi,
            far_clip: 2,
            marker: None,
            parent: None,
            level: None,
            params: BTreeMap::new(),
        },
    );
    Ok(id)
}

/// The letter of an elevation index (a, b, c, d).
fn letter(i: u8) -> char {
    (b'a' + i) as char
}

/// `ElevationMarker.CreateElevationMarker(type, point, scale)`.
pub fn create_marker(m: &mut BimModel, cx: &mut TxnContext<'_>, plan: ElementId, point: V3, scale: u32) -> Result<ElementId, Failure> {
    m.view(plan)?;
    crate::ops::require_numbers("The elevation marker", &point)?;
    let id = cx.new_id();
    m.docs.markers.insert(id, ElevationMarker { id, point, scale, plan, views: BTreeMap::new() });
    Ok(id)
}

/// `ElevationMarker.CreateElevation(plan, index)`: an index in use is refused (OBSERVED: the second call at index 3 of a
/// marker with index 2 made is fine; the harness' "used marker" case at index 3 was made as well, so only a repeated index
/// is refused; ENGINE CHOICE).
pub fn add_elevation_view(m: &mut BimModel, cx: &mut TxnContext<'_>, marker: ElementId, index: u8) -> Result<ElementId, Failure> {
    let mk = m.docs.markers.get(&marker).ok_or_else(|| not_found(marker, "elevation marker"))?.clone();
    if index as usize >= ELEVATION_MARKER_MAX_VIEWS || mk.views.contains_key(&index) {
        return Err(bad("The index is not valid for this marker."));
    }
    // OBSERVED: "Elevation n - a|b|c|d", n the smallest number that leaves the name unused. when an elevation view already
    // has that name the marker's name takes the counter 1, 2, ... until the name is free
    let name = (1u32..)
        .map(|n| format!("Elevation {n} - {}", letter(index)))
        .find(|c| !views_with_name(m, c, ElementId(0)))
        .expect("unbounded");
    let id = cx.new_id();
    let dirs: [V3; 4] = [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let z = dirs[index as usize];
    let x = [-z[1], z[0], 0.0];
    m.docs.views.insert(
        id,
        DocView {
            id,
            kind: DocViewKind::Elevation,
            name,
            scale: mk.scale,
            detail_level: "Coarse".into(),
            display_style: "HLR".into(),
            crop_active: true,
            crop_visible: true,
            origin: mk.point,
            x,
            y: [0.0, 0.0, 1.0],
            z,
            crop_min: [0.0, 0.0, -1.0],
            crop_max: [1.0, 1.0, 0.0],
            far_clip: 2,
            marker: Some(marker),
            parent: None,
            level: None,
            params: BTreeMap::new(),
        },
    );
    m.docs.markers.get_mut(&marker).expect("checked").views.insert(index, id);
    Ok(id)
}

/// `ViewSection.CreateCallout(parent, type, p0, p1)`: a plan callout (`detail = false`) or a detail one. The two points
/// must differ in the plane of the parent (OBSERVED). The callout rectangle is aligned with the parent view's axes whatever
/// the order or position of the two points - centred on the midpoint of their projections, with half-sizes |(Q - P) . u| /
/// 2 and |(Q - P) . r| / 2 - so the crop runs from the lower to the upper corner; the callout takes the PARENT VIEW'S
/// SCALE, records the parent, and its crop box is ACTIVE. UNKNOWN: (detail level and the default name inherited from the
/// parent) and the order.
pub fn add_callout_view(m: &mut BimModel, cx: &mut TxnContext<'_>, parent: ElementId, p0: [f64; 2], p1: [f64; 2], detail: bool, name: Option<&str>) -> Result<ElementId, Failure> {
    let pv = m.view(parent)?.clone();
    crate::ops::require_numbers("The callout", &[p0[0], p0[1], p1[0], p1[1]])?;
    if (p0[0] - p1[0]).abs() < 1e-9 && (p0[1] - p1[1]).abs() < 1e-9 {
        return Err(bad("point1 and point2 do not differ when projected onto a plane perpendicular to the view direction."));
    }
    let (lo, hi) = ([p0[0].min(p1[0]), p0[1].min(p1[1])], [p0[0].max(p1[0]), p0[1].max(p1[1])]);
    let name = match name {
        Some(n) => {
            if views_with_name(m, n, ElementId(0)) {
                return Err(bad("Name must be unique."));
            }
            n.to_owned()
        }
        None if detail => next_name(m, "Detail", 0),
        None => format!("{} - Callout 1", pv.name),
    };
    // ENGINE CHOICE: the callout's plane is the parent's cut plane (1.2 m above its level: the default view range)
    let cut_z = pv.gen_level.and_then(|l| m.levels.get(&l)).map_or(0.0, |l| l.elevation) + 1.2;
    let id = cx.new_id();
    m.docs.views.insert(
        id,
        DocView {
            id,
            kind: if detail { DocViewKind::CalloutDetail } else { DocViewKind::CalloutPlan },
            name,
            scale: pv.scale,
            detail_level: "Coarse".into(),
            display_style: "HLR".into(),
            crop_active: true,
            crop_visible: true,
            origin: [0.0, 0.0, cut_z],
            x: [1.0, 0.0, 0.0],
            y: [0.0, 1.0, 0.0],
            z: [0.0, 0.0, 1.0],
            crop_min: [lo[0], lo[1], if detail { -1.2 } else { -304.8 }],
            crop_max: [hi[0], hi[1], 0.0],
            far_clip: 2,
            marker: None,
            parent: Some(parent),
            level: if detail { None } else { pv.gen_level },
            params: BTreeMap::new(),
        },
    );
    Ok(id)
}

/// Frames a section-like view to the model, `margin` metres around it: the crop box takes the extent of the walls, floors
/// and columns in the view's own frame, and its depth reaches the far side of the model. the shell's Elevation tool calls
/// this in the same transaction so that the elevation it makes shows the model. It is a separate operation on purpose:
/// `add_elevation_view` keeps the values the harness compares. Nothing changes when the model has no such element.
pub fn frame_to_model(m: &mut BimModel, id: ElementId, margin: f64) -> Result<(), Failure> {
    let v = m.docs.views.get(&id).ok_or_else(|| not_found(id, "view"))?.clone();
    let ids: Vec<ElementId> = m.walls.keys().chain(m.floors.keys()).chain(m.columns.keys()).copied().collect();
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for e in ids {
        let Some((a, b)) = crate::element_geometry::bbox(m, e) else { continue };
        for k in 0..8 {
            let c = [if k & 1 == 0 { a[0] } else { b[0] }, if k & 2 == 0 { a[1] } else { b[1] }, if k & 4 == 0 { a[2] } else { b[2] }];
            let p = to_local(&v, c);
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
    }
    if !lo[0].is_finite() {
        return Ok(());
    }
    let w = m.docs.views.get_mut(&id).expect("checked");
    w.crop_min = [lo[0] - margin, lo[1] - margin, (lo[2] - margin).min(-1.0)];
    w.crop_max = [hi[0] + margin, hi[1] + margin, 0.0];
    Ok(())
}

/// The depth of the crop box (the far clip offset), metres.
pub fn far_clip_offset(v: &DocView) -> f64 {
    -v.crop_min[2]
}

/// `View.Name = ...` for any view: unique among views (OBSERVED).
pub fn set_view_name(m: &mut BimModel, id: ElementId, name: &str) -> Result<(), Failure> {
    if views_with_name(m, name, id) {
        return Err(bad("Name must be unique."));
    }
    if let Some(v) = m.docs.views.get_mut(&id) {
        v.name = name.to_owned();
    } else if let Some(t) = m.docs.templates.get_mut(&id) {
        t.name = name.to_owned();
    } else if let Some(v) = m.views.get_mut(&id) {
        v.name = name.to_owned();
    } else {
        return Err(not_found(id, "view"));
    }
    Ok(())
}

/// The crop box setter: `min z` follows the far clip offset and is not moved (OBSERVED).
pub fn set_crop_box(m: &mut BimModel, id: ElementId, min: Option<V3>, max: Option<V3>) -> Result<(), Failure> {
    let v = m.docs.views.get_mut(&id).ok_or_else(|| not_found(id, "view"))?;
    if let Some(a) = min {
        v.crop_min[0] = a[0];
        v.crop_min[1] = a[1];
    }
    if let Some(b) = max {
        v.crop_max = b;
    }
    Ok(())
}

/// The far clip offset parameter: read-only when the view does not clip (OBSERVED: "The parameter is read-only").
pub fn set_far_clip_offset(m: &mut BimModel, id: ElementId, offset: f64) -> Result<(), Failure> {
    let v = m.docs.views.get_mut(&id).ok_or_else(|| not_found(id, "view"))?;
    if v.far_clip == 0 {
        return Err(bad("The parameter is read-only."));
    }
    v.crop_min[2] = -offset;
    Ok(())
}

/// The detail levels and display styles a section-like view stores, as the API reports them (the names the compat replay writes).
pub const DOC_DETAIL_LEVELS: [&str; 3] = ["Coarse", "Medium", "Fine"];
pub const DOC_DISPLAY_STYLES: [&str; 4] = ["Wireframe", "HLR", "Shading", "FlatColors"];

/// A property of a section, elevation or callout view the Properties palette sets: `scale` (1:n, 1..=24000),
/// `detail_level`, `display_style`, `crop_active`, `crop_visible` (the three crop properties are independent), `far_clip`
/// (0 / 1 / 2), `far_clip_offset` (read-only when the view does not clip). the plan view's own setters
/// (`ops::set_view_scale_denominator` ...) are what the values and the refusals copy. A view with a template refuses the properties the
/// template owns (scale, detail level, display style), as the plan's palette does (`view_has_template`).
pub fn set_doc_property(m: &mut BimModel, id: ElementId, key: &str, text: &str, number: f64) -> Result<(), Failure> {
    if !m.docs.views.contains_key(&id) {
        return Err(not_found(id, "view"));
    }
    let owned_by_template = matches!(key, "scale" | "detail_level" | "display_style") && view_has_template(m, id);
    if owned_by_template {
        return Err(bad("The view's template controls this parameter."));
    }
    if key == "far_clip_offset" {
        return set_far_clip_offset(m, id, number);
    }
    let v = m.docs.views.get_mut(&id).ok_or_else(|| not_found(id, "view"))?;
    match key {
        "scale" => {
            let s = number.round();
            if !(1.0..=crate::spec::MAX_VIEW_SCALE as f64).contains(&s) {
                return Err(bad("The scale value must be between 1 and 24000."));
            }
            v.scale = s as u32;
        }
        "detail_level" => {
            let level = DOC_DETAIL_LEVELS.iter().find(|l| l.eq_ignore_ascii_case(text)).ok_or_else(|| bad(&format!("'{text}' is not a detail level")))?;
            v.detail_level = (*level).to_owned();
        }
        "display_style" => {
            let style = DOC_DISPLAY_STYLES.iter().find(|s| s.eq_ignore_ascii_case(text)).ok_or_else(|| bad(&format!("'{text}' is not a display style")))?;
            v.display_style = (*style).to_owned();
        }
        "crop_active" => v.crop_active = number != 0.0,
        "crop_visible" => v.crop_visible = number != 0.0,
        // `Far Clipping`: 0 no clip, 1 clip without lines, 2 clip with lines (the field's own coding, see `DocView::far_clip`)
        "far_clip" => {
            let c = number.round();
            if !(0.0..=2.0).contains(&c) {
                return Err(bad("Far Clipping is 0 (no clip), 1 (clip without line) or 2 (clip with line)."));
            }
            v.far_clip = c as i32;
        }
        other => return Err(bad(&format!("'{other}' is not a property of a section-like view"))),
    }
    Ok(())
}

/// Move the section line or marker of a view (the frame moves with it). The move vector is added to the front cut points
/// and to the view data point, the viewer's bounded space is translated by it (the origin moves, the directions do not) and
/// so is every leader of the marker.
pub fn move_view(m: &mut BimModel, id: ElementId, d: V3) -> Result<(), Failure> {
    crate::ops::require_numbers("The move", &d)?;
    let v = m.docs.views.get_mut(&id).ok_or_else(|| not_found(id, "view"))?;
    v.origin = add(v.origin, d);
    Ok(())
}

/// Rotate the frame of a view about the vertical axis through `(cx, cy)` (OBSERVED: origin, view direction and right
/// direction turn; the crop box in its own frame does not). A rotation transform at the line's origin about its direction
/// by the angle, applied to the whole section.
pub fn rotate_view(m: &mut BimModel, id: ElementId, center: [f64; 2], angle: f64) -> Result<(), Failure> {
    crate::ops::require_numbers("The rotation", &[center[0], center[1], angle])?;
    let v = m.docs.views.get_mut(&id).ok_or_else(|| not_found(id, "view"))?;
    let (s, c) = angle.sin_cos();
    let rot = |p: V3| [p[0] * c - p[1] * s, p[0] * s + p[1] * c, p[2]];
    let o = rot([v.origin[0] - center[0], v.origin[1] - center[1], v.origin[2]]);
    v.origin = [o[0] + center[0], o[1] + center[1], o[2]];
    v.x = rot(v.x);
    v.z = rot(v.z);
    Ok(())
}

/// Delete a view of this slice and what depends on it: a marker takes its elevations, a parent its callouts, a sheet
/// viewport its place. Returns every id removed.
pub fn delete_doc_view(m: &mut BimModel, id: ElementId) -> Vec<ElementId> {
    let mut gone = vec![id];
    if m.docs.markers.contains_key(&id) {
        let views: Vec<ElementId> = m.docs.markers[&id].views.values().copied().collect();
        m.docs.markers.remove(&id);
        for v in views {
            gone.extend(delete_doc_view(m, v));
        }
        return gone;
    }
    if m.docs.views.remove(&id).is_some() {
        for mk in m.docs.markers.values_mut() {
            mk.views.retain(|_, v| *v != id);
        }
    }
    // the callouts of a view (a plan or a section) go with it
    let callouts: Vec<ElementId> = m.docs.views.values().filter(|v| v.parent == Some(id)).map(|v| v.id).collect();
    for c in callouts {
        gone.extend(delete_doc_view(m, c));
    }
    // viewports of the views removed
    let vps: Vec<ElementId> = m.docs.viewports.values().filter(|vp| gone.contains(&vp.view)).map(|vp| vp.id).collect();
    for vp in vps {
        m.docs.viewports.remove(&vp);
        gone.push(vp);
    }
    m.docs.graphics.remove(&id);
    gone
}

// ------------------------------------------------------------------------------------------------ graphics

/// The view whose hidden categories and filters a view reads and writes: its primary view when it is a dependent view
/// (walks the primary-view chain).
pub fn graphics_owner(m: &BimModel, view: ElementId) -> ElementId {
    let mut v = view;
    for _ in 0..16 {
        match m.docs.primary_views.get(&v) {
            Some(p) => v = *p,
            None => break,
        }
    }
    v
}

fn own_graphics<'a>(m: &'a BimModel, view: ElementId) -> std::borrow::Cow<'a, ViewGraphics> {
    use std::borrow::Cow;
    match m.docs.graphics.get(&view) {
        Some(g) => match g.template.and_then(|t| m.docs.templates.get(&t)) {
            Some(t) => Cow::Borrowed(&t.graphics),
            None => Cow::Borrowed(g),
        },
        None => Cow::Owned(ViewGraphics::default()),
    }
}

/// The graphics of a view as the reference product reads them: the template's when the view has one; a dependent view reads
/// the hidden categories and the filters of its primary view (Spec FILTER-02).
pub fn effective_graphics<'a>(m: &'a BimModel, view: ElementId) -> std::borrow::Cow<'a, ViewGraphics> {
    use std::borrow::Cow;
    let owner = graphics_owner(m, view);
    if owner == view {
        return own_graphics(m, view);
    }
    let mut g = own_graphics(m, view).into_owned();
    let p = own_graphics(m, owner);
    g.hidden_categories = p.hidden_categories.clone();
    g.shown_categories = p.shown_categories.clone();
    g.hidden_classes = p.hidden_classes.clone();
    g.filters = p.filters.clone();
    Cow::Owned(g)
}

fn graphics_mut(m: &mut BimModel, view: ElementId) -> Result<&mut ViewGraphics, Failure> {
    if !(m.views.contains_key(&view) || m.docs.views.contains_key(&view) || m.docs.templates.contains_key(&view)) {
        return Err(not_found(view, "view"));
    }
    if m.docs.templates.contains_key(&view) {
        return Ok(&mut m.docs.templates.get_mut(&view).expect("checked").graphics);
    }
    Ok(m.docs.graphics.entry(view).or_default())
}

/// Whether a template controls the view's graphics.
pub fn view_has_template(m: &BimModel, view: ElementId) -> bool {
    m.docs.graphics.get(&view).map_or(false, |g| g.template.is_some())
}

/// Categories a view can hide (the ones the observation reads; a category that is not known cannot be hidden).
pub fn can_hide(category: &str) -> bool {
    GRAPHICS_CATEGORIES.contains(&category) || DEFAULT_HIDDEN_CATEGORIES.contains(&category) || crate::view_categories::def(category).is_some()
}

/// Whether the view shows the category (the row, its parents and its class all keep it), as `View.GetCategoryHidden` negated reads
/// it for a category the dialog lists. A category the engine does not list reads as shown.
pub fn category_hidden(m: &BimModel, view: ElementId, category: &str) -> bool {
    let g = effective_graphics(m, view);
    if crate::view_categories::def(category).is_some() {
        return crate::view_categories::is_hidden(&g, category);
    }
    g.hidden_categories.contains(category)
}

/// Hides or shows a whole category class (a tab of the dialog) in a view. ENGINE CHOICE; a view with a template refuses it
/// like a category (category-class parameter lock).
pub fn set_class_hidden(m: &mut BimModel, view: ElementId, tab: crate::view_categories::Tab, hidden: bool) -> Result<(), Failure> {
    if view_has_template(m, view) {
        return Err(bad("Category cannot be hidden."));
    }
    let owner = graphics_owner(m, view);
    let g = graphics_mut(m, owner)?;
    if hidden {
        g.hidden_classes.insert(tab.code().to_owned());
    } else {
        g.hidden_classes.remove(tab.code());
    }
    Ok(())
}

/// A view with a template refuses it; a dependent view writes its primary's list.
pub fn set_filter_list(m: &mut BimModel, view: ElementId, list: Vec<AppliedFilter>) -> Result<(), Failure> {
    if view_has_template(m, view) {
        return Err(bad("Category cannot be hidden."));
    }
    for f in &list {
        if !m.docs.filters.contains_key(&f.filter) {
            return Err(not_found(f.filter, "filter"));
        }
        check_ogs(&f.ogs)?;
    }
    let owner = graphics_owner(m, view);
    graphics_mut(m, owner)?.filters = list;
    Ok(())
}

/// `View.SetCategoryHidden`.
pub fn set_category_hidden_in_view(m: &mut BimModel, view: ElementId, category: &str, hidden: bool) -> Result<(), Failure> {
    if !can_hide(category) {
        return Err(bad("The category id does not refer to a category."));
    }
    if view_has_template(m, view) {
        return Err(bad("Category cannot be hidden."));
    }
    let owner = graphics_owner(m, view);
    let default_hidden = crate::view_categories::def(category).map_or(false, |d| d.default_hidden);
    let g = graphics_mut(m, owner)?;
    if hidden {
        g.hidden_categories.insert(category.to_owned());
        g.shown_categories.remove(category);
    } else {
        g.hidden_categories.remove(category);
        if default_hidden {
            g.shown_categories.insert(category.to_owned());
        }
    }
    Ok(())
}

/// The patterns an override names must exist in the document.
pub fn check_ogs(o: &Ogs) -> Result<(), Failure> {
    for p in [&o.surface_foreground_pattern, &o.cut_foreground_pattern, &o.surface_background_pattern, &o.cut_background_pattern].into_iter().flatten() {
        if !crate::docs::fill_pattern_exists(p) {
            return Err(bad(&format!("no fill pattern {p}")));
        }
    }
    for p in [&o.projection_line_pattern, &o.cut_line_pattern].into_iter().flatten() {
        if !crate::docs::line_pattern_exists(p) {
            return Err(bad(&format!("no line pattern {p}")));
        }
    }
    Ok(())
}

pub fn set_category_overrides(m: &mut BimModel, view: ElementId, category: &str, ogs: Ogs) -> Result<(), Failure> {
    check_ogs(&ogs)?;
    if !can_hide(category) {
        return Err(bad("Category cannot be overridden."));
    }
    // And: a category a template controls is neither hidden nor overridden in the view (a template itself is edited freely)
    if !m.docs.templates.contains_key(&view) && view_has_template(m, view) {
        return Err(bad("Category cannot be overridden."));
    }
    // OBSERVED (one case): an override of the Grids category (a projection colour) reads back as none. ENGINE CHOICE: the
    // engine keeps none for that category.
    if category == "Grids" {
        graphics_mut(m, view)?;
        return Ok(());
    }
    let g = graphics_mut(m, view)?;
    if ogs == Ogs::default() {
        g.category_overrides.remove(category);
    } else {
        g.category_overrides.insert(category.to_owned(), ogs);
    }
    Ok(())
}

/// `View.SetElementOverrides`. Settings equal to the default clear the element's entry; others are stored for that element.
pub fn set_element_graphics_overrides(m: &mut BimModel, view: ElementId, element: ElementId, ogs: Ogs) -> Result<(), Failure> {
    check_ogs(&ogs)?;
    let g = graphics_mut(m, view)?;
    if ogs == Ogs::default() {
        g.element_overrides.remove(&element);
    } else {
        g.element_overrides.insert(element, ogs);
    }
    Ok(())
}

/// One override field superseded by another set's (Spec FILTER-03): halftone is always taken from the superseding settings;
/// a line weight when it is not -1, a colour or a pattern when one is set. ENGINE CHOICE: the surface transparency is taken
/// when it is not 0 (the rule does not name it).
pub fn supersede(base: &Ogs, by: &Ogs) -> Ogs {
    let mut o = base.clone();
    if by.projection_line_weight != -1 {
        o.projection_line_weight = by.projection_line_weight;
    }
    if by.cut_line_weight != -1 {
        o.cut_line_weight = by.cut_line_weight;
    }
    o.projection_line_color = by.projection_line_color.or(o.projection_line_color);
    o.cut_line_color = by.cut_line_color.or(o.cut_line_color);
    o.surface_foreground_color = by.surface_foreground_color.or(o.surface_foreground_color);
    o.cut_foreground_color = by.cut_foreground_color.or(o.cut_foreground_color);
    o.projection_line_pattern = by.projection_line_pattern.clone().or(o.projection_line_pattern);
    o.cut_line_pattern = by.cut_line_pattern.clone().or(o.cut_line_pattern);
    o.surface_foreground_pattern = by.surface_foreground_pattern.clone().or(o.surface_foreground_pattern);
    o.cut_foreground_pattern = by.cut_foreground_pattern.clone().or(o.cut_foreground_pattern);
    if by.surface_transparency != 0 {
        o.surface_transparency = by.surface_transparency;
    }
    if by.detail_level != 0 {
        o.detail_level = by.detail_level;
    }
    o.surface_background_color = by.surface_background_color.or(o.surface_background_color);
    o.cut_background_color = by.cut_background_color.or(o.cut_background_color);
    o.surface_background_pattern = by.surface_background_pattern.clone().or(o.surface_background_pattern);
    o.cut_background_pattern = by.cut_background_pattern.clone().or(o.cut_background_pattern);
    o.surface_foreground_pattern_visible = by.surface_foreground_pattern_visible;
    o.surface_background_pattern_visible = by.surface_background_pattern_visible;
    o.cut_foreground_pattern_visible = by.cut_foreground_pattern_visible;
    o.cut_background_pattern_visible = by.cut_background_pattern_visible;
    o.halftone = by.halftone;
    o
}

/// The overrides a view applies to one element.
///
/// Spec FILTER-03: the filter overrider visits the applied filters from the last to the first and pushes the overrides of
/// every one the element passes, so each field a filter sets is drawn and the filter nearest the start of the list wins the
/// fields it sets (the value the API-level getter returns is different: [`filter_settings_returned`] drops the lowest
/// match). The category overrider is pushed first, the filter overrider after it, the element (ad hoc) overrider last.
/// ENGINE CHOICE (not specified: how the stack resolves a property): each later layer supersedes the earlier one with the
/// same field rule, so element over filter over category.
pub fn effective_overrides(m: &BimModel, view: ElementId, id: ElementId) -> Ogs {
    let g = effective_graphics(m, view);
    // the category's override, a subcategory's laid over its parent's
    let mut o = Ogs::default();
    if let Some(c) = category_name(m, id) {
        let mut chain = crate::view_categories::ancestors(c);
        chain.reverse();
        chain.push(c);
        for n in chain {
            if let Some(x) = g.category_overrides.get(n) {
                o = supersede(&o, x);
            }
        }
    }
    let mut filt: Option<Ogs> = None;
    for af in g.filters.iter().rev() {
        if !af.enabled {
            continue;
        }
        let Some(f) = m.docs.filters.get(&af.filter) else { continue };
        if !filter_matches(m, f, id) {
            continue;
        }
        filt = Some(match filt {
            None => af.ogs.clone(),
            Some(prev) => supersede(&prev, &af.ogs),
        });
    }
    if let Some(f) = filt {
        o = supersede(&o, &f);
    }
    if let Some(e) = g.element_overrides.get(&id) {
        o = supersede(&o, e);
    }
    o
}

/// Exactly as Spec FILTER-03 states it: the halftone flag is always copied from the superseding settings; line weights
/// (when not -1), colours and pattern ids (when set) are copied only when set; the detail level and the surface
/// transparency are NOT touched.
pub fn overlay_overrides(base: &Ogs, by: &Ogs) -> Ogs {
    let mut o = base.clone();
    if by.projection_line_weight != -1 {
        o.projection_line_weight = by.projection_line_weight;
    }
    if by.cut_line_weight != -1 {
        o.cut_line_weight = by.cut_line_weight;
    }
    o.projection_line_color = by.projection_line_color.or(o.projection_line_color);
    o.cut_line_color = by.cut_line_color.or(o.cut_line_color);
    o.surface_foreground_color = by.surface_foreground_color.or(o.surface_foreground_color);
    o.cut_foreground_color = by.cut_foreground_color.or(o.cut_foreground_color);
    o.projection_line_pattern = by.projection_line_pattern.clone().or(o.projection_line_pattern);
    o.cut_line_pattern = by.cut_line_pattern.clone().or(o.cut_line_pattern);
    o.surface_foreground_pattern = by.surface_foreground_pattern.clone().or(o.surface_foreground_pattern);
    o.cut_foreground_pattern = by.cut_foreground_pattern.clone().or(o.cut_foreground_pattern);
    o.surface_background_color = by.surface_background_color.or(o.surface_background_color);
    o.cut_background_color = by.cut_background_color.or(o.cut_background_color);
    o.surface_background_pattern = by.surface_background_pattern.clone().or(o.surface_background_pattern);
    o.cut_background_pattern = by.cut_background_pattern.clone().or(o.cut_background_pattern);
    // the halftone flag and the four pattern-visible flags are always copied from the superseding settings
    o.surface_foreground_pattern_visible = by.surface_foreground_pattern_visible;
    o.surface_background_pattern_visible = by.surface_background_pattern_visible;
    o.cut_foreground_pattern_visible = by.cut_foreground_pattern_visible;
    o.cut_background_pattern_visible = by.cut_background_pattern_visible;
    o.halftone = by.halftone;
    o
}

/// `None` when none matches (or the view has no filter); one match gives its settings as stored; from the second match the
/// settings of the first-visited (lowest) match are DROPPED (the second match's settings are assigned to the scratch object
/// and then superseded by themselves) and each further match supersedes the result field by field ([`overlay_overrides`]); the
/// match nearest the start of the list has the last word. The visibility and enabled flags are not tested here. This is the
/// value the call returns; what a drawing shows is [`effective_overrides`] (the filter overrider pushes every matching
/// entry, Spec FILTER-03).
pub fn filter_settings_returned(m: &BimModel, view: ElementId, id: ElementId) -> Option<Ogs> {
    let g = effective_graphics(m, view);
    let mut result: Option<Ogs> = None;
    let mut matches = 0usize;
    for af in g.filters.iter().rev() {
        let Some(f) = m.docs.filters.get(&af.filter) else { continue };
        if !filter_matches(m, f, id) {
            continue;
        }
        matches += 1;
        result = Some(match (matches, result) {
            (1, _) => af.ogs.clone(),
            (2, _) => overlay_overrides(&af.ogs, &af.ogs),
            (_, Some(prev)) => overlay_overrides(&prev, &af.ogs),
            (_, None) => af.ogs.clone(),
        });
    }
    result
}

/// A view template from a view: its scale, detail level and graphics (ENGINE CHOICE). The template is a copy of the view;
/// applying it replaces the view's graphic overrides, hidden categories and filters with the template's
/// (`effective_graphics` reads the template's graphics while one is assigned). another type is refused ("The view is not
/// valid for view template creation.").
pub fn create_template(m: &mut BimModel, cx: &mut TxnContext<'_>, from: ElementId, name: Option<&str>) -> Result<ElementId, Failure> {
    if let Some(t) = crate::view_dup::view_type_of(m, from) {
        if !crate::view_dup::template_capable(t) {
            return Err(bad("The view is not valid for view template creation."));
        }
    }
    let (scale, detail) = if let Some(v) = m.views.get(&from) {
        (v.scale, format!("{:?}", v.detail_level))
    } else if let Some(v) = m.docs.views.get(&from) {
        (v.scale, v.detail_level.clone())
    } else {
        return Err(not_found(from, "view"));
    };
    let graphics = effective_graphics(m, from).into_owned();
    let name = match name {
        Some(n) => n.to_owned(),
        None => next_name(m, "Template", 1),
    };
    if views_with_name(m, &name, ElementId(0)) {
        return Err(bad("Name must be unique."));
    }
    let id = cx.new_id();
    let mut g = graphics;
    g.template = None;
    m.docs.templates.insert(id, ViewTemplate { id, name, scale, detail_level: detail, graphics: g });
    Ok(id)
}

/// `View.ViewTemplateId = ...`: apply a template or remove it (the view keeps what the template gave it).
pub fn set_view_template(m: &mut BimModel, view: ElementId, template: Option<ElementId>) -> Result<(), Failure> {
    if !(m.views.contains_key(&view) || m.docs.views.contains_key(&view)) {
        return Err(not_found(view, "view"));
    }
    match template {
        Some(t) => {
            if !m.docs.templates.contains_key(&t) {
                return Err(not_found(t, "view template"));
            }
            m.docs.graphics.entry(view).or_default().template = Some(t);
            let (scale, detail) = {
                let tp = &m.docs.templates[&t];
                (tp.scale, tp.detail_level.clone())
            };
            if let Some(v) = m.views.get_mut(&view) {
                v.scale = scale;
                v.detail_level = match detail.as_str() {
                    "Fine" => crate::model::DetailLevel::Fine,
                    "Medium" => crate::model::DetailLevel::Medium,
                    _ => crate::model::DetailLevel::Coarse,
                };
            } else if let Some(v) = m.docs.views.get_mut(&view) {
                v.scale = scale;
                v.detail_level = detail;
            }
        }
        None => {
            let eff = effective_graphics(m, view).into_owned();
            if let Some(g) = m.docs.graphics.get_mut(&view) {
                if g.template.is_some() {
                    let mut keep = eff;
                    keep.template = None;
                    *g = keep;
                }
            }
        }
    }
    Ok(())
}

/// Which parameters are filterable is table data built at start-up, not specified: the engine keeps a short list.
pub fn create_filter(m: &mut BimModel, cx: &mut TxnContext<'_>, name: &str, categories: Vec<String>, rule: FilterRule) -> Result<ElementId, Failure> {
    const FILTERABLE: [&str; 9] = ["mark", "comments", "type_name", "length", "top_offset", "base_offset", "width", "description", "keynote"];
    if name.trim().is_empty() {
        return Err(bad("name is an empty string or contains only whitespace."));
    }
    if crate::type_names::has_prohibited_char(name) {
        return Err(bad("name cannot include prohibited characters, such as {, }, [, ], |, ;, less-than sign, greater-than sign, ?, `, ~"));
    }
    if m.docs.filters.values().any(|f| f.name == name) {
        return Err(bad("The given value for name is already in use as a filter element name."));
    }
    if !FILTERABLE.contains(&rule.param.as_str()) {
        return Err(Failure::error(
            codes::VIEWFILTER_INVALID,
            "One of the given rules refers to a parameter that does not apply to this filter's categories.",
        ));
    }
    let id = cx.new_id();
    m.docs.filters.insert(id, ViewFilter { id, name: name.to_owned(), categories, rule });
    Ok(id)
}

pub fn attach_filter(m: &mut BimModel, view: ElementId, filter: ElementId, visible: Option<bool>, ogs: Option<Ogs>) -> Result<(), Failure> {
    if !m.docs.filters.contains_key(&filter) {
        return Err(not_found(filter, "filter"));
    }
    if let Some(o) = &ogs {
        check_ogs(o)?;
    }
    let g = graphics_mut(m, graphics_owner(m, view))?;
    // OBSERVED: applying a filter a view already has is refused
    if g.filters.iter().any(|f| f.filter == filter) {
        return Err(bad("Filter is already applied to the view."));
    }
    g.filters.push(AppliedFilter { filter, visible: visible.unwrap_or(true), ogs: ogs.unwrap_or_default(), enabled: true });
    Ok(())
}

/// A dependent view writes its primary's list. A disabled entry neither hides nor overrides.
pub fn set_filter_enabled(m: &mut BimModel, view: ElementId, filter: ElementId, enabled: bool) -> Result<(), Failure> {
    let g = graphics_mut(m, graphics_owner(m, view))?;
    let af = g.filters.iter_mut().find(|f| f.filter == filter).ok_or_else(|| not_found(filter, "applied filter"))?;
    af.enabled = enabled;
    Ok(())
}

pub fn remove_filter(m: &mut BimModel, view: ElementId, filter: ElementId) -> Result<(), Failure> {
    if !m.docs.filters.contains_key(&filter) {
        return Err(not_found(filter, "filter"));
    }
    let g = graphics_mut(m, graphics_owner(m, view))?;
    g.filters.retain(|f| f.filter != filter);
    Ok(())
}

/// Deleting a filter removes it from every view; deleting a template unlinks its views (they keep its graphics).
pub fn delete_filter(m: &mut BimModel, id: ElementId) -> bool {
    if m.docs.filters.remove(&id).is_none() {
        return false;
    }
    for g in m.docs.graphics.values_mut() {
        g.filters.retain(|f| f.filter != id);
    }
    for t in m.docs.templates.values_mut() {
        t.graphics.filters.retain(|f| f.filter != id);
    }
    true
}

pub fn delete_template(m: &mut BimModel, id: ElementId) -> bool {
    let Some(t) = m.docs.templates.remove(&id) else { return false };
    for g in m.docs.graphics.values_mut() {
        if g.template == Some(id) {
            let mut keep = t.graphics.clone();
            keep.template = None;
            *g = keep;
        }
    }
    true
}

/// Hide, unhide (permanent), hide temporarily, isolate temporarily, reset the temporary mode.
pub fn view_visibility(m: &mut BimModel, view: ElementId, mode: &str, ids: &[ElementId]) -> Result<(), Failure> {
    match mode {
        "hide" => {
            if let Some(v) = m.views.get_mut(&view) {
                v.hidden_elements.extend(ids.iter().copied());
            } else if m.docs.views.contains_key(&view) {
                m.docs.graphics.entry(view).or_default().hidden.extend(ids.iter().copied());
            } else {
                return Err(not_found(view, "view"));
            }
        }
        "unhide" => {
            if let Some(v) = m.views.get_mut(&view) {
                for i in ids {
                    v.hidden_elements.remove(i);
                }
            } else if m.docs.views.contains_key(&view) {
                if let Some(g) = m.docs.graphics.get_mut(&view) {
                    for i in ids {
                        g.hidden.remove(i);
                    }
                }
            } else {
                return Err(not_found(view, "view"));
            }
        }
        "hide_temporary" => graphics_mut(m, view)?.temp_hidden.extend(ids.iter().copied()),
        "isolate_temporary" => graphics_mut(m, view)?.isolated = Some(ids.iter().copied().collect()),
        "reset_temporary" => {
            let g = graphics_mut(m, view)?;
            g.temp_hidden.clear();
            g.isolated = None;
        }
        _ => return Err(bad("Unknown visibility mode.")),
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------------ the collector of a view

/// The world box of an element for the collector of a section (None: not tested).
pub fn element_box(m: &BimModel, id: ElementId) -> Option<(V3, V3)> {
    // a wall by its bounding box (OBSERVED: the box of the wall, not the footprint of its view body)
    if let Some(w) = m.walls.get(&id) {
        return wall_box(&crate::regen::wall_state(m, w).ok()?);
    }
    if let Some(b) = crate::groups::element_box(m, id) {
        return Some(b);
    }
    if let Some(r) = m.rooms.get(&id) {
        let s = crate::regen::room_state(m, r).ok()?;
        return Some(([s.bbox_min.x, s.bbox_min.y, s.bbox_min.z], [s.bbox_max.x, s.bbox_max.y, s.bbox_max.z]));
    }
    if let Some(f) = m.floors.get(&id) {
        let st = crate::regen::floor_state(m, f).ok()?;
        let xs = f.boundary.iter().map(|p| p.x);
        let ys = f.boundary.iter().map(|p| p.y);
        return Some((
            [xs.clone().fold(f64::INFINITY, f64::min), ys.clone().fold(f64::INFINITY, f64::min), st.bottom_z],
            [xs.fold(f64::NEG_INFINITY, f64::max), ys.fold(f64::NEG_INFINITY, f64::max), st.top_z],
        ));
    }
    if let Some(g) = m.grids.get(&id) {
        return Some(([g.start.x.min(g.end.x), g.start.y.min(g.end.y), -1.0e4], [g.start.x.max(g.end.x), g.start.y.max(g.end.y), 1.0e4]));
    }
    if let Some(mk) = m.docs.markers.get(&id) {
        // a marker is a point; the box is widened by nothing, and a point on the edge of a crop is outside (OBSERVED)
        return Some((mk.point, mk.point));
    }
    // Every other element that has a body (columns, wall sweeps, toposolids): the box of its body, the same rule as the
    // walls' - the collector of a section lists what its box meets (OBSERVED, for the categories the harness measured:
    // walls, hosted inserts, floors, rooms, grids, levels). That the reference product collects a column or a toposolid by
    // its box as well is ENGINE CHOICE: before this, they were collected by nothing, and so were drawn in a plan and a 3D
    // view and absent from every section and elevation.
    crate::element_geometry::bbox(m, id)
}

/// The levels of the template ("Level 1" at 0 m, "Level 2" at 4 m; ENGINE CHOICE: the elevations are those of the default
/// metric template) that a section sees besides the document's own.
pub const TEMPLATE_LEVELS_M: [f64; 2] = [0.0, 4.0];

pub fn template_levels_in(v: &DocView) -> usize {
    // with the crop box off the view sees every level (OBSERVED)
    if !v.crop_active {
        return TEMPLATE_LEVELS_M.len();
    }
    let (a, b) = (v.origin[2] + v.crop_min[1] * v.y[2], v.origin[2] + v.crop_max[1] * v.y[2]);
    TEMPLATE_LEVELS_M.iter().filter(|z| **z >= a.min(b) - TOUCH_M && **z <= a.max(b) + TOUCH_M).count()
}

/// Elements that touch the crop box count as inside (OBSERVED: a wall whose end face lies on the box is drawn).
const TOUCH_M: f64 = 1e-6;

/// The box of a wall as the collector of a section tests it (OBSERVED): the bounding box of the regenerated wall, not the
/// footprint of its view body.
pub fn wall_box(s: &crate::regen::WallState) -> Option<(V3, V3)> {
    let (lo, hi) = match s.arc_bbox {
        Some(b) => b,
        None => {
            let b = s.unjoined_solid.as_ref()?.aabb();
            (b.min, b.max)
        }
    };
    Some(([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]))
}

/// Whether an element is in the collector of a section-like view: its box meets the crop box, or a level lies in its height.
pub fn in_section(m: &BimModel, v: &DocView, id: ElementId) -> bool {
    in_section_with(m, v, id, None)
}

/// [`in_section`] with the element's box already in hand (a wall's regeneration is the cost of the test: a caller that has it does not
/// repeat it); `None` asks [`element_box`].
pub fn in_section_with(m: &BimModel, v: &DocView, id: ElementId, known_box: Option<(V3, V3)>) -> bool {
    let corners = |lo: V3, hi: V3| -> Vec<V3> {
        let mut c = Vec::new();
        for x in [lo[0], hi[0]] {
            for y in [lo[1], hi[1]] {
                for z in [lo[2], hi[2]] {
                    c.push([x, y, z]);
                }
            }
        }
        c
    };
    let meets = |lo: V3, hi: V3| -> bool {
        let pts: Vec<V3> = corners(lo, hi).into_iter().map(|p| to_local(v, p)).collect();
        (0..3).all(|a| {
            let (mn, mx) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), p| (l.min(p[a]), h.max(p[a])));
            mx >= v.crop_min[a] - TOUCH_M && mn <= v.crop_max[a] + TOUCH_M
        })
    };
    if let Some(l) = m.levels.get(&id) {
        // a level is a horizontal plane: seen when it crosses the height of the crop box (OBSERVED)
        if !v.crop_active {
            return true;
        }
        let (a, b) = (v.origin[2] + v.crop_min[1] * v.y[2], v.origin[2] + v.crop_max[1] * v.y[2]);
        return l.elevation >= a.min(b) - 1e-9 && l.elevation <= a.max(b) + 1e-9;
    }
    // OBSERVED: a grid is in no section, elevation or callout collector (14 measured combinations); ENGINE CHOICE: never
    if m.grids.contains_key(&id) {
        return false;
    }
    match known_box.or_else(|| element_box(m, id)) {
        Some((lo, hi)) if m.docs.markers.contains_key(&id) => {
            // strictly inside
            let p = to_local(v, lo);
            (0..3).all(|a| p[a] > v.crop_min[a] + TOUCH_M && p[a] < v.crop_max[a] - TOUCH_M) || {
                // a plan callout's z range is open on its far side only
                let _ = hi;
                false
            }
        }
        Some((lo, hi)) => meets(lo, hi),
        None => false,
    }
}

// ------------------------------------------------------------------------------------------------ what the graphics hide

/// The category name of a model element as the view graphics name it (the keys of `GRAPHICS_CATEGORIES`), or `None`
/// for an element that is not a model element of a known category.
pub fn category_name(m: &BimModel, id: ElementId) -> Option<&'static str> {
    crate::view_categories::category_of(m, id)
}

/// Whether a view filter selects an element: its category is one of the filter's and its rule holds (OBSERVED: a wall
/// filter on the comments / the length; ENGINE CHOICE for the parameters the harness did not vary).
pub fn filter_matches(m: &BimModel, f: &ViewFilter, id: ElementId) -> bool {
    let Some(cat) = category_name(m, id) else { return false };
    // (2a): an EMPTY category set passes every element; otherwise the element's category must be in it
    if !f.categories.is_empty() && !f.categories.iter().any(|c| c == cat) {
        return false;
    }
    let text_of = |key: &str| {
        m.docs.comments.get(&id).cloned().filter(|_| key == "comments").unwrap_or_else(|| crate::instance_params::element_text(m, id).get(key))
    };
    let (text, number): (Option<String>, Option<f64>) = match f.rule.param.as_str() {
        "comments" => (Some(text_of("comments")), None),
        "mark" => (Some(m.hosted_instances.get(&id).map(|i| i.mark.clone()).unwrap_or_else(|| text_of("mark"))), None),
        "type_name" => (m.walls.get(&id).and_then(|w| m.wall_types.get(&w.type_id)).map(|t| t.name.clone()), None),
        "length" => (None, m.walls.get(&id).and_then(|w| crate::regen::wall_state(m, w).ok()).map(|s| s.length)),
        "top_offset" => (None, m.walls.get(&id).map(|w| w.top_offset)),
        "base_offset" => (None, m.walls.get(&id).map(|w| w.base_offset)),
        "width" => (None, m.walls.get(&id).and_then(|w| m.wall_types.get(&w.type_id)).map(|t| t.structure.width())),
        _ => (None, None),
    };
    match (&f.rule.kind[..], text, number) {
        ("equals", Some(t), _) => f.rule.text.as_deref() == Some(t.as_str()),
        ("contains", Some(t), _) => f.rule.text.as_deref().map_or(false, |x| t.contains(x)),
        ("begins", Some(t), _) => f.rule.text.as_deref().map_or(false, |x| t.starts_with(x)),
        ("equals", _, Some(n)) => f.rule.number.map_or(false, |x| (n - x).abs() < 1e-6),
        ("greater", _, Some(n)) => f.rule.number.map_or(false, |x| n > x + 1e-9),
        ("less", _, Some(n)) => f.rule.number.map_or(false, |x| n < x - 1e-9),
        _ => false,
    }
}

/// Whether the graphics of a view (its own or its template's) keep an element out of the drawing: its category hidden
/// (OBSERVED: a hidden category leaves the collector), hidden temporarily or left out by an isolate (OBSERVED), or selected
/// by a filter applied with visibility off (OBSERVED). The view's own "Hide in View" set is the view's
/// (`View::hidden_elements`), not the graphics'.
pub fn hidden_by_graphics(m: &BimModel, view: ElementId, id: ElementId) -> bool {
    // the view's own permanent hide set (a section's; a plan keeps it on the view itself)
    if m.docs.graphics.get(&view).map_or(false, |g| g.hidden.contains(&id)) {
        return true;
    }
    // an imported section or elevation is a plain view and keeps its Hide in View set on it
    if m.views.get(&view).map_or(false, |v| v.hidden_elements.contains(&id)) {
        return true;
    }
    hidden_by_rules(m, &effective_graphics(m, view), id)
}

/// The rules part of [`hidden_by_graphics`]: category, class, temporary hide, isolate, filters.
fn hidden_by_rules(m: &BimModel, g: &ViewGraphics, id: ElementId) -> bool {
    if !crate::view_categories::hides_any(g) && g.temp_hidden.is_empty() && g.isolated.is_none() && g.filters.is_empty() {
        return false;
    }
    if category_name(m, id).map_or(false, |c| crate::view_categories::is_hidden(g, c)) || g.temp_hidden.contains(&id) {
        return true;
    }
    if g.isolated.as_ref().map_or(false, |iso| !iso.contains(&id)) {
        return true;
    }
    g.filters.iter().filter(|af| !af.visible && af.enabled).any(|af| m.docs.filters.get(&af.filter).map_or(false, |f| filter_matches(m, f, id)))
}

/// The ids of every model element the graphics of `view` hide (see [`hidden_by_graphics`]): what a drawing of the view
/// adds to the view's own hidden set. It walks every collection that holds an element of a listed category
/// ([`crate::view_categories::for_each_element`]), so a hidden category leaves the plan, the sections and the 3D view alike.
pub fn graphics_hidden_set(m: &BimModel, view: ElementId) -> BTreeSet<ElementId> {
    let g = effective_graphics(m, view);
    let own = m.docs.graphics.get(&view);
    let own_hidden = own.map_or(true, |g| g.hidden.is_empty());
    if own_hidden && !crate::view_categories::hides_any(&g) && g.temp_hidden.is_empty() && g.isolated.is_none() && g.filters.is_empty() {
        return BTreeSet::new();
    }
    let view_hidden = m.views.get(&view).map(|v| &v.hidden_elements);
    let mut out = BTreeSet::new();
    crate::view_categories::for_each_element(m, |id, _| {
        if own.map_or(false, |o| o.hidden.contains(&id)) || view_hidden.map_or(false, |h| h.contains(&id)) || hidden_by_rules(m, &g, id) {
            out.insert(id);
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four sides of the crop rectangle in the view's frame, in world coordinates, each with the bound's activity.
    #[test]
    fn spec_vcrop01_the_crop_marker_is_the_rectangle_the_bounds_cut_out_of_the_viewer_plane() {
        let view = |origin: V3, x: V3, y: V3, z: V3, min: V3, max: V3, active: bool| DocView {
            id: asterion_core::ElementId(1),
            kind: DocViewKind::Section,
            name: "S".into(),
            scale: 100,
            detail_level: "Coarse".into(),
            display_style: "HiddenLine".into(),
            crop_active: active,
            crop_visible: true,
            origin,
            x,
            y,
            z,
            crop_min: min,
            crop_max: max,
            far_clip: 2,
            marker: None,
            parent: None,
            level: None,
            params: Default::default(),
        };
        // a section looking along +y from (5, 0, 1): x right, y up, z toward the viewer (-y); crop 4 m wide, 3 m high
        let v = view([5.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0], [-2.0, -1.0, -3.0], [2.0, 2.0, 0.0], true);
        let l = crop_box_lines(&v).expect("a rectangle");
        assert_eq!((l[0].a, l[0].b), ([7.0, 0.0, 0.0], [7.0, 0.0, 3.0]), "x max");
        assert_eq!((l[1].a, l[1].b), ([3.0, 0.0, 0.0], [3.0, 0.0, 3.0]), "x min");
        assert_eq!((l[2].a, l[2].b), ([3.0, 0.0, 3.0], [7.0, 0.0, 3.0]), "y max");
        assert_eq!((l[3].a, l[3].b), ([3.0, 0.0, 0.0], [7.0, 0.0, 0.0]), "y min");
        assert_eq!(l[2].mid, [5.0, 0.0, 3.0]);
        assert!(l.iter().all(|s| s.active));
        // the lines lie in the plane through the origin with the viewer's normal
        assert!(l.iter().all(|s| [s.a, s.b].iter().all(|p| (p[1] - v.origin[1]).abs() < 1e-12)));
        let off = view(v.origin, v.x, v.y, v.z, v.crop_min, v.crop_max, false);
        assert!(crop_box_lines(&off).unwrap().iter().all(|s| !s.active));
        let empty = view(v.origin, v.x, v.y, v.z, [0.0; 3], [0.0; 3], true);
        assert!(crop_box_lines(&empty).is_none());
    }

    #[test]
    fn the_graphics_hide_by_category_filter_and_isolate() {
        use crate::ops;
        let mut d = crate::new_document();
        let (v, w1, w2) = d
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let v = ops::create_view(m, cx, "A", crate::model::ViewKind::FloorPlan, Some(l), None)?;
                let t = ops::create_wall_type(m, cx, "W", crate::model::CompoundStructure { layers: vec![crate::model::Layer { width: 0.2, function: crate::model::LayerFunction::Structure }], exterior_shell_layers: 0, interior_shell_layers: 0 })?;
                let mk = |m: &mut BimModel, cx: &mut TxnContext<'_>, a: f64| {
                    ops::create_wall(m, cx, ops::NewWall { wall_type: t, level: l, start: asterion_geom::Vec2::new(0.0, a), end: asterion_geom::Vec2::new(6.0, a), height: 3.0, base_offset: 0.0, flip: false, structural: false })
                };
                let w1 = mk(m, cx, 0.0)?;
                let w2 = mk(m, cx, 4.0)?;
                m.docs.comments.insert(w1, "north".into());
                Ok((v, w1, w2))
            })
            .value
            .unwrap();
        assert!(graphics_hidden_set(d.model(), v).is_empty());
        d.transact("h", false, |m, _| set_category_hidden_in_view(m, v, "Walls", true)).value.unwrap();
        assert_eq!(graphics_hidden_set(d.model(), v), [w1, w2].into_iter().collect());
        d.transact("u", false, |m, _| set_category_hidden_in_view(m, v, "Walls", false)).value.unwrap();
        let f = d
            .transact("f", false, |m, cx| create_filter(m, cx, "North", vec!["Walls".into()], FilterRule { param: "comments".into(), kind: "contains".into(), text: Some("north".into()), number: None }))
            .value
            .unwrap();
        d.transact("af", false, |m, _| attach_filter(m, v, f, Some(false), None)).value.unwrap();
        assert_eq!(graphics_hidden_set(d.model(), v), [w1].into_iter().collect());
        d.transact("iso", false, |m, _| view_visibility(m, v, "isolate_temporary", &[w2])).value.unwrap();
        assert!(hidden_by_graphics(d.model(), v, w1) && !hidden_by_graphics(d.model(), v, w2));
    }

    #[test]
    fn a_template_controls_the_graphics_of_its_views_and_removing_it_keeps_them() {
        use crate::ops;
        let mut d = crate::new_document();
        let (a, b) = d
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let a = ops::create_view(m, cx, "A", crate::model::ViewKind::FloorPlan, Some(l), None)?;
                let b = ops::create_view(m, cx, "B", crate::model::ViewKind::FloorPlan, Some(l), None)?;
                Ok((a, b))
            })
            .value
            .unwrap();
        d.transact("h", false, |m, _| set_category_hidden_in_view(m, a, "Grids", true));
        let ogs = Ogs { projection_line_weight: 8, ..Ogs::default() };
        d.transact("o", false, |m, _| set_category_overrides(m, a, "Walls", ogs.clone()));
        let t = d.transact("t", false, |m, cx| create_template(m, cx, a, Some("T"))).value.unwrap();
        d.transact("ap", false, |m, _| set_view_template(m, b, Some(t)));
        assert!(effective_graphics(d.model(), b).hidden_categories.contains("Grids"));
        assert_eq!(effective_graphics(d.model(), b).category_overrides["Walls"].projection_line_weight, 8);
        // OBSERVED: a category cannot be hidden in a view with a template
        assert!(d.transact("x", false, |m, _| set_category_hidden_in_view(m, b, "Doors", true)).value.is_none());
        // a template changed later reaches its views; removed, the view keeps what it had
        d.transact("c", false, |m, _| set_category_overrides(m, t, "Walls", Ogs { projection_line_weight: 3, ..Ogs::default() }));
        assert_eq!(effective_graphics(d.model(), b).category_overrides["Walls"].projection_line_weight, 3);
        d.transact("r", false, |m, _| set_view_template(m, b, None));
        assert_eq!(effective_graphics(d.model(), b).category_overrides["Walls"].projection_line_weight, 3);
        assert!(d.transact("y", false, |m, _| set_category_hidden_in_view(m, b, "Doors", true)).committed());
        // a pattern the document does not have is refused
        let bad = Ogs { surface_foreground_pattern: Some("Solid fill".into()), ..Ogs::default() };
        assert!(d.transact("p", false, |m, _| set_category_overrides(m, a, "Walls", bad)).value.is_none());
    }

    #[test]
    fn a_filter_is_applied_once_and_temporary_modes_are_reset() {
        use crate::ops;
        let mut d = crate::new_document();
        let (v, w) = d
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let v = ops::create_view(m, cx, "A", crate::model::ViewKind::FloorPlan, Some(l), None)?;
                let f = create_filter(m, cx, "F", vec!["Walls".into()], FilterRule { param: "length".into(), kind: "greater".into(), text: None, number: Some(1.0) })?;
                Ok((v, f))
            })
            .value
            .unwrap();
        assert!(d.transact("a", false, |m, _| attach_filter(m, v, w, Some(true), None)).committed());
        // OBSERVED: the same filter again is refused
        assert!(d.transact("b", false, |m, _| attach_filter(m, v, w, Some(false), None)).value.is_none());
        assert!(d.transact("t", false, |m, _| view_visibility(m, v, "isolate_temporary", &[])).committed());
        assert!(d.model().docs.graphics[&v].isolated.is_some());
        assert!(d.transact("r", false, |m, _| view_visibility(m, v, "reset_temporary", &[])).committed());
        assert!(d.model().docs.graphics[&v].isolated.is_none());
        assert!(d.transact("u", false, |m, _| view_visibility(m, v, "nonsense", &[])).value.is_none());
    }

    #[test]
    fn moving_and_rotating_a_section_moves_its_frame_not_its_crop_box() {
        let mut d = crate::new_document();
        let s = d
            .transact("s", false, |m, cx| add_section_view(m, cx, Some("S"), [3.0, -3.0, 1.5], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0], [-4.0, -2.0, -10.0], [4.0, 2.0, 0.0]))
            .value
            .unwrap();
        let v0 = d.model().docs.views[&s].clone();
        d.transact("m", false, |m, _| move_view(m, s, [1.0, 0.0, 0.0]));
        assert!((d.model().docs.views[&s].origin[0] - v0.origin[0] - 1.0).abs() < 1e-9);
        d.transact("r", false, |m, _| rotate_view(m, s, [0.0, 0.0], std::f64::consts::FRAC_PI_2));
        let v = &d.model().docs.views[&s];
        assert_eq!((v.crop_min, v.crop_max), (v0.crop_min, v0.crop_max));
        assert!((v.z[0] + v0.z[1]).abs() < 1e-9 && (v.z[1] - v0.z[0]).abs() < 1e-9, "the direction turned a quarter");
        // the far clip offset is read-only when the view does not clip
        d.transact("fc", false, |m, _| {
            m.docs.views.get_mut(&s).unwrap().far_clip = 0;
            Ok(())
        });
        assert!(d.transact("o", false, |m, _| set_far_clip_offset(m, s, 2.0)).value.is_none());
    }

    #[test]
    fn a_box_on_the_viewer_side_is_turned_and_its_origin_goes_to_the_corner() {
        // OBSERVED (`add_section_view` of the first section)
        let (o, x, _y, z, lo, hi) = canonical([3.0, -3.0, 1.5], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0], [-4.0, -2.0, 0.0], [4.0, 2.0, 10.0]);
        assert_eq!(x, [-1.0, 0.0, 0.0]);
        assert_eq!(z, [0.0, 1.0, 0.0]);
        assert!((o[0] - 7.0).abs() < 1e-9 && (o[1] + 3.0).abs() < 1e-9 && (o[2] + 0.5).abs() < 1e-9, "{o:?}");
        assert_eq!((lo, hi), ([0.0, 0.0, -10.0], [8.0, 4.0, 0.0]));
    }
}
