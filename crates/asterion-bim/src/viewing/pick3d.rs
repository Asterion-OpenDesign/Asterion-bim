//! Picking and highlighting in a 3D view: which elements a point or a rectangle of the screen touches.
//!
//! The 3D view draws [`crate::mesh3d::view_mesh`], an orthographic projection; this module answers the questions the shell
//! asks about that picture from the SAME mesh (the bodies of `element_geometry`, with each element's slice of the buffers
//! in [`Mesh3d::ranges`]), so what can be selected is exactly what is drawn.
//!
//! * a point selects every element whose projected triangle contains it (or whose projected edge passes within the
//!   tolerance), the one nearest the eye first;
//! * A window takes the elements whose projected vertices all lie inside the rectangle, a crossing the ones with any
//!   projected triangle or edge touching it.
//!
//! The camera is given as three unit vectors (screen right, screen up, and the direction from the scene toward the eye)
//! so no pixel arithmetic lives here: a query is in metres on the screen plane, `u = p . right`, `v = p . up`.

use crate::mesh3d::Mesh3d;
use asterion_core::ElementId;
use std::collections::{BTreeMap, BTreeSet};

/// The camera of a query: unit screen right, unit screen up and the unit vector from the scene toward the eye.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axes {
    pub right: [f64; 3],
    pub up: [f64; 3],
    pub toward_eye: [f64; 3],
}

/// What is asked of the picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Query {
    /// The elements under a point of the screen plane, within `tol` metres of a projected edge.
    Point { u: f64, v: f64, tol: f64 },
    /// The elements a rectangle of the screen plane takes: wholly inside (`crossing` false) or touched (true). `visible_only`
    /// leaves out the elements that other solids hide completely inside the rectangle (the front-most faces only).
    Box { u0: f64, v0: f64, u1: f64, v1: f64, crossing: bool, visible_only: bool },
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn vertex(f: &[f32], at: usize) -> [f64; 3] {
    [f[at] as f64, f[at + 1] as f64, f[at + 2] as f64]
}

struct P {
    u: f64,
    v: f64,
    depth: f64,
}

fn project(ax: &Axes, p: [f64; 3]) -> P {
    P { u: dot(p, ax.right), v: dot(p, ax.up), depth: dot(p, ax.toward_eye) }
}

fn cross2(o: &P, a: &P, b: &P) -> f64 {
    (a.u - o.u) * (b.v - o.v) - (a.v - o.v) * (b.u - o.u)
}

/// The depth of the triangle `abc` where it covers `(u, v)`, when it does (edges included).
fn triangle_depth_at(a: &P, b: &P, c: &P, u: f64, v: f64) -> Option<f64> {
    let p = P { u, v, depth: 0.0 };
    let area = cross2(a, b, c);
    if area.abs() < 1e-14 {
        return None; // edge-on: no area to hit (its edges are tested apart)
    }
    let (w0, w1, w2) = (cross2(&p, b, c) / area, cross2(a, &p, c) / area, cross2(a, b, &p) / area);
    let eps = -1e-9;
    (w0 >= eps && w1 >= eps && w2 >= eps).then(|| w0 * a.depth + w1 * b.depth + w2 * c.depth)
}

fn seg_distance(a: &P, b: &P, u: f64, v: f64) -> (f64, f64) {
    let (dx, dy) = (b.u - a.u, b.v - a.v);
    let l2 = dx * dx + dy * dy;
    let t = if l2 < 1e-18 { 0.0 } else { (((u - a.u) * dx + (v - a.v) * dy) / l2).clamp(0.0, 1.0) };
    let (qx, qy) = (a.u + dx * t, a.v + dy * t);
    (((qx - u).powi(2) + (qy - v).powi(2)).sqrt(), a.depth + (b.depth - a.depth) * t)
}

fn in_rect(p: &P, r: [f64; 4]) -> bool {
    p.u >= r[0] && p.u <= r[2] && p.v >= r[1] && p.v <= r[3]
}

