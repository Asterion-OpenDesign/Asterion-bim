//! Shape editing of floors and roofs (the reference product's `SlabShapeEditor`): shape vertices that lift or lower the
//! slab's reference surface, split lines, and the quantities of the solid that results.
//!
//! * `Enable` makes the corner vertices of every loop of the sketch (holes included) with offset 0. `DrawPoint` adds an
//!   interior vertex at the given height (absolute: 3.0 is the level), `ModifySubElement(vertex, offset)` sets the offset of
//!   the vertex nearest to the point in plan, `DrawSplitLine` adds creases between existing vertices (one crease per
//!   stretch between vertices on the line), `ResetSlabShape` removes the whole shape (the editor is disabled again);
//! * the REFERENCE SURFACE: with the corner vertices of a four-sided sketch only (no hole, no split), the surface is the
//!   BILINEAR patch through them (`top_z_at (1, 1) = 3 - 0.3 u v` exactly, Area 24.032468 = its area); in every other case it
//!   is a TRIANGULATION of the vertices (a fan around one interior vertex; for a rectangle with a rectangular hole whose
//!   inner corner is lowered the triangulation of least surface area, 3 planar faces, Area 20.1139055);
//! * the SOLID of a floor is its reference surface (the top) and the same surface shifted DOWN by the type thickness, with
//!   vertical sides (volume = plan area x t; surface area = 2 x area + perimeter x t); of a roof, the reference surface
//!   (the underside) and the same surface shifted UP. The Area parameter is the area of the reference surface and the
//!   Volume parameter `Area x t` (Spec R-01c: constant thickness, non-constant offset), except with a VARIABLE LAYER, where
//!   the bottom stays flat at the nominal thickness under the level and the Volume is the solid's.
//! * counts: planar faces of the top (coplanar neighbouring triangles are ONE face; a bilinear patch is one face) + the same
//!   number for the bottom (one when it is flat) + one side per sketch edge; vertices are those of both surfaces (a corner
//!   whose thickness is nil is one vertex); `E = V + F - 2 + 2 holes`.
//!
//! ENGINE CHOICES (named): the tie-break between equally good triangulations (least surface area), the sign of a split line
//! through other vertices, the refusal of shape editing on a sloped slab and of a slope on a shaped roof (OBSERVED for the
//! roof: "slope on a shape-edited roof" is not committed), slabs with arc edges (not modelled).

use crate::model::{BimModel, Slab, SlabAnchor, SlabRole};
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use asterion_geom::Vec2;
use serde::{Deserialize, Serialize};

pub mod codes {
    pub const NOT_A_SLAB: &str = "slab_shape.not_a_slab";
    pub const NOT_ENABLED: &str = "slab_shape.not_enabled";
    pub const NO_VERTEX: &str = "slab_shape.no_vertex";
    pub const OUTSIDE: &str = "slab_shape.outside";
    pub const UNSUPPORTED: &str = "slab_shape.unsupported";
    pub const SLOPE_ON_SHAPED: &str = "slab_shape.slope_on_shaped";
}

fn bad(code: &str, msg: impl Into<String>) -> Failure {
    Failure::error(code, msg)
}

/// One shape vertex: its place in plan and its offset above the slab's reference plane.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShapeVertex {
    pub p: Vec2,
    pub offset: f64,
    /// A corner of the sketch (made by `Enable`) as against a drawn interior point.
    pub corner: bool,
}

/// The shape of one slab.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SlabShape {
    pub vertices: Vec<ShapeVertex>,
    /// User-drawn creases, as pairs of vertex indices.
    pub splits: Vec<(usize, usize)>,
}

// ---------------------------------------------------------------------------------------------- ops

fn slab_of<'a>(m: &'a BimModel, host: ElementId) -> Result<&'a Slab, Failure> {
    let f = m.floors.get(&host).ok_or_else(|| bad(codes::NOT_A_SLAB, "Only a floor or a roof can be shape edited.").on(&[host]))?;
    if !matches!(f.role, SlabRole::Floor | SlabRole::Roof) {
        return Err(bad(codes::NOT_A_SLAB, "Only a floor or a roof can be shape edited.").on(&[host]));
    }
    Ok(f)
}

fn plain_straight(f: &Slab) -> Result<(), Failure> {
    let sloped = f.slope_arrow.is_some() || f.slope_edge.is_some() || f.segments.iter().any(|s| s.defines_slope && s.slope != 0.0);
    if sloped {
        return Err(bad(codes::UNSUPPORTED, "A sloped slab cannot be shape edited (ENGINE CHOICE, OQ-SLAB-SHAPE).").on(&[f.id]));
    }
    if f.has_arcs() {
        return Err(bad(codes::UNSUPPORTED, "A slab with arc edges is not shape edited by the engine (OQ-SLAB-SHAPE).").on(&[f.id]));
    }
    Ok(())
}

/// `SlabShapeEditor.Enable`: the corners of every loop become shape vertices at offset 0. Enabling again changes nothing.
pub fn enable(m: &mut BimModel, _cx: &mut TxnContext<'_>, host: ElementId) -> Result<(), Failure> {
    let f = slab_of(m, host)?;
    plain_straight(f)?;
    if m.slab_shapes.contains_key(&host) {
        return Ok(());
    }
    let mut vertices: Vec<ShapeVertex> = f.boundary.iter().map(|&p| ShapeVertex { p, offset: 0.0, corner: true }).collect();
    for l in &f.loops {
        vertices.extend(l.iter().map(|&p| ShapeVertex { p, offset: 0.0, corner: true }));
    }
    m.slab_shapes.insert(host, SlabShape { vertices, splits: Vec::new() });
    Ok(())
}

fn shape_mut<'a>(m: &'a mut BimModel, host: ElementId) -> Result<&'a mut SlabShape, Failure> {
    slab_of(m, host)?;
    m.slab_shapes.get_mut(&host).ok_or_else(|| bad(codes::NOT_ENABLED, "Shape editing is not enabled on this element.").on(&[host]))
}

