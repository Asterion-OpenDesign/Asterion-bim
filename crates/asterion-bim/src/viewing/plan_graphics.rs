//! The drawing model of a plan view: what is drawn, as a list of primitives with the pen each one
//! takes, between the engine's derived view state ([`crate::views::ViewState`]) and a renderer.
//!
//! # Status of every number in this module
//!
//! * WHICH elements are drawn and as what (cut / projection / beyond) is the view classifier's decision (`crate::views`,
//!   Spec V-02, V-03). This module only draws what that decided.
//! * The table is therefore one place ([`pen_for`]) taking `(role, scale, detail)` so the rule, when it exists, changes one
//!   function and no caller.
//! * Pens are DEVICE PIXELS, never model widths: the renderer applies them after the camera
//!   transform, which is what makes a line keep its width at any zoom (OBSERVED, live capture).
//!
//! # What is deliberately absent (no engine datum, listed as gaps, never faked)
//!
//! Surface and cut patterns of wall types, wall layer lines at Medium / Fine, door and window symbols
//! from the family, text (grid names, room tags, dimensions), elevation / section markers, stair
//! treads, level heads (levels draw in elevations and sections, which the engine has no view for),
//! the north arrow and scale bar (view-level annotation families).
//!
//! # Derived, never stored
//!
//! [`plan_primitives`] is a pure function of the model and the view; nothing is cached.

use crate::model::{BimModel, DetailLevel, View};
use crate::views::{Appearance, ViewState};
use asterion_core::id::ElementId;
use asterion_geom::boolean::union_all;
use asterion_geom::curve::Curve;
use asterion_geom::region::Region;
use asterion_geom::tolerance::Tolerances;
use asterion_geom::Vec2;

/// A line pattern, resolved to pixels by the renderer (the period does not depend on zoom).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    Solid,
    /// The plan look of a grid line: long dash, gap, short dash, gap, in paper millimetres (the renderer's pattern 1,
    /// OBSERVED in the paired grid run; the grid type's line pattern is not specified).
    DashDot,
}

/// What a primitive is, for the pen table and for a future per-category visibility.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Outline of an element the view's cut plane passes through.
    CutEdge,
    /// Outline of an element seen below the cut plane, inside the primary range.
    ProjectionEdge,
    /// Outline of an element in the view depth zone.
    BeyondEdge,
    /// A grid line.
    GridLine,
    /// The circle of a grid bubble.
    GridBubble,
}

/// A pen: width in device pixels, tone and pattern.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pen {
    pub px: f32,
    /// `true`: the secondary tone (`#AAAAAA` in the capture); `false`: black.
    pub secondary: bool,
    pub pattern: Pattern,
}

impl Pen {
    /// The weight class the renderer's line buffer carries (`asterion_view_linework`): the base
    /// class 2.0 = 2 px black, 1.0 = 1 px black, 0.5 = 1 px secondary tone, plus 10 per pattern id.
    pub fn class(self) -> f32 {
        let base = if self.secondary {
            0.5
        } else if self.px >= 1.5 {
            2.0
        } else {
            1.0
        };
        base + match self.pattern {
            Pattern::Solid => 0.0,
            Pattern::DashDot => 10.0,
        }
    }
}

/// The pen of a role at a view scale and detail level. ASSUMED table (module documentation); the
/// parameters are in the signature so the measured table replaces this body only.
pub fn pen_for(role: Role, _scale: u32, _detail: DetailLevel) -> Pen {
    match role {
        Role::CutEdge => Pen { px: 2.0, secondary: false, pattern: Pattern::Solid },
        Role::ProjectionEdge => Pen { px: 1.0, secondary: false, pattern: Pattern::Solid },
        Role::BeyondEdge => Pen { px: 1.0, secondary: true, pattern: Pattern::Solid },
        // OBSERVED (grr-g3-grids, paired grid run 2026-10-15): a placed grid draws black, 1 px, in its pattern
        Role::GridLine => Pen { px: 1.0, secondary: false, pattern: Pattern::DashDot },
        Role::GridBubble => Pen { px: 1.0, secondary: false, pattern: Pattern::Solid },
    }
}

/// Paper diameter of a grid bubble, metres. OBSERVED in the paired grid run of 2026-10-15 (at 1:100, grr-g3-grids-full):
/// the bubble is 0.2063 of the room's outer width there, 13.1 mm on paper; the grid type's head family that sets it is not
/// specified. Model size = paper size x view scale.
pub const GRID_BUBBLE_PAPER_DIAMETER_M: f64 = 0.0131;

