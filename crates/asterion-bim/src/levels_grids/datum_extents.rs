//! The extents of a datum (grid or level) per view: the extent type of each end, the view-specific (2D) curve, the model
//! curve written from a view, extent propagation, the maximised 3D extent and leaders (OBSERVED and 35b, public API).
//!
//! Rows followed: (maximise box, default extents) (the model outline), Spec V-03 (a datum's visibility in a plan by the
//! primary range; the plan curve of a straight grid runs reversed on the cut plane) and Spec G-02 (bubble per view, end 1
//! by default). The rules of the per-view record, propagation candidates and leaders are checked against those rules below.
//!
//! Model: a datum has two ends, end 0 and end 1 (a grid's model start and end, Spec G-02). In a view the datum lies on one
//! line (a grid in a plan: its line at the cut plane, Spec V-03; a level in a section or an elevation: the trace of the
//! view plane at the level's elevation, running along the view's right direction). A point of that line is given by its
//! parameter `t` (the dot product with the line's unit direction, end 0 -> end 1); the model extent is the interval `[t0,
//! t1]` and the view-specific curve, when one was written, another interval stored per view. The curve the view reports
//! runs from end 1 to end 0 (OBSERVED, grids in plans and levels in sections).

use crate::annot::{add, cross, dot, length, scale, sub, unit, V3};
use crate::docviews::DocViewKind;
use crate::model::{BimModel, PlanViewPlane};
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The half size of a level's default model extent along X and along Y, in metres (50 ft). OBSERVED: every level of the
/// scenario reads Model curves of +-15.24 m in the sections and elevations, centred on the project origin. The envelope of
/// a new level is the constants -50 .. +50 ft in both parameters, the same for every level (not derived from the model).
pub const LEVEL_DEFAULT_HALF_EXTENT_M: f64 = 15.24;

/// The vertical (3D) extent `Grid.Create` gives a new grid, in metres (+-15 ft). OBSERVED: a horizontal grid reads `-4.572
/// .. 4.572` in the two elevations that see it, and the plan of the level at 6 m (cut plane 7.2) does not collect it while
/// the plans at 0 and 3 m do. The interactive tool goes through Grid::create (default extents from the model outline box,
/// -1 .. +1 when it is empty) which the engine does not model: the tool keeps an unbounded grid, so this applies to the API
/// creation only ([`apply_api_default_extent`]).
pub const API_GRID_DEFAULT_VERTICAL_EXTENT_M: (f64, f64) = (-4.572, 4.572);

/// The failure codes of this slice (ASTERION's own identifiers; the reference messages are quoted as hints).
pub mod codes {
    pub const NOT_VISIBLE: &str = "datum.not_visible_in_view";
    pub const NOT_COINCIDENT: &str = "datum.curve_not_coincident";
    pub const NOT_A_CANDIDATE: &str = "datum.propagation_target_invalid";
    pub const BUBBLE_HIDDEN: &str = "datum.leader_bubble_hidden";
    pub const LEADER_INVALID: &str = "datum.leader_invalid";
    pub const NOT_A_DATUM: &str = "datum.not_a_datum";
    pub const UNSUPPORTED: &str = "datum.unsupported_view";
}

const COINCIDENCE_TOL_M: f64 = 1e-6;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtentKind {
    #[default]
    Model,
    ViewSpecific,
}

impl ExtentKind {
    /// The reference product's enumeration name (`DatumExtentType`).
    pub fn name(self) -> &'static str {
        match self {
            ExtentKind::Model => "Model",
            ExtentKind::ViewSpecific => "ViewSpecific",
        }
    }
    pub fn from_name(s: &str) -> Option<ExtentKind> {
        match s {
            "Model" => Some(ExtentKind::Model),
            "ViewSpecific" => Some(ExtentKind::ViewSpecific),
            _ => None,
        }
    }
}

/// A leader on a datum end: anchor (at the bubble end of the curve), elbow and end.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Leader {
    pub anchor: V3,
    pub elbow: V3,
    pub end: V3,
}

/// What one view stores for one datum.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DatumInView {
    pub kinds: [ExtentKind; 2],
    /// The view-specific extent `[t0, t1]` along the datum's line in this view; `None` = it follows the model extent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve_2d: Option<[f64; 2]>,
    /// The 2D curve was written running against the view's model curve (OBSERVED: a curve written from left to right is
    /// reported from left to right): the view reports it in the written direction.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip_2d: bool,
    #[serde(default)]
    pub leaders: [Option<Leader>; 2],
}

/// The per-view state of every datum, and the model extents of levels.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DatumViews {
    /// datum id -> view id -> record.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub records: BTreeMap<ElementId, BTreeMap<ElementId, DatumInView>>,
    /// A level's model extent in plan `[x min, x max, y min, y max]`; absent = the default square
    /// ([`LEVEL_DEFAULT_HALF_EXTENT_M`]).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub level_extents: BTreeMap<ElementId, [f64; 4]>,
    /// A level's model curve written running against its default direction, per plan axis (X, Y): the views along that axis
    /// report it in the written direction (OBSERVED: the model curve of L2 written -5 -> 10 from S1 reads -5 -> 10 in S1
    /// and in the parallel S2). ENGINE CHOICE: the flag is kept per axis.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub level_flips: BTreeMap<ElementId, [bool; 2]>,
    /// A level's room Computation Height above its elevation, in metres; absent = 0 (OBSERVED). Spec M-03: the plane z =
    /// level elevation + computation offset decides what bounds the rooms of the level; Spec M-07: a room's base offset is
    /// compared with it. OBSERVED: the level's "Computation Height" is writable (0 -> 1.2) and a 0.5 m partition stops
    /// bounding at 1.2. Kept here, beside the other level state of this slice.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub computation_heights: BTreeMap<ElementId, f64>,
}

