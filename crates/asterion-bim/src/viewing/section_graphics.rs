//! The line work of a section or elevation view, in the view's own frame (metres along the view's X = right and Y = up).
//!
//! What a section-like view LISTS is measured (round 30, `docviews::in_section`: the elements whose box meets the crop box,
//! the levels that cross its height, never a grid) and what its graphics hide is measured (rounds 30 / 30b,
//! `docviews::hidden_by_graphics`).
//!
//! * the section plane is the near face of the crop box (local z = 0; the box extends toward -Z, away from the viewer);
//!   an element whose box straddles the plane is CUT, one that lies wholly behind it (within the far clip) is PROJECTED,
//!   one wholly in front of it is not drawn;
//! * a cut wall is the rectangle between the plane's crossings of its plan body and its base / top, less the openings the
//!   plane passes through (the sill and head pieces of a window stay, a door leaves a gap to the base); a cut slab is the
//!   rectangle between the crossings of its boundary and its bottom / top;
//! * a projected wall or slab is the rectangle of its extent along X and its height; a projected wall shows its openings
//!   as rectangles; a level is a horizontal line across the crop box;
//! * no hidden-line removal between elements (every projected outline is drawn), no line styles, no detail level;
//! * the crop box clips the line work when it is active.
//!
//! Weight classes follow the plan's (`asterion_view_linework`): 2 = cut, 1 = projection, 0.5 = datum.

use crate::docviews::{self, DocView, V3};
use crate::element_geometry::{self, Body};
use crate::model::BimModel;
use asterion_core::{ElementId, Failure};
use asterion_geom::Vec2;

pub const CLASS_CUT: f32 = 2.0;
pub const CLASS_PROJECTION: f32 = 1.0;
pub const CLASS_DATUM: f32 = 0.5;

/// One segment of the view's line work, in the view's frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seg {
    pub element: ElementId,
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub class: f32,
}

/// An axis-aligned rectangle `[x0, y0, x1, y1]` in the view's frame.
pub type Rect = [f64; 4];

/// What one element draws: a rectangle, with the openings in it - or, for a body whose section or silhouette is not a
/// rectangle (a sloped roof, a sloped slab), the polygon.
#[derive(Clone, Debug, PartialEq)]
pub struct Figure {
    pub element: ElementId,
    pub cut: bool,
    /// The box of the figure (the whole figure when `poly` is `None`).
    pub rect: Rect,
    /// `(the opening's element, its rectangle)`: subtracted from a cut figure, outlined on a projected one.
    pub holes: Vec<(ElementId, Rect)>,
    /// The outline when it is not `rect`: counter-clockwise points of the view's frame.
    pub poly: Option<Vec<[f64; 2]>>,
}

const EPS: f64 = 1e-9;

fn local(v: &DocView, p: V3) -> V3 {
    docviews::to_local(v, p)
}

fn rect_of(xs: impl Iterator<Item = f64>, y0: f64, y1: f64) -> Option<Rect> {
    let (lo, hi) = xs.fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), x| (l.min(x), h.max(x)));
    (lo.is_finite() && hi > lo + EPS && y1 > y0 + EPS).then_some([lo, y0.min(y1), hi, y0.max(y1)])
}

/// The local X of the crossings of a plan polygon with the plane z = 0 (the vertical plane of the view).
fn plane_crossings(v: &DocView, poly: &[Vec2], z_ref: f64) -> Vec<f64> {
    let n = poly.len();
    let mut xs = Vec::new();
    for i in 0..n {
        let (p, q) = (poly[i], poly[(i + 1) % n]);
        let (lp, lq) = (local(v, [p.x, p.y, z_ref]), local(v, [q.x, q.y, z_ref]));
        if lp[2].abs() < EPS {
            xs.push(lp[0]);
        }
        if (lp[2] < -EPS && lq[2] > EPS) || (lp[2] > EPS && lq[2] < -EPS) {
            let t = lp[2] / (lp[2] - lq[2]);
            xs.push(lp[0] + (lq[0] - lp[0]) * t);
        }
    }
    xs
}

/// Whether the span of local z of a body is cut by the plane, behind it, or in front of it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Cut,
    Behind,
    Front,
}

fn side_of(zs: impl Iterator<Item = f64>) -> Side {
    let (lo, hi) = zs.fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), z| (l.min(z), h.max(z)));
    if lo < -EPS && hi > EPS {
        Side::Cut
    } else if hi <= EPS {
        Side::Behind
    } else {
        Side::Front
    }
}

