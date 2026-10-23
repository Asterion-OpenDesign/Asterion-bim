//! A closed polyhedron with planar faces: the body type of the elements whose shape is not "a plan region between two
//! planes" (stair treads with slanted risers, stringers, rails, balusters).
//!
//! [`crate::element_geometry::Body`] is a vertical prism (side faces always vertical). A slanted riser, a stringer cut square
//! to its slope or a rail swept along a slope has inclined side faces, so it needs this type. Every number is closed-form
//! (divergence theorem over the faces, the vertices' extremes): nothing is measured on a tessellation. Triangles exist only to
//! be drawn ([`PolySolid::triangles`]) and cut figures only to be drawn ([`PolySolid::slice`]).
//!
//! Faces are planar polygons without holes, listed counter-clockwise seen from outside; faces that share an edge share the
//! two vertex indices, so the edge and vertex counts are exact (the Euler characteristic of a genus-0 solid is 2).

use asterion_geom::Vec2;
use std::collections::{BTreeMap, HashMap};

/// What a solid of an assembly is, for the tone it is drawn with and the symbol a plan gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PartRole {
    /// The treads and risers of a run (one fused body per run).
    Step,
    /// The tread of a landing.
    Landing,
    /// A stringer or carriage.
    Support,
    /// The top rail or hand rail of a railing.
    TopRail,
    /// A rail below the top rail.
    Rail,
    /// A baluster or a post.
    Baluster,
    /// A glass or solid infill panel.
    Panel,
}

/// The volume and surface area of a solid that is drawn as a polyhedron but is not one (a round tube drawn as a 16-gon prism): the analytic values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Exact {
    pub volume: f64,
    pub area: f64,
}

/// One solid of an assembly (a stairs, a railing) with its role.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub role: PartRole,
    pub solid: PolySolid,
    /// The analytic volume and area when the polyhedron only draws the member; `None` when the polyhedron IS the member.
    pub exact: Option<Exact>,
}

impl Part {
    pub fn new(role: PartRole, solid: PolySolid) -> Part {
        Part { role, solid, exact: None }
    }
    pub fn with_exact(mut self, volume: f64, area: f64) -> Part {
        self.exact = Some(Exact { volume, area });
        self
    }
    /// The volume of the member: the analytic one when it has one, else the polyhedron's.
    pub fn volume(&self) -> f64 {
        self.exact.map_or_else(|| self.solid.volume(), |e| e.volume)
    }
    /// The surface area of the member.
    pub fn area(&self) -> f64 {
        self.exact.map_or_else(|| self.solid.surface_area(), |e| e.area)
    }
}

/// A closed polyhedron.
#[derive(Clone, Debug, PartialEq)]
pub struct PolySolid {
    vertices: Vec<[f64; 3]>,
    faces: Vec<Vec<usize>>,
}

/// The vertical plane a profile lives in: `s` runs along `dir` (a plan unit vector) from `origin`, `z` is the world elevation,
/// and the lateral coordinate `u` runs to the left of `dir`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfileFrame {
    pub origin: Vec2,
    pub dir: Vec2,
}

impl ProfileFrame {
    pub fn lateral(&self) -> Vec2 {
        self.dir.perp_ccw()
    }
    /// The world point of profile coordinates `(s, z)` at lateral station `u`.
    pub fn point(&self, s: f64, z: f64, u: f64) -> [f64; 3] {
        let p = self.origin + self.dir * s + self.lateral() * u;
        [p.x, p.y, z]
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

impl PolySolid {
    /// Builds a solid from vertices and counter-clockwise faces; when the faces wind inward (negative volume) they are all
    /// reversed.
    pub fn new(vertices: Vec<[f64; 3]>, faces: Vec<Vec<usize>>) -> PolySolid {
        let mut s = PolySolid { vertices, faces };
        if s.volume() < 0.0 {
            for f in &mut s.faces {
                f.reverse();
            }
        }
        s
    }

    /// The prism of a profile `(s, z)` (counter-clockwise, any number of vertices, simple) extruded across the lateral
    /// stations `u0 < u1`. `None` for a degenerate input.
    pub fn from_profile(frame: &ProfileFrame, profile: &[[f64; 2]], u0: f64, u1: f64) -> Option<PolySolid> {
        let mut p: Vec<[f64; 2]> = profile.to_vec();
        if p.len() > 3 && p[0] == p[p.len() - 1] {
            p.pop();
        }
        let n = p.len();
        if n < 3 || !(u1 > u0) {
            return None;
        }
        let area: f64 = (0..n).map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1]).sum::<f64>() * 0.5;
        if area.abs() < 1e-14 {
            return None;
        }
        if area < 0.0 {
            p.reverse();
        }
        let mut vertices = Vec::with_capacity(2 * n);
        for q in &p {
            vertices.push(frame.point(q[0], q[1], u0));
        }
        for q in &p {
            vertices.push(frame.point(q[0], q[1], u1));
        }
        let mut faces: Vec<Vec<usize>> = Vec::with_capacity(n + 2);
        faces.push((0..n).collect());
        faces.push((0..n).rev().map(|i| n + i).collect());
        for i in 0..n {
            let j = (i + 1) % n;
            faces.push(vec![i, n + i, n + j, j]);
        }
        Some(PolySolid::new(vertices, faces))
    }

