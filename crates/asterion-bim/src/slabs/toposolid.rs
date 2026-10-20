//! Toposolids (OBSERVED, public API).
//!
//! * `Toposolid.Create` takes a closed profile, points, or both, a type and a level. The template has one type,
//!   "Toposolid 1", with a single 0.3 m structure layer. The top surface is flat at the level (plus the `Height Offset
//!   From Level`) when there are no points, else the triangulation of the points; the bottom is flat, one layer thickness
//!   under the level.
//! * The solid reports: volume = area x (top - bottom) summed over the surface, faces = the facets of the top + the bottom +
//!   one per side (a flat top is one facet), edges and end points as of a prism with the triangulation on top.
//! * A profile with a crossing or an edge shorter than the short-curve tolerance is refused, as `Create` does.
//! * `CreateSubDivision(profiles)` makes a toposolid element (no type, no level) that is a 0.3048 m prism over the
//!   profile standing on the host's top surface; a profile outside the host posts an error at commit.
//!
//! ENGINE CHOICES (named, unmeasured): the offset adds to the heights of the points as well; the bottom follows the level
//! and not the lowest point; a subdivision on a sloped host stands on the highest corner of its profile; the triangulation is
//! Delaunay (the measured cases have only one possible triangulation).

use crate::model::BimModel;
use crate::spec::codes;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use asterion_geom::{Region, Tolerances, Vec2};
use serde::{Deserialize, Serialize};

pub type P3 = [f64; 3];

/// The one template type: name and thickness (OBSERVED).
pub const TYPE_NAME: &str = "Toposolid 1";
pub const TYPE_THICKNESS_M: f64 = 0.3;
/// The thickness of a subdivision (1 ft; OBSERVED).
pub const SUBDIVISION_THICKNESS_M: f64 = 0.3048;
/// `Application.ShortCurveTolerance` in metres (0.00256 ft).
const SHORT_CURVE_M: f64 = 0.000_780_3;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Toposolid {
    pub id: ElementId,
    /// `None` for a subdivision.
    pub level: Option<ElementId>,
    /// Points of the surface (absolute heights, metres); empty for a flat toposolid.
    pub points: Vec<P3>,
    /// The closed profile in plan (empty: the hull of the points).
    pub profile: Vec<[f64; 2]>,
    /// `Height Offset From Level`.
    pub offset: f64,
    /// The toposolid a subdivision was cut in.
    pub host: Option<ElementId>,
}

/// What a toposolid reports.
#[derive(Clone, Debug, PartialEq)]
pub struct TopoState {
    pub bbox: (P3, P3),
    pub volume: f64,
    pub area: f64,
    pub faces: usize,
    pub edges: usize,
    pub endpoints: usize,
    /// The points as `SlabShapeEditor.SlabShapeVertices` lists them: the hull counter-clockwise from the vertex with the
    /// greatest x (least y among those), then the interior points as given.
    pub vertices: Vec<P3>,
}

fn bad(code: &str, msg: &str) -> Failure {
    Failure::error(code, msg.to_owned())
}

fn v2(p: [f64; 2]) -> Vec2 {
    Vec2::new(p[0], p[1])
}

fn cross(o: Vec2, a: Vec2, b: Vec2) -> f64 {
    (a - o).cross(b - o)
}

/// Whether the closed polygon crosses itself.
fn self_intersects(poly: &[Vec2]) -> bool {
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        for j in (i + 1)..n {
            if j == i + 1 || (i == 0 && j == n - 1) {
                continue;
            }
            let (c, d) = (poly[j], poly[(j + 1) % n]);
            let (d1, d2, d3, d4) = (cross(a, b, c), cross(a, b, d), cross(c, d, a), cross(c, d, b));
            if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
                return true;
            }
        }
    }
    false
}

