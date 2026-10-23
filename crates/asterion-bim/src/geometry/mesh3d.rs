//! The 3D view's mesh: every element the engine has a body for, as triangles and crease edges.
//!
//! A pure function of the model (`view_mesh`): nothing is cached. The bodies are the ones [`crate::element_geometry`] gives
//! - the SAME the section and elevation line work and the diagnostics of `view_visibility` use - so an element that has a
//! body in the engine is in this mesh, and one that has none is listed in [`Mesh3d::gaps`] with the reason. Nothing here
//! decides what the reference product shows in a 3D view beyond what `view_mesh` states; it only turns the engine's solids
//! into a mesh.
//!
//! # Layout of the buffers
//!
//! * `triangles`: a triangle list, 9 floats per vertex: `x y z nx ny nz r g b` (metres; unit normal).
//! * `edges`: 6 floats per segment: `x0 y0 z0 x1 y1 z1`. The crease edges: the top and bottom loops of every solid, and the
//!   vertical edges where the adjacent side faces meet at more than [`CREASE_DEGREES`] (ASSUMED threshold: the reference's
//!   crease angle is not specified).
//!
//! # What is absent (named in [`Mesh3d::gaps`] or below)
//!
//! Stairs, railings and landings (the stairs module has no solid), toposolids with surface points, shape-edited slabs, the
//! family geometry of doors and windows (only the opening they cut is drawn, outlined), wall reveals and the openings of an
//! arc wall (not subtracted), rooms, grids, levels, wall layers and materials (colours are per element class, ASSUMED:),
//! and an element id per triangle (3D picking).

use crate::element_geometry::{self, Body, ElementClass, Geometry, WallCut};
use crate::polysolid::{Part, PolySolid};
use crate::model::{BimModel, View};
use asterion_core::ElementId;
use asterion_geom::Vec2;
use std::collections::BTreeSet;

/// Side faces that meet at more than this angle get a vertical crease edge (ASSUMED).
pub const CREASE_DEGREES: f64 = 25.0;

/// The exported scene.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh3d {
    /// 9 floats per vertex, 3 vertices per triangle.
    pub triangles: Vec<f32>,
    /// 6 floats per segment.
    pub edges: Vec<f32>,
    /// Number of solids exported.
    pub solids: usize,
    /// The elements this mesh draws: what was handed to the renderer, by element.
    pub elements: BTreeSet<ElementId>,
    /// Elements the 3D view should draw and the engine has no body for (or failed to regenerate), with the reason.
    pub gaps: Vec<(ElementId, String)>,
    /// Where each element's triangles and edges lie in the two buffers (float offsets): what picking and the selection
    /// highlight of a 3D view are made from. An element that owns several pieces (a wall and the openings cut in it) has
    /// several ranges.
    pub ranges: Vec<ElementRange>,
}

/// The slices of [`Mesh3d::triangles`] and [`Mesh3d::edges`] one element owns, as `start..end` float offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElementRange {
    pub element: ElementId,
    pub tris: (usize, usize),
    pub edges: (usize, usize),
}

impl Mesh3d {
    pub fn triangle_count(&self) -> usize {
        self.triangles.len() / 27
    }
    pub fn edge_count(&self) -> usize {
        self.edges.len() / 6
    }
}
fn signed_area(p: &[Vec2]) -> f64 {
    let n = p.len();
    (0..n).map(|i| p[i].cross(p[(i + 1) % n])).sum::<f64>() * 0.5
}

/// Ear-clipping triangulation of a simple polygon given counter-clockwise. Falls back to a fan
/// when no ear is found (a degenerate or self-touching outline), so a solid is never dropped.
pub(crate) fn triangulate(poly: &[Vec2]) -> Vec<[usize; 3]> {
    let n = poly.len();
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n.saturating_sub(2));
    let inside = |a: Vec2, b: Vec2, c: Vec2, p: Vec2| {
        let s1 = (b - a).cross(p - a);
        let s2 = (c - b).cross(p - b);
        let s3 = (a - c).cross(p - c);
        s1 >= -1e-12 && s2 >= -1e-12 && s3 >= -1e-12
    };
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n + 8 {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for k in 0..m {
            let (i0, i1, i2) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
            let (a, b, c) = (poly[i0], poly[i1], poly[i2]);
            if (b - a).cross(c - b) <= 1e-12 {
                continue; // reflex or collinear: not an ear
            }
            let blocked = idx.iter().any(|&j| {
                j != i0 && j != i1 && j != i2 && inside(a, b, c, poly[j]) && !(poly[j] == a || poly[j] == b || poly[j] == c)
            });
            if blocked {
                continue;
            }
            out.push([i0, i1, i2]);
            idx.remove(k);
            clipped = true;
            break;
        }
        if !clipped {
            // no ear: fan the rest
            for k in 1..idx.len() - 1 {
                out.push([idx[0], idx[k], idx[k + 1]]);
            }
            return out;
        }
    }
    if idx.len() == 3 {
        out.push([idx[0], idx[1], idx[2]]);
    }
    out
}