fn seg_cross(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    let cr = |o: (f64, f64), p: (f64, f64), q: (f64, f64)| (p.0 - o.0) * (q.1 - o.1) - (p.1 - o.1) * (q.0 - o.0);
    let (d1, d2, d3, d4) = (cr(c, d, a), cr(c, d, b), cr(a, b, c), cr(a, b, d));
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// Whether the projected segment `a b` meets the rectangle (an end inside, or a crossing of one of its sides).
fn seg_touches_rect(a: &P, b: &P, r: [f64; 4]) -> bool {
    if in_rect(a, r) || in_rect(b, r) {
        return true;
    }
    let c = [(r[0], r[1]), (r[2], r[1]), (r[2], r[3]), (r[0], r[3])];
    (0..4).any(|i| seg_cross((a.u, a.v), (b.u, b.v), c[i], c[(i + 1) % 4]))
}

/// The elements of `mesh` the query takes: `(element, depth)` with the depth toward the eye, nearest to the eye first for a
/// point (a larger depth is nearer the eye) and by id for a box.
pub fn pick(mesh: &Mesh3d, ax: &Axes, q: &Query) -> Vec<(ElementId, f64)> {
    // an element's several ranges (a wall, the openings cut in it) are folded: it is taken when any range is
    let mut hit: BTreeMap<ElementId, (bool, bool, f64)> = BTreeMap::new(); // (any inside, all inside, nearest depth)
    for r in &mesh.ranges {
        let tris = &mesh.triangles[r.tris.0..r.tris.1];
        let edges = &mesh.edges[r.edges.0..r.edges.1];
        if tris.is_empty() && edges.is_empty() {
            continue;
        }
        let entry = hit.entry(r.element).or_insert((false, true, f64::NEG_INFINITY));
        match *q {
            Query::Point { u, v, tol } => {
                for t in tris.chunks_exact(27) {
                    let (a, b, c) = (project(ax, vertex(t, 0)), project(ax, vertex(t, 9)), project(ax, vertex(t, 18)));
                    if u < a.u.min(b.u).min(c.u) - tol || u > a.u.max(b.u).max(c.u) + tol || v < a.v.min(b.v).min(c.v) - tol || v > a.v.max(b.v).max(c.v) + tol {
                        continue;
                    }
                    if let Some(d) = triangle_depth_at(&a, &b, &c, u, v) {
                        entry.0 = true;
                        entry.2 = entry.2.max(d);
                    } else {
                        for (p, s) in [(&a, &b), (&b, &c), (&c, &a)] {
                            let (dist, d) = seg_distance(p, s, u, v);
                            if dist <= tol {
                                entry.0 = true;
                                entry.2 = entry.2.max(d);
                            }
                        }
                    }
                }
                for e in edges.chunks_exact(6) {
                    let (a, b) = (project(ax, vertex(e, 0)), project(ax, vertex(e, 3)));
                    let (dist, d) = seg_distance(&a, &b, u, v);
                    if dist <= tol {
                        entry.0 = true;
                        entry.2 = entry.2.max(d);
                    }
                }
            }
            Query::Box { u0, v0, u1, v1, crossing, .. } => {
                let rect = [u0.min(u1), v0.min(v1), u0.max(u1), v0.max(v1)];
                let centre = (0.5 * (rect[0] + rect[2]), 0.5 * (rect[1] + rect[3]));
                for t in tris.chunks_exact(27) {
                    let (a, b, c) = (project(ax, vertex(t, 0)), project(ax, vertex(t, 9)), project(ax, vertex(t, 18)));
                    let all = in_rect(&a, rect) && in_rect(&b, rect) && in_rect(&c, rect);
                    entry.1 &= all;
                    if crossing && !entry.0 {
                        entry.0 = seg_touches_rect(&a, &b, rect)
                            || seg_touches_rect(&b, &c, rect)
                            || seg_touches_rect(&c, &a, rect)
                            || triangle_depth_at(&a, &b, &c, centre.0, centre.1).is_some();
                    }
                }
                for e in edges.chunks_exact(6) {
                    let (a, b) = (project(ax, vertex(e, 0)), project(ax, vertex(e, 3)));
                    entry.1 &= in_rect(&a, rect) && in_rect(&b, rect);
                    if crossing && !entry.0 {
                        entry.0 = seg_touches_rect(&a, &b, rect);
                    }
                }
                if !crossing {
                    entry.0 = true; // "any" is not the window's test; the verdict below uses `all`
                }
            }
        }
    }
    let mut out: Vec<(ElementId, f64)> = hit
        .into_iter()
        .filter(|(_, (any, all, _))| match *q {
            Query::Point { .. } => *any,
            Query::Box { crossing: true, .. } => *any,
            Query::Box { crossing: false, .. } => *any && *all,
        })
        .map(|(id, (_, _, d))| (id, d))
        .collect();
    if matches!(q, Query::Point { .. }) {
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    }
    if let Query::Box { u0, v0, u1, v1, visible_only: true, .. } = *q {
        let rect = [u0.min(u1), v0.min(v1), u0.max(u1), v0.max(v1)];
        let wanted: BTreeSet<ElementId> = out.iter().map(|h| h.0).collect();
        let seen = visible_in_rect(mesh, ax, rect, &wanted);
        out.retain(|h| seen.contains(&h.0));
    }
    out
}

/// Cells per side of the occlusion grid. The cost is the triangles that reach the rectangle times the cells each covers, never
/// the pixels of a screen.
const OCCLUSION_CELLS: usize = 128;
/// Depth tolerance (metres) of "in front of or level with": faces that meet in one plane do not hide each other.
const OCCLUSION_EPS: f64 = 1e-3;

/// Which of `wanted` show at least one face that no other solid hides, inside `rect` (screen-plane metres).
///
/// The front-most faces are found with a coarse depth grid over the rectangle. A cell is an OCCLUDER only where the nearest faces
/// cover all four of its corners; its depth is the smallest of the four, so a face is called hidden only when it is behind the
/// occluder over the whole cell (a thin element, a sliver, an edge-on face is never hidden by a cell that is not covered; an opening
/// smaller than a cell does not let the grid see through it). Each element of `wanted` is then sampled: at the corners and centre of its triangles that lie in the rectangle and at the
/// centre of every cell its triangles cover; one sample level with or in front of its cell's occluder makes the element visible.
/// An element with no triangles (lines only) has nothing to hide behind and is visible. What drops out is a solid element behind
/// a solid one: the pipe behind a wall, the floor under a floor.
pub fn visible_in_rect(mesh: &Mesh3d, ax: &Axes, rect: [f64; 4], wanted: &BTreeSet<ElementId>) -> BTreeSet<ElementId> {
    let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
    if !(w > 0.0 && h > 0.0) || !w.is_finite() || !h.is_finite() {
        return wanted.clone();
    }
    let cs = w.max(h) / OCCLUSION_CELLS as f64;
    let nx = ((w / cs).ceil() as usize).clamp(1, OCCLUSION_CELLS);
    let ny = ((h / cs).ceil() as usize).clamp(1, OCCLUSION_CELLS);
    let mut occ = vec![f64::NEG_INFINITY; nx * ny];
    let corner = |ix: usize, iy: usize| (rect[0] + ix as f64 * cs, rect[1] + iy as f64 * cs);
    let covers = |a: &P, b: &P, c: &P, area: f64, u: f64, v: f64| -> Option<f64> {
        let p = P { u, v, depth: 0.0 };
        let (w0, w1, w2) = (cross2(&p, b, c) / area, cross2(a, &p, c) / area, cross2(a, b, &p) / area);
        let eps = 1e-9;
        (w0 >= -eps && w1 >= -eps && w2 >= -eps).then(|| w0 * a.depth + w1 * b.depth + w2 * c.depth)
    };
    // the cells a projected triangle can reach: its bounding box on the grid (None when it misses the rectangle)
    let span = |a: &P, b: &P, c: &P| -> Option<(usize, usize, usize, usize)> {
        let (lu, hu) = (a.u.min(b.u).min(c.u), a.u.max(b.u).max(c.u));
        let (lv, hv) = (a.v.min(b.v).min(c.v), a.v.max(b.v).max(c.v));
        if hu < rect[0] || lu > rect[2] || hv < rect[1] || lv > rect[3] {
            return None;
        }
        let x0 = ((lu - rect[0]) / cs).floor().max(0.0) as usize;
        let y0 = ((lv - rect[1]) / cs).floor().max(0.0) as usize;
        let x1 = (((hu - rect[0]) / cs).floor().max(0.0) as usize).min(nx - 1);
        let y1 = (((hv - rect[1]) / cs).floor().max(0.0) as usize).min(ny - 1);
        (x0 < nx && y0 < ny).then_some((x0, y0, x1, y1))
    };
    // pass 1: a depth buffer SAMPLED AT THE GRID'S VERTICES (the nearest face over each vertex), then a cell's occluder depth is the
    // smallest of its four corners, and only when all four are covered. Sampling the vertices rather than testing each triangle for
    // covering a whole cell keeps the union of neighbouring triangles (the two halves of a slab's face meet inside a cell and each
    // covers only part of it); a cell with an uncovered corner hides nothing.
    let stride = nx + 1;
    let mut vz = vec![f64::NEG_INFINITY; stride * (ny + 1)];
    for r in &mesh.ranges {
        for t in mesh.triangles[r.tris.0..r.tris.1].chunks_exact(27) {
            let (a, b, c) = (project(ax, vertex(t, 0)), project(ax, vertex(t, 9)), project(ax, vertex(t, 18)));
            let area = cross2(&a, &b, &c);
            if area.abs() < 1e-14 {
                continue;
            }
            let Some((x0, y0, x1, y1)) = span(&a, &b, &c) else { continue };
            for iy in y0..=(y1 + 1).min(ny) {
                for ix in x0..=(x1 + 1).min(nx) {
                    let (u, v) = corner(ix, iy);
                    if let Some(d) = covers(&a, &b, &c, area, u, v) {
                        let z = &mut vz[iy * stride + ix];
                        *z = z.max(d);
                    }
                }
            }
        }
    }
    for iy in 0..ny {
        for ix in 0..nx {
            let corners = [vz[iy * stride + ix], vz[iy * stride + ix + 1], vz[(iy + 1) * stride + ix], vz[(iy + 1) * stride + ix + 1]];
            occ[iy * nx + ix] = if corners.iter().all(|d| d.is_finite()) { corners.iter().copied().fold(f64::INFINITY, f64::min) } else { f64::NEG_INFINITY };
        }
    }
    // pass 2: the samples of each wanted element
    let mut seen: BTreeSet<ElementId> = BTreeSet::new();
    for r in &mesh.ranges {
        if !wanted.contains(&r.element) || seen.contains(&r.element) {
            continue;
        }
        let tris = &mesh.triangles[r.tris.0..r.tris.1];
        if tris.is_empty() {
            if !mesh.edges[r.edges.0..r.edges.1].is_empty() {
                seen.insert(r.element); // lines only: nothing in front of them can be a solid face
            }
            continue;
        }
        let cell_of = |u: f64, v: f64| {
            let ix = (((u - rect[0]) / cs).floor().max(0.0) as usize).min(nx - 1);
            let iy = (((v - rect[1]) / cs).floor().max(0.0) as usize).min(ny - 1);
            iy * nx + ix
        };
        'tri: for t in tris.chunks_exact(27) {
            let (a, b, c) = (project(ax, vertex(t, 0)), project(ax, vertex(t, 9)), project(ax, vertex(t, 18)));
            let area = cross2(&a, &b, &c);
            let Some((x0, y0, x1, y1)) = span(&a, &b, &c) else { continue };
            // the corners and the centre of the triangle, where they are in the rectangle
            let centroid = P { u: (a.u + b.u + c.u) / 3.0, v: (a.v + b.v + c.v) / 3.0, depth: (a.depth + b.depth + c.depth) / 3.0 };
            for p in [&a, &b, &c, &centroid] {
                if p.u >= rect[0] && p.u <= rect[2] && p.v >= rect[1] && p.v <= rect[3] && p.depth >= occ[cell_of(p.u, p.v)] - OCCLUSION_EPS {
                    seen.insert(r.element);
                    break 'tri;
                }
            }
            if area.abs() < 1e-14 {
                continue;
            }
            // the cells whose centre the triangle covers
            for iy in y0..=y1 {
                for ix in x0..=x1 {
                    let (u, v) = (rect[0] + (ix as f64 + 0.5) * cs, rect[1] + (iy as f64 + 0.5) * cs);
                    if let Some(d) = covers(&a, &b, &c, area, u, v) {
                        if d >= occ[iy * nx + ix] - OCCLUSION_EPS {
                            seen.insert(r.element);
                            break 'tri;
                        }
                    }
                }
            }
        }
    }
    seen
}

