//! Derived state of a **view**.
//!
//! Views are model elements (ADR-0004): the engine computes what a view shows and the shell only
//! renders it. Like every other derived quantity in this engine, nothing here is stored — a
//! [`ViewState`] is a pure function of the model (`crate::model::BimModel`).
//!
//! Evidence levels (see `crate::spec`):
//! * **DOCUMENTED** — the structure of a plan view range (four planes, each a level plus a signed
//!   offset, with the special "level above" / "level below" / "unlimited" bases), the validity
//!   constraint between the planes, the scale and detail-level parameters, and the fact that a
//!   plan view has one generating level.
//! * **ASSUMED** — every rule that decides *how an element appears* in a view (cut vs projection vs beyond vs hidden) and
//!   the crop test. Round 7 of the measurement harness measures these against the reference product; until then they are
//!   explicitly provisional and are marked `ASSUMED` at each rule below.

use crate::model::{BimModel, CropRegion, Grid, PlanViewPlane, RangeBase, RangePlane, View, ViewKind};
use crate::regen::floor_state;
use asterion_core::{ElementId, Failure};
use asterion_geom::{Prism, Vec2};

/// How one element appears in one view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    /// The view's cut plane passes through the element's solid: it draws as cut line work.
    Cut,
    /// Inside the primary range but not cut: it draws as projection line work.
    Projection,
    /// Between the bottom of the primary range and the view depth: projection "beyond" line work.
    Beyond,
    /// Outside the range (or cropped out): not drawn.
    Hidden,
}

impl Appearance {
    pub fn name(self) -> &'static str {
        match self {
            Appearance::Cut => "cut",
            Appearance::Projection => "projection",
            Appearance::Beyond => "beyond",
            Appearance::Hidden => "hidden",
        }
    }

    pub fn is_drawn(self) -> bool {
        self != Appearance::Hidden
    }
}

/// One element as seen in one view.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewedElement {
    pub id: ElementId,
    pub appearance: Appearance,
    /// Plan outline of the element's solid (closing edge implicit). Empty when the element has no
    /// solid (e.g. a zero-height wall).
    pub footprint: Vec<Vec2>,
    /// The element's own elevation span in the model.
    pub z_min: f64,
    pub z_max: f64,
    /// The elevation span of the solid **the view returns**, which is not the element's own span
    /// when the view cuts it.
    ///
    /// OBSERVED: asking the reference for an element's geometry *through a view* returns the solid clipped to `[max(z_min,
    /// bottom), cut]` when **two** conditions both hold - the cut plane crosses the element, **and** the element reaches or
    /// passes the top of the primary range - and the whole solid otherwise.
    ///
    /// The second condition is the part ASTERION got wrong twice. The first correction was "clip
    /// whenever the cut plane crosses it", which round 7 falsified on a 5.0 x 0.2 x 0.9 m wall:
    /// with the range top at 2.3 and the cut lowered to 0.5, the cut plane crosses that wall and
    /// the reference still returns the **whole** 0.9 m3, while a 3.0 m wall beside it in the same
    /// view is clipped to 0.5 m3. The difference between them is that the tall one pokes out of
    /// the top of the range.
    ///
    /// All ten measured data points fit the rule, with no exception:
    ///
    /// | element (z span) | range top | cut | crosses | reaches top | returned |
    /// |---|---|---|---|---|---|
    /// | low  [0, 0.9]  | 2.3 | 1.2 | no  | no  | whole 0.900 |
    /// | low  [0, 0.9]  | 2.3 | 0.5 | yes | no  | **whole 0.900** |
    /// | low  [0, 0.9]  | 3.0 | 2.8 | no  | no  | whole 0.900 |
    /// | tall [0, 3.0]  | 2.3 | 1.2 | yes | yes | clipped 1.200 |
    /// | tall [0, 3.0]  | 2.3 | 0.5 | yes | yes | clipped 0.500 |
    /// | tall [0, 3.0]  | 3.0 | 2.8 | yes | yes (equal) | clipped 2.800 |
    /// | above [2.5, 3.5] | 2.3 | 1.2 | no  | yes | whole 1.000 |
    /// | above [2.5, 3.5] | 2.3 | 0.5 | no  | yes | whole 1.000 |
    /// | above [2.5, 3.5] | 3.0 | 2.8 | yes | yes | clipped 0.300 (from its OWN base) |
    /// | w [0, 3.0] 6.0 m | 2.3 | 1.2 | yes | yes | clipped 1.440, surface 17.280 |
    ///
    /// Note the `tall` row at top 3.0: the element's top is *equal* to the range top and it is
    /// still clipped, so the test is "reaches or passes" (`>=`), not "passes" (`>`).
    ///
    /// Also note `above`: the clip starts at the element's **own base** (2.5), not at the bottom
    /// of the range (0).
    ///
    /// OPEN, two things the measured set cannot separate:
    /// * whether the lower limit is the bottom of the primary range or the view depth - no
    ///   measured element reached below the bottom. ASTERION uses the bottom of the range.
    /// * what an **unlimited** top does. Nothing can reach an unlimited top, so ASTERION returns
    ///   the whole solid there; that branch is ASSUMED, not measured.
    pub view_z_min: f64,
    pub view_z_max: f64,
}

impl ViewedElement {
    /// The solid this element contributes to the view: clipped at the cut plane when the view
    /// cuts it, whole otherwise. `None` when the element has no solid in this view.
    pub fn view_solid(&self) -> Option<Prism> {
        if self.footprint.len() < 3 || self.view_z_max <= self.view_z_min {
            return None;
        }
        Some(Prism::new(self.footprint.clone(), self.view_z_min, self.view_z_max))
    }
}

/// A grid as seen in one view: datums are reference planes, never cut solids, so a grid is
/// either drawn (with its bubbles) or not.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewedGrid {
    pub id: ElementId,
    pub name: String,
    pub visible: bool,
    /// The grid line **as the view draws it**, which is not the model curve.
    ///
    /// OBSERVED (four grids): the drawn curve runs **opposite** to the model curve and sits at the elevation of the **cut
    /// plane**. A grid created (0, -2) -> (0, 12) is drawn (0, 12, 1.2) -> (0, -2, 1.2); a horizontal one created (-2, 0)
    /// -> (14, 0) is drawn (14, 0, 1.2) -> (-2, 0, 1.2); in the plan of the level above, z is 5.2 = 4.0 + 1.2. The model
    /// curve itself keeps the creation order - only the view's curve is reversed.
    ///
    /// OPEN: "reverses" cannot be told apart from "normalises to a descending direction" with the
    /// grids measured so far. A grid created in the descending direction has been added to
    /// scenario 027 to settle it on the next measurement.
    ///
    /// UNKNOWN (Spec V-03): the actual vector d for plan views (only the sign of its z component enters), so "reversed in
    /// every plan view" stays the harness's observation. The second reversal (per-view extents, bubble end parameter below
    /// the free end) needs per-view extent records the engine does not hold.
    pub start: Vec2,
    pub end: Vec2,
    /// Elevation the view draws the grid at: the cut plane, or `None` outside a plan view.
    pub z: Option<f64>,
    pub bubble: [bool; 2],
    /// The drawn curve as a polyline from `start` to `end` (two points for a straight grid; an arc grid by its stored
    /// samples).
    pub polyline: Vec<Vec2>,
}

/// A resolved range plane: absolute elevation, or `None` when the plane is unlimited.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedPlane {
    pub plane: PlanViewPlane,
    /// Absolute elevation in metres; `None` = unlimited (no clipping on that side).
    pub z: Option<f64>,
}

/// Derived state of a view.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewState {
    pub kind: ViewKind,
    pub scale: u32,
    /// Elevation of the generating level (plan views only).
    pub level_z: Option<f64>,
    /// Absolute elevations of the range planes, in [`PlanViewPlane::ALL`] order - which since
    /// round 7 includes the fifth, underlay-bottom plane. Each is `None` when unlimited, and all
    /// are `None` for a non-plan view.
    pub planes: [Option<f64>; 5],
    /// DOCUMENTED: the reference rejects a range whose planes are out of order
    /// (`ViewPlan.CheckPlanViewRangeValidity`); the ordering constraint is
    /// `view depth <= bottom <= cut <= top`.
    pub range_valid: bool,
    /// Effective crop rectangle (only when the crop is active).
    pub crop: Option<CropRegion>,
    /// Every graphical element of the model, classified for this view. Elements that are not
    /// drawn are kept with [`Appearance::Hidden`] so a comparison can see the whole decision.
    pub elements: Vec<ViewedElement>,
    /// Datums (grids) as seen in this view.
    pub grids: Vec<ViewedGrid>,
}

impl ViewState {
    pub fn plane(&self, p: PlanViewPlane) -> Option<f64> {
        self.planes[p as usize]
    }

    pub fn top_z(&self) -> Option<f64> {
        self.plane(PlanViewPlane::TopClip)
    }

    pub fn cut_z(&self) -> Option<f64> {
        self.plane(PlanViewPlane::Cut)
    }

    pub fn bottom_z(&self) -> Option<f64> {
        self.plane(PlanViewPlane::BottomClip)
    }

    pub fn depth_z(&self) -> Option<f64> {
        self.plane(PlanViewPlane::ViewDepth)
    }

    pub fn drawn(&self) -> impl Iterator<Item = &ViewedElement> {
        self.elements.iter().filter(|e| e.appearance.is_drawn())
    }

    pub fn count(&self, a: Appearance) -> usize {
        self.elements.iter().filter(|e| e.appearance == a).count()
    }
}