/// The triangles of a simple polygon (any orientation) as vertex triples, for flat fills in plan.
pub fn triangulate_polygon(poly: &[Vec2]) -> Vec<[Vec2; 3]> {
    let mut p: Vec<Vec2> = poly.to_vec();
    if p.len() > 3 && p[0] == p[p.len() - 1] {
        p.pop();
    }
    if p.len() < 3 {
        return Vec::new();
    }
    if signed_area(&p) < 0.0 {
        p.reverse();
    }
    triangulate(&p).into_iter().map(|t| [p[t[0]], p[t[1]], p[t[2]]]).collect()
}

fn push_vertex(out: &mut Vec<f32>, p: [f64; 3], n: [f64; 3], c: [f32; 3]) {
    out.extend_from_slice(&[p[0] as f32, p[1] as f32, p[2] as f32, n[0] as f32, n[1] as f32, n[2] as f32, c[0], c[1], c[2]]);
}

fn push_edge(out: &mut Vec<f32>, a: [f64; 3], b: [f64; 3]) {
    out.extend_from_slice(&[a[0] as f32, a[1] as f32, a[2] as f32, b[0] as f32, b[1] as f32, b[2] as f32]);
}

fn orient(poly: &[Vec2]) -> Vec<Vec2> {
    let mut p: Vec<Vec2> = poly.to_vec();
    if p.len() > 3 && p[0] == p[p.len() - 1] {
        p.pop();
    }
    if signed_area(&p) < 0.0 {
        p.reverse();
    }
    p
}

/// The triangles of a region with holes (any number, no ear clipping across them): the plan is cut into
/// vertical strips between the x of consecutive vertices, and the part of each strip inside the region is a
/// set of trapezoids. Counter-clockwise triangles.
fn triangulate_region(outer: &[Vec2], holes: &[Vec<Vec2>]) -> Vec<[Vec2; 3]> {
    if holes.is_empty() {
        let tris = triangulate(outer);
        return tris.into_iter().map(|t| [outer[t[0]], outer[t[1]], outer[t[2]]]).collect();
    }
    let loops: Vec<&[Vec2]> = std::iter::once(outer).chain(holes.iter().map(|h| h.as_slice())).collect();
    let mut xs: Vec<f64> = loops.iter().flat_map(|l| l.iter().map(|p| p.x)).collect();
    xs.sort_by(|a, b| a.total_cmp(b));
    xs.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    let mut out = Vec::new();
    for w in xs.windows(2) {
        let (xa, xb) = (w[0], w[1]);
        let xm = 0.5 * (xa + xb);
        // the edges that span the strip, with their y at the strip's two sides and in its middle
        let mut edges: Vec<(f64, f64, f64)> = Vec::new();
        for l in &loops {
            for i in 0..l.len() {
                let (p, q) = (l[i], l[(i + 1) % l.len()]);
                if (p.x - xm) * (q.x - xm) < 0.0 {
                    let y_at = |x: f64| p.y + (q.y - p.y) * (x - p.x) / (q.x - p.x);
                    edges.push((y_at(xm), y_at(xa), y_at(xb)));
                }
            }
        }
        edges.sort_by(|a, b| a.0.total_cmp(&b.0));
        for pair in edges.chunks_exact(2) {
            let (e1, e2) = (pair[0], pair[1]);
            let q = [Vec2::new(xa, e1.1), Vec2::new(xb, e1.2), Vec2::new(xb, e2.2), Vec2::new(xa, e2.1)];
            for t in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                if (t[1] - t[0]).cross(t[2] - t[0]).abs() > 1e-14 {
                    out.push(t);
                }
            }
        }
    }
    out
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    [v[0] / l, v[1] / l, v[2] / l]
}