/// The geometry of a primitive, in model metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Segment(Vec2, Vec2),
    Circle { centre: Vec2, radius: f64 },
}

/// One drawn thing: the element it belongs to, its role, its pen and its shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Primitive {
    pub element: ElementId,
    pub role: Role,
    pub pen: Pen,
    pub shape: Shape,
}

/// Number of segments a circle is flattened to (ASSUMED: fixed, a bubble is small on screen).
pub const CIRCLE_SEGMENTS: usize = 32;

impl Primitive {
    /// Appends the primitive to a `[x0, y0, x1, y1, weight_class]` buffer.
    pub fn flatten(&self, out: &mut Vec<f32>) {
        let w = self.pen.class();
        match self.shape {
            Shape::Segment(a, b) => out.extend_from_slice(&[a.x as f32, a.y as f32, b.x as f32, b.y as f32, w]),
            Shape::Circle { centre, radius } => {
                let at = |k: usize| {
                    let t = k as f64 / CIRCLE_SEGMENTS as f64 * std::f64::consts::TAU;
                    Vec2::new(centre.x + radius * t.cos(), centre.y + radius * t.sin())
                };
                for k in 0..CIRCLE_SEGMENTS {
                    let (a, b) = (at(k), at(k + 1));
                    out.extend_from_slice(&[a.x as f32, a.y as f32, b.x as f32, b.y as f32, w]);
                }
            }
        }
    }
}

/// `[x0, y0, x1, y1, weight_class]` for every primitive, in order.
pub fn flatten(prims: &[Primitive]) -> Vec<f32> {
    let mut out = Vec::with_capacity(prims.len() * 5 * 2);
    for p in prims {
        p.flatten(&mut out);
    }
    out
}

/// The primitives of a plan view: element outlines by appearance (walls with their joined outlines
/// merged: no line is drawn across a join), floors, and grids with the bubbles the view shows.
///
/// OBSERVED (live capture of the reference, Coarse detail): the outline of joined walls has no line at the join. ASSUMED as
/// a rule: the merge is the union of the wall footprints of one appearance class.
pub fn plan_primitives(model: &BimModel, view: &View, st: &ViewState) -> Vec<Primitive> {
    let detail = view.detail_level;
    let scale = st.scale.max(1);
    let mut out: Vec<Primitive> = Vec::new();

    let role_of = |a: Appearance| match a {
        Appearance::Cut => Some(Role::CutEdge),
        Appearance::Projection => Some(Role::ProjectionEdge),
        Appearance::Beyond => Some(Role::BeyondEdge),
        Appearance::Hidden => None,
    };

    // walls, per appearance class, merged
    for appearance in [Appearance::Beyond, Appearance::Projection, Appearance::Cut] {
        let Some(role) = role_of(appearance) else { continue };
        let pen = pen_for(role, scale, detail);
        let walls: Vec<(ElementId, &Vec<Vec2>)> = st
            .elements
            .iter()
            .filter(|e| e.appearance == appearance && model.walls.contains_key(&e.id) && e.footprint.len() >= 3)
            .map(|e| (e.id, &e.footprint))
            .collect();
        if walls.is_empty() {
            continue;
        }
        match merged_outlines(&walls.iter().map(|(_, f)| (*f).clone()).collect::<Vec<_>>()) {
            Some(loops) => {
                // the merged outline belongs to no single wall: attributed to the first one of the group
                let element = walls[0].0;
                for l in loops {
                    for i in 0..l.len() {
                        out.push(Primitive { element, role, pen, shape: Shape::Segment(l[i], l[(i + 1) % l.len()]) });
                    }
                }
            }
            None => {
                for (id, f) in &walls {
                    for i in 0..f.len() {
                        out.push(Primitive { element: *id, role, pen, shape: Shape::Segment(f[i], f[(i + 1) % f.len()]) });
                    }
                }
            }
        }
    }
    // everything else the view classified (floors, ...): own outline
    for e in st.elements.iter().filter(|e| !model.walls.contains_key(&e.id)) {
        let Some(role) = role_of(e.appearance) else { continue };
        let pen = pen_for(role, scale, detail);
        let f = &e.footprint;
        for i in 0..f.len() {
            out.push(Primitive { element: e.id, role, pen, shape: Shape::Segment(f[i], f[(i + 1) % f.len()]) });
        }
    }

    // Grids: the line, and a bubble at each end the view shows one at (OBSERVED: per-view bubble flags).
    let line = pen_for(Role::GridLine, scale, detail);
    let bubble = pen_for(Role::GridBubble, scale, detail);
    let radius = GRID_BUBBLE_PAPER_DIAMETER_M * 0.5 * f64::from(scale);
    for g in st.grids.iter().filter(|g| g.visible && !view.hidden_elements.contains(&g.id)) {
        let poly: Vec<Vec2> = if g.polyline.len() >= 2 { g.polyline.clone() } else { vec![g.start, g.end] };
        for w in poly.windows(2) {
            out.push(Primitive { element: g.id, role: Role::GridLine, pen: line, shape: Shape::Segment(w[0], w[1]) });
        }
        // the bubble sits beyond each end along the curve's tangent there (an arc grid's end
        // tangents, a straight grid's one direction)
        let n = poly.len();
        let (Some(d_start), Some(d_end)) = ((poly[1] - poly[0]).normalized(), (poly[n - 1] - poly[n - 2]).normalized()) else { continue };
        // [end 1, end 2] of the native datum = the model curve's start and end (engine End0 / End1): the view curve of a
        // straight grid runs reversed (its end is the model start), an arc grid's runs forward
        let at_start = poly[0] - d_start * radius;
        let at_end = poly[n - 1] + d_end * radius;
        let curved = model.grids.get(&g.id).map_or(false, |m| m.is_curved());
        let (end0, end1) = if curved { (at_start, at_end) } else { (at_end, at_start) };
        if g.bubble[1] {
            out.push(Primitive { element: g.id, role: Role::GridBubble, pen: bubble, shape: Shape::Circle { centre: end1, radius } });
        }
        if g.bubble[0] {
            out.push(Primitive { element: g.id, role: Role::GridBubble, pen: bubble, shape: Shape::Circle { centre: end0, radius } });
        }
    }
    out
}