impl DatumViews {
    pub fn is_empty(&self) -> bool {
        self.records.is_empty() && self.level_extents.is_empty() && self.level_flips.is_empty() && self.computation_heights.is_empty()
    }
}

/// The line a datum lies on in one view: a point at parameter 0, the unit direction (end 0 -> end 1) and the model
/// extent `[t0, t1]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DatumLine {
    pub origin: V3,
    pub dir: V3,
    pub model: [f64; 2],
    /// The view's curve runs from end 1 to end 0 (a grid in a plan, Spec V-03) rather than from end 0 to end 1 (a level in
    /// a section: OBSERVED, the curve runs from the view's left to its right in S1, S2 and both elevations).
    pub reversed: bool,
}

impl DatumLine {
    pub fn point(&self, t: f64) -> V3 {
        add(self.origin, scale(self.dir, t))
    }
    pub fn param(&self, p: V3) -> f64 {
        dot(sub(p, self.origin), self.dir)
    }
    /// The distance of a point from the (infinite) line.
    pub fn distance(&self, p: V3) -> f64 {
        let d = sub(p, self.origin);
        length(sub(d, scale(self.dir, dot(d, self.dir))))
    }
}

fn level_extent(m: &BimModel, level: ElementId) -> [f64; 4] {
    let h = LEVEL_DEFAULT_HALF_EXTENT_M;
    m.datum_views.level_extents.get(&level).copied().unwrap_or([-h, h, -h, h])
}

fn not_visible() -> Failure {
    Failure::error(codes::NOT_VISIBLE, "The datum plane cannot be visible in the view.")
}

fn is_datum(m: &BimModel, id: ElementId) -> bool {
    m.grids.contains_key(&id) || m.levels.contains_key(&id)
}

/// The plan view (an id of `m.views`) a view answers for: itself, or the parent plan of a callout.
fn plan_of(m: &BimModel, view: ElementId) -> Option<ElementId> {
    if m.views.contains_key(&view) {
        return Some(view);
    }
    let dv = m.docs.views.get(&view)?;
    match dv.kind {
        DocViewKind::CalloutPlan | DocViewKind::CalloutDetail => dv.parent.and_then(|p| plan_of(m, p)),
        _ => None,
    }
}

fn cut_z(m: &BimModel, plan: ElementId) -> Option<f64> {
    let v = m.views.get(&plan)?;
    crate::views::view_state(m, v).ok()?.planes[PlanViewPlane::Cut as usize]
}

/// * a grid in a plan: the plan's view state shows it (Spec V-03: the vertical extent against the primary range; the crop
///   on top).
/// * a straight grid in a section or an elevation: its bounded vertical plane crosses the view plane (OBSERVED: the grid
///   along the section's direction that starts 2 m behind the section line is not visible, the grid parallel to it neither;
///   the horizontal grid is visible in the two elevations across it). ENGINE CHOICE for the crop and the far clip, which
///   are not tested.
/// * a level: never in a plan; in a section or an elevation when its extent crosses the view plane (OBSERVED).
pub fn can_be_visible(m: &BimModel, datum: ElementId, view: ElementId) -> bool {
    if let Some(plan) = plan_of(m, view) {
        if m.levels.contains_key(&datum) {
            return false;
        }
        let Some(v) = m.views.get(&plan) else { return false };
        return crate::views::view_state(m, v).map_or(false, |vs| vs.grids.iter().any(|g| g.id == datum && g.visible));
    }
    let Some(dv) = m.docs.views.get(&view) else { return false };
    let n = [dv.z[0], dv.z[1], 0.0];
    if length(n) < 1e-9 {
        return false;
    }
    let n = unit(n);
    if let Some(g) = m.grids.get(&datum) {
        if g.is_curved() {
            return false; // ENGINE CHOICE: an arc grid in a section is not modelled
        }
        let d0 = dot(sub([g.start.x, g.start.y, 0.0], dv.origin), n);
        let d1 = dot(sub([g.end.x, g.end.y, 0.0], dv.origin), n);
        return d0 * d1 <= 0.0 && (d0.abs() > 1e-12 || d1.abs() > 1e-12);
    }
    if m.levels.contains_key(&datum) {
        let [x0, x1, y0, y1] = level_extent(m, datum);
        let ds: Vec<f64> = [[x0, y0], [x1, y0], [x0, y1], [x1, y1]].iter().map(|c| dot(sub([c[0], c[1], 0.0], dv.origin), n)).collect();
        return ds.iter().any(|d| *d <= 0.0) && ds.iter().any(|d| *d >= 0.0);
    }
    false
}