/// The triangles of a body: both caps (on its planes) and the vertical side faces of every loop.
fn push_body_faces(mesh: &mut Mesh3d, body: &Body, colour: [f32; 3]) {
    let outer = orient(&body.outer);
    let up = unit([-body.top.gx, -body.top.gy, 1.0]);
    let down = unit([body.bottom.gx, body.bottom.gy, -1.0]);
    for t in triangulate_region(&outer, &body.holes) {
        for p in t {
            push_vertex(&mut mesh.triangles, [p.x, p.y, body.top.at(p)], up, colour);
        }
        for p in t.iter().rev() {
            push_vertex(&mut mesh.triangles, [p.x, p.y, body.bottom.at(*p)], down, colour);
        }
    }
    for l in std::iter::once(&outer).chain(body.holes.iter()) {
        let n = l.len();
        for i in 0..n {
            let (a, b) = (l[i], l[(i + 1) % n]);
            let d = b - a;
            let len = d.length().max(1e-12);
            let nn = [d.y / len, -d.x / len, 0.0];
            let (a0, b0, b1, a1) = ([a.x, a.y, body.bottom.at(a)], [b.x, b.y, body.bottom.at(b)], [b.x, b.y, body.top.at(b)], [a.x, a.y, body.top.at(a)]);
            for q in [a0, b0, b1, a0, b1, a1] {
                push_vertex(&mut mesh.triangles, q, nn, colour);
            }
        }
    }
}

/// The crease edges of a body: both loops of every ring and the vertical edges where two side faces meet at
/// more than [`CREASE_DEGREES`].
fn push_body_edges(mesh: &mut Mesh3d, body: &Body) {
    let outer = orient(&body.outer);
    for l in std::iter::once(&outer).chain(body.holes.iter()) {
        let n = l.len();
        let normals: Vec<[f64; 2]> = (0..n)
            .map(|i| {
                let d = l[(i + 1) % n] - l[i];
                let len = d.length().max(1e-12);
                [d.y / len, -d.x / len]
            })
            .collect();
        for i in 0..n {
            let (a, b) = (l[i], l[(i + 1) % n]);
            push_edge(&mut mesh.edges, [a.x, a.y, body.top.at(a)], [b.x, b.y, body.top.at(b)]);
            push_edge(&mut mesh.edges, [a.x, a.y, body.bottom.at(a)], [b.x, b.y, body.bottom.at(b)]);
            let (prev, cur) = (normals[(i + n - 1) % n], normals[i]);
            let cos = (prev[0] * cur[0] + prev[1] * cur[1]).clamp(-1.0, 1.0);
            if cos.acos().to_degrees() > CREASE_DEGREES {
                push_edge(&mut mesh.edges, [a.x, a.y, body.bottom.at(a)], [a.x, a.y, body.top.at(a)]);
            }
        }
    }
}

/// Appends a body: its triangles and its crease edges.
pub fn push_body(mesh: &mut Mesh3d, body: &Body, colour: [f32; 3]) {
    if body.outer.len() < 3 {
        return;
    }
    push_body_faces(mesh, body, colour);
    push_body_edges(mesh, body);
    mesh.solids += 1;
}

/// Appends a polyhedron: its triangles and its crease edges.
pub fn push_polysolid(mesh: &mut Mesh3d, solid: &PolySolid, colour: [f32; 3]) {
    for (t, n) in solid.triangles() {
        for p in t {
            push_vertex(&mut mesh.triangles, p, n, colour);
        }
    }
    for (a, b) in solid.crease_edges(CREASE_DEGREES) {
        push_edge(&mut mesh.edges, a, b);
    }
    mesh.solids += 1;
}

/// Appends the prism of `footprint` between `z0` and `z1`.
pub fn push_prism(mesh: &mut Mesh3d, footprint: &[Vec2], z0: f64, z1: f64, colour: [f32; 3]) {
    if footprint.len() < 3 || !(z1 > z0) {
        return;
    }
    push_body(mesh, &Body::prism(footprint, z0, z1), colour);
}