/// Resolve one range plane to an absolute elevation.
///
/// DOCUMENTED: the plane is the elevation of its base level plus its offset; `Unlimited` means the
/// view is not clipped on that side. ASSUMED: when the model has no level above/below, the plane
/// falls back to the view's own level (the reference keeps the stored special id; its effective
/// elevation in that situation is not measured yet).
pub fn resolve_plane(model: &BimModel, gen_level: ElementId, p: RangePlane) -> Result<Option<f64>, Failure> {
    let own = model.level(gen_level)?.elevation;
    let base = match p.base {
        RangeBase::AssociatedLevel => own,
        RangeBase::LevelAbove => model.level_above(gen_level).map(|l| l.elevation).unwrap_or(own),
        RangeBase::LevelBelow => model.level_below(gen_level).map(|l| l.elevation).unwrap_or(own),
        RangeBase::Unlimited => return Ok(None),
    };
    Ok(Some(base + p.offset))
}

/// DOCUMENTED (`ViewPlan.CheckPlanViewRangeValidity`): the planes must stay ordered
/// `view depth <= bottom <= cut <= top`. An unlimited top is always above, an unlimited depth
/// always below.
pub fn range_valid(top: Option<f64>, cut: Option<f64>, bottom: Option<f64>, depth: Option<f64>) -> bool {
    let cut = match cut {
        Some(c) => c,
        None => return false,
    };
    let bottom = match bottom {
        Some(b) => b,
        None => return false,
    };
    let top_ok = top.map(|t| t >= cut).unwrap_or(true);
    let depth_ok = depth.map(|d| d <= bottom).unwrap_or(true);
    top_ok && bottom <= cut && depth_ok
}

/// The part of a view's state that does not depend on the view: every graphical element's plan footprint and vertical
/// extent. A pure function of the model, computed once per BATCH of views (never stored on the model): regenerating every
/// wall body is the cost of a view's state, and a document of 1183 walls and 21 views paid it 21 times per listing.
pub struct ViewBodies {
    walls: Vec<(ElementId, Vec<Vec2>, f64, f64)>,
    floors: Vec<(ElementId, Vec<Vec2>, f64, f64)>,
}

/// Below this many items the coordination of threads costs more than it saves (the reference product's own graphics
/// traversal has the same kind of floor: 301 representations); the number is the same order of magnitude, and the benchmark
/// is what would move it.
const PARALLEL_MIN_ITEMS: usize = 256;

/// The detail level a view of scale `1:scale` takes from the document's scale table. With the ratio `r = 1 / scale`, coarse
/// when `r <= 1/50`, medium when `1/50 < r <= 1/10`, fine above, for the DEFAULT metric bounds (an imperial document uses
/// 1/48 and 1/12). UNKNOWN: the bounds stored in the template file (only the defaults are read), and that a later scale
/// edit does not apply the table.
pub fn detail_level_for_scale(scale: u32) -> crate::model::DetailLevel {
    use crate::model::DetailLevel;
    if scale >= 50 {
        DetailLevel::Coarse
    } else if scale >= 10 {
        DetailLevel::Medium
    } else {
        DetailLevel::Fine
    }
}

/// Worker threads for a pure batch: `ASTERION_THREADS` if set (1 = single thread), else the machine's parallelism up to 8.
pub fn worker_threads() -> usize {
    if let Some(n) = std::env::var("ASTERION_THREADS").ok().and_then(|s| s.trim().parse::<usize>().ok()) {
        return n.max(1);
    }
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(8)
}

/// `f` over `items`, in order. With at least [`PARALLEL_MIN_ITEMS`] items and more than one worker thread the slice is split
/// into one contiguous block per worker; the blocks are concatenated in order, so the output equals `items.iter().map(f)`.
/// `f` must be a pure function of data every thread may read (a `&BimModel`): nothing is shared mutably, and each worker
/// has its own thread-local indexes, dropped when it ends (a worker never keeps a closed document alive).
fn parallel_ordered<T: Sync, R: Send>(items: &[T], init: impl Fn() + Sync, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    const CHUNK: usize = 16;
    let threads = worker_threads().min(items.len().div_ceil(CHUNK)).max(1);
    if items.len() < PARALLEL_MIN_ITEMS || threads <= 1 {
        return items.iter().map(&f).collect();
    }
    // Small chunks taken from a shared counter balance the load (an arc wall costs far more than a straight one).
    let chunks: Vec<&[T]> = items.chunks(CHUNK).collect();
    let slots: Vec<Mutex<Option<Vec<R>>>> = (0..chunks.len()).map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    let (f, init_ref, chunks_ref, slots_ref, next_ref) = (&f, &init, &chunks, &slots, &next);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                std::thread::Builder::new()
                    .name("asterion-regen".into())
                    .stack_size(32 << 20)
                    .spawn_scoped(scope, move || {
                        init_ref();
                        loop {
                        let k = next_ref.fetch_add(1, Ordering::Relaxed);
                        let Some(chunk) = chunks_ref.get(k) else { break };
                        let done: Vec<R> = chunk.iter().map(f).collect();
                        *slots_ref[k].lock().unwrap() = Some(done);
                        }
                    })
                    .expect("spawn regeneration worker")
            })
            .collect();
        for h in handles {
            if let Err(p) = h.join() {
                std::panic::resume_unwind(p);
            }
        }
    });
    let mut out: Vec<R> = Vec::with_capacity(items.len());
    for s in slots {
        out.extend(s.into_inner().unwrap().expect("every chunk was taken"));
    }
    out
}
/// Regenerates the bodies [`view_state_with`] classifies.
pub fn view_bodies(model: &BimModel) -> Result<ViewBodies, Failure> {
    // every wall is regenerated: each is a pure function of the (immutable) model, so with enough of them the work is split
    // over the machine's cores - in contiguous blocks that are put back in wall order, so the result, and the first error if
    // there is one, are exactly those of a single thread (see `parallel_ordered`)
    let all_walls: Vec<&crate::model::Wall> = model.walls.values().collect();
    // one wall grid, built once here and shared read-only by every worker
    let shared = crate::joins::shared_wall_grid(model);
    let bodies = parallel_ordered(&all_walls, || shared.install(), |w| -> Result<(ElementId, Vec<Vec2>, f64, f64), Failure> {
        let (footprint, base_z, top_z) = crate::regen::wall_view_body(model, w)?;
        Ok((w.id, footprint, base_z, top_z))
    });
    let mut walls = Vec::with_capacity(bodies.len());
    for b in bodies {
        walls.push(b?);
    }
    // Floors only. Ceilings and roofs share the map but their appearance in a view was never
    // measured, and classifying them as floors would manufacture a rule nobody observed.
    let all_floors: Vec<&crate::model::Floor> = model.floors.values().filter(|f| f.role == crate::model::SlabRole::Floor).collect();
    let floor_bodies = parallel_ordered(&all_floors, || shared.install(), |f| -> Result<(ElementId, Vec<Vec2>, f64, f64), Failure> {
        let s = floor_state(model, f)?;
        // A floor whose body is not a flat prism (sloped, arc-edged, shape-edited) has no `solid` in its state: its outline is still
        // its sketch's, and its elevations are the state's. An EMPTY footprint classifies as hidden, so it used to vanish from the plan.
        let footprint = match s.solid.as_ref() {
            Some(p) => p.footprint.clone(),
            None if s.top_z > s.bottom_z => f.outline_polygon(),
            None => Vec::new(),
        };
        Ok((f.id, footprint, s.bottom_z, s.top_z))
    });
    let mut floors = Vec::with_capacity(floor_bodies.len());
    for b in floor_bodies {
        floors.push(b?);
    }
    Ok(ViewBodies { walls, floors })
}

/// Classifies elements of ONE view by their vertical span, without regenerating anything: the view's
/// planes and crop are resolved once. What a hover needs ("is this wall drawn here?") for every wall of a
/// large document without the cost of a whole view state.
pub struct SpanClassifier {
    view: View,
    level_z: f64,
    planes: [Option<f64>; 5],
    crop: Option<CropRegion>,
}

impl SpanClassifier {
    pub fn new(model: &BimModel, view: &View) -> Result<SpanClassifier, Failure> {
        let st = view_state_with(model, view, &ViewBodies { walls: Vec::new(), floors: Vec::new() })?;
        Ok(SpanClassifier { view: view.clone(), level_z: st.level_z.unwrap_or(0.0), planes: st.planes, crop: st.crop })
    }

    /// The appearance of an element spanning `z_min..z_max` whose plan outline is `footprint`.
    pub fn appearance(&self, z_min: f64, z_max: f64, footprint: &[Vec2]) -> Appearance {
        classify(&self.view, self.level_z, &self.planes, self.crop.as_ref(), ElementId(0), footprint.to_vec(), z_min, z_max).appearance
    }
}

/// [`view_state`] without the wall and floor bodies: the planes, level, crop, validity and grids (what the failure cases of`n/// [`view_state`] depend on), and no classified elements. It costs no regeneration, so a caller that needs the state of the`n/// view but not of every wall (the outline of one hosted instance) does not pay for all of them.
pub fn view_state_light(model: &BimModel, view: &View) -> Result<ViewState, Failure> {
    view_state_with(model, view, &ViewBodies { walls: Vec::new(), floors: Vec::new() })
}

thread_local! {
    /// The wall and floor bodies of the document generation last asked for on this thread (see [`view_bodies_of`]).
    static BODIES_OF_GENERATION: std::cell::RefCell<Option<(u64, std::sync::Arc<ViewBodies>)>> = const { std::cell::RefCell::new(None) };
}