/// The figures of a prism (a plan polygon between two elevations): its cut or projected rectangle.
fn prism_figure(v: &DocView, id: ElementId, poly: &[Vec2], z0: f64, z1: f64) -> Option<Figure> {
    if poly.len() < 3 || z1 <= z0 + EPS {
        return None;
    }
    let corners: Vec<V3> = poly.iter().flat_map(|p| [local(v, [p.x, p.y, z0]), local(v, [p.x, p.y, z1])]).collect();
    let (y0, y1) = (local(v, [poly[0].x, poly[0].y, z0])[1], local(v, [poly[0].x, poly[0].y, z1])[1]);
    match side_of(corners.iter().map(|c| c[2])) {
        Side::Front => None,
        Side::Cut => rect_of(plane_crossings(v, poly, z0).into_iter(), y0, y1).map(|rect| Figure { element: id, cut: true, rect, holes: Vec::new(), poly: None }),
        Side::Behind => rect_of(corners.iter().map(|c| c[0]), y0, y1).map(|rect| Figure { element: id, cut: false, rect, holes: Vec::new(), poly: None }),
    }
}

fn bbox_of(pts: &[[f64; 2]]) -> Rect {
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in pts {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    [lo[0], lo[1], hi[0], hi[1]]
}

/// Convex hull (Andrew's monotone chain), counter-clockwise, collinear points dropped.
fn hull(mut pts: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    pts.dedup_by(|a, b| (a[0] - b[0]).abs() < EPS && (a[1] - b[1]).abs() < EPS);
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut lower: Vec<[f64; 2]> = Vec::new();
    for p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], *p) <= EPS {
            lower.pop();
        }
        lower.push(*p);
    }
    let mut upper: Vec<[f64; 2]> = Vec::new();
    for p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], *p) <= EPS {
            upper.pop();
        }
        upper.push(*p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// The figures of ANY body in a vertical view (a section or an elevation): the cut, the projection behind the plane, or
/// nothing in front of it. A plain prism without holes takes the rectangle path of [`prism_figure`] (a wall's); a sloped
/// or holed body is cut by the plane into the polygons its planes bound (`Cut`), or drawn as the silhouette of its corners
/// (`Behind`, exact for a convex body - the roof pieces are - and for a prism).
pub fn body_figures(v: &DocView, id: ElementId, b: &Body) -> Vec<Figure> {
    let side = side_of(b.corners().into_iter().map(|c| local(v, c)[2]));
    body_figures_as(v, id, b, side)
}

/// The figures of every body of ONE element: the element as a whole is cut by the plane, behind it or in front of it, and each
/// of its bodies is drawn that way. Deciding per body would lose the cut of an element whose bodies meet exactly on the plane
/// (a gable roof cut along its ridge: one slope touches the plane from the front, the other from behind, and neither is cut).
pub fn element_figures(v: &DocView, id: ElementId, bodies: &[Body]) -> Vec<Figure> {
    let side = side_of(bodies.iter().flat_map(|b| b.corners()).map(|c| local(v, c)[2]));
    bodies
        .iter()
        .flat_map(|b| {
            // A body that touches the plane takes the element's side (the seam case above). A body clear of the plane keeps its own:
            // the panels of a curtain wall that stand wholly behind the plane that cuts the others are projected, not lost.
            let zs: Vec<f64> = b.corners().into_iter().map(|c| local(v, c)[2]).collect();
            let (lo, hi) = zs.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), z| (l.min(*z), h.max(*z)));
            let touches = lo.abs() <= EPS || hi.abs() <= EPS || (lo < -EPS && hi > EPS);
            body_figures_as(v, id, b, if touches { side } else { side_of(zs.into_iter()) })
        })
        .collect()
}

