//! Persistent BIM model state. Derived quantities are never stored here; they are
//! regenerated from this state by [`crate::regen`].

use crate::spec::codes;
use asterion_core::{ElementId, Failure};
use asterion_geom::Vec2;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Level {
    pub id: ElementId,
    pub name: String,
    /// Metres above the project origin.
    pub elevation: f64,
    /// The level that is the top of this level's story (the Level's "story above" element-id parameter; `None` = the
    /// invalid id). Spec L-01: it is validated at regeneration of the level and cleared when it names a level that is not
    /// above this one or is not a building story. Which UI path writes it is UNKNOWN (Spec L-01).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub story_above: Option<ElementId>,
    /// The building-story flag of the level; Spec L-01: true at construction.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub building_story: bool,
}

fn is_true(b: &bool) -> bool {
    *b
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerFunction {
    Structure,
    Substrate,
    ThermalAir,
    Membrane,
    Finish1,
    Finish2,
}

impl LayerFunction {
    pub fn reference_name(self) -> &'static str {
        match self {
            LayerFunction::Structure => "Structure",
            LayerFunction::Substrate => "Substrate",
            LayerFunction::ThermalAir => "Insulation",
            LayerFunction::Membrane => "Membrane",
            LayerFunction::Finish1 => "Finish1",
            LayerFunction::Finish2 => "Finish2",
        }
    }

    /// The scenario vocabulary (`structure`, `substrate`, `thermal_air` / `insulation`, `membrane`, `finish1`, `finish2`).
    pub fn from_scenario_name(s: &str) -> Option<LayerFunction> {
        match s.to_ascii_lowercase().as_str() {
            "structure" => Some(LayerFunction::Structure),
            "substrate" => Some(LayerFunction::Substrate),
            "thermal_air" | "insulation" => Some(LayerFunction::ThermalAir),
            "membrane" => Some(LayerFunction::Membrane),
            "finish1" => Some(LayerFunction::Finish1),
            "finish2" => Some(LayerFunction::Finish2),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub width: f64,
    pub function: LayerFunction,
}

/// Ordered layers from the exterior face to the interior face. The core is the
/// run of layers between the exterior and interior shells.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompoundStructure {
    pub layers: Vec<Layer>,
    pub exterior_shell_layers: usize,
    pub interior_shell_layers: usize,
}

/// Why a layer width is refused, or `None` when it is valid. Widths in METRES, tested in feet (internal units).
///
/// * a width must be a non-negative length: `-1e-9 <= w <= 30000.000000001` ft, else "a layer cannot have negative width".
/// * a width with `|w| <= 1e-9` ft is accepted only for the layer function codes 100 (membrane),
///   0 and 200 (structural deck), or a layer with a non-rectangular region; otherwise "only
///   membrane layers can have zero width";
/// * a layer with function 200 must have `|w| <= 1e-9` ("structural deck must have zero width").
///
/// UNKNOWN: what API function the internal code 0 is - none of this engine's functions is mapped to it, so only `Membrane`
/// (100) may be zero; structural deck (200) and non-rectangular regions are not represented by this engine's `Layer`.
pub fn layer_width_fault(width_m: f64, function: LayerFunction) -> Option<&'static str> {
    let w = width_m * crate::spec::FEET_PER_METRE;
    if !w.is_finite() || !(-1e-9..=30000.000000001).contains(&w) {
        return Some("a layer cannot have a negative width");
    }
    if w.abs() <= 1e-9 && function != LayerFunction::Membrane {
        return Some("only membrane layers can have zero width");
    }
    None
}

impl CompoundStructure {
    /// Validates and builds a structure.
    ///
    /// OBSERVED: a single structure layer of width 0 is rejected
    /// ("One or more layers is not valid."). DOCUMENTED: membrane layers may have
    /// zero thickness. ASSUMED: shells must leave at least one core layer.
    pub fn new(layers: Vec<Layer>, exterior_shell_layers: usize, interior_shell_layers: usize) -> Result<Self, Failure> {
        if layers.is_empty() {
            return Err(Failure::error(codes::INVALID_LAYERS, "A compound structure needs at least one layer."));
        }
        for (i, l) in layers.iter().enumerate() {
            if let Some(fault) = layer_width_fault(l.width, l.function) {
                return Err(Failure::error(
                    codes::INVALID_LAYERS,
                    format!("Layer {i} is not valid (width {}): {fault}.", l.width),
                ));
            }
        }
        if exterior_shell_layers + interior_shell_layers >= layers.len() {
            return Err(Failure::error(codes::INVALID_LAYERS, "Shell layers leave no core layer."));
        }
        Ok(CompoundStructure { layers, exterior_shell_layers, interior_shell_layers })
    }

    pub fn width(&self) -> f64 {
        self.layers.iter().map(|l| l.width).sum()
    }

    pub fn first_core_layer_index(&self) -> usize {
        self.exterior_shell_layers
    }

    pub fn last_core_layer_index(&self) -> usize {
        self.layers.len() - 1 - self.interior_shell_layers
    }

    /// Signed distance from the wall centre plane to `line`, positive towards the exterior face. OBSERVED for all six
    /// values on symmetric and asymmetric structures.
    pub fn offset_for_location_line(&self, line: LocationLine) -> f64 {
        let half = self.width() / 2.0;
        let ext_shell: f64 = self.layers[..self.exterior_shell_layers].iter().map(|l| l.width).sum();
        let int_shell: f64 = self.layers[self.layers.len() - self.interior_shell_layers..].iter().map(|l| l.width).sum();
        let core_ext = half - ext_shell;
        let core_int = -half + int_shell;
        match line {
            LocationLine::WallCenterline => 0.0,
            LocationLine::CoreCenterline => (core_ext + core_int) / 2.0,
            LocationLine::FinishFaceExterior => half,
            LocationLine::FinishFaceInterior => -half,
            LocationLine::CoreFaceExterior => core_ext,
            LocationLine::CoreFaceInterior => core_int,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WallType {
    pub id: ElementId,
    pub name: String,
    pub structure: CompoundStructure,
}

/// Wall location line. Integer codes are the values OBSERVED for the reference "Location Line" parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocationLine {
    WallCenterline = 0,
    CoreCenterline = 1,
    FinishFaceExterior = 2,
    FinishFaceInterior = 3,
    CoreFaceExterior = 4,
    CoreFaceInterior = 5,
}

impl LocationLine {
    pub const ALL: [LocationLine; 6] = [
        LocationLine::WallCenterline,
        LocationLine::CoreCenterline,
        LocationLine::FinishFaceExterior,
        LocationLine::FinishFaceInterior,
        LocationLine::CoreFaceExterior,
        LocationLine::CoreFaceInterior,
    ];

    pub fn code(self) -> i64 {
        self as i64
    }

    pub fn name(self) -> &'static str {
        match self {
            LocationLine::WallCenterline => "wall_centerline",
            LocationLine::CoreCenterline => "core_centerline",
            LocationLine::FinishFaceExterior => "finish_face_exterior",
            LocationLine::FinishFaceInterior => "finish_face_interior",
            LocationLine::CoreFaceExterior => "core_face_exterior",
            LocationLine::CoreFaceInterior => "core_face_interior",
        }
    }

    pub fn from_name(s: &str) -> Option<LocationLine> {
        LocationLine::ALL.into_iter().find(|l| l.name() == s)
    }
}


/// A straight wall instance. `start`/`end` describe the wall centre line in plan; it is what the reference API reports as
/// the location curve in every observed case (scenario: changing the location line leaves it in place).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Wall {
    pub id: ElementId,
    pub type_id: ElementId,
    pub base_level: ElementId,
    pub base_offset: f64,
    /// Level the top is attached to; `None` = unconnected.
    pub top_level: Option<ElementId>,
    /// Stored even while unconnected. OBSERVED: deleting the top level keeps the offset value, while unconnecting through
    /// the parameter resets it to 0.
    pub top_offset: f64,
    /// Height used while unconnected (and the last value set or computed).
    pub unconnected_height: f64,
    pub start: Vec2,
    pub end: Vec2,
    pub location_line: LocationLine,
    pub flipped: bool,
    pub structural: bool,
    /// Join permission at start (0) and end (1). Joins themselves are not modelled yet.
    pub join_allowed: [bool; 2],
    /// A point the location ARC passes through, between `start` and `end`; `None` for a straight wall.
    #[serde(default)]
    pub through: Option<Vec2>,
    /// Whether the wall bounds rooms. OBSERVED: a wall that stops being room-bounding stops enclosing, and toggling it back
    /// restores the room - the same built-in parameter a ceiling and a roof carry. Defaults to true.
    #[serde(default = "crate::model::default_true")]
    pub room_bounding: bool,
}

// ---------------------------------------------------------------------------- rooms
//
// A room is DERIVED: a seed point, a level, limits and a name/number. four walls thickened, deleted or lowered all changed
// the room with no action on it.

/// A room: the stored half of the element.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Room {
    pub id: ElementId,
    pub level: ElementId,
    /// The seed point the room was placed at.
    pub point: Vec2,
    /// `Base Offset`: where the room starts above its level. OBSERVED: refused AT COMMIT when it is
    /// above the Computation Height, which is read-only 0.
    pub base_offset: f64,
    /// `Limit Offset`: OBSERVED default 2.4384 m (8 ft), the same as the unbounded height.
    pub limit_offset: f64,
    /// `Upper Limit`: `None` reads as the room's own level.
    pub upper_limit: Option<ElementId>,
    /// The name WITHOUT the number: the reference's `Name` reads "name + space + number".
    pub name: String,
    pub number: String,
    /// The room carries a tag (its name and number drawn at its point in the plans of its level). OBSERVED: the Room tool's
    /// "Tag on Placement" (on by default) places a tag with the room (paired room run, 2026-10-15). ASSUMED representation:
    /// in the reference the tag is an element of its own (movable, deletable, typed); ASTERION records only that the room
    /// was tagged. Documents written before the field existed read it as untagged.
    #[serde(default)]
    pub tagged: bool,
}

/// A room separation line: a zero-thickness boundary that closes an open region.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoomSeparation {
    pub id: ElementId,
    pub level: ElementId,
    pub start: Vec2,
    pub end: Vec2,
}

/// OBSERVED default of `Limit Offset` and of the unbounded height: 8 ft.
pub const DEFAULT_ROOM_LIMIT_OFFSET_M: f64 = 2.4384;

// ---------------------------------------------------------------------------- slabs
//
// ONE kind with three ROLES, not three parallel kinds. `Ceiling.Create` and `Floor.Create` have the same signature shape;
// and all three read their quantities through the same three host-object parameter ids. The only real divergences - which
// face the level offset pins, per-segment slope, and the cutoff plane - are FIELDS.

/// Which family of slab this is. ASSUMED to be a field rather than three kinds (section 8a).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlabRole {
    #[default]
    Floor,
    Ceiling,
    Roof,
}

impl SlabRole {
    /// OBSERVED for `Floor`; ASSUMED for the others (section 2a) - a ceiling hangs the other way up, because the level it
    /// is drawn on is the room's ceiling plane.
    pub fn default_anchor(self) -> SlabAnchor {
        match self {
            // OBSERVED: a ceiling hangs UP from level + offset, and so does a roof.
            SlabRole::Ceiling | SlabRole::Roof => SlabAnchor::BottomAtLevel,
            SlabRole::Floor => SlabAnchor::TopAtLevel,
        }
    }

