//! The roof solid: a footprint roof's lower envelope, its perpendicular thickness and its cut-off.
//!
//! Every rule below is OBSERVED unless marked otherwise, and each is pinned by a unit test that replays a measured row.
//!
//! # The model
//!
//! Every slope-defining boundary edge `i` carries a plane that holds the edge at the base
//! elevation and rises into the footprint at its slope `s_i`:
//!
//! ```text
//! z_i(p) = base_z + s_i * d_i(p)          d_i = distance from p to edge i's line, inward
//! ```
//!
//! * **The reference plane is the UNDERSIDE.** OBSERVED: the bottom of a sloped roof sits on the
//!   base (`bbox.min.z = 0.000`), and the body grows UP from it.
//! * **The bottom surface is the lower envelope** `min_i z_i(p)`: a gable is two planes meeting in
//!   a ridge, a hip is four. OBSERVED for one, two and four planes.
//! * **Thickness is PERPENDICULAR**: each plane's top is the plane lifted by `t * sqrt(1 + s_i^2)`
//!   vertically, and the top surface is the lower envelope of those lifted planes (a mitred
//!   offset). OBSERVED: `bbox.max.z = 0.5 * 4 + 0.3 * sqrt(1.25) = 2.3354`.
//! * **`Area` is the underside**, inclined: `plan_area * sqrt(1 + s^2)` summed over the planes.
//!   OBSERVED: 26.8328 = 24 * sqrt(1.25), and for a cut roof 13.4164 = 6 * 2 * sqrt(1.25) - the
//!   underside only up to the cut, not the whole roof.
//! * **`Volume` comes from the SOLID, not from `Area * t`.** OBSERVED: the clipped roof has
//!   `Area` 13.4164, `Area * t` 4.0249 and a real volume of 3.3499, because the perpendicular
//!   thickness reaches the cut plane before the underside does.
//! * **The cut-off is a horizontal plane** that removes everything above it.
//!
//! * Spec R-01a: the Area is the LARGER of the two sides' summed face areas (real, inclined faces). [`RoofSolid::area`] is
//!   that maximum of the underside and the top faces; the horizontal face a cut-off leaves is not counted in the top side
//!   (which faces the side getters return is selected by face-history tags whose names are not specified).
//! * Spec R-01c: the Volume is the solid's; `Area x t` only when the offset is not constant, which no footprint roof
//!   without a shape-edit step meets.
//! * Spec R-02: the cut-off cuts the solid IN PLACE with a horizontal plane at the up-to level elevation plus its offset,
//!   and the volume comes from the cut solid, so it is not `Area x t`.
//!
//! # Chains of unequal slopes (Spec R-01b2, R-04b)
//!
//! Slope-defining edges that follow each other around the footprint form a CHAIN. The vertical separation of the two shells
//! is `t / cos(a_min)` with the chain's smallest slope angle, for every edge of the chain; the top surface is the lower
//! envelope of the edge planes lifted by it. The underside of a steeper facet is its top plane brought down by its own
//! perpendicular thickness `t / cos(a_i)` and the underside is flat at the roof's base where that goes below it (ENGINE
//! CHOICES and: OBSERVED in the vertices of round 10 and round 13, 13 faces / 27 edges / 16 vertices and 9 / 19 / 12,
//! volume to 1e-9). Edges that are not neighbours are separate chains and keep their own thickness.
//!
//! # What is not reproduced
//!
//! A cut-off through a multi-plane roof keeps the unclipped topology counts. Recorded, not hidden.

use asterion_geom::{Vec2, Vec3};

const EPS: f64 = 1e-9;

/// A roof's inputs, all in SI.
#[derive(Clone, Debug)]
pub struct RoofInput<'a> {
    pub boundary: &'a [Vec2],
    /// `(boundary edge index, slope)` of every slope-defining edge. Edge `i` runs
    /// `boundary[i] -> boundary[i + 1]`.
    pub slopes: Vec<(usize, f64)>,
    /// Elevation the slope-defining edges are held at.
    pub base_z: f64,
    /// `(boundary edge index, offset)`: an edge whose plane starts `offset` above `base_z` (the segment's Offset; OBSERVED:
    /// edge 0 at 0.3 lifts its eave to 3.3 and moves the ridge to y = 1.7, Area and Volume unchanged). Edges not listed
    /// start at `base_z`.
    pub offsets: Vec<(usize, f64)>,
    /// Perpendicular thickness of the type.
    pub thickness: f64,
    /// Horizontal cut-off plane, if any.
    pub cutoff_z: Option<f64>,
}

/// Everything the engine reports about a roof, derived from its solid.
#[derive(Clone, Debug, PartialEq)]
pub struct RoofSolid {
    /// The reported `Area`: the underside, inclined, up to the cut.
    pub area: f64,
    pub volume: f64,
    pub surface_area: f64,
    pub plan_area: f64,
    pub bbox_min: Vec3,
    pub bbox_max: Vec3,
    pub face_count: usize,
    pub edge_count: usize,
    pub vertex_count: usize,
    /// Lowest point of the UNCLIPPED underside: what the cut-off must stay above.
    pub lowest_bottom_z: f64,
    /// Distinct bottom planes carrying area. 1 for a flat or shed roof, 2 for a gable.
    pub active_planes: usize,
}

/// `z = c0 + g . p`.
#[derive(Clone, Copy, Debug)]
struct Lin {
    c0: f64,
    g: Vec2,
}

