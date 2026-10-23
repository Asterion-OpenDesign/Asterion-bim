//! Reference planes and model lines / arcs (OBSERVED and).
//!
//! * A reference plane is named "Reference Plane" until it is named. A copy of a plane whose name ends in a number takes
//!   the first unused number above it.
//! * A model line or arc is made on a sketch plane and must lie on it (`modelline.not_in_plane`, raised at the call).
//!   An arc is made from its two ends and a point on it; its length, centre, radius and box follow from the circle
//!   through the three points.

use crate::annot::{self, add, cross, dot, length, scale, sub, unit, CurveGeom, ModelCurve, RefPlane, V3};
use crate::model::BimModel;
use crate::spec::codes;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};

pub const DEFAULT_PLANE_NAME: &str = "Reference Plane";

fn bad(code: &str, msg: &str) -> Failure {
    Failure::error(code, msg.to_owned())
}

/// The next name of a copy: the trailing number incremented to the first unused one; a name with no number is kept.
pub fn next_name(existing: &[&str], name: &str) -> String {
    let digits = name.chars().rev().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return name.to_owned();
    }
    let (prefix, num) = name.split_at(name.len() - digits);
    let mut n: u64 = num.parse().unwrap_or(0) + 1;
    loop {
        let candidate = format!("{prefix}{n}");
        if !existing.contains(&candidate.as_str()) {
            return candidate;
        }
        n += 1;
    }
}

/// An EMPTY name is always accepted, and so is a name equal to the plane's own; otherwise the name must differ from the
/// text of every OTHER reference plane (`NameNotUnique`: "The name entered is already in use. Enter a unique name.",
/// OBSERVED as posted at commit). The default name "Reference Plane" of an unnamed plane may repeat (OBSERVED).
fn check_plane_name(m: &BimModel, cx: &mut TxnContext<'_>, id: Option<ElementId>, name: &str) {
    if !name.is_empty() && name != DEFAULT_PLANE_NAME && m.annot.ref_planes.values().any(|p| Some(p.id) != id && p.name == name) {
        cx.report(Failure::error(codes::REFPLANE_DUPLICATE_NAME, "The name entered is already in use. Enter a unique name."));
    }
}

pub fn create_ref_plane(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    bubble: V3,
    free: V3,
    cut: V3,
    view: Option<ElementId>,
    name: Option<&str>,
) -> Result<ElementId, Failure> {
    crate::ops::require_numbers("The reference plane", &[bubble[0], bubble[1], bubble[2], free[0], free[1], free[2], cut[0], cut[1], cut[2]])?;
    if length(sub(bubble, free)) < 1e-9 {
        return Err(bad(codes::REFPLANE_INVALID, "The two ends of a reference plane are the same point."));
    }
    let d = unit(sub(bubble, free));
    if length(cross(d, cut)) < 1e-9 {
        return Err(bad(codes::REFPLANE_INVALID, "The cut vector is parallel to the plane."));
    }
    // a plane belongs to a view that exists (QA: a view id that names nothing was stored and left a dangling reference)
    if let Some(v) = view {
        if !m.views.contains_key(&v) && !m.docs.views.contains_key(&v) {
            return Err(Failure::error(codes::NOT_FOUND, "No such view.").on(&[v]));
        }
    }
    let name = name.unwrap_or(DEFAULT_PLANE_NAME).to_owned();
    let id = cx.new_id();
    check_plane_name(m, cx, None, &name);
    let normal = unit(cross(d, cut));
    m.annot.ref_planes.insert(id, RefPlane { id, name, bubble, free, cut, normal, view });
    Ok(id)
}