    /// OBSERVED: a ceiling created without an explicit offset sits 2.600 m above its level - the reference's default
    /// `Height Offset From Level` for ceilings - and not on it.
    pub fn default_height_offset(self) -> f64 {
        match self {
            SlabRole::Ceiling => 2.6,
            SlabRole::Floor | SlabRole::Roof => 0.0,
        }
    }

    /// ASSUMED `Perpendicular` for `Roof`, `Vertical` otherwise (section 3b measures it). The two
    /// agree exactly for a flat slab, so no round-6 field can move either way.
    pub fn default_thickness_axis(self) -> ThicknessAxis {
        match self {
            SlabRole::Roof => ThicknessAxis::Perpendicular,
            _ => ThicknessAxis::Vertical,
        }
    }

    /// A ceiling with no compound structure is legal; a floor or roof needs a thickness
    /// (DOCUMENTED, section 1a).
    pub fn allows_zero_thickness(self) -> bool {
        matches!(self, SlabRole::Ceiling)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SlabRole::Floor => "floor",
            SlabRole::Ceiling => "ceiling",
            SlabRole::Roof => "roof",
        }
    }
}

/// Which face the level offset pins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlabAnchor {
    /// OBSERVED: a floor's TOP sits at `level.elevation + height_offset` and the body hangs downwards.
    #[default]
    TopAtLevel,
    /// ASSUMED (section 2a): a ceiling's BOTTOM sits at the level and the body rises.
    BottomAtLevel,
}

/// How the type's thickness is measured away from the reference plane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThicknessAxis {
    #[default]
    Vertical,
    /// ASSUMED for roofs (section 3b): the two are identical for a flat slab, so this choice is
    /// unobservable until a sloped roof is measured.
    Perpendicular,
}

/// Where a sloped edge's base offset is measured to (section 4b).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoofBearing {
    #[default]
    Rafter,
    Truss,
}

/// One boundary segment's slope data. Index-aligned with [`Slab::boundary`]: segment `i` runs
/// `boundary[i] -> boundary[i + 1]`, and the last one closes the loop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SlabSegment {
    pub defines_slope: bool,
    /// Rise over run - DOCUMENTED unit (section 1b), the same unit `asterion_geom::slab::Plane`
    /// uses for its gradient, so no conversion happens anywhere.
    pub slope: f64,
    pub offset_from_base: f64,
    /// Only meaningful when the slab's bearing is [`RoofBearing::Truss`].
    pub plate_offset: f64,
}

/// A slab type: floor, ceiling or roof. v0.1 models the single structural thickness (the
/// reference's compound structure is reduced to its total width here).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlabType {
    pub id: ElementId,
    pub name: String,
    pub thickness: f64,
    #[serde(default)]
    pub role: SlabRole,
    #[serde(default)]
    pub thickness_axis: ThicknessAxis,
}

/// A slab sketched as a closed planar boundary on a level.
///
/// The ceiling and roof variations of that rule are [`SlabAnchor`] and [`SlabSegment`], and both are ASSUMED until round
/// 10.
///
/// Every new field defaults to the v0.1 floor behaviour, so an `.asterion` file written before
/// this slice loads as `role = Floor, anchor = TopAtLevel, thickness_axis = Vertical,
/// segments = []` and reproduces round 6 by construction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Slab {
    pub id: ElementId,
    pub type_id: ElementId,
    pub level: ElementId,
    /// Floor/ceiling: the reference's height offset from level. Roof: its base offset from level.
    pub height_offset: f64,
    /// Closed boundary polygon in plan, in order (the closing edge is implicit).
    pub boundary: Vec<Vec2>,
    pub structural: bool,
    #[serde(default)]
    pub role: SlabRole,
    #[serde(default)]
    pub anchor: SlabAnchor,
    /// Defaults true (section 2c).
    #[serde(default = "crate::model::default_true")]
    pub room_bounding: bool,
    /// Empty means flat. When non-empty its length equals `boundary.len()`.
    #[serde(default)]
    pub segments: Vec<SlabSegment>,
    /// `(level, offset)` above which the roof is truncated. Roof only.
    #[serde(default)]
    pub cutoff: Option<(ElementId, f64)>,
    #[serde(default)]
    pub bearing: RoofBearing,
    /// A floor's slope arrow. OBSERVED; `None` for a flat slab and for every ceiling and roof.
    #[serde(default)]
    pub slope_arrow: Option<SlopeArrow>,
    /// Further closed loops beyond [`Slab::boundary`].
    ///
    /// OBSERVED: whether a loop is material or a hole is the PARITY of its nesting depth among all the loops - not "the
    /// first loop is the outer one". A loop inside the boundary is a hole; a loop OUTSIDE it is accepted as a second island
    /// and ADDS its area (24 + 1 = 25). A loop at depth 2 is material again.
    #[serde(default)]
    pub loops: Vec<Vec<Vec2>>,
    /// Arc edges of [`Slab::boundary`], index-aligned with it: entry `i` is the edge
    /// `boundary[i] -> boundary[i + 1]` (the last closes the loop); `Some(p)` makes that edge the
    /// circular arc from its start through `p` to its end, `None` keeps it straight. Empty = every
    /// edge straight (every document written before arcs existed).
    ///
    /// The engine keeps the arc exactly (three points define it) and tessellates only for drawing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boundary_arcs: Vec<Option<Vec2>>,
    /// Arc edges of each of [`Slab::loops`], the same encoding as [`Slab::boundary_arcs`]; an empty
    /// outer vector (or an empty entry) = straight edges.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loop_arcs: Vec<Vec<Option<Vec2>>>,
    /// A floor's or ceiling's slope-defining boundary edge; `None` when no edge defines the slope. A slab may not have both
    /// this and [`Slab::slope_arrow`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slope_edge: Option<SlabSlopeEdge>,
}

/// A slope-defining boundary edge of a floor or a ceiling.
///
/// For a sketch curve flagged slope-defining that is a STRAIGHT line, the slope line passes through the start point of the
/// edge at its (level and offset adjusted) height, with the direction `cos(a) w + sin(a) Z`, `w` = the edge direction x the
/// sketch normal (+Z), or its reverse; the slab's reference plane contains that slope line and the horizontal direction
/// perpendicular to it, i.e. it holds the edge's line at that height and rises at `a` along `w`. The slope unit is the
/// ratio `tan(a)`.
///
/// What the engine takes as INPUT because the reading stops short of it (the importer maps the
/// stored data once these are read):
/// * The overlap test against the loop array is not specified.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlabSlopeEdge {
    /// Index of the defining edge in [`Slab::boundary`] (edge `i` runs `boundary[i] -> boundary[i+1]`).
    pub edge: usize,
    /// Rise over run (`tan(angle)`).
    pub slope: f64,
    /// Height of the edge's line above the slab's reference elevation, metres.
    #[serde(default)]
    pub height: f64,
    /// `false`: the plane rises toward the RIGHT of the edge's travel (edge direction x +Z); `true`: toward its left.
    #[serde(default)]
    pub reversed: bool,
}

impl SlabSlopeEdge {
    /// The default slope of a floor or ceiling sketch edge when it is flagged slope-defining without an angle of its own -
    /// the curve data a new sketch edge receives holds the angle `atan(1/12)`, i.e. the ratio 1/12 (the same for a floor
    /// and a ceiling).
    pub const DEFAULT_SLOPE_RATIO: f64 = 1.0 / 12.0;

    /// Edge `edge` made slope-defining with the default slope, at the slab's reference height, rising to the right of its
    /// travel.
    pub fn with_default_slope(edge: usize) -> Self {
        SlabSlopeEdge { edge, slope: Self::DEFAULT_SLOPE_RATIO, height: 0.0, reversed: false }
    }

    /// Unit horizontal direction along which the plane rises, for an edge running `a -> b`.
    pub fn rise_direction(&self, a: Vec2, b: Vec2) -> Option<Vec2> {
        let d = (b - a).normalized()?;
        // d x Z = (d.y, -d.x): the right of the travel
        let right = Vec2::new(d.y, -d.x);
        Some(if self.reversed { -right } else { right })
    }
}

/// One closed sketch loop of a slab with its arc edges: the input shape of
/// [`crate::ops::create_slab_with_curve_loops`] and the read shape of [`Slab::sketch_loops`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SlabLoop {
    /// The loop's vertices in order; edge `i` runs `points[i] -> points[i + 1]`, the last edge
    /// closes the loop.
    pub points: Vec<Vec2>,
    /// Per edge, `Some(p)` = an arc from the edge's start through `p` to its end. Empty = straight.
    #[serde(default)]
    pub arcs: Vec<Option<Vec2>>,
}

impl SlabLoop {
    /// A loop of straight edges.
    pub fn polygon(points: Vec<Vec2>) -> Self {
        SlabLoop { points, arcs: Vec::new() }
    }

    /// The through point of edge `i`, `None` for a straight edge.
    pub fn arc_of(&self, i: usize) -> Option<Vec2> {
        self.arcs.get(i).copied().flatten()
    }

    pub fn has_arcs(&self) -> bool {
        self.arcs.iter().any(|a| a.is_some())
    }

    /// The analytic curve of edge `i`; `Err(i)` when its arc's three points are collinear or
    /// coincide (no circle passes through them).
    pub fn edge_curve(&self, i: usize) -> Result<asterion_geom::Curve, usize> {
        let n = self.points.len();
        let (a, b) = (self.points[i], self.points[(i + 1) % n]);
        match self.arc_of(i) {
            None => Ok(asterion_geom::Curve::line(a, b)),
            Some(t) => asterion_geom::Arc::from_three_points(a, t, b, ARC_COLLINEAR_TOL_M)
                .map(asterion_geom::Curve::Arc)
                .map_err(|_| i),
        }
    }

    /// Every edge as an analytic curve, in order; `Err(i)` names the first degenerate arc.
    pub fn curves(&self) -> Result<Vec<asterion_geom::Curve>, usize> {
        (0..self.points.len()).map(|i| self.edge_curve(i)).collect()
    }

    /// Smallest number of points a loop can close with: three for straight edges, two when an arc
    /// joins them (two half circles, or an arc and its chord).
    pub fn min_points(&self) -> usize {
        if self.has_arcs() {
            2
        } else {
            3
        }
    }

    /// The loop as a polygon for drawing and hit-testing: straight edges as they are, every arc
    /// replaced by its display sample points, uniform in the angle ([`asterion_geom::stokes::arc_samples_m`]). Quantities
    /// never come from this.
    pub fn outline(&self) -> Vec<Vec2> {
        let tols = asterion_geom::stokes::TessTols::display_default();
        let mut out = Vec::with_capacity(self.points.len());
        for i in 0..self.points.len() {
            match self.edge_curve(i) {
                Ok(asterion_geom::Curve::Arc(a)) => match asterion_geom::stokes::arc_samples_m(&a, &tols) {
                    Ok(pts) => out.extend(pts[..pts.len() - 1].iter().copied()),
                    Err(_) => out.push(self.points[i]),
                },
                _ => out.push(self.points[i]),
            }
        }
        out
    }

    /// The loop translated by `delta` (vertices and through points).
    pub fn translated(&self, delta: Vec2) -> Self {
        SlabLoop {
            points: self.points.iter().map(|p| *p + delta).collect(),
            arcs: self.arcs.iter().map(|a| a.map(|p| p + delta)).collect(),
        }
    }
}

/// An arc edge whose through point lies within this distance (metres) of the chord's line has no circle: ASTERION's own
/// input rule (the reference stores arcs as centre and radius, and never meets the three-point case).
pub const ARC_COLLINEAR_TOL_M: f64 = 1e-9;

