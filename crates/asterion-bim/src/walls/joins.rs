//! Automatic wall joins (butt joins at shared corners and T-junctions).
//!
//! All rules below are OBSERVED unless marked otherwise. Joins change only the wall **solid** (and therefore the computed
//! Area/Volume); the location curve and the Length parameter stay at the original centreline (observed: L_param unchanged
//! while Area grew).
//!
//! Resolution is a pure function of the model: for each wall end, [`end_extension`] returns a
//! signed outward extension (metres) along the wall axis â€” positive extends the solid past the
//! location endpoint, negative trims it back.
//!
//! Rules (two walls meeting; `h_q` = other wall's half width; Î³ = angle between the two wall body
//! directions at the shared corner, in (0, Ï€)):
//! * **Corner butt join** (two wall *ends* share a point, non-collinear, joins allowed at both): the wall that is first in
//!   the join list ([`join_list_precedes`]; the engine's list order is ascending `ElementId`, UNKNOWN as a product rule) is
//!   the *through* wall and extends by `h_q Â· tan(Î³/2)`; the later wall *butts* and trims by `h_q Â· cot(Î³/2)`.
//! * **T-junction** (this wall's end lies on another wall's interior): this wall is always the
//!   butt and trims by `h_host / sin Î¸` (Î¸ = angle between the two axes); the host is unchanged.
//! * **Collinear** (Î³ â‰ˆ Ï€): no change.
//! * **Join disallowed at the end** (`Wall::join_allowed[end] == false`): no change.
//!
//! Limitations (documented, not yet reproduced): more than two walls at one corner (only the partner with the largest
//! extension is used, UNKNOWN); the exact mitred corner *shape* (face/edge/vertex counts) â€” this model extends/trims
//! axially, reproducing Area/Volume/length/bbox-extent but not the notched corner solids the reference product builds.
//!
//! Spec W-07c
//!
//! The formulas above are fitted to harness measurements of the DEFAULT join (type 0, butt) at 90,
//! 45 and 135 degrees, and they reproduce every recorded round. The native rules are written in
//! [`crate::join_end`] (join type map, angle flags, target choice, third-and-later walls, layer
//! routing priority). Where they bear on this module:
//!
//! * The collinear test is `|sin| <= 0.1 degrees`, implemented by [`is_collinear`] (the same constant the engine already
//!   used).
//! * Two straight walls whose axes are within 20 degrees of straight (or of folded back) are forced to join type 1 (miter)
//!   and the support check is skipped. DIVERGENCE kept on purpose: this module still applies the butt formula in that band,
//!   where `tan(gamma/2)` grows without bound (11 half widths at 170 degrees).
//! * The join type is read from the FIRST wall of the join for both walls; the engine has no join-type state (default type
//!   0 only). Which wall is first is [`join_list_precedes`].
//! * So the engine's "the lower element id is first" is NOT a product rule: it is the engine's own list order, kept because
//!   it is the order the recorded rounds were produced in (a check, not a source). The rules that DO operate on a given
//!   array (span reductions, neighbour selection, join-order predicate) are written in [`crate::join_order`] as pure
//!   functions over an explicit array; they are not wired here because the array order is the unknown.
//! * A stacked wall never receives a join step. The engine models no stacked wall, so every wall of the engine is in the
//!   "returns false, receives a step" class.
//! * The wall Area is the LARGER of the two side faces; see [`side_face_spread`].
//! * Three or more walls at one end use the reference product's neighbour ordering and the offset loop; the engine picks
//!   the partner with the largest extension. UNKNOWN (the branch structure after the neighbour selection; the array order
//!   that needs to know which walls precede the wall is UNKNOWN).
//! * The per-end disallow flag is one boolean per end on the wall itself (`Wall::join_allowed`), the geometry of a wall at
//!   an end depends only on ITS OWN flag (the observation quoted in [`end_extension`]).
//! * The end is re-trimmed against the CURRENT neighbour geometry at every regeneration and the control point never moves -
//!   which is what a pure function of the model does by construction.

use crate::model::{BimModel, Wall};
use asterion_geom::{Tolerances, Vec2};
use asterion_core::{ElementId, IndexCache};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// A uniform grid over the walls' bounding boxes, so that the walls near a point or inside a box are found without
/// scanning every wall (measured: one wall's regeneration scanned all of them, 255 ms at 100 000 walls).
///
/// The grid is DERIVED from `model.walls` and never stored in the model: it lives in a per-thread
/// [`IndexCache`] and is brought up to date, before every query, with only the chunks of `model.walls` that changed
/// since it last looked (see `asterion_core::index`). An edit of one wall therefore re-indexes one chunk; undo and
/// redo move it to the older or newer state the same way; a stale answer is impossible by construction. Every
/// query returns candidates in ascending element id (the order `model.walls` iterates in), so a loop over the
/// candidates behaves exactly like the scan of every wall it replaces - the candidates are a SUPERSET of the walls
/// that can pass the geometric tests inside the loop.
#[derive(Clone)]
pub(crate) struct WallGrid {
    cells: HashMap<(i64, i64), Vec<ElementId>>,
    /// Walls whose box covers too many cells, or has no finite box (a huge arc, a NaN coordinate): every query
    /// returns them, so a degenerate wall can never be missed.
    wide: Vec<ElementId>,
}

const CELL: f64 = 2.0;
const MAX_CELLS_PER_WALL: i64 = 4096;

enum Reach {
    Cells(i64, i64, i64, i64),
    Wide,
}

/// The cells a wall must be listed in: its box padded by the tolerance, grown to the whole circle for an arc
/// (a conservative superset - the arc bulges outside the chord's box).
fn reach(w: &Wall) -> Reach {
    let pad = Tolerances::default().vertex * 2.0 + 1e-9;
    let (mut x0, mut x1) = (w.start.x.min(w.end.x), w.start.x.max(w.end.x));
    let (mut y0, mut y1) = (w.start.y.min(w.end.y), w.start.y.max(w.end.y));
    if let Some(t) = w.through {
        if let Some(a) = crate::regen::arc_from_points(w.start, w.end, t) {
            x0 = x0.min(a.center.x - a.radius);
            x1 = x1.max(a.center.x + a.radius);
            y0 = y0.min(a.center.y - a.radius);
            y1 = y1.max(a.center.y + a.radius);
        }
    }
    let (x0, x1, y0, y1) = (x0 - pad, x1 + pad, y0 - pad, y1 + pad);
    if !(x0.is_finite() && x1.is_finite() && y0.is_finite() && y1.is_finite()) {
        return Reach::Wide;
    }
    let (cx0, cx1) = ((x0 / CELL).floor(), (x1 / CELL).floor());
    let (cy0, cy1) = ((y0 / CELL).floor(), (y1 / CELL).floor());
    if (cx1 - cx0 + 1.0) * (cy1 - cy0 + 1.0) > MAX_CELLS_PER_WALL as f64 {
        return Reach::Wide;
    }
    Reach::Cells(cx0 as i64, cx1 as i64, cy0 as i64, cy1 as i64)
}

fn grid_add(g: &mut WallGrid, w: &Wall) {
    match reach(w) {
        Reach::Cells(cx0, cx1, cy0, cy1) => {
            for cx in cx0..=cx1 {
                for cy in cy0..=cy1 {
                    // each cell stays sorted by id, so a query on one cell needs no sorting
                    let v = g.cells.entry((cx, cy)).or_default();
                    if let Err(at) = v.binary_search(&w.id) {
                        v.insert(at, w.id);
                    }
                }
            }
        }
        Reach::Wide => g.wide.push(w.id),
    }
}

/// `w` is the wall exactly as it was when `grid_add` listed it.
fn grid_remove(g: &mut WallGrid, w: &Wall) {
    match reach(w) {
        Reach::Cells(cx0, cx1, cy0, cy1) => {
            for cx in cx0..=cx1 {
                for cy in cy0..=cy1 {
                    if let Some(v) = g.cells.get_mut(&(cx, cy)) {
                        if let Ok(at) = v.binary_search(&w.id) {
                            v.remove(at);
                        }
                        if v.is_empty() {
                            g.cells.remove(&(cx, cy));
                        }
                    }
                }
            }
        }
        Reach::Wide => g.wide.retain(|i| *i != w.id),
    }
}

thread_local! {
    // the grid sits behind an Arc so a batch can hand the same immutable grid to its worker threads (`shared_wall_grid`); an
    // update takes the Arc back with `make_mut`, which copies nothing while no worker still holds it
    static WALL_GRIDS: RefCell<IndexCache<Wall, Arc<WallGrid>>> = const { RefCell::new(IndexCache::new()) };
    // the grid a worker thread of a batch queries instead of building its own (valid for one version of `model.walls`)
    static FROZEN_GRID: RefCell<Option<(u64, Arc<WallGrid>)>> = const { RefCell::new(None) };
}

/// An immutable snapshot of the wall grid for one version of `model.walls`, to share with the worker threads of a batch of
/// regenerations: each calls [`SharedWallGrid::install`] and its queries use this grid, so no worker rebuilds the index.
pub struct SharedWallGrid {
    version: u64,
    grid: Arc<WallGrid>,
}

impl SharedWallGrid {
    /// Makes this thread's wall queries on the model it was taken from use the shared grid (dropped with the thread).
    pub fn install(&self) {
        FROZEN_GRID.with(|f| *f.borrow_mut() = Some((self.version, self.grid.clone())));
    }
}

pub fn shared_wall_grid(model: &BimModel) -> SharedWallGrid {
    SharedWallGrid { version: model.walls.version(), grid: with_grid_arc(model) }
}

fn with_grid_arc(model: &BimModel) -> Arc<WallGrid> {
    WALL_GRIDS.with(|c| {
        c.borrow_mut().with(
            &model.walls,
            || Arc::new(WallGrid { cells: HashMap::new(), wide: Vec::new() }),
            |g, _, w| grid_add(Arc::make_mut(g), w),
            |g, _, w| grid_remove(Arc::make_mut(g), w),
            |g| g.clone(),
        )
    })
}

fn with_grid<R>(model: &BimModel, answer: impl FnOnce(&WallGrid) -> R) -> R {
    let version = model.walls.version();
    let mut answer = Some(answer);
    let frozen = FROZEN_GRID.with(|f| match f.borrow().as_ref() {
        Some((v, g)) if *v == version => Some((answer.take().expect("once"))(g)),
        _ => None,
    });
    if let Some(r) = frozen {
        return r;
    }
    let answer = answer.expect("not consumed");
    WALL_GRIDS.with(|c| {
        c.borrow_mut().with(
            &model.walls,
            || Arc::new(WallGrid { cells: HashMap::new(), wide: Vec::new() }),
            |g, _, w| grid_add(Arc::make_mut(g), w),
            |g, _, w| grid_remove(Arc::make_mut(g), w),
            |g| answer(g),
        )
    })
}

/// Drops this thread's wall grids (and the model chunks they keep shared). Call when a document is closed.
pub fn release_wall_grids() {
    WALL_GRIDS.with(|c| c.borrow_mut().clear());
}

/// Kept for the call sites that bracket a regeneration of many walls: the grid is now always available and
/// always current, so there is nothing to install.
pub fn with_wall_index<R>(_model: &BimModel, f: impl FnOnce() -> R) -> R {
    f()
}

/// The walls whose box (padded by the tolerance) can contain `p`, ascending by id.
pub(crate) fn walls_near(model: &BimModel, p: Vec2) -> Vec<&Wall> {
    walls_in_box(model, p, p)
}

/// The walls near `p` that coexist with `w` (and `w` itself).
pub(crate) fn walls_coexisting<'a>(model: &'a BimModel, w: &Wall, p: Vec2) -> Vec<&'a Wall> {
    walls_near(model, p).into_iter().filter(|q| q.id == w.id || coexists(model, w, q)).collect()
}

/// The walls whose box can meet the box `lo..=hi`, ascending by id. `lo`/`hi` are not padded here: a caller whose
/// test reaches further than the tolerance (a half width, a snap distance) widens the box itself.
pub(crate) fn walls_in_box(model: &BimModel, lo: Vec2, hi: Vec2) -> Vec<&Wall> {
    let span = ((hi.x / CELL).floor() - (lo.x / CELL).floor() + 1.0) * ((hi.y / CELL).floor() - (lo.y / CELL).floor() + 1.0);
    // a box so wide (or so degenerate) that walking its cells costs more than the scan it replaces
    if !span.is_finite() || span > 65_536.0 {
        return model.walls.values().collect();
    }
    let (cx0, cx1) = ((lo.x / CELL).floor() as i64, (hi.x / CELL).floor() as i64);
    let (cy0, cy1) = ((lo.y / CELL).floor() as i64, (hi.y / CELL).floor() as i64);
    if cx0 == cx1 && cy0 == cy1 {
        // one cell, the common case (a point query): its list is already ascending and has no duplicates
        return with_grid(model, |g| {
            let mut out: Vec<&Wall> = Vec::with_capacity(g.cells.get(&(cx0, cy0)).map_or(0, Vec::len) + g.wide.len());
            if g.wide.is_empty() {
                out.extend(g.cells.get(&(cx0, cy0)).into_iter().flatten().filter_map(|i| model.walls.get(i)));
            } else {
                let mut ids = g.wide.clone();
                ids.extend(g.cells.get(&(cx0, cy0)).into_iter().flatten().copied());
                ids.sort_unstable_by_key(|i| i.0);
                ids.dedup();
                out.extend(ids.iter().filter_map(|i| model.walls.get(i)));
            }
            out
        });
    }
    let mut ids: Vec<ElementId> = with_grid(model, |g| {
        let mut ids = g.wide.clone();
        for cx in cx0..=cx1 {
            for cy in cy0..=cy1 {
                if let Some(v) = g.cells.get(&(cx, cy)) {
                    ids.extend_from_slice(v);
                }
            }
        }
        ids
    });
    ids.sort_unstable_by_key(|i| i.0);
    ids.dedup();
    ids.into_iter().filter_map(|i| model.walls.get(&i)).collect()
}