/// The outline loops of the union of `polys` (outer loops and holes, as polylines), or `None` when
/// the kernel cannot build it (the caller then draws every footprint on its own).
/// The union of `regions`, exactly as `union_all` computes it, run per CONNECTED GROUP: regions whose
/// boxes (grown by twice the vertex tolerance) overlap, directly or through others, form a group, and a
/// region of one group can never touch a region of another, so `union_all` would only keep them apart.
/// Each group keeps its regions in their input order, and the groups' results are ordered the way
/// `union_all` orders its output, so the result is the same; what it saves is the booleans between a
/// grown region and the regions of other groups that its box reaches (the plans of a large sample project).
fn union_by_groups(regions: &[Region], tol: &Tolerances) -> asterion_geom::GeomResult<Vec<Region>> {
    let n = regions.len();
    let pad = 2.0 * tol.vertex + 1e-9;
    let boxes: Vec<_> = regions.iter().map(|r| r.aabb()).collect();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for i in 0..n {
        for j in (i + 1)..n {
            let (a, b) = (&boxes[i], &boxes[j]);
            let apart = a.max.x + pad < b.min.x || b.max.x + pad < a.min.x || a.max.y + pad < b.min.y || b.max.y + pad < a.min.y;
            if !apart {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[ri.max(rj)] = ri.min(rj);
                }
            }
        }
    }
    let mut groups: std::collections::BTreeMap<usize, Vec<Region>> = std::collections::BTreeMap::new();
    for (i, r) in regions.iter().enumerate() {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(r.clone());
    }
    let mut out = Vec::new();
    for (_, g) in groups {
        out.extend(union_all(&g, tol)?);
    }
    out.sort_by(|x, y| {
        y.area()
            .total_cmp(&x.area())
            .then(x.aabb().min.x.total_cmp(&y.aabb().min.x))
            .then(x.aabb().min.y.total_cmp(&y.aabb().min.y))
    });
    Ok(out)
}