/// A sloped floor's slope arrow: the plane passes through `level + offset` at `tail` and rises
/// toward `head` at `slope`.
///
/// OBSERVED: `slope` is a RISE/RUN RATIO although the reference documents it as "the slope angle" - a 6 m run at 0.08314123
/// rises 0.498847 - and the thickness is PERPENDICULAR to the inclined faces while the end faces stay vertical.
///
/// The arrow's line starts at the tail at the START HEIGHT and either passes through the head at the END HEIGHT (the arrow
/// is given by heights) or rises at the stored angle (the arrow is given by a slope). [`SlopeArrow::tail_height`] is the
/// start height (`None`: the slab's own base height, which is what an arrow made by `create_sloped_slab` has);
/// [`SlopeArrow::head_height`] present means the arrow is given by heights and `slope` is then not used.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlopeArrow {
    pub tail: Vec2,
    pub head: Vec2,
    /// Rise over run when the arrow is given by a slope (`tan(angle)`).
    pub slope: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail_height: Option<ArrowHeight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_height: Option<ArrowHeight>,
}

/// One end height of a slope arrow: the elevation of `level` when it resolves to a level of the model, otherwise the slab's
/// base height (level elevation + height offset), plus `offset`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArrowHeight {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<ElementId>,
    #[serde(default)]
    pub offset: f64,
}

impl SlopeArrow {
    /// An arrow given by a slope, its line starting at the slab's base height.
    pub fn with_slope(tail: Vec2, head: Vec2, slope: f64) -> SlopeArrow {
        SlopeArrow { tail, head, slope, tail_height: None, head_height: None }
    }

    /// The height of one end - the elevation of its level when the level resolves, otherwise `base` - plus its offset; no
    /// height spec at all is `base`.
    pub fn end_height(model: &BimModel, h: Option<ArrowHeight>, base: f64) -> f64 {
        match h {
            None => base,
            Some(h) => h.level.and_then(|l| model.levels.get(&l)).map_or(base, |l| l.elevation) + h.offset,
        }
    }

    /// The start height and the rise over run of the arrow's line, for a slab whose base height is `base`. By heights:
    /// `tan(atan2(end - start, plan length))` (0 for an arrow shorter than 1e-9); by slope: the stored ratio.
    pub fn line(&self, model: &BimModel, base: f64) -> (f64, f64) {
        let start = Self::end_height(model, self.tail_height, base);
        let ratio = match self.head_height {
            Some(_) => Self::slope_from_heights(self.tail, self.head, start, Self::end_height(model, self.head_height, base)),
            None => self.slope,
        };
        (start, ratio)
    }

    /// Slope units are the RATIO `tan(angle)`.
    pub fn units_from_angle(angle: f64) -> f64 {
        angle.tan()
    }

    /// `angle = atan(units)`.
    pub fn angle_from_units(units: f64) -> f64 {
        units.atan()
    }

    /// An arrow given by tail and head HEIGHTS has the angle `atan2(head_height - tail_height, arrow_length)` and 0 for an
    /// arrow shorter than 1e-9; its slope in ratio units is `tan(angle)`. The plane is then taken through the tail at the
    /// TAIL HEIGHT (the caller applies it as the base height of the slab "start height").
    pub fn slope_from_heights(tail: Vec2, head: Vec2, tail_height: f64, head_height: f64) -> f64 {
        let len = (head - tail).length();
        if len < 1e-9 {
            return 0.0;
        }
        Self::units_from_angle((head_height - tail_height).atan2(len))
    }
}

pub(crate) fn default_true() -> bool {
    true
}

impl Slab {
    /// The slab's slope, as the common value of its slope-defining segments.
    ///
    /// ASSUMED (section 8e): `None` when nothing defines a slope, and `None` again when the
    /// definers disagree - a multi-plane roof has no single reported slope. Returning the first
    /// value instead would be a guess that round 10 could not distinguish from a measurement.
    pub fn reported_slope(&self) -> Option<f64> {
        let mut found: Option<f64> = None;
        for s in self.segments.iter().filter(|s| s.defines_slope) {
            match found {
                None => found = Some(s.slope),
                Some(v) if (v - s.slope).abs() <= 1e-12 => {}
                Some(_) => return None,
            }
        }
        found
    }

    /// True when exactly one segment defines a non-zero slope, i.e. the single-plane shed roof of
    /// section 2b - the only sloped case this slice can regenerate.
    pub fn is_single_plane_slope(&self) -> bool {
        self.segments.iter().filter(|s| s.defines_slope && s.slope != 0.0).count() == 1
    }

    /// The outer boundary as a [`SlabLoop`] with its arcs.
    pub fn boundary_loop(&self) -> SlabLoop {
        SlabLoop { points: self.boundary.clone(), arcs: self.boundary_arcs.clone() }
    }

    /// The further loops as [`SlabLoop`]s with their arcs.
    pub fn extra_loops(&self) -> Vec<SlabLoop> {
        self.loops
            .iter()
            .enumerate()
            .map(|(k, l)| SlabLoop { points: l.clone(), arcs: self.loop_arcs.get(k).cloned().unwrap_or_default() })
            .collect()
    }

    /// Every sketch loop in stored order: the boundary first, then [`Slab::loops`].
    pub fn sketch_loops(&self) -> Vec<SlabLoop> {
        let mut v = vec![self.boundary_loop()];
        v.extend(self.extra_loops());
        v
    }

    /// True when any edge of any loop is an arc.
    pub fn has_arcs(&self) -> bool {
        self.boundary_arcs.iter().any(|a| a.is_some()) || self.loop_arcs.iter().flatten().any(|a| a.is_some())
    }

    /// The outer boundary as a polygon for drawing and hit-testing ([`SlabLoop::outline`]); the
    /// stored vertices themselves when every edge is straight.
    pub fn outline_polygon(&self) -> Vec<Vec2> {
        if self.boundary_arcs.iter().any(|a| a.is_some()) {
            self.boundary_loop().outline()
        } else {
            self.boundary.clone()
        }
    }
}

/// The underlying type is [`SlabType`].
pub type FloorType = SlabType;
/// Retained for the same reason. The underlying type is [`Slab`].
pub type Floor = Slab;

// ---------------------------------------------------------------------------- views
//
// Views are first-class model elements, not windows: the engine derives each view's output from the model and the shell
// merely renders it. Evidence level of every rule below is DOCUMENTED (public API of the reference product semantics) until
// round 7 of the measurement harness lands; the derived-geometry classification in `crate::views` is explicitly ASSUMED.

/// View family. DOCUMENTED: mirrors the reference's `ViewFamily` values that this slice supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    FloorPlan,
    CeilingPlan,
    ThreeD,
    Section,
    Elevation,
    Drafting,
    Schedule,
    Sheet,
}

impl ViewKind {
    pub const ALL: [ViewKind; 8] = [
        ViewKind::FloorPlan,
        ViewKind::CeilingPlan,
        ViewKind::ThreeD,
        ViewKind::Section,
        ViewKind::Elevation,
        ViewKind::Drafting,
        ViewKind::Schedule,
        ViewKind::Sheet,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ViewKind::FloorPlan => "floor_plan",
            ViewKind::CeilingPlan => "ceiling_plan",
            ViewKind::ThreeD => "three_d",
            ViewKind::Section => "section",
            ViewKind::Elevation => "elevation",
            ViewKind::Drafting => "drafting",
            ViewKind::Schedule => "schedule",
            ViewKind::Sheet => "sheet",
        }
    }

    pub fn from_name(s: &str) -> Option<ViewKind> {
        ViewKind::ALL.into_iter().find(|k| k.name() == s)
    }

    /// DOCUMENTED: only plan views carry a generating level and a plan view range.
    pub fn is_plan(self) -> bool {
        matches!(self, ViewKind::FloorPlan | ViewKind::CeilingPlan)
    }

    /// DOCUMENTED: schedules and sheets have no model geometry of their own.
    pub fn is_graphical(self) -> bool {
        !matches!(self, ViewKind::Schedule | ViewKind::Sheet)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetailLevel {
    Coarse,
    Medium,
    Fine,
}

impl DetailLevel {
    pub fn name(self) -> &'static str {
        match self {
            DetailLevel::Coarse => "coarse",
            DetailLevel::Medium => "medium",
            DetailLevel::Fine => "fine",
        }
    }

    pub fn from_name(s: &str) -> Option<DetailLevel> {
        [DetailLevel::Coarse, DetailLevel::Medium, DetailLevel::Fine].into_iter().find(|d| d.name() == s)
    }
}

/// The model display style of a view (the entries of the View Control Bar's visual style menu that the
/// engine draws: Textures and Realistic need materials and a render pipeline it does not have).
///
/// DOCUMENTED: the reference's `DisplayStyle` enumeration. The DEFAULT per view kind is ASSUMED: hidden line for plans,
/// shaded for a 3D view the engine creates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisualStyle {
    Wireframe,
    #[default]
    HiddenLine,
    Shaded,
    ConsistentColors,
}

impl VisualStyle {
    pub const ALL: [VisualStyle; 4] =
        [VisualStyle::Wireframe, VisualStyle::HiddenLine, VisualStyle::Shaded, VisualStyle::ConsistentColors];

    pub fn name(self) -> &'static str {
        match self {
            VisualStyle::Wireframe => "wireframe",
            VisualStyle::HiddenLine => "hidden_line",
            VisualStyle::Shaded => "shaded",
            VisualStyle::ConsistentColors => "consistent_colors",
        }
    }

    pub fn from_name(s: &str) -> Option<VisualStyle> {
        VisualStyle::ALL.into_iter().find(|v| v.name() == s)
    }
}

/// The four planes of a plan view range. DOCUMENTED: `PlanViewPlane`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanViewPlane {
    TopClip,
    Cut,
    BottomClip,
    ViewDepth,
    /// OBSERVED: the reference reports a **fifth** plane, the underlay bottom, with offset 0 and **no level**. ASTERION
    /// ignored it silently until round 7 surfaced it, which is why it is listed apart from [`PlanViewPlane::PRIMARY`]: it
    /// takes no part in the ordering rule and no part in classifying an element.
    UnderlayBottom,
}

impl PlanViewPlane {
    /// Every plane the reference reports, in report order.
    pub const ALL: [PlanViewPlane; 5] = [
        PlanViewPlane::TopClip,
        PlanViewPlane::Cut,
        PlanViewPlane::BottomClip,
        PlanViewPlane::ViewDepth,
        PlanViewPlane::UnderlayBottom,
    ];

    /// The four planes that actually bound a plan view. The underlay bottom is deliberately
    /// excluded: it belongs to the underlay feature, not to the view range's ordering rule.
    pub const PRIMARY: [PlanViewPlane; 4] =
        [PlanViewPlane::TopClip, PlanViewPlane::Cut, PlanViewPlane::BottomClip, PlanViewPlane::ViewDepth];

    pub fn name(self) -> &'static str {
        match self {
            PlanViewPlane::TopClip => "top_clip",
            PlanViewPlane::Cut => "cut",
            PlanViewPlane::BottomClip => "bottom_clip",
            PlanViewPlane::ViewDepth => "view_depth",
            PlanViewPlane::UnderlayBottom => "underlay_bottom",
        }
    }

    pub fn from_name(s: &str) -> Option<PlanViewPlane> {
        PlanViewPlane::ALL.into_iter().find(|p| p.name() == s)
    }
}

/// What a range plane is measured from. DOCUMENTED: the reference stores a level id per plane and reserves special ids for
/// "level above", "level below" and "unlimited".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeBase {
    /// The view's own generating level.
    AssociatedLevel,
    LevelAbove,
    LevelBelow,
    /// Only meaningful for the top clip and the view depth plane.
    Unlimited,
}