fn body_figures_as(v: &DocView, id: ElementId, b: &Body, side: Side) -> Vec<Figure> {
    let plain = b.is_prism() && b.holes.is_empty();
    if plain {
        let (y0, y1) = (local(v, [b.outer[0].x, b.outer[0].y, b.bottom.c0])[1], local(v, [b.outer[0].x, b.outer[0].y, b.top.c0])[1]);
        return match side {
            Side::Front => Vec::new(),
            Side::Cut => rect_of(plane_crossings(v, &b.outer, b.bottom.c0).into_iter(), y0, y1)
                .map(|rect| Figure { element: id, cut: true, rect, holes: Vec::new(), poly: None })
                .into_iter()
                .collect(),
            Side::Behind => {
                let xs = b.outer.iter().map(|p| local(v, [p.x, p.y, b.bottom.c0])[0]);
                rect_of(xs, y0, y1).map(|rect| Figure { element: id, cut: false, rect, holes: Vec::new(), poly: None }).into_iter().collect()
            }
        };
    }
    let corners: Vec<V3> = b.corners().into_iter().map(|c| local(v, c)).collect();
    match side {
        Side::Front => Vec::new(),
        Side::Behind => {
            let h = hull(corners.iter().map(|c| [c[0], c[1]]).collect());
            if h.len() < 3 || (bbox_of(&h)[2] - bbox_of(&h)[0]) < EPS || (bbox_of(&h)[3] - bbox_of(&h)[1]) < EPS {
                return Vec::new();
            }
            vec![Figure { element: id, cut: false, rect: bbox_of(&h), holes: Vec::new(), poly: Some(h) }]
        }
        Side::Cut => {
            // the plane's crossings of every loop, paired along the plane (even-odd: a hole splits the cut)
            let mut crossings: Vec<(f64, f64, f64)> = Vec::new(); // (local x, bottom y, top y)
            for l in std::iter::once(&b.outer).chain(b.holes.iter()) {
                let n = l.len();
                for i in 0..n {
                    let (p, q) = (l[i], l[(i + 1) % n]);
                    let (lp, lq) = (local(v, [p.x, p.y, 0.0]), local(v, [q.x, q.y, 0.0]));
                    if (lp[2] < 0.0) == (lq[2] < 0.0) {
                        continue;
                    }
                    let t = lp[2] / (lp[2] - lq[2]);
                    let at = Vec2::new(p.x + (q.x - p.x) * t, p.y + (q.y - p.y) * t);
                    let (lo, hi) = (local(v, [at.x, at.y, b.bottom.at(at)]), local(v, [at.x, at.y, b.top.at(at)]));
                    crossings.push((lo[0], lo[1], hi[1]));
                }
            }
            crossings.sort_by(|a, c| a.0.total_cmp(&c.0));
            let mut out = Vec::new();
            for pair in crossings.chunks_exact(2) {
                let (a, c) = (pair[0], pair[1]);
                if c.0 - a.0 < EPS {
                    continue;
                }
                let poly = vec![[a.0, a.1], [c.0, c.1], [c.0, c.2], [a.0, a.2]];
                out.push(Figure { element: id, cut: true, rect: bbox_of(&poly), holes: Vec::new(), poly: Some(poly) });
            }
            out
        }
    }
}

/// The figures of the polyhedral parts of one element (stair steps, stringers, rails, balusters) in a vertical view: a part
/// the plane crosses is CUT (the loops of the plane's section, in the view's frame), a part wholly behind it is PROJECTED
/// (its outline seen from the viewer), a part in front of it is not drawn.
pub fn part_figures(v: &DocView, id: ElementId, parts: &[element_geometry::Part]) -> Vec<Figure> {
    let mut out = Vec::new();
    for part in parts {
        let zs: Vec<f64> = part.solid.vertices().iter().map(|c| local(v, *c)[2]).collect();
        let loops = match side_of(zs.into_iter()) {
            Side::Front => continue,
            Side::Cut => part.solid.slice(v.origin, v.z).into_iter().map(|l| (true, l)).collect::<Vec<_>>(),
            Side::Behind => part.solid.silhouette(v.z).into_iter().map(|l| (false, l)).collect::<Vec<_>>(),
        };
        for (cut, l) in loops {
            let mut poly: Vec<[f64; 2]> = l.iter().map(|p| {
                let q = local(v, *p);
                [q[0], q[1]]
            }).collect();
            let n = poly.len();
            if n < 3 {
                continue;
            }
            let area: f64 = (0..n).map(|i| poly[i][0] * poly[(i + 1) % n][1] - poly[(i + 1) % n][0] * poly[i][1]).sum::<f64>() * 0.5;
            if area.abs() < 1e-12 {
                continue;
            }
            if area < 0.0 {
                poly.reverse();
            }
            out.push(Figure { element: id, cut, rect: bbox_of(&poly), holes: Vec::new(), poly: Some(poly) });
        }
    }
    out
}

/// A cheap test that never rejects what [`docviews::in_section`] accepts: whether the plan footprint of a wall, a slab or a column can
/// reach the crop box of a VERTICAL view along the view's two horizontal axes (local x and the depth). The collector regenerates every
/// element it tests (a wall's body, a slab's geometry), which on a project of a thousand walls is most of a section's cost, and almost
/// every wall is far from the section line. Any other element, an arc wall and an oblique view pass untested.
fn near_view(m: &BimModel, v: &DocView, id: ElementId) -> bool {
    if v.x[2].abs() > 1e-9 || v.z[2].abs() > 1e-9 {
        return true;
    }
    let (pts, margin): (Vec<Vec2>, f64) = if let Some(w) = m.walls.get(&id) {
        if w.through.is_some() {
            return true;
        }
        (vec![w.start, w.end], 1.5)
    } else if let Some(f) = m.floors.get(&id) {
        (f.boundary.clone(), 0.5)
    } else if let Some(c) = m.columns.get(&id) {
        (vec![c.location], 2.0)
    } else {
        return true;
    };
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for p in pts {
        let l = local(v, [p.x, p.y, 0.0]);
        for (k, a) in [0usize, 2].into_iter().enumerate() {
            lo[k] = lo[k].min(l[a]);
            hi[k] = hi[k].max(l[a]);
        }
    }
    [0usize, 2].into_iter().enumerate().all(|(k, a)| hi[k] + margin >= v.crop_min[a] - 1e-6 && lo[k] - margin <= v.crop_max[a] + 1e-6)
}

