//! The canonical reference geometry of the model's elements: ONE place that says what body, if any, each
//! element has, for every view that draws it.
//!
//! Before this module each kind of view assembled its picture from a different producer: the plan from
//! `plan_graphics` plus additions in the FFI, the 3D view from `mesh3d` (walls and flat slabs only) plus
//! additions in the shell (columns, curtain grids), the section and elevation from `section_graphics`
//! (walls and slabs only), and the collector of a section from `docviews::element_box`, which knew no
//! column. An element type that one producer knew and another did not was drawn in some views and silently
//! absent from the others. Now the 3D mesh, the section / elevation line work and the diagnostics of
//! [`crate::view_visibility`] ask this module, so an element has the same body in every view, and an
//! element without one is NAMED ([`Geometry::NotGenerated`]) instead of dropped.
//!
//! Nothing is stored: every function is a pure function of the model, recomputed on each call. The bodies are the solids
//! the engine already regenerates for quantities (`regen::wall_view_body`, `regen::slab_geometry`, `columns::column_state`,
//! ...); this module only gathers them. What each rule is based on is stated where it is made; where the reference
//! product's own presentation is not specified it is an ENGINE CHOICE and says so.

use crate::model::{BimModel, HostedKind, SlabRole};
use asterion_core::ElementId;
use asterion_geom::Vec2;

/// `z = c0 + gx x + gy y`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lin {
    pub c0: f64,
    pub gx: f64,
    pub gy: f64,
}

impl Lin {
    pub fn flat(z: f64) -> Lin {
        Lin { c0: z, gx: 0.0, gy: 0.0 }
    }
    pub fn at(&self, p: Vec2) -> f64 {
        self.c0 + self.gx * p.x + self.gy * p.y
    }
}

/// A solid: a plan region (counter-clockwise outer loop, clockwise holes) between a bottom and a top plane.
/// A vertical prism has two flat planes; a sloped slab or a roof piece has inclined ones. Side faces are
/// always vertical.
#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    pub outer: Vec<Vec2>,
    pub holes: Vec<Vec<Vec2>>,
    pub bottom: Lin,
    pub top: Lin,
}

fn drop_closing(mut p: Vec<Vec2>) -> Vec<Vec2> {
    if p.len() > 3 && p[0] == p[p.len() - 1] {
        p.pop();
    }
    p
}

/// `(signed area, first moment x, first moment y)` of a simple polygon.
fn moments(p: &[Vec2]) -> (f64, f64, f64) {
    let n = p.len();
    let (mut a, mut mx, mut my) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (u, v) = (p[i], p[(i + 1) % n]);
        let cr = u.x * v.y - v.x * u.y;
        a += cr;
        mx += (u.x + v.x) * cr;
        my += (u.y + v.y) * cr;
    }
    (a / 2.0, mx / 6.0, my / 6.0)
}

impl Body {
    /// The prism of `footprint` between two elevations (the outline is oriented counter-clockwise).
    pub fn prism(footprint: &[Vec2], z0: f64, z1: f64) -> Body {
        let mut p = drop_closing(footprint.to_vec());
        if moments(&p).0 < 0.0 {
            p.reverse();
        }
        Body { outer: p, holes: Vec::new(), bottom: Lin::flat(z0), top: Lin::flat(z1) }
    }

    /// An axis-aligned box.
    pub fn aabb_box(lo: [f64; 3], hi: [f64; 3]) -> Body {
        Body::prism(&[Vec2::new(lo[0], lo[1]), Vec2::new(hi[0], lo[1]), Vec2::new(hi[0], hi[1]), Vec2::new(lo[0], hi[1])], lo[2], hi[2])
    }

    /// Plan area of the region (the holes subtracted).
    pub fn plan_area(&self) -> f64 {
        moments(&self.outer).0.abs() - self.holes.iter().map(|h| moments(h).0.abs()).sum::<f64>()
    }

    /// The volume: the integral of `top - bottom` over the region (both are linear, so it is the thickness at
    /// the centroid times the area).
    pub fn volume(&self) -> f64 {
        let (mut a, mut mx, mut my) = (moments(&self.outer).0.abs(), 0.0, 0.0);
        let (oa, ox, oy) = moments(&self.outer);
        let s = if oa < 0.0 { -1.0 } else { 1.0 };
        mx += s * ox;
        my += s * oy;
        for h in &self.holes {
            let (ha, hx, hy) = moments(h);
            let hs = if ha < 0.0 { -1.0 } else { 1.0 };
            a -= ha.abs();
            mx -= hs * hx;
            my -= hs * hy;
        }
        (self.top.c0 - self.bottom.c0) * a + (self.top.gx - self.bottom.gx) * mx + (self.top.gy - self.bottom.gy) * my
    }

    /// Every vertex of the outer loop at the bottom and at the top (the hull of these is the body's silhouette).
    pub fn corners(&self) -> Vec<[f64; 3]> {
        let mut out = Vec::with_capacity(self.outer.len() * 2);
        for p in &self.outer {
            out.push([p.x, p.y, self.bottom.at(*p)]);
            out.push([p.x, p.y, self.top.at(*p)]);
        }
        out
    }