impl RangeBase {
    pub const ALL: [RangeBase; 4] =
        [RangeBase::AssociatedLevel, RangeBase::LevelAbove, RangeBase::LevelBelow, RangeBase::Unlimited];

    pub fn name(self) -> &'static str {
        match self {
            RangeBase::AssociatedLevel => "associated_level",
            RangeBase::LevelAbove => "level_above",
            RangeBase::LevelBelow => "level_below",
            RangeBase::Unlimited => "unlimited",
        }
    }

    pub fn from_name(s: &str) -> Option<RangeBase> {
        RangeBase::ALL.into_iter().find(|b| b.name() == s)
    }
}

/// One plane of the range: a base plus a signed offset in metres.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RangePlane {
    pub base: RangeBase,
    pub offset: f64,
}

impl RangePlane {
    pub fn new(base: RangeBase, offset: f64) -> Self {
        RangePlane { base, offset }
    }

    /// A plane measured from the view's own level.
    pub fn at(offset: f64) -> Self {
        RangePlane { base: RangeBase::AssociatedLevel, offset }
    }

    /// OBSERVED: the underlay bottom is reported with offset 0 and a null level.
    pub fn underlay_bottom_default() -> Self {
        RangePlane { base: RangeBase::Unlimited, offset: 0.0 }
    }
}

/// A plan view range: the primary range (top / cut / bottom) plus the view depth.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewRange {
    pub top: RangePlane,
    pub cut: RangePlane,
    pub bottom: RangePlane,
    pub depth: RangePlane,
    /// OBSERVED: reported with offset 0 and **no level**, which is why its base is [`RangeBase::Unlimited`] rather than the
    /// associated level - it resolves to no elevation.
    #[serde(default = "RangePlane::underlay_bottom_default")]
    pub underlay_bottom: RangePlane,
}

impl ViewRange {
    /// OBSERVED (measured on two different levels): the metric default floor-plan range is top 2.300 m, cut 1.200 m, bottom
    /// 0 and view depth 0, every one measured from the **associated level**. This was ASSUMED and the measurement confirmed
    /// it exactly.
    ///
    /// Spec V-01: for a floor plan of a non-structural view family the five slots start at the associated level with offset
    /// 0, then cut = (4.0 ft | 1.2 m), top = (7.5 ft | 2.3 m), bottom and view depth stay 0; the pair member is chosen
    /// inside (UNKNOWN (Spec V-01): not specified, so the metric member is the harness's observation). The fifth plane, the
    /// underlay bottom, is not stored in the range (the underlay level lives in the view): the engine reports it as "no
    /// level, offset 0".
    pub const DEFAULT_TOP_M: f64 = 2.3;
    pub const DEFAULT_CUT_M: f64 = 1.2;

    pub fn plane(&self, p: PlanViewPlane) -> RangePlane {
        match p {
            PlanViewPlane::TopClip => self.top,
            PlanViewPlane::Cut => self.cut,
            PlanViewPlane::BottomClip => self.bottom,
            PlanViewPlane::ViewDepth => self.depth,
            PlanViewPlane::UnderlayBottom => self.underlay_bottom,
        }
    }

    pub fn set_plane(&mut self, p: PlanViewPlane, v: RangePlane) {
        match p {
            PlanViewPlane::TopClip => self.top = v,
            PlanViewPlane::Cut => self.cut = v,
            PlanViewPlane::BottomClip => self.bottom = v,
            PlanViewPlane::ViewDepth => self.depth = v,
            PlanViewPlane::UnderlayBottom => self.underlay_bottom = v,
        }
    }
}

impl Default for ViewRange {
    fn default() -> Self {
        ViewRange {
            top: RangePlane::at(ViewRange::DEFAULT_TOP_M),
            cut: RangePlane::at(ViewRange::DEFAULT_CUT_M),
            bottom: RangePlane::at(0.0),
            depth: RangePlane::at(0.0),
            underlay_bottom: RangePlane::underlay_bottom_default(),
        }
    }
}

/// A rectangular crop region in plan coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CropRegion {
    pub min: Vec2,
    pub max: Vec2,
}

/// A view element.
///
/// DOCUMENTED: a view carries its family, a name unique among views, a scale (the denominator of
/// 1:scale), a detail level, an optional crop region with independent "active"/"visible" flags,
/// and - for plan views only - a generating level and a plan view range.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub id: ElementId,
    pub name: String,
    pub kind: ViewKind,
    /// Denominator of the view scale (1:scale). DOCUMENTED default for a metric floor plan: 100.
    pub scale: u32,
    pub detail_level: DetailLevel,
    /// Generating level; `Some` exactly for plan views.
    pub gen_level: Option<ElementId>,
    /// Plan view range; meaningful for plan views only.
    pub range: ViewRange,
    pub crop: Option<CropRegion>,
    pub crop_active: bool,
    pub crop_visible: bool,
    /// The view origin's Z, **frozen at creation**.
    ///
    /// OBSERVED: a plan view's `Origin` does NOT follow its level. Raising the level from 0 to 0.5 left both a floor plan
    /// and a ceiling plan reporting `[0, 0, 0]`, and a view created on a level at elevation 3.0 reports `[0, 0, 3]`. So the
    /// origin is the level's elevation AT THE MOMENT THE VIEW WAS MADE - stored state, not a derivation - while the view
    /// RANGE keeps following the level.
    ///
    /// `None` only for a document written before this was modelled; the reader then falls back to
    /// the level's present elevation, which is what ASTERION used to report.
    #[serde(default)]
    pub origin_z: Option<f64>,
    /// The model display style (see [`VisualStyle`]).
    #[serde(default)]
    pub visual_style: VisualStyle,
    /// The display settings of the Properties palette (`crate::view_display`).
    #[serde(default)]
    pub display: crate::view_display::ViewDisplay,
    /// Elements hidden in THIS view (the reference's "Hide in View > Elements").
    ///
    /// The view keeps its OWN set of the element ids hidden in it (the read path asks the view's own object, never the view
    /// template; membership is by the element's own id; the elements of linked models are kept apart, which the engine does
    /// not model). A component is hidden when its parent / host is in the set - for the 17 categories of walls, curtain
    /// parts, stair runs and landings, rails, cornices, reveals and grids; the engine draws stair runs and rails from their
    /// stair / railing element, so hiding that element hides them. NOT specified: which parent list the family operation
    /// returns for a WALL (stacked-wall members), so a wall hosted by another is not derived; whether applying a view
    /// template copies or merges the set; and whether a hidden element still bounds rooms or cuts. ENGINE CHOICE: the set
    /// is consulted by the view's drawing only (a hidden element keeps every other behaviour), and `set_hidden_in_view`
    /// refuses what is not a model element or a datum.
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub hidden_elements: std::collections::BTreeSet<ElementId>,
    /// The camera of a 3D view; `None` for a view that is not 3D and for a document that stored none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<crate::view_camera::ViewCamera>,
}

// ---------------------------------------------------------------------------- grids
//
// A grid is a **datum**: like a level, it is an annotation-bearing reference plane rather than a solid. DOCUMENTED:
// `Grid.Create(doc, Line)` / `(doc, Arc)`, `Grid.Name` (unique among grids), `Grid.Curve`, per-view extents
// (`DatumExtentType::Model` vs `ViewSpecific`), per-end bubble visibility (`DatumEnds::End0` / `End1`) and 3D vertical
// extents. v0.1 models straight grids with a model extent.

/// Which end of a datum a bubble sits on. DOCUMENTED: `DatumEnds::End0` / `End1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatumEnd {
    End0,
    End1,
}

impl DatumEnd {
    pub const ALL: [DatumEnd; 2] = [DatumEnd::End0, DatumEnd::End1];

    pub fn index(self) -> usize {
        match self {
            DatumEnd::End0 => 0,
            DatumEnd::End1 => 1,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            DatumEnd::End0 => "end0",
            DatumEnd::End1 => "end1",
        }
    }

    pub fn from_name(s: &str) -> Option<DatumEnd> {
        DatumEnd::ALL.into_iter().find(|e| e.name() == s)
    }
}

/// A straight grid line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Grid {
    pub id: ElementId,
    /// Unique among grids. DOCUMENTED: the reference rejects a duplicate grid name.
    pub name: String,
    pub start: Vec2,
    pub end: Vec2,
    /// 3D extent `(bottom, top)` in metres, or `None` for unbounded. DOCUMENTED: a datum carries
    /// vertical extents, which decide the elevations where it appears. ASSUMED: a new grid is
    /// unbounded, i.e. it shows in every plan view until its extents are narrowed.
    pub vertical_extent: Option<(f64, f64)>,
    /// Bubble visibility **per view**, keyed by the view's id.
    ///
    /// OBSERVED: bubble visibility is a per-view override, not a property of the grid. The reference exposes it as /
    /// `HideBubbleInView`, and the measurement proves the state really is per view: after hiding End0 in the lower plan and
    /// showing it again, the lower plan reports End0 **shown** while the upper plan - which was never touched - still
    /// reports it **hidden**, because that is the default.
    ///
    /// A view with no entry here uses [`Grid::DEFAULT_BUBBLE`].
    #[serde(default)]
    pub bubble_in_view: BTreeMap<ElementId, [bool; 2]>,
    /// A point the grid's ARC passes through between `start` and `end`; `None` for a straight grid.
    ///
    /// A grid is curved exactly when its datum surface is a cylinder; cut by a horizontal plane the cylinder gives its base
    /// circle, and the plan arc runs counter-clockwise from the angle of the surface envelope's corner 0 (u min) to that of
    /// corner 2 (u max), with the vertical range v min .. v max. The engine keeps the arc as three points, like an arc
    /// wall.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub through: Option<Vec2>,
}

impl Grid {
    /// The analytic curve of the grid in plan: the arc through `through`, or the segment.
    pub fn curve(&self) -> Option<asterion_geom::Curve> {
        match self.through {
            None => Some(asterion_geom::Curve::line(self.start, self.end)),
            Some(t) => asterion_geom::Arc::from_three_points(self.start, t, self.end, ARC_COLLINEAR_TOL_M)
                .ok()
                .map(asterion_geom::Curve::Arc),
        }
    }

    /// The plan curve as a polyline (the arc by its display samples): what is drawn.
    pub fn polyline(&self) -> Vec<Vec2> {
        match self.curve() {
            Some(asterion_geom::Curve::Arc(a)) => asterion_geom::stokes::arc_samples_m(&a, &asterion_geom::stokes::TessTols::display_default())
                .unwrap_or_else(|_| vec![self.start, self.end]),
            _ => vec![self.start, self.end],
        }
    }

    pub fn is_curved(&self) -> bool {
        self.through.is_some()
    }

    /// OBSERVED (five grids): a new grid shows its bubble at the **second end only**. ASTERION defaulted to `[true, true]`
    /// until the measurement said otherwise.
    ///
    /// Spec G-02: the default lives on the grid TYPE, not the grid (initialises "bubble at end 1" to false and "bubble at
    /// end 2" to true. A view with no override shows the type default).
    pub const DEFAULT_BUBBLE: [bool; 2] = [false, true];

    /// Bubble visibility of this grid as seen in one view.
    pub fn bubble_in(&self, view: ElementId) -> [bool; 2] {
        self.bubble_in_view.get(&view).copied().unwrap_or(Grid::DEFAULT_BUBBLE)
    }