impl Lin {
    fn at(&self, p: Vec2) -> f64 {
        self.c0 + self.g.x * p.x + self.g.y * p.y
    }
    fn plus(&self, dz: f64) -> Lin {
        Lin { c0: self.c0 + dz, g: self.g }
    }
    fn minus(&self, o: &Lin) -> Lin {
        Lin { c0: self.c0 - o.c0, g: Vec2::new(self.g.x - o.g.x, self.g.y - o.g.y) }
    }
}

/// One roof plane: the bottom and the top surface of the facet that hangs on one slope-defining edge.
///
/// `bottom` is the edge's plane lowered by `lowering` (0 unless the edge's chain holds a flatter edge,
/// see [`chain_of`]); `top` is the edge's plane lifted by the chain's vertical thickness.
#[derive(Clone, Copy, Debug)]
struct Plane {
    bottom: Lin,
    top: Lin,
    slope: f64,
    /// How far the bottom lies below the edge's own plane (Spec R-01b2: the chain's smallest slope sets the vertical
    /// thickness of every curve of the chain, the steeper facets keep their perpendicular thickness).
    lowering: f64,
}

type Poly = Vec<Vec2>;

fn signed_area(p: &[Vec2]) -> f64 {
    let n = p.len();
    let mut s = 0.0;
    for i in 0..n {
        let a = p[i];
        let b = p[(i + 1) % n];
        s += a.x * b.y - b.x * a.y;
    }
    s / 2.0
}

fn area(p: &[Vec2]) -> f64 {
    signed_area(p).abs()
}

fn centroid(p: &[Vec2]) -> Vec2 {
    let a = signed_area(p);
    if a.abs() < EPS {
        let n = p.len().max(1) as f64;
        return Vec2::new(p.iter().map(|v| v.x).sum::<f64>() / n, p.iter().map(|v| v.y).sum::<f64>() / n);
    }
    let n = p.len();
    let (mut cx, mut cy) = (0.0, 0.0);
    for i in 0..n {
        let (u, v) = (p[i], p[(i + 1) % n]);
        let cr = u.x * v.y - v.x * u.y;
        cx += (u.x + v.x) * cr;
        cy += (u.y + v.y) * cr;
    }
    Vec2::new(cx / (6.0 * a), cy / (6.0 * a))
}

/// Keep the part of `poly` where `f(p) >= 0` (Sutherland-Hodgman against one half-plane).
fn clip(poly: &[Vec2], f: &Lin) -> Poly {
    let n = poly.len();
    if n == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(n + 2);
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        let (fa, fb) = (f.at(a), f.at(b));
        let (ina, inb) = (fa >= -EPS, fb >= -EPS);
        if ina {
            out.push(a);
        }
        if (ina && !inb) || (!ina && inb) {
            let t = fa / (fa - fb);
            out.push(Vec2::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t));
        }
    }
    if out.len() < 3 || area(&out) < EPS {
        Vec::new()
    } else {
        out
    }
}

fn dedupe_planes(planes: Vec<Plane>) -> Vec<Plane> {
    let mut out: Vec<Plane> = Vec::new();
    for p in planes {
        let dup = out.iter().any(|q| {
            (q.bottom.c0 - p.bottom.c0).abs() < EPS
                && (q.bottom.g.x - p.bottom.g.x).abs() < EPS
                && (q.bottom.g.y - p.bottom.g.y).abs() < EPS
        });
        if !dup {
            out.push(p);
        }
    }
    out
}

fn chain_of(n: usize, slopes: &[(usize, f64)]) -> Vec<usize> {
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let mut defining = vec![false; n];
    for &(i, _) in slopes {
        if i < n {
            defining[i] = true;
        }
    }
    for i in 0..n {
        let j = (i + 1) % n;
        if defining[i] && defining[j] {
            let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
            if ri != rj {
                parent[ri.max(rj)] = ri.min(rj);
            }
        }
    }
    slopes.iter().map(|&(i, _)| if i < n { find(&mut parent, i) } else { usize::MAX }).collect()
}