fn reference_z(m: &BimModel, f: &Slab) -> f64 {
    m.levels.get(&f.level).map_or(0.0, |l| l.elevation) + f.height_offset
}

fn nearest(shape: &SlabShape, p: Vec2) -> Option<usize> {
    shape
        .vertices
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1.p - p).length().total_cmp(&(b.1.p - p).length()))
        .map(|(i, _)| i)
}

/// `SlabShapeEditor.DrawPoint(xyz)`: an interior vertex at the absolute height `z`.
pub fn add_point(m: &mut BimModel, _cx: &mut TxnContext<'_>, host: ElementId, point: [f64; 3]) -> Result<(), Failure> {
    let f = slab_of(m, host)?;
    let base = reference_z(m, f);
    let p = Vec2::new(point[0], point[1]);
    let inside = inside_material(f, p);
    let shape = shape_mut(m, host)?;
    if shape.vertices.iter().any(|v| (v.p - p).length() < 1e-9) {
        return Ok(());
    }
    if !inside {
        return Err(bad(codes::OUTSIDE, "The point is not on the slab.").on(&[host]));
    }
    shape.vertices.push(ShapeVertex { p, offset: point[2] - base, corner: false });
    Ok(())
}

/// `SlabShapeEditor.ModifySubElement(vertex, offset)` on the vertex nearest to `at` in plan.
pub fn offset_vertex(m: &mut BimModel, _cx: &mut TxnContext<'_>, host: ElementId, at: [f64; 2], offset: f64) -> Result<(), Failure> {
    let shape = shape_mut(m, host)?;
    let i = nearest(shape, Vec2::new(at[0], at[1])).ok_or_else(|| bad(codes::NO_VERTEX, "The element has no shape vertex.").on(&[host]))?;
    shape.vertices[i].offset = offset;
    Ok(())
}

/// `SlabShapeEditor.DrawSplitLine(a, b)` between the vertices nearest to the two points.
pub fn add_split_line(m: &mut BimModel, _cx: &mut TxnContext<'_>, host: ElementId, start: [f64; 2], end: [f64; 2]) -> Result<(), Failure> {
    let shape = shape_mut(m, host)?;
    let a = nearest(shape, Vec2::new(start[0], start[1])).ok_or_else(|| bad(codes::NO_VERTEX, "The element has no shape vertex.").on(&[host]))?;
    let b = nearest(shape, Vec2::new(end[0], end[1])).ok_or_else(|| bad(codes::NO_VERTEX, "The element has no shape vertex.").on(&[host]))?;
    if a == b {
        return Err(bad(codes::NO_VERTEX, "A split line needs two different vertices.").on(&[host]));
    }
    let (pa, pb) = (shape.vertices[a].p, shape.vertices[b].p);
    // the vertices on the line, in order: one crease for each stretch between two of them
    let dir = pb - pa;
    let len2 = dir.dot(dir);
    let mut on: Vec<(f64, usize)> = shape
        .vertices
        .iter()
        .enumerate()
        .filter_map(|(i, v)| {
            let t = (v.p - pa).dot(dir) / len2;
            let off = (v.p - pa).cross(dir).abs() / len2.sqrt();
            (off < 1e-9 && t > -1e-9 && t < 1.0 + 1e-9).then_some((t, i))
        })
        .collect();
    on.sort_by(|x, y| x.0.total_cmp(&y.0));
    for w in on.windows(2) {
        let (i, j) = (w[0].1.min(w[1].1), w[0].1.max(w[1].1));
        if !shape.splits.contains(&(i, j)) {
            shape.splits.push((i, j));
        }
    }
    Ok(())
}

/// `SlabShapeEditor.ResetSlabShape`: the shape is removed and the editor is disabled again.
pub fn reset(m: &mut BimModel, _cx: &mut TxnContext<'_>, host: ElementId) -> Result<(), Failure> {
    slab_of(m, host)?;
    m.slab_shapes.remove(&host);
    Ok(())
}

/// Whether a plan point lies on the material of the slab (inside the outer loop, outside the holes).
pub fn inside_material(f: &Slab, p: Vec2) -> bool {
    use asterion_geom::region::polygon_contains_point;
    polygon_contains_point(&f.boundary, p) && !f.loops.iter().any(|l| polygon_contains_point(l, p))
}

// ------------------------------------------------------------------------------------- the surface

/// The reference surface of a shaped slab.
#[derive(Clone, Debug)]
pub enum Surface {
    /// The bilinear patch through four corners (counter-clockwise), with their offsets.
    Bilinear { corners: [Vec2; 4], z: [f64; 4] },
    /// Triangles over the shape vertices (counter-clockwise in plan).
    Triangles { tris: Vec<[usize; 3]> },
}

/// Everything the engine reports of a shaped slab.
#[derive(Clone, Debug)]
pub struct ShapeGeometry {
    pub points: Vec<(Vec2, f64)>,
    pub surface: Surface,
}

fn polygon_area(p: &[Vec2]) -> f64 {
    let n = p.len();
    (0..n).map(|i| p[i].cross(p[(i + 1) % n])).sum::<f64>() / 2.0
}