fn validate_profile(profile: &[[f64; 2]]) -> Result<(), Failure> {
    let poly: Vec<Vec2> = profile.iter().map(|p| v2(*p)).collect();
    if poly.len() < 3 {
        return Err(bad(codes::TOPOSOLID_INVALID, "The input curve loops cannot compose a valid boundary."));
    }
    for i in 0..poly.len() {
        if (poly[(i + 1) % poly.len()] - poly[i]).length() < SHORT_CURVE_M {
            return Err(bad(codes::TOPOSOLID_INVALID, "Curve length is too small for the reference product's tolerance (as identified by Application.ShortCurveTolerance)."));
        }
    }
    if self_intersects(&poly) {
        return Err(bad(codes::TOPOSOLID_INVALID, "The input curve loops cannot compose a valid boundary."));
    }
    Ok(())
}

/// `Toposolid.Create`: from a profile, points, or both.
pub fn create_toposolid(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    level: ElementId,
    points: Vec<P3>,
    profile: Vec<[f64; 2]>,
) -> Result<ElementId, Failure> {
    m.level(level)?;
    if points.is_empty() && profile.is_empty() {
        return Err(bad(codes::TOPOSOLID_INVALID, "The boundary loops are empty, so they cannot form a boundary."));
    }
    if !profile.is_empty() {
        validate_profile(&profile)?;
    }
    if !points.is_empty() && points.len() < 3 {
        return Err(bad(codes::TOPOSOLID_INVALID, "At least three points are needed."));
    }
    let id = cx.new_id();
    m.docs.toposolids.insert(id, Toposolid { id, level: Some(level), points, profile, offset: 0.0, host: None });
    Ok(id)
}

/// `Toposolid.CreateSubDivision`: the profile must lie inside the host; otherwise an error is posted for the commit
/// (OBSERVED: "An error occurred during the sub-divide action. The sub-divide can not be completed.").
///
/// Spec TOPO-03: a subdivision of a subdivision is refused (the host carries a host id), and the host level must be a
/// Level; both before the sketch is made. No containment test is made in that operation (the commit-time error above is the
/// reference product's, measured).
pub fn create_subdivision(m: &mut BimModel, cx: &mut TxnContext<'_>, host: ElementId, profile: Vec<[f64; 2]>) -> Result<ElementId, Failure> {
    let Some(h) = m.docs.toposolids.get(&host) else {
        return Err(Failure::error(codes::NOT_FOUND, "No such toposolid.").on(&[host]));
    };
    if h.host.is_some() {
        return Err(bad(codes::TOPOSOLID_INVALID, "Can not create a toposolid sub-division on another sub-division.").on(&[host]));
    }
    if !h.level.map_or(false, |l| m.levels.contains_key(&l)) {
        return Err(bad(codes::TOPOSOLID_INVALID, "The toposolid does not have a valid level.").on(&[host]));
    }
    validate_profile(&profile)?;
    let id = cx.new_id();
    let inside = outline(m, host).map_or(false, |o| {
        let poly: Vec<Vec2> = profile.iter().map(|p| v2(*p)).collect();
        match (Region::from_polygon(&o, &Tolerances::default()), Region::from_polygon(&poly, &Tolerances::default())) {
            (Ok(h), Ok(p)) => h.intersection(&p, &Tolerances::default()).map_or(false, |v| (v.iter().map(Region::area).sum::<f64>() - p.area()).abs() < 1e-6),
            _ => false,
        }
    });
    if !inside {
        cx.report(Failure::error(codes::TOPOSOLID_SUBDIVISION_FAILED, "An error occurred during the sub-divide action. The sub-divide can not be completed.").on(&[id]));
        return Ok(id);
    }
    m.docs.toposolids.insert(id, Toposolid { id, level: None, points: Vec::new(), profile, offset: 0.0, host: Some(host) });
    Ok(id)
}

/// `Height Offset From Level`.
pub fn set_offset(m: &mut BimModel, id: ElementId, offset: f64) -> Result<(), Failure> {
    let t = m.docs.toposolids.get_mut(&id).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such toposolid.").on(&[id]))?;
    if t.host.is_some() {
        return Err(bad(codes::TOPOSOLID_INVALID, "A subdivision has no height offset."));
    }
    t.offset = offset;
    Ok(())
}