/// The edges (`x0 y0 z0 x1 y1 z1` per segment) of the given elements in `mesh`: what a 3D view highlights when they are
/// selected or pre-selected.
pub fn edges_of(mesh: &Mesh3d, ids: &[ElementId]) -> Vec<f32> {
    let mut out = Vec::new();
    for r in mesh.ranges.iter().filter(|r| ids.contains(&r.element)) {
        out.extend_from_slice(&mesh.edges[r.edges.0..r.edges.1]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh3d::{push_prism, view_mesh};
    use crate::model::ViewKind;
    use crate::{ops, CompoundStructure, Layer, LayerFunction};
    use asterion_geom::Vec2;

    fn top_down() -> Axes {
        Axes { right: [1.0, 0.0, 0.0], up: [0.0, 1.0, 0.0], toward_eye: [0.0, 0.0, 1.0] }
    }

    fn two_walls() -> (crate::BimDocument, ElementId, ElementId, ElementId) {
        let mut doc = crate::new_document();
        let out = doc.transact("fx", false, |m, cx| {
            let l0 = ops::create_level(m, cx, "L0", 0.0)?;
            let wt = ops::create_wall_type(m, cx, "W200", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
            let mut ids = Vec::new();
            for (a, b) in [((0.0, 0.0), (5.0, 0.0)), ((0.0, 10.0), (5.0, 10.0))] {
                ids.push(ops::create_wall(
                    m,
                    cx,
                    ops::NewWall { wall_type: wt, level: l0, start: Vec2::new(a.0, a.1), end: Vec2::new(b.0, b.1), height: 3.0, base_offset: 0.0, flip: false, structural: false },
                )?);
            }
            let three = ops::create_view(m, cx, "{3D}", ViewKind::ThreeD, None, None)?;
            Ok((ids[0], ids[1], three))
        });
        let (a, b, three) = out.value.expect("committed");
        (doc, a, b, three)
    }

    #[test]
    fn a_point_takes_the_wall_drawn_under_it_and_a_window_the_walls_inside_it() {
        let (doc, a, b, three) = two_walls();
        let m = doc.model();
        let mesh = view_mesh(m, m.view(three).unwrap()).unwrap();
        assert!(mesh.ranges.iter().any(|r| r.element == a) && mesh.ranges.iter().any(|r| r.element == b));
        let ax = top_down();
        // looking down on the first wall (x 0..5, y -0.1..0.1)
        let at = pick(&mesh, &ax, &Query::Point { u: 2.5, v: 0.0, tol: 0.01 });
        assert_eq!(at.iter().map(|h| h.0).collect::<Vec<_>>(), vec![a]);
        assert!(pick(&mesh, &ax, &Query::Point { u: 2.5, v: 5.0, tol: 0.01 }).is_empty(), "nothing between the walls");
        // a window around the first wall only; a crossing that reaches the second wall as well
        let win = pick(&mesh, &ax, &Query::Box { u0: -1.0, v0: -1.0, u1: 6.0, v1: 1.0, crossing: false, visible_only: false });
        assert_eq!(win.iter().map(|h| h.0).collect::<Vec<_>>(), vec![a]);
        let partial = pick(&mesh, &ax, &Query::Box { u0: 2.0, v0: -1.0, u1: 3.0, v1: 1.0, crossing: false, visible_only: false });
        assert!(partial.is_empty(), "a window takes whole elements only");
        let crossing = pick(&mesh, &ax, &Query::Box { u0: 2.0, v0: -1.0, u1: 3.0, v1: 11.0, crossing: true, visible_only: false });
        assert_eq!(crossing.iter().map(|h| h.0).collect::<Vec<_>>(), vec![a, b]);
        // nothing hides these two walls from each other: the visible-only box takes the same
        let seen = pick(&mesh, &ax, &Query::Box { u0: 2.0, v0: -1.0, u1: 3.0, v1: 11.0, crossing: true, visible_only: true });
        assert_eq!(seen.iter().map(|h| h.0).collect::<Vec<_>>(), vec![a, b]);
    }

    fn prism(mesh: &mut Mesh3d, id: u64, x0: f64, y0: f64, x1: f64, y1: f64, z0: f64, z1: f64) {
        let (t0, e0) = (mesh.triangles.len(), mesh.edges.len());
        push_prism(mesh, &[Vec2::new(x0, y0), Vec2::new(x1, y0), Vec2::new(x1, y1), Vec2::new(x0, y1)], z0, z1, [0.5; 3]);
        mesh.ranges.push(crate::mesh3d::ElementRange { element: ElementId(id), tris: (t0, mesh.triangles.len()), edges: (e0, mesh.edges.len()) });
    }

    fn ids(v: &[(ElementId, f64)]) -> Vec<u64> {
        v.iter().map(|h| h.0 .0).collect()
    }

    #[test]
    fn a_box_with_visible_only_leaves_out_what_a_solid_hides_and_keeps_what_shows() {
        // seen from above: a floor slab (1) under a wide slab (2); a thin wall (3) standing on the wide slab; a small block (4)
        // beside everything; a lone pipe-like sliver (5) under the wide slab (thinner than a grid cell)
        let mut mesh = Mesh3d::default();
        prism(&mut mesh, 1, 0.0, 0.0, 4.0, 4.0, 0.0, 0.3);
        prism(&mut mesh, 2, -1.0, -1.0, 5.0, 5.0, 1.0, 1.3);
        prism(&mut mesh, 3, 0.0, 2.0, 4.0, 2.1, 1.3, 4.0);
        prism(&mut mesh, 4, 6.0, 0.0, 7.0, 1.0, 0.0, 0.5);
        prism(&mut mesh, 5, 1.0, 1.0, 3.0, 1.02, 0.4, 0.5);
        let ax = top_down();
        let q = |crossing: bool, visible_only: bool| Query::Box { u0: -2.0, v0: -2.0, u1: 8.0, v1: 6.0, crossing, visible_only };
        assert_eq!(ids(&pick(&mesh, &ax, &q(true, false))), vec![1, 2, 3, 4, 5], "every element in the box without occlusion");
        assert_eq!(ids(&pick(&mesh, &ax, &q(false, false))), vec![1, 2, 3, 4, 5]);
        // the slab under the wide slab and the sliver under it are hidden; the wall on top and the block beside stay
        assert_eq!(ids(&pick(&mesh, &ax, &q(true, true))), vec![2, 3, 4], "crossing keeps the front-most only");
        assert_eq!(ids(&pick(&mesh, &ax, &q(false, true))), vec![2, 3, 4], "a window keeps the front-most only");
        // a thin wall under nothing is never lost to the coarse grid
        let mut lone = Mesh3d::default();
        prism(&mut lone, 9, 0.0, 0.0, 50.0, 0.1, 0.0, 3.0);
        prism(&mut lone, 8, 0.0, 5.0, 50.0, 5.1, 0.0, 3.0);
        let wide = Query::Box { u0: -5.0, v0: -5.0, u1: 60.0, v1: 10.0, crossing: false, visible_only: true };
        assert_eq!(ids(&pick(&lone, &ax, &wide)), vec![8, 9]);
        // seen from the side the stacked slabs are all in front of one another along the view: from the front (+y toward the eye)
        // the block 4 at y 0..1 is behind the wall 3 at y 2..2.1 only where they overlap in x (they do not: 6..7 vs 0..4)
        let front = Axes { right: [1.0, 0.0, 0.0], up: [0.0, 0.0, 1.0], toward_eye: [0.0, -1.0, 0.0] };
        let front_box = Query::Box { u0: -2.0, v0: -1.0, u1: 8.0, v1: 5.0, crossing: true, visible_only: true };
        let seen = ids(&pick(&mesh, &front, &front_box));
        assert!(seen.contains(&4) && seen.contains(&2), "{seen:?}");
        // the wall 3 stands behind slab 2's front edge only below z 1.3; its upper part shows
        assert!(seen.contains(&3), "{seen:?}");
    }

    #[test]
    fn an_element_with_lines_only_and_a_degenerate_box_are_never_dropped() {
        let mut mesh = Mesh3d::default();
        prism(&mut mesh, 1, 0.0, 0.0, 2.0, 2.0, 0.0, 1.0);
        let all: BTreeSet<ElementId> = [ElementId(1)].into_iter().collect();
        assert_eq!(visible_in_rect(&mesh, &top_down(), [1.0, 1.0, 1.0, 1.0], &all), all, "a zero-size box answers everything it was given");
        assert!(visible_in_rect(&mesh, &top_down(), [f64::NAN, 0.0, 1.0, 1.0], &all) == all);
        mesh.edges.extend_from_slice(&[0.0, 0.0, 5.0, 1.0, 1.0, 5.0]);
        mesh.ranges.push(crate::mesh3d::ElementRange { element: ElementId(2), tris: (mesh.triangles.len(), mesh.triangles.len()), edges: (mesh.edges.len() - 6, mesh.edges.len()) });
        let both: BTreeSet<ElementId> = [ElementId(1), ElementId(2)].into_iter().collect();
        assert_eq!(visible_in_rect(&mesh, &top_down(), [-1.0, -1.0, 3.0, 3.0], &both), both, "a line element has nothing to hide behind");
    }

    #[test]
    fn the_nearest_element_to_the_eye_comes_first_and_the_edges_belong_to_the_element() {
        let mut mesh = Mesh3d::default();
        // two stacked prisms under one point: the upper one is nearer a camera looking down
        push_prism(&mut mesh, &[Vec2::new(0.0, 0.0), Vec2::new(2.0, 0.0), Vec2::new(2.0, 2.0), Vec2::new(0.0, 2.0)], 0.0, 1.0, [0.5; 3]);
        mesh.ranges.push(crate::mesh3d::ElementRange { element: ElementId(1), tris: (0, mesh.triangles.len()), edges: (0, mesh.edges.len()) });
        let (t0, e0) = (mesh.triangles.len(), mesh.edges.len());
        push_prism(&mut mesh, &[Vec2::new(0.0, 0.0), Vec2::new(2.0, 0.0), Vec2::new(2.0, 2.0), Vec2::new(0.0, 2.0)], 2.0, 3.0, [0.5; 3]);
        mesh.ranges.push(crate::mesh3d::ElementRange { element: ElementId(2), tris: (t0, mesh.triangles.len()), edges: (e0, mesh.edges.len()) });
        let hits = pick(&mesh, &top_down(), &Query::Point { u: 1.0, v: 1.0, tol: 0.0 });
        assert_eq!(hits.iter().map(|h| h.0 .0).collect::<Vec<_>>(), vec![2, 1]);
        assert!((hits[0].1 - 3.0).abs() < 1e-6, "depth of the upper face {}", hits[0].1);
        assert_eq!(edges_of(&mesh, &[ElementId(2)]).len(), mesh.edges.len() - e0);
        assert_eq!(edges_of(&mesh, &[]).len(), 0);
    }
}