    /// Next name in the reference's sequence.
    ///
    /// OBSERVED: the cursor is the **last name assigned**, not the last grid created. Measured: with grids `1` and `2`,
    /// renaming the first to `A` makes the next automatic name `B` - not `3` - and the one after that `C`. So the
    /// predecessor comes from [`BimModel::last_grid_name`], which `create_grid` *and* `rename_grid` both update.
    ///
    /// The first grid of a project is named `1`. (the string incrementer behind, module [`crate::item_number`]): the
    /// trailing run of characters of one strategy (ASCII letters, digits, other alphabetic) is changed - digits add 1 with
    /// carry ("A101" -> "A102", "A9" -> "A10", "A099" -> "A100", width kept), a run of one repeated letter steps to the
    /// next letter and "Z" -> "AA", "AA" -> "BB" - and a string that cannot be incremented ("AZ") falls back to the base
    /// string (the caller's base is not specified; the engine uses "1").
    pub fn next_name(previous: Option<&str>) -> String {
        crate::item_number::next_item_number(previous)
    }
}

// ------------------------------------------------------------------- openings / inserts
//
// Everything in this section is ASSUMED unless marked otherwise: no number below was observed in the reference product.

/// What kind of thing is inserted into a host object.
///
/// [`InsertKind::RectOpening`] ([`WallOpening`]) and [`InsertKind::HostedInstance`]
/// ([`HostedInstance`]) are both produced in v0.1 - that is the whole point of modelling the
/// relation once and the elements twice (section 6c). [`InsertKind::ShaftOpening`] and
/// [`InsertKind::FaceOpening`] stay **reserved** (section 6c asks for them explicitly so that
/// adding them later is not a breaking change to this taxonomy); no element kind produces them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertKind {
    /// A rectangular opening cut directly in a wall ([`WallOpening`]).
    RectOpening,
    /// A door/window family instance hosted in a wall ([`HostedInstance`]).
    HostedInstance,
    /// Reserved: a shaft opening spanning two levels. NOT implemented in v0.1.
    ShaftOpening,
    /// Reserved: an opening sketched on the face of a beam/brace/column. NOT implemented in v0.1.
    FaceOpening,
}

impl InsertKind {
    pub const ALL: [InsertKind; 4] =
        [InsertKind::RectOpening, InsertKind::HostedInstance, InsertKind::ShaftOpening, InsertKind::FaceOpening];

    pub fn name(self) -> &'static str {
        match self {
            InsertKind::RectOpening => "rect_opening",
            InsertKind::HostedInstance => "hosted_instance",
            InsertKind::ShaftOpening => "shaft_opening",
            InsertKind::FaceOpening => "face_opening",
        }
    }

    pub fn from_name(s: &str) -> Option<InsertKind> {
        InsertKind::ALL.into_iter().find(|k| k.name() == s)
    }

    /// Whether ASTERION v0.1 can actually build an element of this kind.
    pub fn is_implemented(self) -> bool {
        matches!(self, InsertKind::RectOpening | InsertKind::HostedInstance)
    }
}

/// A rectangular opening cut directly in a wall, with no type and no family.
///
/// Coordinates, as specified:
/// * `u_min`/`u_max` - station along the **host's location curve**, in metres from its start. a `WallOpening` carries
///   corners rather than a station in the reference, so turning the two world-space corners into a station pair is
///   ASTERION's own normalisation.
/// * `z_min`/`z_max` - **absolute** elevation in metres, i.e. what `NewOpening` receives. The field annotation wins,
///   because it is the one tied to the reference call and because the two agree for every scenario in (host base at 0.0).
///
/// Both pairs are stored **normalised** (`min <= max`): the reference's contract is "two opposite
/// corners", so corner order carries no information (ASSUMED).
///
/// Derived state - the clipped cut area, the cut prism, and the host's net Area/Volume - is NOT
/// stored here; see [`crate::regen::opening_state`] and [`crate::regen::wall_state`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WallOpening {
    pub id: ElementId,
    /// The host wall. DOCUMENTED: `Opening.Host`.
    pub host: ElementId,
    /// Lower station along the host location curve, metres from its start.
    pub u_min: f64,
    /// Upper station along the host location curve, metres from its start.
    pub u_max: f64,
    /// Lower absolute elevation of the cut, metres.
    pub z_min: f64,
    /// Upper absolute elevation of the cut, metres.
    pub z_max: f64,
}

impl WallOpening {
    /// Width of the cut rectangle along the host location curve (metres).
    pub fn width(&self) -> f64 {
        self.u_max - self.u_min
    }

    /// Height of the cut rectangle (metres).
    pub fn height(&self) -> f64 {
        self.z_max - self.z_min
    }

    /// Area of the cut rectangle **before** clipping to the host outline (m2).
    /// DOCUMENTED (section 6b): the geometry source of an `Opening` is its own boundary.
    pub fn cut_area(&self) -> f64 {
        self.width() * self.height()
    }

    /// This opening's contribution to the host-side insert relation, in host (station, absolute z)
    /// coordinates. DOCUMENTED (section 7b): "cut rectangle of a `WallOpening` - its own stored
    /// corners".
    pub fn rect(&self) -> crate::regen::CutRect {
        crate::regen::CutRect { u_lo: self.u_min, u_hi: self.u_max, z_lo: self.z_min, z_hi: self.z_max }
    }
}

// -------------------------------------------------------------- hosted instances
//
// The second half: doors and windows. no number here was observed in the reference product.
//
// Section 6c's recommendation is "one relation, two element classes": the host-side cut relation
// is modelled once ([`crate::regen::Insert`]) and the elements twice ([`WallOpening`] and
// [`HostedInstance`]), because their *stored* state has almost nothing in common (section 6b)
// while their effect on the host is identical.

/// Door versus window.
///
/// DOCUMENTED: the reference carries two disjoint built-in parameter sets (`DOOR_WIDTH`/`DOOR_HEIGHT`/`DOOR_THICKNESS`
/// against `WINDOW_WIDTH`/`WINDOW_HEIGHT`/`WINDOW_THICKNESS`/`WINDOW_INSET`) and two separate family templates (`Metric
/// Door.rft`, `Metric Window.rft`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostedKind {
    Door,
    Window,
}

impl HostedKind {
    pub const ALL: [HostedKind; 2] = [HostedKind::Door, HostedKind::Window];

    pub fn name(self) -> &'static str {
        match self {
            HostedKind::Door => "door",
            HostedKind::Window => "window",
        }
    }

    pub fn from_name(s: &str) -> Option<HostedKind> {
        HostedKind::ALL.into_iter().find(|k| k.name() == s)
    }

    /// Whether this kind carries an inset into the host.
    ///
    /// DOCUMENTED (section 3b): `WINDOW_INSET` / `FAMILY_WINDOW_INSET_PARAM` is "the inset of the
    /// window into the wall" and exists for windows only. ASSUMED (section 1c): the door template
    /// has none, so a door type's inset is pinned to 0 rather than rejected.
    pub fn has_inset(self) -> bool {
        matches!(self, HostedKind::Window)
    }

    /// The sill height an instance of this kind gets at placement, given its type's default.
    ///
    /// DOCUMENTED (section 3c): on a window the type's *Default Sill Height* gives the initial
    /// instance *Sill Height*. ASSUMED (section 3c): "a door placed from a template family
    /// defaults to Sill Height = 0", so a door ignores the type value entirely.
    pub fn initial_sill_height(self, type_default: f64) -> f64 {
        match self {
            HostedKind::Door => 0.0,
            HostedKind::Window => type_default,
        }
    }
}

/// A loadable-family **type** for a door or a window, reduced to the fields that drive geometry.
///
/// Modelled exactly like [`WallType`] / [`FloorType`]: a flat record in [`BimModel`], SI units, no
/// derived state. Section 7a names every field; section 3b is the parameter evidence.
///
/// Field-by-field evidence (all lengths in **metres**):
/// * `width` - DOCUMENTED (sections 2a, 3b): `DOOR_WIDTH` / `WINDOW_WIDTH` (or the generic
///   `FAMILY_WIDTH_PARAM`; *which* one a template-built family exposes is OPEN, section 3b) drives
///   the **opening cut**. INFERRED (section 2a): for an out-of-the-box door/window the host is cut
///   by the family's own `Opening` element, sized by `Width`/`Height`.
/// * `height` - DOCUMENTED: `WINDOW_HEIGHT` is "the height of the opening of the window".
/// * `thickness` - DOCUMENTED: `DOOR_THICKNESS` / `WINDOW_THICKNESS`. Leaf/frame thickness; it has
///   **no cut effect** in v0.1 (section 7a), because ASTERION models no leaf, frame or glass.
/// * `rough_width` / `rough_height` - DOCUMENTED (section 2a): `FAMILY_ROUGH_WIDTH_PARAM` / `FAMILY_ROUGH_HEIGHT_PARAM` are
///   documented only as values that "can be scheduled or exported", with no stated geometric effect. ASSUMED, and the
///   falsifiable prediction that scenario step `rough-opening-changed-area-must-not-move` exists to break: changing either
///   **never** moves the host's Area.
/// * `default_sill_height` - INFERRED (section 3b, from an exhaustive grep of all 3 583 `BuiltInParameter` ids): there is
///   **no** built-in named "Default Sill Height", yet the vendor product help documents a window type property of that
///   name, so it is an ordinary family parameter. ASTERION carries it as a field of its own type record, as section 3b
///   instructs.
/// * `inset` - DOCUMENTED (section 3b): "the inset of the window into the wall"; windows only.
///   ASSUMED (section 7f): v0.1 **records** it and does not let it move the cut, because the
///   through-thickness placement of the cut is out of scope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostedType {
    pub id: ElementId,
    /// Unique among hosted types. ASSUMED (section 7a: "name unique among hosted types").
    pub name: String,
    pub kind: HostedKind,
    pub width: f64,
    pub height: f64,
    pub thickness: f64,
    pub rough_width: f64,
    pub rough_height: f64,
    pub default_sill_height: f64,
    pub inset: f64,
}

/// The type parameters a scenario may set, in **ASTERION's own vocabulary**.
///
/// DOCUMENTED (section 8): the `set_hosted_type_param` op contract names exactly these seven keys
/// and maps each to the reference's built-ins inside the harness - "the same discipline already
/// used for `location_line` and layer `function`". They are deliberately not copies of
/// reference-product identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostedTypeParam {
    Width,
    Height,
    Thickness,
    RoughWidth,
    RoughHeight,
    Inset,
    DefaultSillHeight,
}

impl HostedTypeParam {
    pub const ALL: [HostedTypeParam; 7] = [
        HostedTypeParam::Width,
        HostedTypeParam::Height,
        HostedTypeParam::Thickness,
        HostedTypeParam::RoughWidth,
        HostedTypeParam::RoughHeight,
        HostedTypeParam::Inset,
        HostedTypeParam::DefaultSillHeight,
    ];

    pub fn name(self) -> &'static str {
        match self {
            HostedTypeParam::Width => "width",
            HostedTypeParam::Height => "height",
            HostedTypeParam::Thickness => "thickness",
            HostedTypeParam::RoughWidth => "rough_width",
            HostedTypeParam::RoughHeight => "rough_height",
            HostedTypeParam::Inset => "inset",
            HostedTypeParam::DefaultSillHeight => "default_sill_height",
        }
    }

    pub fn from_name(s: &str) -> Option<HostedTypeParam> {
        HostedTypeParam::ALL.into_iter().find(|p| p.name() == s)
    }

    /// Whether writing this parameter changes the **cut**, and therefore every host that carries
    /// an instance of the type. ASSUMED (sections 2a, 7c): only `Width` and `Height` do - which is
    /// the same prediction that makes `rough_width` a reporting-only value.
    pub fn moves_the_cut(self) -> bool {
        matches!(self, HostedTypeParam::Width | HostedTypeParam::Height)
    }

    /// Whether this parameter must be strictly positive. ASSUMED (section 5): only the two that
    /// drive the cut; a rough dimension or an inset of 0 is a legitimate "not authored" value.
    pub fn must_be_positive(self) -> bool {
        self.moves_the_cut()
    }
}