/// The line of a datum in a view, with its model extent. `None` when the datum cannot be visible there.
pub fn line_in_view(m: &BimModel, datum: ElementId, view: ElementId) -> Option<DatumLine> {
    if !can_be_visible(m, datum, view) {
        return None;
    }
    if let Some(g) = m.grids.get(&datum) {
        if g.is_curved() {
            return None;
        }
        let z = match plan_of(m, view) {
            Some(plan) => cut_z(m, plan)?,
            // a grid in a section / elevation is a vertical line: not modelled as a line here (its extent is vertical)
            None => return None,
        };
        let a = [g.start.x, g.start.y, z];
        let b = [g.end.x, g.end.y, z];
        let dir = unit(sub(b, a));
        // the point of the line nearest the project origin carries the parameter 0
        let origin = sub(a, scale(dir, dot(sub(a, [0.0, 0.0, z]), dir)));
        let line = DatumLine { origin, dir, model: [0.0, 0.0], reversed: true };
        let model = [line.param(a), line.param(b)];
        return Some(DatumLine { model, ..line });
    }
    let l = m.levels.get(&datum)?;
    let dv = m.docs.views.get(&view)?;
    let right = unit([dv.x[0], dv.x[1], 0.0]);
    if length(right) < 1e-9 {
        return None;
    }
    // the view plane's trace at the level's elevation: through the view origin, along the right direction
    let origin = [dv.origin[0] - right[0] * dot([dv.origin[0], dv.origin[1], 0.0], right), dv.origin[1] - right[1] * dot([dv.origin[0], dv.origin[1], 0.0], right), l.elevation];
    let [x0, x1, y0, y1] = level_extent(m, datum);
    // the extent along the right direction: the range of the corners' parameters inside the rectangle along this line
    // (an axis-aligned view reads the X or Y range; ENGINE CHOICE for an oblique one: the range of the corners)
    let model = if right[1].abs() < 1e-9 {
        let (a, b) = (x0 * right[0], x1 * right[0]);
        [a.min(b), a.max(b)]
    } else if right[0].abs() < 1e-9 {
        let (a, b) = (y0 * right[1], y1 * right[1]);
        [a.min(b), a.max(b)]
    } else {
        let ts: Vec<f64> = [[x0, y0], [x1, y0], [x0, y1], [x1, y1]].iter().map(|c| c[0] * right[0] + c[1] * right[1]).collect();
        [ts.iter().cloned().fold(f64::INFINITY, f64::min), ts.iter().cloned().fold(f64::NEG_INFINITY, f64::max)]
    };
    // ENGINE CHOICE: a level's end 0 is the view's left end; the curve runs end 0 -> end 1 unless a model curve was written
    // the other way along this axis
    let flips = m.datum_views.level_flips.get(&datum).copied().unwrap_or([false, false]);
    let reversed = if right[1].abs() < 1e-9 { flips[0] } else if right[0].abs() < 1e-9 { flips[1] } else { false };
    Some(DatumLine { origin, dir: right, model, reversed })
}

fn record(m: &BimModel, datum: ElementId, view: ElementId) -> DatumInView {
    m.datum_views.records.get(&datum).and_then(|r| r.get(&view)).cloned().unwrap_or_default()
}

fn record_mut(m: &mut BimModel, datum: ElementId, view: ElementId) -> &mut DatumInView {
    m.datum_views.records.entry(datum).or_default().entry(view).or_default()
}

/// The extent type of each end in a view (`GetDatumExtentTypeInView`).
pub fn extent_kinds(m: &BimModel, datum: ElementId, view: ElementId) -> Result<[ExtentKind; 2], Failure> {
    if !can_be_visible(m, datum, view) {
        return Err(not_visible());
    }
    Ok(record(m, datum, view).kinds)
}

/// The curve a view reports for a datum (`GetCurvesInView(kind, view)`): `(start, end)` (see [`DatumLine::reversed`]).
/// OBSERVED: an end of type Model follows the model extent in the view-specific curve as well; an end of type ViewSpecific
/// with a written 2D curve takes the 2D curve's end.
pub fn curve_in_view(m: &BimModel, datum: ElementId, view: ElementId, kind: ExtentKind) -> Result<(V3, V3), Failure> {
    let line = line_in_view(m, datum, view).ok_or_else(not_visible)?;
    let rec = record(m, datum, view);
    let (t, rev) = match (kind, rec.curve_2d) {
        (ExtentKind::ViewSpecific, Some(c)) => {
            let pick = |i: usize| if rec.kinds[i] == ExtentKind::ViewSpecific { c[i] } else { line.model[i] };
            ([pick(0), pick(1)], line.reversed ^ rec.flip_2d)
        }
        _ => (line.model, line.reversed),
    };
    Ok(if rev { (line.point(t[1]), line.point(t[0])) } else { (line.point(t[0]), line.point(t[1])) })
}

/// `SetDatumExtentType(end(s), view, kind)`. OBSERVED: per end; both ends back to Model make the view-specific curve follow
/// the model again (round 35b: P0 back to V = M).
pub fn set_extent_kind(m: &mut BimModel, _cx: &mut TxnContext<'_>, datum: ElementId, view: ElementId, ends: &[usize], kind: ExtentKind) -> Result<(), Failure> {
    if !is_datum(m, datum) {
        return Err(Failure::error(codes::NOT_A_DATUM, "The element is not a datum plane.").on(&[datum]));
    }
    if !can_be_visible(m, datum, view) {
        return Err(not_visible());
    }
    let rec = record_mut(m, datum, view);
    for e in ends {
        rec.kinds[(*e).min(1)] = kind;
    }
    if rec.kinds == [ExtentKind::Model; 2] {
        rec.curve_2d = None;
        rec.flip_2d = false;
    }
    Ok(())
}