    /// The vertical prism of a plan polygon between two elevations.
    pub fn from_prism(footprint: &[Vec2], z0: f64, z1: f64) -> Option<PolySolid> {
        let mut p: Vec<Vec2> = footprint.to_vec();
        if p.len() > 3 && p[0] == p[p.len() - 1] {
            p.pop();
        }
        let n = p.len();
        if n < 3 || !(z1 > z0) {
            return None;
        }
        let area: f64 = (0..n).map(|i| p[i].cross(p[(i + 1) % n])).sum::<f64>() * 0.5;
        if area.abs() < 1e-14 {
            return None;
        }
        if area < 0.0 {
            p.reverse();
        }
        let mut vertices: Vec<[f64; 3]> = p.iter().map(|q| [q.x, q.y, z0]).collect();
        vertices.extend(p.iter().map(|q| [q.x, q.y, z1]));
        let mut faces: Vec<Vec<usize>> = vec![(0..n).rev().collect(), (0..n).map(|i| n + i).collect()];
        for i in 0..n {
            let j = (i + 1) % n;
            faces.push(vec![i, j, n + j, n + i]);
        }
        Some(PolySolid::new(vertices, faces))
    }

    /// An axis-aligned box.
    pub fn aabb_box(lo: [f64; 3], hi: [f64; 3]) -> Option<PolySolid> {
        PolySolid::from_prism(&[Vec2::new(lo[0], lo[1]), Vec2::new(hi[0], lo[1]), Vec2::new(hi[0], hi[1]), Vec2::new(lo[0], hi[1])], lo[2], hi[2])
    }