/// The bodies of every wall and floor of `doc`'s model ([`view_bodies`]), reused while the document's
/// [`asterion_core::txn::Document::generation`] is the one they were made for. The bodies are a pure function of the model and
/// the generation names the model's state (it is renewed by every operation that can change the model and never repeats), so this
/// returns exactly what [`view_bodies`] would: it only avoids regenerating every wall for each of the several reads a single edit
/// causes (the plan, its texts, the selection outline, the hover pick ...). One entry per thread: a read of another document, or
/// of the same one after any edit, replaces it.
pub fn view_bodies_of(doc: &crate::BimDocument) -> Result<std::sync::Arc<ViewBodies>, Failure> {
    let generation = doc.generation();
    let hit = BODIES_OF_GENERATION.with(|c| c.borrow().as_ref().filter(|(g, _)| *g == generation).map(|(_, b)| b.clone()));
    if let Some(b) = hit {
        return Ok(b);
    }
    let bodies = std::sync::Arc::new(view_bodies(doc.model())?);
    BODIES_OF_GENERATION.with(|c| *c.borrow_mut() = Some((generation, bodies.clone())));
    Ok(bodies)
}

/// [`view_state`] of a view of `doc`'s model, through [`view_bodies_of`]: the same result as `view_state(doc.model(), view)`.
pub fn view_state_of(doc: &crate::BimDocument, view: &View) -> Result<ViewState, Failure> {
    let model = doc.model();
    if view.kind.is_graphical() {
        let bodies = view_bodies_of(doc)?;
        view_state_with(model, view, bodies.as_ref())
    } else {
        view_state_with(model, view, &ViewBodies { walls: Vec::new(), floors: Vec::new() })
    }
}

/// Derived state of `view`: a pure function of the model.
pub fn view_state(model: &BimModel, view: &View) -> Result<ViewState, Failure> {
    if view.kind.is_graphical() {
        view_state_with(model, view, &view_bodies(model)?)
    } else {
        view_state_with(model, view, &ViewBodies { walls: Vec::new(), floors: Vec::new() })
    }
}

/// [`view_state`] with the bodies already regenerated by [`view_bodies`] for THIS model: the same
/// result, without regenerating every wall again for each view of a listing.
pub fn view_state_with(model: &BimModel, view: &View, bodies: &ViewBodies) -> Result<ViewState, Failure> {
    let mut planes = [None; 5];
    let mut level_z = None;
    let mut valid = true;

    if view.kind.is_plan() {
        let gen = view.gen_level.ok_or_else(|| {
            Failure::error(crate::spec::codes::VIEW_NO_LEVEL, "A plan view must have a generating level.").on(&[view.id])
        })?;
        level_z = Some(model.level(gen)?.elevation);
        for p in PlanViewPlane::ALL {
            planes[p as usize] = resolve_plane(model, gen, view.range.plane(p))?;
        }
        // Only the four primary planes take part in the ordering rule; the underlay bottom does not bound the view
        // (OBSERVED).
        valid = range_valid(planes[0], planes[1], planes[2], planes[3]);
    }

    let crop = if view.crop_active { view.crop } else { None };
    let mut elements = Vec::new();
    if view.kind.is_graphical() {
        for (id, footprint, z_min, z_max) in bodies.walls.iter().chain(bodies.floors.iter()) {
            let mut e = classify(view, level_z.unwrap_or(0.0), &planes, crop.as_ref(), *id, footprint.clone(), *z_min, *z_max);
            // "Hide in View" (the view's own set of ids): an element hidden in this view is not drawn
            if view.hidden_elements.contains(id) {
                e.appearance = Appearance::Hidden;
            }
            elements.push(e);
        }
        elements.sort_by_key(|e| e.id);
    }

    let mut grids = Vec::new();
    if view.kind.is_graphical() {
        for g in model.grids.values() {
            // A grid hidden in the view keeps its curves and bubbles there (OBSERVED: a hidden grid still answers its view
            // curves, extent type and bubbles); the drawing skips it (`plan_graphics`).
            grids.push(grid_in_view(view, &planes, crop.as_ref(), g));
        }
        grids.sort_by_key(|g| g.id);
    }

    Ok(ViewState { kind: view.kind, scale: view.scale, level_z, planes, range_valid: valid, crop, elements, grids })
}

/// The elevation interval a plan view contributes to a datum's visibility test, in metres, as
/// `(low, high)`; `None` ends are unbounded.
///
/// Spec V-03: for a plan view the vertical range is built ONLY from the PRIMARY range elevations - the projections of the
/// cut plane elevation and of ONE clip: the bottom clip for a floor plan, the top clip for a ceiling plan; the range is
/// `[min, max]` of the two, using the clip elevations that regeneration keeps ordered. The view depth plane is not
/// specified there, and the top clip is not specified for a floor plan. Planes arrive resolved (`None` = unlimited): an
/// unlimited clip is an unbounded end.
///
/// UNKNOWN (Spec V-03): the cut plane being unlimited (the native reads a warning and an unspecified elevation) - it is
/// treated as an unbounded end here; and the other view classes (section, elevation, 3D) take their range from the bounded
/// space and are not modelled.
fn datum_view_range(kind: ViewKind, planes: &[Option<f64>; 5]) -> (Option<f64>, Option<f64>) {
    let cut = planes[PlanViewPlane::Cut as usize];
    let clip = if kind == ViewKind::CeilingPlan {
        planes[PlanViewPlane::TopClip as usize]
    } else {
        planes[PlanViewPlane::BottomClip as usize]
    };
    match (cut, clip) {
        (Some(c), Some(k)) => (Some(c.min(k)), Some(c.max(k))),
        // The unlimited side of the clip is unbounded in the direction the clip lies in.
        (Some(c), None) if kind == ViewKind::CeilingPlan => (Some(c), None),
        (Some(c), None) => (None, Some(c)),
        (None, Some(k)) if kind == ViewKind::CeilingPlan => (None, Some(k)),
        (None, Some(k)) => (Some(k), None),
        (None, None) => (None, None),
    }
}

/// How a grid appears in one view.
///
/// OBSERVED: a datum's visibility in a plan is decided by the **primary range**, not by the cut plane. Measured: a vertical
/// extent of [-0.5, 0.5] keeps the grid in a plan whose range is [0, 2.3] even though the cut plane (1.2) is outside that
/// extent, and removes it from a plan whose range is [4.0, 6.3]. ASTERION previously tested the cut plane, which round 8
/// falsified.
///
/// Spec V-03: the datum is visible when `max(lo_view, lo_surface) <= min(hi_view, hi_surface) + 1e-9` (feet), where the
/// view's range is [`datum_view_range`] - [bottom clip, cut plane] for a floor plan, [cut plane, top clip] for a ceiling
/// plan - and the surface range is the grid's vertical extent. A grid with no extent is always visible (ASSUMED: the
/// envelope of an unbounded datum is not specified). So an extent that lies between the cut plane and the top clip of a
/// FLOOR plan is NOT visible - the earlier rule here tested `[bottom, top]`, which the two round-8 measurements cannot tell
/// apart from this one.
///
/// The crop still applies on top of that (ASSUMED - round 8 did not vary the crop).
fn grid_in_view(view: &View, planes: &[Option<f64>; 5], crop: Option<&CropRegion>, g: &Grid) -> ViewedGrid {
    let mut visible = true;
    let mut z = None;
    if view.kind.is_plan() {
        z = planes[PlanViewPlane::Cut as usize];
        if let Some((lo, hi)) = g.vertical_extent {
            let eps = 1e-9 * asterion_core::units::METRES_PER_FOOT;
            let (view_lo, view_hi) = datum_view_range(view.kind, planes);
            let lo_max = view_lo.map_or(lo, |v| v.max(lo));
            let hi_min = view_hi.map_or(hi, |v| v.min(hi));
            visible = lo_max <= hi_min + eps;
        }
    }
    // Spec V-03: a curved datum is visible in a plan when its cylinder axis is parallel to the view direction (a vertical
    // cylinder in a plan: always) and the bounded-space test holds; the crop test below runs on the arc's samples.
    let model_poly = g.polyline();
    if visible {
        if let Some(c) = crop {
            visible = overlaps_crop(&model_poly, c);
        }
    }
    // OBSERVED: the view draws the curve reversed, at the cut plane's elevation, and bubble visibility is a PER-VIEW
    // override. A datum that is not visible in a view reports both bubbles hidden - the reference has no bubble to show
    // there.
    let bubble = if visible { g.bubble_in(view.id) } else { [false, false] };
    // OBSERVED: an ARC grid's curve in a plan runs the same way as its model curve (start to end), where a straight grid's
    // is reversed. Spec V-03 states the flip rule but not the vector d of a plan.
    if g.is_curved() {
        return ViewedGrid { id: g.id, name: g.name.clone(), visible, start: g.start, end: g.end, z, bubble, polyline: model_poly };
    }
    let polyline: Vec<Vec2> = model_poly.into_iter().rev().collect();
    ViewedGrid { id: g.id, name: g.name.clone(), visible, start: g.end, end: g.start, z, bubble, polyline }
}

/// The height of an element's top above the view's level from which a plan view clips it at the cut plane (OBSERVED: 1.9 m
/// whole, 2.0 m clipped; see `classify`).
pub const CLIP_MIN_TOP_ABOVE_LEVEL_M: f64 = 2.0;