    /// The world box `(min, max)`.
    pub fn aabb(&self) -> ([f64; 3], [f64; 3]) {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for c in self.corners() {
            for k in 0..3 {
                lo[k] = lo[k].min(c[k]);
                hi[k] = hi[k].max(c[k]);
            }
        }
        (lo, hi)
    }

    /// Whether both planes are horizontal (a plain vertical prism).
    pub fn is_prism(&self) -> bool {
        self.bottom.gx == 0.0 && self.bottom.gy == 0.0 && self.top.gx == 0.0 && self.top.gy == 0.0
    }
}

/// What kind of model element an id is, as far as drawing it is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ElementClass {
    Wall,
    CurtainWall,
    Floor,
    Ceiling,
    Roof,
    Column,
    Door,
    Window,
    WallOpening,
    WallSweep,
    WallReveal,
    Toposolid,
    Stairs,
    StairsRun,
    StairsLanding,
    StairsRailing,
    Room,
    RoomSeparation,
    Level,
    Grid,
    Shaft,
    Group,
    Annotation,
    View,
}

impl ElementClass {
    pub fn name(self) -> &'static str {
        match self {
            ElementClass::Wall => "wall",
            ElementClass::CurtainWall => "curtain_wall",
            ElementClass::Floor => "floor",
            ElementClass::Ceiling => "ceiling",
            ElementClass::Roof => "roof",
            ElementClass::Column => "column",
            ElementClass::Door => "door",
            ElementClass::Window => "window",
            ElementClass::WallOpening => "wall_opening",
            ElementClass::WallSweep => "wall_sweep",
            ElementClass::WallReveal => "wall_reveal",
            ElementClass::Toposolid => "toposolid",
            ElementClass::Stairs => "stairs",
            ElementClass::StairsRun => "stairs_run",
            ElementClass::StairsLanding => "stairs_landing",
            ElementClass::StairsRailing => "stairs_railing",
            ElementClass::Room => "room",
            ElementClass::RoomSeparation => "room_separation",
            ElementClass::Level => "level",
            ElementClass::Grid => "grid",
            ElementClass::Shaft => "shaft",
            ElementClass::Group => "group",
            ElementClass::Annotation => "annotation",
            ElementClass::View => "view",
        }
    }
}

/// The class of an element id, or `None` when no collection of the model holds it.
pub fn classify(m: &BimModel, id: ElementId) -> Option<ElementClass> {
    if m.walls.contains_key(&id) {
        return Some(if m.is_curtain_wall(id) { ElementClass::CurtainWall } else { ElementClass::Wall });
    }
    if let Some(f) = m.floors.get(&id) {
        return Some(match f.role {
            SlabRole::Floor => ElementClass::Floor,
            SlabRole::Ceiling => ElementClass::Ceiling,
            SlabRole::Roof => ElementClass::Roof,
        });
    }
    if m.columns.contains_key(&id) {
        return Some(ElementClass::Column);
    }
    if let Some(i) = m.hosted_instances.get(&id) {
        let window = m.hosted_types.get(&i.type_id).map_or(false, |t| t.kind == HostedKind::Window);
        return Some(if window { ElementClass::Window } else { ElementClass::Door });
    }
    if m.wall_openings.contains_key(&id) {
        return Some(ElementClass::WallOpening);
    }
    if let Some(s) = m.annot.sweeps.get(&id) {
        return Some(if s.kind == crate::annot::SweepKind::Reveal { ElementClass::WallReveal } else { ElementClass::WallSweep });
    }
    if m.docs.toposolids.contains_key(&id) {
        return Some(ElementClass::Toposolid);
    }
    if m.stairs.stairs.contains_key(&id) {
        return Some(ElementClass::Stairs);
    }
    if m.stairs.runs.contains_key(&id) {
        return Some(ElementClass::StairsRun);
    }
    if m.stairs.landings.contains_key(&id) {
        return Some(ElementClass::StairsLanding);
    }
    if m.stairs.railings.contains_key(&id) {
        return Some(ElementClass::StairsRailing);
    }
    if m.rooms.contains_key(&id) {
        return Some(ElementClass::Room);
    }
    if m.room_separations.contains_key(&id) {
        return Some(ElementClass::RoomSeparation);
    }
    if m.levels.contains_key(&id) {
        return Some(ElementClass::Level);
    }
    if m.grids.contains_key(&id) {
        return Some(ElementClass::Grid);
    }
    if m.shafts.contains_key(&id) || m.slab_openings.contains_key(&id) {
        return Some(ElementClass::Shaft);
    }
    if m.annot.groups.contains_key(&id) {
        return Some(ElementClass::Group);
    }
    if m.annot.ref_planes.contains_key(&id)
        || m.annot.curves.contains_key(&id)
        || m.annot.dims.contains_key(&id)
        || m.annot.notes.contains_key(&id)
        || m.annot.tags.contains_key(&id)
    {
        return Some(ElementClass::Annotation);
    }
    if m.views.contains_key(&id) || m.docs.views.contains_key(&id) || m.docs.markers.contains_key(&id) {
        return Some(ElementClass::View);
    }
    None
}