/// The geometry of the shape of `f` (absolute heights), or `None` when it has no shape.
pub fn geometry(m: &BimModel, f: &Slab) -> Option<ShapeGeometry> {
    let shape = m.slab_shapes.get(&f.id)?;
    let base = reference_z(m, f);
    let points: Vec<(Vec2, f64)> = shape.vertices.iter().map(|v| (v.p, base + v.offset)).collect();
    // a four-sided sketch with its four corners only: the bilinear patch
    if f.loops.is_empty() && shape.vertices.len() == 4 && f.boundary.len() == 4 && shape.splits.is_empty() {
        let mut idx: Vec<usize> = Vec::new();
        let ccw = polygon_area(&f.boundary) > 0.0;
        let order: Vec<Vec2> = if ccw { f.boundary.clone() } else { f.boundary.iter().rev().copied().collect() };
        for q in &order {
            idx.push(points.iter().position(|(p, _)| (*p - *q).length() < 1e-9)?);
        }
        let corners = [order[0], order[1], order[2], order[3]];
        let z = [points[idx[0]].1, points[idx[1]].1, points[idx[2]].1, points[idx[3]].1];
        return Some(ShapeGeometry { points, surface: Surface::Bilinear { corners, z } });
    }
    let tris = triangulate(f, &points, &shape.splits)?;
    Some(ShapeGeometry { points, surface: Surface::Triangles { tris } })
}

fn tri_area_3d(p: &[(Vec2, f64)], t: [usize; 3]) -> f64 {
    let (a, b, c) = (p[t[0]], p[t[1]], p[t[2]]);
    let u = (b.0.x - a.0.x, b.0.y - a.0.y, b.1 - a.1);
    let v = (c.0.x - a.0.x, c.0.y - a.0.y, c.1 - a.1);
    let cr = (u.1 * v.2 - u.2 * v.1, u.2 * v.0 - u.0 * v.2, u.0 * v.1 - u.1 * v.0);
    0.5 * (cr.0 * cr.0 + cr.1 * cr.1 + cr.2 * cr.2).sqrt()
}

fn orient(a: Vec2, b: Vec2, c: Vec2) -> f64 {
    (b - a).cross(c - a)
}

fn segments_cross(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    let (d1, d2, d3, d4) = (orient(c, d, a), orient(c, d, b), orient(a, b, c), orient(a, b, d));
    d1 * d2 < -1e-12 && d3 * d4 < -1e-12
}

/// Ear clipping of the sketch (outer loop and holes joined by bridges) over the first `nb` points.
fn triangulate_boundary(f: &Slab, pts: &[(Vec2, f64)]) -> Option<Vec<[usize; 3]>> {
    let find = |q: Vec2| pts.iter().position(|(p, _)| (*p - q).length() < 1e-9);
    let mut outer: Vec<usize> = f.boundary.iter().map(|&q| find(q)).collect::<Option<_>>()?;
    if polygon_area(&f.boundary) < 0.0 {
        outer.reverse();
    }
    let mut holes: Vec<Vec<usize>> = Vec::new();
    for l in &f.loops {
        let mut h: Vec<usize> = l.iter().map(|&q| find(q)).collect::<Option<_>>()?;
        if polygon_area(l) > 0.0 {
            h.reverse();
        }
        holes.push(h);
    }
    let all_edges = |outer: &[usize], holes: &[Vec<usize>]| -> Vec<(usize, usize)> {
        let mut e = Vec::new();
        for lp in std::iter::once(outer).chain(holes.iter().map(|h| h.as_slice())) {
            for i in 0..lp.len() {
                e.push((lp[i], lp[(i + 1) % lp.len()]));
            }
        }
        e
    };
    let edges = all_edges(&outer, &holes);
    // join the holes, rightmost first
    holes.sort_by(|a, b| {
        let mx = |h: &Vec<usize>| h.iter().map(|&i| pts[i].0.x).fold(f64::NEG_INFINITY, f64::max);
        mx(b).total_cmp(&mx(a))
    });
    let mut ring = outer.clone();
    for h in &holes {
        let hv = (0..h.len()).max_by(|&a, &b| pts[h[a]].0.x.total_cmp(&pts[h[b]].0.x))?;
        let mut cand: Vec<usize> = (0..ring.len()).collect();
        cand.sort_by(|&a, &b| (pts[ring[a]].0 - pts[h[hv]].0).length().total_cmp(&(pts[ring[b]].0 - pts[h[hv]].0).length()));
        let mut joined = None;
        for ci in cand {
            let (u, v) = (pts[ring[ci]].0, pts[h[hv]].0);
            if (u - v).length() < 1e-12 {
                continue;
            }
            let blocked = edges.iter().any(|&(a, b)| {
                let (pa, pb) = (pts[a].0, pts[b].0);
                ![ring[ci], h[hv]].iter().any(|&x| x == a || x == b) && segments_cross(u, v, pa, pb)
            });
            let mid = (u + v) * 0.5;
            if !blocked && inside_material(f, mid) {
                joined = Some(ci);
                break;
            }
        }
        let ci = joined?;
        let mut next: Vec<usize> = ring[..=ci].to_vec();
        for k in 0..=h.len() {
            next.push(h[(hv + k) % h.len()]);
        }
        next.push(ring[ci]);
        next.extend_from_slice(&ring[ci + 1..]);
        ring = next;
    }
    // ear clipping
    let mut tris: Vec<[usize; 3]> = Vec::new();
    let mut guard = 0usize;
    while ring.len() > 3 && guard < 10_000 {
        guard += 1;
        let n = ring.len();
        let mut clipped = false;
        for i in 0..n {
            let (a, b, c) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
            let (pa, pb, pc) = (pts[a].0, pts[b].0, pts[c].0);
            if orient(pa, pb, pc) <= 1e-12 {
                continue;
            }
            let any_inside = ring.iter().any(|&x| {
                if x == a || x == b || x == c {
                    return false;
                }
                let q = pts[x].0;
                if [pa, pb, pc].iter().any(|p| (*p - q).length() < 1e-12) {
                    return false;
                }
                orient(pa, pb, q) >= -1e-12 && orient(pb, pc, q) >= -1e-12 && orient(pc, pa, q) >= -1e-12
            });
            if any_inside {
                continue;
            }
            tris.push([a, b, c]);
            ring.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            return None;
        }
    }
    if ring.len() == 3 && orient(pts[ring[0]].0, pts[ring[1]].0, pts[ring[2]].0) > 1e-12 {
        tris.push([ring[0], ring[1], ring[2]]);
    }
    Some(tris)
}