/// The roof planes of `input` (one per slope-defining edge, or the single flat plane) and the floor of
/// the underside, or `None` when an edge or a slope is not usable.
///
/// Spec R-01b2: the vertical separation of the two shells at a slope edge is `t / cos(a_min)`, where `a_min` is the
/// SMALLEST slope angle of the edge's chain - the same for every curve of the chain (the plumb-cut eave type puts all of it
/// on the top shell), so the top surface is the envelope of the edge planes lifted by that one distance.
///
/// That flat floor at the roof's base (bbox min z = base) is the same kind of ENGINE CHOICE.
fn build_planes(input: &RoofInput<'_>) -> Option<(Vec<Plane>, Option<f64>)> {
    let n = input.boundary.len();
    if n < 3 {
        return None;
    }
    let footprint = input.boundary;
    let ccw = signed_area(footprint) > 0.0;
    let chains = chain_of(n, &input.slopes);
    let mut smallest: Vec<(usize, f64)> = Vec::new();
    for (k, &(i, s)) in input.slopes.iter().enumerate() {
        if i >= n || !s.is_finite() {
            return None;
        }
        match smallest.iter_mut().find(|(c, _)| *c == chains[k]) {
            Some(e) => e.1 = e.1.min(s.abs()),
            None => smallest.push((chains[k], s.abs())),
        }
    }
    let mut planes: Vec<Plane> = Vec::new();
    for (k, &(i, s)) in input.slopes.iter().enumerate() {
        let a = footprint[i];
        let b = footprint[(i + 1) % n];
        let dir = (b - a).normalized()?;
        // Inward normal: the left of the edge for a CCW footprint, the right for a CW one.
        let mut nrm = Vec2::new(-dir.y, dir.x);
        if !ccw {
            nrm = Vec2::new(-nrm.x, -nrm.y);
        }
        let g = Vec2::new(nrm.x * s, nrm.y * s);
        let off = input.offsets.iter().find(|(e, _)| *e == i).map_or(0.0, |(_, o)| *o);
        let c0 = input.base_z + off - (a.x * g.x + a.y * g.y);
        let own = Lin { c0, g };
        let vt = input.thickness * (1.0 + s * s).sqrt();
        let s_min = smallest.iter().find(|(c, _)| *c == chains[k]).map_or(s.abs(), |e| e.1);
        let chain_vt = input.thickness * (1.0 + s_min * s_min).sqrt();
        if vt - chain_vt > 1e-12 {
            let lowering = vt - chain_vt;
            planes.push(Plane { bottom: own.plus(-lowering), top: own.plus(chain_vt), slope: s, lowering });
        } else {
            planes.push(Plane { bottom: own, top: own.plus(vt), slope: s, lowering: 0.0 });
        }
    }
    if planes.is_empty() {
        let bottom = Lin { c0: input.base_z, g: Vec2::new(0.0, 0.0) };
        planes.push(Plane { bottom, top: bottom.plus(input.thickness), slope: 0.0, lowering: 0.0 });
    }
    let floor = planes.iter().any(|q| q.lowering > 0.0).then_some(input.base_z);
    Some((dedupe_planes(planes), floor))
}

/// Elevation of the roof's UNDERSIDE (the lower envelope of its bottom planes, before any cut-off)
/// above plan point `p`; `None` when the inputs give no plane (a degenerate defining edge).
pub fn underside_z_at(input: &RoofInput<'_>, p: Vec2) -> Option<f64> {
    let (planes, floor) = build_planes(input)?;
    let z = planes.iter().map(|q| q.bottom.at(p)).fold(f64::INFINITY, f64::min);
    let z = if z.is_finite() { z } else { input.base_z };
    Some(floor.map_or(z, |f| z.max(f)))
}

/// A bottom region of the roof solid: a convex plan polygon whose underside is `lin`.
#[derive(Clone, Debug)]
struct BottomRegion {
    /// Index of the plane whose facet (or floor part) this is.
    plane: usize,
    poly: Poly,
    lin: Lin,
    /// Slope of the underside facet (0 for the flat floor).
    slope: f64,
    /// The flat floor region (the underside at the roof's base, see [`build_planes`]).
    flat: bool,
}

/// Corner points of a convex plan polygon: consecutive duplicates and collinear points removed.
fn corners_of(poly: &[Vec2]) -> Vec<Vec2> {
    let mut pts: Vec<Vec2> = Vec::new();
    for p in poly {
        if pts.last().map_or(true, |q| (q.x - p.x).abs() > 1e-9 || (q.y - p.y).abs() > 1e-9) {
            pts.push(*p);
        }
    }
    while pts.len() > 1 && (pts[0].x - pts[pts.len() - 1].x).abs() <= 1e-9 && (pts[0].y - pts[pts.len() - 1].y).abs() <= 1e-9 {
        pts.pop();
    }
    loop {
        let n = pts.len();
        if n < 3 {
            break;
        }
        let drop = (0..n).find(|&i| {
            let (a, b, c) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            (b - a).cross(c - b).abs() <= 1e-9 * (b - a).length().max(1.0) * (c - b).length().max(1.0)
        });
        match drop {
            Some(i) => {
                pts.remove(i);
            }
            None => break,
        }
    }
    pts
}