/// Set the name and / or the ends of a reference plane (the normal follows the new direction).
pub fn set_ref_plane(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    id: ElementId,
    name: Option<&str>,
    bubble: Option<V3>,
    free: Option<V3>,
) -> Result<(), Failure> {
    if !m.annot.ref_planes.contains_key(&id) {
        return Err(Failure::error(codes::NOT_FOUND, "No such reference plane.").on(&[id]));
    }
    if let Some(n) = name {
        check_plane_name(m, cx, Some(id), n);
    }
    // The new direction runs from the free end to the bubble end; a new end on the other end's location is refused, and so
    // is one that would tilt the plane (the direction must stay perpendicular to the CURRENT normal, 1e-9 ft); the normal
    // never changes when an end moves.
    let tol = 1e-9 / crate::spec::FEET_PER_METRE;
    let normal = m.annot.ref_planes[&id].normal;
    let (cur_bubble, cur_free) = (m.annot.ref_planes[&id].bubble, m.annot.ref_planes[&id].free);
    if let Some(b) = bubble {
        let d = sub(b, free.unwrap_or(cur_free));
        if length(d) <= tol {
            return Err(bad(codes::REFPLANE_INVALID, "The new bubble end is the same location as the free end (within tolerance)."));
        }
        if dot(d, normal).abs() > tol {
            return Err(bad(codes::REFPLANE_INVALID, "The vector from BubbleEnd -> FreeEnd is not perpendicular to the normal vector."));
        }
    }
    if let Some(f) = free {
        let d = sub(bubble.unwrap_or(cur_bubble), f);
        if length(d) <= tol {
            return Err(bad(codes::REFPLANE_INVALID, "The new free end is the same location as the bubble end (within tolerance)."));
        }
        if dot(d, normal).abs() > tol {
            return Err(bad(codes::REFPLANE_INVALID, "The vector from BubbleEnd -> FreeEnd is not perpendicular to the normal vector."));
        }
    }
    let p = m.annot.ref_planes.get_mut(&id).expect("checked");
    if let Some(n) = name {
        p.name = n.to_owned();
    }
    if let Some(b) = bubble {
        p.bubble = b;
    }
    if let Some(f) = free {
        p.free = f;
    }
    Ok(())
}

/// Flip a reference plane. The plane is rebuilt with the SAME direction and the SAME origin (free end) and the NEGATED
/// normal; the bubble and free ends stay where they are. (The envelope is mirrored across the direction axis: the engine
/// keeps no envelope.)
pub fn flip_ref_plane(m: &mut BimModel, id: ElementId) -> Result<(), Failure> {
    let p = m.annot.ref_planes.get_mut(&id).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such reference plane.").on(&[id]))?;
    p.normal = scale(p.normal, -1.0);
    p.cut = scale(p.cut, -1.0);
    Ok(())
}

/// `SketchPlane.Create(document, plane)`: every creator makes a NEW work plane element - none looks for an existing one. A
/// plane with no normal is refused.
pub fn create_sketch_plane(m: &mut BimModel, cx: &mut TxnContext<'_>, origin: V3, normal: V3) -> Result<ElementId, Failure> {
    if length(normal) < 1e-9 || !(origin.iter().chain(normal.iter()).all(|c| c.is_finite())) {
        return Err(bad(codes::MODEL_LINE_INVALID, "A work plane needs a finite origin and a normal."));
    }
    let id = cx.new_id();
    m.annot.sketch_planes.insert(id, annot::SketchPlane { id, origin, normal: unit(normal), datum: None });
    Ok(id)
}

/// The new work plane follows the datum.
pub fn create_sketch_plane_on_datum(m: &mut BimModel, cx: &mut TxnContext<'_>, datum: ElementId) -> Result<ElementId, Failure> {
    let Some((origin, normal)) = datum_plane(m, datum) else {
        return Err(bad(codes::MODEL_LINE_INVALID, "ElementId does not correspond to a datum plane.").on(&[datum]));
    };
    let id = cx.new_id();
    m.annot.sketch_planes.insert(id, annot::SketchPlane { id, origin, normal, datum: Some(datum) });
    Ok(id)
}

/// The plane of a datum in its present state: a level is horizontal at its elevation; a grid is the vertical plane through
/// its line; a reference plane is its own plane (origin the free end).
fn datum_plane(m: &BimModel, id: ElementId) -> Option<(V3, V3)> {
    if let Some(l) = m.levels.get(&id) {
        return Some(([0.0, 0.0, l.elevation], [0.0, 0.0, 1.0]));
    }
    if let Some(g) = m.grids.get(&id) {
        let d = (g.end - g.start).normalized()?;
        return Some(([g.start.x, g.start.y, 0.0], [-d.y, d.x, 0.0]));
    }
    m.annot.ref_planes.get(&id).map(|p| (p.free, p.plane_normal()))
}

/// `SketchPlane.GetPlane`: the work plane's origin and unit normal; one made from a datum reads the datum's present plane.
pub fn sketch_plane_geometry(m: &BimModel, id: ElementId) -> Option<(V3, V3)> {
    let p = m.annot.sketch_planes.get(&id)?;
    match p.datum {
        Some(d) => datum_plane(m, d),
        None => Some((p.origin, p.normal)),
    }
}