/// What the engine can say about an element's body.
#[derive(Clone, Debug, PartialEq)]
pub enum Geometry {
    /// The element has a body: these solids.
    Solid(Vec<Body>),
    /// The element removes material from another one (an opening, a reveal): these are the removed solids.
    Void(Vec<Body>),
    /// The element has no body BY DESIGN (a level, a grid, a room, an annotation): it is drawn as line work,
    /// text or not at all. The reason is for a human.
    Abstract(String),
    /// The element should have a body and the engine does not generate one: a gap, never to be read as
    /// "cropped" or "hidden". The reason names the gap.
    NotGenerated(String),
    /// Regenerating the element failed.
    Failed(String),
}

impl Geometry {
    pub fn solids(&self) -> &[Body] {
        match self {
            Geometry::Solid(b) => b,
            _ => &[],
        }
    }
    pub fn voids(&self) -> &[Body] {
        match self {
            Geometry::Void(b) => b,
            _ => &[],
        }
    }
    pub fn has_solid(&self) -> bool {
        matches!(self, Geometry::Solid(b) if !b.is_empty())
    }
    /// A short label for logs and the diagnosis.
    pub fn label(&self) -> String {
        match self {
            Geometry::Solid(b) => format!("solid ({} bod{})", b.len(), if b.len() == 1 { "y" } else { "ies" }),
            Geometry::Void(b) => format!("void ({} bod{})", b.len(), if b.len() == 1 { "y" } else { "ies" }),
            Geometry::Abstract(r) => format!("no body by design: {r}"),
            Geometry::NotGenerated(r) => format!("NOT GENERATED: {r}"),
            Geometry::Failed(r) => format!("regeneration FAILED: {r}"),
        }
    }
}

pub mod tone {
    pub const WALL: [f32; 3] = [0.70, 0.70, 0.70];
    pub const FLOOR: [f32; 3] = [0.80, 0.80, 0.78];
    pub const CEILING: [f32; 3] = [0.90, 0.90, 0.90];
    pub const ROOF: [f32; 3] = [0.55, 0.55, 0.55];
    pub const COLUMN: [f32; 3] = [0.76, 0.76, 0.74];
    pub const GLASS: [f32; 3] = [0.62, 0.78, 0.88];
    pub const MULLION: [f32; 3] = [0.35, 0.35, 0.37];
    pub const SWEEP: [f32; 3] = [0.62, 0.62, 0.62];
    pub const TOPOSOLID: [f32; 3] = [0.55, 0.60, 0.45];
}

/// The colour a class is drawn with.
pub fn class_tone(c: ElementClass) -> [f32; 3] {
    match c {
        ElementClass::Floor => tone::FLOOR,
        ElementClass::Ceiling => tone::CEILING,
        ElementClass::Roof => tone::ROOF,
        ElementClass::Column => tone::COLUMN,
        ElementClass::WallSweep => tone::SWEEP,
        ElementClass::Toposolid => tone::TOPOSOLID,
        _ => tone::WALL,
    }
}

/// The mullion section a curtain wall's mullion type gets when the template does not name a size (the 3D view of the shell
/// always drew 25 mm half sections: ENGINE CHOICE).
pub const MULLION_FALLBACK_HALF_M: f64 = 0.025;

fn stairs_gap() -> Geometry {
    Geometry::NotGenerated("the stairs module has no solid (treads, risers, stringers): only run location lines and quantities exist (OQ-STAIR-GEOM)".into())
}