    pub fn vertices(&self) -> &[[f64; 3]] {
        &self.vertices
    }
    pub fn faces(&self) -> &[Vec<usize>] {
        &self.faces
    }
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }
    pub fn face_count(&self) -> usize {
        self.faces.len()
    }
    /// The number of distinct edges (a pair of vertex indices shared by faces counts once).
    pub fn edge_count(&self) -> usize {
        self.edge_faces().len()
    }

    fn edge_faces(&self) -> BTreeMap<(usize, usize), Vec<usize>> {
        let mut m: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
        for (fi, f) in self.faces.iter().enumerate() {
            for k in 0..f.len() {
                let (a, b) = (f[k], f[(k + 1) % f.len()]);
                m.entry((a.min(b), a.max(b))).or_default().push(fi);
            }
        }
        m
    }

    /// The signed sum `sum (p0 . (pi x pi+1))` over a face's fan, i.e. 2 x (area vector . p0).
    fn face_fan(&self, f: &[usize]) -> ([f64; 3], f64) {
        // Newell area vector (twice the area) and the volume term
        let mut a = [0.0; 3];
        let p0 = self.vertices[f[0]];
        let mut vol6 = 0.0;
        for k in 1..f.len() - 1 {
            let (p1, p2) = (self.vertices[f[k]], self.vertices[f[k + 1]]);
            let c = cross(sub(p1, p0), sub(p2, p0));
            a[0] += c[0];
            a[1] += c[1];
            a[2] += c[2];
            vol6 += dot(p0, cross(p1, p2));
        }
        (a, vol6)
    }

    /// The volume (divergence theorem over the faces). Exact for planar faces.
    pub fn volume(&self) -> f64 {
        self.faces.iter().map(|f| self.face_fan(f).1).sum::<f64>() / 6.0
    }

    /// The surface area: the sum of the faces' areas.
    pub fn surface_area(&self) -> f64 {
        self.faces.iter().map(|f| norm(self.face_fan(f).0) * 0.5).sum()
    }

    /// The area of the faces whose outward normal has `|nz|` above `min_abs_nz` and the sign of `up` (true = facing up).
    pub fn area_facing(&self, up: bool, min_abs_nz: f64) -> f64 {
        self.faces
            .iter()
            .map(|f| {
                let (a, _) = self.face_fan(f);
                let l = norm(a);
                if l < 1e-18 {
                    return 0.0;
                }
                let nz = a[2] / l;
                if (up && nz >= min_abs_nz) || (!up && nz <= -min_abs_nz) {
                    l * 0.5
                } else {
                    0.0
                }
            })
            .sum()
    }

    /// The world box `(min, max)`.
    pub fn aabb(&self) -> ([f64; 3], [f64; 3]) {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for v in &self.vertices {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        (lo, hi)
    }

    /// The unit outward normal of a face (`None` for a degenerate face).
    pub fn face_normal(&self, face: usize) -> Option<[f64; 3]> {
        let (a, _) = self.face_fan(&self.faces[face]);
        let l = norm(a);
        (l > 1e-18).then(|| [a[0] / l, a[1] / l, a[2] / l])
    }

    /// The solid moved by `d`.
    pub fn translated(&self, d: [f64; 3]) -> PolySolid {
        PolySolid { vertices: self.vertices.iter().map(|v| [v[0] + d[0], v[1] + d[1], v[2] + d[2]]).collect(), faces: self.faces.clone() }
    }

    /// The triangles of every face, outward (counter-clockwise from outside), each with its face's unit normal. For drawing only.
    pub fn triangles(&self) -> Vec<([[f64; 3]; 3], [f64; 3])> {
        let mut out = Vec::new();
        for (fi, f) in self.faces.iter().enumerate() {
            let Some(n) = self.face_normal(fi) else { continue };
            if f.len() == 3 {
                out.push(([self.vertices[f[0]], self.vertices[f[1]], self.vertices[f[2]]], n));
                continue;
            }
            // project onto the face's plane in a basis (a, b, n) right-handed so a counter-clockwise face stays counter-clockwise
            let axis = if n[0].abs() <= n[1].abs() && n[0].abs() <= n[2].abs() {
                [1.0, 0.0, 0.0]
            } else if n[1].abs() <= n[2].abs() {
                [0.0, 1.0, 0.0]
            } else {
                [0.0, 0.0, 1.0]
            };
            let a = cross(n, axis);
            let la = norm(a);
            let a = [a[0] / la, a[1] / la, a[2] / la];
            let b = cross(n, a);
            let poly: Vec<Vec2> = f.iter().map(|&i| Vec2::new(dot(self.vertices[i], a), dot(self.vertices[i], b))).collect();
            for t in crate::mesh3d::triangulate(&poly) {
                out.push(([self.vertices[f[t[0]]], self.vertices[f[t[1]]], self.vertices[f[t[2]]]], n));
            }
        }
        out
    }

    /// The edges where the two faces that meet differ by more than `degrees` (and, for an edge of one face only, none): the
    /// crease lines of a drawing. Coplanar faces left apart by a boolean do not draw a line.
    pub fn crease_edges(&self, degrees: f64) -> Vec<([f64; 3], [f64; 3])> {
        let cos_limit = degrees.to_radians().cos();
        let mut out = Vec::new();
        for ((a, b), fs) in self.edge_faces() {
            let draw = match fs.as_slice() {
                [f, g] => match (self.face_normal(*f), self.face_normal(*g)) {
                    (Some(n), Some(m)) => dot(n, m) < cos_limit,
                    _ => true,
                },
                _ => true,
            };
            if draw {
                out.push((self.vertices[a], self.vertices[b]));
            }
        }
        out
    }

    /// The outline loops of the solid seen from a viewer in direction `toward_viewer` (a vector from the solid to the eye): the edges
    /// between a face that looks at the viewer and one that does not (edge-on faces do not), chained into closed polylines in world
    /// coordinates. For drawing the part of an element that lies behind a section plane. Exact for a closed solid.
    pub fn silhouette(&self, toward_viewer: [f64; 3]) -> Vec<Vec<[f64; 3]>> {
        let l = norm(toward_viewer);
        if l < 1e-18 {
            return Vec::new();
        }
        let v = [toward_viewer[0] / l, toward_viewer[1] / l, toward_viewer[2] / l];
        let front: Vec<bool> = (0..self.faces.len()).map(|f| self.face_normal(f).map_or(false, |n| dot(n, v) > 1e-9)).collect();
        let mut segs = Vec::new();
        for ((a, b), fs) in self.edge_faces() {
            let n_front = fs.iter().filter(|f| front[**f]).count();
            if n_front >= 1 && n_front < fs.len() {
                segs.push((self.vertices[a], self.vertices[b]));
            } else if fs.len() == 1 && n_front == 1 {
                segs.push((self.vertices[a], self.vertices[b]));
            }
        }
        chain_segments(segs)
    }

    /// The loops in which the plane through `origin` with `normal` cuts the solid (closed polylines in world coordinates, in
    /// no particular orientation). Vertices on the plane count as on its positive side, so a plane through an edge cuts
    /// consistently. Faces lying in the plane contribute nothing. For drawing a cut (a plan at its cut plane, a section).
    pub fn slice(&self, origin: [f64; 3], normal: [f64; 3]) -> Vec<Vec<[f64; 3]>> {
        let ln = norm(normal);
        if ln < 1e-18 {
            return Vec::new();
        }
        let nrm = [normal[0] / ln, normal[1] / ln, normal[2] / ln];
        let d: Vec<f64> = self.vertices.iter().map(|v| dot(sub(*v, origin), nrm)).collect();
        let mut segs: Vec<([f64; 3], [f64; 3])> = Vec::new();
        for (fi, f) in self.faces.iter().enumerate() {
            let n = f.len();
            if f.iter().all(|&i| d[i].abs() < 1e-12) {
                continue;
            }
            if f.iter().all(|&i| d[i] >= 0.0) || f.iter().all(|&i| d[i] < 0.0) {
                continue;
            }
            let Some(fnorm) = self.face_normal(fi) else { continue };
            let along = cross(fnorm, nrm);
            let la = norm(along);
            if la < 1e-12 {
                continue;
            }
            let along = [along[0] / la, along[1] / la, along[2] / la];
            let mut pts: Vec<([f64; 3], f64)> = Vec::new();
            for k in 0..n {
                let (i, j) = (f[k], f[(k + 1) % n]);
                if (d[i] >= 0.0) != (d[j] >= 0.0) {
                    let t = d[i] / (d[i] - d[j]);
                    let (p, q) = (self.vertices[i], self.vertices[j]);
                    let x = [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t, p[2] + (q[2] - p[2]) * t];
                    pts.push((x, dot(x, along)));
                }
            }
            pts.sort_by(|a, b| a.1.total_cmp(&b.1));
            for pair in pts.chunks_exact(2) {
                if (pair[1].1 - pair[0].1).abs() > 1e-12 {
                    // direction: the face's inside is on its left; keep the segment oriented so loops of one solid agree
                    segs.push((pair[0].0, pair[1].0));
                }
            }
        }
        chain_segments(segs)
    }
}