/// The plan outline: the profile, or the hull of the points (counter-clockwise).
pub fn outline(m: &BimModel, id: ElementId) -> Option<Vec<Vec2>> {
    let t = m.docs.toposolids.get(&id)?;
    if !t.profile.is_empty() {
        return Some(ccw(t.profile.iter().map(|p| v2(*p)).collect()));
    }
    let pts: Vec<Vec2> = t.points.iter().map(|p| Vec2::new(p[0], p[1])).collect();
    Some(hull(&pts))
}

fn area2(poly: &[Vec2]) -> f64 {
    (0..poly.len()).map(|i| poly[i].cross(poly[(i + 1) % poly.len()])).sum::<f64>()
}

fn ccw(mut poly: Vec<Vec2>) -> Vec<Vec2> {
    if area2(&poly) < 0.0 {
        poly.reverse();
    }
    poly
}

/// Convex hull, counter-clockwise.
fn hull(pts: &[Vec2]) -> Vec<Vec2> {
    let mut p: Vec<Vec2> = pts.to_vec();
    p.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    p.dedup_by(|a, b| (*a - *b).length() < 1e-9);
    if p.len() < 3 {
        return p;
    }
    let mut lower: Vec<Vec2> = Vec::new();
    for q in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], *q) <= 1e-12 {
            lower.pop();
        }
        lower.push(*q);
    }
    let mut upper: Vec<Vec2> = Vec::new();
    for q in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], *q) <= 1e-12 {
            upper.pop();
        }
        upper.push(*q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Delaunay triangulation (Bowyer-Watson) of the points: index triples.
fn delaunay(pts: &[Vec2]) -> Vec<[usize; 3]> {
    let n = pts.len();
    if n < 3 {
        return Vec::new();
    }
    let (mut lo, mut hi) = (pts[0], pts[0]);
    for p in pts {
        lo = Vec2::new(lo.x.min(p.x), lo.y.min(p.y));
        hi = Vec2::new(hi.x.max(p.x), hi.y.max(p.y));
    }
    let d = (hi.x - lo.x).max(hi.y - lo.y).max(1.0) * 20.0;
    let mid = Vec2::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0);
    let mut all: Vec<Vec2> = pts.to_vec();
    all.push(Vec2::new(mid.x - d, mid.y - d));
    all.push(Vec2::new(mid.x + d, mid.y - d));
    all.push(Vec2::new(mid.x, mid.y + d));
    let mut tris: Vec<[usize; 3]> = vec![[n, n + 1, n + 2]];
    let in_circle = |t: &[usize; 3], p: Vec2| -> bool {
        let (a, b, c) = (all[t[0]], all[t[1]], all[t[2]]);
        let orient = cross(a, b, c);
        let (ax, ay, bx, by, cx, cy) = (a.x - p.x, a.y - p.y, b.x - p.x, b.y - p.y, c.x - p.x, c.y - p.y);
        let det = (ax * ax + ay * ay) * (bx * cy - cx * by) - (bx * bx + by * by) * (ax * cy - cx * ay) + (cx * cx + cy * cy) * (ax * by - bx * ay);
        if orient > 0.0 {
            det > 1e-12
        } else {
            det < -1e-12
        }
    };
    for (i, p) in pts.iter().enumerate() {
        let bad_t: Vec<[usize; 3]> = tris.iter().copied().filter(|t| in_circle(t, *p)).collect();
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for t in &bad_t {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                if let Some(k) = edges.iter().position(|e| (e.0 == b && e.1 == a) || (e.0 == a && e.1 == b)) {
                    edges.remove(k);
                } else {
                    edges.push((a, b));
                }
            }
        }
        tris.retain(|t| !bad_t.contains(t));
        for (a, b) in edges {
            tris.push([a, b, i]);
        }
    }
    tris.retain(|t| t.iter().all(|i| *i < n));
    tris
}