/// The curtain wall's parts: glass boxes and mullion boxes, or the wall's envelope when the grid has no part.
fn curtain_geometry(m: &BimModel, wall: ElementId, envelope: Vec<Body>) -> Geometry {
    let Ok(grid) = crate::curtains::grid_state(m, wall) else {
        return Geometry::Solid(envelope);
    };
    let w = match m.walls.get(&wall) {
        Some(w) => w,
        None => return Geometry::Solid(envelope),
    };
    let mut bodies = Vec::new();
    // the glass: the world box the grid reports for each panel (exact for a wall along an axis; the box of an
    // oblique wall's glass is the axis-aligned box that holds it)
    for p in grid.panels.iter().filter(|p| !p.empty) {
        let (lo, hi) = (p.bbox_min, p.bbox_max);
        if hi[0] >= lo[0] && hi[1] >= lo[1] && hi[2] > lo[2] && (hi[0] > lo[0] || hi[1] > lo[1]) {
            let (lo, hi) = ([lo[0], lo[1], lo[2]], [hi[0].max(lo[0] + 1e-6), hi[1].max(lo[1] + 1e-6), hi[2]]);
            bodies.push(Body::aabb_box(lo, hi));
        }
    }
    let d = (w.end - w.start).normalized().unwrap_or(Vec2::new(1.0, 0.0));
    let n = d.perp_ccw();
    for mu in &grid.mullions {
        let size = m
            .mullion_types
            .values()
            .find(|t| t.name == mu.type_name)
            .and_then(|t| crate::curtains::template_size(&t.name));
        if let Some(b) = mu.frame {
            // the oriented box the grid computed (inside the wall edge for a border mullion, cut back to the vertical mullions for a
            // horizontal one)
            let (a, c) = (w.start + d * b.s0, w.start + d * b.s1);
            let corners = [a - n * b.half_t, c - n * b.half_t, c + n * b.half_t, a + n * b.half_t];
            bodies.push(Body::prism(&corners, b.z0, b.z1));
            continue;
        }
        let (side, thick) = size.map_or((MULLION_FALLBACK_HALF_M, MULLION_FALLBACK_HALF_M), |s| (s.side1.max(s.side2), s.half_thickness));
        let (s, e) = (Vec2::new(mu.start[0], mu.start[1]), Vec2::new(mu.end[0], mu.end[1]));
        let (z0, z1) = (mu.start[2].min(mu.end[2]), mu.start[2].max(mu.end[2]));
        let vertical = (z1 - z0) > 1e-9;
        // along the wall by `side` (a vertical mullion) or by its own length (a horizontal one), across it by
        // its half thickness
        let (a, b) = if vertical { (s - d * side, s + d * side) } else { (s, e) };
        let (zl, zh) = if vertical { (z0, z1) } else { (z0 - side, z0 + side) };
        let corners = [a - n * thick, b - n * thick, b + n * thick, a + n * thick];
        bodies.push(Body::prism(&corners, zl, zh));
    }
    if bodies.is_empty() {
        Geometry::Solid(envelope)
    } else {
        Geometry::Solid(bodies)
    }
}

/// The body of an element, as every view draws it.
pub fn geometry_of(m: &BimModel, id: ElementId) -> Geometry {
    let Some(class) = classify(m, id) else {
        return Geometry::Failed("no collection of the model holds this id".into());
    };
    match class {
        ElementClass::Wall | ElementClass::CurtainWall => {
            let w = &m.walls[&id];
            match crate::regen::wall_view_body(m, w) {
                Ok((fp, z0, z1)) if fp.len() >= 3 && z1 > z0 => {
                    let envelope = vec![Body::prism(&fp, z0, z1)];
                    if class == ElementClass::CurtainWall {
                        curtain_geometry(m, id, envelope)
                    } else {
                        Geometry::Solid(envelope)
                    }
                }
                Ok(_) => Geometry::NotGenerated("the wall has no positive height or no plan outline".into()),
                Err(e) => Geometry::Failed(e.message),
            }
        }
        ElementClass::Floor | ElementClass::Ceiling | ElementClass::Roof => crate::regen::slab_geometry(m, &m.floors[&id]),
        ElementClass::Column => match crate::columns::column_state(m, &m.columns[&id]) {
            Ok(s) if s.height > 0.0 && s.footprint.len() >= 3 => Geometry::Solid(vec![Body::prism(&s.footprint, s.base_z, s.top_z)]),
            Ok(_) => Geometry::NotGenerated("the column's height is not positive".into()),
            Err(e) => Geometry::Failed(e.message),
        },
        ElementClass::Door | ElementClass::Window => match crate::regen::hosted_instance_state(m, &m.hosted_instances[&id]) {
            Ok(s) => match s.solid {
                Some(p) => Geometry::Void(vec![Body::prism(&p.footprint, p.z0, p.z1)]),
                None => Geometry::NotGenerated("the insert's cut misses its host wall".into()),
            },
            Err(e) => Geometry::Failed(e.message),
        },
        ElementClass::WallOpening => match crate::regen::opening_state(m, &m.wall_openings[&id]) {
            Ok(s) => match s.solid {
                Some(p) => Geometry::Void(vec![Body::prism(&p.footprint, p.z0, p.z1)]),
                None => Geometry::NotGenerated("the opening's cut misses its host wall".into()),
            },
            Err(e) => Geometry::Failed(e.message),
        },
        ElementClass::WallSweep | ElementClass::WallReveal => {
            let sw = &m.annot.sweeps[&id];
            match crate::sweeps::body(m, sw) {
                Some(b) => {
                    let tess = |poly: &asterion_geom::Polycurve| poly.tessellate(0.005);
                    let bodies: Vec<Body> = b
                        .pieces
                        .iter()
                        .filter(|r| r.outer().corner_points().len() >= 3 && b.z1 > b.z0)
                        .map(|r| {
                            let mut body = Body::prism(&tess(r.outer()), b.z0, b.z1);
                            body.holes = r.holes().iter().map(|h| {
                                let mut p = drop_closing(tess(h));
                                if moments(&p).0 > 0.0 {
                                    p.reverse();
                                }
                                p
                            }).collect();
                            body
                        })
                        .collect();
                    if bodies.is_empty() {
                        Geometry::NotGenerated("the sweep's body has no region".into())
                    } else if class == ElementClass::WallReveal {
                        Geometry::Void(bodies)
                    } else {
                        Geometry::Solid(bodies)
                    }
                }
                None => Geometry::NotGenerated("the sweep has no body (it lies off its wall or the wall has none)".into()),
            }
        }
        ElementClass::Toposolid => {
            let t = &m.docs.toposolids[&id];
            if !t.points.is_empty() && t.host.is_none() {
                return Geometry::NotGenerated("a toposolid with surface points has a triangulated top: the engine reports its quantities, not a body (OQ-TOPOSOLID-BODY)".into());
            }
            match (crate::toposolid::toposolid_state(m, id), crate::toposolid::outline(m, id)) {
                (Some(s), Some(o)) if o.len() >= 3 && s.bbox.1[2] > s.bbox.0[2] => Geometry::Solid(vec![Body::prism(&o, s.bbox.0[2], s.bbox.1[2])]),
                _ => Geometry::NotGenerated("the toposolid's level or outline is missing".into()),
            }
        }
        ElementClass::Stairs | ElementClass::StairsRun | ElementClass::StairsLanding | ElementClass::StairsRailing => stairs_gap(),
        ElementClass::Room => Geometry::Abstract("a room is a space: drawn in plans by its tag or seed point, it has no body in 3D or in sections".into()),
        ElementClass::RoomSeparation => Geometry::Abstract("a room separation is a line of the plan".into()),
        ElementClass::Level => Geometry::Abstract("a level is a datum plane: drawn as a line in sections and elevations".into()),
        ElementClass::Grid => Geometry::Abstract("a grid is a datum: drawn in plans, never in sections or elevations (OBSERVED)".into()),
        ElementClass::Shaft => Geometry::Abstract("a shaft or opening: it cuts the slabs it crosses, whose bodies carry the hole".into()),
        ElementClass::Group => Geometry::Abstract("a group: its members are drawn as themselves".into()),
        ElementClass::Annotation => Geometry::Abstract("annotation: drawn by the view that owns it".into()),
        ElementClass::View => Geometry::Abstract("a view or an elevation marker is not model geometry".into()),
    }
}