/// ASSUMED classification rules (round 7 will measure them against the reference product):
///
/// 1. an element with no solid, or cropped out, is hidden;
/// 2. in a **plan** view: the cut plane strictly inside the element's vertical extent makes it
///    *cut*; otherwise an overlap with the primary range `[bottom, top]` makes it *projection*;
///    otherwise an overlap with `[depth, bottom)` makes it *beyond*; otherwise it is hidden;
/// 3. in a **3D / section / elevation** view: no range applies in this slice, so every element
///    with a solid draws as projection.
fn classify(
    view: &View,
    level_z: f64,
    planes: &[Option<f64>; 5],
    crop: Option<&CropRegion>,
    id: ElementId,
    footprint: Vec<Vec2>,
    z_min: f64,
    z_max: f64,
) -> ViewedElement {
    // Everything but a cut element returns its whole solid (OBSERVED).
    let whole = |appearance, footprint| ViewedElement {
        id,
        appearance,
        footprint,
        z_min,
        z_max,
        view_z_min: z_min,
        view_z_max: z_max,
    };
    let hidden = |footprint| whole(Appearance::Hidden, footprint);
    if footprint.is_empty() {
        return hidden(footprint);
    }
    if let Some(c) = crop {
        if !overlaps_crop(&footprint, c) {
            return hidden(footprint);
        }
    }
    if !view.kind.is_plan() {
        return whole(Appearance::Projection, footprint);
    }
    let top = planes[PlanViewPlane::TopClip as usize];
    let cut = planes[PlanViewPlane::Cut as usize];
    let bottom = planes[PlanViewPlane::BottomClip as usize];
    let depth = planes[PlanViewPlane::ViewDepth as usize];

    // Spec V-02: an element is IN the cut plane when `zmin < C - eps` and `C - eps <= zmax` (eps = 1e-9 ft), for both plan
    // classes - so an element whose top is exactly at the cut plane IS cut. UNKNOWN (Spec V-02): the mode `m` of the
    // element category (category lists not specified) matters to every other branch, which therefore stay the ASSUMED /
    // OBSERVED rules below.
    let cut_through = cut.map(|c| is_in_cut_plane(z_min, z_max, c)).unwrap_or(false);
    if cut_through {
        // OBSERVED: the solid is clipped only when the element ALSO reaches or passes the top of the primary range; an
        // element wholly inside the range comes back whole even though the cut plane crosses it. See
        // `ViewedElement::view_z_min` for the ten data points.
        let c = cut.expect("cut_through implies a cut plane");
        // OBSERVED: a wall whose top is less than 2.0 m above the view's level comes back whole, one at or above 2.0 m is
        // clipped at the cut plane - for every range measured (cut 0.5 / 0.8 / 1.2 / 2.0, top 1.9 / 2.3 / 3.0), with or
        // without a join, whatever the top of the range is (a 1.9 m wall under a top of 1.9 m is whole). The earlier
        // "reaches the top of the range" rule of round 7 is replaced: its three clipped rows have tops above 2.0 m. The
        // threshold lies in (1.9, 2.0].
        let reaches_top = z_max - level_z >= CLIP_MIN_TOP_ABOVE_LEVEL_M - 1e-9;
        let (view_z_min, view_z_max) = if reaches_top {
            (bottom.map(|b| z_min.max(b)).unwrap_or(z_min), c)
        } else {
            (z_min, z_max)
        };
        return ViewedElement { id, appearance: Appearance::Cut, footprint, z_min, z_max, view_z_min, view_z_max };
    }
    // This replaces the earlier ASSUMED "in the primary range = projection", which drew every wall wholly above the cut
    // plane of the story below it (the Snowdon L2 plan drew the L3 solar walls of the block above; the reference's L2 does
    // not: section 4, live capture). UNKNOWN (Spec V-02): what the helper collapses 5 to, and the per-type draw callbacks
    // of conditions 3, 5 and 2;.
    if view.kind == ViewKind::FloorPlan && cut.is_some() {
        let planes_m = PlanClassifierPlanes {
            ceiling: false,
            cut,
            top,
            bottom,
            depth,
            underlay: None,
        };
        if classify_plan_condition(z_min, z_max, &planes_m) == PlanCondition::AboveCut {
            return hidden(footprint);
        }
    }
    // ASSUMED: the overlap with the primary range is tested on the OPEN interval `(bottom, top)`,
    // so an element that merely touches a plane is not inside the primary range. This is what
    // makes a slab whose top sits exactly on the level (the usual case: the slab hangs below its
    // level, which is also the bottom of the range) fall into the view-depth band and draw as
    // "beyond" line work, which is what the reference shows in a floor plan.
    let in_primary = {
        let above_bottom = bottom.map(|b| z_max > b).unwrap_or(true);
        let below_top = top.map(|t| z_min < t).unwrap_or(true);
        above_bottom && below_top
    };
    if in_primary {
        return whole(Appearance::Projection, footprint);
    }
    let in_depth = match (depth, bottom) {
        (_, None) => false,
        (None, Some(b)) => z_min < b,
        (Some(d), Some(b)) => z_max >= d && z_min < b,
    };
    if in_depth {
        return whole(Appearance::Beyond, footprint);
    }
    hidden(footprint)
}

/// ASSUMED: the crop test is an axis-aligned overlap between the crop rectangle and the element's
/// plan bounding box. The reference crops against the region itself; for rectangular crops of
/// prismatic elements the two agree except exactly on the boundary.
fn overlaps_crop(footprint: &[Vec2], c: &CropRegion) -> bool {
    let (mut lo, mut hi) = (Vec2::new(f64::MAX, f64::MAX), Vec2::new(f64::MIN, f64::MIN));
    for p in footprint {
        lo = Vec2::new(lo.x.min(p.x), lo.y.min(p.y));
        hi = Vec2::new(hi.x.max(p.x), hi.y.max(p.y));
    }
    let (cmin, cmax) = (
        Vec2::new(c.min.x.min(c.max.x), c.min.y.min(c.max.y)),
        Vec2::new(c.min.x.max(c.max.x), c.min.y.max(c.max.y)),
    );
    hi.x >= cmin.x && lo.x <= cmax.x && hi.y >= cmin.y && lo.y <= cmax.y
}

/// The numbers are the reference enumeration's; the classifier returns only 1, 2, 3, 4, 5 and 7 (0 is "unknown" and 6
/// "above top clip" are never produced by it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanCondition {
    /// 1, `VC_Invisible`: not drawn.
    NotDrawn,
    /// 2, `VC_BelowBottomClip`: the element is in the view depth zone, beyond the bottom clip.
    BelowBottomClip,
    /// 3, `VC_BelowCutPlane`: projected, between the cut plane and the bottom clip.
    BelowCut,
    /// 4, `VC_InCutPlane`: `zmin < cut - 1e-9` and `cut - 1e-9 <= zmax`, for both plan classes.
    Cut,
    /// 5, `VC_AboveCutPlane`: in a ceiling plan the projected element between cut plane and top
    /// clip; in a floor plan an element above the cut that starts below the top clip.
    AboveCut,
    /// 7, `VC_Underlay`.
    Underlay,
    Unknown { reason: &'static str },
}

impl PlanCondition {
    /// The reference's numeric condition where it is decided, else `None`.
    pub fn reference_code(self) -> Option<u8> {
        match self {
            PlanCondition::NotDrawn => Some(1),
            PlanCondition::BelowBottomClip => Some(2),
            PlanCondition::BelowCut => Some(3),
            PlanCondition::Cut => Some(4),
            PlanCondition::AboveCut => Some(5),
            PlanCondition::Underlay => Some(7),
            PlanCondition::Unknown { .. } => None,
        }
    }
}

/// The plane elevations a plan view hands to the classifier (Spec V-02), in metres. `None` is an unlimited plane. `ceiling`
/// selects the ceiling-plan class (1 = floor plan is `false`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanClassifierPlanes {
    pub ceiling: bool,
    pub cut: Option<f64>,
    pub top: Option<f64>,
    pub bottom: Option<f64>,
    pub depth: Option<f64>,
    /// `(underlay bottom, underlay top)` elevations, or `None` when the view has no valid underlay
    /// level (the engine models no underlay level).
    pub underlay: Option<(f64, f64)>,
}

/// Spec V-02: the element-in-cut test shared by both plan classes, `zmin < C - eps` and `C - eps <= zmax`, with eps = 1e-9
/// ft, on metres.
pub fn is_in_cut_plane(z_min: f64, z_max: f64, cut: f64) -> bool {
    let eps = PLAN_CLASSIFIER_EPS_M;
    z_min < cut - eps && cut - eps <= z_max
}

/// eps of the classifier, 1e-9 in internal units (feet), in metres.
const PLAN_CLASSIFIER_EPS_M: f64 = 1e-9 * asterion_core::units::METRES_PER_FOOT;
const PLAN_CLASSIFIER_K_M: f64 = 3.999999999 * asterion_core::units::METRES_PER_FOOT;
/// The elevation a plane that names no level resolves to (Spec V-02), feet.
const UNLIMITED_FT: f64 = 999_999_999.999_999_9;