fn plane_through(a: P3, b: P3, c: P3) -> Option<(f64, f64, f64)> {
    // z = gx x + gy y + k
    let (ux, uy, uz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
    let (vx, vy, vz) = (c[0] - a[0], c[1] - a[1], c[2] - a[2]);
    let det = ux * vy - uy * vx;
    if det.abs() < 1e-12 {
        return None;
    }
    let gx = (uz * vy - uy * vz) / det;
    let gy = (ux * vz - uz * vx) / det;
    Some((gx, gy, a[2] - gx * a[0] - gy * a[1]))
}

fn centroid(poly: &[Vec2]) -> Vec2 {
    let a2 = area2(poly);
    if a2.abs() < 1e-18 {
        return poly[0];
    }
    let (mut cx, mut cy) = (0.0, 0.0);
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let w = p.cross(q);
        cx += (p.x + q.x) * w;
        cy += (p.y + q.y) * w;
    }
    Vec2::new(cx / (3.0 * a2), cy / (3.0 * a2))
}

/// The state of a toposolid.
pub fn toposolid_state(m: &BimModel, id: ElementId) -> Option<TopoState> {
    let t = m.docs.toposolids.get(&id)?;
    if let Some(host) = t.host {
        return subdivision_state(m, t, host);
    }
    let level_z = m.levels.get(&t.level?)?.elevation;
    let top0 = level_z + t.offset;
    let bottom = top0 - TYPE_THICKNESS_M;
    let poly = outline(m, id)?;
    let n = poly.len();
    let tol = Tolerances::default();
    let region = Region::from_polygon(&poly, &tol).ok()?;
    let plan_area = region.area();
    let perimeter: f64 = (0..n).map(|i| (poly[(i + 1) % n] - poly[i]).length()).sum();
    let (lo, hi) = poly.iter().fold((poly[0], poly[0]), |(l, h), p| (Vec2::new(l.x.min(p.x), l.y.min(p.y)), Vec2::new(h.x.max(p.x), h.y.max(p.y))));
    if t.points.is_empty() {
        return Some(TopoState {
            bbox: ([lo.x, lo.y, bottom], [hi.x, hi.y, top0]),
            volume: plan_area * TYPE_THICKNESS_M,
            area: 2.0 * plan_area + perimeter * TYPE_THICKNESS_M,
            faces: n + 2,
            edges: 3 * n,
            endpoints: 2 * n,
            vertices: Vec::new(),
        });
    }
    let pts3: Vec<P3> = t.points.iter().map(|p| [p[0], p[1], p[2] + t.offset]).collect();
    let pts2: Vec<Vec2> = pts3.iter().map(|p| Vec2::new(p[0], p[1])).collect();
    let tris = delaunay(&pts2);
    let (mut volume, mut top_area, mut facets) = (0.0, 0.0, 0usize);
    for tr in &tris {
        let tri_poly = [pts2[tr[0]], pts2[tr[1]], pts2[tr[2]]];
        let Ok(tri_region) = Region::from_polygon(&tri_poly, &tol) else { continue };
        let Ok(parts) = region.intersection(&tri_region, &tol) else { continue };
        let Some((gx, gy, k)) = plane_through(pts3[tr[0]], pts3[tr[1]], pts3[tr[2]]) else { continue };
        for part in parts {
            let a = part.area();
            if a < 1e-12 {
                continue;
            }
            let c = centroid(&part.outer_corner_points());
            volume += a * (gx * c.x + gy * c.y + k - bottom);
            top_area += a * (1.0 + gx * gx + gy * gy).sqrt();
            facets += 1;
        }
    }
    // the side facets: along each edge of the outline, from the bottom to the surface
    let height_at = |p: Vec2| -> f64 {
        for tr in &tris {
            let (a, b, c) = (pts2[tr[0]], pts2[tr[1]], pts2[tr[2]]);
            let d = cross(a, b, c);
            if d.abs() < 1e-18 {
                continue;
            }
            let (l1, l2) = (cross(p, b, c) / d, cross(a, p, c) / d);
            let l3 = 1.0 - l1 - l2;
            if l1 >= -1e-9 && l2 >= -1e-9 && l3 >= -1e-9 {
                return l1 * pts3[tr[0]][2] + l2 * pts3[tr[1]][2] + l3 * pts3[tr[2]][2];
            }
        }
        top0
    };
    let mut sides = 0.0;
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        sides += (b - a).length() * ((height_at(a) + height_at(b)) / 2.0 - bottom);
    }
    // interior edges of the triangulation: those shared by two triangles
    let mut shared = 0usize;
    for (i, a) in tris.iter().enumerate() {
        for (j, b) in tris.iter().enumerate().skip(i + 1) {
            let common = a.iter().filter(|x| b.contains(x)).count();
            if common == 2 {
                shared += 1;
            }
        }
    }
    let top_z = pts3.iter().map(|p| p[2]).fold(f64::NEG_INFINITY, f64::max);
    let h = hull(&pts2);
    // the vertices as the shape editor lists them
    let start = h.iter().enumerate().max_by(|(_, a), (_, b)| a.x.total_cmp(&b.x).then(b.y.total_cmp(&a.y))).map(|(i, _)| i).unwrap_or(0);
    let mut vertices: Vec<P3> = Vec::new();
    for k in 0..h.len() {
        let q = h[(start + k) % h.len()];
        if let Some(p) = pts3.iter().find(|p| (p[0] - q.x).abs() < 1e-9 && (p[1] - q.y).abs() < 1e-9) {
            vertices.push(*p);
        }
    }
    for p in &pts3 {
        if !vertices.iter().any(|v| v == p) {
            vertices.push(*p);
        }
    }
    Some(TopoState {
        bbox: ([lo.x, lo.y, bottom], [hi.x, hi.y, top_z.max(top0.min(top_z))]),
        volume,
        area: top_area + plan_area + sides,
        faces: facets + 1 + n,
        edges: shared + 3 * n,
        endpoints: pts3.len() + n,
        vertices,
    })
}