/// The figures a section-like view draws: walls with their openings, slabs. Levels are lines (see [`section_segments`]).
pub fn section_figures(m: &BimModel, view: ElementId) -> Result<Vec<Figure>, Failure> {
    section_figures_with(m, view, true)
}

/// [`section_figures`] with the cheap prefilter ([`near_view`]) on or off; the answers are the same (a test compares them).
pub fn section_figures_with(m: &BimModel, view: ElementId, prefilter: bool) -> Result<Vec<Figure>, Failure> {
    let v = docviews::doc_view(m, view).ok_or_else(|| Failure::error(crate::spec::codes::NOT_FOUND, "No such view.").on(&[view]))?;
    let v = &*v;
    let drawn = |id: ElementId| (!prefilter || near_view(m, v, id)) && docviews::in_section(m, v, id) && !docviews::hidden_by_graphics(m, view, id);
    let mut out = Vec::new();
    for w in m.walls.values() {
        // a curtain wall is its glass and mullions (`element_geometry`), drawn with the other bodies below
        if m.is_curtain_wall(w.id) || (prefilter && !near_view(m, v, w.id)) {
            continue;
        }
        // the wall is regenerated once: its state gives the box the collector tests and the direction its openings follow
        let Ok(ws) = crate::regen::wall_state(m, w) else { continue };
        if !docviews::in_section_with(m, v, w.id, docviews::wall_box(&ws)) || docviews::hidden_by_graphics(m, view, w.id) {
            continue;
        }
        let Ok((footprint, base_z, top_z)) = crate::regen::wall_view_body(m, w) else { continue };
        let Some(mut fig) = prism_figure(v, w.id, &footprint, base_z, top_z) else { continue };
        let start = [w.start.x, w.start.y, 0.0];
        let dir = [ws.direction.x, ws.direction.y, 0.0];
        let along = dir[0] * v.z[0] + dir[1] * v.z[1] + dir[2] * v.z[2];
        let at = |u: f64, z: f64| local(v, [start[0] + dir[0] * u, start[1] + dir[1] * u, z]);
        for i in m.hosted_instances.values().filter(|i| i.host == w.id) {
            let Ok(st) = crate::regen::hosted_instance_state(m, i) else { continue };
            let (y_sill, y_head) = (at(st.rect.u_lo, st.sill_z)[1], at(st.rect.u_lo, st.head_z)[1]);
            let hole = if fig.cut {
                if along.abs() < 1e-9 {
                    // the wall runs along the plane: every opening the plane passes through is cut
                    rect_of([at(st.rect.u_lo, st.sill_z)[0], at(st.rect.u_hi, st.sill_z)[0]].into_iter(), y_sill, y_head)
                } else {
                    // the plane crosses the wall at one station: an opening there splits the cut
                    let u = ((v.origin[0] - start[0]) * v.z[0] + (v.origin[1] - start[1]) * v.z[1] + (v.origin[2] - start[2]) * v.z[2]) / along;
                    (u >= st.rect.u_lo - EPS && u <= st.rect.u_hi + EPS).then_some([fig.rect[0], y_sill.min(y_head), fig.rect[2], y_sill.max(y_head)])
                }
            } else if docviews::hidden_by_graphics(m, view, i.id) {
                None
            } else {
                rect_of([at(st.rect.u_lo, st.sill_z)[0], at(st.rect.u_hi, st.sill_z)[0]].into_iter(), y_sill, y_head)
            };
            if let Some(h) = hole {
                let h = [h[0].max(fig.rect[0]), h[1].max(fig.rect[1]), h[2].min(fig.rect[2]), h[3].min(fig.rect[3])];
                if h[2] > h[0] + EPS && h[3] > h[1] + EPS {
                    fig.holes.push((i.id, h));
                }
            }
        }
        out.push(fig);
    }
    // every other element that has a body: slabs, ceilings and roofs (sloped ones as polygons), columns, curtain
    // walls, wall sweeps, toposolids - the SAME bodies the 3D mesh is built from
    let others: Vec<ElementId> = m
        .floors
        .keys()
        .chain(m.columns.keys())
        .chain(m.walls.keys().filter(|id| m.is_curtain_wall(**id)))
        .chain(m.annot.sweeps.iter().filter(|(_, s)| s.kind == crate::annot::SweepKind::Sweep).map(|(k, _)| k))
        .chain(m.docs.toposolids.keys())
        .copied()
        .collect();
    // a curtain wall loses its glass or its mullions with the categories of the view (`view_categories::part_filter`)
    let parts = crate::view_categories::part_filter(m, view);
    for id in others {
        if !drawn(id) {
            continue;
        }
        out.extend(element_figures(v, id, element_geometry::geometry_of_in(m, id, parts).solids()));
    }
    // stairs and railings: polyhedral parts, drawn under the id of the component that owns them (the same ids as the 3D mesh)
    for s in m.stairs.stairs.keys() {
        for (owner, p) in crate::stair_solids::stairs_parts(m, *s) {
            if drawn(owner) {
                out.extend(part_figures(v, owner, &p));
            }
        }
    }
    for r in m.stairs.railings.keys().chain(m.stairs_scene.path_railings.keys()) {
        if drawn(*r) {
            out.extend(part_figures(v, *r, element_geometry::geometry_of(m, *r).parts()));
        }
    }
    Ok(out)
}