/// A model line on a work plane element ("the plane a new model line gets is the one the caller passes").
pub fn create_model_line_on(m: &mut BimModel, cx: &mut TxnContext<'_>, start: V3, end: V3, plane: ElementId) -> Result<ElementId, Failure> {
    let (origin, normal) = sketch_plane_geometry(m, plane).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such work plane.").on(&[plane]))?;
    create_model_line(m, cx, start, end, origin, normal)
}

/// `NewModelCurve` of a line on the sketch plane `(origin, normal)`.
///
/// The sketch plane of a new model line is the one the CALLER passes (no operation of the element chooses it), and a model
/// line made without a style shows the style of its default category (`<Lines>`), so the engine stores no style of its own.
/// UNKNOWN: the creation operation the API uses for a model line. Spec ML-03: the ends and the length of the element are
/// those of its geometry curve in model coordinates, and a curve with none of the detail / view-specific / reference-line /
/// by-points flags is the model-curve kind (1); the engine's model lines are straight, so their curve kind is a line.
pub fn create_model_line(m: &mut BimModel, cx: &mut TxnContext<'_>, start: V3, end: V3, origin: V3, normal: V3) -> Result<ElementId, Failure> {
    numbers(&[start, end, origin, normal])?;
    if length(sub(start, end)) < 1e-9 {
        return Err(bad(codes::MODEL_LINE_INVALID, "A model line has no length."));
    }
    check_in_plane(&[start, end], origin, normal)?;
    let id = cx.new_id();
    m.annot.curves.insert(id, ModelCurve { id, geom: CurveGeom::Line { start, end }, plane_origin: origin, plane_normal: unit(normal), reference_line: false });
    Ok(id)
}

/// `ModelCurve.SetGeometryCurve`: another curve on the same sketch plane (a line for an arc and the reverse are
/// allowed; a curve off the plane is refused).
pub fn set_model_curve(m: &mut BimModel, cx: &mut TxnContext<'_>, id: ElementId, geom: CurveGeom) -> Result<(), Failure> {
    let _ = cx;
    match geom {
        CurveGeom::Line { start, end } => numbers(&[start, end])?,
        CurveGeom::Arc { start, end, mid } => numbers(&[start, end, mid])?,
    }
    let (origin, normal) = {
        let c = m.annot.curves.get(&id).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such model curve.").on(&[id]))?;
        (c.plane_origin, c.plane_normal)
    };
    // OBSERVED: the new curve must be of the same type as the old one (a line for a line, an arc for an arc)
    let same = matches!((m.annot.curves[&id].geom, geom), (CurveGeom::Line { .. }, CurveGeom::Line { .. }) | (CurveGeom::Arc { .. }, CurveGeom::Arc { .. }));
    if !same {
        return Err(bad(codes::MODEL_LINE_INVALID, "The input curve must of the same type as the current curve."));
    }
    match geom {
        CurveGeom::Line { start, end } => {
            if length(sub(start, end)) < 1e-9 {
                return Err(bad(codes::MODEL_LINE_INVALID, "A model line has no length."));
            }
            check_in_plane(&[start, end], origin, normal)?;
        }
        CurveGeom::Arc { start, end, mid } => {
            if circle(start, end, mid).is_none() {
                return Err(bad(codes::MODEL_LINE_INVALID, "The three points of an arc are on a line."));
            }
            check_in_plane(&[start, end, mid], origin, normal)?;
        }
    }
    m.annot.curves.get_mut(&id).expect("checked").geom = geom;
    Ok(())
}

/// `ModelCurve.ChangeToReferenceLine`. OBSERVED: "only permitted in family document", so a project refuses it.
pub fn change_to_reference_line(m: &mut BimModel, id: ElementId) -> Result<(), Failure> {
    m.annot.curves.get(&id).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such model curve.").on(&[id]))?;
    Err(bad(codes::MODEL_LINE_INVALID, "The attempted operation is only permitted in family document."))
}

/// `NewModelCurve` of an arc through three points on the sketch plane.
pub fn create_model_arc(m: &mut BimModel, cx: &mut TxnContext<'_>, start: V3, end: V3, mid: V3, origin: V3, normal: V3) -> Result<ElementId, Failure> {
    numbers(&[start, end, mid, origin, normal])?;
    if circle(start, end, mid).is_none() {
        return Err(bad(codes::MODEL_LINE_INVALID, "The three points of an arc are on a line."));
    }
    check_in_plane(&[start, end, mid], origin, normal)?;
    let id = cx.new_id();
    m.annot.curves.insert(id, ModelCurve { id, geom: CurveGeom::Arc { start, end, mid }, plane_origin: origin, plane_normal: unit(normal), reference_line: false });
    Ok(id)
}