/// Build a roof's solid, or `None` when the footprint or the slopes give no body.
pub fn roof_solid(input: &RoofInput<'_>) -> Option<RoofSolid> {
    let n = input.boundary.len();
    if n < 3 || !(input.thickness > 0.0) {
        return None;
    }
    let footprint: Poly = input.boundary.to_vec();
    let plan_area = area(&footprint);
    if plan_area < EPS {
        return None;
    }

    let (planes, floor) = build_planes(input)?;

    let bottom_env = |p: Vec2| planes.iter().map(|q| q.bottom.at(p)).fold(f64::INFINITY, f64::min);
    let bottom_at = |p: Vec2| floor.map_or(bottom_env(p), |f| bottom_env(p).max(f));
    let top_at = |p: Vec2| planes.iter().map(|q| q.top.at(p)).fold(f64::INFINITY, f64::min);
    let cut = input.cutoff_z;
    let thickness_at = |p: Vec2| {
        let top = cut.map_or(top_at(p), |c| top_at(p).min(c));
        (top - bottom_at(p)).max(0.0)
    };

    // Regions: bottom region (a facet, or the flat floor) AND top plane j active, both inside the footprint.
    struct Piece {
        poly: Poly,
        bottom: BottomRegion,
        top: usize,
    }
    let mut bottom_regions: Vec<BottomRegion> = Vec::new();
    for (i, qi) in planes.iter().enumerate() {
        let mut r = footprint.clone();
        for (k, qk) in planes.iter().enumerate() {
            if k != i && !r.is_empty() {
                r = clip(&r, &qk.bottom.minus(&qi.bottom));
            }
        }
        if let Some(c) = cut {
            if !r.is_empty() {
                // bottom <= c  <=>  c - z_i >= 0
                r = clip(&r, &Lin { c0: c - qi.bottom.c0, g: Vec2::new(-qi.bottom.g.x, -qi.bottom.g.y) });
            }
        }
        if r.is_empty() {
            continue;
        }
        match floor {
            None => bottom_regions.push(BottomRegion { plane: i, poly: r, lin: qi.bottom, slope: qi.slope, flat: false }),
            Some(f) => {
                // above the floor: the facet; below it: the flat floor
                let above = clip(&r, &Lin { c0: qi.bottom.c0 - f, g: qi.bottom.g });
                if !above.is_empty() {
                    bottom_regions.push(BottomRegion { plane: i, poly: above, lin: qi.bottom, slope: qi.slope, flat: false });
                }
                let flat = clip(&r, &Lin { c0: f - qi.bottom.c0, g: Vec2::new(-qi.bottom.g.x, -qi.bottom.g.y) });
                if !flat.is_empty() {
                    bottom_regions.push(BottomRegion {
                        plane: i,
                        poly: flat,
                        lin: Lin { c0: f, g: Vec2::new(0.0, 0.0) },
                        slope: 0.0,
                        flat: true,
                    });
                }
            }
        }
    }
    if bottom_regions.is_empty() {
        return None;
    }

    let mut pieces: Vec<Piece> = Vec::new();
    for br in &bottom_regions {
        for (j, qj) in planes.iter().enumerate() {
            let mut p = br.poly.clone();
            for (k, qk) in planes.iter().enumerate() {
                if k != j && !p.is_empty() {
                    p = clip(&p, &qk.top.minus(&qj.top));
                }
            }
            if !p.is_empty() {
                pieces.push(Piece { poly: p, bottom: br.clone(), top: j });
            }
        }
    }

    // Quantities.
    let mut area_under = 0.0; // inclined underside, up to the cut
    let mut volume = 0.0;
    let mut top_surface = 0.0; // inclined top face that survives the cut
    let mut cut_face = 0.0; // horizontal face the cut leaves
    let mut z_lo = f64::INFINITY;
    let mut z_hi = f64::NEG_INFINITY;
    // Every vertex where the thickness can change slope along a footprint edge: region corners and
    // the places a cut-off crosses a lifted top.
    let mut kinks: Vec<Vec2> = Vec::new();
    let mut xs = (f64::INFINITY, f64::NEG_INFINITY);
    let mut ys = (f64::INFINITY, f64::NEG_INFINITY);

    for br in &bottom_regions {
        area_under += area(&br.poly) * (1.0 + br.slope * br.slope).sqrt();
    }
    for pc in &pieces {
        let (qb, qt) = (&pc.bottom, &planes[pc.top]);
        match cut {
            None => {
                let a = area(&pc.poly);
                let c = centroid(&pc.poly);
                volume += a * (qt.top.at(c) - qb.lin.at(c));
                top_surface += a * (1.0 + qt.slope * qt.slope).sqrt();
                for v in &pc.poly {
                    z_hi = z_hi.max(qt.top.at(*v));
                }
            }
            Some(c) => {
                // Where the lifted top is at or below the cut, the full thickness remains.
                let below = clip(&pc.poly, &Lin { c0: c - qt.top.c0, g: Vec2::new(-qt.top.g.x, -qt.top.g.y) });
                kinks.extend(below.iter().copied());
                if !below.is_empty() {
                    let a = area(&below);
                    let ctr = centroid(&below);
                    volume += a * (qt.top.at(ctr) - qb.lin.at(ctr));
                    top_surface += a * (1.0 + qt.slope * qt.slope).sqrt();
                    for v in &below {
                        z_hi = z_hi.max(qt.top.at(*v));
                    }
                }
                // Where the top is above the cut but the underside is not, the cut caps the body.
                let above = clip(&pc.poly, &Lin { c0: qt.top.c0 - c, g: qt.top.g });
                kinks.extend(above.iter().copied());
                if !above.is_empty() {
                    let a = area(&above);
                    cut_face += a;
                    let ctr = centroid(&above);
                    volume += a * (c - qb.lin.at(ctr));
                    z_hi = z_hi.max(c);
                }
            }
        }
        kinks.extend(pc.poly.iter().copied());
        for v in &pc.poly {
            z_lo = z_lo.min(qb.lin.at(*v));
            xs = (xs.0.min(v.x), xs.1.max(v.x));
            ys = (ys.0.min(v.y), ys.1.max(v.y));
        }
    }
    if let Some(c) = cut {
        z_hi = z_hi.min(c);
    }

    // The lowest underside point of the UNCLIPPED roof: the cut-off must stay above it.
    let lowest_bottom_z = footprint.iter().map(|p| bottom_at(*p)).fold(f64::INFINITY, f64::min);

    // Side faces: each footprint edge, integrated along its length (the thickness is piecewise
    // linear, with kinks at region vertices that lie on the edge).
    let mut sides = 0.0;
    let mut side_faces = 0usize;
    for e in 0..n {
        let a = footprint[e];
        let b = footprint[(e + 1) % n];
        let len = (b - a).length();
        if len < EPS {
            continue;
        }
        let dir = (b - a) * (1.0 / len);
        let mut ts = vec![0.0, len];
        for v in &kinks {
            let t = (*v - a).dot(dir);
            let off = (*v - a).cross(dir).abs();
            if off < 1e-7 && t > EPS && t < len - EPS {
                ts.push(t);
            }
        }
        ts.sort_by(|x, y| x.total_cmp(y));
        ts.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
        let mut edge_area = 0.0;
        for w in ts.windows(2) {
            let (p0, p1) = (a + dir * w[0], a + dir * w[1]);
            // Only the part of the edge that still carries a body (bottom below the cut).
            let (t0, t1) = (thickness_at(p0), thickness_at(p1));
            edge_area += (t0 + t1) / 2.0 * (w[1] - w[0]);
        }
        if edge_area > 1e-9 {
            sides += edge_area;
            side_faces += 1;
        }
    }
    let surface_area = area_under + top_surface + cut_face + sides;

    // Topology.
    let active_planes = bottom_regions.iter().filter(|r| !r.flat).count();
    let (face_count, edge_count, vertex_count);
    if floor.is_some() && cut.is_none() {
        // A chain of unequal slopes: the underside has sloped facets and a flat floor, the top its own facets (Spec
        // R-01b2). Faces: sloped bottom facets + flat floor + top facets + sides; the vertices are the corners of those
        // faces; a closed body of disc faces has E = V + F - 2.
        let (v, f) = mixed_topology(&planes, &bottom_regions, floor, side_faces);
        face_count = f;
        vertex_count = v;
        edge_count = v + f - 2;
    } else if cut.is_none() || active_planes > 1 {
        // Unclipped (or a clipped multi-plane roof, whose counts are not reproduced).
        let mut verts: Vec<Vec2> = Vec::new();
        for r in &bottom_regions {
            for v in &r.poly {
                if !verts.iter().any(|u| (u.x - v.x).abs() < 1e-7 && (u.y - v.y).abs() < 1e-7) {
                    verts.push(*v);
                }
            }
        }
        let v = verts.len() * 2;
        let f = n + 2 * active_planes;
        face_count = f;
        vertex_count = v;
        edge_count = v + f - 2;
    } else {
        // One bottom plane and a cut: build the vertex set explicitly.
        let r0 = &bottom_regions[0];
        let q = &planes[r0.plane];
        let r = &r0.poly;
        let c = cut.expect("checked");
        let mut pts3: Vec<Vec3> = Vec::new();
        let mut push = |p: Vec3| {
            if !pts3.iter().any(|u| (u.x - p.x).abs() < 1e-7 && (u.y - p.y).abs() < 1e-7 && (u.z - p.z).abs() < 1e-7) {
                pts3.push(p);
            }
        };
        for v in r {
            push(Vec3::new(v.x, v.y, q.bottom.at(*v)));
            push(Vec3::new(v.x, v.y, q.top.at(*v).min(c)));
        }
        // Where the lifted top crosses the cut inside the region.
        let below = clip(r, &Lin { c0: c - q.top.c0, g: Vec2::new(-q.top.g.x, -q.top.g.y) });
        for v in &below {
            push(Vec3::new(v.x, v.y, q.top.at(*v).min(c)));
        }
        let has_top = area(&below) > 1e-9;
        let has_cut = cut_face > 1e-9;
        let mut f = 1 + side_faces;
        if has_top {
            f += 1;
        }
        if has_cut {
            f += 1;
        }
        face_count = f;
        vertex_count = pts3.len();
        edge_count = vertex_count + f - 2;
    }

    Some(RoofSolid {
        // Spec R-01a: the larger of the two sides
        area: area_under.max(top_surface),
        volume,
        surface_area,
        plan_area: bottom_regions.iter().map(|r| area(&r.poly)).sum(),
        bbox_min: Vec3::new(xs.0, ys.0, z_lo),
        bbox_max: Vec3::new(xs.1, ys.1, z_hi),
        face_count,
        edge_count,
        vertex_count,
        lowest_bottom_z,
        active_planes,
    })
}