/// The boundary of `outer` less `holes` as axis-aligned segments (the holes are clamped to `outer`).
pub fn rect_minus(outer: Rect, holes: &[Rect]) -> Vec<[f64; 4]> {
    let mut xs = vec![outer[0], outer[2]];
    let mut ys = vec![outer[1], outer[3]];
    for h in holes {
        xs.extend([h[0].clamp(outer[0], outer[2]), h[2].clamp(outer[0], outer[2])]);
        ys.extend([h[1].clamp(outer[1], outer[3]), h[3].clamp(outer[1], outer[3])]);
    }
    let dedup = |mut v: Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        v.dedup_by(|a, b| (*a - *b).abs() < EPS);
        v
    };
    let (xs, ys) = (dedup(xs), dedup(ys));
    let inside = |i: usize, j: usize| -> bool {
        if i >= xs.len() - 1 || j >= ys.len() - 1 {
            return false;
        }
        let (cx, cy) = ((xs[i] + xs[i + 1]) * 0.5, (ys[j] + ys[j + 1]) * 0.5);
        !holes.iter().any(|h| cx > h[0] && cx < h[2] && cy > h[1] && cy < h[3])
    };
    let mut out = Vec::new();
    // vertical edges at x = xs[i] between the cell on the left (i - 1) and on the right (i)
    for i in 0..xs.len() {
        let mut run: Option<(f64, f64)> = None;
        for j in 0..ys.len() - 1 {
            let left = i > 0 && inside(i - 1, j);
            let right = inside(i, j);
            if left != right {
                run = Some(match run {
                    Some((a, _)) => (a, ys[j + 1]),
                    None => (ys[j], ys[j + 1]),
                });
            } else if let Some((a, b)) = run.take() {
                out.push([xs[i], a, xs[i], b]);
            }
        }
        if let Some((a, b)) = run {
            out.push([xs[i], a, xs[i], b]);
        }
    }
    for j in 0..ys.len() {
        let mut run: Option<(f64, f64)> = None;
        for i in 0..xs.len() - 1 {
            let below = j > 0 && inside(i, j - 1);
            let above = inside(i, j);
            if below != above {
                run = Some(match run {
                    Some((a, _)) => (a, xs[i + 1]),
                    None => (xs[i], xs[i + 1]),
                });
            } else if let Some((a, b)) = run.take() {
                out.push([a, ys[j], b, ys[j]]);
            }
        }
        if let Some((a, b)) = run {
            out.push([a, ys[j], b, ys[j]]);
        }
    }
    out
}

pub fn rect_edges(r: Rect) -> [[f64; 4]; 4] {
    [[r[0], r[1], r[2], r[1]], [r[2], r[1], r[2], r[3]], [r[2], r[3], r[0], r[3]], [r[0], r[3], r[0], r[1]]]
}

/// The segments of a figure: a cut figure's outline less its holes; a projected figure's outline and its holes' outlines.
pub fn figure_segments(f: &Figure) -> Vec<Seg> {
    let class = if f.cut { CLASS_CUT } else { CLASS_PROJECTION };
    let mut out = Vec::new();
    if let Some(poly) = &f.poly {
        let n = poly.len();
        out.extend((0..n).map(|i| Seg { element: f.element, a: poly[i], b: poly[(i + 1) % n], class }));
        return out;
    }
    if f.cut {
        let holes: Vec<Rect> = f.holes.iter().map(|h| h.1).collect();
        out.extend(rect_minus(f.rect, &holes).into_iter().map(|s| Seg { element: f.element, a: [s[0], s[1]], b: [s[2], s[3]], class }));
    } else {
        out.extend(rect_edges(f.rect).into_iter().map(|s| Seg { element: f.element, a: [s[0], s[1]], b: [s[2], s[3]], class }));
        for (id, h) in &f.holes {
            out.extend(rect_edges(*h).into_iter().map(|s| Seg { element: *id, a: [s[0], s[1]], b: [s[2], s[3]], class }));
        }
    }
    out
}