/// Every coordinate finite and within the engine's numeric limit (`input.invalid_number`).
fn numbers(points: &[V3]) -> Result<(), Failure> {
    let flat: Vec<f64> = points.iter().flat_map(|p| p.iter().copied()).collect();
    crate::ops::require_numbers("The curve", &flat)
}

fn check_in_plane(points: &[V3], origin: V3, normal: V3) -> Result<(), Failure> {
    let n = unit(normal);
    for p in points {
        if dot(sub(*p, origin), n).abs() > 1e-6 {
            return Err(bad(codes::MODEL_LINE_NOT_IN_PLANE, "The curve must be in the plane of its sketch plane."));
        }
    }
    Ok(())
}

/// The circle through three points: `(centre, radius, unit normal)`.
pub fn circle(a: V3, b: V3, c: V3) -> Option<(V3, f64, V3)> {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let n = cross(ab, ac);
    let nn = dot(n, n);
    if nn < 1e-18 {
        return None;
    }
    // centre = a + ((|ac|^2 (ab x ac) x ab ... ) the standard circumcentre formula
    let t = add(scale(cross(cross(ab, ac), ab), dot(ac, ac)), scale(cross(ac, cross(ab, ac)), dot(ab, ab)));
    let centre = add(a, scale(t, 1.0 / (2.0 * nn)));
    let r = length(sub(a, centre));
    Some((centre, r, unit(n)))
}

impl CurveGeom {
    pub fn ends(&self) -> (V3, V3) {
        match *self {
            CurveGeom::Line { start, end } => (start, end),
            CurveGeom::Arc { start, end, .. } => (start, end),
        }
    }

    /// The length of the curve.
    pub fn length(&self) -> f64 {
        match *self {
            CurveGeom::Line { start, end } => length(sub(end, start)),
            CurveGeom::Arc { start, end, mid } => match arc_frame(start, end, mid) {
                Some((_, r, _, _, sweep)) => r * sweep.abs(),
                None => 0.0,
            },
        }
    }

    /// The axis-aligned box of the curve: `(min, max)`.
    pub fn bbox(&self) -> (V3, V3) {
        match *self {
            CurveGeom::Line { start, end } => (
                [start[0].min(end[0]), start[1].min(end[1]), start[2].min(end[2])],
                [start[0].max(end[0]), start[1].max(end[1]), start[2].max(end[2])],
            ),
            CurveGeom::Arc { start, end, mid } => {
                let mut lo = [start[0].min(end[0]), start[1].min(end[1]), start[2].min(end[2])];
                let mut hi = [start[0].max(end[0]), start[1].max(end[1]), start[2].max(end[2])];
                if let Some((c, r, u, v, sweep)) = arc_frame(start, end, mid) {
                    for axis in 0..3 {
                        // the angles where this coordinate is extreme: tan t = v_a / u_a, and the opposite one
                        let t0 = v[axis].atan2(u[axis]);
                        for t in [t0, t0 + std::f64::consts::PI] {
                            let t = if sweep >= 0.0 { t.rem_euclid(std::f64::consts::TAU) } else { (-t).rem_euclid(std::f64::consts::TAU) };
                            if t <= sweep.abs() + 1e-12 {
                                let ang = if sweep >= 0.0 { t } else { -t };
                                let p = c[axis] + r * (ang.cos() * u[axis] + ang.sin() * v[axis]);
                                lo[axis] = lo[axis].min(p);
                                hi[axis] = hi[axis].max(p);
                            }
                        }
                    }
                }
                (lo, hi)
            }
        }
    }

    pub fn map_points(&self, f: &dyn Fn(V3) -> V3) -> CurveGeom {
        match *self {
            CurveGeom::Line { start, end } => CurveGeom::Line { start: f(start), end: f(end) },
            CurveGeom::Arc { start, end, mid } => CurveGeom::Arc { start: f(start), end: f(end), mid: f(mid) },
        }
    }
}