/// One convex piece of a roof's body: a plan polygon whose underside and top are the planes
/// `z = c0 + gx x + gy y` (`[c0, gx, gy]`). Together the pieces ARE the solid [`roof_solid`] measures
/// (same planes, same regions, same cut-off): the sum of `area x (top - bottom at the centroid)` over
/// them is its `volume`.
#[derive(Clone, Debug, PartialEq)]
pub struct RoofPiece {
    pub poly: Vec<Vec2>,
    pub bottom: [f64; 3],
    pub top: [f64; 3],
}

/// The convex pieces of the roof's solid, for drawing it (3D, sections, elevations): the bottom regions
/// split by the active top plane, and where a cut-off caps the body the part under the cut plane with a
/// horizontal top. `None` exactly when [`roof_solid`] gives no body.
pub fn roof_pieces(input: &RoofInput<'_>) -> Option<Vec<RoofPiece>> {
    roof_solid(input)?;
    let (planes, floor) = build_planes(input)?;
    let cut = input.cutoff_z;
    let lin = |l: &Lin| [l.c0, l.g.x, l.g.y];
    let mut bottom_regions: Vec<(Poly, Lin)> = Vec::new();
    for (i, qi) in planes.iter().enumerate() {
        let mut r: Poly = input.boundary.to_vec();
        for (k, qk) in planes.iter().enumerate() {
            if k != i && !r.is_empty() {
                r = clip(&r, &qk.bottom.minus(&qi.bottom));
            }
        }
        if let Some(c) = cut {
            if !r.is_empty() {
                r = clip(&r, &Lin { c0: c - qi.bottom.c0, g: Vec2::new(-qi.bottom.g.x, -qi.bottom.g.y) });
            }
        }
        if r.is_empty() {
            continue;
        }
        match floor {
            None => bottom_regions.push((r, qi.bottom)),
            Some(f) => {
                let above = clip(&r, &Lin { c0: qi.bottom.c0 - f, g: qi.bottom.g });
                if !above.is_empty() {
                    bottom_regions.push((above, qi.bottom));
                }
                let flat = clip(&r, &Lin { c0: f - qi.bottom.c0, g: Vec2::new(-qi.bottom.g.x, -qi.bottom.g.y) });
                if !flat.is_empty() {
                    bottom_regions.push((flat, Lin { c0: f, g: Vec2::new(0.0, 0.0) }));
                }
            }
        }
    }
    let mut out: Vec<RoofPiece> = Vec::new();
    for (poly, bottom) in &bottom_regions {
        for (j, qj) in planes.iter().enumerate() {
            let mut p = poly.clone();
            for (k, qk) in planes.iter().enumerate() {
                if k != j && !p.is_empty() {
                    p = clip(&p, &qk.top.minus(&qj.top));
                }
            }
            if p.is_empty() {
                continue;
            }
            match cut {
                None => out.push(RoofPiece { poly: p, bottom: lin(bottom), top: lin(&qj.top) }),
                Some(c) => {
                    let below = clip(&p, &Lin { c0: c - qj.top.c0, g: Vec2::new(-qj.top.g.x, -qj.top.g.y) });
                    if !below.is_empty() {
                        out.push(RoofPiece { poly: below, bottom: lin(bottom), top: lin(&qj.top) });
                    }
                    let above = clip(&p, &Lin { c0: qj.top.c0 - c, g: qj.top.g });
                    if !above.is_empty() {
                        out.push(RoofPiece { poly: above, bottom: lin(bottom), top: [c, 0.0, 0.0] });
                    }
                }
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Vertex and face counts of an uncut roof whose underside has a flat floor (see [`build_planes`]).
fn mixed_topology(planes: &[Plane], bottom_regions: &[BottomRegion], floor: Option<f64>, side_faces: usize) -> (usize, usize) {
    use asterion_geom::{Region, Tolerances};
    let tol = Tolerances::default();
    let mut faces = side_faces;
    let mut pts3: Vec<Vec3> = Vec::new();
    let mut push = |p: Vec2, z: f64| {
        let p3 = Vec3::new(p.x, p.y, z);
        if !pts3.iter().any(|u| (u.x - p3.x).abs() < 1e-7 && (u.y - p3.y).abs() < 1e-7 && (u.z - p3.z).abs() < 1e-7) {
            pts3.push(p3);
        }
    };
    // sloped bottom facets
    for br in bottom_regions.iter().filter(|r| !r.flat) {
        faces += 1;
        for v in corners_of(&br.poly) {
            push(v, br.lin.at(v));
        }
    }
    // the flat floor: the parts that touch are one face
    let flat: Vec<Region> = bottom_regions
        .iter()
        .filter(|r| r.flat)
        .filter_map(|r| Region::from_polygon(&corners_of(&r.poly), &tol).ok())
        .collect();
    if let Ok(merged) = asterion_geom::union_all(&flat, &tol) {
        for m in merged {
            faces += 1;
            let simple = m.outer().simplified(&tol).unwrap_or_else(|_| m.outer().clone());
            for v in simple.corner_points() {
                push(v, floor.unwrap_or(0.0));
            }
        }
    }
    // top facets: the cell of the lower envelope of the top planes of each plane
    let mut top_cells = 0usize;
    for (j, qj) in planes.iter().enumerate() {
        // the cell of plane j is convex; it is cut into pieces by the bottom regions, so take their hull
        let mut cell: Option<Poly> = None;
        for br in bottom_regions {
            let mut p = br.poly.clone();
            for (k, qk) in planes.iter().enumerate() {
                if k != j && !p.is_empty() {
                    p = clip(&p, &qk.top.minus(&qj.top));
                }
            }
            if p.is_empty() {
                continue;
            }
            cell = Some(match cell {
                None => p,
                Some(prev) => convex_hull(&prev, &p),
            });
        }
        if let Some(c) = cell {
            top_cells += 1;
            for v in corners_of(&c) {
                push(v, qj.top.at(v));
            }
        }
    }
    (pts3.len(), faces + top_cells)
}

/// Convex hull of two convex plan polygons that tile a convex region together (Andrew's monotone chain).
fn convex_hull(a: &[Vec2], b: &[Vec2]) -> Poly {
    let mut pts: Vec<Vec2> = a.iter().chain(b.iter()).copied().collect();
    pts.sort_by(|p, q| p.x.total_cmp(&q.x).then(p.y.total_cmp(&q.y)));
    pts.dedup_by(|p, q| (p.x - q.x).abs() < 1e-9 && (p.y - q.y).abs() < 1e-9);
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: Vec2, p: Vec2, q: Vec2| (p - o).cross(q - o);
    let mut lower: Vec<Vec2> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 1e-12 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<Vec2> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 1e-12 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(w: f64, d: f64) -> Vec<Vec2> {
        vec![Vec2::new(0.0, 0.0), Vec2::new(w, 0.0), Vec2::new(w, d), Vec2::new(0.0, d)]
    }

    fn near(a: f64, b: f64, tol: f64, what: &str) {
        assert!((a - b).abs() <= tol, "{what}: got {a}, want {b}");
    }

    fn solid(slopes: &[(usize, f64)], base: f64, t: f64, cut: Option<f64>) -> RoofSolid {
        let b = rect(6.0, 4.0);
        roof_solid(&RoofInput { boundary: &b, slopes: slopes.to_vec(), base_z: base, offsets: Vec::new(), thickness: t, cutoff_z: cut })
            .expect("a roof")
    }

    /// Round 34: the gable 0.5 / 0.5 on edges 0 and 2 (6 x 4, 0.3, base 3.0) with edge 0's Offset at 0.3: the ridge rises
    /// to 4.4854 (from 4.3354), Area and Volume do not change (26.8328 / 8.0498).
    #[test]
    fn a_segment_offset_lifts_its_plane_and_moves_the_ridge() {
        let b = rect(6.0, 4.0);
        let r = roof_solid(&RoofInput { boundary: &b, slopes: vec![(0, 0.5), (2, 0.5)], base_z: 3.0, offsets: vec![(0, 0.3)], thickness: 0.3, cutoff_z: None }).unwrap();
        near(r.bbox_max.z, 4.485410196624968, 1e-9, "ridge + t");
        near(r.area, 26.832815729997478, 1e-9, "area");
        near(r.volume, 8.049844718999243, 1e-9, "volume");
    }

    /// Row `flat-roof-6x4-t300` of round 10.
    #[test]
    fn a_flat_roof_grows_up_from_its_base() {
        let s = solid(&[], 0.0, 0.3, None);
        near(s.area, 24.0, 1e-9, "area");
        near(s.volume, 7.2, 1e-9, "volume");
        near(s.bbox_max.z, 0.3, 1e-9, "top");
        near(s.bbox_min.z, 0.0, 1e-9, "bottom");
        assert_eq!((s.face_count, s.edge_count, s.vertex_count), (6, 12, 8));
        near(s.surface_area, 54.0, 1e-9, "surface");
    }

    /// Row `slope-0.5-edge-0`: Area 26.8328, Volume 8.0498, bbox top 2.3354, bottom ON the base,
    /// surface 60.3738, 6 faces / 12 edges / 8 vertices.
    #[test]
    fn a_single_slope_is_the_inclined_underside_with_perpendicular_thickness() {
        let s = solid(&[(0, 0.5)], 0.0, 0.3, None);
        near(s.area, 26.832815729997478, 1e-9, "area");
        near(s.volume, 8.049844718999237, 1e-9, "volume = Area x t");
        near(s.bbox_min.z, 0.0, 1e-9, "the underside sits ON the base");
        near(s.bbox_max.z, 2.0 + 0.3 * 1.25_f64.sqrt(), 1e-9, "top = rise + perpendicular t");
        near(s.surface_area, 60.3738, 1e-3, "surface");
        assert_eq!((s.face_count, s.edge_count, s.vertex_count), (6, 12, 8));
        near(s.lowest_bottom_z, 0.0, 1e-9, "lowest underside");
    }

    /// Row `gable-two-opposite-edges`: same Area and Volume, ridge halves the height, 8/18/12.
    #[test]
    fn a_gable_is_two_planes_meeting_in_a_ridge() {
        let s = solid(&[(0, 0.5), (2, 0.5)], 0.0, 0.3, None);
        near(s.area, 26.832815729997478, 1e-9, "area");
        near(s.volume, 8.049844718999237, 1e-9, "volume");
        near(s.bbox_max.z, 1.0 + 0.3 * 1.25_f64.sqrt(), 1e-9, "ridge + t");
        assert_eq!((s.face_count, s.edge_count, s.vertex_count), (8, 18, 12));
        assert_eq!(s.active_planes, 2);
    }

    /// Row `slope-written-through-instance-param` (0.25): Area 24.7386, top 1.3092.
    #[test]
    fn the_instance_slope_parameter_is_a_ratio_not_an_angle() {
        let s = solid(&[(0, 0.25)], 0.0, 0.3, None);
        near(s.area, 24.738633753705958, 1e-9, "area");
        near(s.bbox_max.z, 1.0 + 0.3 * 1.0625_f64.sqrt(), 1e-9, "top");
    }

    /// Row `hip-all-four-edges`: slopes 0.25, 0.5, 0.5, 0.5, one chain, so the vertical thickness is 0.3 / cos(atan 0.25)
    /// everywhere (Spec R-01b2): Area, Volume, surface, top and the 13 faces / 27 edges / 16 vertices of the reference
    /// product (the steeper facets' undersides start 0.0523 m inside their edges and a flat floor lies between).
    #[test]
    fn a_hip_of_a_mixed_slope_chain_follows_the_smallest_slope() {
        let s = solid(&[(0, 0.25), (1, 0.5), (2, 0.5), (3, 0.5)], 0.0, 0.3, None);
        near(s.area, 25.74694359414261, 1e-9, "area");
        near(s.volume, 7.719688284366454, 1e-9, "volume");
        near(s.bbox_max.z, 0.9758995885879913, 1e-9, "top = ridge 2/3 + 0.3 / cos(atan 0.25)");
        near(s.bbox_min.z, 0.0, 1e-9, "bottom");
        near(s.surface_area, 57.626928594656796, 1e-9, "surface");
        assert_eq!((s.face_count, s.edge_count, s.vertex_count), (13, 27, 16));
    }

    /// Row `two-adjacent-edges-0.5-and-0.25`: edges 0 (0.5) and 1 (0.25) form a chain; 9 faces / 19 edges / 12 vertices,
    /// Area 25.5239519948, Volume 7.6571855984, top 1.80923.
    #[test]
    fn two_adjacent_edges_of_different_slopes_are_one_chain() {
        let s = solid(&[(0, 0.5), (1, 0.25)], 0.0, 0.3, None);
        near(s.area, 25.523951994815278, 1e-9, "area");
        near(s.volume, 7.6571855984445785, 1e-9, "volume");
        near(s.bbox_max.z, 1.8092329219213237, 1e-9, "top");
        near(s.surface_area, 57.3014265428299, 1e-9, "surface");
        assert_eq!((s.face_count, s.edge_count, s.vertex_count), (9, 19, 12));
    }

    /// Two slope-defining edges that are NOT neighbours are two chains: each keeps its own thickness
    /// (round 13 `gable-unequal-0.25-and-0.5`: 8 faces / 18 edges, top 0.3092 at y = 0 and 0.3354 at y = 4).
    #[test]
    fn edges_that_are_not_neighbours_are_separate_chains() {
        let s = solid(&[(0, 0.25), (2, 0.5)], 0.0, 0.3, None);
        near(s.area, 25.43669441246979, 1e-9, "area");
        near(s.volume, 7.628267324897298, 1e-9, "volume");
        near(s.bbox_max.z, 0.984625346822539, 1e-9, "top");
        assert_eq!((s.face_count, s.edge_count, s.vertex_count), (8, 18, 12));
    }

    /// Row `cutoff-at-4.5-crossing-the-roof` (base 3.5, cut 4.5): the clipped roof breaks
    /// `Volume = Area x t`. Area 13.4164, Volume 3.3499, bbox y 0..2, z 3.5..4.5, 6/12/8,
    /// surface 29.4868.
    #[test]
    fn a_cutoff_clips_by_a_horizontal_plane_and_the_volume_comes_from_the_solid() {
        let s = solid(&[(0, 0.5)], 3.5, 0.3, Some(4.5));
        near(s.area, 13.416407864998723, 1e-9, "area: the underside up to the cut");
        near(s.volume, 3.3499223594996184, 1e-9, "volume of the SOLID");
        assert!((s.area * 0.3 - s.volume).abs() > 0.5, "Volume != Area x t here");
        near(s.bbox_max.y, 2.0, 1e-9, "the cut stops the underside at y = 2");
        near(s.bbox_min.z, 3.5, 1e-9, "bottom");
        near(s.bbox_max.z, 4.5, 1e-9, "top = the cut");
        near(s.surface_area, 29.4868, 1e-3, "surface");
        assert_eq!((s.face_count, s.edge_count, s.vertex_count), (6, 12, 8));
    }

    /// A cut above the whole roof changes nothing (row `cutoff-at-6.0-above-the-roof`).
    #[test]
    fn a_cutoff_above_the_roof_has_no_effect() {
        let s = solid(&[(0, 0.5)], 3.5, 0.3, Some(6.0));
        near(s.area, 26.83281572999748, 1e-9, "area");
        near(s.volume, 8.049844718999251, 1e-9, "volume");
        assert_eq!((s.face_count, s.edge_count), (6, 12));
    }

    /// Row `thicken-type-to-500`: Volume 4.8332 on the clipped roof.
    #[test]
    fn thickening_a_clipped_roof_moves_only_the_volume() {
        let s = solid(&[(0, 0.5)], 3.5, 0.5, Some(4.5));
        near(s.area, 13.416407864998723, 1e-9, "area unchanged");
        near(s.volume, 4.833203932499363, 1e-9, "volume");
    }

    /// Row `after-delete-cutoff-level-L2` (cut 4.0, t 0.5): the lifted top is entirely above the
    /// cut, so the body is a triangular wedge: 5 faces / 9 edges / 6 vertices, Volume 1.5.
    #[test]
    fn a_cut_that_removes_the_whole_top_leaves_a_wedge() {
        let s = solid(&[(0, 0.5)], 3.5, 0.5, Some(4.0));
        near(s.area, 6.708203932499367, 1e-9, "area");
        near(s.volume, 1.4999999999999984, 1e-9, "volume");
        near(s.bbox_max.y, 1.0, 1e-9, "y extent");
        assert_eq!((s.face_count, s.edge_count, s.vertex_count), (5, 9, 6));
        near(s.surface_area, 16.2082, 1e-3, "surface");
    }

    /// The cut-off is refused when it is at or below the lowest underside point.
    #[test]
    fn the_lowest_underside_is_what_the_cutoff_must_clear() {
        let s = solid(&[(0, 0.5)], 4.5, 0.3, None);
        near(s.lowest_bottom_z, 4.5, 1e-9, "roof raised to 4.5");
    }

    /// Spec R-01a: the Area is the larger side. On the clipped roof the underside (13.4164) is larger than the top faces
    /// that survive the cut; on an unclipped single slope both sides are the same inclined area.
    #[test]
    fn the_area_is_the_larger_of_the_two_sides() {
        let s = solid(&[(0, 0.5)], 3.5, 0.3, Some(4.5));
        near(s.area, 13.416407864998723, 1e-9, "the underside wins");
        let u = solid(&[(0, 0.5)], 0.0, 0.3, None);
        near(u.area, 24.0 * 1.25_f64.sqrt(), 1e-9, "both sides equal");
    }
}