/// The box of a wall's own geometry (chord, plus the arc's circle), for a query about that wall.
pub(crate) fn walls_around<'a>(model: &'a BimModel, w: &Wall, margin: f64) -> Vec<&'a Wall> {
    let (mut x0, mut x1) = (w.start.x.min(w.end.x), w.start.x.max(w.end.x));
    let (mut y0, mut y1) = (w.start.y.min(w.end.y), w.start.y.max(w.end.y));
    if let Some(t) = w.through {
        if let Some(a) = crate::regen::arc_from_points(w.start, w.end, t) {
            x0 = x0.min(a.center.x - a.radius);
            x1 = x1.max(a.center.x + a.radius);
            y0 = y0.min(a.center.y - a.radius);
            y1 = y1.max(a.center.y + a.radius);
        }
    }
    let m = margin + Tolerances::default().vertex * 2.0 + 1e-9;
    walls_in_box(model, Vec2::new(x0 - m, y0 - m), Vec2::new(x1 + m, y1 + m))
}

/// Whether `a` comes BEFORE `b` in the join list of the point where they meet (so `a` is the
/// FIRST wall of the pair and `b` the second). The first wall is the through wall of a corner.
///
/// The reference product compares no element id to choose entry 0. Every place that decides "first" goes through this
/// function so that reading the reference product changes one line.
pub fn join_list_precedes(a: &Wall, b: &Wall) -> bool {
    a.id.0 < b.id.0
}

/// Removes from the ordered join array every wall that does not coexist with the wall: such a wall is not trimmed against
/// the neighbour. A document with no phase data has every wall in the one default phase, which coexist.
pub(crate) fn coexists(model: &BimModel, a: &Wall, b: &Wall) -> bool {
    model.phases.elements.is_empty() || crate::phases::elems_coexist(model, a.id, b.id)
}

/// The walls' structural significance is not modelled: every wall counts as the same.
pub(crate) fn pair_in_join(model: &BimModel, w: &Wall, q: &Wall, tweaked: bool) -> bool {
    coexists(model, w, q) && (!tweaked || model.phases.elements.is_empty() || crate::phases::phases_equal(model, w.id, q.id))
}

/// The walls at the join of `end` of `w` (the reference product's `LocationCurve.ElementsAtJoin(end)`), in the engine's
/// join list order: ascending element id ([`join_list_precedes`]), `w` itself included.
///
/// A wall is at the join when one of its ends is at the point (a corner, a collinear continuation) or the point lies on its
/// interior (a T host). An end whose join is disallowed lists nothing. whether the wall lists itself and what a disallowed
/// end answers are measured by round 33 / `-099`, section `wall_ends`.
pub fn elements_at_join(model: &BimModel, w: &Wall, end: usize) -> Vec<ElementId> {
    if end > 1 || !w.join_allowed[end] {
        return Vec::new();
    }
    let tol = Tolerances::default().vertex;
    let p = if end == 0 { w.start } else { w.end };
    // the walls with an end at p (and which of their ends), and the walls that have p on their interior (hosts of a T)
    let near = walls_coexisting(model, w, p);
    let ender_end = |q: &Wall| -> Option<usize> { (0..2).find(|e| ((if *e == 0 { q.start } else { q.end }) - p).length() <= tol) };
    let hosts: Vec<&Wall> = near.iter().copied().filter(|q| q.id != w.id && on_path_interior(q, p, tol)).collect();
    let mut out: Vec<ElementId>;
    if !hosts.is_empty() {
        // (2b, 3): a stem meeting the interior of a host is joined as [host, stem]; the join info read is the one of THAT
        // side of the host, so stems on the SAME side share one array in the order of attachment and a stem on the OPPOSITE
        // side is in another (OBSERVED: t2 lists [t1, t2], t3 lists [t1, t3]). The engine's order of attachment is the
        // order of creation (ascending id), the hosts first.
        let side = |q: &Wall, qe: usize| -> f64 {
            let dir = end_dir(q, qe).unwrap_or(Vec2::new(0.0, 0.0));
            match crate::curved_joins::Path::of(hosts[0]) {
                Some(path) => crate::curved_ends::side_of(&path, p + dir * 0.1),
                None => 0.0,
            }
        };
        let side_w = side(w, end);
        let mut stems: Vec<ElementId> = near
            .iter()
            .copied()
            .filter(|q| q.id != w.id && !hosts.iter().any(|h| h.id == q.id))
            .filter_map(|q| ender_end(q).map(|qe| (q, qe)))
            .filter(|(q, qe)| side(q, *qe) * side_w > 0.0)
            .map(|(q, _)| q.id)
            .collect();
        stems.push(w.id);
        stems.sort_by_key(|i| i.0);
        out = hosts.iter().map(|h| h.id).collect();
        out.sort_by_key(|i| i.0);
        out.extend(stems);
    } else {
        out = near.iter().copied().filter(|q| q.id == w.id || ender_end(q).is_some()).map(|q| q.id).collect();
        out.sort_by_key(|i| i.0);
        // OBSERVED (r: r2 has its end disallowed and none of r1 / r3 / r4 lists it): with three walls or more at a point
        // the walls whose end does not allow the join are not listed. ENGINE CHOICE.
        if out.len() >= 3 {
            out.retain(|id| *id == w.id || model.walls.get(id).map_or(true, |q| ender_end(q).map_or(true, |e| q.join_allowed[e])));
        }
    }
    // OBSERVED (i: `i1` lists [i2, i1] with the end of `i2` disallowed; round 33 f: `f2` lists [f1, f2] with the end of
    // `f1` disallowed): in a two-wall join the wall whose end does not allow the join is listed first.
    if out.len() == 2 && hosts.is_empty() {
        let other = if out[0] == w.id { out[1] } else { out[0] };
        if let Some(q) = model.walls.get(&other) {
            let q_end_disallowed = ender_end(q).map_or(false, |e| !q.join_allowed[e]);
            if q_end_disallowed && out[0] == w.id {
                out.swap(0, 1);
            }
        }
    }
    // OBSERVED (section `wall_ends`: 934 free ends in round 33 and every free end of round 32 answer an EMPTY list; no
    // measured end lists only its own wall): an end with no other wall at its point lists nothing.
    if out.len() == 1 && out[0] == w.id {
        out.clear();
    }
    out
}

/// The point where the lines of two walls meet (their drive curves, straight walls only), `None` when they are parallel.
fn drive_intersection(a: &Wall, b: &Wall) -> Option<Vec2> {
    if a.through.is_some() || b.through.is_some() {
        return None;
    }
    let (da, db) = ((a.end - a.start).normalized()?, (b.end - b.start).normalized()?);
    let den = da.cross(db);
    if den.abs() < 1e-9 {
        return None;
    }
    let t = (b.start - a.start).cross(db) / den;
    Some(a.start + da * t)
}

/// The ends of a wall that are DETERMINED BY NEIGHBOURS follow the drive curve of the neighbour as soon as that curve moves
/// (a flip moves it by twice the offset of the location line). The new end is the intersection of this wall's drive curve
/// with the neighbour's drive curve; for two walls of equal join rank BOTH ends of an L corner are determined by neighbours
/// and follow each other; the stem of a T follows its host (the host's mid span is no end), and the stems that end on a
/// moved host follow it. Straight walls with the join allowed at both ends; `old` are the end points `wall` had before it
/// moved. ENGINE CHOICE for several neighbours at one end (the first one decides).
pub(crate) fn follow_neighbours(model: &mut BimModel, wall: ElementId, old: (Vec2, Vec2)) {
    let tol = Tolerances::default().vertex;
    let Some(w0) = model.walls.get(&wall).cloned() else { return };
    if w0.through.is_some() {
        return;
    }
    let mut updates: Vec<(ElementId, usize, Vec2)> = Vec::new();
    for (k, p) in [(0usize, old.0), (1usize, old.1)] {
        if !w0.join_allowed[k] {
            continue;
        }
        let partner_ends: Vec<(ElementId, usize)> = walls_near(model, p)
            .into_iter()
            .filter(|q| q.id != wall && q.through.is_none())
            .filter_map(|q| (0..2).find(|e| ((if *e == 0 { q.start } else { q.end }) - p).length() <= tol && q.join_allowed[*e]).map(|e| (q.id, e)))
            .collect();
        let hosts: Vec<ElementId> = walls_near(model, p).into_iter().filter(|q| q.id != wall && q.through.is_none() && on_path_interior(q, p, tol)).map(|q| q.id).collect();
        if let Some((qid, qe)) = partner_ends.first().copied() {
            // an L corner: both ends follow each other
            if let Some(x) = model.walls.get(&qid).and_then(|q| drive_intersection(&w0, q)) {
                updates.push((wall, k, x));
                updates.push((qid, qe, x));
            }
        } else if let Some(hid) = hosts.first().copied() {
            // the stem follows its host
            if let Some(x) = model.walls.get(&hid).and_then(|h| drive_intersection(&w0, h)) {
                updates.push((wall, k, x));
            }
        }
    }
    // the stems that ended on the old line of `wall` follow it
    let (lo, hi) = (Vec2::new(old.0.x.min(old.1.x) - 1e-6, old.0.y.min(old.1.y) - 1e-6), Vec2::new(old.0.x.max(old.1.x) + 1e-6, old.0.y.max(old.1.y) + 1e-6));
    let stems: Vec<(ElementId, usize)> = walls_in_box(model, lo, hi)
        .into_iter()
        .filter(|q| q.id != wall && q.through.is_none())
        .flat_map(|q| {
            (0..2).filter_map(move |e| {
                let qp = if e == 0 { q.start } else { q.end };
                (q.join_allowed[e] && point_on_segment_interior(qp, old.0, old.1, tol)).then_some((q.id, e))
            })
        })
        .collect();
    for (sid, se) in stems {
        if let Some(x) = model.walls.get(&sid).and_then(|s| drive_intersection(s, &w0)) {
            updates.push((sid, se, x));
        }
    }
    for (id, e, x) in updates {
        if let Some(w) = model.walls.get_mut(&id) {
            if e == 0 {
                w.start = x;
            } else {
                w.end = x;
            }
        }
    }
}

/// How the corner at one end of a wall is built when exactly two walls meet there at their ends.
///
/// The join type is read from the FIRST wall of the join for both walls; two line walls whose axes are within 20 degrees of
/// straight or of folded back are mitred whatever type is stored; Spec W-05a: type 2 on the second wall is read as type 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CornerKind {
    /// Type 0: the engine's butt / through rule (fitted on round 5, see the module docs).
    Butt,
    /// With the same offset on both curves those intersections lie on the internal bisector of the corner, whose plan
    /// direction (unit, from the corner into the angle between the bodies) is `plane`.
    Miter { plane: Vec2 },
    /// Type 2 read for the first wall: Spec W-05a, a plane square to the wall through an intersection of the offset side
    /// curves, at outward offset `ext` from the location end.
    SquareOff { ext: f64 },
    /// Type 2 stored, this wall second: read as type 0 (Spec W-05a), so the body is the butt one; kept apart only for the
    /// edge bookkeeping.
    SquareOffSecond,
    /// A mitre at a corner with an ARC wall (stored, or forced by the curved): the vertical plane through two explicit
    /// corners, which do not lie on the bisector.
    MiterAt { corners: crate::curved_joins::MiterCorners },
    /// A butt (type 0) corner or a T with an ARC wall: the end is bounded by the side face of the OTHER wall, a plane for a
    /// line wall and a cylinder for an arc wall.
    Cut { corners: crate::curved_joins::MiterCorners, circle: Option<(Vec2, f64)>, via: Option<Vec2> },
}

/// The only other wall at the end `end` of `w`, when it meets `w` there at one of its own ENDS and no third wall touches
/// the point (Spec W-05a: the first two walls of a join). `(wall, its end)`.
fn sole_corner_partner<'a>(model: &'a BimModel, w: &Wall, end: usize) -> Option<(&'a Wall, usize)> {
    let tol = Tolerances::default().vertex;
    let p = if end == 0 { w.start } else { w.end };
    let list: Vec<&Wall> = walls_coexisting(model, w, p);
    let mut found: Option<(&Wall, usize)> = None;
    let mut count = 0usize;
    for q in list {
        if q.id == w.id || !coexists(model, w, q) {
            continue;
        }
        let at_end = if (q.start - p).length() <= tol {
            Some(0)
        } else if (q.end - p).length() <= tol {
            Some(1)
        } else {
            None
        };
        match at_end {
            Some(qend) => {
                count += 1;
                found = Some((q, qend));
            }
            None if on_path_interior(q, p, tol) => count += 1,
            None => {}
        }
    }
    if count == 1 {
        found
    } else {
        None
    }
}