/// Liang-Barsky: the part of a segment inside a rectangle.
pub fn clip_segment(a: [f64; 2], b: [f64; 2], r: Rect) -> Option<([f64; 2], [f64; 2])> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (p, q) in [(-dx, a[0] - r[0]), (dx, r[2] - a[0]), (-dy, a[1] - r[1]), (dy, r[3] - a[1])] {
        if p.abs() < EPS {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let t = q / p;
        if p < 0.0 {
            t0 = t0.max(t);
        } else {
            t1 = t1.min(t);
        }
        if t0 > t1 {
            return None;
        }
    }
    Some(([a[0] + dx * t0, a[1] + dy * t0], [a[0] + dx * t1, a[1] + dy * t1]))
}

/// The level lines of the view (the levels its collector lists, unless the Levels category is hidden).
pub fn level_segments(m: &BimModel, view: ElementId, v: &DocView) -> Vec<Seg> {
    let mut out = Vec::new();
    for l in m.levels.values() {
        if !docviews::in_section(m, v, l.id) || docviews::hidden_by_graphics(m, view, l.id) {
            continue;
        }
        let y = local(v, [v.origin[0], v.origin[1], l.elevation])[1];
        out.push(Seg { element: l.id, a: [v.crop_min[0], y], b: [v.crop_max[0], y], class: CLASS_DATUM });
    }
    out
}

pub fn crop_rect(v: &DocView) -> Option<Rect> {
    v.crop_active.then_some([v.crop_min[0], v.crop_min[1], v.crop_max[0], v.crop_max[1]])
}

fn clipped(segs: Vec<Seg>, crop: Option<Rect>) -> Vec<Seg> {
    match crop {
        None => segs,
        Some(r) => segs.into_iter().filter_map(|s| clip_segment(s.a, s.b, r).map(|(a, b)| Seg { a, b, ..s })).collect(),
    }
}

/// The line work of a section-like view (not a plan callout: that is its parent plan's, clipped).
pub fn section_segments(m: &BimModel, view: ElementId) -> Result<Vec<Seg>, Failure> {
    segments_of(m, view, &section_figures(m, view)?)
}

/// [`section_segments`] from figures already collected ([`section_figures`]): a caller that draws, picks and outlines the same view
/// between two edits collects the figures once (the collector regenerates every element it tests).
pub fn segments_of(m: &BimModel, view: ElementId, figures: &[Figure]) -> Result<Vec<Seg>, Failure> {
    let v = docviews::doc_view(m, view).ok_or_else(|| Failure::error(crate::spec::codes::NOT_FOUND, "No such view.").on(&[view]))?;
    let v = &*v;
    let mut segs: Vec<Seg> = figures.iter().flat_map(figure_segments).collect();
    segs.extend(level_segments(m, view, v));
    Ok(clipped(segs, crop_rect(v)))
}

fn in_rect(p: [f64; 2], r: Rect, tol: f64) -> bool {
    p[0] >= r[0] - tol && p[0] <= r[2] + tol && p[1] >= r[1] - tol && p[1] <= r[3] + tol
}

/// The element at a point of the view: an opening before its wall, a cut figure before a projected one, then a level
/// line within `tol`. `None` when nothing is there.
pub fn section_pick(m: &BimModel, view: ElementId, p: [f64; 2], tol: f64) -> Option<ElementId> {
    pick_in(m, view, &section_figures(m, view).ok()?, p, tol)
}

/// [`section_pick`] over figures already collected.
pub fn pick_in(m: &BimModel, view: ElementId, figs: &[Figure], p: [f64; 2], tol: f64) -> Option<ElementId> {
    let v = docviews::doc_view(m, view)?;
    let v = &*v;
    if let Some(r) = crop_rect(v) {
        if !in_rect(p, r, tol) {
            return None;
        }
    }
    for cut in [true, false] {
        for f in figs.iter().filter(|f| f.cut == cut) {
            if let Some((id, _)) = f.holes.iter().find(|(_, h)| in_rect(p, *h, tol)) {
                if !f.cut || m.hosted_instances.contains_key(id) {
                    return Some(*id);
                }
            }
            if in_rect(p, f.rect, tol) {
                return Some(f.element);
            }
        }
    }
    level_segments(m, view, v).into_iter().find(|s| (p[1] - s.a[1]).abs() <= tol && p[0] >= s.a[0].min(s.b[0]) - tol && p[0] <= s.a[0].max(s.b[0]) + tol).map(|s| s.element)
}