fn in_circle(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> f64 {
    // > 0 when d is inside the circle through the counter-clockwise a, b, c
    let (ax, ay, bx, by, cx, cy) = (a.x - d.x, a.y - d.y, b.x - d.x, b.y - d.y, c.x - d.x, c.y - d.y);
    (ax * ax + ay * ay) * (bx * cy - cx * by) - (bx * bx + by * by) * (ax * cy - cx * ay) + (cx * cx + cy * cy) * (ax * by - bx * ay)
}

/// Triangulation of the shape: the sketch is ear-clipped, the interior vertices are inserted, then the edges that are not
/// sketch edges or split lines are flipped to the Delaunay choice, and, where two choices are equally good in plan, to the
/// one of least surface area (ENGINE CHOICE).
fn triangulate(f: &Slab, pts: &[(Vec2, f64)], splits: &[(usize, usize)]) -> Option<Vec<[usize; 3]>> {
    let nb = f.boundary.len() + f.loops.iter().map(|l| l.len()).sum::<usize>();
    let mut tris = triangulate_boundary(f, pts)?;
    // insert the interior vertices
    for k in nb..pts.len() {
        let q = pts[k].0;
        if tris.iter().any(|t| t.contains(&k)) {
            continue;
        }
        let mut hit: Option<usize> = None;
        for (i, t) in tris.iter().enumerate() {
            let (a, b, c) = (pts[t[0]].0, pts[t[1]].0, pts[t[2]].0);
            if orient(a, b, q) >= -1e-9 && orient(b, c, q) >= -1e-9 && orient(c, a, q) >= -1e-9 {
                hit = Some(i);
                break;
            }
        }
        let i = hit?;
        let t = tris[i];
        let on_edge = (0..3).find(|&e| orient(pts[t[e]].0, pts[t[(e + 1) % 3]].0, q).abs() < 1e-9);
        match on_edge {
            None => {
                tris.swap_remove(i);
                tris.push([t[0], t[1], k]);
                tris.push([t[1], t[2], k]);
                tris.push([t[2], t[0], k]);
            }
            Some(e) => {
                let (a, b, c) = (t[e], t[(e + 1) % 3], t[(e + 2) % 3]);
                // the neighbour across a-b, if any
                let other = tris.iter().position(|u| u != &t && u.contains(&a) && u.contains(&b));
                tris.retain(|u| u != &t);
                tris.push([a, k, c]);
                tris.push([k, b, c]);
                if let Some(_) = other {
                    let o = tris.iter().position(|u| u.contains(&a) && u.contains(&b) && !u.contains(&k)).unwrap();
                    let ot = tris.swap_remove(o);
                    let d = *ot.iter().find(|&&x| x != a && x != b)?;
                    tris.push([b, k, d]);
                    tris.push([k, a, d]);
                }
            }
        }
    }
    // the edges that must stay: the sketch's and the split lines
    let mut fixed: Vec<(usize, usize)> = splits.to_vec();
    let find = |q: Vec2| pts.iter().position(|(p, _)| (*p - q).length() < 1e-9);
    for lp in std::iter::once(&f.boundary).chain(f.loops.iter()) {
        for i in 0..lp.len() {
            if let (Some(a), Some(b)) = (find(lp[i]), find(lp[(i + 1) % lp.len()])) {
                fixed.push((a, b));
            }
        }
    }
    let is_fixed = |a: usize, b: usize| fixed.iter().any(|&(x, y)| (x == a && y == b) || (x == b && y == a));
    // split lines first: bring them into the triangulation by flipping what crosses them
    for &(a, b) in splits {
        for _ in 0..200 {
            if tris.iter().any(|t| t.contains(&a) && t.contains(&b)) {
                break;
            }
            let mut flipped = false;
            let cross_edges: Vec<(usize, usize)> = {
                let mut e = Vec::new();
                for t in &tris {
                    for k in 0..3 {
                        let (u, v) = (t[k], t[(k + 1) % 3]);
                        if u < v && !is_fixed(u, v) && segments_cross(pts[a].0, pts[b].0, pts[u].0, pts[v].0) {
                            e.push((u, v));
                        }
                    }
                }
                e
            };
            for (u, v) in cross_edges {
                if flip_edge(&mut tris, pts, u, v) {
                    flipped = true;
                    break;
                }
            }
            if !flipped {
                break;
            }
        }
    }
    // Lawson flips
    for _ in 0..(tris.len() * tris.len() + 50) {
        let mut changed = false;
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for t in &tris {
            for k in 0..3 {
                let (u, v) = (t[k], t[(k + 1) % 3]);
                if u < v {
                    edges.push((u, v));
                }
            }
        }
        for (u, v) in edges {
            if is_fixed(u, v) {
                continue;
            }
            let Some((c, d)) = opposite(&tris, u, v) else { continue };
            // convex quad only
            if orient(pts[c].0, pts[u].0, pts[d].0) * orient(pts[c].0, pts[v].0, pts[d].0) >= 0.0 {
                continue;
            }
            let (a, b) = (pts[u].0, pts[v].0);
            let (pc, pd) = (pts[c].0, pts[d].0);
            // orient (u, v, c) counter-clockwise for the test
            let det = if orient(a, b, pc) > 0.0 { in_circle(a, b, pc, pd) } else { in_circle(b, a, pc, pd) };
            let span = (a - b).length().max((pc - pd).length()).max(1e-3);
            let scale = 1e-9 * span.powi(4);
            let do_flip = if det > scale {
                true
            } else if det.abs() <= scale {
                // equally good in plan: the choice of least surface area
                let now = tri_area_3d(pts, [u, v, c]) + tri_area_3d(pts, [u, v, d]);
                let after = tri_area_3d(pts, [c, d, u]) + tri_area_3d(pts, [c, d, v]);
                after < now - 1e-12
            } else {
                false
            };
            if do_flip && flip_edge(&mut tris, pts, u, v) {
                changed = true;
                break;
            }
        }
        if !changed {
            break;
        }
    }
    Some(tris)
}

/// The two vertices opposite the edge `u`-`v` in its two triangles.
fn opposite(tris: &[[usize; 3]], u: usize, v: usize) -> Option<(usize, usize)> {
    let mut opp = tris.iter().filter(|t| t.contains(&u) && t.contains(&v)).map(|t| *t.iter().find(|&&x| x != u && x != v).unwrap());
    let (c, d) = (opp.next()?, opp.next()?);
    Some((c, d))
}

fn flip_edge(tris: &mut Vec<[usize; 3]>, pts: &[(Vec2, f64)], u: usize, v: usize) -> bool {
    let Some((c, d)) = opposite(tris, u, v) else { return false };
    if orient(pts[c].0, pts[u].0, pts[d].0) * orient(pts[c].0, pts[v].0, pts[d].0) >= 0.0 {
        return false;
    }
    tris.retain(|t| !(t.contains(&u) && t.contains(&v)));
    let mut make = |a: usize, b: usize, c: usize| {
        if orient(pts[a].0, pts[b].0, pts[c].0) > 0.0 {
            tris.push([a, b, c]);
        } else {
            tris.push([a, c, b]);
        }
    };
    make(c, d, u);
    make(c, d, v);
    true
}

// ----------------------------------------------------------------------------------- quantities

/// Gauss-Legendre nodes and weights on [0, 1] (`n` points).
fn gauss_legendre(n: usize) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for i in 0..n {
        let mut x = (std::f64::consts::PI * (i as f64 + 0.75) / (n as f64 + 0.5)).cos();
        let mut dp = 1.0;
        for _ in 0..100 {
            let (mut p0, mut p1) = (1.0, x);
            for k in 2..=n {
                let p2 = ((2 * k - 1) as f64 * x * p1 - (k - 1) as f64 * p0) / k as f64;
                p0 = p1;
                p1 = p2;
            }
            dp = n as f64 * (x * p1 - p0) / (x * x - 1.0);
            let dx = p1 / dp;
            x -= dx;
            if dx.abs() < 1e-15 {
                break;
            }
        }
        let w = 2.0 / ((1.0 - x * x) * dp * dp);
        out.push((0.5 * (x + 1.0), 0.5 * w));
    }
    out
}