fn key(p: [f64; 3]) -> (i64, i64, i64) {
    ((p[0] * 1e7).round() as i64, (p[1] * 1e7).round() as i64, (p[2] * 1e7).round() as i64)
}

/// Joins unordered segments end to end into closed loops (open chains are dropped).
fn chain_segments(segs: Vec<([f64; 3], [f64; 3])>) -> Vec<Vec<[f64; 3]>> {
    let mut at: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for (i, (a, b)) in segs.iter().enumerate() {
        at.entry(key(*a)).or_default().push(i);
        at.entry(key(*b)).or_default().push(i);
    }
    let mut used = vec![false; segs.len()];
    let mut loops = Vec::new();
    for start in 0..segs.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let mut pts = vec![segs[start].0, segs[start].1];
        let first = key(segs[start].0);
        loop {
            let tail = *pts.last().expect("non-empty");
            let k = key(tail);
            if k == first && pts.len() > 2 {
                pts.pop();
                loops.push(pts);
                break;
            }
            let next = at.get(&k).and_then(|v| v.iter().copied().find(|&j| !used[j]));
            match next {
                Some(j) => {
                    used[j] = true;
                    let (a, b) = segs[j];
                    pts.push(if key(a) == k { b } else { a });
                }
                None => break,
            }
        }
    }
    loops
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> ProfileFrame {
        ProfileFrame { origin: Vec2::new(1.0, 2.0), dir: Vec2::new(1.0, 0.0) }
    }

    #[test]
    fn a_box_has_the_closed_form_numbers() {
        let b = PolySolid::aabb_box([0.0, 0.0, 0.0], [2.0, 3.0, 4.0]).unwrap();
        assert!((b.volume() - 24.0).abs() < 1e-12);
        assert!((b.surface_area() - 2.0 * (6.0 + 8.0 + 12.0)).abs() < 1e-12);
        assert_eq!((b.vertex_count(), b.face_count(), b.edge_count()), (8, 6, 12));
        assert_eq!(b.aabb(), ([0.0; 3], [2.0, 3.0, 4.0]));
    }

    #[test]
    fn a_profile_extrusion_is_outward_whatever_the_profile_winding() {
        // a right triangle in (s, z), clockwise on purpose
        let f = frame();
        let a = PolySolid::from_profile(&f, &[[0.0, 0.0], [0.0, 1.0], [2.0, 0.0]], -0.5, 0.5).unwrap();
        let b = PolySolid::from_profile(&f, &[[0.0, 0.0], [2.0, 0.0], [0.0, 1.0]], -0.5, 0.5).unwrap();
        assert!((a.volume() - 1.0).abs() < 1e-12 && (b.volume() - 1.0).abs() < 1e-12);
        // Euler: V - E + F = 2
        assert_eq!(a.vertex_count() as i64 - a.edge_count() as i64 + a.face_count() as i64, 2);
        // every face normal points away from the centroid
        let (lo, hi) = a.aabb();
        let nv = a.vertices().len() as f64;
        let c = [0, 1, 2].map(|k| a.vertices().iter().map(|v| v[k]).sum::<f64>() / nv);
        for (i, f) in a.faces().iter().enumerate() {
            let n = a.face_normal(i).unwrap();
            assert!(dot(n, sub(a.vertices()[f[0]], c)) > 0.0, "face {i} points inward");
        }
        // lateral stations run to the LEFT of the direction (+y for dir +x)
        assert!((lo[1] - 1.5).abs() < 1e-12 && (hi[1] - 2.5).abs() < 1e-12);
    }

    #[test]
    fn a_non_convex_profile_has_the_volume_of_its_area() {
        // an L: 3 x 1 plus 1 x 1 on top at the left, area 4
        let f = frame();
        let l = [[0.0, 0.0], [3.0, 0.0], [3.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0]];
        let s = PolySolid::from_profile(&f, &l, 0.0, 2.0).unwrap();
        assert!((s.volume() - 8.0).abs() < 1e-12);
        let tri: f64 = s.triangles().iter().map(|(t, _)| norm(cross(sub(t[1], t[0]), sub(t[2], t[0]))) * 0.5).sum();
        assert!((tri - s.surface_area()).abs() < 1e-9, "the triangles cover the faces");
    }

    #[test]
    fn a_slice_is_the_cross_section() {
        let b = PolySolid::aabb_box([0.0, 0.0, 0.0], [2.0, 3.0, 4.0]).unwrap();
        let loops = b.slice([0.0, 0.0, 1.0], [0.0, 0.0, 1.0]);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].len(), 4);
        assert!(loops[0].iter().all(|p| (p[2] - 1.0).abs() < 1e-12));
        // a plane that misses it
        assert!(b.slice([0.0, 0.0, 9.0], [0.0, 0.0, 1.0]).is_empty());
    }

    #[test]
    fn creases_skip_coplanar_neighbours() {
        let b = PolySolid::aabb_box([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]).unwrap();
        assert_eq!(b.crease_edges(25.0).len(), 12);
    }
}