/// The unit tangent of `w` at its `end` pointing INTO the body of the wall (the direction of the chord for a line).
pub(crate) fn end_dir(w: &Wall, end: usize) -> Option<Vec2> {
    match w.through {
        None => (if end == 0 { w.end - w.start } else { w.start - w.end }).normalized(),
        Some(_) => crate::curved_joins::Path::of(w)?.outward(end).map(|o| -o),
    }
}

/// `p` lies on the interior of the path of `q` (a line, or an arc not within the tolerance of its ends).
pub(crate) fn on_path_interior(q: &Wall, p: Vec2, tol: f64) -> bool {
    match q.through {
        None => point_on_segment_interior(p, q.start, q.end, tol),
        Some(_) => match crate::curved_joins::Path::of(q) {
            Some(crate::curved_joins::Path::Arc(arc)) => {
                if ((p - arc.center).length() - arc.radius).abs() > tol {
                    return false;
                }
                let a = (p.y - arc.center.y).atan2(p.x - arc.center.x);
                let dir = if arc.sweep >= 0.0 { 1.0 } else { -1.0 };
                let travelled = (dir * (a - arc.start_angle)).rem_euclid(std::f64::consts::TAU);
                let margin = tol / arc.radius;
                travelled > margin && travelled < arc.sweep.abs() - margin
            }
            _ => false,
        },
    }
}

/// The unit tangent of the path of `q` at the point `p` of it, in the direction of travel.
pub(crate) fn path_dir_at(q: &Wall, p: Vec2) -> Option<Vec2> {
    match q.through {
        None => (q.end - q.start).normalized(),
        Some(_) => match crate::curved_joins::Path::of(q)? {
            crate::curved_joins::Path::Arc(arc) => {
                let radial = (p - arc.center).normalized()?;
                Some(if arc.sweep >= 0.0 { radial.perp_ccw() } else { -radial.perp_ccw() })
            }
            crate::curved_joins::Path::Line { a, b } => (b - a).normalized(),
        },
    }
}

/// The only wall that has `p` (the point of the `end` of `w`) on its path INTERIOR when no wall has an end there: the host of a T.
fn sole_tee_host<'a>(model: &'a BimModel, w: &Wall, end: usize) -> Option<&'a Wall> {
    let tol = Tolerances::default().vertex;
    let p = if end == 0 { w.start } else { w.end };
    let mut found: Option<&Wall> = None;
    for q in walls_coexisting(model, w, p) {
        if q.id == w.id {
            continue;
        }
        if (q.start - p).length() <= tol || (q.end - p).length() <= tol {
            return None;
        }
        if on_path_interior(q, p, tol) {
            if found.is_some() {
                return None;
            }
            found = Some(q);
        }
    }
    found
}

/// Spec W-05a: the kind of corner at `end` of `w`. arc joins are a residual.
pub(crate) fn corner_kind(model: &BimModel, w: &Wall, end: usize) -> CornerKind {
    if end > 1 || !w.join_allowed[end] {
        return CornerKind::Butt;
    }
    // three or more walls at the point: the join array decides
    if crate::junctions::junction(model, w, end).is_some() {
        return match crate::junctions::cut(model, w, end) {
            Some(c) => CornerKind::Cut { corners: c.corners, circle: c.circle, via: c.via },
            None => CornerKind::Butt,
        };
    }
    let Some((q, qend)) = sole_corner_partner(model, w, end) else {
        // a T with an ARC (the stem or the host): the stem ends on the side face of the host
        if let Some(host) = sole_tee_host(model, w, end) {
            if w.through.is_some() || host.through.is_some() {
                let (hw, hh) = (half_width(model, w), half_width(model, host));
                if let Some(cut) = crate::curved_ends::tee_cut(w, end, hw, host, hh) {
                    return CornerKind::Cut { corners: cut.corners, circle: cut.circle, via: cut.via };
                }
            }
        }
        return CornerKind::Butt;
    };
    if w.through.is_some() || q.through.is_some() {
        // OBSERVED: a wall whose own end allows the join is built even when the OTHER wall's end does not (as for two
        // lines, each wall's geometry at an end depends on its own permission), and then it is the SECOND wall of the pair
        // (`curved_corner_kind`)
        return curved_corner_kind(model, w, end, q, qend);
    }
    if !q.join_allowed[qend] {
        return CornerKind::Butt;
    }
    let p = if end == 0 { w.start } else { w.end };
    let Some(dir_w) = (if end == 0 { w.end - w.start } else { w.start - w.end }).normalized() else {
        return CornerKind::Butt;
    };
    let Some(dir_q) = (if qend == 0 { q.end - q.start } else { q.start - q.end }).normalized() else {
        return CornerKind::Butt;
    };
    let (collinear, forced) = crate::join_end::corner_flags(dir_w, dir_q);
    if collinear {
        return CornerKind::Butt;
    }
    let w_first = join_list_precedes(w, q);
    let stored = if w_first { model.join_type_at(w.id, end) } else { model.join_type_at(q.id, qend) };
    use crate::model::WallJoinKind as K;
    if forced || stored == K::Miter {
        // the internal bisector: the side curves offset by the same distance meet on it
        let Some(plane) = (dir_w + dir_q).normalized() else { return CornerKind::Butt };
        return CornerKind::Miter { plane };
    }
    if stored == K::SquareOff {
        if !w_first {
            return CornerKind::SquareOffSecond;
        }
        // That point P is on a side face of this wall and the end plane through it is square to the wall path. f9 is "this
        // end is the start of the path", f10 "the other end is the finish of its path", f11 "the join value (the signed
        // sine of the two outward / inward tangents) is positive". No choice among candidate intersections exists.
        //
        // In this frame (`n_w`, `n_q` the left normals of the travel arriving at and leaving the corner) the offsets reduce
        // to the side of this wall at `+s h_w` and of the other at `-s h_q`, `s` the sign of the join value. ENGINE CHOICE
        // (not specified: the sign convention): a positive offset is taken to the RIGHT of the curve direction - the only
        // choice that reproduces OBSERVED (corners of 90, 60 and 120 degrees, equal widths: the plane through the farther
        // of the crossing pairs).
        let t_w = -dir_w; // travel arriving at the corner along w
        let n_w = t_w.perp_ccw();
        let t_q = dir_q; // travel leaving the corner along q
        let n_q = t_q.perp_ccw();
        let (h_w, h_q) = (half_width(model, w), half_width(model, q));
        let den = t_w.cross(t_q);
        if den.abs() <= 1e-12 {
            return CornerKind::Butt;
        }
        let mu = |v: f64, vq: f64| {
            let r = n_q * vq - n_w * v;
            r.cross(t_q) / den
        };
        let s = if den > 0.0 { 1.0 } else { -1.0 };
        let ext = mu(s * h_w, -s * h_q);
        let _ = p;
        return CornerKind::SquareOff { ext };
    }
    CornerKind::Butt
}

/// The corner of two walls of which at least one is an ARC (with the curvature terms): a mitre when it is forced (the pair
/// is nearly straight) or stored, otherwise the butt rule. The target of a butt corner against an arc (the line cut by the
/// cylinder of the arc's side face, the arc cut by the plane of the line's side face) is not modelled.
fn curved_corner_kind(model: &BimModel, w: &Wall, end: usize, q: &Wall, qend: usize) -> CornerKind {
    use crate::curved_joins as cj;
    use crate::model::WallJoinKind as K;
    let (Some(pa), Some(pb)) = (cj::Path::of(w), cj::Path::of(q)) else { return CornerKind::Butt };
    let (hw, hq) = (half_width(model, w), half_width(model, q));
    let Some(f) = cj::join_flags(&pa, end, hw * 2.0, &pb, qend, hq * 2.0) else { return CornerKind::Butt };
    if f.collinear {
        return CornerKind::Butt;
    }
    // ENGINE CHOICE: OBSERVED (i and round 33
    // f) the reference product lists the wall whose end disallows the join FIRST in a two-wall join; the geometry of `i1`
    //    (cut by the INNER cylinder of `i2`, which is side 0 of the second wall) follows that order
    let w_first = if !q.join_allowed[qend] { false } else if !w.join_allowed[end] { true } else { join_list_precedes(w, q) };
    let stored = if w_first { model.join_type_at(w.id, end) } else { model.join_type_at(q.id, qend) };
    if !q.join_allowed[qend] {
        // the other end is detached: only the cut of this wall's own end applies
        return match crate::curved_ends::corner_cut(w, end, hw, q, qend, hq, w_first) {
            Some(cut) => CornerKind::Cut { corners: cut.corners, circle: cut.circle, via: cut.via },
            None => CornerKind::Butt,
        };
    }
    if f.nearly_straight || stored == K::Miter {
        let p = if end == 0 { w.start } else { w.end };
        if let Some(corners) = cj::mitre_corners(&pa, end, hw, &pb, qend, hq, p) {
            return CornerKind::MiterAt { corners };
        }
    }
    // The butt corner (type 0, or type 2 read as 0 on the second wall) is bounded by the other wall's side face
    if stored == K::Abut || (stored == K::SquareOff && !w_first) {
        if let Some(cut) = crate::curved_ends::corner_cut(w, end, hw, q, qend, hq, w_first) {
            return CornerKind::Cut { corners: cut.corners, circle: cut.circle, via: cut.via };
        }
    }
    CornerKind::Butt
}

/// For an end built as a MITRE, the outward offsets (m, along the wall axis from the location end) of its LEFT (`+width/2`
/// along the left normal of the start-to-end direction) and RIGHT corners: the points where the side faces meet the mitre
/// plane. `None` for any other end.
pub(crate) fn mitre_corner_offsets(model: &BimModel, w: &Wall, end: usize) -> Option<(f64, f64)> {
    let kind = corner_kind(model, w, end);
    if let CornerKind::MiterAt { corners } = kind {
        // the corners themselves: their offsets along the axis from the location end
        let axis = (w.end - w.start).normalized()?;
        let outward = if end == 0 { -axis } else { axis };
        let p = if end == 0 { w.start } else { w.end };
        let n = axis.perp_ccw();
        let (l, r) = if (corners.left - p).dot(n) >= (corners.right - p).dot(n) { (corners.left, corners.right) } else { (corners.right, corners.left) };
        return Some(((l - p).dot(outward), (r - p).dot(outward)));
    }
    let CornerKind::Miter { plane } = kind else { return None };
    let axis = (w.end - w.start).normalized()?;
    let outward = if end == 0 { -axis } else { axis };
    let n = axis.perp_ccw();
    let (b_d, b_n) = (plane.dot(outward), plane.dot(n));
    if b_n.abs() <= 1e-12 {
        return None;
    }
    let h = half_width(model, w);
    Some((h * b_d / b_n, -h * b_d / b_n))
}

/// For the THROUGH wall of a two-wall butt corner (type 0, this wall first): the outward offsets of its LEFT and RIGHT
/// corners on its end plane, which is the neighbour's side face AWAY from this wall's body (Spec W-05a type 0: the end is
/// bounded by a side face of the neighbour). Used for the box only: OBSERVED (300 / 200 corners at 60 degrees: `x1` max x
/// 6.2021, `y2` min (5.7979, 139.85)) the box reaches those corners and not the axial extension, which equals them only for
/// equal widths. `None` for any other end.
pub(crate) fn through_corner_offsets(model: &BimModel, w: &Wall, end: usize) -> Option<(f64, f64)> {
    if w.through.is_some() || !matches!(corner_kind(model, w, end), CornerKind::Butt) {
        return None;
    }
    let eff = end_effect(model, w, end);
    if eff.candidates != 1 || !eff.corner {
        return None;
    }
    let (q, qend) = sole_corner_partner(model, w, end)?;
    if q.through.is_some() || !join_list_precedes(w, q) {
        return None;
    }
    let p = if end == 0 { w.start } else { w.end };
    let axis = (w.end - w.start).normalized()?;
    let outward = if end == 0 { -axis } else { axis };
    let dir_w = -outward;
    let dir_q = (if qend == 0 { q.end - q.start } else { q.start - q.end }).normalized()?;
    let n_q = dir_q.perp_ccw();
    let away = if n_q.dot(dir_w) < 0.0 { n_q } else { -n_q };
    let a = p + away * half_width(model, q);
    let den = outward.cross(dir_q);
    if den.abs() <= 1e-12 {
        return None;
    }
    let n = axis.perp_ccw();
    let h = half_width(model, w);
    let mu = |v: f64| ((a - p) - n * v).cross(dir_q) / den;
    Some((mu(h), mu(-h)))
}

/// What one end of a wall gets from the walls at its control point, and from which neighbour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EndEffect {
    /// Signed outward extension (m), as [`end_extension`] returns it.
    pub ext: f64,
    /// Unit direction of the chosen neighbour's axis (either sign). `None` when nothing joins.
    pub partner_dir: Option<Vec2>,
    /// How many neighbours contribute a candidate extension (corners and T hosts). More than one is the three-or-more-walls
    /// case, UNKNOWN.
    pub candidates: usize,
    /// The chosen neighbour meets this wall at its END (a corner), not at its side (a T).
    pub corner: bool,
}