fn bilinear_point(corners: &[Vec2; 4], z: &[f64; 4], s: f64, t: f64) -> (Vec2, f64) {
    let w = [(1.0 - s) * (1.0 - t), s * (1.0 - t), s * t, (1.0 - s) * t];
    let mut p = Vec2::new(0.0, 0.0);
    let mut h = 0.0;
    for k in 0..4 {
        p = p + corners[k] * w[k];
        h += z[k] * w[k];
    }
    (p, h)
}

/// The height of the reference surface above the plan point `p` (`None` where `p` is not over the surface).
pub fn surface_z_at(g: &ShapeGeometry, p: Vec2) -> Option<f64> {
    match &g.surface {
        Surface::Bilinear { corners, z } => {
            // inverse bilinear map by Newton from the middle
            let (mut s, mut t) = (0.5_f64, 0.5_f64);
            for _ in 0..60 {
                let (q, _) = bilinear_point(corners, z, s, t);
                let r = p - q;
                let ps = (corners[1] - corners[0]) * (1.0 - t) + (corners[2] - corners[3]) * t;
                let pt = (corners[3] - corners[0]) * (1.0 - s) + (corners[2] - corners[1]) * s;
                let det = ps.cross(pt);
                if det.abs() < 1e-14 {
                    return None;
                }
                let ds = r.cross(pt) / det;
                let dt = ps.cross(r) / det;
                s += ds;
                t += dt;
                if ds.abs() + dt.abs() < 1e-14 {
                    break;
                }
            }
            if s < -1e-9 || s > 1.0 + 1e-9 || t < -1e-9 || t > 1.0 + 1e-9 {
                return None;
            }
            Some(bilinear_point(corners, z, s, t).1)
        }
        Surface::Triangles { tris } => {
            for t in tris {
                let (a, b, c) = (g.points[t[0]], g.points[t[1]], g.points[t[2]]);
                let area = orient(a.0, b.0, c.0);
                let (u, v, w) = (orient(p, b.0, c.0) / area, orient(a.0, p, c.0) / area, orient(a.0, b.0, p) / area);
                if u >= -1e-9 && v >= -1e-9 && w >= -1e-9 {
                    return Some(u * a.1 + v * b.1 + w * c.1);
                }
            }
            None
        }
    }
}

/// The area of the reference surface.
fn surface_area(g: &ShapeGeometry) -> f64 {
    match &g.surface {
        Surface::Triangles { tris } => tris.iter().map(|&t| tri_area_3d(&g.points, t)).sum(),
        Surface::Bilinear { corners, z } => {
            let gl = gauss_legendre(24);
            let mut total = 0.0;
            for &(s, ws) in &gl {
                for &(t, wt) in &gl {
                    let ps = (corners[1] - corners[0]) * (1.0 - t) + (corners[2] - corners[3]) * t;
                    let pt = (corners[3] - corners[0]) * (1.0 - s) + (corners[2] - corners[1]) * s;
                    let zs = (z[1] - z[0]) * (1.0 - t) + (z[2] - z[3]) * t;
                    let zt = (z[3] - z[0]) * (1.0 - s) + (z[2] - z[1]) * s;
                    let n = (ps.y * zt - zs * pt.y, zs * pt.x - ps.x * zt, ps.x * pt.y - ps.y * pt.x);
                    total += ws * wt * (n.0 * n.0 + n.1 * n.1 + n.2 * n.2).sqrt();
                }
            }
            total
        }
    }
}