/// Spec V-02: the plan view condition of the element z range `[z_min, z_max]` (metres).
///
/// The classifier of the native plan view: inputs are the view class, the element's z range, a mode `m` in {0, 1, 2} that
/// depends on the element's category, and the elevations C (cut), B (bottom clip), T (top clip), Ub / Ut (underlay bottom /
/// top), D (view depth). Regeneration keeps them ordered - top = max(cut, top), bottom = min(cut, bottom), depth =
/// min(bottom, depth) for a floor plan and max(top, depth) for a ceiling plan - and that ordering is applied here. Every
/// comparison uses eps = 1e-9 ft; k = 3.999999999 ft.
///
/// UNKNOWN (Spec V-02) and therefore answered as [`PlanCondition::Unknown`]:
/// * All three modes are evaluated and a value is returned only when they agree, so a result never depends on a mode the
///   engine cannot know.
/// * the band-overlap test (underlay band; the band between D and B of a floor plan): its inequality is not in the rule, so
///   a decision that needs it is `Unknown`.
/// * the exact form of "D and B differ (relative 1e-9)": read here as `|D - B| > 1e-9 * max(|D|, |B|)`.
///
/// For a view with no underlay and the default range (view depth = bottom clip) every branch is
/// decided.
pub fn classify_plan_condition(z_min: f64, z_max: f64, planes: &PlanClassifierPlanes) -> PlanCondition {
    let results: Vec<PlanCondition> = (0u8..=2).map(|m| classify_plan_condition_mode(z_min, z_max, planes, m)).collect();
    if results.iter().all(|r| *r == results[0]) {
        results[0]
    } else {
        PlanCondition::Unknown { reason: "the result depends on the element category's mode (category lists not specified)" }
    }
}

fn classify_plan_condition_mode(z_min: f64, z_max: f64, p: &PlanClassifierPlanes, m: u8) -> PlanCondition {
    let eps = PLAN_CLASSIFIER_EPS_M;
    let k = PLAN_CLASSIFIER_K_M;
    let ft = asterion_core::units::METRES_PER_FOOT;
    let unlimited = UNLIMITED_FT * ft;
    let cut = p.cut.unwrap_or(-unlimited);
    let top0 = p.top.unwrap_or(unlimited);
    let bottom0 = p.bottom.unwrap_or(-unlimited);
    let depth0 = p.depth.unwrap_or(if p.ceiling { unlimited } else { -unlimited });
    let top = cut.max(top0);
    let bottom = cut.min(bottom0);
    let depth = if p.ceiling { top.max(depth0) } else { bottom.min(depth0) };
    let (ub, ut) = p.underlay.unwrap_or((unlimited, -unlimited));

    // (a) gate.
    let in_gate = if p.ceiling {
        z_min <= depth - eps && z_max >= cut - eps
    } else {
        z_min <= top - eps && z_max >= depth - k
    };
    if !in_gate {
        return if z_min < ut - eps {
            PlanCondition::Unknown { reason: "the underlay band overlap test is not specified" }
        } else {
            PlanCondition::NotDrawn
        };
    }
    // (b) the underlay flag.
    let underlay = ((z_max > ub + eps || z_min > ub - eps) && (z_max < ut + eps || z_min < ut - eps))
        || (m != 2 && ub - k - eps < z_min && z_min < ub + eps);
    // (c) cut.
    if is_in_cut_plane(z_min, z_max, cut) {
        return PlanCondition::Cut;
    }
    // (d)
    if p.ceiling {
        if (z_max > cut + eps || z_min > cut - eps) && (z_max < top + eps || z_min < top - eps) {
            return PlanCondition::AboveCut;
        }
    } else {
        if (z_max > bottom + eps || z_min > bottom - eps) && (z_max < cut + eps || z_min < cut - eps) {
            return PlanCondition::BelowCut;
        }
        if z_max < cut && ((m == 0 && bottom - k < z_max) || (m == 1 && bottom - k < z_min)) {
            return PlanCondition::BelowCut;
        }
    }
    // (e)
    if underlay {
        return PlanCondition::Underlay;
    }
    // (f)
    let differ = |a: f64, b: f64| (a - b).abs() > 1e-9 * a.abs().max(b.abs());
    if p.ceiling {
        if differ(depth, top) && top - eps < z_min && z_min < depth + eps {
            return PlanCondition::BelowBottomClip;
        }
    } else {
        if cut - eps < z_min && z_min < top + eps {
            return PlanCondition::AboveCut;
        }
        if differ(depth, bottom) {
            return PlanCondition::Unknown { reason: "the band overlap test between view depth and bottom clip is not specified" };
        }
    }
    // (g)
    PlanCondition::NotDrawn
}