/// The instance parameters a scenario may address, in ASTERION's own vocabulary.
///
/// DOCUMENTED (section 8): the `set_hosted_instance_param` op contract names
/// `{sill_height, head_height, mark}`. [`HostedInstanceParam::Level`] and
/// [`HostedInstanceParam::HostId`] are added because section 3a lists both as instance parameters
/// and section 7d routes a write to either through `param.read_only`; they exist so that the
/// rejection is reachable and testable rather than merely asserted in prose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostedInstanceParam {
    SillHeight,
    HeadHeight,
    Mark,
    Level,
    HostId,
}

impl HostedInstanceParam {
    pub const ALL: [HostedInstanceParam; 5] = [
        HostedInstanceParam::SillHeight,
        HostedInstanceParam::HeadHeight,
        HostedInstanceParam::Mark,
        HostedInstanceParam::Level,
        HostedInstanceParam::HostId,
    ];

    pub fn name(self) -> &'static str {
        match self {
            HostedInstanceParam::SillHeight => "sill_height",
            HostedInstanceParam::HeadHeight => "head_height",
            HostedInstanceParam::Mark => "mark",
            HostedInstanceParam::Level => "level",
            HostedInstanceParam::HostId => "host_id",
        }
    }

    pub fn from_name(s: &str) -> Option<HostedInstanceParam> {
        HostedInstanceParam::ALL.into_iter().find(|p| p.name() == s)
    }

    /// ASSUMED (sections 3a, 3c, 7d): `Sill Height` and `Head Height` are both **writable** (the
    /// always-read-only set is the *computed* quantities, not these) and `Mark` is writable, while
    /// `Level` and `Host Id` are read-only after placement. Section 7e compares the three
    /// `*_read_only` flags, which is what makes this a falsifiable commitment and not a
    /// convenience.
    pub fn read_only(self) -> bool {
        matches!(self, HostedInstanceParam::Level | HostedInstanceParam::HostId)
    }
}

/// A door or window **instance** hosted in a wall.
///
/// Section 7a names every stored field. Nothing derived lives here: `head_height`, `mirrored`, the
/// cut rectangle, the facing/hand vectors and the host's net quantities are all pure functions of
/// this record plus its type, host and level - see [`crate::regen::hosted_instance_state`].
///
/// Field-by-field evidence:
/// * In this case the host property would return the wall in which the window is contained." ASSUMED: v0.1 hosts only in a
///   [`Wall`].
/// * `level` - DOCUMENTED (section 1b): the creation overload ASTERION mirrors is the one that
///   takes point + symbol + host + level.
/// * `host_station` - INFERRED (section 1c), "the parameter value of the insertion point of the instance along the wall's
///   location curve". whether the reference reports it as a raw curve parameter in feet or a normalized `[0,1]` value is
///   OPEN (section 1c), which is why ASTERION stores the **normalised** form - metres along the location curve from its
///   start - and the replayer/harness converts at the boundary.
/// * `sill_height` - DOCUMENTED (section 3a): `INSTANCE_SILL_HEIGHT_PARAM`, "the height of the
///   sill in relation to the level on which this instance is placed", and changing it does not
///   resize the instance. The **head** height is not stored: ASSUMED (section 3c) that
///   `Head Height = Sill Height + type.height` is one relationship seen from two sides, so storing
///   both would let the two disagree.
/// * `facing_flipped` / `hand_flipped` - DOCUMENTED (section 1d): `FacingFlipped` / `HandFlipped` are two independent
///   flags, toggled through. `mirrored` is **never stored**: it is DOCUMENTED to mean "only one axis is flipped", so it is
///   derived as `facing_flipped ^ hand_flipped` (section 1d prediction 3).
/// * `mark` - DOCUMENTED (section 3a): `ALL_MODEL_MARK`, "identifies or enumerates a particular
///   instance as specified by the user". The auto-incrementing sequence is explicitly out of v0.1
///   scope (section 7f), so a placed instance starts with an empty mark.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostedInstance {
    pub id: ElementId,
    pub type_id: ElementId,
    /// The host wall.
    pub host: ElementId,
    pub level: ElementId,
    /// Station along the host's location curve, in metres from its start.
    pub host_station: f64,
    /// Metres above `level`.
    pub sill_height: f64,
    pub facing_flipped: bool,
    pub hand_flipped: bool,
    pub mark: String,
    /// This instance's own width, replacing its type's. `None` is the type's. Additive: documents without it read as
    /// before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_override: Option<f64>,
    /// This instance's own height (see [`HostedInstance::width_override`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_override: Option<f64>,
    /// This instance's own cut loop, relative to its station and sill, replacing the rectangle its type's (or its own)
    /// width x height gives. ENGINE CHOICE (: how the reference product stores a family's cut loop for an instance and how
    /// the host takes it is not specified yet). `None` is the width x height rectangle centred on the station.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut_loop: Option<InstanceCutLoop>,
}

/// A rectangular cut loop of one hosted instance, in the instance's own frame: metres along the host from the instance's
/// station metres above its sill.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstanceCutLoop {
    pub u_min: f64,
    pub u_max: f64,
    pub z_min: f64,
    pub z_max: f64,
}

impl HostedInstance {
    /// The width of this instance's cut: its own override, else its type's.
    pub fn width_in(&self, t: &HostedType) -> f64 {
        self.width_override.unwrap_or(t.width)
    }

    /// The height of this instance's cut: its own override, else its type's.
    pub fn height_in(&self, t: &HostedType) -> f64 {
        self.height_override.unwrap_or(t.height)
    }

    /// ASSUMED (section 7b): "`can_flip_facing` / `can_flip_hand` - true for both kinds in v0.1".
    /// DOCUMENTED (section 1d): the reference exposes `CanFlipFacing` / `CanFlipHand`, and a
    /// refused flip is "a no-op with a false result", not a transaction failure - which is why
    /// [`crate::ops::flip_facing`] returns a bool and never reports.
    pub const CAN_FLIP_FACING: bool = true;
    pub const CAN_FLIP_HAND: bool = true;

    /// Section 7e compares this as `host_face_null`.
    pub const HOST_FACE_NULL: bool = true;