fn subdivision_state(m: &BimModel, t: &Toposolid, host: ElementId) -> Option<TopoState> {
    let poly: Vec<Vec2> = ccw(t.profile.iter().map(|p| v2(*p)).collect());
    let n = poly.len();
    let h = m.docs.toposolids.get(&host)?;
    let base = {
        let level_z = m.levels.get(&h.level?)?.elevation;
        let top0 = level_z + h.offset;
        if h.points.is_empty() {
            top0
        } else {
            h.points.iter().map(|p| p[2] + h.offset).fold(f64::NEG_INFINITY, f64::max)
        }
    };
    let region = Region::from_polygon(&poly, &Tolerances::default()).ok()?;
    let a = region.area();
    let perimeter: f64 = (0..n).map(|i| (poly[(i + 1) % n] - poly[i]).length()).sum();
    let (lo, hi) = poly.iter().fold((poly[0], poly[0]), |(l, hh), p| (Vec2::new(l.x.min(p.x), l.y.min(p.y)), Vec2::new(hh.x.max(p.x), hh.y.max(p.y))));
    Some(TopoState {
        bbox: ([lo.x, lo.y, base], [hi.x, hi.y, base + SUBDIVISION_THICKNESS_M]),
        volume: a * SUBDIVISION_THICKNESS_M,
        area: 2.0 * a + perimeter * SUBDIVISION_THICKNESS_M,
        faces: n + 2,
        edges: 3 * n,
        endpoints: 2 * n,
        vertices: Vec::new(),
    })
}