/// Signed outward extension (m) of wall `w` at `end` (0 = start, 1 = end): + extends, âˆ’ trims.
pub fn end_extension(model: &BimModel, w: &Wall, end: usize) -> f64 {
    end_effect(model, w, end).ext
}

pub(crate) fn end_effect(model: &BimModel, w: &Wall, end: usize) -> EndEffect {
    const NONE: EndEffect = EndEffect { ext: 0.0, partner_dir: None, candidates: 0, corner: false };
    // A stacked wall never receives a join step (its members do)
    if !w.join_allowed[end] || model.stacked_types.contains_key(&w.type_id) {
        return NONE;
    }
    // the ends of a junction of three walls or more are built by `crate::junctions`
    if crate::junctions::junction(model, w, end).is_some() {
        return NONE;
    }
    let tol = Tolerances::default();
    let p = if end == 0 { w.start } else { w.end };
    // Body direction of w leaving the shared point (into w's body).
    let dir_w = match end_dir(w, end) {
        Some(d) => d,
        None => return NONE,
    };
    let hw_w = half_width(model, w);

    let mut best: Option<f64> = None;
    let mut best_dir: Option<Vec2> = None;
    let mut best_corner = false;
    let mut candidates = 0usize;
    let list: Vec<&Wall> = walls_coexisting(model, w, p);
    for q in list {
        if q.id == w.id || !coexists(model, w, q) {
            continue;
        }
        let hw_q = half_width(model, q);

        // --- corner join: q has an endpoint coincident with p ---
        //
        // OBSERVED (j10): disallowing the join at the HOST's end returns the host to its unjoined body (area 30.0, bbox
        // x[0..10]) but leaves the CORNER wall's body exactly as joined (area 12.6037, the same mitred value as before).
        // Each wall's geometry at an end therefore depends only on ITS OWN permission there - the partner's flag decides
        // the partner's body, never this one. The edge and vertex counts are different: both walls count as "touching,
        // disallowed" (12 / 10).
        for qend in 0..2 {
            let qp = if qend == 0 { q.start } else { q.end };
            if (qp - p).length() > tol.vertex {
                continue;
            }
            let dir_q = match end_dir(q, qend) {
                Some(d) => d,
                None => continue,
            };
            let gamma = angle_between(dir_w, dir_q);
            // Collinear continuation (straight) or fully overlapping: no join geometry.
            if is_collinear(dir_w, dir_q) {
                continue;
            }
            // OBSERVED: the join geometry depends only on the **acute** angle between the two wall AXES, not on which side
            // the bodies are.
            //
            // So the angle driving the extension is always the obtuse one. A mitre is a plane cut
            // between two axes, and that plane does not care which side you stand on.
            let gamma_eff = gamma.max(std::f64::consts::PI - gamma);
            let half = gamma_eff / 2.0;
            let ext = if join_list_precedes(w, q) {
                hw_q * half.tan() // through wall extends
            } else {
                -hw_q / half.tan() // butt wall trims (cot = 1/tan)
            };
            candidates += 1;
            if best.map_or(true, |b: f64| ext.abs() > b.abs()) {
                best = Some(ext);
                best_dir = Some(dir_q);
                best_corner = true;
            }
        }

        // --- T-junction: p lies on q's interior (not at q's endpoints) ---
        if on_path_interior(q, p, tol.vertex) {
            let dir_q = match path_dir_at(q, p) {
                Some(d) => d,
                None => continue,
            };
            let theta = angle_between(dir_w, dir_q).min(std::f64::consts::PI - angle_between(dir_w, dir_q));
            let s = theta.sin();
            if s > 1e-9 {
                let trim = -hw_q / s; // this (stem) wall trims; host unchanged
                candidates += 1;
                if best.map_or(true, |b: f64| trim.abs() > b.abs()) {
                    best = Some(trim);
                    best_dir = Some(dir_q);
                    best_corner = false;
                }
            }
        }
        let _ = (hw_w, dir_w); // dir_w/hw_w reserved for future multi-join handling
    }
    // A mitred end is the bisector plane through the control point, so the body keeps the location end as its axial end
    // (its plan area is `length x width` exactly: what the wedge gains on one side it loses on the other; OBSERVED, every
    // mitred wall has Volume = length x width x height). Spec W-05a type 2 on the first wall: the square end.
    if candidates == 1 && best_corner {
        match corner_kind(model, w, end) {
            CornerKind::Miter { .. } => best = Some(0.0),
            // the corners of a curved mitre are placed by `mitre_corner_offsets`; the body keeps the location end as its axial end
            // and the mean of the two offsets goes into the plan area (`mitre_plan_correction`)
            CornerKind::MiterAt { .. } => best = Some(0.0),
            CornerKind::SquareOff { ext } => best = Some(ext),
            CornerKind::Butt | CornerKind::SquareOffSecond => {}
            // the body of a wall cut by the side face of an arc wall is the loop of `crate::curved_ends`
            CornerKind::Cut { .. } => best = Some(0.0),
        }
    }
    EndEffect { ext: best.unwrap_or(0.0), partner_dir: best_dir, candidates, corner: best_corner }
}

/// How much LONGER the larger side face of `w` is than the mean of its two side faces.
///
/// The wall Area the reference product reports is the larger of the two side faces' areas (a maximum, not a sum and not an
/// average), measured on the geometry as the regeneration left it: there is no mitre-specific code, the end trimming is
/// already in the faces. The two side faces of a wall have different lengths exactly when an end plane is not square to the
/// axis. A butt end is bounded by a SIDE FACE of the neighbour (type 0 "side face of the neighbour"), so its plane is
/// parallel to the neighbour's axis, whichever of the neighbour's two side faces it is (that choice moves the mean length,
/// which the engine takes from its Volume rule, and never the difference between the faces).
///
/// In the wall frame (`u` along the axis, `v` to the left) an end plane parallel to a neighbour
/// axis with direction `(q_u, q_v)` meets the side `v = Â±h_w` at `u_e Â± h_w Â· q_u / q_v`, so the two
/// side faces differ in length by `2 h_w Â· |s_end - s_start|` with `s = q_u / q_v` (0 for a free
/// end, which is square), and the larger exceeds the mean by `h_w Â· |s_end - s_start|`. That is the
/// value returned. Two ends slanted the same way (a parallelogram) cancel; mirrored ends add.
///
/// * The forced-mitre band formula `h tan(gamma/2)` is not decided by.
/// * an end has more than one neighbour (three or more walls at a point), UNKNOWN (the branch structure after the neighbour
///   selection).
pub fn side_face_spread(model: &BimModel, w: &Wall) -> Option<f64> {
    let hw = half_width(model, w);
    if hw <= 0.0 {
        return Some(0.0);
    }
    let axis = (w.end - w.start).normalized()?;
    let mut slope = [0.0_f64; 2];
    for end in 0..2 {
        let eff = end_effect(model, w, end);
        if eff.candidates == 0 {
            continue; // a free end is square: slope 0
        }
        if eff.candidates > 1 {
            return None;
        }
        // A mitred end plane runs along the bisector; Spec W-05a type 2: a square one.
        match corner_kind(model, w, end) {
            CornerKind::Miter { plane } => {
                let (bu, bv) = (plane.dot(axis), axis.cross(plane));
                if bv.abs() <= 1e-12 {
                    return None;
                }
                slope[end] = bu / bv;
                continue;
            }
            CornerKind::MiterAt { .. } => {
                // the two corners' offsets l and r: the sides differ by l - r at this end
                let (l, r) = mitre_corner_offsets(model, w, end)?;
                slope[end] = (l - r) / 2.0 / hw * if end == 1 { 1.0 } else { -1.0 };
                continue;
            }
            CornerKind::SquareOff { .. } => continue, // square: slope 0
            // the side faces of a wall cut by the side face of an arc wall are measured on its loop (`crate::curved_ends`)
            CornerKind::Cut { .. } => return None,
            CornerKind::Butt | CornerKind::SquareOffSecond => {}
        }
        let q = eff.partner_dir?;
        if eff.corner {
            let into_w = if end == 0 { axis } else { -axis };
            if crate::join_end::corner_flags(into_w, q).1 {
                return None;
            }
        }
        let (qu, qv) = (q.dot(axis), axis.cross(q));
        if qv.abs() <= 1e-9 {
            return None;
        }
        slope[end] = qu / qv;
    }
    Some(hw * (slope[1] - slope[0]).abs())
}

/// The length of the LARGER side face of `w`, the length whose product with the height is the wall Area before openings.
///
/// `solid_length` is the engine's axial length (`L + ext_start + ext_end`, which for equal widths
/// and one oblique end already is the larger face), `mitre_correction` is [`mitre_plan_correction`].
/// The mean of the two faces is `solid_length + mitre_correction / width`, the length the engine's
/// Volume rule was verified against; the larger face is the mean plus [`side_face_spread`]. When
/// the spread is not decided, or the correction was clipped, the axial length is returned, which is
/// the behaviour before this function existed.
pub fn larger_side_face_length(model: &BimModel, w: &Wall, solid_length: f64, mitre_correction: f64) -> f64 {
    let width = half_width(model, w) * 2.0;
    if width <= 0.0 || solid_length <= 0.0 {
        return solid_length;
    }
    let mean = solid_length + mitre_correction / width;
    if mean <= 0.0 {
        return solid_length;
    }
    match side_face_spread(model, w) {
        Some(spread) => mean + spread,
        None => solid_length,
    }
}

/// Plan-area correction a mitred corner applies to this wall's body, in mÂ².
///
/// OBSERVED: a wall's **Volume** is not its axial body's volume. At a mitred corner the body gains or loses a wedge, and
/// the correction is
///
/// > `h_q Â· w Â· cot Î³`
///
/// where `w` is this wall's width, `h_q` the partner's half width, and `Î³` the angle between the
/// two walls' **body** directions at the shared corner. It is summed over the wall's joined ends.
///
/// Four measured walls, two corners, no exception â€” and the same correction applies to the through
/// wall and to the butt wall, because they share `Î³`:
///
/// | wall | Î³ | axial plan | correction | volume | reference |
/// |---|---|---|---|---|---|
/// | `a_h` through | 135Â° | 1.0482842712 | âˆ’0.020 | 3.0848528137424 | 3.0848528137424 |
/// | `a_d` butt | 135Â° | 0.8402438662 | âˆ’0.020 | 2.4607315985292 | 2.4607315985292 |
/// | `o_h` through | 45Â° | 1.0082842712 | **+0.020** | 3.0848528137424 | 3.0848528137424 |
/// | `o_d` butt | 45Â° | 0.8002438662 | **+0.020** | 2.4607315985292 | 2.4607315985292 |
///
/// Two things make this worth trusting. `cot 90Â° = 0`, so a **perpendicular** join corrects by
/// nothing â€” which is exactly why every perpendicular case already matched and why wiring this in
/// cannot regress them. And the sign flips with the corner: an acute body angle *adds* material
/// where an obtuse one removes it, which is what a mitre does and what no axial trim can express.
///
/// This is the closed form of the same wedge `crates/asterion-geom`'s boolean produces for the
/// butt wall (`angled_join_difference_reproduces_the_measured_volume_exactly`). The closed form is
/// used here because it also covers the **through** wall, which the one-sided difference does not:
/// subtracting the butt's untrimmed band from the through body removes 0.0282842712 mÂ², while the
/// reference removes exactly 0.02.
///
/// **Area is deliberately NOT corrected.** Round 5 measured the two apart: `a_d` reports Area
/// 12.603657992645926, which is the **axial** body (4.2012193309 Ã— 3.000), while its Volume is the
/// mitred one. So `Volume â‰  Area Ã— width` for a joined wall â€” the identity that holds everywhere
/// else in the engine breaks here, and it breaks in the reference, not in ASTERION.
pub fn mitre_plan_correction(model: &BimModel, w: &Wall) -> f64 {
    let width = half_width(model, w) * 2.0;
    if width <= 0.0 {
        return 0.0;
    }
    let tol = Tolerances::default();
    let mut total = 0.0;
    for end in 0..2 {
        if !w.join_allowed[end] {
            continue;
        }
        let p = if end == 0 { w.start } else { w.end };
        let Some(dir_w) = end_dir(w, end) else {
            continue;
        };
        // Spec W-05a: a mitred or squared end carries no butt wedge (see `end_effect`); an end cut by the side face of an
        // arc wall has its own loop (`crate::curved_ends`)
        if matches!(corner_kind(model, w, end), CornerKind::Miter { .. } | CornerKind::SquareOff { .. } | CornerKind::Cut { .. })
            || crate::junctions::junction(model, w, end).is_some()
        {
            continue;
        }
        // a curved mitre: the plane does not pass through the control point, the plan area gains the mean of the two offsets
        if let CornerKind::MiterAt { .. } = corner_kind(model, w, end) {
            if let Some((l, r)) = mitre_corner_offsets(model, w, end) {
                total += (l + r) / 2.0 * width;
            }
            continue;
        }
        // The partner chosen is the one `end_extension` acts on, so the two stay in step.
        let Some(q) = corner_partner(model, w, end) else { continue };
        let qend = if (q.start - p).length() <= tol.vertex { 0 } else { 1 };
        let Some(dir_q) = end_dir(q, qend) else {
            continue;
        };
        let gamma = angle_between(dir_w, dir_q);
        if is_collinear(dir_w, dir_q) {
            continue; // collinear: no corner, no wedge
        }
        // Same obtuse convention as `end_extension`: the correction must be computed on the
        // angle the extension was, or the two describe different corners.
        let gamma_eff = gamma.max(std::f64::consts::PI - gamma);
        let tan = gamma_eff.tan();
        if !tan.is_finite() || tan.abs() < 1e-12 {
            continue; // perpendicular: cot = 0, nothing to correct
        }
        total += half_width(model, q) * width / tan;
    }
    total
}