    /// `mirrored` - DERIVED, never stored.
    ///
    /// ASSUMED (section 1d prediction 3): `Mirrored == FacingFlipped XOR HandFlipped`, which is
    /// literally what the reference's own documentation - "only one axis is flipped" - says.
    /// Flipping **both** axes is a 180 degree rotation and is therefore *not* mirrored; that is
    /// the falsifiable half of the prediction.
    pub fn mirrored(&self) -> bool {
        self.facing_flipped ^ self.hand_flipped
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BimModel {
    pub levels: asterion_core::IdMap<Level>,
    pub wall_types: asterion_core::IdMap<WallType>,
    pub walls: asterion_core::IdMap<Wall>,
    #[serde(default)]
    pub floor_types: asterion_core::IdMap<FloorType>,
    #[serde(default)]
    pub floors: asterion_core::IdMap<Floor>,
    #[serde(default)]
    pub views: asterion_core::IdMap<View>,
    #[serde(default)]
    pub grids: asterion_core::IdMap<Grid>,
    /// Rectangular wall openings. `#[serde(default)]` keeps documents written before this slice loadable, as for `floors`,
    /// `views` and `grids`.
    #[serde(default)]
    pub wall_openings: asterion_core::IdMap<WallOpening>,
    /// Door / window **types**.
    #[serde(default)]
    pub hosted_types: asterion_core::IdMap<HostedType>,
    /// Door / window **instances** hosted in a wall.
    #[serde(default)]
    pub hosted_instances: asterion_core::IdMap<HostedInstance>,
    /// The last grid name **assigned**, which is the cursor the automatic name sequence continues from (OBSERVED).
    ///
    /// Genuinely stored state, not derived: renaming a grid moves the cursor, so it cannot be
    /// recovered from the set of grids that exist. It is the one place in the BIM model where that
    /// is true, and it is deliberate - the reference keeps the same state.
    #[serde(default)]
    pub last_grid_name: Option<String>,
    #[serde(default)]
    pub geometry_joins: Vec<GeometryJoin>,
    /// Architectural column types and columns (`crate::columns`, observed in round 21).
    #[serde(default, skip_serializing_if = "asterion_core::IdMap::is_empty")]
    pub column_types: asterion_core::IdMap<crate::columns::ColumnType>,
    #[serde(default, skip_serializing_if = "asterion_core::IdMap::is_empty")]
    pub columns: asterion_core::IdMap<crate::columns::Column>,
    /// Curtain walls (`crate::curtains`, observed in rounds 22 and 22b): the curtain part of a wall TYPE
    /// (keyed by the wall type's id), the grid of a wall (keyed by the wall's id) and the mullion types.
    #[serde(default, skip_serializing_if = "asterion_core::IdMap::is_empty")]
    pub curtain_types: asterion_core::IdMap<crate::curtains::CurtainType>,
    #[serde(default, skip_serializing_if = "asterion_core::IdMap::is_empty")]
    pub curtain_grids: asterion_core::IdMap<crate::curtains::CurtainGridInstance>,
    #[serde(default, skip_serializing_if = "asterion_core::IdMap::is_empty")]
    pub mullion_types: asterion_core::IdMap<crate::curtains::MullionType>,
    /// The elements that are pinned (`Element.Pinned`): they may not be moved, rotated or mirrored,
    /// and deleting one posts a warning. Stored state of the element (a deleted element leaves the
    /// set). ENGINE CHOICE until the harness round 19 measures it: see `ops::set_pinned`.
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub pinned: std::collections::BTreeSet<ElementId>,
    /// What the user did to the IMPLICIT join of two walls that cross in an X: unjoined it, or switched its order. A
    /// crossing pair is joined by default (OBSERVED: `JoinGeometry` on the pair refuses with "already joined"), so only the
    /// departures from the default are stored. See `crate::crossings`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cross_overrides: Vec<CrossOverride>,
    /// The join type the user set at a wall end (the reference product's per-end `LocationCurve.JoinType`); an end that is
    /// not listed has the default, [`WallJoinKind::Abut`]. See [`WallJoinTypeAtEnd`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wall_join_types: Vec<WallJoinTypeAtEnd>,
    /// Shafts and the openings cut in floors, ceilings and roofs (`crate::shafts`, rounds 37 / 40).
    #[serde(default, skip_serializing_if = "asterion_core::IdMap::is_empty")]
    pub shafts: asterion_core::IdMap<crate::shafts::Shaft>,
    #[serde(default, skip_serializing_if = "asterion_core::IdMap::is_empty")]
    pub slab_openings: asterion_core::IdMap<crate::shafts::SlabOpening>,
    /// The profile sketches of walls, keyed by the wall (`crate::wall_profiles`).
    #[serde(default, skip_serializing_if = "asterion_core::IdMap::is_empty")]
    pub wall_profiles: asterion_core::IdMap<crate::wall_profiles::WallProfile>,
    /// The rafter cut of the roofs that do not have the default plumb cut (`crate::rafter_cut`, rounds 34 / 34b).
    #[serde(default, skip_serializing_if = "asterion_core::IdMap::is_empty")]
    pub rafter_cuts: asterion_core::IdMap<crate::rafter_cut::RafterCut>,
    /// The stacked wall types (`crate::stacked`); each is also a wall type of `wall_types` with the width of its members.
    #[serde(default, skip_serializing_if = "asterion_core::IdMap::is_empty")]
    pub stacked_types: asterion_core::IdMap<crate::stacked::StackedWallType>,
    /// The shape editing of floors and roofs, keyed by the slab (`crate::slab_shape`).
    #[serde(default, skip_serializing_if = "asterion_core::IdMap::is_empty")]
    pub slab_shapes: asterion_core::IdMap<crate::slab_shape::SlabShape>,
    /// The variable layer of a slab type (index of the layer of its compound structure; OBSERVED), by type.
    #[serde(default, skip_serializing_if = "asterion_core::IdMap::is_empty")]
    pub slab_variable_layers: asterion_core::IdMap<usize>,
    /// Rooms.
    #[serde(default)]
    pub rooms: asterion_core::IdMap<Room>,
    #[serde(default)]
    pub room_separations: asterion_core::IdMap<RoomSeparation>,
    /// The project-wide "Volume Computations" switch, stored INVERTED so that the `Default` value (false) is the
    /// reference's default. OBSERVED: computation is ON out of the box in this template, `Volume` is 53.7423 with no
    /// action, and switching it OFF gives 0 while `Area` is untouched.
    #[serde(default)]
    pub volume_computation_off: bool,
    /// The last room NUMBER assigned: the cursor the next number continues from.
    ///
    /// OBSERVED: renaming a room 101 and deleting it makes the next one 102, not 1 - the number follows the last one
    /// assigned, not the highest that exists. The same shape as the grid name cursor, and genuinely stored state for the
    /// same reason.
    #[serde(default)]
    pub last_room_number: Option<u64>,
    /// Stair-by-run elements: types, stairs, runs, landings, railings. One field, owned by [`crate::stairs`], so that slice
    /// evolves in its own file.
    #[serde(default)]
    pub stairs: crate::stairs::StairsModel,
    /// What the component workflow adds around [`crate::stairs`]: run types, railing types, run justification and style,
    /// landing boundaries, path railings (OBSERVED).
    #[serde(default, skip_serializing_if = "crate::stairs_scene::StairsScene::is_empty")]
    pub stairs_scene: crate::stairs_scene::StairsScene,
    /// Reference planes, model lines, groups, wall sweeps, dimensions, tags and text notes (OBSERVED).
    #[serde(default, skip_serializing_if = "crate::annot::AnnotModel::is_empty")]
    pub annot: crate::annot::AnnotModel,
    /// Materials, toposolids, section views, view graphics, schedules and sheets (OBSERVED).
    #[serde(default, skip_serializing_if = "crate::docs::DocsModel::is_empty")]
    pub docs: crate::docs::DocsModel,
    /// The name of the browser organisation the project browser's Views node is grouped by (the node reads `Views
    /// (<name>)`). the starter template of a new project sets "all" (ASSUMED).
    #[serde(default)]
    pub browser_organization: Option<String>,
    /// What a document stores beyond its elements - the active browser organisation, the starting view, default types,
    /// units element and the stored facts of imported views and grids. Owned by [`crate::document_settings`]; empty for a
    /// document authored in ASTERION.
    #[serde(default)]
    pub settings: crate::document_settings::DocumentSettings,
    /// The free-text Comments and Mark of elements, by element id (`crate::instance_params`).
    #[serde(default, skip_serializing_if = "asterion_core::IdMap::is_empty")]
    pub texts: asterion_core::IdMap<crate::instance_params::ElementText>,
    /// The base points, the project position, the levels' Elevation Base and spot elevations (`crate::base_points`,
    /// OBSERVED).
    #[serde(default, skip_serializing_if = "crate::base_points::BasePoints::is_default")]
    pub base_points: crate::base_points::BasePoints,
    /// Unplaced rooms, kept boxes and column families with a solid (`crate::rooms_ext`, OBSERVED).
    #[serde(default, skip_serializing_if = "crate::rooms_ext::RoomsExt::is_empty")]
    pub rooms_ext: crate::rooms_ext::RoomsExt,
    /// Element, view and room phases (`crate::phases`, OBSERVED).
    #[serde(default, skip_serializing_if = "crate::phases::PhaseModel::is_empty")]
    pub phases: crate::phases::PhaseModel,
    /// The per-view extents of datums and the model extents of levels (`crate::datum_extents`, OBSERVED).
    #[serde(default, skip_serializing_if = "crate::datum_extents::DatumViews::is_empty")]
    pub datum_views: crate::datum_extents::DatumViews,
}

/// A Join Geometry RELATION between two elements.
///
/// OBSERVED: a join is a relation, not a one-off cut. One of the two elements CUTS the other; `JoinGeometry(floor, wall)`
/// makes the floor cut the wall, `SwitchJoinOrder` reverses it, and the relation persists when the elements stop
/// overlapping (the quantities come back, the join stays, and a warning is posted). It is genuinely stored state - which of
/// the two cuts is not derivable from the geometry.
///
/// Spec F-05a: the reference product stores the relation as ONE cut-pair step held by the CUT element and naming the CUTTER
/// (plus a symmetric clean-pair step on both). `cutter` here is the id inside that step; the other element is the holder.
/// Which one cuts is decided at join time by `crate::join_order::resolve_cut_order` and exchanged by `SwitchJoinOrder`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeometryJoin {
    pub a: ElementId,
    pub b: ElementId,
    /// The element that cuts the other; one of `a` and `b`.
    pub cutter: ElementId,
}

/// How the user changed the implicit join of a crossing pair of walls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrossState {
    /// `UnjoinGeometry` removed the join: neither wall is cut (OBSERVED, step 12).
    Unjoined,
    /// `SwitchJoinOrder` exchanged the cutter and the cut wall (OBSERVED, step 10).
    Swapped,
}

/// The stored departure of one crossing pair from the default join.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossOverride {
    pub a: ElementId,
    pub b: ElementId,
    pub state: CrossState,
}

/// The type of a join is read from the FIRST wall of the join for both walls of the pair.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WallJoinKind {
    #[default]
    Abut,
    Miter,
    SquareOff,
}

impl WallJoinKind {
    /// The reference product's enumeration name (`JoinType`), as the harness stringifies it.
    pub fn name(self) -> &'static str {
        match self {
            WallJoinKind::Abut => "Abut",
            WallJoinKind::Miter => "Miter",
            WallJoinKind::SquareOff => "SquareOff",
        }
    }

    pub fn from_name(s: &str) -> Option<WallJoinKind> {
        match s.to_ascii_lowercase().as_str() {
            "abut" | "butt" => Some(WallJoinKind::Abut),
            "miter" | "mitre" => Some(WallJoinKind::Miter),
            "square_off" | "squareoff" => Some(WallJoinKind::SquareOff),
            _ => None,
        }
    }
}

/// One stored departure from the default join type at a wall end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WallJoinTypeAtEnd {
    pub wall: ElementId,
    /// 0 = start, 1 = end.
    pub end: u8,
    pub kind: WallJoinKind,
}

impl BimModel {
    /// The stored state of the crossing pair `(a, b)`, if the user changed it.
    pub fn cross_override(&self, a: ElementId, b: ElementId) -> Option<CrossState> {
        self.cross_overrides.iter().find(|o| (o.a == a && o.b == b) || (o.a == b && o.b == a)).map(|o| o.state)
    }

    /// The join type stored at `end` (0 / 1) of `wall`: the default when none was set.
    pub fn join_type_at(&self, wall: ElementId, end: usize) -> WallJoinKind {
        self.wall_join_types.iter().find(|j| j.wall == wall && usize::from(j.end) == end).map_or(WallJoinKind::Abut, |j| j.kind)
    }
}

impl GeometryJoin {
    pub fn involves(&self, id: ElementId) -> bool {
        self.a == id || self.b == id
    }

    pub fn other(&self, id: ElementId) -> ElementId {
        if self.a == id {
            self.b
        } else {
            self.a
        }
    }

    pub fn matches(&self, x: ElementId, y: ElementId) -> bool {
        (self.a == x && self.b == y) || (self.a == y && self.b == x)
    }

    /// The CUT element: the holder of the cut-pair step.
    pub fn cut_element(&self) -> ElementId {
        self.other(self.cutter)
    }
}

impl BimModel {
    /// When `a` is the cutter of the join between them.
    pub fn is_cutter_of_join(&self, a: ElementId, b: ElementId) -> bool {
        self.geometry_joins.iter().any(|j| j.matches(a, b) && j.cutter == a)
    }

    /// The elements named by the cut-pair steps `e` holds, i.e. the cutters of `e`. Ascending id.
    pub fn cutting_elements_of(&self, e: ElementId) -> Vec<ElementId> {
        let mut v: Vec<ElementId> =
            self.geometry_joins.iter().filter(|j| j.involves(e) && j.cutter != e).map(|j| j.cutter).collect();
        v.sort();
        v
    }

    /// The elements that hold a cut-pair step naming `e`, i.e. the elements `e` cuts. Ascending id.
    pub fn cut_elements_of(&self, e: ElementId) -> Vec<ElementId> {
        let mut v: Vec<ElementId> =
            self.geometry_joins.iter().filter(|j| j.cutter == e).map(|j| j.cut_element()).collect();
        v.sort();
        v
    }
}

impl BimModel {
    pub fn level(&self, id: ElementId) -> Result<&Level, Failure> {
        self.levels.get(&id).ok_or_else(|| missing(id, "level"))
    }

    pub fn wall_type(&self, id: ElementId) -> Result<&WallType, Failure> {
        self.wall_types.get(&id).ok_or_else(|| missing(id, "wall type"))
    }

    pub fn wall(&self, id: ElementId) -> Result<&Wall, Failure> {
        self.walls.get(&id).ok_or_else(|| missing(id, "wall"))
    }

    pub fn wall_mut(&mut self, id: ElementId) -> Result<&mut Wall, Failure> {
        self.walls.get_mut(&id).ok_or_else(|| missing(id, "wall"))
    }

    /// The slab accessors are the real names.
    pub fn slab_type(&self, id: ElementId) -> Result<&SlabType, Failure> {
        self.floor_types.get(&id).ok_or_else(|| missing(id, "slab type"))
    }

    pub fn slab(&self, id: ElementId) -> Result<&Slab, Failure> {
        self.floors.get(&id).ok_or_else(|| missing(id, "slab"))
    }

    pub fn slab_mut(&mut self, id: ElementId) -> Result<&mut Slab, Failure> {
        self.floors.get_mut(&id).ok_or_else(|| missing(id, "slab"))
    }

    pub fn floor_type(&self, id: ElementId) -> Result<&FloorType, Failure> {
        self.floor_types.get(&id).ok_or_else(|| missing(id, "floor type"))
    }

    pub fn floor(&self, id: ElementId) -> Result<&Floor, Failure> {
        self.floors.get(&id).ok_or_else(|| missing(id, "floor"))
    }

    pub fn floor_mut(&mut self, id: ElementId) -> Result<&mut Floor, Failure> {
        self.floors.get_mut(&id).ok_or_else(|| missing(id, "floor"))
    }

    pub fn view(&self, id: ElementId) -> Result<&View, Failure> {
        self.views.get(&id).ok_or_else(|| missing(id, "view"))
    }

    pub fn view_mut(&mut self, id: ElementId) -> Result<&mut View, Failure> {
        self.views.get_mut(&id).ok_or_else(|| missing(id, "view"))
    }