/// The outline of one cut through a straight wall as edges: its rectangle on both faces of the wall and the four
/// reveal edges between them. The cut's element (the door, window or opening) owns them.
fn push_cut_edges(mesh: &mut Mesh3d, start: Vec2, dir: Vec2, half: f64, cut: &WallCut) {
    let n = dir.perp_ccw();
    let at = |u: f64, z: f64, side: f64| {
        let p = start + dir * u + n * (side * half);
        [p.x, p.y, z]
    };
    let corners = [(cut.u_lo, cut.z_lo), (cut.u_hi, cut.z_lo), (cut.u_hi, cut.z_hi), (cut.u_lo, cut.z_hi)];
    let edges_before = mesh.edges.len();
    for side in [-1.0, 1.0] {
        for i in 0..4 {
            let ((u0, z0), (u1, z1)) = (corners[i], corners[(i + 1) % 4]);
            push_edge(&mut mesh.edges, at(u0, z0, side), at(u1, z1, side));
        }
    }
    for (u, z) in corners {
        push_edge(&mut mesh.edges, at(u, z, -1.0), at(u, z, 1.0));
    }
    // the opening's outline belongs to the door / window / opening that cuts it: its own range, no triangles
    mesh.ranges.push(ElementRange { element: cut.element, tris: (mesh.triangles.len(), mesh.triangles.len()), edges: (edges_before, mesh.edges.len()) });
    mesh.elements.insert(cut.element);
}

/// Adds one element's solids to the mesh, or records why it has none.
fn append_element_mesh(model: &BimModel, id: ElementId, mesh: &mut Mesh3d, hosts: &BTreeSet<ElementId>, hidden: &BTreeSet<ElementId>, parts: crate::view_categories::PartFilter, styler: Option<&crate::override_style::ViewStyler>) {
    let class = match element_geometry::classify(model, id) {
        Some(c) => c,
        None => return,
    };
    let (tri0, edge0) = (mesh.triangles.len(), mesh.edges.len());
    match element_geometry::geometry_of_in(model, id, parts) {
        Geometry::Solid(bodies) => {
            let colour = element_geometry::class_tone(class);
            let tint = |c: [f32; 3]| styler.map_or(c, |s| s.mesh_colour(id, c));
            let colour = tint(colour);
            match class {
                ElementClass::Wall => {
                    let w = &model.walls[&id];
                    // only a wall that hosts something is asked for its cuts (each asks for the wall's full state)
                    let cuts: Vec<WallCut> = if hosts.contains(&id) { element_geometry::wall_cuts(model, id) } else { Vec::new() };
                    let dir = (w.end - w.start).normalized();
                    match (bodies.first(), dir) {
                        (Some(gross), Some(dir)) if !cuts.is_empty() => {
                            // the wall minus its openings: faces from the pieces, edges from the whole wall and the cuts
                            for p in element_geometry::wall_pieces(gross, w.start, dir, &cuts) {
                                push_body_faces(mesh, &p, colour);
                            }
                            push_body_edges(mesh, gross);
                            let n = dir.perp_ccw();
                            let off: Vec<f64> = gross.outer.iter().map(|p| (*p - w.start).dot(n)).collect();
                            let half = 0.5 * (off.iter().cloned().fold(f64::NEG_INFINITY, f64::max) - off.iter().cloned().fold(f64::INFINITY, f64::min));
                            // the wall keeps the opening of a hidden door; only the door's outline is not drawn
                            for c in cuts.iter().filter(|c| !hidden.contains(&c.element)) {
                                push_cut_edges(mesh, w.start, dir, half, c);
                            }
                            mesh.solids += 1;
                        }
                        _ => bodies.iter().for_each(|b| push_body(mesh, b, colour)),
                    }
                }
                ElementClass::CurtainWall => {
                    let grid = crate::curtains::grid_state(model, id).ok();
                    // the glass first, then the mullions (`element_geometry::curtain_parts`); a wall whose grid has no part is its envelope
                    let split = element_geometry::curtain_parts(model, id).filter(|(g, m)| !(g.is_empty() && m.is_empty()));
                    let glass = match (&split, parts.panels) {
                        (Some((g, _)), true) => g.len(),
                        _ => 0,
                    };
                    for (i, b) in bodies.iter().enumerate() {
                        let tone = if split.is_none() {
                            colour
                        } else if i < glass {
                            tint(element_geometry::tone::GLASS)
                        } else {
                            tint(element_geometry::tone::MULLION)
                        };
                        push_body(mesh, b, tone);
                    }
                    // the grid lines, drawn as edges (a line without mullions has no body of its own)
                    if let (Some(g), true) = (&grid, parts.is_all()) {
                        for line in g.u_lines.iter().chain(g.v_lines.iter()) {
                            if let Ok((a, b)) = g.line_curve(model, id, line) {
                                push_edge(&mut mesh.edges, a, b);
                            }
                        }
                    }
                }
                _ => bodies.iter().for_each(|b| push_body(mesh, b, colour)),
            }
            mesh.elements.insert(id);
            // the cuts of a wall's openings pushed their own ranges inside this span; the element's range is the whole span
            mesh.ranges.push(ElementRange { element: id, tris: (tri0, mesh.triangles.len()), edges: (edge0, mesh.edges.len()) });
        }
        Geometry::NotGenerated(r) | Geometry::Failed(r) => mesh.gaps.push((id, r)),
        Geometry::Void(_) | Geometry::Abstract(_) | Geometry::Parts(_) => {}
    }
}

