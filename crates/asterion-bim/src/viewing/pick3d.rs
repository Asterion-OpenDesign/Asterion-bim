//! Picking and highlighting in a 3D view: which elements a point or a rectangle of the screen touches.
//!
//! The 3D view draws [`crate::mesh3d::view_mesh`], an orthographic projection; this module answers the questions the shell
//! asks about that picture from the SAME mesh (the bodies of `element_geometry`, with each element's slice of the buffers
//! in [`Mesh3d::ranges`]), so what can be selected is exactly what is drawn.
//!
//! * a point selects every element whose projected triangle contains it (or whose projected edge passes within the
//!   tolerance), the one nearest the eye first;
//! * a window takes the elements whose projected vertices all lie inside the rectangle, a crossing the ones with any
//!   projected triangle or edge touching it; occlusion by other solids is not evaluated (the picture's depth test is the
//!   renderer's).
//!
//! The camera is given as three unit vectors (screen right, screen up, and the direction from the scene toward the eye)
//! so no pixel arithmetic lives here: a query is in metres on the screen plane, `u = p . right`, `v = p . up`.

use crate::mesh3d::Mesh3d;
use asterion_core::ElementId;
use std::collections::BTreeMap;

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
    /// The elements a rectangle of the screen plane takes: wholly inside (`crossing` false) or touched (true).
    Box { u0: f64, v0: f64, u1: f64, v1: f64, crossing: bool },
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
            Query::Box { u0, v0, u1, v1, crossing } => {
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
    out
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
        let win = pick(&mesh, &ax, &Query::Box { u0: -1.0, v0: -1.0, u1: 6.0, v1: 1.0, crossing: false });
        assert_eq!(win.iter().map(|h| h.0).collect::<Vec<_>>(), vec![a]);
        let partial = pick(&mesh, &ax, &Query::Box { u0: 2.0, v0: -1.0, u1: 3.0, v1: 1.0, crossing: false });
        assert!(partial.is_empty(), "a window takes whole elements only");
        let crossing = pick(&mesh, &ax, &Query::Box { u0: 2.0, v0: -1.0, u1: 3.0, v1: 11.0, crossing: true });
        assert_eq!(crossing.iter().map(|h| h.0).collect::<Vec<_>>(), vec![a, b]);
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