/// The wall sharing a corner with `w` at `end` (the largest mitre magnitude among the walls whose
/// end meets it, as [`end_extension`] picks its extension). Corner joins only: a T-junction has no mitre, and round 5 measured none.
/// Extra end-cap length a mitred corner gives this wall, in metres, summed over its ends.
///
/// OBSERVED: a mitre replaces the wall's **square** end cap, of length `w`, with a **diagonal** one. A diagonal across a
/// band of width `w` cut at an axis angle `Î¸` is `w / sin Î¸` long, so the cap grows by
///
/// > `w Â· (1 / sin Î¸ âˆ’ 1)`
///
/// and that is exactly what the measured surface areas need: 0.2 Â· (âˆš2 âˆ’ 1) = 0.0828427125 m at the 45Â° corner, the
/// same for all four of its walls.
///
/// The formula is **derived, not fitted**, and the check that matters is its limit: at Î¸ = 90Â° the
/// cut is square, `1 / sin 90Â° âˆ’ 1 = 0`, and a perpendicular join changes nothing. That is why
/// every perpendicular case keeps passing, and it is what distinguishes this from a constant that
/// happens to fit one corner.
pub fn mitre_cap_extra_length(model: &BimModel, w: &Wall) -> f64 {
    let width = half_width(model, w) * 2.0;
    if width <= 0.0 {
        return 0.0;
    }
    let tol = Tolerances::default();
    let mut total = 0.0;
    for end in 0..2 {
        if !w.join_allowed[end] {
            continue;
        }
        let p = if end == 0 { w.start } else { w.end };
        if crate::junctions::junction(model, w, end).is_some() {
            continue;
        }
        let Some(dir_w) = end_dir(w, end) else {
            continue;
        };
        // A mitred end cap is the bisector plane across the band, `w / |sin|` long with the angle between the plane and the
        // axis; Spec W-05a type 2: a squared end cap is square (no extra).
        match corner_kind(model, w, end) {
            CornerKind::Miter { plane } => {
                let s = dir_w.cross(plane).abs();
                if s > 1e-12 {
                    total += width * (1.0 / s - 1.0);
                }
                continue;
            }
            CornerKind::MiterAt { corners } => {
                total += (corners.left - corners.right).length() - width;
                continue;
            }
            CornerKind::SquareOff { .. } | CornerKind::Cut { .. } => continue,
            CornerKind::Butt | CornerKind::SquareOffSecond => {}
        }
        let Some(q) = corner_partner(model, w, end) else {
            // Spec W-05a type 0: a T stem's end is bounded by a SIDE face of the host, a plane parallel to the host axis,
            // so its cap crosses the band obliquely, `w / sin(theta)` long (OBSERVED stems at 30 / 60 / 120 degrees: +0.6
            // and +0.0928 m2).
            let eff = end_effect(model, w, end);
            if eff.candidates == 1 && !eff.corner {
                if let Some(qd) = eff.partner_dir {
                    let s = dir_w.cross(qd).abs();
                    if s > 1e-12 {
                        total += width * (1.0 / s - 1.0);
                    }
                }
            }
            continue;
        };
        let qend = if (q.start - p).length() <= tol.vertex { 0 } else { 1 };
        let Some(dir_q) = end_dir(q, qend) else {
            continue;
        };
        let gamma = angle_between(dir_w, dir_q);
        // The ACUTE angle between the axes: a corner and its supplement mitre identically.
        let theta = gamma.min(std::f64::consts::PI - gamma);
        let s = theta.sin();
        if s <= 1e-12 {
            continue;
        }
        total += width * (1.0 / s - 1.0);
    }
    total
}

fn corner_partner<'a>(model: &'a BimModel, w: &Wall, end: usize) -> Option<&'a Wall> {
    let tol = Tolerances::default();
    let p = if end == 0 { w.start } else { w.end };
    let dir_w = end_dir(w, end)?;
    let mut best: Option<(f64, &Wall)> = None;
    for q in walls_near(model, p) {
        if q.id == w.id || !coexists(model, w, q) {
            continue;
        }
        for qend in 0..2 {
            let qp = if qend == 0 { q.start } else { q.end };
            if (qp - p).length() > tol.vertex {
                continue;
            }
            let Some(dir_q) = end_dir(q, qend) else {
                continue;
            };
            let gamma = angle_between(dir_w, dir_q);
            if is_collinear(dir_w, dir_q) {
                continue;
            }
            let mag = half_width(model, q) * (gamma.max(std::f64::consts::PI - gamma) / 2.0).tan();
            if best.map_or(true, |(m, _)| mag > m) {
                best = Some((mag, q));
            }
        }
    }
    best.map(|(_, q)| q)
}

// ---------------------------------------------------------------------------------------------
// Join classes: the reference's B-rep bookkeeping, as a STEP FUNCTION of the join class.
// ---------------------------------------------------------------------------------------------

/// What a wall's body looks like to the reference's B-rep, per join incidence.
///
/// OBSERVED: the edge and vertex counts of a joined wall are a step function of the join CLASS and not of the geometry. The
/// probe varied the join continuously - stem on the face, at the centreline, through the host, overshooting into a cross; a
/// corner at 90 and at 45 degrees, pulled 0.2 inside and pushed 0.2 beyond - and the counts never moved within a class.
///
/// | class (the wall being counted) | edges | vertices | faces |
/// |---|---:|---:|---:|
/// | lone wall | 12 | 8 | 6 |
/// | T junction - the host | 14 | 12 | 6 |
/// | T junction - the stem | 10 | 8 | 6 |
/// | L corner - the through wall | 11 | 10 | 6 |
/// | L corner - the butt wall | 9 | 8 | 6 |
/// | collinear (either wall) | 8 | 8 | 6 |
/// | join disallowed, walls still touching | 12 | 10 | 6 |
///
/// The offsets are relative to the lone wall (12 / 8) and are ADDED per incidence: that is the
/// only model that reproduces the one combination no formula explained, `j11` - a host that is
/// collinear at one end and has a disallowed corner at the other - which measured **8 edges with
/// 10 vertices**: `12 - 4 + 0 = 8` and `8 + 0 + 2 = 10`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinClass {
    TeeHost,
    TeeStem,
    CornerThrough,
    CornerButt,
    Collinear,
    TouchingDisallowed,
    /// The earlier wall of two walls that CROSS in an X (OBSERVED: a 10 m wall crossed by a 10 m wall at their middles,
    /// both 0.2 thick): its solid keeps its volume and gains 4 edges and 8 vertices (16 / 16).
    CrossHost,
    /// The later wall of such a pair: it is CUT by the earlier one's band, two lumps (OBSERVED: 20
    /// edges, 16 vertices, 8 faces with the cut; `12 - 2` edges before it).
    CrossCut,
    /// A crossing host whose crossing walls are all lower than it (OBSERVED, `h1`: 14 edges, 12 vertices; the same offsets
    /// as a T host).
    CrossHostLower,
    /// A cut wall lower than its cutter (OBSERVED, `h2`: 22 edges, 16 vertices).
    CrossCutLower,
    /// OBSERVED forced mitres at 10, 20, 160 and 170 degrees and stored mitres at 60, 90 and 120 degrees: every mitred wall
    /// has 10 edges and 8 vertices.
    Mitre,
    /// The first wall of a SQUARE-OFF corner that is not perpendicular (OBSERVED at 60 and 120 degrees: 12 edges, 10
    /// vertices; at 90 degrees the squared end is the through wall's and the counts are the through wall's).
    SquareOffFirst,
    /// The second wall of such a corner (OBSERVED, same walls: 10 edges, 8 vertices; at 90 degrees
    /// the butt wall's).
    SquareOffSecond,
}

impl JoinClass {
    /// `(edges, vertices)` offset from a lone wall's `(12, 8)`.
    pub fn offsets(self) -> (i64, i64) {
        match self {
            JoinClass::TeeHost => (2, 4),
            JoinClass::TeeStem => (-2, 0),
            JoinClass::CornerThrough => (-1, 2),
            JoinClass::CornerButt => (-3, 0),
            JoinClass::Collinear => (-4, 0),
            JoinClass::TouchingDisallowed => (0, 2),
            JoinClass::CrossHost => (4, 8),
            JoinClass::CrossCut => (-2, 0),
            JoinClass::CrossHostLower => (2, 4),
            JoinClass::CrossCutLower => (-1, 0),
            JoinClass::Mitre => (-2, 0),
            JoinClass::SquareOffFirst => (0, 2),
            JoinClass::SquareOffSecond => (-2, 0),
        }
    }
}

/// The walls that CROSS `w` in an X, each with whether `w` is the one that is CUT (the later one:
/// `join_list_precedes(other, w)`: the other wall's id is lower). Two straight walls of one base level cross when their
/// location lines intersect at a point interior to both and neither wall's end lies inside the other's body (otherwise it
/// is a corner or a T, handled by the end joins). OBSERVED: the wall created second is cut by the band of the first; the
/// number of walls, the widths and the angle are not yet measured beyond that one perpendicular pair of equal widths (round
/// 23 measures them).
pub fn crossings(model: &BimModel, w: &Wall) -> Vec<(ElementId, bool)> {
    crate::crossings::raw_crossings(model, w).into_iter().map(|c| (c.q, c.cut)).collect()
}

/// Every join incidence of `w`: one per joined end, plus one per wall whose end lands on `w`'s
/// interior (those make `w` a T host).
///
/// Mirrors [`end_extension`]'s partner choice so the geometry and the bookkeeping can never
/// describe different joins.
pub fn join_classes(model: &BimModel, w: &Wall) -> Vec<JoinClass> {
    join_classes_impl(model, w, false)
}

/// A neighbour that does not COEXIST with `w` is not in the join array at all; with `tweaked` set (the edge and vertex
/// bookkeeping) a neighbour whose created or demolished phase differs is joined but NOT tweaked, so it adds none of the
/// shared-edge offsets of its join class.
fn join_classes_impl(model: &BimModel, w: &Wall, tweaked: bool) -> Vec<JoinClass> {
    let tol = Tolerances::default();
    let mut out = Vec::new();

    for end in 0..2 {
        let p = if end == 0 { w.start } else { w.end };
        // the ends of a junction of three walls or more are counted by `crate::junctions::offsets`
        if crate::junctions::junction(model, w, end).is_some() {
            continue;
        }
        if !w.join_allowed[end] {
            let tol = Tolerances::default().vertex;
            let p = if end == 0 { w.start } else { w.end };
            let others = walls_near(model, p)
                .into_iter()
                .filter(|q| q.id != w.id && q.through.is_none() && (0..2).any(|e| ((if e == 0 { q.start } else { q.end }) - p).length() <= tol && q.join_allowed[e]))
                .count();
            if others >= 3 {
                continue;
            }
        }
        let Some(dir_w) = end_dir(w, end) else {
            continue;
        };
        // The class chosen for this end: the first partner by ascending id, so the result does
        // not depend on map iteration order.
        let mut chosen: Option<(u64, JoinClass)> = None;
        for q in walls_near(model, p) {
            if q.id == w.id || !pair_in_join(model, w, q, tweaked) {
                continue;
            }
            let mut class: Option<JoinClass> = None;

            // Corner / collinear: one of q's ENDS coincides with p.
            for qend in 0..2 {
                let qp = if qend == 0 { q.start } else { q.end };
                if (qp - p).length() > tol.vertex {
                    continue;
                }
                let Some(dir_q) = end_dir(q, qend) else {
                    continue;
                };
                // The two stay apart only because the engine's observed bookkeeping is for the continuation (round 10,
                // j11).
                let collinear = is_collinear(dir_w, dir_q);
                if collinear && dir_w.dot(dir_q) > 0.0 {
                    continue; // folded back on itself: no join geometry
                }
                class = Some(if !w.join_allowed[end] || !q.join_allowed[qend] {
                    JoinClass::TouchingDisallowed
                } else if collinear {
                    JoinClass::Collinear
                } else if join_list_precedes(w, q) {
                    JoinClass::CornerThrough
                } else {
                    JoinClass::CornerButt
                });
            }

            // T: p lies on q's interior, so w is the stem. a PARALLEL wall (an overlapping or collinear one) is no T
            // partner: `end_effect` makes no join there (sin of the angle <= 1e-9), so neither does the bookkeeping.
            // OBSERVED: two overlapping parallel walls keep 12 edges / 8 vertices each.
            let q_dir = path_dir_at(q, p);
            let not_parallel = q_dir.map_or(false, |d| dir_w.cross(d).abs() > 1e-9);
            if class.is_none() && not_parallel && on_path_interior(q, p, tol.vertex) {
                class = Some(if w.join_allowed[end] {
                    JoinClass::TeeStem
                } else {
                    JoinClass::TouchingDisallowed
                });
            }

            if let Some(c) = class {
                // Determinism only (not the join list order): among several partners the class of
                // the lowest id is reported, so the result does not depend on map iteration order.
                if chosen.map_or(true, |(id, _)| q.id.0 < id) {
                    chosen = Some((q.id.0, c));
                }
            }
        }
        if let Some((_, c)) = chosen {
            // Spec W-05a: a mitred or squared two-wall corner has its own bookkeeping
            let c = match (c, corner_kind(model, w, end)) {
                (JoinClass::CornerThrough | JoinClass::CornerButt, CornerKind::Miter { .. } | CornerKind::MiterAt { .. }) => JoinClass::Mitre,
                (JoinClass::CornerThrough, CornerKind::SquareOff { .. }) if !perpendicular_corner(model, w, end) => JoinClass::SquareOffFirst,
                (JoinClass::CornerButt, CornerKind::SquareOffSecond) if !perpendicular_corner(model, w, end) => JoinClass::SquareOffSecond,
                (c, _) => c,
            };
            out.push(c);
        }
    }

    // an X crossing: the earlier wall hosts, the later one is cut
    out.extend(crate::crossings::classes(model, w));
    // w is a HOST for every wall whose end lands on w's interior (a PARALLEL wall is no stem, see
    // the T case above).
    for q in walls_around(model, w, 0.0) {
        if q.id == w.id || !pair_in_join(model, w, q, tweaked) {
            continue;
        }
        for qend in 0..2 {
            let qp = if qend == 0 { q.start } else { q.end };
            if on_path_interior(w, qp, tol.vertex) {
                // a PARALLEL wall is no stem (the tangent of the host at the stem's point, the stem's own at its end)
                if !matches!((path_dir_at(w, qp), end_dir(q, qend)), (Some(a), Some(b)) if a.cross(b).abs() > 1e-9) {
                    continue;
                }
                out.push(if q.join_allowed[qend] {
                    JoinClass::TeeHost
                } else {
                    JoinClass::TouchingDisallowed
                });
            }
        }
    }
    out
}