/// `SetCurveInView(kind, view, curve)`. The call refuses in this order - the datum cannot be visible in the view ("The
/// datum plane cannot be visible in the view."), then the curve is not valid in the view (it must be bound and coincident
/// with at least one of the datum's curves in that view for that extent mode: "The curve is unbound or not coincident with
/// the original one of the datum plane.") - and only then writes.
///
/// OBSERVED: refused when the datum cannot be visible in the view; a curve shorter than the short-curve tolerance is
/// refused ("Curve length is too small"); a curve that is not on the datum's own line in that view (a grid's at the plan's
/// cut plane) is refused "unbound or not coincident". A Model curve moves the datum's model extent, which every view sees
/// (a grid's start and end; the 2D curves of other views are kept). A ViewSpecific curve is stored for that view, and
/// writing it makes BOTH ends ViewSpecific (round 35b: end 0 set, end 1 converted too). The ends of the written curve are
/// matched to the datum's ends by their order along the datum (ENGINE CHOICE: every sample wrote the curve in the reference
/// product's own direction).
pub fn set_datum_extent_in_view(m: &mut BimModel, _cx: &mut TxnContext<'_>, datum: ElementId, view: ElementId, kind: ExtentKind, start: V3, end: V3) -> Result<(), Failure> {
    if !is_datum(m, datum) {
        return Err(Failure::error(codes::NOT_A_DATUM, "The element is not a datum plane.").on(&[datum]));
    }
    let line = line_in_view(m, datum, view).ok_or_else(not_visible)?;
    if length(sub(end, start)) < crate::spec::creation_curve_floor() {
        return Err(Failure::error(
            crate::spec::codes::CURVE_TOO_SHORT,
            "Curve length is too small for the reference product's tolerance (as identified by Application.ShortCurveTolerance).",
        ));
    }
    if line.distance(start) > COINCIDENCE_TOL_M || line.distance(end) > COINCIDENCE_TOL_M {
        return Err(Failure::error(codes::NOT_COINCIDENT, "The curve is unbound or not coincident with the original one of the datum plane."));
    }
    let (ta, tb) = (line.param(start), line.param(end));
    let t = [ta.min(tb), ta.max(tb)];
    // the written curve runs toward end 0 (decreasing parameter)
    let decreasing = ta > tb;
    match kind {
        ExtentKind::Model => {
            if let Some(g) = m.grids.get_mut(&datum) {
                let (a, b) = (line.point(t[0]), line.point(t[1]));
                g.start = asterion_geom::Vec2::new(a[0], a[1]);
                g.end = asterion_geom::Vec2::new(b[0], b[1]);
            } else {
                let mut ext = level_extent(m, datum);
                if line.dir[1].abs() < 1e-9 {
                    let (a, b) = (t[0] * line.dir[0], t[1] * line.dir[0]);
                    ext[0] = a.min(b);
                    ext[1] = a.max(b);
                } else if line.dir[0].abs() < 1e-9 {
                    let (a, b) = (t[0] * line.dir[1], t[1] * line.dir[1]);
                    ext[2] = a.min(b);
                    ext[3] = a.max(b);
                } else {
                    return Err(Failure::error(codes::UNSUPPORTED, "The model extent of a level is written from an axis-aligned view only."));
                }
                m.datum_views.level_extents.insert(datum, ext);
                let axis = if line.dir[1].abs() < 1e-9 { 0 } else { 1 };
                m.datum_views.level_flips.entry(datum).or_insert([false, false])[axis] = decreasing;
                if m.datum_views.level_flips[&datum] == [false, false] {
                    m.datum_views.level_flips.remove(&datum);
                }
            }
        }
        ExtentKind::ViewSpecific => {
            let rec = record_mut(m, datum, view);
            rec.curve_2d = Some(t);
            rec.flip_2d = decreasing != line.reversed;
            rec.kinds = [ExtentKind::ViewSpecific; 2];
        }
    }
    Ok(())
}

/// `GetPropagationViews(view)`.
///
/// From a section or an elevation, the ELEVATIONS parallel to it in which the datum can be visible (OBSERVED: the
/// template's North and South elevations for a section looking north; the parallel section 2 m behind is refused). In id
/// order.
pub fn propagation_views(m: &BimModel, datum: ElementId, view: ElementId) -> Vec<ElementId> {
    let mut out = Vec::new();
    if m.views.contains_key(&view) {
        for v in m.views.keys() {
            if *v != view && can_be_visible(m, datum, *v) {
                out.push(*v);
            }
        }
    } else if let Some(src) = m.docs.views.get(&view) {
        for (id, dv) in &m.docs.views {
            if *id == view || dv.kind != DocViewKind::Elevation {
                continue;
            }
            let parallel = (dot(src.z, dv.z).abs() - 1.0).abs() < 1e-9;
            if parallel && can_be_visible(m, datum, *id) {
                out.push(*id);
            }
        }
    }
    out
}

/// `PropagateToViews(view, targets)`: every target must be a candidate ([`propagation_views`]), else the whole call is
/// refused (OBSERVED: "One of the target views cannot receive the extent"); each target takes the source's extent types and
/// view-specific curve (OBSERVED: P1 reads the 2D curve 12..-4 of P0 with both ends ViewSpecific).
pub fn propagate(m: &mut BimModel, _cx: &mut TxnContext<'_>, datum: ElementId, view: ElementId, targets: &[ElementId]) -> Result<(), Failure> {
    if !can_be_visible(m, datum, view) {
        return Err(not_visible());
    }
    let candidates = propagation_views(m, datum, view);
    if targets.iter().any(|t| !candidates.contains(t)) {
        return Err(Failure::error(codes::NOT_A_CANDIDATE, "One of the target views cannot receive the extent."));
    }
    let src = record(m, datum, view);
    for t in targets {
        let rec = record_mut(m, datum, *t);
        rec.kinds = src.kinds;
        rec.curve_2d = src.curve_2d;
        rec.flip_2d = src.flip_2d;
    }
    Ok(())
}

/// The part of the model outline that the document template contributes, along X, in metres. The measured result of
/// `Maximize3DExtents` on a template document with no model elements (round 35 / 35b: -17.854660439467946 ..
/// 18.593928806024245 m along X, identical in both rounds although their walls differ) is that union along X taken as data
/// of the template; its Y and Z ranges were never measured, so they are NOT carried (an unknown axis is skipped, never
/// guessed). Acceptance to drop the constant: the boxes of the template's elements read from the template file.
pub const TEMPLATE_MODEL_OUTLINE_X_M: (f64, f64) = (-17.854660439467946, 18.593928806024245);

/// The inflation of the model outline in the datum containment test, in metres (4 ft).
const MODEL_OUTLINE_INFLATE_M: f64 = 4.0 * 0.3048;

/// An axis-aligned 3D outline whose axes may be unknown (an axis nobody has measured is `None` and skipped).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Outline3 {
    pub axis: [Option<(f64, f64)>; 3],
}