/// What the engine has no body for, in the whole model: `(what, count, reason)` by reason, so a window can say it instead
/// of drawing nothing ("what the script does not verify"). Stairs, runs, landings and railings are counted without
/// regenerating anything; slabs (floors, ceilings, roofs) and toposolids ask [`geometry_of`], so a shape-edited slab or a
/// toposolid with surface points is named by the reason the registry gives. A body that FAILED to regenerate is listed too,
/// as "failed". Elements that have a body (even a partial one) are not listed here: their partial parts are named in the
/// docs.
pub fn model_gaps(m: &BimModel) -> Vec<(String, usize, String)> {
    let mut by: std::collections::BTreeMap<(String, String), usize> = std::collections::BTreeMap::new();
    let mut add = |what: &str, reason: String| *by.entry((what.to_string(), reason)).or_insert(0) += 1;
    let stairs_reason = "stairs have no body in the engine (OQ-STAIR-GEOM)";
    for _ in m.stairs.stairs.keys() {
        add("stairs", stairs_reason.into());
    }
    for _ in m.stairs.runs.keys() {
        add("stair runs", stairs_reason.into());
    }
    for _ in m.stairs.landings.keys() {
        add("stair landings", stairs_reason.into());
    }
    for _ in m.stairs.railings.keys() {
        add("railings", "railings have no body in the engine (OQ-STAIR-GEOM)".into());
    }
    for id in m.floors.keys().chain(m.docs.toposolids.keys()) {
        let what = classify(m, *id).map_or("element", |c| c.name());
        match geometry_of(m, *id) {
            Geometry::NotGenerated(r) => add(what, r),
            Geometry::Failed(r) => add(what, format!("failed: {r}")),
            _ => {}
        }
    }
    by.into_iter().map(|((what, reason), n)| (what, n, reason)).collect()
}

/// The world box of an element's solids (`None` without a solid).
pub fn bbox(m: &BimModel, id: ElementId) -> Option<([f64; 3], [f64; 3])> {
    let g = geometry_of(m, id);
    let mut acc: Option<([f64; 3], [f64; 3])> = None;
    for b in g.solids() {
        let (lo, hi) = b.aabb();
        acc = Some(match acc {
            None => (lo, hi),
            Some((a, c)) => ([a[0].min(lo[0]), a[1].min(lo[1]), a[2].min(lo[2])], [c[0].max(hi[0]), c[1].max(hi[1]), c[2].max(hi[2])]),
        });
    }
    acc
}