/// Map the plan positions of a toposolid (its heights stay).
pub fn map_plan(t: &mut Toposolid, f: impl Fn(Vec2) -> Vec2) {
    for p in &mut t.points {
        let q = f(Vec2::new(p[0], p[1]));
        p[0] = q.x;
        p[1] = q.y;
    }
    for p in &mut t.profile {
        let q = f(Vec2::new(p[0], p[1]));
        *p = [q.x, q.y];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops;

    fn doc() -> (crate::BimDocument, ElementId) {
        let mut d = crate::new_document();
        let l = d.transact("l", false, |m, cx| ops::create_level(m, cx, "L0", 0.0)).value.unwrap();
        (d, l)
    }

    #[test]
    fn the_flat_and_the_triangulated_toposolid_have_the_observed_solids() {
        let (mut d, l) = doc();
        let sq = |x0: f64, y0: f64, x1: f64, y1: f64| vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        let t1 = d.transact("t1", false, |m, cx| create_toposolid(m, cx, l, Vec::new(), sq(0.0, 0.0, 10.0, 10.0))).value.unwrap();
        let s = toposolid_state(d.model(), t1).unwrap();
        assert!((s.volume - 30.0).abs() < 1e-9 && (s.area - 212.0).abs() < 1e-9);
        assert_eq!((s.faces, s.edges, s.endpoints), (6, 12, 8));
        assert_eq!((s.bbox.0[2], s.bbox.1[2]), (-0.3, 0.0));
        let pts = vec![[20.0, 0.0, 0.0], [30.0, 0.0, 0.0], [30.0, 10.0, 1.0], [20.0, 10.0, 1.0], [25.0, 5.0, 2.0]];
        let t2 = d.transact("t2", false, |m, cx| create_toposolid(m, cx, l, pts, Vec::new())).value.unwrap();
        let s = toposolid_state(d.model(), t2).unwrap();
        assert!((s.volume - 130.0).abs() < 1e-6, "{s:?}");
        assert!((s.area - 236.861364).abs() < 1e-4, "{}", s.area);
        assert_eq!((s.faces, s.edges, s.endpoints), (9, 16, 9));
        assert_eq!(s.vertices[0], [30.0, 0.0, 0.0]);
        assert_eq!(s.vertices[4], [25.0, 5.0, 2.0]);
        // refused profiles and a subdivision outside its host
        assert!(d.transact("x", false, |m, cx| create_toposolid(m, cx, l, Vec::new(), vec![[0.0, 20.0], [10.0, 20.0], [0.0, 30.0], [10.0, 30.0]])).value.is_none());
        assert!(d.transact("sl", false, |m, cx| create_toposolid(m, cx, l, Vec::new(), vec![[0.0, 20.0], [10.0, 20.0], [10.0, 20.0001]])).value.is_none());
        let out = d.transact("sd", false, |m, cx| create_subdivision(m, cx, t1, sq(20.0, 20.0, 25.0, 25.0)));
        assert!(!out.committed());
        let sd = d.transact("sd2", false, |m, cx| create_subdivision(m, cx, t1, sq(2.0, 2.0, 5.0, 5.0))).value.unwrap();
        let s = toposolid_state(d.model(), sd).unwrap();
        assert!((s.volume - 2.7432).abs() < 1e-9 && s.bbox.1[2] == SUBDIVISION_THICKNESS_M);
    }

    /// Spec TOPO-03: a subdivision of a subdivision is refused with the reference product's text.
    #[test]
    fn a_subdivision_of_a_subdivision_is_refused() {
        let (mut d, l) = doc();
        let sq = |x0: f64, y0: f64, x1: f64, y1: f64| vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        let t1 = d.transact("t1", false, |m, cx| create_toposolid(m, cx, l, Vec::new(), sq(0.0, 0.0, 10.0, 10.0))).value.unwrap();
        let sd = d.transact("sd", false, |m, cx| create_subdivision(m, cx, t1, sq(2.0, 2.0, 5.0, 5.0))).value.unwrap();
        let out = d.transact("sd-sd", false, |m, cx| create_subdivision(m, cx, sd, sq(3.0, 3.0, 4.0, 4.0)));
        assert!(!out.committed());
        assert!(out.failures.iter().any(|f| f.message.contains("another sub-division")), "{:?}", out.failures);
    }
}