/// Classify a hosted door/window instance against the cut plane of a plan view.
///
/// The cut plane is the horizontal plane at the view's cut-plane elevation. The instance's Z range is its host level
/// elevation plus the sill height, up to the sill plus the type height: the engine models no instance solid, so this is the
/// extent of its opening, ASSUMED to be the range the reference takes from its bounding box ("tightened when needed" is not
/// specified). How the instance is DRAWN in each condition is not specified and is not produced here.
pub fn hosted_plan_condition(model: &BimModel, instance: ElementId, view: ElementId) -> Result<PlanCondition, Failure> {
    let inst = model.hosted_instance(instance)?;
    let ty = model.hosted_type(inst.type_id)?;
    let v = model.view(view)?;
    if !v.kind.is_plan() {
        return Err(Failure::error(crate::spec::codes::VIEW_NOT_PLAN, "Only a plan view has a cut plane.").on(&[view]));
    }
    let st = view_state(model, v)?;
    if st.cut_z().is_none() {
        return Ok(PlanCondition::Unknown { reason: "the cut plane is unlimited" });
    }
    let base = model.level(inst.level)?.elevation;
    let z_min = base + inst.sill_height;
    let z_max = z_min + ty.height;
    let planes = PlanClassifierPlanes {
        ceiling: v.kind == ViewKind::CeilingPlan,
        cut: st.cut_z(),
        top: st.top_z(),
        bottom: st.bottom_z(),
        depth: st.depth_z(),
        underlay: None,
    };
    Ok(classify_plan_condition(z_min, z_max, &planes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CropRegion, DetailLevel, ViewRange};
    use crate::{ops, CompoundStructure, Layer, LayerFunction};
    use asterion_core::ElementId;

    struct Fx {
        doc: crate::BimDocument,
        l0: ElementId,
        l1: ElementId,
        wall: ElementId,
        floor: ElementId,
        plan: ElementId,
    }

    /// Two levels 0 m / 3 m, a 3 m tall 0.2 m wall on L0, a 0.2 m slab on L0, and a floor plan of L0.
    fn fixture() -> Fx {
        let mut doc = crate::new_document();
        let out = doc.transact("fx", false, |m, cx| {
            let l0 = ops::create_level(m, cx, "L0", 0.0)?;
            let l1 = ops::create_level(m, cx, "L1", 3.0)?;
            let wt = ops::create_wall_type(
                m,
                cx,
                "W200",
                CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?,
            )?;
            let wall = ops::create_wall(
                m,
                cx,
                ops::NewWall {
                    wall_type: wt,
                    level: l0,
                    start: Vec2::new(0.0, 0.0),
                    end: Vec2::new(5.0, 0.0),
                    height: 3.0,
                    base_offset: 0.0,
                    flip: false,
                    structural: false,
                },
            )?;
            let ft = ops::create_floor_type(m, cx, "SLAB200", 0.2)?;
            let floor = ops::create_floor(
                m,
                cx,
                ft,
                l0,
                vec![Vec2::new(0.0, 0.0), Vec2::new(5.0, 0.0), Vec2::new(5.0, 4.0), Vec2::new(0.0, 4.0)],
                0.0,
                false,
            )?;
            let plan = ops::create_view(m, cx, "Level 0", ViewKind::FloorPlan, Some(l0), None)?;
            Ok((l0, l1, wall, floor, plan))
        });
        let (l0, l1, wall, floor, plan) = out.value.expect("fixture committed");
        assert!(out.committed(), "fixture must commit");
        Fx { doc, l0, l1, wall, floor, plan }
    }

    fn state(fx: &Fx) -> ViewState {
        let m = fx.doc.model();
        view_state(m, m.view(fx.plan).unwrap()).unwrap()
    }

    #[test]
    fn plan_view_defaults_are_documented_values() {
        let fx = fixture();
        let m = fx.doc.model();
        let v = m.view(fx.plan).unwrap();
        assert_eq!(v.scale, crate::spec::DEFAULT_VIEW_SCALE);
        assert_eq!(v.detail_level, DetailLevel::Coarse);
        assert_eq!(v.gen_level, Some(fx.l0));
        assert!(!v.crop_active);
        let s = state(&fx);
        assert_eq!(s.level_z, Some(0.0));
        assert_eq!(s.top_z(), Some(ViewRange::DEFAULT_TOP_M));
        assert_eq!(s.cut_z(), Some(ViewRange::DEFAULT_CUT_M));
        assert_eq!(s.bottom_z(), Some(0.0));
        assert_eq!(s.depth_z(), Some(0.0));
        assert!(s.range_valid);
    }

    #[test]
    fn range_planes_resolve_against_level_above_below_and_unlimited() {
        let fx = fixture();
        let m = fx.doc.model();
        // top measured from the level above (3 m) with a -0.5 m offset
        assert_eq!(resolve_plane(m, fx.l0, RangePlane::new(RangeBase::LevelAbove, -0.5)).unwrap(), Some(2.5));
        // there is no level below L0: falls back to the view's own level (ASSUMED)
        assert_eq!(resolve_plane(m, fx.l0, RangePlane::new(RangeBase::LevelBelow, 0.0)).unwrap(), Some(0.0));
        // from L1 the level below is L0
        assert_eq!(resolve_plane(m, fx.l1, RangePlane::new(RangeBase::LevelBelow, 0.0)).unwrap(), Some(0.0));
        assert_eq!(resolve_plane(m, fx.l0, RangePlane::new(RangeBase::Unlimited, 0.0)).unwrap(), None);
    }

    #[test]
    fn wall_crossing_the_cut_plane_is_cut_and_the_slab_below_is_beyond() {
        let fx = fixture();
        let s = state(&fx);
        let wall = s.elements.iter().find(|e| e.id == fx.wall).unwrap();
        let floor = s.elements.iter().find(|e| e.id == fx.floor).unwrap();
        // wall spans 0..3 m, cut plane at 1.2 m -> cut
        assert_eq!(wall.appearance, Appearance::Cut);
        // slab hangs from 0 to -0.2 m: below the bottom of the primary range, inside the depth
        assert_eq!(floor.appearance, Appearance::Beyond);
        assert_eq!(s.count(Appearance::Cut), 1);
        assert_eq!(s.drawn().count(), 2);
    }

    #[test]
    fn a_wall_wholly_below_the_cut_plane_is_projection() {
        let mut fx = fixture();
        let plan = fx.plan;
        // a 1.0 m high wall: below the 1.2 m cut plane, inside the primary range [0, 2.3]
        let out = fx.doc.transact("lower", false, |m, cx| ops::set_unconnected_height(m, cx, fx.wall, 1.0));
        assert!(out.committed(), "lowering the wall must commit");
        let s = {
            let m = fx.doc.model();
            view_state(m, m.view(plan).unwrap()).unwrap()
        };
        let wall = s.elements.iter().find(|e| e.id == fx.wall).unwrap();
        // Spec V-02: condition 3, projected between the cut plane and the bottom clip
        assert_eq!(wall.appearance, Appearance::Projection);
    }

    #[test]
    fn a_wall_wholly_above_the_cut_plane_is_not_drawn_in_a_floor_plan() {
        let mut fx = fixture();
        let plan = fx.plan;
        // raise the base of the wall to 2 m: it starts above the 1.2 m cut plane and below the 2.3 m top clip
        let out = fx.doc.transact("raise", false, |m, cx| ops::set_wall_base_offset(m, cx, fx.wall, 2.0));
        assert!(out.committed(), "raising the wall must commit");
        let s = {
            let m = fx.doc.model();
            view_state(m, m.view(plan).unwrap()).unwrap()
        };
        let wall = s.elements.iter().find(|e| e.id == fx.wall).unwrap();
        // Spec V-02: condition 5 (above the cut plane); a floor plan keeps 5 for nothing
        assert_eq!(wall.appearance, Appearance::Hidden);
    }

    #[test]
    fn element_outside_the_crop_is_hidden() {
        let mut fx = fixture();
        let plan = fx.plan;
        let out = fx.doc.transact("crop", false, |m, cx| {
            ops::set_view_crop(
                m,
                cx,
                plan,
                Some(CropRegion { min: Vec2::new(50.0, 50.0), max: Vec2::new(60.0, 60.0) }),
                Some(true),
                Some(true),
            )
        });
        assert!(out.committed());
        let s = {
            let m = fx.doc.model();
            view_state(m, m.view(plan).unwrap()).unwrap()
        };
        assert_eq!(s.drawn().count(), 0, "nothing lies inside the crop rectangle");
        // the crop only hides; deactivating it brings the elements back
        let out = fx.doc.transact("uncrop", false, |m, cx| ops::set_view_crop(m, cx, plan, None, Some(false), None));
        assert!(out.committed());
        let s = {
            let m = fx.doc.model();
            view_state(m, m.view(plan).unwrap()).unwrap()
        };
        assert_eq!(s.drawn().count(), 2);
    }

    #[test]
    fn an_out_of_order_range_is_rejected_not_committed() {
        let mut fx = fixture();
        let plan = fx.plan;
        // cut plane above the top clip plane -> invalid
        let out = fx.doc.transact("bad", false, |m, cx| {
            ops::set_view_range_plane(m, cx, plan, PlanViewPlane::Cut, RangePlane::at(9.0))
        });
        assert!(out.value.is_none(), "an invalid range must be rejected outright");
        assert!(!out.committed());
        let s = state(&fx);
        assert_eq!(s.cut_z(), Some(ViewRange::DEFAULT_CUT_M), "the range must be untouched");
        // a valid move commits
        let out = fx.doc.transact("good", false, |m, cx| {
            ops::set_view_range_plane(m, cx, plan, PlanViewPlane::TopClip, RangePlane::new(RangeBase::LevelAbove, 0.0))
        });
        assert!(out.committed());
        assert_eq!(state(&fx).top_z(), Some(3.0));
    }

    #[test]
    fn three_d_view_has_no_range_and_draws_everything() {
        let mut fx = fixture();
        let out = fx.doc.transact("3d", false, |m, cx| ops::create_view(m, cx, "{3D}", ViewKind::ThreeD, None, None));
        assert!(out.committed());
        let v3d = out.value.expect("a 3D view needs no level");
        let s = {
            let m = fx.doc.model();
            view_state(m, m.view(v3d).unwrap()).unwrap()
        };
        assert_eq!(s.planes, [None; 5]);
        assert_eq!(s.drawn().count(), 2);
        assert!(s.elements.iter().all(|e| e.appearance == Appearance::Projection));
    }

    #[test]
    fn view_creation_rules_and_scale_bounds() {
        let mut fx = fixture();
        // a plan view without a level is rejected
        let out = fx.doc.transact("no level", false, |m, cx| ops::create_view(m, cx, "X", ViewKind::FloorPlan, None, None));
        assert!(out.value.is_none());
        // a duplicate name is rejected
        let out = fx.doc.transact("dup", false, |m, cx| {
            ops::create_view(m, cx, "Level 0", ViewKind::FloorPlan, Some(fx.l1), None)
        });
        assert!(out.value.is_none());
        // scale bounds
        let plan = fx.plan;
        let out = fx.doc.transact("scale0", false, |m, cx| ops::set_view_scale_denominator(m, cx, plan, 0));
        assert!(out.value.is_none());
        let out = fx.doc.transact("scale above the maximum", false, |m, cx| ops::set_view_scale_denominator(m, cx, plan, 24001));
        assert!(out.value.is_none());
        let out = fx.doc.transact("scale50", false, |m, cx| ops::set_view_scale_denominator(m, cx, plan, 50));
        assert!(out.committed());
        assert_eq!(fx.doc.model().view(plan).unwrap().scale, 50);
    }

    #[test]
    fn deleting_the_level_deletes_its_plan_view() {
        let mut fx = fixture();
        let l0 = fx.l0;
        let out = fx.doc.transact("delete level", false, |m, cx| ops::delete(m, cx, l0));
        assert!(out.committed());
        let deleted = out.value.expect("deleting a level is allowed");
        assert!(deleted.contains(&fx.plan), "the plan view generated by the level goes with it");
        assert!(fx.doc.model().views.get(&fx.plan).is_none());
    }

    #[test]
    fn moving_the_level_moves_the_whole_range() {
        let mut fx = fixture();
        let l0 = fx.l0;
        let out = fx.doc.transact("move", false, |m, cx| ops::set_level_elevation(m, cx, l0, 10.0));
        assert!(out.committed());
        let s = state(&fx);
        assert_eq!(s.level_z, Some(10.0));
        assert_eq!(s.cut_z(), Some(10.0 + ViewRange::DEFAULT_CUT_M));
        assert_eq!(s.top_z(), Some(10.0 + ViewRange::DEFAULT_TOP_M));
    }

    /// The default floor-plan planes of the engine: cut 1.2, top 2.3, bottom 0, view depth 0.
    fn default_floor_planes() -> PlanClassifierPlanes {
        PlanClassifierPlanes {
            ceiling: false,
            cut: Some(1.2),
            top: Some(2.3),
            bottom: Some(0.0),
            depth: Some(0.0),
            underlay: None,
        }
    }

    /// Spec V-02: the inequalities, eps = 1e-9 ft in metres.
    #[test]
    fn v02_the_plan_condition_follows_the_classifier_inequalities() {
        let p = default_floor_planes();
        let eps = 1e-9 * asterion_core::units::METRES_PER_FOOT;
        // (c) in the cut plane: a door 0..2.1 and a window 1.0..2.0 are cut.
        assert_eq!(classify_plan_condition(0.0, 2.1, &p), PlanCondition::Cut);
        assert_eq!(classify_plan_condition(1.0, 2.0, &p), PlanCondition::Cut);
        // (d) below the cut and above the bottom clip: condition 3.
        assert_eq!(classify_plan_condition(0.2, 0.7, &p), PlanCondition::BelowCut);
        // (f) above the cut and starting below the top clip: condition 5.
        assert_eq!(classify_plan_condition(1.5, 2.0, &p), PlanCondition::AboveCut);
        // (a) the gate: an element that starts above the top clip is not drawn (condition 1).
        assert_eq!(classify_plan_condition(2.4, 3.0, &p), PlanCondition::NotDrawn);
        // The cut inequality is `zmin < C - eps` and `C - eps <= zmax`: an element whose top is
        // exactly AT the cut plane is cut; one that starts exactly at it is not.
        assert_eq!(classify_plan_condition(0.0, 1.2, &p), PlanCondition::Cut);
        assert_eq!(classify_plan_condition(1.2, 2.0, &p), PlanCondition::AboveCut);
        // The eps slack: zmax = C - eps/2 is still cut; C - 2 eps is below the cut.
        assert_eq!(classify_plan_condition(0.0, 1.2 - eps / 2.0, &p), PlanCondition::Cut);
        assert_eq!(classify_plan_condition(0.0, 1.2 - 2.0 * eps, &p), PlanCondition::BelowCut);
        assert_eq!(classify_plan_condition(0.0, 2.1, &p).reference_code(), Some(4));
        assert_eq!(classify_plan_condition(1.5, 2.0, &p).reference_code(), Some(5));
        assert!(is_in_cut_plane(0.0, 1.2, 1.2) && !is_in_cut_plane(1.2, 2.0, 1.2));
    }

    /// Spec V-02: the mode of the element category is not specified, so a result that depends on it is Unknown; and a
    /// decision that needs the band-overlap test is Unknown too.
    #[test]
    fn v02_what_depends_on_unread_parts_is_unknown() {
        let p = default_floor_planes();
        // A slab 0.2 thick hanging from the level: mode 0 reads it as condition 3, mode 2 as not drawn (view depth equals
        // bottom clip): the category decides, and it is not specified.
        assert!(matches!(classify_plan_condition(-0.2, 0.0, &p), PlanCondition::Unknown { .. }));
        // View depth below the bottom clip: the element deep in the band needs the overlap test.
        let deep = PlanClassifierPlanes { depth: Some(-1.2), ..p };
        assert!(matches!(classify_plan_condition(-1.0, -0.5, &deep), PlanCondition::Unknown { .. }));
        // An underlay that the element does not reach in z is a decided "not drawn"; one it may
        // overlap is not.
        let with_underlay = PlanClassifierPlanes { underlay: Some((-3.0, -2.0)), ..p };
        assert_eq!(classify_plan_condition(3.0, 4.0, &with_underlay), PlanCondition::NotDrawn);
        assert!(matches!(classify_plan_condition(-9.0, -2.5, &with_underlay), PlanCondition::Unknown { .. }));
    }

    /// Spec V-02, ceiling-plan branch: (d) condition 5 between the cut plane and the top clip, (f) condition 2 beyond the
    /// top clip up to the view depth (the level above).
    #[test]
    fn v02_ceiling_plan_branches() {
        let p = PlanClassifierPlanes {
            ceiling: true,
            cut: Some(1.2),
            top: Some(2.3),
            bottom: Some(0.0),
            depth: Some(3.0),
            underlay: None,
        };
        assert_eq!(classify_plan_condition(1.5, 2.0, &p), PlanCondition::AboveCut);
        assert_eq!(classify_plan_condition(0.0, 2.1, &p), PlanCondition::Cut);
        assert_eq!(classify_plan_condition(2.4, 2.9, &p), PlanCondition::BelowBottomClip);
        // below the cut plane the gate (`zmax >= C - eps`) refuses: not drawn.
        assert_eq!(classify_plan_condition(0.0, 1.0, &p), PlanCondition::NotDrawn);
    }

    /// A door instance against the default plan range (cut 1.2 m above its level).
    #[test]
    fn a_hosted_instance_is_classified_against_the_view_cut_plane() {
        use crate::model::HostedKind;
        let mut fx = fixture();
        let out = fx.doc.transact("inst", false, |m, cx| {
            let door = ops::create_hosted_type(m, cx, "D", HostedKind::Door, 0.9, 2.1, 0.2, 0.9, 2.1, 0.0, 0.0)?;
            let win = ops::create_hosted_type(m, cx, "W", HostedKind::Window, 0.9, 0.5, 0.2, 0.9, 0.5, 0.0, 0.0)?;
            let a = ops::place_hosted_instance(
                m,
                cx,
                ops::NewHostedInstance { type_id: door, host: fx.wall, level: fx.l0, host_station: 1.0, sill_height: Some(0.0) },
            )?;
            let b = ops::place_hosted_instance(
                m,
                cx,
                ops::NewHostedInstance { type_id: win, host: fx.wall, level: fx.l0, host_station: 3.0, sill_height: Some(0.2) },
            )?;
            let c = ops::place_hosted_instance(
                m,
                cx,
                ops::NewHostedInstance { type_id: win, host: fx.wall, level: fx.l0, host_station: 4.0, sill_height: Some(1.5) },
            )?;
            Ok((a, b, c))
        });
        let (door, low_window, high_window) = out.value.expect("created");
        let m = fx.doc.model();
        assert_eq!(hosted_plan_condition(m, door, fx.plan).unwrap(), PlanCondition::Cut);
        assert_eq!(hosted_plan_condition(m, low_window, fx.plan).unwrap(), PlanCondition::BelowCut);
        // Spec V-02: above the cut plane and starting below the top clip is condition 5.
        assert_eq!(hosted_plan_condition(m, high_window, fx.plan).unwrap(), PlanCondition::AboveCut);
    }
}

#[cfg(test)]
mod grid_tests {
    use super::*;
    use crate::model::{DatumEnd, Grid};
    use crate::ops;

    #[test]
    fn the_name_sequence_continues_from_the_last_grid() {
        // DOCUMENTED: the first grid of a project is "1" and each new grid continues the sequence.
        assert_eq!(Grid::next_name(None), "1");
        assert_eq!(Grid::next_name(Some("1")), "2");
        assert_eq!(Grid::next_name(Some("9")), "10");
        assert_eq!(Grid::next_name(Some("09")), "10");
        // Increment rules (module `item_number`): a run of one repeated letter steps, "AZ" cannot be incremented
        assert_eq!(Grid::next_name(Some("A")), "B");
        assert_eq!(Grid::next_name(Some("Z")), "AA");
        assert_eq!(Grid::next_name(Some("AA")), "BB");
        assert_eq!(Grid::next_name(Some("AZ")), "1");
        assert_eq!(Grid::next_name(Some("a")), "b");
        assert_eq!(Grid::next_name(Some("A1")), "A2");
        assert_eq!(Grid::next_name(Some("GRID-7")), "GRID-8");
    }

    #[test]
    fn grids_are_auto_named_unique_and_deletable() {
        let mut doc = crate::new_document();
        let out = doc.transact("grids", false, |m, cx| {
            let a = ops::create_grid(m, cx, None, Vec2::new(0.0, 0.0), Vec2::new(0.0, 10.0))?;
            let b = ops::create_grid(m, cx, None, Vec2::new(5.0, 0.0), Vec2::new(5.0, 10.0))?;
            Ok((a, b))
        });
        assert!(out.committed());
        let (a, b) = out.value.unwrap();
        assert_eq!(doc.model().grid(a).unwrap().name, "1");
        assert_eq!(doc.model().grid(b).unwrap().name, "2");
        // a duplicate name is rejected, on creation and on rename
        let out = doc.transact("dup", false, |m, cx| ops::create_grid(m, cx, Some("2"), Vec2::new(9.0, 0.0), Vec2::new(9.0, 9.0)));
        assert!(out.value.is_none());
        let out = doc.transact("ren", false, |m, cx| ops::rename_grid(m, cx, a, "2"));
        assert!(out.value.is_none());
        let out = doc.transact("rename grid", false, |m, cx| ops::rename_grid(m, cx, a, "A"));
        assert!(out.committed());
        // OBSERVED: the cursor is the last name ASSIGNED, so after renaming "1" to "A" the next automatic name is "B" - not
        // "3". ASTERION predicted "3" and was falsified.
        let out = doc.transact("next", false, |m, cx| ops::create_grid(m, cx, None, Vec2::new(7.0, 0.0), Vec2::new(7.0, 9.0)));
        assert!(out.committed());
        assert_eq!(doc.model().grid(out.value.unwrap()).unwrap().name, "B");
        // and the one after that is "C", which is what the measurement showed next.
        let out = doc.transact("next2", false, |m, cx| ops::create_grid(m, cx, None, Vec2::new(6.0, 0.0), Vec2::new(6.0, 9.0)));
        assert!(out.committed());
        assert_eq!(doc.model().grid(out.value.unwrap()).unwrap().name, "C");
        let out = doc.transact("del", false, |m, cx| ops::delete(m, cx, b));
        assert!(out.committed());
        assert!(doc.model().grids.get(&b).is_none());
    }

    #[test]
    fn a_grid_shows_where_its_vertical_extent_meets_the_primary_range() {
        let mut doc = crate::new_document();
        let out = doc.transact("fx", false, |m, cx| {
            let l0 = ops::create_level(m, cx, "L0", 0.0)?;
            let g = ops::create_grid(m, cx, None, Vec2::new(-1.0, 0.0), Vec2::new(9.0, 0.0))?;
            let v = ops::create_view(m, cx, "Plan", ViewKind::FloorPlan, Some(l0), None)?;
            Ok((g, v))
        });
        assert!(out.committed());
        let (g, v) = out.value.unwrap();
        let st = {
            let m = doc.model();
            view_state(m, m.view(v).unwrap()).unwrap()
        };
        assert_eq!(st.grids.len(), 1);
        assert!(st.grids[0].visible, "a new grid is unbounded, so it shows in every plan");
        // OBSERVED (five grids): a new grid bubbles at End1 only.
        assert_eq!(st.grids[0].bubble, [false, true]);
        // OBSERVED: visibility is decided by the PRIMARY RANGE, not by the cut plane. An extent of [-0.5, 0.5] stays
        // visible in a plan whose range is [0, 2.3] even though the cut plane (1.2) is outside that extent. ASTERION tested
        // the cut plane and was falsified.
        let out = doc.transact("extent", false, |m, cx| ops::set_grid_vertical_extent(m, cx, g, -0.5, 0.5));
        assert!(out.committed());
        let st = {
            let m = doc.model();
            view_state(m, m.view(v).unwrap()).unwrap()
        };
        assert!(st.grids[0].visible, "the extent still meets the primary range [0, 2.3]");
        // Raise the whole range above the extent and it does drop out, which is the other half of
        // the same measurement (a plan whose range is [4.0, 6.3]).
        // The planes move top-down so the range never passes through an out-of-order state,
        // which the engine rejects before applying (round 7 confirmed that channel).
        let out = doc.transact("raise", false, |m, cx| {
            ops::set_view_range_plane(m, cx, v, PlanViewPlane::TopClip, RangePlane::at(6.3))?;
            ops::set_view_range_plane(m, cx, v, PlanViewPlane::Cut, RangePlane::at(5.2))?;
            ops::set_view_range_plane(m, cx, v, PlanViewPlane::BottomClip, RangePlane::at(4.0))
        });
        assert!(out.committed());
        let st = {
            let m = doc.model();
            view_state(m, m.view(v).unwrap()).unwrap()
        };
        assert!(!st.grids[0].visible, "the extent no longer meets the primary range [4.0, 6.3]");
        // an inverted extent is rejected
        let out = doc.transact("bad", false, |m, cx| ops::set_grid_vertical_extent(m, cx, g, 2.0, 1.0));
        assert!(out.value.is_none());
        // OBSERVED: bubble visibility is per END and per VIEW. The default is End1 only, and a view that was never touched
        // keeps that default while another view diverges - which is exactly what round 8 measured on two plans of the same
        // grid.
        let v2 = doc
            .transact("v2", false, |m, cx| {
                let l1 = ops::create_level(m, cx, "L1", 4.0)?;
                ops::create_view(m, cx, "Plan2", ViewKind::FloorPlan, Some(l1), None)
            })
            .value
            .unwrap();
        assert_eq!(doc.model().grid(g).unwrap().bubble_in(v), [false, true], "the default is End1 only");
        let out = doc.transact("bub", false, |m, cx| ops::set_grid_bubble(m, cx, g, v, DatumEnd::End0, true));
        assert!(out.committed());
        assert_eq!(doc.model().grid(g).unwrap().bubble_in(v), [true, true]);
        assert_eq!(
            doc.model().grid(g).unwrap().bubble_in(v2),
            [false, true],
            "the other view is untouched - this is what made the per-grid model wrong"
        );
        // Hiding a bubble in a view that does not exist is rejected, not silently stored.
        let out = doc.transact("nov", false, |m, cx| {
            ops::set_grid_bubble(m, cx, g, asterion_core::ElementId(999_999), DatumEnd::End0, false)
        });
        assert!(out.value.is_none());
    }


    /// Spec V-03: in a FLOOR plan the datum's vertical range is [bottom clip, cut plane] - the top clip is not specified -
    /// and in a CEILING plan [cut plane, top clip]; the test is `max(lo) <= min(hi) + 1e-9` (feet).
    #[test]
    fn v03_a_plan_reads_only_the_cut_plane_and_one_clip() {
        let mut doc = crate::new_document();
        let out = doc.transact("fx", false, |m, cx| {
            let l0 = ops::create_level(m, cx, "L0", 0.0)?;
            let g = ops::create_grid(m, cx, None, Vec2::new(-1.0, 0.0), Vec2::new(9.0, 0.0))?;
            let floor = ops::create_view(m, cx, "Floor", ViewKind::FloorPlan, Some(l0), None)?;
            let ceiling = ops::create_view(m, cx, "Ceiling", ViewKind::CeilingPlan, Some(l0), None)?;
            Ok((g, floor, ceiling))
        });
        let (g, floor, ceiling) = out.value.unwrap();
        let visible = |doc: &crate::BimDocument, v: ElementId| {
            let m = doc.model();
            view_state(m, m.view(v).unwrap()).unwrap().grids[0].visible
        };
        let extent = |doc: &mut crate::BimDocument, lo: f64, hi: f64| {
            assert!(doc.transact("extent", false, |m, cx| ops::set_grid_vertical_extent(m, cx, g, lo, hi)).committed());
        };
        // Default range: bottom 0, cut 1.2, top 2.3. An extent between the cut plane and the top clip
        // is outside a floor plan's [0, 1.2] and inside a ceiling plan's [1.2, 2.3].
        extent(&mut doc, 1.5, 2.0);
        assert!(!visible(&doc, floor), "the top clip is not specified for a floor plan");
        assert!(visible(&doc, ceiling));
        // An extent below the level is the other way round.
        extent(&mut doc, -0.5, 0.5);
        assert!(visible(&doc, floor));
        assert!(!visible(&doc, ceiling), "the bottom clip is not specified for a ceiling plan");
        // The tolerance: touching the bottom clip within 1e-9 ft is visible, 2e-9 ft clear is not.
        let eps = 1e-9 * asterion_core::units::METRES_PER_FOOT;
        extent(&mut doc, -1.0, -0.5 * eps);
        assert!(visible(&doc, floor));
        extent(&mut doc, -1.0, -2.0 * eps);
        assert!(!visible(&doc, floor));
    }

    /// Spec G-01: a name equal to the stored one does nothing, the cursor included; any other name overwrites the cursor,
    /// whatever its value.
    #[test]
    fn g01_a_rename_to_the_same_name_leaves_the_cursor_and_any_other_overwrites_it() {
        let mut doc = crate::new_document();
        let out = doc.transact("g", false, |m, cx| {
            let a = ops::create_grid(m, cx, None, Vec2::new(0.0, 0.0), Vec2::new(0.0, 5.0))?;
            ops::create_grid(m, cx, None, Vec2::new(1.0, 0.0), Vec2::new(1.0, 5.0))?;
            ops::create_grid(m, cx, None, Vec2::new(2.0, 0.0), Vec2::new(2.0, 5.0))?;
            Ok(a)
        });
        let a = out.value.unwrap();
        assert_eq!(doc.model().last_grid_name.as_deref(), Some("3"));
        // Same name: nothing happens, the cursor stays at "3".
        assert!(doc.transact("same", false, |m, cx| ops::rename_grid(m, cx, a, "1")).committed());
        assert_eq!(doc.model().last_grid_name.as_deref(), Some("3"));
        // A rename to a LOWER number moves the cursor down: it is overwritten, not a maximum.
        assert!(doc.transact("low", false, |m, cx| ops::rename_grid(m, cx, a, "0")).committed());
        assert_eq!(doc.model().last_grid_name.as_deref(), Some("0"));
    }

    /// Spec G-02: a new grid's bubbles come from the type: end 1 false, end 2 true; a per-view override survives only where
    /// it differs from that default and does not leak to other views.
    #[test]
    fn g02_the_type_default_is_end_one_hidden_end_two_shown() {
        assert_eq!(Grid::DEFAULT_BUBBLE, [false, true]);
        let mut doc = crate::new_document();
        let out = doc.transact("g", false, |m, cx| {
            let l0 = ops::create_level(m, cx, "L0", 0.0)?;
            let l1 = ops::create_level(m, cx, "L1", 4.0)?;
            let g = ops::create_grid(m, cx, None, Vec2::new(0.0, 0.0), Vec2::new(0.0, 5.0))?;
            let v0 = ops::create_view(m, cx, "P0", ViewKind::FloorPlan, Some(l0), None)?;
            let v1 = ops::create_view(m, cx, "P1", ViewKind::FloorPlan, Some(l1), None)?;
            Ok((g, v0, v1))
        });
        let (g, v0, v1) = out.value.unwrap();
        // Showing end 2 (already shown) and hiding end 1 (already hidden) change nothing observable.
        assert!(doc
            .transact("noop", false, |m, cx| {
                ops::set_grid_bubble(m, cx, g, v0, DatumEnd::End1, true)?;
                ops::set_grid_bubble(m, cx, g, v0, DatumEnd::End0, false)
            })
            .committed());
        assert_eq!(doc.model().grid(g).unwrap().bubble_in(v0), [false, true]);
        // Hiding end 2 in one view leaves the other view on the type default.
        assert!(doc.transact("hide", false, |m, cx| ops::set_grid_bubble(m, cx, g, v0, DatumEnd::End1, false)).committed());
        assert_eq!(doc.model().grid(g).unwrap().bubble_in(v0), [false, false]);
        assert_eq!(doc.model().grid(g).unwrap().bubble_in(v1), [false, true]);
    }

    /// Spec V-02: an element whose top is exactly at the cut plane IS in the cut plane (`C - eps <= zmax`), which the plain
    /// open-interval test got wrong.
    #[test]
    fn v02_an_element_ending_at_the_cut_plane_is_cut() {
        let mut doc = crate::new_document();
        let out = doc.transact("fx", false, |m, cx| {
            let l0 = ops::create_level(m, cx, "L0", 0.0)?;
            let wt = ops::create_wall_type(
                m,
                cx,
                "W",
                crate::CompoundStructure::new(vec![crate::Layer { width: 0.2, function: crate::LayerFunction::Structure }], 0, 0)?,
            )?;
            let w = ops::create_wall(
                m,
                cx,
                ops::NewWall {
                    wall_type: wt,
                    level: l0,
                    start: Vec2::new(0.0, 0.0),
                    end: Vec2::new(5.0, 0.0),
                    height: 1.2,
                    base_offset: 0.0,
                    flip: false,
                    structural: false,
                },
            )?;
            let v = ops::create_view(m, cx, "P", ViewKind::FloorPlan, Some(l0), None)?;
            Ok((w, v))
        });
        let (w, v) = out.value.unwrap();
        let m = doc.model();
        let st = view_state(m, m.view(v).unwrap()).unwrap();
        let e = st.elements.iter().find(|e| e.id == w).unwrap();
        assert_eq!(e.appearance, Appearance::Cut, "wall 0..1.2 against a cut plane at 1.2");
    }
}