/// The outline of one element in the view (weight class 3, as the plan's selection outline), empty when it is not drawn.
pub fn section_outline(m: &BimModel, view: ElementId, element: ElementId) -> Vec<Seg> {
    match section_figures(m, view) {
        Ok(figs) => outline_in(m, view, &figs, element),
        Err(_) => Vec::new(),
    }
}

/// [`section_outline`] over figures already collected.
pub fn outline_in(m: &BimModel, view: ElementId, figs: &[Figure], element: ElementId) -> Vec<Seg> {
    let Some(v) = docviews::doc_view(m, view) else { return Vec::new() };
    let v = &*v;
    let mut out = Vec::new();
    for f in figs {
        if f.element == element {
            out.extend(figure_segments(f).into_iter().map(|s| Seg { class: 3.0, ..s }));
        }
        for (id, h) in &f.holes {
            if *id == element {
                out.extend(rect_edges(*h).into_iter().map(|s| Seg { element, a: [s[0], s[1]], b: [s[2], s[3]], class: 3.0 }));
            }
        }
    }
    if out.is_empty() {
        out.extend(level_segments(m, view, v).into_iter().filter(|s| s.element == element).map(|s| Seg { class: 3.0, ..s }));
    }
    clipped(out, crop_rect(v))
}

/// The segments as the renderer's flat buffer: `[x0, y0, x1, y1, class]` per segment.
pub fn flatten(segs: &[Seg]) -> Vec<f32> {
    let mut out = Vec::with_capacity(segs.len() * 5);
    for s in segs {
        out.extend_from_slice(&[s.a[0] as f32, s.a[1] as f32, s.b[0] as f32, s.b[1] as f32, s.class]);
    }
    out
}