impl Outline3 {
    pub fn is_empty(&self) -> bool {
        self.axis.iter().all(|a| a.is_none())
    }
    pub fn add_range(&mut self, i: usize, lo: f64, hi: f64) {
        self.axis[i] = Some(match self.axis[i] {
            None => (lo.min(hi), lo.max(hi)),
            Some((a, b)) => (a.min(lo).min(hi), b.max(lo).max(hi)),
        });
    }
    pub fn union(&mut self, o: &Outline3) {
        for i in 0..3 {
            if let Some((lo, hi)) = o.axis[i] {
                self.add_range(i, lo, hi);
            }
        }
    }
    pub fn inflated_contains(&self, other: &Outline3, by: f64) -> bool {
        (0..3).all(|i| match (self.axis[i], other.axis[i]) {
            (Some((a, b)), Some((c, d))) => c >= a - by && d <= b + by,
            _ => true,
        })
    }
}

/// . The union of the model boxes of the elements that are not types and not in the excluded categories (cameras, guide
/// grids, sketch grids, stair railing balusters). The engine unions the solids of walls, floors / ceilings / roofs and
/// columns plus the template's contribution ([`TEMPLATE_MODEL_OUTLINE_X_M`]).
pub fn model_outline(m: &BimModel) -> Outline3 {
    let mut o = Outline3::default();
    o.add_range(0, TEMPLATE_MODEL_OUTLINE_X_M.0, TEMPLATE_MODEL_OUTLINE_X_M.1);
    let ids: Vec<ElementId> = m.walls.keys().chain(m.floors.keys()).chain(m.columns.keys()).copied().collect();
    for id in ids {
        if let Some((lo, hi)) = crate::element_geometry::bbox(m, id) {
            for i in 0..3 {
                o.add_range(i, lo[i], hi[i]);
            }
        }
    }
    o
}

/// The outline of a datum surface: a level's envelope at its elevation; a grid's line (its arc's three points when curved)
/// with its vertical extent (an unbounded one leaves Z unknown).
fn datum_outline(m: &BimModel, id: ElementId) -> Option<Outline3> {
    let mut o = Outline3::default();
    if let Some(l) = m.levels.get(&id) {
        let [x0, x1, y0, y1] = level_extent(m, id);
        o.add_range(0, x0, x1);
        o.add_range(1, y0, y1);
        o.add_range(2, l.elevation, l.elevation);
        return Some(o);
    }
    let g = m.grids.get(&id)?;
    let mut pts = vec![g.start, g.end];
    pts.extend(g.through);
    for p in &pts {
        o.add_range(0, p.x, p.x);
        o.add_range(1, p.y, p.y);
    }
    if let Some((b, t)) = g.vertical_extent {
        o.add_range(2, b, t);
    }
    Some(o)
}

/// The box `Maximize3DExtents` projects. The model outline, united with the outline of every datum of the document (all of
/// them when the model outline is empty, else those inside the model outline inflated by 4 ft), the datum being maximised
/// included.
pub fn maximise_box(m: &BimModel) -> Outline3 {
    let model = model_outline(m);
    let mut b = model;
    for id in m.levels.keys().chain(m.grids.keys()) {
        if let Some(d) = datum_outline(m, *id) {
            if model.is_empty() || model.inflated_contains(&d, MODEL_OUTLINE_INFLATE_M) {
                b.union(&d);
            }
        }
    }
    b
}

/// `Maximize3DExtents`. The datum surface's envelope is rebuilt from the projection of the 8 corners of [`maximise_box`] on
/// the surface, BOTH axes replaced: a straight grid's extent along its line becomes the range of the corners' parameters
/// (and its vertical extent the range of the corners' Z); a level's rectangle becomes the box's X and Y ranges. OBSERVED:
/// the grid is then visible in the plan of the level at 6 m, and a level's 2D curve in a section returns to its default
/// (the view records' 2D curves are dropped; the rule does not say whether a record survives: ENGINE CHOICE). The Z range
/// of the box is not carried (the template's Z range was never measured), so a maximised grid keeps an unbounded vertical
/// extent.
pub fn maximize_3d(m: &mut BimModel, _cx: &mut TxnContext<'_>, datum: ElementId) -> Result<(), Failure> {
    if !is_datum(m, datum) {
        return Err(Failure::error(codes::NOT_A_DATUM, "The element is not a datum plane.").on(&[datum]));
    }
    let b = maximise_box(m);
    if m.levels.contains_key(&datum) {
        if let (Some((x0, x1)), Some((y0, y1))) = (b.axis[0], b.axis[1]) {
            m.datum_views.level_extents.insert(datum, [x0, x1, y0, y1]);
        }
    } else if let Some(g) = m.grids.get_mut(&datum) {
        g.vertical_extent = None;
        if !g.is_curved() {
            if let (Some((x0, x1)), Some((y0, y1))) = (b.axis[0], b.axis[1]) {
                let (ddx, ddy) = (g.end.x - g.start.x, g.end.y - g.start.y);
                let len = (ddx * ddx + ddy * ddy).sqrt();
                if len > 0.0 {
                    let (dx, dy) = (ddx / len, ddy / len);
                    // the line's foot nearest the project origin carries the parameter 0 (as in `line_in_view`)
                    let t0 = g.start.x * dx + g.start.y * dy;
                    let foot = (g.start.x - dx * t0, g.start.y - dy * t0);
                    let ts: Vec<f64> = [[x0, y0], [x1, y0], [x0, y1], [x1, y1]].iter().map(|c| c[0] * dx + c[1] * dy).collect();
                    let tmin = ts.iter().cloned().fold(f64::INFINITY, f64::min);
                    let tmax = ts.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                    g.start = asterion_geom::Vec2::new(foot.0 + dx * tmin, foot.1 + dy * tmin);
                    g.end = asterion_geom::Vec2::new(foot.0 + dx * tmax, foot.1 + dy * tmax);
                }
            }
        }
    }
    if let Some(r) = m.datum_views.records.get_mut(&datum) {
        for rec in r.values_mut() {
            rec.curve_2d = None;
            rec.flip_2d = false;
        }
    }
    Ok(())
}