/// A stable fingerprint of an element's reference geometry (vertices rounded to 1e-9 m): equal for the same
/// body whatever view asks, different as soon as the body changes.
pub fn signature(g: &Geometry) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut put = |v: f64| {
        let q = (v * 1e9).round() as i64;
        for byte in q.to_le_bytes() {
            h ^= byte as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    let bodies: &[Body] = match g {
        Geometry::Solid(b) | Geometry::Void(b) => b,
        _ => &[],
    };
    for b in bodies {
        for l in std::iter::once(&b.outer).chain(b.holes.iter()) {
            put(l.len() as f64);
            for p in l.iter() {
                put(p.x);
                put(p.y);
            }
        }
        for v in [b.bottom, b.top] {
            put(v.c0);
            put(v.gx);
            put(v.gy);
        }
    }
    h
}

// ------------------------------------------------------------------------------------------------ walls with openings

/// A rectangle cut through a wall, in the wall's own coordinates: stations along its location line and
/// absolute elevations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WallCut {
    /// The door, window or opening that makes the cut.
    pub element: ElementId,
    pub u_lo: f64,
    pub u_hi: f64,
    pub z_lo: f64,
    pub z_hi: f64,
}

/// The cuts of a straight wall that remove material (the host-side insert relation of `regen::inserts`, already clipped to the
/// wall's outline). An arc wall has none here: its openings are not cut from the drawn body (a recorded gap).
pub fn wall_cuts(m: &BimModel, wall: ElementId) -> Vec<WallCut> {
    let Some(w) = m.walls.get(&wall) else { return Vec::new() };
    if w.through.is_some() {
        return Vec::new();
    }
    crate::regen::inserts(m, wall)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| i.cut.map(|c| WallCut { element: i.element, u_lo: c.u_lo, u_hi: c.u_hi, z_lo: c.z_lo, z_hi: c.z_hi }))
        .collect()
}

fn clip_half_plane(poly: &[Vec2], f: impl Fn(Vec2) -> f64) -> Vec<Vec2> {
    let n = poly.len();
    let mut out = Vec::with_capacity(n + 2);
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        let (fa, fb) = (f(a), f(b));
        let (ina, inb) = (fa >= 0.0, fb >= 0.0);
        if ina {
            out.push(a);
        }
        if ina != inb {
            let t = fa / (fa - fb);
            out.push(Vec2::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t));
        }
    }
    out
}