/// One owner's polyhedral parts, as one range of the mesh.
fn add_parts(mesh: &mut Mesh3d, owner: ElementId, parts: &[Part], tint: &dyn Fn(ElementId, [f32; 3]) -> [f32; 3]) {
    let (tri0, edge0) = (mesh.triangles.len(), mesh.edges.len());
    for p in parts {
        push_polysolid(mesh, &p.solid, tint(owner, element_geometry::part_tone(p.role)));
    }
    mesh.elements.insert(owner);
    mesh.ranges.push(ElementRange { element: owner, tris: (tri0, mesh.triangles.len()), edges: (edge0, mesh.edges.len()) });
}

/// Stairs and railings: each run, landing and railing is drawn under its own id and the stringers under the stairs' (the same ids the
/// section and the diagnosis use). A component the engine cannot build is listed in `gaps` with the reason.
fn add_stairs_and_railings(model: &BimModel, mesh: &mut Mesh3d, hidden: &BTreeSet<ElementId>, styler: Option<&crate::override_style::ViewStyler>) {
    let tint = |id: ElementId, c: [f32; 3]| styler.map_or(c, |s| s.mesh_colour(id, c));
    for s in model.stairs.stairs.keys().filter(|id| !hidden.contains(id)) {
        let parts = crate::stair_solids::stairs_parts(model, *s);
        let mut any = false;
        for (owner, p) in &parts {
            if *owner != *s && hidden.contains(owner) {
                continue;
            }
            add_parts(mesh, *owner, p, &|id, c| tint(*s, tint(id, c)));
            any = true;
        }
        if any {
            mesh.elements.insert(*s);
        }
        let comps = model.stairs.runs_of(*s).into_iter().map(|r| r.id).chain(model.stairs.landings.values().filter(|l| l.stairs == *s).map(|l| l.id));
        for c in comps.filter(|c| !hidden.contains(c) && !parts.iter().any(|(o, _)| o == c)) {
            if let Geometry::NotGenerated(r) | Geometry::Failed(r) = element_geometry::geometry_of(model, c) {
                mesh.gaps.push((c, r));
            }
        }
        if !any {
            if let Geometry::NotGenerated(r) | Geometry::Failed(r) = element_geometry::geometry_of(model, *s) {
                mesh.gaps.push((*s, r));
            }
        }
    }
    for r in model.stairs.railings.keys().chain(model.stairs_scene.path_railings.keys()).filter(|id| !hidden.contains(id)) {
        match element_geometry::geometry_of(model, *r) {
            Geometry::Parts(p) => add_parts(mesh, *r, &p, &tint),
            Geometry::NotGenerated(why) | Geometry::Failed(why) => mesh.gaps.push((*r, why)),
            _ => {}
        }
    }
}