/// `(edge, vertex)` count of a wall's solid with no openings: `(12, 8)` plus the offsets of every
/// join incidence.
pub fn join_edge_vertex_counts(model: &BimModel, w: &Wall) -> (i64, i64) {
    let (je, jv, _) = crate::junctions::offsets(model, w);
    let mut e = 12 + je;
    let mut v = 8 + jv;
    for c in join_classes_impl(model, w, true) {
        let (de, dv) = c.offsets();
        e += de;
        v += dv;
    }
    let (de, dv) = unequal_height_corners(model, w);
    e += de;
    v += dv;
    (e, v)
}

/// The top of a wall, from its stored fields (no regeneration: this is called from it).
fn quick_top(model: &BimModel, w: &Wall) -> f64 {
    let base = model.levels.get(&w.base_level).map_or(0.0, |l| l.elevation) + w.base_offset;
    match w.top_level.and_then(|l| model.levels.get(&l)) {
        Some(l) => l.elevation + w.top_offset,
        None => base + w.unconnected_height,
    }
}

/// OBSERVED (a room of four walls, one of them 2.5 m then 3.2 m among 3 m ones), per corner join with a wall of another
/// height, `(edges, vertices)` added to the solid of `w`:
/// * the wall that runs through the corner: one edge when the butting wall is lower; when it is taller no edge and one vertex less;
/// * the butting wall: one edge when it is the lower, two edges and one vertex when it is the taller. ENGINE CHOICE: fitted
///   to those two cases.
///
/// OBSERVED: corners of different BOTTOMS (`e`: the second wall 0.5 up, `f`: the first 0.5 down, both also with different
/// tops) add one more edge to each wall (12 / 9 for both walls of both corners); a T whose stem has another height (`g`
/// lower, `h` taller) gives the host 13 / 10 and the stem 11 / 8 (a T host is 14 / 12 and a stem 10 / 8 at equal heights).
/// Same ENGINE CHOICE, fitted to those four joins.
fn unequal_height_corners(model: &BimModel, w: &Wall) -> (i64, i64) {
    let tol = Tolerances::default();
    let (mut n, mut nv) = (0, 0);
    for end in 0..2 {
        let p = if end == 0 { w.start } else { w.end };
        let partner = walls_near(model, p)
            .into_iter()
            .filter(|q| q.id != w.id && pair_in_join(model, w, q, true))
            .filter(|q| [q.start, q.end].iter().any(|qp| (*qp - p).length() <= tol.vertex))
            .filter(|q| w.join_allowed[end])
            .min_by_key(|q| q.id.0);
        if let Some(q) = partner {
            let (tq, tw) = (quick_top(model, q), quick_top(model, w));
            if (quick_bottom(model, q) - quick_bottom(model, w)).abs() > 1e-9 {
                n += 1;
            }
            if (tq - tw).abs() > 1e-9 {
                if join_list_precedes(w, q) {
                    // w runs through: the partner butts
                    if tq < tw {
                        n += 1;
                    } else {
                        nv -= 1;
                    }
                } else if tw < tq {
                    n += 1;
                } else {
                    n += 2;
                    nv += 1;
                }
            }
        }
    }
    // T joins with another top: `w` as the stem, then `w` as the host of each stem
    for end in 0..2 {
        if !w.join_allowed[end] {
            continue;
        }
        let p = if end == 0 { w.start } else { w.end };
        // a parallel (overlapping) wall is no T partner, as in `join_classes`
        let not_parallel = |a: &Wall, b: &Wall| match ((a.end - a.start).normalized(), (b.end - b.start).normalized()) {
            (Some(x), Some(y)) => x.cross(y).abs() > 1e-9,
            _ => false,
        };
        let tops_differ = walls_near(model, p).into_iter().filter(|q| q.id != w.id && pair_in_join(model, w, q, true) && q.through.is_none() && not_parallel(w, q)).any(|q| {
            point_on_segment_interior(p, q.start, q.end, tol.vertex) && (quick_top(model, q) - quick_top(model, w)).abs() > 1e-9
        });
        if tops_differ {
            n += 1;
        }
    }
    if w.through.is_none() {
        let w_dir = (w.end - w.start).normalized();
        for q in walls_around(model, w, 0.0).into_iter().filter(|q| q.id != w.id && pair_in_join(model, w, q, true)) {
            if !matches!((w_dir, (q.end - q.start).normalized()), (Some(a), Some(b)) if a.cross(b).abs() > 1e-9) {
                continue;
            }
            for qend in 0..2 {
                let qp = if qend == 0 { q.start } else { q.end };
                if q.join_allowed[qend]
                    && point_on_segment_interior(qp, w.start, w.end, tol.vertex)
                    && (quick_top(model, q) - quick_top(model, w)).abs() > 1e-9
                {
                    n -= 1;
                    nv -= 2;
                }
            }
        }
    }
    (n, nv)
}

/// The bottom of a wall, from its stored fields.
fn quick_bottom(model: &BimModel, w: &Wall) -> f64 {
    model.levels.get(&w.base_level).map_or(0.0, |l| l.elevation) + w.base_offset
}

/// The two walls of the corner at `end` of `w` are perpendicular (`|cos| <= 1e-9`).
fn perpendicular_corner(model: &BimModel, w: &Wall, end: usize) -> bool {
    let Some((q, qend)) = sole_corner_partner(model, w, end) else { return false };
    let dir_w = (if end == 0 { w.end - w.start } else { w.start - w.end }).normalized();
    let dir_q = (if qend == 0 { q.end - q.start } else { q.start - q.end }).normalized();
    matches!((dir_w, dir_q), (Some(a), Some(b)) if a.dot(b).abs() <= 1e-9)
}

pub(crate) fn half_width(model: &BimModel, w: &Wall) -> f64 {
    model.wall_types.get(&w.type_id).map(|t| t.structure.width() / 2.0).unwrap_or(0.0)
}

/// Two unit directions leaving the shared corner are collinear (a straight continuation, or one wall folded back on the
/// other) when `|sine|` of the angle between them is at most `0.0017453292519943279` (0.1 degrees). For two line walls that
/// is exactly the test the reference product runs inside; it replaces the engine's earlier plain angle comparison with the
/// same constant.
fn is_collinear(a: Vec2, b: Vec2) -> bool {
    crate::join_end::corner_flags(a, b).0
}

/// Unsigned angle between two unit vectors, in [0, Ï€].
fn angle_between(a: Vec2, b: Vec2) -> f64 {
    a.dot(b).clamp(-1.0, 1.0).acos()
}

/// True if `p` lies on the interior of the location line of `q` (a T attachment to its side).
pub(crate) fn point_on_wall_interior(q: &Wall, p: Vec2, tol: f64) -> bool {
    point_on_segment_interior(p, q.start, q.end, tol)
}