/// `(centre, radius, u, v, signed sweep)` of an arc: `u` points at the start, `v = normal x u`, the sweep runs
/// from the start to the end through the middle point.
pub fn arc_frame(start: V3, end: V3, mid: V3) -> Option<(V3, f64, V3, V3, f64)> {
    let (c, r, n) = circle(start, end, mid)?;
    let u = unit(sub(start, c));
    let v = unit(cross(n, u));
    let ang = |p: V3| dot(sub(p, c), v).atan2(dot(sub(p, c), u)).rem_euclid(std::f64::consts::TAU);
    let (tm, te) = (ang(mid), ang(end));
    let sweep = if tm < te { te } else { te - std::f64::consts::TAU };
    Some((c, r, u, v, sweep))
}

/// Silence an unused import when the helpers above are the only users of `annot`.
#[allow(dead_code)]
fn _touch() -> V3 {
    annot::sub([0.0; 3], [0.0; 3])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BimDocument;

    fn doc() -> BimDocument {
        crate::new_document()
    }

    #[test]
    fn a_reference_plane_has_the_observed_direction_normal_and_origin() {
        // OBSERVED: bubble (0,0,0), free (5,0,0), cut (0,0,1): direction (-1,0,0), normal (0,1,0), origin = free end
        let mut d = doc();
        let id = d.transact("p", false, |m, cx| create_ref_plane(m, cx, [0.0; 3], [5.0, 0.0, 0.0], [0.0, 0.0, 1.0], None, Some("AST-RP-1"))).value.unwrap();
        let p = &d.model().annot.ref_planes[&id];
        assert_eq!(p.direction(), [-1.0, 0.0, 0.0]);
        assert_eq!(p.plane_normal(), [0.0, 1.0, 0.0]);
        assert_eq!(p.normal, [0.0, 1.0, 0.0]);
        // a vertical plane: bubble (10,0,0), free (10,0,5), cut (0,1,0): normal (1,0,0)
        let v = d.transact("v", false, |m, cx| create_ref_plane(m, cx, [10.0, 0.0, 0.0], [10.0, 0.0, 5.0], [0.0, 1.0, 0.0], None, None)).value.unwrap();
        assert_eq!(d.model().annot.ref_planes[&v].plane_normal(), [1.0, 0.0, 0.0]);
        assert_eq!(d.model().annot.ref_planes[&v].name, DEFAULT_PLANE_NAME);
    }

    #[test]
    fn a_duplicate_name_does_not_commit_but_the_default_name_may_repeat() {
        let mut d = doc();
        d.transact("a", false, |m, cx| create_ref_plane(m, cx, [0.0; 3], [5.0, 0.0, 0.0], [0.0, 0.0, 1.0], None, Some("A")));
        let dup = d.transact("b", false, |m, cx| create_ref_plane(m, cx, [0.0, 1.0, 0.0], [5.0, 1.0, 0.0], [0.0, 0.0, 1.0], None, Some("A")));
        assert!(!dup.committed());
        assert_eq!(d.model().annot.ref_planes.len(), 1);
        assert!(d.transact("c", false, |m, cx| create_ref_plane(m, cx, [0.0, 2.0, 0.0], [5.0, 2.0, 0.0], [0.0, 0.0, 1.0], None, None)).committed());
        assert!(d.transact("d", false, |m, cx| create_ref_plane(m, cx, [0.0, 3.0, 0.0], [5.0, 3.0, 0.0], [0.0, 0.0, 1.0], None, None)).committed());
    }

    #[test]
    fn a_flip_negates_the_normal_and_keeps_the_ends_and_an_empty_name_may_repeat() {
        // Same direction, same origin, negated normal; an empty name needs no check
        let mut d = doc();
        let p = d.transact("p", false, |m, cx| create_ref_plane(m, cx, [0.0; 3], [5.0, 0.0, 0.0], [0.0, 0.0, 1.0], None, Some("A"))).value.unwrap();
        let (dir, ends) = (d.model().annot.ref_planes[&p].direction(), (d.model().annot.ref_planes[&p].bubble, d.model().annot.ref_planes[&p].free));
        assert!(d.transact("f", false, |m, _| flip_ref_plane(m, p)).committed());
        let q = &d.model().annot.ref_planes[&p];
        assert_eq!((q.direction(), q.bubble, q.free), (dir, ends.0, ends.1));
        assert_eq!(q.plane_normal(), [0.0, -1.0, 0.0]);
        assert_eq!(q.normal, [0.0, -1.0, 0.0]);
        assert!(d.transact("f2", false, |m, _| flip_ref_plane(m, ElementId(999))).value.is_none());
        let a = d.transact("a", false, |m, cx| create_ref_plane(m, cx, [0.0, 1.0, 0.0], [5.0, 1.0, 0.0], [0.0, 0.0, 1.0], None, Some(""))).value;
        let b = d.transact("b", false, |m, cx| create_ref_plane(m, cx, [0.0, 2.0, 0.0], [5.0, 2.0, 0.0], [0.0, 0.0, 1.0], None, Some(""))).value;
        assert!(a.is_some() && b.is_some());
    }

    #[test]
    fn a_work_plane_is_a_new_element_each_time_and_follows_its_datum() {
        // From a Plane a NEW element each time; from an id the element must be a level, a grid or a reference plane
        let mut d = doc();
        let (lv, rp) = d
            .transact("fx", false, |m, cx| {
                let l = crate::ops::create_level(m, cx, "L0", 3.0)?;
                let p = create_ref_plane(m, cx, [0.0; 3], [5.0, 0.0, 0.0], [0.0, 0.0, 1.0], None, Some("R"))?;
                Ok((l, p))
            })
            .value
            .unwrap();
        let a = d.transact("a", false, |m, cx| create_sketch_plane(m, cx, [0.0, 0.0, 1.0], [0.0, 0.0, 2.0])).value.unwrap();
        let b = d.transact("b", false, |m, cx| create_sketch_plane(m, cx, [0.0, 0.0, 1.0], [0.0, 0.0, 2.0])).value.unwrap();
        assert_ne!(a, b);
        assert_eq!(sketch_plane_geometry(d.model(), a), Some(([0.0, 0.0, 1.0], [0.0, 0.0, 1.0])));
        assert!(d.transact("bad", false, |m, cx| create_sketch_plane(m, cx, [0.0; 3], [0.0; 3])).value.is_none());
        let on_level = d.transact("l", false, |m, cx| create_sketch_plane_on_datum(m, cx, lv)).value.unwrap();
        let on_plane = d.transact("p", false, |m, cx| create_sketch_plane_on_datum(m, cx, rp)).value.unwrap();
        assert_eq!(sketch_plane_geometry(d.model(), on_level), Some(([0.0, 0.0, 3.0], [0.0, 0.0, 1.0])));
        assert_eq!(sketch_plane_geometry(d.model(), on_plane).map(|p| p.1), Some([0.0, 1.0, 0.0]));
        // the plane on a level moves with the level; a model line made on it lies at the level's elevation
        d.transact("up", false, |m, cx| crate::ops::set_level_elevation(m, cx, lv, 4.0)).value.unwrap();
        assert_eq!(sketch_plane_geometry(d.model(), on_level), Some(([0.0, 0.0, 4.0], [0.0, 0.0, 1.0])));
        assert!(d.transact("ml", false, |m, cx| create_model_line_on(m, cx, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0], on_level)).committed());
        // a wall (or anything that is not a datum plane) is refused
        assert!(d.transact("nd", false, |m, cx| create_sketch_plane_on_datum(m, cx, a)).value.is_none());
    }

    #[test]
    fn a_copy_takes_the_first_unused_number() {
        assert_eq!(next_name(&["AST-RP-1", "AST-RP-2"], "AST-RP-1"), "AST-RP-3");
        assert_eq!(next_name(&["A"], "A"), "A");
    }

    #[test]
    fn a_line_must_lie_on_its_plane_and_an_arc_has_its_circle() {
        let mut d = doc();
        assert!(d.transact("l", false, |m, cx| create_model_line(m, cx, [0.0; 3], [4.0, 0.0, 0.0], [0.0; 3], [0.0, 0.0, 1.0])).committed());
        assert!(d.transact("bad", false, |m, cx| create_model_line(m, cx, [0.0, 0.0, 5.0], [4.0, 0.0, 0.0], [0.0; 3], [0.0, 0.0, 1.0])).value.is_none());
        let a = d.transact("a", false, |m, cx| create_model_arc(m, cx, [10.0, 0.0, 0.0], [14.0, 0.0, 0.0], [12.0, 2.0, 0.0], [0.0; 3], [0.0, 0.0, 1.0])).value.unwrap();
        let g = d.model().annot.curves[&a].geom;
        assert!((g.length() - std::f64::consts::TAU).abs() < 1e-9, "{}", g.length());
        let (lo, hi) = g.bbox();
        assert!((lo[0] - 10.0).abs() < 1e-9 && (hi[0] - 14.0).abs() < 1e-9 && lo[1].abs() < 1e-9 && (hi[1] - 2.0).abs() < 1e-9, "{lo:?} {hi:?}");
    }
}