fn merged_outlines(polys: &[Vec<Vec2>]) -> Option<Vec<Vec<Vec2>>> {
    let tol = Tolerances::default();
    let mut regions = Vec::with_capacity(polys.len());
    for p in polys {
        regions.push(Region::from_polygon(p, &tol).ok()?);
    }
    let merged = union_by_groups(&regions, &tol).ok()?;
    let mut loops: Vec<Vec<Vec2>> = Vec::new();
    for r in &merged {
        for l in std::iter::once(r.outer()).chain(r.holes().iter()) {
            let mut pts: Vec<Vec2> = Vec::new();
            for c in l.curves() {
                match c {
                    Curve::Line(s) => pts.push(s.start),
                    Curve::Arc(a) => {
                        // an arc wall's outline keeps its curvature as a polyline (2 mm sagitta)
                        let t = c.tessellate(0.002);
                        let _ = a;
                        pts.extend(t.iter().take(t.len().saturating_sub(1)).copied());
                    }
                }
            }
            if pts.len() >= 3 {
                loops.push(pts);
            }
        }
    }
    Some(loops)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ops, CompoundStructure, Layer, LayerFunction};
    use crate::model::ViewKind;

    #[test]
    fn union_by_groups_equals_union_all() {
        // touching, overlapping, separate and nested-box squares in a scrambled order
        let tol = Tolerances::default();
        let sq = |x: f64, y: f64, s: f64| vec![Vec2::new(x, y), Vec2::new(x + s, y), Vec2::new(x + s, y + s), Vec2::new(x, y + s)];
        let polys = [sq(10.0, 0.0, 1.0), sq(0.0, 0.0, 1.0), sq(1.0, 0.0, 1.0), sq(20.0, 20.0, 2.0), sq(0.5, 0.5, 1.0), sq(11.0, 0.0, 1.0), sq(2.5, 0.0, 1.0), sq(20.5, 20.5, 0.5)];
        let regions: Vec<Region> = polys.iter().map(|p| Region::from_polygon(p, &tol).unwrap()).collect();
        let a = union_all(&regions, &tol).unwrap();
        let b = union_by_groups(&regions, &tol).unwrap();
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x.area() - y.area()).abs() < 1e-12);
            assert_eq!(x.outer_corner_points(), y.outer_corner_points());
        }
        assert_eq!(a.len(), 4);
    }

    fn two_walls_in_an_l() -> (crate::BimDocument, ElementId) {
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
            ops::create_grid(m, cx, None, Vec2::new(0.0, -2.0), Vec2::new(0.0, 6.0))?;
            ops::create_view(m, cx, "Level 0", ViewKind::FloorPlan, Some(l0), None)
        });
        let view = out.value.expect("fixture committed");
        (doc, view)
    }

    #[test]
    fn a_joined_corner_has_no_line_across_the_join() {
        let (doc, view) = two_walls_in_an_l();
        let m = doc.model();
        let v = m.view(view).unwrap();
        let st = crate::views::view_state(m, v).unwrap();
        let prims = plan_primitives(m, v, &st);
        let cut: Vec<_> = prims.iter().filter(|p| p.role == Role::CutEdge).collect();
        // the raw footprints are 2 rectangles = 8 edges; their union is one L-shaped loop of 6 edges
        let raw: usize = st.elements.iter().filter(|e| e.appearance == Appearance::Cut).map(|e| e.footprint.len()).sum();
        assert_eq!(raw, 8);
        assert!(cut.len() < raw, "{} edges of the merged outline, {} of the raw footprints", cut.len(), raw);
        assert!(cut.len() >= 6);
    }

    #[test]
    fn pens_are_device_pixels_cut_heavier_than_projection() {
        let cut = pen_for(Role::CutEdge, 100, DetailLevel::Coarse);
        let proj = pen_for(Role::ProjectionEdge, 100, DetailLevel::Coarse);
        assert!(cut.px > proj.px);
        assert_eq!(cut.class(), 2.0);
        assert_eq!(proj.class(), 1.0);
        assert_eq!(pen_for(Role::BeyondEdge, 100, DetailLevel::Coarse).class(), 0.5);
        assert_eq!(pen_for(Role::GridLine, 100, DetailLevel::Coarse).class(), 11.0);
    }

    #[test]
    fn a_grid_draws_a_dash_dot_line_and_one_bubble_at_the_default_end() {
        let (doc, view) = two_walls_in_an_l();
        let m = doc.model();
        let v = m.view(view).unwrap();
        let st = crate::views::view_state(m, v).unwrap();
        let prims = plan_primitives(m, v, &st);
        let lines = prims.iter().filter(|p| p.role == Role::GridLine).count();
        let bubbles: Vec<_> = prims.iter().filter(|p| p.role == Role::GridBubble).collect();
        assert_eq!(lines, 1);
        // Spec G-02: the type default is a bubble at end 2 only
        assert_eq!(bubbles.len(), 1);
        let Shape::Circle { centre, radius } = bubbles[0].shape else { panic!("a bubble is a circle") };
        let g = &st.grids[0];
        // the circle touches the line's end: its centre is one radius beyond the end, on the line
        assert!((centre.distance_to(g.start) - radius).abs() < 1e-9);
        assert!((radius - GRID_BUBBLE_PAPER_DIAMETER_M * 0.5 * f64::from(st.scale)).abs() < 1e-12);
    }
}