    /// The level immediately above `level` by elevation, or `None` if it is the topmost.
    /// DOCUMENTED: "level above"/"level below" in a view range resolve by elevation order.
    pub fn level_above(&self, level: ElementId) -> Option<&Level> {
        let z = self.levels.get(&level)?.elevation;
        self.levels
            .values()
            .filter(|l| l.id != level && l.elevation > z)
            .min_by(|a, b| a.elevation.total_cmp(&b.elevation))
    }

    /// The level immediately below `level` by elevation, or `None` if it is the lowest.
    pub fn level_below(&self, level: ElementId) -> Option<&Level> {
        let z = self.levels.get(&level)?.elevation;
        self.levels
            .values()
            .filter(|l| l.id != level && l.elevation < z)
            .max_by(|a, b| a.elevation.total_cmp(&b.elevation))
    }

    pub fn grid(&self, id: ElementId) -> Result<&Grid, Failure> {
        self.grids.get(&id).ok_or_else(|| missing(id, "grid"))
    }

    pub fn grid_mut(&mut self, id: ElementId) -> Result<&mut Grid, Failure> {
        self.grids.get_mut(&id).ok_or_else(|| missing(id, "grid"))
    }

    /// A rectangular wall opening. Mirrors [`BimModel::floor`] / [`BimModel::grid`].
    pub fn opening(&self, id: ElementId) -> Result<&WallOpening, Failure> {
        self.wall_openings.get(&id).ok_or_else(|| missing(id, "wall opening"))
    }

    pub fn opening_mut(&mut self, id: ElementId) -> Result<&mut WallOpening, Failure> {
        self.wall_openings.get_mut(&id).ok_or_else(|| missing(id, "wall opening"))
    }

    /// Ids of the openings hosted by `host`, in ascending id order.
    ///
    /// DOCUMENTED: the reference exposes **one** query for everything inserted into a host (`HostObject.FindInserts`). In
    /// v0.1 the only insert kind that exists is the rectangular wall opening, so this is that query's whole result.
    pub fn openings_of(&self, host: ElementId) -> Vec<ElementId> {
        self.wall_openings.values().filter(|o| o.host == host).map(|o| o.id).collect()
    }

    /// A door / window type. Mirrors [`BimModel::wall_type`] / [`BimModel::floor_type`].
    pub fn hosted_type(&self, id: ElementId) -> Result<&HostedType, Failure> {
        self.hosted_types.get(&id).ok_or_else(|| missing(id, "hosted type"))
    }

    pub fn hosted_type_mut(&mut self, id: ElementId) -> Result<&mut HostedType, Failure> {
        self.hosted_types.get_mut(&id).ok_or_else(|| missing(id, "hosted type"))
    }

    /// A door / window instance.
    pub fn hosted_instance(&self, id: ElementId) -> Result<&HostedInstance, Failure> {
        self.hosted_instances.get(&id).ok_or_else(|| missing(id, "hosted instance"))
    }

    pub fn hosted_instance_mut(&mut self, id: ElementId) -> Result<&mut HostedInstance, Failure> {
        self.hosted_instances.get_mut(&id).ok_or_else(|| missing(id, "hosted instance"))
    }

    /// Ids of the hosted instances carried by `host`, in ascending id order.
    ///
    /// DOCUMENTED (section 2a): together with [`BimModel::openings_of`] this is the whole result
    /// of the reference's single insert query (`HostObject.FindInserts(true, ..)`). The *derived*
    /// union of the two, with each element's cut rectangle, is [`crate::regen::inserts`].
    pub fn hosted_instances_of(&self, host: ElementId) -> Vec<ElementId> {
        self.hosted_instances.values().filter(|i| i.host == host).map(|i| i.id).collect()
    }

    /// Ids of the instances of `type_id`, in ascending id order.
    pub fn instances_of_hosted_type(&self, type_id: ElementId) -> Vec<ElementId> {
        self.hosted_instances.values().filter(|i| i.type_id == type_id).map(|i| i.id).collect()
    }

    /// Whether `name` is already taken by a hosted type other than `except`.
    /// ASSUMED (section 7a): a hosted type's name is unique among hosted types.
    pub fn hosted_type_name_taken(&self, name: &str, except: Option<ElementId>) -> bool {
        self.hosted_types.values().any(|t| t.name == name && Some(t.id) != except)
    }

    pub fn contains(&self, id: ElementId) -> bool {
        self.levels.contains_key(&id)
            || self.wall_types.contains_key(&id)
            || self.walls.contains_key(&id)
            || self.floor_types.contains_key(&id)
            || self.floors.contains_key(&id)
            || self.views.contains_key(&id)
            || self.grids.contains_key(&id)
            || self.wall_openings.contains_key(&id)
            || self.hosted_types.contains_key(&id)
            || self.hosted_instances.contains_key(&id)
            || self.column_types.contains_key(&id)
            || self.columns.contains_key(&id)
            || self.mullion_types.contains_key(&id)
            || self.rooms.contains_key(&id)
            || self.room_separations.contains_key(&id)
            || self.stairs.types.contains_key(&id)
            || self.stairs.stairs.contains_key(&id)
            || self.stairs.runs.contains_key(&id)
            || self.stairs.landings.contains_key(&id)
            || self.stairs.railings.contains_key(&id)
            || self.stairs_scene.run_types.contains_key(&id)
            || self.stairs_scene.railing_types.contains_key(&id)
            || self.stairs_scene.path_railings.contains_key(&id)
            || self.annot.contains(id)
            || self.docs.contains(id)
            || crate::shafts::contains(self, id)
            || self.base_points.spots.contains_key(&id)
    }

    /// Elements whose regenerated state depends on `id` (the dependency graph of this slice:
    /// wall -> type / base level / top level; floor -> type / level; wall opening -> host wall).
    pub fn room(&self, id: ElementId) -> Result<&Room, Failure> {
        self.rooms.get(&id).ok_or_else(|| missing(id, "room"))
    }

    /// True when `element` is pinned.
    pub fn is_pinned(&self, element: ElementId) -> bool {
        // OBSERVED: the members of a group are reported pinned
        self.pinned.contains(&element) || self.annot.group_of(element).is_some()
    }

    /// True when `element` is hidden in `view` ("Hide in View": the view's own set of ids; the parent -> component rule is
    /// not derived, see [`View::hidden_elements`]).
    pub fn is_hidden_in_view(&self, view: ElementId, element: ElementId) -> bool {
        self.views.get(&view).map_or(false, |v| v.hidden_elements.contains(&element))
    }

    /// The largest element id the model holds in any of its element maps (`None` for an empty model). A loaded document
    /// raises its id counter above it (Spec P-01: the counter is raised to cover ids it did not hand out, never lowered).
    pub fn max_element_id(&self) -> Option<ElementId> {
        let k = |m: Option<&ElementId>| m.copied();
        [
            k(self.levels.keys().next_back()),
            k(self.wall_types.keys().next_back()),
            k(self.walls.keys().next_back()),
            k(self.floor_types.keys().next_back()),
            k(self.floors.keys().next_back()),
            k(self.views.keys().next_back()),
            k(self.grids.keys().next_back()),
            k(self.wall_openings.keys().next_back()),
            k(self.hosted_types.keys().next_back()),
            k(self.hosted_instances.keys().next_back()),
            k(self.column_types.keys().next_back()),
            k(self.columns.keys().next_back()),
            k(self.mullion_types.keys().next_back()),
            k(self.rooms.keys().next_back()),
            k(self.room_separations.keys().next_back()),
            k(self.stairs.types.keys().next_back()),
            k(self.stairs.stairs.keys().next_back()),
            k(self.stairs.runs.keys().next_back()),
            k(self.stairs.landings.keys().next_back()),
            k(self.stairs.railings.keys().next_back()),
            self.stairs_scene.max_id(),
            self.annot.max_id(),
            self.docs.max_id(),
        ]
        .into_iter()
        .flatten()
        .max()
    }

    /// The elements whose derived state depends on `id` (its type, its level, its host).
    ///
    /// Every element record owns parent sets (deletion parents, regeneration parents, dependency parents, ...) and an
    /// element's associated level is a stored id whose change touches the element's own associated-level atom; the
    /// per-class choice of which atom of the level a dependent registers is not specified. The engine's derived state is a
    /// pure function of the model, so this list is only what a commit-time check must revisit.
    pub fn dependents(&self, id: ElementId) -> Vec<ElementId> {
        let walls = self
            .walls
            .values()
            .filter(|w| w.type_id == id || w.base_level == id || w.top_level == Some(id))
            .map(|w| w.id);
        let floors = self.floors.values().filter(|f| f.type_id == id || f.level == id).map(|f| f.id);
        // A view's derived content depends on its generating level (and, through the range, on the
        // levels above and below it); the view element itself survives a level change.
        let views = self.views.values().filter(|v| v.gen_level == Some(id)).map(|v| v.id);
        // ASSUMED: an opening's cut is a pure function of its host, so any host change re-derives it - and a host change
        // that pushes the cut out of the host is a commit-time failure, which is why the opening itself must be revisited.
        let openings = self.wall_openings.values().filter(|o| o.host == id).map(|o| o.id);
        // ASSUMED: "`BimModel::dependents` gains: wall -> its hosted instances and wall openings; level -> the instances
        // placed on it; hosted type -> its instances." All three are listed here because each one re-derives the instance's
        // cut (the host gives the outline and the thickness, the level gives the sill datum, the type gives the size), and
        // a change that pushes the cut out of the host is a commit-time failure - so the instance itself has to be
        // revisited, exactly as an opening is.
        let instances = self
            .hosted_instances
            .values()
            .filter(|i| i.host == id || i.level == id || i.type_id == id)
            .map(|i| i.id);
        walls.chain(floors).chain(views).chain(openings).chain(instances).collect()
    }
}

fn missing(id: ElementId, what: &str) -> Failure {
    Failure::error(codes::NOT_FOUND, format!("No {what} with id {id}."))
}

#[cfg(test)]
mod layer_width_tests {
    use super::*;

    /// Non-negative length; |w| <= 1e-9 ft only for a membrane (here); a structure layer of width 0 is refused (OBSERVED
    /// earlier: "One or more layers is not valid.").
    #[test]
    fn a_layer_width_follows_the_read_validity_rule() {
        assert_eq!(layer_width_fault(0.2, LayerFunction::Structure), None);
        assert_eq!(layer_width_fault(0.0, LayerFunction::Membrane), None);
        assert!(layer_width_fault(0.0, LayerFunction::Structure).is_some(), "only membranes may be zero");
        assert!(layer_width_fault(-0.01, LayerFunction::Membrane).unwrap().contains("negative"));
        // inside the 1e-9 ft slack a tiny negative width is a zero width, not a negative one
        assert_eq!(layer_width_fault(-1e-10 * 0.3048, LayerFunction::Membrane), None);
        assert!(layer_width_fault(-1e-10 * 0.3048, LayerFunction::Finish1).unwrap().contains("zero"));
        // beyond the length limit of 30000 ft (+1e-9) is not a valid length
        assert!(layer_width_fault(30000.1 * 0.3048, LayerFunction::Structure).is_some());
        assert!(layer_width_fault(f64::NAN, LayerFunction::Structure).is_some());
        // and the structure builder uses it
        let e = CompoundStructure::new(vec![Layer { width: 0.0, function: LayerFunction::Structure }], 0, 0);
        assert!(e.is_err());
        let ok = CompoundStructure::new(
            vec![
                Layer { width: 0.2, function: LayerFunction::Structure },
                Layer { width: 0.0, function: LayerFunction::Membrane },
            ],
            0,
            0,
        );
        assert!(ok.is_ok());
    }
}