/// [`flatten`] with the graphic overrides of `view` (`override_style`): a segment of an element the view overrides takes the weight
/// class that carries the override.
pub fn flatten_in(m: &BimModel, view: ElementId, segs: &[Seg]) -> Vec<f32> {
    let Some(styler) = crate::override_style::ViewStyler::new(m, view) else { return flatten(segs) };
    let mut out = Vec::with_capacity(segs.len() * 5);
    for s in segs {
        out.extend_from_slice(&[s.a[0] as f32, s.a[1] as f32, s.b[0] as f32, s.b[1] as f32, styler.class_for(s.element, s.class)]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CompoundStructure, HostedKind, Layer, LayerFunction};
    use crate::ops;

    fn rooms_document() -> (crate::BimDocument, ElementId, ElementId, ElementId) {
        let mut d = crate::new_document();
        let (l0, t, w_south, w_north, door) = d
            .transact("fx", false, |m, cx| {
                let l0 = ops::create_level(m, cx, "L0", 0.0)?;
                let t = ops::create_wall_type(m, cx, "W200", CompoundStructure { layers: vec![Layer { width: 0.2, function: LayerFunction::Structure }], exterior_shell_layers: 0, interior_shell_layers: 0 })?;
                let mk = |m: &mut BimModel, cx: &mut asterion_core::txn::TxnContext<'_>, a: [f64; 2], b: [f64; 2]| {
                    ops::create_wall(m, cx, ops::NewWall { wall_type: t, level: l0, start: Vec2::new(a[0], a[1]), end: Vec2::new(b[0], b[1]), height: 3.0, base_offset: 0.0, flip: false, structural: false })
                };
                let s = mk(m, cx, [0.0, 0.0], [6.0, 0.0])?;
                let n = mk(m, cx, [0.0, 4.0], [6.0, 4.0])?;
                let dt = ops::create_hosted_type(m, cx, "D", HostedKind::Door, 1.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0)?;
                let door = ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: dt, host: s, level: l0, host_station: 3.0, sill_height: None })?;
                Ok((l0, t, s, n, door))
            })
            .value
            .unwrap();
        let _ = (l0, t);
        (d, w_south, w_north, door)
    }

    /// A section whose plane is y = 0 with the viewer south of it: origin (−1, 0, 0), X = +X, Y = up, Z = −Y. The south
    /// wall's centre line lies in the plane (cut along its length); the north wall lies 4 m behind it (projected).
    #[test]
    fn a_section_through_a_wall_cuts_it_and_projects_the_wall_behind() {
        let (mut d, w_south, w_north, door) = rooms_document();
        let s = d
            .transact("s", false, |m, cx| docviews::add_section_view(m, cx, Some("S"), [-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0], [0.0, 0.0, -10.0], [8.0, 5.0, 0.0]))
            .value
            .unwrap();
        let figs = section_figures(d.model(), s).unwrap();
        let south = figs.iter().find(|f| f.element == w_south).expect("the south wall is cut");
        assert!(south.cut);
        assert!((south.rect[1] - 0.0).abs() < 1e-9 && (south.rect[3] - 3.0).abs() < 1e-9, "{:?}", south.rect);
        assert!(south.rect[0] >= 0.9 && south.rect[2] <= 7.1, "the cut runs along the wall: {:?}", south.rect);
        // the door is cut with the wall: a hole from the base to its head
        let hole = south.holes.iter().find(|h| h.0 == door).expect("the door is in the cut");
        assert!((hole.1[1] - 0.0).abs() < 1e-9 && (hole.1[3] - 2.0).abs() < 1e-9 && (hole.1[0] - 3.5).abs() < 1e-9 && (hole.1[2] - 4.5).abs() < 1e-9, "{:?}", hole.1);
        let north = figs.iter().find(|f| f.element == w_north).expect("the north wall is behind the plane");
        assert!(!north.cut);
        let segs = section_segments(d.model(), s).unwrap();
        assert!(segs.iter().any(|g| g.class == CLASS_CUT) && segs.iter().any(|g| g.class == CLASS_PROJECTION) && segs.iter().any(|g| g.class == CLASS_DATUM));
        // the pick: the door inside its opening, the wall beside it, the north wall where only it is
        assert_eq!(section_pick(d.model(), s, [4.0, 1.0], 0.01), Some(door));
        assert_eq!(section_pick(d.model(), s, [2.0, 1.0], 0.01), Some(w_south));
        assert_eq!(section_pick(d.model(), s, [2.0, 2.9], 0.01), Some(w_south));
        assert!(!section_outline(d.model(), s, w_north).is_empty());
        // the Walls category hidden: nothing of the walls, the level line stays
        d.transact("h", false, |m, _| docviews::set_category_hidden_in_view(m, s, "Walls", true)).value.unwrap();
        let segs = section_segments(d.model(), s).unwrap();
        assert!(segs.iter().all(|g| g.class == CLASS_DATUM), "{segs:?}");
        assert_eq!(section_pick(d.model(), s, [2.0, 1.0], 0.01), None);
    }

    #[test]
    fn a_rectangle_less_a_door_is_a_u_shape() {
        let segs = rect_minus([0.0, 0.0, 6.0, 3.0], &[[2.0, 0.0, 3.0, 2.0]]);
        // 8 edges: bottom-left, left, top, right, bottom-right, and the three sides of the opening
        assert_eq!(segs.len(), 8, "{segs:?}");
        let total: f64 = segs.iter().map(|s| ((s[2] - s[0]).powi(2) + (s[3] - s[1]).powi(2)).sqrt()).sum();
        assert!((total - (6.0 + 3.0 + 6.0 + 3.0 - 1.0 + 2.0 + 1.0 + 2.0)).abs() < 1e-9, "{total}");
        assert_eq!(rect_minus([0.0, 0.0, 6.0, 3.0], &[]).len(), 4);
        // a window hole: 4 outer + 4 inner edges
        assert_eq!(rect_minus([0.0, 0.0, 6.0, 3.0], &[[2.0, 1.0, 3.0, 2.5]]).len(), 8);
    }

    /// An element made of several bodies (a curtain wall's panels, a roof's pieces) that the plane cuts through some of: the ones
    /// wholly behind it are projected, the ones it crosses are cut, the ones in front are not drawn.
    #[test]
    fn bodies_of_one_element_keep_their_own_side_unless_they_touch_the_plane() {
        use crate::element_geometry::Body;
        let (mut d, ..) = rooms_document();
        let s = d
            .transact("s", false, |m, cx| docviews::add_section_view(m, cx, Some("S"), [-1.0, 2.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0], [0.0, 0.0, -10.0], [8.0, 5.0, 0.0]))
            .value
            .unwrap();
        let v = &d.model().docs.views[&s];
        let b = |y0: f64, y1: f64| Body::prism(&[Vec2::new(0.0, y0), Vec2::new(1.0, y0), Vec2::new(1.0, y1), Vec2::new(0.0, y1)], 0.0, 2.0);
        // the plane is y = 2: one body crosses it (1.5..2.5), one is behind it (3..4), one in front (0..1)
        let figs = element_figures(v, ElementId(7), &[b(1.5, 2.5), b(3.0, 4.0), b(0.0, 1.0)]);
        assert_eq!((figs.iter().filter(|f| f.cut).count(), figs.iter().filter(|f| !f.cut).count()), (1, 1), "{figs:?}");
    }

    #[test]
    fn a_segment_is_clipped_to_the_crop() {
        assert_eq!(clip_segment([-1.0, 1.0], [5.0, 1.0], [0.0, 0.0, 4.0, 2.0]), Some(([0.0, 1.0], [4.0, 1.0])));
        assert_eq!(clip_segment([-1.0, 5.0], [5.0, 5.0], [0.0, 0.0, 4.0, 2.0]), None);
    }
}