/// The level nearest to the elevation `z` among `levels` given in enumeration order (the reference product walks the opened
/// Level elements in ASCENDING ElementId). The kept level starts invalid with the best distance -1; a level replaces it
/// when none is kept or when `|elevation - z|` is STRICTLY smaller than the best distance, so among equidistant levels the
/// FIRST of the enumeration wins (a level that came with the template, lower id, beats one created later at the same
/// elevation). Returns the level and the offset `z - elevation`, forced to exactly 0 when `|offset| <= 1e-9` (the second
/// overload).
pub fn nearest_level_among(levels: impl IntoIterator<Item = (ElementId, f64)>, z: f64) -> Option<(ElementId, f64)> {
    let mut best: Option<(ElementId, f64, f64)> = None;
    for (id, elevation) in levels {
        let d = (elevation - z).abs();
        if best.map_or(true, |(_, _, b)| d < b) {
            best = Some((id, elevation, d));
        }
    }
    best.map(|(id, elevation, _)| {
        let off = z - elevation;
        (id, if off.abs() <= 1e-9 { 0.0 } else { off })
    })
}

/// [`nearest_level_among`] over the levels of the document. UNKNOWN: which operation asks for the nearest level of a pasted
/// wall and with which elevation, and the levels of the reference product template, which the engine does not carry as
/// elements: the copy of a wall therefore keeps its level.
pub fn nearest_level_id(m: &BimModel, z: f64) -> Option<(ElementId, f64)> {
    nearest_level_among(m.levels.values().map(|l| (l.id, l.elevation)), z)
}

/// Whether the bubble of a datum end is shown in a view (Spec G-02: per view, end 1 by default).
pub fn bubble_visible(m: &BimModel, datum: ElementId, view: ElementId, end: usize) -> bool {
    match m.grids.get(&datum) {
        Some(g) => g.bubble_in(view)[end.min(1)],
        None => crate::model::Grid::DEFAULT_BUBBLE[end.min(1)],
    }
}

/// `AddLeader(end, view)` + `SetLeader(end, view, leader)` with the given elbow and end.
///
/// OBSERVED: refused when the bubble of that end is hidden in the view ("The bubble of that end of the datum is hidden in
/// this view"), and refused for every leader the scenarios wrote with the reference product's conditions "the leader End,
/// Elbow, Anchor should be in the view; the End of leader should be in the datum plane curves; Elbow is between End and
/// Anchor". ENGINE CHOICE (no leader was ever accepted, so its stored shape is not measured): those three conditions are
/// tested literally, with the anchor at the curve's end of that datum end; an accepted leader is stored.
pub fn set_leader(m: &mut BimModel, _cx: &mut TxnContext<'_>, datum: ElementId, view: ElementId, end: usize, elbow: V3, end_point: V3) -> Result<(), Failure> {
    let line = line_in_view(m, datum, view).ok_or_else(not_visible)?;
    let end = end.min(1);
    if !bubble_visible(m, datum, view, end) {
        return Err(Failure::error(codes::BUBBLE_HIDDEN, "The bubble of that end of the datum is hidden in this view."));
    }
    let (c_start, c_end) = curve_in_view(m, datum, view, ExtentKind::ViewSpecific)?;
    // the anchor is the curve's end point at that datum end: the start when the curve runs from that end
    let runs_from_end1 = dot(sub(c_end, c_start), line.dir) < 0.0;
    let anchor = if (end == 1) == runs_from_end1 { c_start } else { c_end };
    let in_view = |p: V3| match plan_of(m, view) {
        Some(_) => (p[2] - line.origin[2]).abs() <= COINCIDENCE_TOL_M,
        None => m.docs.views.get(&view).map_or(false, |dv| {
            let n = unit([dv.z[0], dv.z[1], 0.0]);
            dot(sub(p, dv.origin), n).abs() <= COINCIDENCE_TOL_M
        }),
    };
    let t_end = line.param(end_point);
    let (lo, hi) = (line.param(c_end).min(line.param(c_start)), line.param(c_end).max(line.param(c_start)));
    let on_curve = line.distance(end_point) <= COINCIDENCE_TOL_M && t_end >= lo - COINCIDENCE_TOL_M && t_end <= hi + COINCIDENCE_TOL_M;
    let ea = sub(anchor, end_point);
    let between = length(ea) > 0.0 && {
        let s = dot(sub(elbow, end_point), ea) / dot(ea, ea);
        s > 0.0 && s < 1.0
    };
    if !(in_view(anchor) && in_view(elbow) && in_view(end_point) && on_curve && between) {
        return Err(Failure::error(
            codes::LEADER_INVALID,
            "One of the conditions is not valid: the input leader and view is not null; The leader End, Elbow, Anchor should be in the view; the End of leader should be in the datum plane curves; Elbow is between End and Anchor.",
        ));
    }
    record_mut(m, datum, view).leaders[end] = Some(Leader { anchor, elbow, end: end_point });
    Ok(())
}

/// The leader of a datum end in a view, if one was stored.
pub fn leader(m: &BimModel, datum: ElementId, view: ElementId, end: usize) -> Option<Leader> {
    record(m, datum, view).leaders[end.min(1)]
}

/// `Grid.Create`'s vertical extent on a grid just made by the API (see [`API_GRID_DEFAULT_VERTICAL_EXTENT_M`]).
pub fn apply_api_default_extent(m: &mut BimModel, grid: ElementId) {
    if let Some(g) = m.grids.get_mut(&grid) {
        g.vertical_extent = Some(API_GRID_DEFAULT_VERTICAL_EXTENT_M);
    }
}

/// The room Computation Height of a level, in metres above its elevation (0 unless written).
pub fn computation_height(m: &BimModel, level: ElementId) -> f64 {
    m.datum_views.computation_heights.get(&level).copied().unwrap_or(crate::regen::ROOM_COMPUTATION_OFFSET_M)
}