/// True if `p` lies strictly between `a` and `b` (on the segment, not at an endpoint).
fn point_on_segment_interior(p: Vec2, a: Vec2, b: Vec2, tol: f64) -> bool {
    let ab = b - a;
    let len = ab.length();
    if len < tol {
        return false;
    }
    let d = ab * (1.0 / len);
    let t = (p - a).dot(d);
    if t <= tol || t >= len - tol {
        return false; // at or past an endpoint
    }
    let perp = (p - a).cross(d).abs(); // distance from the line
    perp <= tol
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ops, CompoundStructure, Layer, LayerFunction};
    use asterion_geom::Vec2;

    fn build(widths: &[(f64, f64, f64, f64, f64)]) -> (crate::BimDocument, Vec<asterion_core::ElementId>) {
        // widths entries: (w, x1,y1,x2,y2) each a wall of given width
        let mut doc = crate::new_document();
        let mut ids = Vec::new();
        let o = doc.transact("b", false, |m, cx| {
            let level = ops::create_level(m, cx, "L0", 0.0)?;
            let mut out = Vec::new();
            for (i, (w, x1, y1, x2, y2)) in widths.iter().enumerate() {
                let t = ops::create_wall_type(m, cx, &format!("T{i}"), CompoundStructure::new(vec![Layer { width: *w, function: LayerFunction::Structure }], 0, 0)?)?;
                let id = ops::create_wall(m, cx, ops::NewWall { wall_type: t, level, start: Vec2::new(*x1, *y1), end: Vec2::new(*x2, *y2), height: 3.0, base_offset: 0.0, flip: false, structural: false })?;
                out.push(id);
            }
            Ok(out)
        });
        ids.extend(o.value.unwrap());
        (doc, ids)
    }

    /// OBSERVED: two 10 m walls of 0.2 crossing at their middles - the earlier one is whole (30 m2, 6 m3, 16 edges, 16
    /// vertices), the later one loses the 0.2 x 0.2 x 3 band (29.4 m2, 5.88 m3, 8 faces, 20 edges, 16 vertices, surface
    /// 65.12).
    #[test]
    fn two_walls_crossing_in_an_x_cut_the_later_one_by_the_earlier_ones_band() {
        let (doc, ids) = build(&[(0.2, 0.0, 0.0, 10.0, 0.0), (0.2, 5.0, -5.0, 5.0, 5.0)]);
        let m = doc.model();
        let a = crate::regen::wall_state(m, &m.walls[&ids[0]]).unwrap();
        let b = crate::regen::wall_state(m, &m.walls[&ids[1]]).unwrap();
        assert!((a.area - 30.0).abs() < 1e-9 && (a.volume - 6.0).abs() < 1e-9, "{} {}", a.area, a.volume);
        assert_eq!((a.solid_face_count, a.solid_edge_count, a.solid_vertex_count), (6, 16, 16));
        assert!((b.area - 29.4).abs() < 1e-9 && (b.volume - 5.88).abs() < 1e-9, "{} {}", b.area, b.volume);
        assert_eq!((b.solid_face_count, b.solid_edge_count, b.solid_vertex_count), (8, 20, 16));
        assert!((b.surface_area - 65.12).abs() < 1e-9, "{}", b.surface_area);
        // the cut is by id: the order the walls were made in decides which one
        assert_eq!(crossings(m, &m.walls[&ids[0]]), vec![(ids[1], false)]);
        assert_eq!(crossings(m, &m.walls[&ids[1]]), vec![(ids[0], true)]);
    }

    #[test]
    fn a_corner_a_t_and_parallel_walls_are_not_crossings() {
        // an L corner, a T (the stem ends on the host) and two parallel walls
        let (doc, ids) = build(&[
            (0.2, 0.0, 0.0, 10.0, 0.0),
            (0.2, 10.0, 0.0, 10.0, 5.0),
            (0.2, 4.0, 0.0, 4.0, 6.0),
            (0.2, 0.0, 8.0, 10.0, 8.0),
        ]);
        let m = doc.model();
        for id in &ids {
            assert!(crossings(m, &m.walls[id]).is_empty(), "{id:?}");
        }
        // a wall that passes a stem's end by its width is still a T, not an X
        let (doc2, ids2) = build(&[(0.2, 0.0, 0.0, 10.0, 0.0), (0.2, 5.0, -0.05, 5.0, 5.0)]);
        assert!(crossings(doc2.model(), &doc2.model().walls[&ids2[0]]).is_empty());
    }

    #[test]
    fn perpendicular_equal_width_through_extends_butt_trims() {
        // H (id first) 0,0->5,0 ; V 5,0->5,4 ; width 0.2 -> half 0.1
        let (doc, ids) = build(&[(0.2, 0.0, 0.0, 5.0, 0.0), (0.2, 5.0, 0.0, 5.0, 4.0)]);
        let m = doc.model();
        let h = m.wall(ids[0]).unwrap();
        let v = m.wall(ids[1]).unwrap();
        // H is through (lower id): extends at end 1 by other half-width 0.1
        assert!((end_extension(m, h, 1) - 0.1).abs() < 1e-9, "H end1 {}", end_extension(m, h, 1));
        assert!(end_extension(m, h, 0).abs() < 1e-9);
        // V is butt: trims at end 0 by through half-width 0.1
        assert!((end_extension(m, v, 0) + 0.1).abs() < 1e-9, "V end0 {}", end_extension(m, v, 0));
    }

    #[test]
    fn different_widths_use_other_half_width() {
        // thin H 0.1 (through) , thick V 0.3 : through extends by other(0.15), butt trims by through(0.05)
        let (doc, ids) = build(&[(0.1, 0.0, 0.0, 5.0, 0.0), (0.3, 5.0, 0.0, 5.0, 4.0)]);
        let m = doc.model();
        assert!((end_extension(m, m.wall(ids[0]).unwrap(), 1) - 0.15).abs() < 1e-9);
        assert!((end_extension(m, m.wall(ids[1]).unwrap(), 0) + 0.05).abs() < 1e-9);
    }

    #[test]
    fn collinear_no_change() {
        let (doc, ids) = build(&[(0.2, 0.0, 0.0, 4.0, 0.0), (0.2, 4.0, 0.0, 9.0, 0.0)]);
        let m = doc.model();
        assert!(end_extension(m, m.wall(ids[0]).unwrap(), 1).abs() < 1e-9);
        assert!(end_extension(m, m.wall(ids[1]).unwrap(), 0).abs() < 1e-9);
    }

    #[test]
    fn t_junction_stem_trims_host_unchanged() {
        // main 0,0->8,0 ; stem 4,0->4,5 ; stem end0 on main interior -> trim by host half 0.1
        let (doc, ids) = build(&[(0.2, 0.0, 0.0, 8.0, 0.0), (0.2, 4.0, 0.0, 4.0, 5.0)]);
        let m = doc.model();
        assert!((end_extension(m, m.wall(ids[1]).unwrap(), 0) + 0.1).abs() < 1e-9);
        // host main: its ends are free (no coincident partner) -> 0
        assert!(end_extension(m, m.wall(ids[0]).unwrap(), 0).abs() < 1e-9);
        assert!(end_extension(m, m.wall(ids[0]).unwrap(), 1).abs() < 1e-9);
    }

    #[test]
    fn floors_regenerate_from_boundary_level_and_thickness() {
        // OBSERVED (022): top of slab at level+offset, slab hangs downwards; area = plan region, volume = area*thickness,
        // perimeter = boundary perimeter.
        let mut doc = crate::new_document();
        let out = doc.transact("floors", false, |m, cx| {
            let l = ops::create_level(m, cx, "L0", 0.0)?;
            let ft = ops::create_floor_type(m, cx, "FT200", 0.2)?;
            let rect = vec![Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0), Vec2::new(6.0, 4.0), Vec2::new(0.0, 4.0)];
            let f = ops::create_floor(m, cx, ft, l, rect, 0.0, false)?;
            let tri = vec![Vec2::new(20.0, 0.0), Vec2::new(25.0, 0.0), Vec2::new(20.0, 4.0)];
            let t = ops::create_floor(m, cx, ft, l, tri, 0.0, false)?;
            Ok((l, ft, f, t))
        });
        let (l, ft, f, t) = out.value.unwrap();
        let m = doc.model();
        let s = crate::regen::floor_state(m, m.floor(f).unwrap()).unwrap();
        assert!((s.area - 24.0).abs() < 1e-9, "area {}", s.area);
        assert!((s.volume - 4.8).abs() < 1e-9);
        assert!((s.perimeter - 20.0).abs() < 1e-9);
        assert!((s.top_z - 0.0).abs() < 1e-9 && (s.bottom_z + 0.2).abs() < 1e-9, "slab hangs below the level");
        let solid = s.solid.as_ref().unwrap();
        assert_eq!((solid.vertices().len(), solid.face_count(), solid.edge_count()), (8, 6, 12));
        // triangle: area 10, prism counts 6/5/9 (matches the reference product)
        let st = crate::regen::floor_state(m, m.floor(t).unwrap()).unwrap();
        assert!((st.area - 10.0).abs() < 1e-9);
        let tsolid = st.solid.as_ref().unwrap();
        assert_eq!((tsolid.vertices().len(), tsolid.face_count(), tsolid.edge_count()), (6, 5, 9));

        // offset raises the slab; level move carries it; deleting the type deletes its floors
        doc.transact("off", false, |m, cx| ops::set_floor_height_offset(m, cx, f, 0.5));
        let s = crate::regen::floor_state(doc.model(), doc.model().floor(f).unwrap()).unwrap();
        assert!((s.top_z - 0.5).abs() < 1e-9 && (s.bottom_z - 0.3).abs() < 1e-9);
        doc.transact("lvl", false, |m, cx| ops::set_level_elevation(m, cx, l, 3.0).map(|_| ()));
        let s = crate::regen::floor_state(doc.model(), doc.model().floor(f).unwrap()).unwrap();
        assert!((s.top_z - 3.5).abs() < 1e-9, "floor follows its level");
        let o = doc.transact("del", false, |m, cx| ops::delete(m, cx, ft));
        assert!(o.committed());
        assert!(doc.model().floors.is_empty(), "deleting the floor type deletes its floors");
    }

    /// Walls whose axes are within 0.1 degrees (sine) of straight are collinear and build no target; beyond that the pair
    /// is a corner. The same limit holds for a wall folded back on the other (the test is on `|sine|`).
    #[test]
    fn spec_w05b2_collinear_means_a_tenth_of_a_degree() {
        let corner = |turn_deg: f64| {
            let t = turn_deg.to_radians();
            let (doc, ids) =
                build(&[(0.2, 0.0, 0.0, 5.0, 0.0), (0.2, 5.0, 0.0, 5.0 + 5.0 * t.cos(), 5.0 * t.sin())]);
            let m = doc.model();
            let (a, b) = (m.wall(ids[0]).unwrap(), m.wall(ids[1]).unwrap());
            (end_extension(m, a, 1), end_extension(m, b, 0), join_classes(m, a))
        };
        let (ea, eb, classes) = corner(0.05);
        assert!(ea.abs() < 1e-12 && eb.abs() < 1e-12, "{ea} {eb}");
        assert_eq!(classes, vec![JoinClass::Collinear]);
        // 0.2 degrees is a corner, inside the forced-mitre band: a mitre, no axial extension
        let (ea, _, classes) = corner(0.2);
        assert!(ea.abs() < 1e-12, "{ea}");
        assert_eq!(classes, vec![JoinClass::Mitre]);
        let (ea, _, classes) = corner(25.0);
        assert!(ea.abs() > 1e-6, "25 degrees is a butt corner: {ea}");
        assert_eq!(classes, vec![JoinClass::CornerThrough]);
        // folded back within 0.1 degrees: no join geometry at all (the engine's own convention)
        let (doc, ids) = build(&[(0.2, 0.0, 0.0, 5.0, 0.0), (0.2, 5.0, 0.0, 0.0, 0.0)]);
        let m = doc.model();
        assert!(end_extension(m, m.wall(ids[0]).unwrap(), 1).abs() < 1e-12);
    }

    // --------------------------------------------------------------------------------------- : the wall Area is the LARGER
    // of the two side faces. ---------------------------------------------------------------------------------------

    fn area_and_volume(doc: &crate::BimDocument, id: asterion_core::ElementId) -> (f64, f64) {
        let m = doc.model();
        let s = crate::regen::wall_state(m, m.wall(id).unwrap()).unwrap();
        (s.area, s.volume)
    }

    /// Its end plane meets the axis `h_q / sin(theta)` past the corner, the two side faces differ by `w / tan(theta)` and
    /// the Area uses the longer one: `L + h_q/sin + h_w/tan`. The engine's earlier axial length used `h_q/tan` for the last
    /// term, right only for equal widths.
    #[test]
    fn spec_w05d1_area_is_the_larger_side_face_for_unequal_widths_at_an_oblique_corner() {
        let (doc, ids) = build(&[(0.2, 0.0, 0.0, 5.0, 0.0), (0.6, 5.0, 0.0, 8.0, 3.0)]);
        let r2 = 2.0_f64.sqrt();
        // through wall H: hq = 0.3, hw = 0.1, theta = 45 degrees
        let mean = 5.0 + 0.3 * r2;
        let (a, v) = area_and_volume(&doc, ids[0]);
        assert!((a - (mean + 0.1) * 3.0).abs() < 1e-9, "Area {a} vs larger face {}", (mean + 0.1) * 3.0);
        // Volume is the final solid: width x mean length x height (the trapezoid)
        assert!((v - 0.2 * mean * 3.0).abs() < 1e-9, "Volume {v}");
        // butt wall D: hq = 0.1 (through half width), hw = 0.3: mean = L - hq/sin, larger = mean + hw/tan
        let l_d = 3.0 * r2;
        let mean_d = l_d - 0.1 * r2;
        let (a, v) = area_and_volume(&doc, ids[1]);
        assert!((a - (mean_d + 0.3) * 3.0).abs() < 1e-9, "Area {a}");
        assert!((v - 0.6 * mean_d * 3.0).abs() < 1e-9, "Volume {v}");
    }

    /// On the recorded corner (round 5, equal widths 0.2, 45 degrees): the larger side face coincides with the axial length
    /// the engine reported, so the Area of the through wall stays `(5 + h tan(67.5)) x 3` and the one of the butt stays
    /// `4.2012193309 x 3`.
    #[test]
    fn spec_w05d1_equal_widths_keep_the_recorded_round_5_areas() {
        let (doc, ids) = build(&[(0.2, 0.0, 0.0, 5.0, 0.0), (0.2, 5.0, 0.0, 8.0, 3.0)]);
        let t = 67.5_f64.to_radians().tan();
        let (a_h, v_h) = area_and_volume(&doc, ids[0]);
        assert!((a_h - (5.0 + 0.1 * t) * 3.0).abs() < 1e-9, "{a_h}");
        assert!((v_h - 3.0848528137424).abs() < 1e-9, "{v_h}");
        let (a_d, v_d) = area_and_volume(&doc, ids[1]);
        assert!((a_d - 12.603657992645926).abs() < 1e-9, "{a_d}");
        assert!((v_d - 2.4607315985292).abs() < 1e-9, "{v_d}");
    }

    /// (side attach, type 0: the stem's end is the host's side face): an oblique T stem has two side faces of different
    /// length and reports the larger one.
    #[test]
    fn spec_w05d1_an_oblique_t_stem_reports_its_larger_side_face() {
        // host 0.2 thick along x; stem 0.4 thick from (4,0) at 45 degrees, length 3 sqrt 2
        let (doc, ids) = build(&[(0.2, 0.0, 0.0, 8.0, 0.0), (0.4, 4.0, 0.0, 7.0, 3.0)]);
        let r2 = 2.0_f64.sqrt();
        let mean = 3.0 * r2 - 0.1 * r2; // the end meets the axis h_host / sin(theta) in
        let (a, v) = area_and_volume(&doc, ids[1]);
        assert!((a - (mean + 0.2) * 3.0).abs() < 1e-9, "Area {a}");
        assert!((v - 0.4 * mean * 3.0).abs() < 1e-9, "Volume {v}");
        // the host is unchanged
        let (ah, _) = area_and_volume(&doc, ids[0]);
        assert!((ah - 8.0 * 3.0).abs() < 1e-9);
    }

    /// The Area is a maximum of two faces, not an extension per end. Two ends slanted the same way (a parallelogram plan)
    /// have two side faces of EQUAL length, so the larger face is the mean, not the mean plus two spreads.
    #[test]
    fn spec_w05d1_a_parallelogram_wall_has_equal_side_faces() {
        // M along x, N1 and N2 parallel (1,1) at its two ends
        let (doc, ids) = build(&[
            (0.2, 0.0, 0.0, 4.0, 0.0),
            (0.2, 0.0, 0.0, 2.0, 2.0),
            (0.2, 4.0, 0.0, 6.0, 2.0),
        ]);
        let m = doc.model();
        let w = m.wall(ids[0]).unwrap();
        let spread = side_face_spread(m, w).expect("decided");
        assert!(spread.abs() < 1e-12, "parallel end planes cancel: {spread}");
        let mean = 4.0 + 2.0 * 0.1 * 2.0_f64.sqrt();
        let (a, v) = area_and_volume(&doc, ids[0]);
        assert!((a - mean * 3.0).abs() < 1e-9, "Area {a} vs {}", mean * 3.0);
        assert!((v - 0.2 * mean * 3.0).abs() < 1e-9, "Volume {v}");
        // mirrored ends (a trapezoid) add: N2 mirrored to (-1,1)
        let (doc, ids) = build(&[
            (0.2, 0.0, 0.0, 4.0, 0.0),
            (0.2, 0.0, 0.0, 2.0, 2.0),
            (0.2, 4.0, 0.0, 2.0, 2.0),
        ]);
        let m = doc.model();
        let spread = side_face_spread(m, m.wall(ids[0]).unwrap()).expect("decided");
        assert!((spread - 0.1 * 2.0).abs() < 1e-9, "mirrored ends add: {spread}");
    }

    /// FlagB: inside the forced-miter band (axes within 20 degrees of straight) the end plane is the mitre plane, the
    /// internal bisector through the control point, whatever the widths (each wall offsets both curves by its OWN half
    /// width). Three walls at a point keep the axial length.
    #[test]
    fn spec_w05d1_keeps_the_axial_length_where_no_accepted_row_decides_the_end_plane() {
        // 170 degrees: inside the band, a mitre: larger face = 5 + 0.1 tan(5 deg), Volume = 5 x 0.2 x 3
        let t = 10.0_f64.to_radians();
        let (doc, ids) = build(&[(0.2, 0.0, 0.0, 5.0, 0.0), (0.4, 5.0, 0.0, 5.0 + 5.0 * t.cos(), 5.0 * t.sin())]);
        let m = doc.model();
        let w = m.wall(ids[0]).unwrap();
        let spread = side_face_spread(m, w).expect("a mitre plane is decided");
        assert!((spread - 0.1 * 5.0_f64.to_radians().tan()).abs() < 1e-9, "{spread}");
        let s = crate::regen::wall_state(m, w).unwrap();
        assert!((s.area - (5.0 + spread) * 3.0).abs() < 1e-9 && (s.volume - 3.0).abs() < 1e-9, "{s:?}");
        // three walls at one end
        let (doc, ids) = build(&[
            (0.2, 0.0, 0.0, 5.0, 0.0),
            (0.4, 5.0, 0.0, 8.0, 3.0),
            (0.2, 5.0, 0.0, 5.0, 4.0),
        ]);
        let m = doc.model();
        // The ends of a junction of three walls are built from the join array (`crate::junctions`), not from the legacy
        // partner choice: the spread of the legacy end planes is nil
        assert_eq!(side_face_spread(m, m.wall(ids[0]).unwrap()), Some(0.0));
    }

    /// No element id comparison takes part in choosing entry 0 in any the reference product operation; the join list order
    /// is produced in the reference product. The engine's list order is a single function so that the open item is one
    /// line.
    #[test]
    fn spec_w05c1_the_engine_list_order_is_one_function() {
        let (doc, ids) = build(&[(0.2, 0.0, 0.0, 5.0, 0.0), (0.2, 5.0, 0.0, 5.0, 4.0)]);
        let m = doc.model();
        let (a, b) = (m.wall(ids[0]).unwrap(), m.wall(ids[1]).unwrap());
        assert!(join_list_precedes(a, b) && !join_list_precedes(b, a));
        // the through wall of the corner is the one that precedes
        assert!(end_extension(m, a, 1) > 0.0 && end_extension(m, b, 0) < 0.0);
    }

    #[test]
    fn disallowed_join_no_change() {
        let (mut doc, ids) = build(&[(0.2, 0.0, 0.0, 5.0, 0.0), (0.2, 5.0, 0.0, 5.0, 4.0)]);
        doc.transact("disallow", false, |m, cx| ops::set_join_allowed(m, cx, ids[0], 1, false)).value;
        let m = doc.model();
        assert!(end_extension(m, m.wall(ids[0]).unwrap(), 1).abs() < 1e-9);
    }

    /// A disallowed end and a free end (OBSERVED) list nothing.
    #[test]
    fn elements_at_join_lists_the_walls_at_the_point_in_the_engine_order() {
        let (mut doc, ids) = build(&[(0.2, 0.0, 0.0, 10.0, 0.0), (0.2, 5.0, 0.0, 5.0, 4.0), (0.2, 5.0, 0.0, 9.0, -4.0), (0.2, 20.0, 0.0, 25.0, 0.0)]);
        let m = doc.model();
        // Two stems meeting the host from OPPOSITE sides keep one array per side
        assert_eq!(elements_at_join(m, m.wall(ids[1]).unwrap(), 0), vec![ids[0], ids[1]]);
        assert_eq!(elements_at_join(m, m.wall(ids[2]).unwrap(), 0), vec![ids[0], ids[2]]);
        assert!(elements_at_join(m, m.wall(ids[1]).unwrap(), 1).is_empty());
        assert!(elements_at_join(m, m.wall(ids[3]).unwrap(), 0).is_empty());
        doc.transact("disallow", false, |m, cx| ops::set_join_allowed(m, cx, ids[1], 0, false)).value;
        assert!(elements_at_join(doc.model(), doc.model().wall(ids[1]).unwrap(), 0).is_empty());
    }

    /// `set_join_type` stores a per-end value, the default is abut, and abut clears the entry.
    #[test]
    fn join_type_is_stored_per_end_and_abut_is_the_default() {
        use crate::model::WallJoinKind;
        let (mut doc, ids) = build(&[(0.2, 0.0, 0.0, 5.0, 0.0), (0.2, 5.0, 0.0, 5.0, 4.0)]);
        assert_eq!(doc.model().join_type_at(ids[0], 1), WallJoinKind::Abut);
        doc.transact("miter", false, |m, cx| ops::set_join_type(m, cx, ids[0], 1, WallJoinKind::Miter)).value.unwrap();
        assert_eq!(doc.model().join_type_at(ids[0], 1), WallJoinKind::Miter);
        assert_eq!(doc.model().join_type_at(ids[0], 0), WallJoinKind::Abut);
        // Written on every wall whose end is at the join
        assert_eq!(doc.model().join_type_at(ids[1], 0), WallJoinKind::Miter);
        doc.transact("square", false, |m, cx| ops::set_join_type(m, cx, ids[0], 1, WallJoinKind::SquareOff)).value.unwrap();
        assert_eq!(doc.model().join_type_at(ids[0], 1), WallJoinKind::SquareOff);
        doc.transact("abut", false, |m, cx| ops::set_join_type(m, cx, ids[1], 0, WallJoinKind::Abut)).value.unwrap();
        assert!(doc.model().wall_join_types.is_empty());
        // a free end has no join info: nothing is stored (OBSERVED, `f-miter-on-a-free-end`)
        doc.transact("free", false, |m, cx| ops::set_join_type(m, cx, ids[0], 0, WallJoinKind::Miter)).value.unwrap();
        assert!(doc.model().wall_join_types.is_empty());
        assert!(doc.transact("bad", false, |m, cx| ops::set_join_type(m, cx, ids[0], 2, WallJoinKind::Miter)).value.is_none());
    }

    #[test]
    fn mitred_and_squared_corners_reproduce_round_33() {
        use crate::model::WallJoinKind;
        let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
        // 90 degree corner, mitre set on the first wall
        let (mut doc, ids) = build(&[(0.2, 0.0, 0.0, 6.0, 0.0), (0.2, 6.0, 0.0, 6.0, 4.0)]);
        doc.transact("miter", false, |m, cx| ops::set_join_type(m, cx, ids[0], 1, WallJoinKind::Miter)).value.unwrap();
        let m = doc.model();
        let a1 = crate::regen::wall_state(m, m.wall(ids[0]).unwrap()).unwrap();
        let a2 = crate::regen::wall_state(m, m.wall(ids[1]).unwrap()).unwrap();
        assert!(near(a1.volume, 3.6) && near(a1.area, 18.3) && near(a1.surface_area, 39.848528137), "{a1:?}");
        assert!(near(a2.volume, 2.4) && near(a2.area, 12.3) && near(a2.surface_area, 27.048528137));
        assert_eq!((a1.solid_edge_count, a1.solid_vertex_count), (10, 8));
        let b = a2.unjoined_solid.as_ref().unwrap().aabb();
        assert!(near(b.min.y, -0.1) && near(b.max.x, 6.1) && near(b.min.x, 5.9));
        let (doc, ids) = build(&[(0.2, 0.0, 0.0, 6.0, 0.0), (0.2, 6.0, 0.0, 9.939231012, 0.694592711)]);
        let m = doc.model();
        let c1 = crate::regen::wall_state(m, m.wall(ids[0]).unwrap()).unwrap();
        let c2 = crate::regen::wall_state(m, m.wall(ids[1]).unwrap()).unwrap();
        assert!(near(c1.volume, 3.6) && near(c1.area, 18.026246599) && near(c1.surface_area, 39.602291903), "{c1:?}");
        assert!(near(c2.volume, 2.4) && near(c2.area, 12.026246599));
        // 60 degree corner, square-off on the first wall: the first wall squared, the second butts
        let (mut doc, ids) = build(&[(0.2, 0.0, 20.0, 6.0, 20.0), (0.2, 6.0, 20.0, 8.0, 23.464101615)]);
        doc.transact("sq", false, |m, cx| ops::set_join_type(m, cx, ids[0], 1, WallJoinKind::SquareOff)).value.unwrap();
        let m = doc.model();
        let b1 = crate::regen::wall_state(m, m.wall(ids[0]).unwrap()).unwrap();
        let b2 = crate::regen::wall_state(m, m.wall(ids[1]).unwrap()).unwrap();
        assert!(near(b1.volume, 3.703923048) && near(b1.area, 18.519615242) && near(b1.surface_area, 40.708512719), "{b1:?}");
        assert!(near(b2.volume, 2.330717968), "{b2:?}");
        assert_eq!((b1.solid_edge_count, b1.solid_vertex_count, b2.solid_edge_count, b2.solid_vertex_count), (12, 10, 10, 8));
    }

    /// The square-off plane passes through the intersection of this wall's side line (offset by its OWN half width) with
    /// the other wall's side line (offset by ITS half width, on the other side), whatever the two widths.
    #[test]
    fn spec_w05f_the_square_off_point_uses_each_walls_own_half_width() {
        use crate::model::WallJoinKind;
        for (hb_wall, to) in [(0.5, (8.0, 23.464101615)), (0.1, (8.0, 23.464101615)), (0.5, (6.0, 24.0)), (0.5, (4.0, 23.464101615))] {
            let (mut doc, ids) = build(&[(0.2, 0.0, 20.0, 6.0, 20.0), (hb_wall, 6.0, 20.0, to.0, to.1)]);
            doc.transact("sq", false, |m, cx| ops::set_join_type(m, cx, ids[0], 1, WallJoinKind::SquareOff)).value.unwrap();
            let m = doc.model();
            let w = m.wall(ids[0]).unwrap();
            let CornerKind::SquareOff { ext } = corner_kind(m, w, 1) else { panic!("a square-off corner") };
            let q = m.wall(ids[1]).unwrap();
            let t_w = (w.end - w.start).normalized().unwrap();
            let t_q = (q.end - q.start).normalized().unwrap();
            let s = if t_w.cross(t_q) > 0.0 { 1.0 } else { -1.0 };
            let (h_w, h_q) = (0.1, hb_wall / 2.0);
            let p = w.end;
            let point = p + t_w.perp_ccw() * (s * h_w) + t_w * ext;
            // on the other wall's side line, at -s h_q along its left normal
            let off = (point - p).dot(t_q.perp_ccw());
            assert!((off + s * h_q).abs() < 1e-9, "widths 0.2 / {hb_wall}: {off} vs {}", -s * h_q);
        }
    }

    /// `set_wall_arc` replaces the location curve by an arc and keeps the hosted door by its station (Spec W-10b) and the
    /// ends of a corner partner (round 10 drag).
    #[test]
    fn set_wall_arc_turns_a_line_into_an_arc_and_keeps_the_door_station() {
        let (mut doc, ids) = build(&[(0.2, 0.0, 0.0, 6.0, 0.0), (0.2, 6.0, 0.0, 6.0, 4.0)]);
        let level = *doc.model().levels.keys().next().unwrap();
        let door = doc
            .transact("door", false, |m, cx| {
                let t = ops::create_hosted_type(m, cx, "D", crate::model::HostedKind::Door, 0.9, 2.1, 0.0, 0.0, 0.0, 0.0, 0.0)?;
                ops::place_hosted_instance(
                    m,
                    cx,
                    ops::NewHostedInstance { type_id: t, host: ids[0], level, host_station: 3.0, sill_height: None },
                )
            })
            .value
            .unwrap();
        let station_before = doc.model().hosted_instances[&door].host_station;
        let out = doc.transact("arc", false, |m, cx| ops::set_wall_arc(m, cx, ids[0], Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0), Vec2::new(3.0, 2.0)));
        assert!(out.committed(), "{:?}", out.failures);
        let m = doc.model();
        let w = m.wall(ids[0]).unwrap();
        assert_eq!(w.through, Some(Vec2::new(3.0, 2.0)));
        let st = crate::regen::wall_state(m, w).unwrap();
        assert!((st.length - 7.644033846118383).abs() < 1e-9, "{}", st.length);
        assert!((m.hosted_instances[&door].host_station - station_before).abs() < 1e-12);
        assert_eq!(m.hosted_instances[&door].host, ids[0]);
        // collinear points are raised at the call
        assert!(doc.transact("flat", false, |m, cx| ops::set_wall_arc(m, cx, ids[0], Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0), Vec2::new(3.0, 0.0))).value.is_none());
    }
}