/// The integral of the surface height over the plan.
fn height_integral(g: &ShapeGeometry) -> f64 {
    match &g.surface {
        Surface::Triangles { tris } => tris
            .iter()
            .map(|t| {
                let (a, b, c) = (g.points[t[0]], g.points[t[1]], g.points[t[2]]);
                orient(a.0, b.0, c.0).abs() / 2.0 * (a.1 + b.1 + c.1) / 3.0
            })
            .sum(),
        Surface::Bilinear { corners, z } => {
            let gl = gauss_legendre(24);
            let mut total = 0.0;
            for &(s, ws) in &gl {
                for &(t, wt) in &gl {
                    let ps = (corners[1] - corners[0]) * (1.0 - t) + (corners[2] - corners[3]) * t;
                    let pt = (corners[3] - corners[0]) * (1.0 - s) + (corners[2] - corners[1]) * s;
                    let (_, h) = bilinear_point(corners, z, s, t);
                    total += ws * wt * ps.cross(pt).abs() * h;
                }
            }
            total
        }
    }
}

/// Number of planar faces of the reference surface: coplanar neighbouring triangles are one face.
fn planar_faces(g: &ShapeGeometry) -> usize {
    let Surface::Triangles { tris } = &g.surface else { return 1 };
    let plane = |t: &[usize; 3]| {
        let (a, b, c) = (g.points[t[0]], g.points[t[1]], g.points[t[2]]);
        let u = (b.0.x - a.0.x, b.0.y - a.0.y, b.1 - a.1);
        let v = (c.0.x - a.0.x, c.0.y - a.0.y, c.1 - a.1);
        let n = (u.1 * v.2 - u.2 * v.1, u.2 * v.0 - u.0 * v.2, u.0 * v.1 - u.1 * v.0);
        let l = (n.0 * n.0 + n.1 * n.1 + n.2 * n.2).sqrt();
        let n = (n.0 / l, n.1 / l, n.2 / l);
        let d = n.0 * a.0.x + n.1 * a.0.y + n.2 * a.1;
        // a canonical sign (counter-clockwise triangles face up)
        (n, d)
    };
    let planes: Vec<_> = tris.iter().map(plane).collect();
    let mut parent: Vec<usize> = (0..tris.len()).collect();
    fn find(p: &mut Vec<usize>, mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for i in 0..tris.len() {
        for j in (i + 1)..tris.len() {
            let shared = tris[i].iter().filter(|v| tris[j].contains(v)).count();
            if shared == 2 {
                let ((n1, d1), (n2, d2)) = (planes[i], planes[j]);
                if (n1.0 - n2.0).abs() < 1e-9 && (n1.1 - n2.1).abs() < 1e-9 && (n1.2 - n2.2).abs() < 1e-9 && (d1 - d2).abs() < 1e-9 {
                    let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                    parent[ri] = rj;
                }
            }
        }
    }
    (0..tris.len()).filter(|&i| find(&mut parent, i) == i).count()
}

/// What a shaped slab reports.
#[derive(Clone, Debug)]
pub struct ShapedQuantities {
    pub area: f64,
    pub volume_param: f64,
    pub volume: f64,
    pub surface_area: f64,
    pub bbox_z: (f64, f64),
    pub faces: usize,
    pub edges: usize,
    pub vertices: usize,
}

/// The quantities of the shaped slab `f` with thickness `t`; `variable` is the type's variable layer (a floor whose bottom
/// then stays flat at the nominal thickness under the reference plane).
pub fn quantities(m: &BimModel, f: &Slab, t: f64, variable: bool) -> Option<ShapedQuantities> {
    let g = geometry(m, f)?;
    let base = reference_z(m, f);
    let plan_area = polygon_area(&f.boundary).abs() - f.loops.iter().map(|l| polygon_area(l).abs()).sum::<f64>();
    let a_top = surface_area(&g);
    let floor = f.role == SlabRole::Floor && f.anchor == SlabAnchor::TopAtLevel;
    let z_vals: Vec<f64> = g.points.iter().map(|p| p.1).collect();
    let (zmin, zmax) = (z_vals.iter().cloned().fold(f64::INFINITY, f64::min), z_vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
    let flat_bottom = floor && variable;
    // heights of the solid
    let top_faces = planar_faces(&g);
    let bottom_faces = if flat_bottom { 1 } else { top_faces };
    // perimeter integral of the thickness over the sketch edges
    let mut sides = 0.0;
    let mut side_faces = 0usize;
    let edge_z = |p: Vec2| surface_z_at(&g, p);
    for lp in std::iter::once(&f.boundary).chain(f.loops.iter()) {
        for i in 0..lp.len() {
            let (a, b) = (lp[i], lp[(i + 1) % lp.len()]);
            let len = (b - a).length();
            side_faces += 1;
            if flat_bottom {
                // the top is linear along a sketch edge between its two corners
                let (za, zb) = (edge_z(a).unwrap_or(base), edge_z(b).unwrap_or(base));
                sides += len * ((za + zb) / 2.0 - (base - t));
            } else {
                sides += len * t;
            }
        }
    }
    let mut pinches = 0usize;
    let (volume, surface_area, z_lo, z_hi, vertices) = if flat_bottom {
        let vol = height_integral(&g) - plan_area * (base - t);
        let sa = a_top + plan_area + sides;
        // the vertices of the top and of the flat bottom (corner vertices only); a corner whose thickness is nil is one vertex
        let mut pts3: Vec<(f64, f64, f64)> = g.points.iter().map(|p| (p.0.x, p.0.y, p.1)).collect();
        for v in g.points.iter().take(f.boundary.len() + f.loops.iter().map(|l| l.len()).sum::<usize>()) {
            let q = (v.0.x, v.0.y, base - t);
            if !pts3.iter().any(|u| (u.0 - q.0).abs() < 1e-9 && (u.1 - q.1).abs() < 1e-9 && (u.2 - q.2).abs() < 1e-9) {
                pts3.push(q);
            } else {
                // the top meets the flat bottom at this corner: the solid pinches to a point (OBSERVED: one edge more than Euler's)
                pinches += 1;
            }
        }
        (vol, sa, (base - t).min(zmin), zmax, pts3.len())
    } else if floor {
        (plan_area * t, 2.0 * a_top + sides, zmin - t, zmax, 2 * g.points.len())
    } else {
        // a roof (and a ceiling): the reference surface is the underside, the solid grows up
        (plan_area * t, 2.0 * a_top + sides, zmin, zmax + t, 2 * g.points.len())
    };
    let faces = top_faces + bottom_faces + side_faces;
    let edges = (vertices + faces + 2 * f.loops.len() + pinches).saturating_sub(2);
    Some(ShapedQuantities {
        area: a_top,
        volume_param: if flat_bottom { volume } else { a_top * t },
        volume,
        surface_area,
        bbox_z: (z_lo, z_hi),
        faces,
        edges,
        vertices,
    })
}

/// The volume of the SOLID of a shaped slab (the Volume parameter is [`ShapedQuantities::volume_param`]), `None` when the
/// slab has no shape.
pub fn solid_volume(m: &BimModel, f: &Slab) -> Option<f64> {
    let t = m.slab_type(f.type_id).ok()?.thickness;
    let variable = m.slab_variable_layers.contains_key(&f.type_id);
    quantities(m, f, t, variable).map(|q| q.volume)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SlabRole;
    use crate::ops;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Vec2> {
        vec![Vec2::new(x0, y0), Vec2::new(x1, y0), Vec2::new(x1, y1), Vec2::new(x0, y1)]
    }

    fn near(a: f64, b: f64, tol: f64, what: &str) {
        assert!((a - b).abs() <= tol, "{what}: got {a}, want {b}");
    }

    /// Round 41's floor: 6 x 4, top at 3.0, type 0.2.
    fn floor_doc() -> (crate::BimDocument, ElementId, ElementId, ElementId) {
        let mut d = crate::new_document();
        let l = d.transact("l", false, |m, cx| ops::create_level(m, cx, "L1", 3.0)).value.unwrap();
        let t = d.transact("t", false, |m, cx| ops::create_floor_type(m, cx, "FT200", 0.2)).value.unwrap();
        let f = d.transact("f", false, |m, cx| ops::create_floor(m, cx, t, l, rect(0.0, 0.0, 6.0, 4.0), 0.0, false)).value.unwrap();
        (d, l, t, f)
    }

    fn probes(d: &crate::BimDocument, f: ElementId) -> Vec<f64> {
        let m = d.model();
        [[1.0, 1.0], [3.0, 2.0], [5.0, 3.0], [5.5, 0.5]]
            .iter()
            .map(|p| crate::regen::slab_reference_z_at(m, &m.floors[&f], Vec2::new(p[0], p[1])).unwrap())
            .collect()
    }

    /// Round 41 `corner-6-4-lowered-0.3`: the four corners only make the bilinear patch.
    #[test]
    fn four_corners_make_a_bilinear_patch() {
        let (mut d, _l, _t, f) = floor_doc();
        d.transact("e", false, |m, cx| enable(m, cx, f)).value.unwrap();
        d.transact("o", false, |m, cx| offset_vertex(m, cx, f, [6.0, 4.0], -0.3)).value.unwrap();
        for (z, want) in probes(&d, f).iter().zip([2.9875, 2.925, 2.8125, 2.965625]) {
            near(*z, want, 1e-9, "top_z_at");
        }
        let m = d.model();
        let s = crate::regen::slab_state(m, &m.floors[&f]).unwrap();
        near(s.area, 24.032467966141784, 1e-7, "area");
        near(s.volume, 4.806493593228358, 1e-7, "Volume parameter = area x t (Spec R-01c)");
        near(solid_volume(m, &m.floors[&f]).unwrap(), 4.8, 1e-9, "the solid is the plan area x t");
        near(s.surface_area, 52.06493593228356, 1e-7, "surface");
        near(s.bbox_min.z, 2.5, 1e-9, "bottom");
        near(s.bbox_max.z, 3.0, 1e-9, "top");
        assert_eq!((s.face_count, s.edge_count, s.vertex_count), (6, 12, 8));
    }

    /// Round 41 `centre-point-lowered-0.2-warped-surface` and `split-line-along-the-diagonal`: a fan of four triangles.
    #[test]
    fn an_interior_vertex_makes_a_triangulation() {
        let (mut d, _l, _t, f) = floor_doc();
        d.transact("e", false, |m, cx| enable(m, cx, f)).value.unwrap();
        d.transact("o", false, |m, cx| offset_vertex(m, cx, f, [6.0, 4.0], -0.3)).value.unwrap();
        d.transact("p", false, |m, cx| add_point(m, cx, f, [3.0, 2.0, 3.0])).value.unwrap();
        for (z, want) in probes(&d, f).iter().zip([3.0, 3.0, 2.825, 2.9875]) {
            near(*z, want, 1e-9, "top_z_at with the centre at 3.0");
        }
        {
            let m = d.model();
            let s = crate::regen::slab_state(m, &m.floors[&f]).unwrap();
            near(s.area, 24.048651376813922, 1e-7, "area");
            // the two flat triangles are one face, the two others one: 2 + 2 + 4 sides
            assert_eq!((s.face_count, s.edge_count, s.vertex_count), (8, 16, 10));
        }
        d.transact("c", false, |m, cx| offset_vertex(m, cx, f, [3.0, 2.0], -0.2)).value.unwrap();
        for (z, want) in probes(&d, f).iter().zip([2.933333333, 2.8, 2.758333333, 2.954166667]) {
            near(*z, want, 1e-8, "top_z_at with the centre lowered");
        }
        let m = d.model();
        let s = crate::regen::slab_state(m, &m.floors[&f]).unwrap();
        near(s.area, 24.07029388931154, 1e-7, "area");
        assert_eq!((s.face_count, s.edge_count, s.vertex_count), (12, 20, 10));
        // a split line along the diagonal changes nothing
        let mut d2 = d;
        d2.transact("s", false, |m, cx| add_split_line(m, cx, f, [0.0, 0.0], [6.0, 4.0])).value.unwrap();
        let m = d2.model();
        let s2 = crate::regen::slab_state(m, &m.floors[&f]).unwrap();
        near(s2.area, s.area, 1e-9, "area");
        assert_eq!(m.slab_shapes[&f].splits.len(), 2, "one crease for each stretch between vertices");
    }

    /// Round 41 `core-layer-variable`: the bottom stays flat at the nominal thickness under the reference plane.
    #[test]
    fn a_variable_layer_keeps_the_bottom_flat() {
        let (mut d, _l, t, f) = floor_doc();
        d.transact("th", false, |m, cx| ops::set_floor_type_thickness(m, cx, t, 0.3)).value.unwrap();
        d.transact("e", false, |m, cx| enable(m, cx, f)).value.unwrap();
        d.transact("o", false, |m, cx| offset_vertex(m, cx, f, [6.0, 4.0], -0.3)).value.unwrap();
        d.transact("p", false, |m, cx| add_point(m, cx, f, [3.0, 2.0, 3.0])).value.unwrap();
        d.transact("c", false, |m, cx| offset_vertex(m, cx, f, [3.0, 2.0], -0.2)).value.unwrap();
        d.transact("v", false, |m, cx| ops::set_floor_variable_layer(m, cx, t, 0)).value.unwrap();
        let m = d.model();
        let s = crate::regen::slab_state(m, &m.floors[&f]).unwrap();
        near(s.volume, 4.4, 1e-9, "Volume");
        near(s.surface_area, 52.57029388931154, 1e-7, "surface");
        near(s.bbox_min.z, 2.7, 1e-9, "flat bottom");
        near(s.bbox_max.z, 3.0, 1e-9, "top");
        // the corner whose top meets the flat bottom is one vertex (and an edge more: the pinch)
        assert_eq!((s.face_count, s.edge_count, s.vertex_count), (9, 16, 8));
        d.transact("o2", false, |m, cx| offset_vertex(m, cx, f, [0.0, 0.0], 0.5)).value.unwrap();
        let m = d.model();
        let s = crate::regen::slab_state(m, &m.floors[&f]).unwrap();
        near(s.volume, 6.4, 1e-9, "Volume with the corner raised");
        near(s.bbox_max.z, 3.5, 1e-9, "top");
    }

    /// Round 41 `floor-with-a-hole-inner-corner-lowered`: the triangulation of the annulus of least surface area.
    #[test]
    fn a_floor_with_a_hole_is_triangulated_with_the_least_surface_area() {
        let (mut d, l, t, _f) = floor_doc();
        d.transact("th", false, |m, cx| ops::set_floor_type_thickness(m, cx, t, 0.3)).value.unwrap();
        let f2 = d
            .transact("f2", false, |m, cx| {
                ops::create_slab_with_loops(m, cx, t, l, rect(10.0, 0.0, 16.0, 4.0), vec![rect(12.0, 1.0, 14.0, 3.0)], 0.0, false)
            })
            .value
            .unwrap();
        d.transact("e", false, |m, cx| enable(m, cx, f2)).value.unwrap();
        d.transact("o", false, |m, cx| offset_vertex(m, cx, f2, [12.0, 1.0], -0.2)).value.unwrap();
        let m = d.model();
        let s = crate::regen::slab_state(m, &m.floors[&f2]).unwrap();
        near(s.area, 20.113905525929354, 1e-7, "area");
        near(s.volume, 6.034171657778805, 1e-7, "Volume = area x t");
        near(s.surface_area, 48.6278110518587, 1e-6, "surface");
        near(s.bbox_min.z, 2.5, 1e-9, "bottom");
        assert_eq!((s.face_count, s.edge_count, s.vertex_count), (14, 30, 16));
    }

    /// Round 41 `flat-roof-corner-raised-0.6-by-shape-editing`: a roof's shape is its underside, the solid grows up; a slope is refused.
    #[test]
    fn a_shaped_roof_grows_up_from_its_surface_and_takes_no_slope() {
        let mut d = crate::new_document();
        let l = d.transact("l", false, |m, cx| ops::create_level(m, cx, "L1", 3.0)).value.unwrap();
        let t = d.transact("t", false, |m, cx| ops::create_slab_type(m, cx, "RT300", 0.3, SlabRole::Roof)).value.unwrap();
        let r = d.transact("r", false, |m, cx| ops::create_slab(m, cx, t, l, rect(20.0, 0.0, 26.0, 4.0), 0.0, false)).value.unwrap();
        d.transact("e", false, |m, cx| enable(m, cx, r)).value.unwrap();
        d.transact("o", false, |m, cx| offset_vertex(m, cx, r, [26.0, 4.0], 0.6)).value.unwrap();
        let m = d.model();
        let s = crate::regen::slab_state(m, &m.floors[&r]).unwrap();
        near(s.area, 24.1294910363766, 1e-7, "area");
        near(s.volume, 7.238847310912979, 1e-7, "Volume = area x t");
        near(s.bbox_min.z, 3.0, 1e-9, "underside");
        near(s.bbox_max.z, 3.9, 1e-9, "top");
        near(solid_volume(m, &m.floors[&r]).unwrap(), 7.2, 1e-9, "solid");
        let out = d.transact("slope", false, |m, cx| ops::set_slab_segment_slope(m, cx, r, 0, true, 0.5));
        assert!(!out.committed());
        // resetting removes the shape
        d.transact("reset", false, |m, cx| reset(m, cx, r)).value.unwrap();
        assert!(d.model().slab_shapes.is_empty());
    }
}