/// A straight wall's gross prism with the cuts taken out, as prisms: the wall split along its length where a cut starts or
/// ends, and each part reduced to the elevations no cut covers there. The pieces tile the wall exactly, so their volumes add
/// up to the gross volume less the volume the cuts remove.
pub fn wall_pieces(gross: &Body, start: Vec2, dir: Vec2, cuts: &[WallCut]) -> Vec<Body> {
    const EPS: f64 = 1e-9;
    let (z0, z1) = (gross.bottom.c0, gross.top.c0);
    let us: Vec<f64> = gross.outer.iter().map(|p| (*p - start).dot(dir)).collect();
    let (umin, umax) = us.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), u| (l.min(*u), h.max(*u)));
    let cuts: Vec<WallCut> = cuts
        .iter()
        .filter_map(|c| {
            let c = WallCut { u_lo: c.u_lo.max(umin), u_hi: c.u_hi.min(umax), z_lo: c.z_lo.max(z0), z_hi: c.z_hi.min(z1), ..*c };
            (c.u_hi > c.u_lo + EPS && c.z_hi > c.z_lo + EPS).then_some(c)
        })
        .collect();
    if cuts.is_empty() {
        return vec![gross.clone()];
    }
    let mut bounds = vec![umin, umax];
    for c in &cuts {
        bounds.extend([c.u_lo, c.u_hi]);
    }
    bounds.sort_by(|a, b| a.total_cmp(b));
    bounds.dedup_by(|a, b| (*a - *b).abs() < EPS);
    // for each span of stations, the elevations that stay (as pairs), merging neighbours that keep the same
    let mut spans: Vec<(f64, f64, Vec<(f64, f64)>)> = Vec::new();
    for w in bounds.windows(2) {
        let (ua, ub) = (w[0], w[1]);
        if ub - ua < EPS {
            continue;
        }
        let mid = 0.5 * (ua + ub);
        let mut covered: Vec<(f64, f64)> = cuts.iter().filter(|c| c.u_lo <= mid && mid <= c.u_hi).map(|c| (c.z_lo, c.z_hi)).collect();
        covered.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut keep: Vec<(f64, f64)> = Vec::new();
        let mut z = z0;
        for (a, b) in covered {
            if a > z + EPS {
                keep.push((z, a));
            }
            z = z.max(b);
        }
        if z1 > z + EPS {
            keep.push((z, z1));
        }
        match spans.last_mut() {
            Some(last) if last.2.len() == keep.len() && last.2.iter().zip(&keep).all(|(p, q)| (p.0 - q.0).abs() < EPS && (p.1 - q.1).abs() < EPS) => last.1 = ub,
            _ => spans.push((ua, ub, keep)),
        }
    }
    let mut out = Vec::new();
    for (ua, ub, keep) in spans {
        if keep.is_empty() {
            continue;
        }
        let part = clip_half_plane(&gross.outer, |p| (p - start).dot(dir) - ua);
        let part = clip_half_plane(&part, |p| ub - (p - start).dot(dir));
        if part.len() < 3 || moments(&part).0.abs() < EPS {
            continue;
        }
        for (a, b) in keep {
            out.push(Body::prism(&part, a, b));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Vec2> {
        vec![Vec2::new(x0, y0), Vec2::new(x1, y0), Vec2::new(x1, y1), Vec2::new(x0, y1)]
    }

    #[test]
    fn a_prism_has_its_volume_and_a_slanted_body_the_volume_at_its_centroid() {
        let b = Body::prism(&rect(0.0, 0.0, 4.0, 3.0), 1.0, 3.0);
        assert!((b.volume() - 24.0).abs() < 1e-9 && (b.plan_area() - 12.0).abs() < 1e-9);
        // a roof piece: bottom z = x / 2, top z = x / 2 + 0.3 over 4 x 3: volume 0.3 x 12
        let slanted = Body { outer: b.outer.clone(), holes: vec![], bottom: Lin { c0: 0.0, gx: 0.5, gy: 0.0 }, top: Lin { c0: 0.3, gx: 0.5, gy: 0.0 } };
        assert!((slanted.volume() - 3.6).abs() < 1e-9, "{}", slanted.volume());
        // a hole takes its own volume out
        let mut holed = b.clone();
        holed.holes.push(vec![Vec2::new(1.0, 1.0), Vec2::new(1.0, 2.0), Vec2::new(2.0, 2.0), Vec2::new(2.0, 1.0)]);
        assert!((holed.volume() - 22.0).abs() < 1e-9, "{}", holed.volume());
    }

    #[test]
    fn the_pieces_of_a_wall_tile_it_less_what_the_cuts_remove() {
        let gross = Body::prism(&rect(0.0, -0.1, 6.0, 0.1), 0.0, 3.0);
        let cut = |u0: f64, u1: f64, z0: f64, z1: f64| WallCut { element: ElementId(9), u_lo: u0, u_hi: u1, z_lo: z0, z_hi: z1 };
        // a door from the floor and a window between two stations
        let pieces = wall_pieces(&gross, Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), &[cut(1.0, 2.0, 0.0, 2.1), cut(3.0, 4.5, 0.9, 2.1)]);
        let removed = 1.0 * 2.1 * 0.2 + 1.5 * 1.2 * 0.2;
        let vol: f64 = pieces.iter().map(Body::volume).sum();
        assert!((vol - (gross.volume() - removed)).abs() < 1e-9, "{vol} vs {}", gross.volume() - removed);
        // no cut: the wall as it is
        assert_eq!(wall_pieces(&gross, Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), &[]), vec![gross.clone()]);
        // a cut that hangs past the wall end is clipped to it
        let pieces = wall_pieces(&gross, Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), &[cut(5.0, 7.0, 0.0, 1.0)]);
        let vol: f64 = pieces.iter().map(Body::volume).sum();
        assert!((vol - (gross.volume() - 1.0 * 1.0 * 0.2)).abs() < 1e-9, "{vol}");
    }

    fn doc_with_slabs() -> (crate::BimDocument, ElementId, ElementId, ElementId, ElementId, ElementId) {
        use crate::{ops, CompoundStructure, Layer, LayerFunction};
        let mut d = crate::new_document();
        let out = d
            .transact("fx", false, |m, cx| {
                let l0 = ops::create_level(m, cx, "L0", 0.0)?;
                let wt = ops::create_wall_type(m, cx, "W", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
                let wall = ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l0, start: Vec2::new(0.0, 0.0), end: Vec2::new(6.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false })?;
                let dt = ops::create_hosted_type(m, cx, "D", HostedKind::Door, 1.0, 2.1, 0.0, 0.0, 0.0, 0.0, 0.0)?;
                ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: dt, host: wall, level: l0, host_station: 1.5, sill_height: None })?;
                let wnt = ops::create_hosted_type(m, cx, "Win", HostedKind::Window, 1.2, 1.2, 0.0, 0.0, 0.0, 0.9, 0.0)?;
                ops::place_hosted_instance(m, cx, ops::NewHostedInstance { type_id: wnt, host: wall, level: l0, host_station: 4.0, sill_height: None })?;
                let ft = ops::create_slab_type(m, cx, "F", 0.25, SlabRole::Floor)?;
                let rect = vec![Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0), Vec2::new(6.0, 4.0), Vec2::new(0.0, 4.0)];
                let holed = ops::create_slab_with_loops(m, cx, ft, l0, rect.clone(), vec![vec![Vec2::new(1.0, 1.0), Vec2::new(2.0, 1.0), Vec2::new(2.0, 2.0), Vec2::new(1.0, 2.0)]], 0.0, false)?;
                let sloped = ops::create_sloped_slab(m, cx, ft, l0, rect.clone(), 0.0, false, Vec2::new(0.0, 2.0), Vec2::new(6.0, 2.0), 0.1)?;
                let rt = ops::create_slab_type(m, cx, "R", 0.3, SlabRole::Roof)?;
                let roof = ops::create_slab(m, cx, rt, l0, rect, 3.0, false)?;
                ops::set_slab_segment_slope(m, cx, roof, 0, true, 0.5)?;
                ops::set_slab_segment_slope(m, cx, roof, 2, true, 0.5)?;
                Ok((wall, holed, sloped, roof, l0))
            })
            .value
            .expect("fixture");
        (d, out.0, out.1, out.2, out.3, out.4)
    }

    /// The bodies the views draw ARE the solids whose quantities the engine measures: their volumes agree.
    #[test]
    fn the_bodies_have_the_volume_the_engine_reports() {
        let (d, wall, holed, sloped, roof, _) = doc_with_slabs();
        let m = d.model();
        // a holed floor
        let st = crate::regen::slab_state(m, &m.floors[&holed]).unwrap();
        let g = geometry_of(m, holed);
        let vol: f64 = g.solids().iter().map(Body::volume).sum();
        assert!((vol - st.volume).abs() < 1e-9, "holed floor {vol} vs {}", st.volume);
        assert_eq!(g.solids().len(), 1);
        assert_eq!(g.solids()[0].holes.len(), 1);
        // a sloped floor
        let st = crate::regen::slab_state(m, &m.floors[&sloped]).unwrap();
        let vol: f64 = geometry_of(m, sloped).solids().iter().map(Body::volume).sum();
        assert!((vol - st.volume).abs() < 1e-9, "sloped floor {vol} vs {}", st.volume);
        assert!(!geometry_of(m, sloped).solids()[0].is_prism(), "a sloped floor is not drawn as a flat prism");
        // a gable roof: the pieces ARE the solid roof_solid measures
        let st = crate::regen::slab_state(m, &m.floors[&roof]).unwrap();
        let g = geometry_of(m, roof);
        let vol: f64 = g.solids().iter().map(Body::volume).sum();
        assert!((vol - st.volume).abs() < 1e-9, "roof {vol} vs {}", st.volume);
        let (lo, hi) = bbox(m, roof).unwrap();
        assert!((lo[2] - st.bbox_min.z).abs() < 1e-9 && (hi[2] - st.bbox_max.z).abs() < 1e-9, "{lo:?} {hi:?} vs {:?} {:?}", st.bbox_min, st.bbox_max);
        // a wall with a door and a window: the pieces tile it less the cuts, as the wall's own Volume says
        let w = &m.walls[&wall];
        let ws = crate::regen::wall_state(m, w).unwrap();
        let gross = &geometry_of(m, wall).solids()[0].clone();
        let pieces = wall_pieces(gross, w.start, (w.end - w.start).normalized().unwrap(), &wall_cuts(m, wall));
        let vol: f64 = pieces.iter().map(Body::volume).sum();
        assert!((vol - ws.volume).abs() < 1e-9, "wall {vol} vs {}", ws.volume);
        assert_eq!(wall_cuts(m, wall).len(), 2);
    }

    /// A vertical plane through a gable roof, square to its ridge, cuts the two slopes: the polygon the section draws has the
    /// area of the cross-section, which is the vertical thickness times the run.
    #[test]
    fn a_section_through_a_gable_is_its_cross_section() {
        use crate::docviews::{self, DocView};
        let (mut d, _, _, _, roof, _) = doc_with_slabs();
        let sec = d
            .transact("s", false, |m, cx| docviews::add_section_view(m, cx, Some("across"), [3.0, -3.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 0.0, -10.0], [10.0, 8.0, 0.0]))
            .value
            .expect("section");
        let m = d.model();
        let v: &DocView = &m.docs.views[&sec];
        let figs = crate::section_graphics::element_figures(v, roof, crate::element_geometry::geometry_of(m, roof).solids());
        assert!(!figs.is_empty() && figs.iter().all(|f| f.cut && f.poly.is_some()), "{figs:?}");
        let area: f64 = figs
            .iter()
            .map(|f| {
                let p = f.poly.as_ref().unwrap();
                (0..p.len()).map(|i| p[i][0] * p[(i + 1) % p.len()][1] - p[(i + 1) % p.len()][0] * p[i][1]).sum::<f64>().abs() / 2.0
            })
            .sum();
        let expected = 4.0 * 0.3 * (1.0f64 + 0.25).sqrt();
        assert!((area - expected).abs() < 1e-6, "cut area {area} vs {expected}");
    }

    #[test]
    fn the_signature_follows_the_body() {
        let a = Geometry::Solid(vec![Body::prism(&rect(0.0, 0.0, 1.0, 1.0), 0.0, 1.0)]);
        let b = Geometry::Solid(vec![Body::prism(&rect(0.0, 0.0, 1.0, 1.0), 0.0, 1.0)]);
        let c = Geometry::Solid(vec![Body::prism(&rect(0.0, 0.0, 1.0, 1.0), 0.0, 1.001)]);
        assert_eq!(signature(&a), signature(&b));
        assert_ne!(signature(&a), signature(&c));
    }
}