/// Write a level's room Computation Height (the level's "Computation Height" parameter; OBSERVED: committed).
pub fn set_computation_height(m: &mut BimModel, _cx: &mut TxnContext<'_>, level: ElementId, value: f64) -> Result<(), Failure> {
    m.level(level)?;
    if !value.is_finite() {
        return Err(Failure::error(crate::spec::codes::WRONG_KIND, "The computation height must be a number."));
    }
    if value == 0.0 {
        m.datum_views.computation_heights.remove(&level);
    } else {
        m.datum_views.computation_heights.insert(level, value);
    }
    Ok(())
}

/// Forget what deleted elements (datums or views) stored here.
pub fn elements_deleted(m: &mut BimModel, gone: &[ElementId]) {
    for g in gone {
        m.datum_views.records.remove(g);
        m.datum_views.level_extents.remove(g);
        m.datum_views.level_flips.remove(g);
        m.datum_views.computation_heights.remove(g);
    }
    for r in m.datum_views.records.values_mut() {
        r.retain(|v, _| !gone.contains(v));
    }
    m.datum_views.records.retain(|_, r| !r.is_empty());
}

/// The normal of a view plane, for callers that project points onto it.
pub fn view_normal(m: &BimModel, view: ElementId) -> Option<V3> {
    if plan_of(m, view).is_some() {
        return Some([0.0, 0.0, 1.0]);
    }
    let dv = m.docs.views.get(&view)?;
    Some(unit(cross(dv.x, dv.y)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ViewKind;
    use asterion_geom::Vec2;

    fn setup() -> (crate::BimDocument, [ElementId; 3], [ElementId; 2], ElementId) {
        let mut d = crate::new_document();
        let mut levels = [ElementId(0); 3];
        let mut plans = [ElementId(0); 2];
        let mut grid = ElementId(0);
        d.transact("setup", false, |m, cx| {
            for (i, z) in [0.0, 3.0, 6.0].iter().enumerate() {
                levels[i] = crate::ops::create_level(m, cx, &format!("L{i}"), *z)?;
            }
            plans[0] = crate::ops::create_view(m, cx, "P0", ViewKind::FloorPlan, Some(levels[0]), None)?;
            plans[1] = crate::ops::create_view(m, cx, "P2", ViewKind::FloorPlan, Some(levels[2]), None)?;
            grid = crate::ops::create_grid(m, cx, Some("G1"), Vec2::new(0.0, -1.0), Vec2::new(0.0, 9.0))?;
            apply_api_default_extent(m, grid);
            Ok(())
        });
        (d, levels, plans, grid)
    }

    #[test]
    fn the_api_default_extent_hides_the_grid_from_the_plan_at_6_m_until_maximised() {
        let (mut d, _, plans, g) = setup();
        assert!(can_be_visible(d.model(), g, plans[0]));
        assert!(!can_be_visible(d.model(), g, plans[1]));
        assert!(d.transact("max", false, |m, cx| maximize_3d(m, cx, g)).committed());
        assert!(can_be_visible(d.model(), g, plans[1]));
    }

    #[test]
    fn maximise_projects_the_model_outline_united_with_the_datum_outlines_on_the_datum() {
        // Both axes of the envelope are replaced by the range of the box corners.
        let (mut d, levels, _, g) = setup();
        let h = d.transact("g2", false, |m, cx| crate::ops::create_grid(m, cx, Some("G2"), Vec2::new(-1.0, 2.0), Vec2::new(9.0, 2.0)));
        let g2 = h.value.unwrap();
        assert!(d.transact("max", false, |m, cx| maximize_3d(m, cx, g2)).committed());
        let gr = &d.model().grids[&g2];
        assert_eq!((gr.start.x, gr.end.x), TEMPLATE_MODEL_OUTLINE_X_M);
        assert!((gr.start.y - 2.0).abs() < 1e-12 && (gr.end.y - 2.0).abs() < 1e-12);
        // the grid along Y takes the datum Y range: the template's Y range is not carried
        assert!(d.transact("max", false, |m, cx| maximize_3d(m, cx, g)).committed());
        let g1 = &d.model().grids[&g];
        assert!((g1.start.y + LEVEL_DEFAULT_HALF_EXTENT_M).abs() < 1e-12 && (g1.end.y - LEVEL_DEFAULT_HALF_EXTENT_M).abs() < 1e-12, "{:?}", (g1.start, g1.end));
        // a level takes the box's X range
        assert!(d.transact("max", false, |m, cx| maximize_3d(m, cx, levels[2])).committed());
        let ext = d.model().datum_views.level_extents[&levels[2]];
        assert_eq!((ext[0], ext[1]), TEMPLATE_MODEL_OUTLINE_X_M);
        // a datum outside the model outline inflated by 4 ft does not enter the box (containment on the known axes)
        let mut o = Outline3::default();
        o.add_range(0, 0.0, 1.0);
        let mut far = Outline3::default();
        far.add_range(0, 5.0, 6.0);
        assert!(!o.inflated_contains(&far, MODEL_OUTLINE_INFLATE_M));
        far.add_range(0, 1.0, 2.0);
        assert!(!o.inflated_contains(&far, MODEL_OUTLINE_INFLATE_M));
        let mut near = Outline3::default();
        near.add_range(0, 1.0, 2.0);
        assert!(o.inflated_contains(&near, MODEL_OUTLINE_INFLATE_M));
    }

    #[test]
    fn the_nearest_level_keeps_the_first_of_the_equidistant_ones() {
        // Strict less-than, ascending id: the template level (lower id) at the same elevation wins
        let levels = [(ElementId(13071), 0.0), (ElementId(208685), 0.0), (ElementId(208686), 3.0), (ElementId(13081), 4.0)];
        assert_eq!(nearest_level_among(levels, 0.0), Some((ElementId(13071), 0.0)));
        assert_eq!(nearest_level_among(levels, 3.4), Some((ElementId(208686), 0.3999999999999999)));
        assert_eq!(nearest_level_among(levels, 3.5).map(|x| x.0), Some(ElementId(208686)), "3.5 is equidistant from 3 and 4: the first wins");
        assert_eq!(nearest_level_among(levels, 1.0 + 1e-10 - 1.0).map(|x| x.1), Some(0.0), "an offset of 1e-10 is forced to 0");
        assert_eq!(nearest_level_among(std::iter::empty(), 1.0), None);
        let (d, lv, _, _) = setup();
        assert_eq!(nearest_level_id(d.model(), 5.0).map(|x| x.0), Some(lv[2]));
    }

    #[test]
    fn a_plan_curve_runs_end_to_start_at_the_cut_plane_and_a_2d_curve_converts_both_ends() {
        let (mut d, _, plans, g) = setup();
        let (s, e) = curve_in_view(d.model(), g, plans[0], ExtentKind::Model).unwrap();
        assert!((s[1] - 9.0).abs() < 1e-9 && (e[1] + 1.0).abs() < 1e-9 && (s[2] - 1.2).abs() < 1e-9, "{s:?} {e:?}");
        // a curve off the cut plane is refused
        let bad = d.transact("raw", false, |m, cx| set_datum_extent_in_view(m, cx, g, plans[0], ExtentKind::ViewSpecific, [0.0, -4.0, 0.0], [0.0, 9.0, 0.0]));
        assert!(bad.value.is_none());
        assert!(d.transact("k", false, |m, cx| set_extent_kind(m, cx, g, plans[0], &[0], ExtentKind::ViewSpecific)).committed());
        assert!(d.transact("c", false, |m, cx| set_datum_extent_in_view(m, cx, g, plans[0], ExtentKind::ViewSpecific, [0.0, 9.0, 1.2], [0.0, -4.0, 1.2])).committed());
        assert_eq!(extent_kinds(d.model(), g, plans[0]).unwrap(), [ExtentKind::ViewSpecific; 2]);
        let (s, e) = curve_in_view(d.model(), g, plans[0], ExtentKind::ViewSpecific).unwrap();
        assert!((s[1] - 9.0).abs() < 1e-9 && (e[1] + 4.0).abs() < 1e-9, "{s:?} {e:?}");
        // the model is untouched
        assert_eq!(d.model().grids[&g].start, Vec2::new(0.0, -1.0));
        // a model curve moves the grid's start and end and keeps the 2D curve
        assert!(d.transact("m", false, |m, cx| set_datum_extent_in_view(m, cx, g, plans[0], ExtentKind::Model, [0.0, 10.0, 1.2], [0.0, -2.0, 1.2])).committed());
        assert!((d.model().grids[&g].start.y + 2.0).abs() < 1e-9 && (d.model().grids[&g].end.y - 10.0).abs() < 1e-9);
        let (_, e) = curve_in_view(d.model(), g, plans[0], ExtentKind::ViewSpecific).unwrap();
        assert!((e[1] + 4.0).abs() < 1e-9);
        // back to Model on both ends: the 2D curve follows the model again
        assert!(d.transact("b", false, |m, cx| set_extent_kind(m, cx, g, plans[0], &[0, 1], ExtentKind::Model)).committed());
        let (_, e) = curve_in_view(d.model(), g, plans[0], ExtentKind::ViewSpecific).unwrap();
        assert!((e[1] + 2.0).abs() < 1e-9);
    }

    #[test]
    fn propagation_refuses_a_view_that_does_not_see_the_datum_and_leaders_need_the_bubble() {
        let (mut d, levels, plans, g) = setup();
        let p1 = d.transact("p1", false, |m, cx| crate::ops::create_view(m, cx, "P1", ViewKind::FloorPlan, Some(levels[1]), None)).value.unwrap();
        assert_eq!(propagation_views(d.model(), g, plans[0]), vec![p1]);
        assert!(d.transact("bad", false, |m, cx| propagate(m, cx, g, plans[0], &[p1, plans[1]])).value.is_none());
        assert!(d.transact("c", false, |m, cx| set_datum_extent_in_view(m, cx, g, plans[0], ExtentKind::ViewSpecific, [0.0, 12.0, 1.2], [0.0, -4.0, 1.2])).committed());
        assert!(d.transact("ok", false, |m, cx| propagate(m, cx, g, plans[0], &[p1])).committed());
        let (s, e) = curve_in_view(d.model(), g, p1, ExtentKind::ViewSpecific).unwrap();
        assert!((s[1] - 12.0).abs() < 1e-9 && (e[1] + 4.0).abs() < 1e-9 && (s[2] - 4.2).abs() < 1e-9, "{s:?} {e:?}");
        // end 0's bubble is hidden by default: no leader there
        let out = d.transact("l0", false, |m, cx| set_leader(m, cx, g, plans[0], 0, [0.5, -4.5, 1.2], [1.0, -5.0, 1.2]));
        assert!(out.value.is_none());
        // end 1: the end of the leader must lie on the datum's curve
        let out = d.transact("l1", false, |m, cx| set_leader(m, cx, g, plans[0], 1, [0.5, 13.0, 0.0], [1.0, 14.0, 0.0]));
        assert!(out.value.is_none());
        let out = d.transact("l1ok", false, |m, cx| set_leader(m, cx, g, plans[0], 1, [0.0, 11.0, 1.2], [0.0, 10.0, 1.2]));
        assert!(out.committed());
        assert!(leader(d.model(), g, plans[0], 1).is_some());
    }

    #[test]
    fn deleting_a_view_forgets_its_records() {
        let (mut d, _, plans, g) = setup();
        assert!(d.transact("k", false, |m, cx| set_extent_kind(m, cx, g, plans[0], &[0], ExtentKind::ViewSpecific)).committed());
        assert!(d.transact("del", false, |m, cx| crate::ops::delete(m, cx, plans[0])).committed());
        assert!(d.model().datum_views.records.is_empty());
    }
}