/// The mesh of a 3D view: every element that has a body in the engine (`element_geometry`): walls with their
/// openings cut, floors, ceilings and roofs (sloped ones included), columns, curtain wall glass and mullions, wall
/// sweeps and toposolids. Elements the engine has no body for are listed in `gaps` with the reason, never
/// dropped without a trace. Empty for any other view kind (a plan draws its line work, not this).
///
/// A 3D view has no view range: nothing here is classified by elevation (the only exclusion a 3D view applies is its
/// section box, which this engine does not model; no range or cut plane takes part). The view's own hidden set is the only
/// filter.
pub fn view_mesh(model: &BimModel, view: &View) -> Result<Mesh3d, asterion_core::Failure> {
    let mut mesh = Mesh3d::default();
    if view.kind != crate::model::ViewKind::ThreeD {
        return Ok(mesh);
    }
    let hidden = |id: &ElementId| view.hidden_elements.contains(id);
    let parts = crate::view_categories::part_filter(model, view.id);
    let styler = crate::override_style::ViewStyler::new(model, view.id);
    // every wall is regenerated: one wall index for the pass (same states, joins included)
    crate::joins::with_wall_index(model, || {
        let ids: Vec<ElementId> = model
            .walls
            .keys()
            .chain(model.floors.keys())
            .chain(model.columns.keys())
            .chain(model.annot.sweeps.iter().filter(|(_, s)| s.kind == crate::annot::SweepKind::Sweep).map(|(k, _)| k))
            .chain(model.docs.toposolids.keys())
            .copied()
            .filter(|id| !hidden(id))
            .collect();
        let hosts: BTreeSet<ElementId> = model.hosted_instances.values().map(|i| i.host).chain(model.wall_openings.values().map(|o| o.host)).collect();
        for id in ids {
            append_element_mesh(model, id, &mut mesh, &hosts, &view.hidden_elements, parts, styler.as_ref());
        }
    });
    add_stairs_and_railings(model, &mut mesh, &view.hidden_elements, styler.as_ref());
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element_geometry::tone::WALL;
    use crate::{ops, CompoundStructure, Layer, LayerFunction};
    use crate::model::ViewKind;

    fn square() -> Vec<Vec2> {
        vec![Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0), Vec2::new(4.0, 3.0), Vec2::new(0.0, 3.0)]
    }

    #[test]
    fn a_box_has_twelve_triangles_and_twelve_edges() {
        let mut m = Mesh3d::default();
        push_prism(&mut m, &square(), 0.0, 2.0, WALL);
        // 2 caps x 2 triangles + 4 sides x 2 triangles
        assert_eq!(m.triangle_count(), 12);
        // 4 + 4 loop edges + 4 vertical creases (right angles)
        assert_eq!(m.edge_count(), 12);
        assert_eq!(m.solids, 1);
    }

    #[test]
    fn a_clockwise_outline_gives_outward_normals() {
        let mut ccw = Mesh3d::default();
        push_prism(&mut ccw, &square(), 0.0, 2.0, WALL);
        let mut cw = Mesh3d::default();
        let mut p = square();
        p.reverse();
        push_prism(&mut cw, &p, 0.0, 2.0, WALL);
        // every side normal points away from the centre (2, 1.5)
        for mesh in [&ccw, &cw] {
            for v in mesh.triangles.chunks_exact(9) {
                let (px, py, nx, ny) = (v[0] as f64 - 2.0, v[1] as f64 - 1.5, v[3] as f64, v[4] as f64);
                if nx != 0.0 || ny != 0.0 {
                    assert!(px * nx + py * ny > 0.0, "normal {nx},{ny} at {px},{py}");
                }
            }
        }
    }

    #[test]
    fn a_concave_outline_is_triangulated_without_covering_the_notch() {
        // an L: area 12 - 4 = 8
        let l = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(4.0, 0.0),
            Vec2::new(4.0, 2.0),
            Vec2::new(2.0, 2.0),
            Vec2::new(2.0, 4.0),
            Vec2::new(0.0, 4.0),
        ];
        let tris = triangulate(&l);
        assert_eq!(tris.len(), 4);
        let area: f64 = tris.iter().map(|t| signed_area(&[l[t[0]], l[t[1]], l[t[2]]]).abs()).sum();
        assert!((area - 12.0).abs() < 1e-9, "{area}");
    }

    #[test]
    fn a_three_d_view_exports_the_walls_and_a_plan_exports_nothing() {
        let mut doc = crate::new_document();
        let out = doc.transact("fx", false, |m, cx| {
            let l0 = ops::create_level(m, cx, "L0", 0.0)?;
            let wt = ops::create_wall_type(
                m,
                cx,
                "W200",
                CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?,
            )?;
            for (a, b) in [((0.0, 0.0), (5.0, 0.0)), ((5.0, 0.0), (5.0, 4.0))] {
                ops::create_wall(
                    m,
                    cx,
                    ops::NewWall {
                        wall_type: wt,
                        level: l0,
                        start: Vec2::new(a.0, a.1),
                        end: Vec2::new(b.0, b.1),
                        height: 3.0,
                        base_offset: 0.0,
                        flip: false,
                        structural: false,
                    },
                )?;
            }
            let plan = ops::create_view(m, cx, "Level 0", ViewKind::FloorPlan, Some(l0), None)?;
            let three = ops::create_view(m, cx, "{3D}", ViewKind::ThreeD, None, None)?;
            Ok((plan, three))
        });
        let (plan, three) = out.value.expect("committed");
        let m = doc.model();
        let e3 = view_mesh(m, m.view(three).unwrap()).unwrap();
        assert_eq!(e3.solids, 2);
        assert!(e3.triangle_count() >= 24 && e3.edge_count() >= 16);
        let ep = view_mesh(m, m.view(plan).unwrap()).unwrap();
        assert_eq!(ep, Mesh3d::default());
    }
}
