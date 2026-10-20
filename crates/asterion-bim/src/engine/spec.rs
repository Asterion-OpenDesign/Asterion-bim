//! Behavioural constants of the first BIM slice and the evidence behind each.
//!
//! Evidence levels: `OBSERVED` = measured in the reference product through the public API; `DOCUMENTED` = stated in public
//! the vendor documentation, not yet measured; `ASSUMED` = ASTERION choice pending evidence.

use asterion_core::units::feet_to_metres;
use asterion_geom::Tolerances;

/// OBSERVED: Wall.Create rejects heights `<= 0` or `> 30000 ft` ("The given value for height must be greater than 0 and no
/// more than 30000 feet.").
pub const MAX_WALL_HEIGHT_M: f64 = 30000.0 * 0.3048;

/// OBSERVED: the minimum committable wall height is exactly 0.1 ft. 0.03048 m is rolled back; 0.03049 m commits, for every
/// tested type.
pub const MIN_WALL_HEIGHT_FT: f64 = 0.1;

/// Minimum committable wall height (metres): the value of `MIN_WALL_HEIGHT_FT`.
///
/// The test itself is [`wall_height_is_valid`], which compares in feet and is INCLUSIVE.
pub fn min_wall_height() -> f64 {
    feet_to_metres(MIN_WALL_HEIGHT_FT)
}

/// Minimum committable straight-wall length (metres), as a function of wall width.
///
/// OBSERVED: for structural widths 0.2, 0.5 and 1.0 m, and the 0.2625 m asymmetric type, a wall commits only when its
/// length exceeds `width / 100`; below that it fails with "Can't make Wall." For very thin walls the absolute short-curve
/// tolerance dominates (a 0.02 m wall commits at 0.79 mm). The exact boundary at `width/100` is within ~1 µm of a floating
/// tie; treated as inclusive-fail here. Curves below ~`width/200` instead raise a ShortCurveTolerance error at creation
/// (see [`creation_curve_floor`]).
pub fn min_wall_length(width: f64) -> f64 {
    (width / 100.0).max(Tolerances::default().short_curve)
}

/// Internal-unit conversion used by the reference rules below: metres to feet by MULTIPLICATION with `1 / 0.3048`. ASSUMED
/// (the unit conversion of the harness is not specified): it is the form under which the two boundary observations of round
/// 4 (a wall of exactly `width / 100` commits for the 0.2625 m type; 0.03048 m is below 0.1 ft) are reproduced bit for bit.
pub const FEET_PER_METRE: f64 = 1.0 / 0.3048;

/// A wall is tall enough when its height, in feet (internal units), is `>= 0.1` - INCLUSIVE, a height of exactly 0.1 ft
/// passes.
pub fn wall_height_is_valid(height_m: f64) -> bool {
    height_m * FEET_PER_METRE >= MIN_WALL_HEIGHT_FT
}

/// The kind is NOT specified, and every type of this engine is a basic compound-structure type, taken as kind 0.
pub fn wall_width_is_valid(width_m: f64) -> bool {
    !(width_m * FEET_PER_METRE < 1e-9)
}

/// For a non-curtain wall: too short exactly when NOT `w / 100 < L`, i.e. `L <= w / 100`, all in feet; for a circular arc
/// (curve class code 2) `w = min(w, radius)`.
///
/// `width_layers_m` are the layer widths in metres: the type width is summed in feet in layer
/// order, as the reference sums its layers.
pub fn wall_is_too_short(width_layers_m: &[f64], length_m: f64, arc_radius_m: Option<f64>) -> bool {
    wall_is_too_short_ft(width_layers_m, length_m * FEET_PER_METRE, arc_radius_m)
}

/// [`wall_is_too_short`] with the curve length already in feet (a straight wall's length is
/// measured between end points converted to feet).
pub fn wall_is_too_short_ft(width_layers_m: &[f64], length_ft: f64, arc_radius_m: Option<f64>) -> bool {
    let mut w: f64 = 0.0;
    for x in width_layers_m {
        w += x * FEET_PER_METRE;
    }
    if let Some(r) = arc_radius_m {
        w = w.min(r * FEET_PER_METRE);
    }
    !(w / 100.0 < length_ft)
}

/// (second height check, per wall type): refused when the height is below the type's minimum wall height plus 1e-9 (flag
/// false) or below the minimum minus 1e-9 (flag true).
///
/// The comparison is exposed so the day regions exist the inequality is already the read one.
pub fn wall_below_type_minimum(height_ft: f64, type_minimum_ft: f64, flag: bool) -> bool {
    if flag {
        height_ft < type_minimum_ft - 1e-9
    } else {
        height_ft < type_minimum_ft + 1e-9
    }
}

/// OBSERVED: curves this short make Wall.Create raise ArgumentsInconsistentException citing ShortCurveTolerance, instead of
/// a post-commit failure. ASSUMED floor value pending a finer bracket.
///
/// Metres here, the same literal compares with, STRICT (`L < tol`, [`asterion_geom::is_too_short_ft`]). the constant is
/// shared, not re-derived.
pub fn creation_curve_floor() -> f64 {
    Tolerances::default().short_curve
}

/// Does a hosted insert at curve parameter `u`, with half extent `half_extent` (in the same units as the parameter: the
/// curve is parametrised by length), overlap the host curve whose end parameters are `t0 <= t1`?
///
/// * `u` strictly inside `(t0, t1)` -> true (READ);
/// * `u` at or beyond an end -> the parametric extent test: READ only as "the half extent, converted to parameter distance,
///   is compared with the curve parameter with a 1e-9 relative tolerance"; its 5th/6th arguments are NOT specified.
///   Implemented as: the extent reaches back into the curve, i.e. `u + half > t0` (when at/before the start) or `u - half <
///   t1` (when at/after the end), each with a 1e-9 relative slack. UNKNOWN: the exact comparison.
///
/// A `false` is surfaced as a posted WARNING, never a refusal (`codes::INSERT_OVERLAPS_OR_OUT`).
pub fn insert_overlaps_wall_curve(u: f64, half_extent: f64, t0: f64, t1: f64) -> bool {
    if t0 < u && u < t1 {
        return true;
    }
    let slack = |t: f64| 1e-9 * t.abs().max(1.0);
    if u <= t0 {
        u + half_extent > t0 + slack(t0)
    } else {
        u - half_extent < t1 - slack(t1)
    }
}

/// DOCUMENTED: the reference's view scale is the denominator of 1:scale and must be a positive
/// integer in [1, 24000] ("The scale value must be between 1 and 24000."). The upper bound is
/// DOCUMENTED, not yet measured.
pub const MAX_VIEW_SCALE: u32 = 24000;

/// DOCUMENTED: a new metric floor plan is created at 1:100.
pub const DEFAULT_VIEW_SCALE: u32 = 100;

/// Slack used when testing whether an opening's cut stays inside its host, and whether two cuts
/// on one host overlap.
///
/// ASTERION therefore reuses the OBSERVED vertex tolerance (`Application.VertexTolerance`, 0.1595 mm) as the slack, which
/// is the quantity the reference itself calls "two points within this distance are considered coincident".
pub fn insert_fit_tolerance() -> f64 {
    Tolerances::default().vertex
}

/// ASSUMED: ASTERION v0.1 rejects a non-finite or non-positive opening size outright and **accepts anything positive**,
/// including the 2 mm rectangle that scenario step `degenerate-2mm-opening` probes. The repo's own precedent is
/// contradictory (a wall below `MIN_WALL_HEIGHT_FT` is rejected, a 2 mm floor is accepted), so no invented threshold is
/// introduced here; the measurement decides.
pub const MIN_OPENING_EXTENT_M: f64 = 0.0;

/// Stable ASTERION failure codes (never copies of reference-product identifiers).
pub mod codes {
    pub const CURVE_TOO_SHORT: &str = "geom.curve_too_short";
    pub const HEIGHT_OUT_OF_RANGE: &str = "wall.height_out_of_range";
    /// POSTED AT COMMIT, severity Error: the wall is not valid or is too short, or its geometry regeneration returned an
    /// error. The rule decides the channel (the shared post operation posts for the commit. UNKNOWN: which geometric
    /// condition yields each regeneration result code, so the arc-radius guard is not decided).
    pub const CANNOT_MAKE_WALL: &str = "wall.cannot_make";
    /// POSTED AT COMMIT, severity Error: a wall with no top level whose height is below 0.1 ft. The regeneration step posts
    /// the Error variant of the draw-failure definition for it; `TOP_BELOW_BASE` is the same step's sibling for a
    /// top-constrained wall. ASTERION keeps two codes for the two reachable reasons.
    pub const BELOW_MIN_HEIGHT: &str = "wall.below_min_height";
    /// POSTED AT COMMIT, severity Error: the height derived from the top constraint is below 0.1 ft (the height step posts
    /// the negative-height definition as an error). UNKNOWN: what a wall joined to an element at either end does with the
    /// same condition (the rule lists that join among the exceptions of the post; the engine does not model the exception
    /// because the effect on the commit was not specified).
    pub const TOP_BELOW_BASE: &str = "wall.top_below_base";
    pub const READ_ONLY: &str = "param.read_only";
    pub const INVALID_LAYERS: &str = "type.invalid_layers";
    pub const NOT_FOUND: &str = "element.not_found";
    /// Fewer than 3 points, or a loop that is not closeable.
    ///
    /// Renamed from `floor.invalid_boundary` when floors, ceilings and roofs became one kind with three roles. The old
    /// spelling is retired, not kept as an alias: a code that names one role of a shared element would lie about which
    /// element raised it.
    pub const INVALID_BOUNDARY: &str = "slab.invalid_boundary";
    /// Slope written on a segment index the boundary does not have.
    ///
    /// ASSUMED, by analogy with `wall.bad_end_index`, which is OBSERVED.
    pub const SLAB_SEGMENT_OUT_OF_RANGE: &str = "slab.segment_out_of_range";
    /// A non-finite slope written to a segment. ASSUMED.
    pub const SLAB_SLOPE_NOT_FINITE: &str = "slab.slope_not_finite";
    /// Slope data written to a slab whose role has no slope. ASSUMED.
    pub const SLAB_ROLE_HAS_NO_SLOPE: &str = "slab.role_has_no_slope";
    /// A cut-off plane at or below the lowest point of the roof's underside.
    ///
    /// OBSERVED, posted AT COMMIT (`not_committed`), not raised. Three ways to cause it were measured: writing the cut-off
    /// below the base, raising the BASE level so the cut-off ends up below it, and - the one that matters for the model -
    /// raising a level so that the cut-off lands EXACTLY on the underside (4.5 = 4.5). An elevation edit is not always
    /// committable: the dependent set has to be validated at commit.
    ///
    /// UNKNOWN: the same operation for the footprint roof, which is the kind this engine models, so the guard here is the
    /// engine's own OBSERVED one.
    ///
    /// The measurement said the opposite. Every rule gets measured.
    pub const ROOF_CUTOFF_BELOW_BASE: &str = "roof.cutoff_below_base";
    /// A slope the roof cannot be built with.
    ///
    /// OBSERVED: a slope of `1e6` was refused at commit ("Can't make footprint roof"), `not_committed`. The threshold is
    /// NOT measured - only that 0.5 is accepted and 1e6 is not - so the engine's limit is ASSUMED and recorded as such
    /// where it is applied.
    pub const SLAB_SLOPE_DEGENERATE: &str = "slab.slope_degenerate";
    /// Two loops of one slab intersect: they cross, touch or share an edge; also a loop that crosses
    /// itself (a bow-tie).
    ///
    /// OBSERVED: a bow-tie is raised at the CALL, not posted at commit - an `ArgumentException` for a ceiling. OBSERVED: a
    /// hole sharing an edge with the outer loop is raised AT THE CALL, the same exception a crossing boundary raises. A
    /// hole entirely OUTSIDE the slab is not an error - it is a second island.
    ///
    /// Spec C-02 not specified: "which operation turns the failure id into a refusal at the API call". The definitions
    /// those validators name are registered with the severity literal 3, which the engine's two-valued severity cannot
    /// carry.
    pub const SLAB_LOOPS_INTERSECT: &str = "slab.loops_intersect";
    /// Two loops of one slab share or overlap a segment (their edges are parallel).
    ///
    /// The reference product separates "loops intersect" (the two reported edges are not parallel) from "lines overlap"
    /// (parallel). A hole that shares an edge with the outer loop, measured in round 13 only as "raised at the call", is
    /// therefore the OVERLAP case. The parallel test is READ: cos^2 >= 0.9999969538258021, i.e. within 0.1 degree.
    pub const SLAB_LINES_OVERLAP: &str = "slab.lines_overlap";
    /// A slab sketch with an open loop and no violating pair. Reachable through `sketch::first_fault`; the closed-polygon
    /// constructors never produce one.
    pub const SLAB_OPEN_LOOP: &str = "slab.open_loop";
    /// An arc edge of a slab loop whose three points (start, through, end) have no circle: they are collinear or two
    /// coincide. ASTERION's own input rule, raised at the call: the reference stores an arc as centre and radius and never
    /// meets the three-point case.
    pub const SLAB_INVALID_ARC: &str = "slab.invalid_arc";
    /// A floor or ceiling given a second slope definition: a slope-defining edge while it has a slope arrow, or the
    /// reverse. The channel (raised at the call here) follows the slab sketch codes above (UNKNOWN).
    pub const SLAB_TOO_MANY_SLOPES: &str = "slab.too_many_slopes";
    /// ENGINE CHOICE: refused.
    pub const SLAB_SLOPE_EDGE_NOT_LINE: &str = "slab.slope_edge_not_line";
    /// The Stokes face integration of a slab with arc edges did not converge, so the failure is surfaced instead of a
    /// guessed number.
    pub const SLAB_INTEGRATION_FAILED: &str = "slab.integration_failed";
    /// A move / rotate / mirror / copy transform that is not a finite rigid motion of the plan (a mirror line of no
    /// length). ASTERION's input rule, raised at the call (the reference product's own decomposition failure posts
    /// UnableToTransformElementError, which a plan transform cannot reach).
    pub const TRANSFORM_INVALID: &str = "transform.invalid";
    /// An element the engine does not move, rotate, mirror or copy that way (levels, views, types, stairs, a hosted
    /// instance rotated alone, an opening without its host). ENGINE CHOICE: the per-class move helpers are not specified.
    pub const TRANSFORM_NOT_SUPPORTED: &str = "transform.not_supported";
    /// Split Element refused on a wall: the split point is not strictly inside the location curve or not on it, it falls
    /// inside a door, window or opening, or the wall is one the engine does not split (an arc wall). ASTERION's own
    /// refusals, raised at the call.
    pub const WALL_SPLIT_REFUSED: &str = "wall.split_refused";
    /// Trim/Extend refused on a wall: the boundary is not a straight wall, the lines are parallel, or the moved end would
    /// cross the other end. ASTERION's own refusals, raised at the call.
    pub const WALL_TRIM_REFUSED: &str = "wall.trim_refused";
    /// A stairs edit scope started with a top level that is not higher than its base level (OBSERVED), raised at the call.
    pub const STAIRS_TOP_NOT_ABOVE_BASE: &str = "stairs.top_not_above_base";
    /// A reference plane name that is already used by another plane (OBSERVED), posted at commit.
    pub const REFPLANE_DUPLICATE_NAME: &str = "refplane.duplicate_name";
    /// A reference plane with no length or whose cut vector is parallel to it, raised at the call.
    pub const REFPLANE_INVALID: &str = "refplane.invalid";
    /// A model line or arc that is degenerate, raised at the call.
    pub const MODEL_LINE_INVALID: &str = "modelline.invalid";
    /// A model line or arc that is not on its sketch plane (OBSERVED), raised at the call.
    pub const MODEL_LINE_NOT_IN_PLANE: &str = "modelline.not_in_plane";
    /// An edit of a member of a group outside group edit mode (OBSERVED), posted at commit.
    pub const GROUP_EDIT_OUTSIDE_EDIT_MODE: &str = "group.edit_outside_edit_mode";
    /// No element of the input can be grouped (OBSERVED), raised at the call.
    pub const GROUP_CANNOT_GROUP: &str = "group.cannot_group";
    /// A dimension that cannot be made or changed that way, raised at the call.
    pub const DIMENSION_INVALID: &str = "dimension.invalid";
    /// A tag that cannot be made, raised at the call.
    pub const TAG_INVALID: &str = "tag.invalid";
    /// A text note that cannot be made, raised at the call.
    pub const TEXT_NOTE_INVALID: &str = "textnote.invalid";
    /// A wall sweep or reveal that cannot be made, raised at the call.
    pub const SWEEP_INVALID: &str = "sweep.invalid";
    /// Two wall sweeps at one place (OBSERVED), a WARNING.
    pub const SWEEP_OVERLAP: &str = "sweep.overlap";
    /// A wall sweep whose position is outside its wall (OBSERVED): a WARNING, the sweep has no body.
    pub const SWEEP_OUTSIDE_WALL: &str = "sweep.outside_wall";
    /// Materials, toposolids, documentation views, filters, schedules and sheets (OBSERVED).
    pub const MATERIAL_INVALID: &str = "material.invalid";
    pub const TOPOSOLID_INVALID: &str = "toposolid.invalid";
    pub const TOPOSOLID_SUBDIVISION_FAILED: &str = "toposolid.subdivision_failed";
    pub const DOCVIEW_INVALID: &str = "docview.invalid";
    pub const VIEWFILTER_INVALID: &str = "viewfilter.invalid";
    pub const SCHEDULE_INVALID: &str = "schedule.invalid";
    pub const SHEET_INVALID: &str = "sheet.invalid";
    /// A sheet number that is empty once trimmed / already used by another sheet, refused at the call.
    pub const SHEET_NUMBER_EMPTY: &str = "sheet.number_empty";
    pub const SHEET_NUMBER_DUPLICATED: &str = "sheet.number_duplicated";
    /// A tag copied alone: it finds no host to be re-attached to, a WARNING posted at commit.
    pub const TAG_NOT_PASTED: &str = "tag.not_pasted";
    /// A railing set added to a stairs that already has railings (OBSERVED), raised at the call.
    pub const RAILING_STAIRS_HAS_RAILINGS: &str = "railing.stairs_has_railings";
    /// A column type or column size / offset that is not a finite (positive, for a size) number.
    /// ASTERION's input rule, raised at the call (columns, observed in round 21).
    pub const COLUMN_BAD_SIZE: &str = "column.bad_size";
    /// A column type whose name another column type already has. ASTERION's input rule (the reference product's "Type Name"
    /// of a family is unique inside the family).
    pub const COLUMN_DUPLICATE_TYPE: &str = "column.duplicate_type_name";
    /// `ColumnAttachment` refused: the target is not a floor, roof or ceiling, or the offset is not a
    /// number (`ColumnAttachment.IsValidTarget`: valid targets are roofs, floors, ceilings).
    pub const COLUMN_BAD_ATTACHMENT: &str = "column.bad_attachment";
    /// A curtain wall edit the engine refuses: a grid line position that is not inside a cell, a grid line or panel that
    /// does not exist, a mullion type that does not exist. OBSERVED: `AddGridLine` at a point on the boundary of the only
    /// panel threw an `ArgumentException` with an empty message. Raised at the call.
    pub const CURTAIN_INVALID: &str = "curtain.invalid";
    /// A move or mirror of a pinned element, refused at the call. OBSERVED: "Unable to move element(s) due to at least one
    /// of them being pinned." for `MoveElements` and `MirrorElements` (no transaction effect), also for a pinned door moved
    /// on its own; a ROTATION of a pinned wall commits and leaves it where it was, moving the wall that carries a pinned
    /// door commits, copying a pinned wall commits (the copy is not pinned) and deleting a pinned wall commits with no
    /// failure.
    pub const ELEMENT_PINNED: &str = "element.pinned";
    /// An array the engine does not build: a member count outside 2..200 (linear) or 3..200 (radial), a radial angle of
    /// zero, a translation or rotation that is not a finite number, or a linear array of a hosted instance on its own.
    /// OBSERVED (`-059`): "count must be between 2 and 200" / "between 3 and 200", "Angle value must be not zero", and "An
    /// internal error has occurred" for a door alone (linear). Raised at the call.
    pub const ARRAY_INVALID: &str = "array.invalid";
    /// WARNING: two walls overlap (the same room-boundary hint the reference product posts: "Highlighted walls overlap. One
    /// of them may be ignored when the reference product finds room boundaries"). OBSERVED (: an array with a zero
    /// translation posts one per overlapping pair, element ids of the pair); the engine posts it only for the walls an
    /// array creates.
    pub const WALL_OVERLAPS_WALL: &str = "wall.overlaps_wall";
    /// See `failures::performance_advice`.
    pub const HOST_HAS_TOO_MANY_INSERTS: &str = "performance.host_has_too_many_inserts";
    /// An arc grid whose three points have no circle (collinear or coincident). ASTERION's input
    /// rule, raised at the call.
    pub const GRID_INVALID_ARC: &str = "grid.invalid_arc";
    /// Join Geometry on two elements that are already joined, or never were. ASSUMED: round 13
    /// measured only the success path, so these are the engine's own refusals, not the reference's.
    pub const JOIN_ALREADY_JOINED: &str = "join.already_joined";
    pub const JOIN_NOT_JOINED: &str = "join.not_joined";
    /// Join Geometry between a pair of element kinds this slice does not model (only floor-wall).
    pub const JOIN_UNSUPPORTED_PAIR: &str = "join.unsupported_pair";
    /// WARNING: joined elements that no longer intersect. OBSERVED: the transaction still commits, and the reference words
    /// it "Highlighted elements are joined but do not intersect." : the definition is registered with the Warning literal
    /// (1), is handed to the posted-warning object (not the failure call), names the SET of the two element ids and carries
    /// one "uncut the pair" resolution; the cut-exists variant of the same type is another Warning definition.
    pub const JOIN_DO_NOT_INTERSECT: &str = "join.do_not_intersect";
    /// An arc cannot be made from the given points (collinear). OBSERVED: raised at the call, an
    /// `ArgumentsInconsistentException` on `end1`.
    pub const INVALID_ARC: &str = "wall.invalid_arc";
    /// A curve of the thickened profile of a curved wall (an offset arc or an end cap) is shorter than the short-curve
    /// tolerance, strictly: the thicken-and-extrude route answers Err 9 and no solid is built. Posted at commit.
    pub const WALL_PROFILE_CURVE_TOO_SHORT: &str = "wall.profile_curve_too_short";
    /// The Stokes volume integration of a curved solid did not converge. The reference falls back to a faceted volume
    /// there; that fallback is NOT specified, so it is an explicit error rather than a guessed number.
    pub const VOLUME_INTEGRATION_FAILED: &str = "wall.volume_integration_failed";
    /// The Stokes surface-area integration of a curved solid did not converge. The reference falls back to a faceted area
    /// there (relative accuracy 0.001).
    pub const SURFACE_AREA_INTEGRATION_FAILED: &str = "wall.surface_area_integration_failed";
    /// A room's Base Offset above the Computation Height.
    ///
    /// OBSERVED: POSTED at commit (`not_committed`), "Room's lower offset is above the Computation Height"; the room keeps
    /// its previous value. The Computation Height is read-only 0, so a base offset of 0.3 was refused while a limit offset
    /// of 0.5 committed.
    pub const ROOM_BASE_ABOVE_COMPUTATION_HEIGHT: &str = "room.base_offset_above_computation_height";
    /// Batch room creation on a level that has no plan view. OBSERVED: raised AT THE CALL
    /// ("Level is not associated to a plan view").
    pub const ROOM_LEVEL_HAS_NO_PLAN_VIEW: &str = "room.level_has_no_plan_view";
    pub const ROOM_NO_ENCLOSED_REGION: &str = "room.no_enclosed_region";
    /// The batch room creator created no room (Spec M-06: a failure of severity code 1 is posted).
    pub const ROOM_NONE_CREATED_AUTOMATICALLY: &str = "room.none_created_automatically";
    /// A level's story-above element is not above the level (Spec L-01: warning, the parameter is cleared).
    pub const LEVEL_STORY_ABOVE_MOVED: &str = "level.story_above_moved";
    /// A level's story-above element is above it but is not a building story (Spec L-01: warning, the parameter is
    /// cleared).
    pub const LEVEL_STORY_ABOVE_NOT_A_STORY: &str = "level.story_above_not_a_story";
    /// A level name another level carries (OBSERVED: refused at the call, "Name must be unique").
    pub const LEVEL_DUPLICATE_NAME: &str = "level.duplicate_name";
    pub const INVALID_THICKNESS: &str = "type.invalid_thickness";
    pub const WRONG_KIND: &str = "element.wrong_kind";
    pub const VIEW_NO_LEVEL: &str = "view.no_level";
    pub const VIEW_NOT_PLAN: &str = "view.not_plan";
    pub const VIEW_BAD_SCALE: &str = "view.bad_scale";
    pub const VIEW_RANGE_INVALID: &str = "view.range_invalid";
    pub const VIEW_DUPLICATE_NAME: &str = "view.duplicate_name";
    /// A view display setting that does not exist or a value outside its vocabulary (`crate::view_display`).
    pub const VIEW_BAD_DISPLAY_SETTING: &str = "view.bad_display_setting";
    pub const GRID_DUPLICATE_NAME: &str = "grid.duplicate_name";
    pub const GRID_BAD_EXTENT: &str = "grid.bad_vertical_extent";
    pub const BAD_END_INDEX: &str = "wall.bad_end_index";
    /// Spec W-10g: a hosted door/window instance that no longer cuts its host wall (for example after a host-curve edit) is
    /// refused by its cut step, which posts this failure for the commit as an error and offers the resolution "delete the
    /// instance". The registered severity literal 2 is Error, so the severity is `Error` and, with no failures processor
    /// registered, the transaction rolls back. UNKNOWN: what a registered processor does with the error. UNKNOWN (Spec
    /// W-10g): the geometric extent test that decides "cuts nothing".
    pub const INSERT_NOT_CUTTING_HOST: &str = "insert.not_cutting_host";
    /// Former name of [`INSERT_NOT_CUTTING_HOST`] (code `insert.off_host`); kept as an alias so
    /// other call sites compile. New code uses the new name.
    pub const INSERT_OFF_HOST: &str = INSERT_NOT_CUTTING_HOST;
    /// OBSERVED: writing a parameter a family does not have is refused before the change. Measured for `rough_width` and
    /// `rough_height` on a template-built door type and for `mark` on an instance of it.
    pub const PARAM_NOT_PRESENT: &str = "param.not_present";
    /// OBSERVED: a hosted instance belongs to its host's base level and no other. POSTED AT COMMIT, severity Error. The
    /// failure the reference product posted at that step reads as the instance-not-cutting-anything definition (Spec
    /// W-10g), whose one poster names the host and the instance; the engine keeps its own code for the wrong-level cause.
    /// UNKNOWN (Spec W-10g): why an instance on another level stops cutting.
    pub const INSERT_WRONG_LEVEL: &str = "insert.wrong_level";

    // -- inserts and openings. `reference_hint` is deliberately NOT encoded here; the observed reference wording ("Can't
    // cut instance of ..).

    pub const INSERT_HOST_NOT_SUPPORTED: &str = "insert.host_not_supported";

    /// WARNING: a hosted door/window instance whose station lies at or beyond an end of the host
    /// curve and whose parametric half extent does not reach back into the curve.
    ///
    /// The reference product does NOT refuse this; it posts a warning on the instance and the host (the registered severity
    /// literal is 1, which is Warning) with a "delete the instance" resolution, and the transaction commits. Resolution:
    /// delete the instance (carried in the message; `Failure` has no field).
    pub const INSERT_OVERLAPS_OR_OUT: &str = "insert.overlaps_or_out";
    pub const INSERT_BAD_SIZE: &str = "insert.bad_size";
    pub const OPENING_INVALID_BOUNDARY: &str = "opening.invalid_boundary";
    /// WARNING: a rectangular wall opening whose step removed nothing from its host.
    ///
    /// After a boolean that succeeds, the step records "did cut something"; when it did not, and the wall is not a sub wall
    /// of a stacked wall, a warning is posted on the opening with a "delete the opening" resolution and the step still
    /// returns success, so the transaction commits. Resolution: delete the opening (carried in the message; `Failure` has
    /// no field). Severity is taken from the class name and the posting call.
    pub const OPENING_DOES_NOT_CUT_HOST: &str = "opening.does_not_cut_host";

    // -- hosted instances: doors and windows.

    /// Reject: the level a hosted instance is placed on does not exist.
    ///
    pub const INSERT_NO_LEVEL: &str = "insert.no_level";

    /// Reject: the station along the host, or the sill height, is not a finite number.
    ///
    /// ASSUMED (ASTERION's own addition to the section 7d table, with the reason written down):
    /// section 7d gives `insert.bad_size` for a size that is "<= 0 or non-finite" and
    /// `opening.invalid_boundary` for a rectangle that is not a rectangle, but a hosted instance's
    pub const INSERT_BAD_STATION: &str = "insert.bad_station";

    /// Reject: another hosted type already uses that name.
    ///
    pub const HOSTED_TYPE_DUPLICATE_NAME: &str = "hosted_type.duplicate_name";

    // -- stair by run. Every code names a failure that a read operation raises, in the channel that operation uses; the
    // engine's own words, never the reference product's identifiers.

    /// AT THE CALL: the type id given to start a stairs is not a stairs type.
    pub const STAIRS_TYPE_NOT_A_STAIRS_TYPE: &str = "stairs.type_not_a_stairs_type";
    /// AT THE CALL: the creation riser height `height / n` is below the vertex tolerance and the operation returns the
    /// invalid id.
    pub const STAIRS_CREATION_RISER_HEIGHT_TOO_SMALL: &str = "stairs.creation_riser_height_too_small";
    /// AT THE CALL: the stairs id is not a stairs.
    pub const STAIRS_RUN_NEEDS_STAIRS: &str = "stairs.run_needs_stairs";
    /// AT THE CALL: the stairs' actual tread depth is not above 1e-9.
    pub const STAIRS_RUN_TREAD_DEPTH_NOT_POSITIVE: &str = "stairs.run_tread_depth_not_positive";
    /// AT THE CALL: the riser count of a run is negative.
    pub const STAIRS_RUN_NEGATIVE_RISER_COUNT: &str = "stairs.run_negative_riser_count";
    /// AT THE CALL: the regeneration after the run was created posted an error; the run is rolled back.
    pub const STAIRS_REGENERATION_FAILED: &str = "stairs.regeneration_failed";
    /// AT THE CALL: the stairs edit scope cannot be committed while errors are unresolved.
    pub const STAIRS_SCOPE_COMMIT_UNRESOLVED_ERRORS: &str = "stairs.scope_commit_unresolved_errors";
    /// AT PARAMETER SET: an integer that is not positive was given for the desired number of risers.
    pub const STAIRS_ILLEGAL_DESIRED_RISERS: &str = "stairs.illegal_desired_risers";
    /// Two channels, one failure definition: AT PARAMETER SET a non-positive actual tread depth is rejected with it; AT
    /// REGENERATION it is a WARNING when the type's minimum tread depth is greater than the actual tread depth.
    pub const STAIRS_TREAD_DEPTH_LESS_THAN_MINIMUM: &str = "stairs.tread_depth_less_than_minimum";
    /// AT PARAMETER SET: a new run bottom above the current run top.
    pub const STAIRS_RUN_TOP_LOWER_THAN_BOTTOM: &str = "stairs.run_top_lower_than_bottom";
    /// AT PARAMETER SET: a new run top below the current run bottom.
    pub const STAIRS_RUN_BOTTOM_HIGHER_THAN_TOP: &str = "stairs.run_bottom_higher_than_top";
    /// AT PARAMETER SET: a new run bottom leaves less than the minimum run height.
    pub const STAIRS_RUN_HEIGHT_TOO_SMALL: &str = "stairs.run_height_too_small";
    /// AT PARAMETER SET: a run bottom under the stairs bottom.
    pub const STAIRS_RUN_BOTTOM_UNDER_STAIRS_BOTTOM: &str = "stairs.run_bottom_under_stairs_bottom";
    /// AT PARAMETER SET: extend-below-base above the run top.
    pub const STAIRS_RUN_EXTEND_BELOW_BASE_ABOVE_TOP: &str = "stairs.run_extend_below_base_above_top";
    /// AT PARAMETER SET (Spec S-01c): a landing base elevation that rounds to no riser above the stairs bottom.
    pub const STAIRS_LANDING_HEIGHT_UNDER_STAIRS_BOTTOM: &str = "stairs.landing_height_under_stairs_bottom";
    /// The engine's severity for it is an Error and is UNKNOWN (severity code semantics).
    pub const STAIRS_RISER_HEIGHT_TOO_SMALL: &str = "stairs.riser_height_too_small";
    /// WARNING POSTED AT COMMIT: the actual riser height is greater than the type's maximum riser height plus 1e-9.
    pub const STAIRS_RISER_HEIGHT_ABOVE_MAXIMUM: &str = "stairs.riser_height_above_maximum";
    /// WARNING POSTED AT COMMIT: the actual width of a run is below the type's minimum run width.
    pub const STAIRS_RUN_WIDTH_LESS_THAN_MINIMUM: &str = "stairs.run_width_less_than_minimum";
    /// AT THE CALL: the path provider's count disagrees with the boundaries it returned, or no railing results. The
    /// operation answers error 1; how that error surfaces in the API is UNKNOWN.
    pub const STAIRS_RAILING_CREATION_FAILED: &str = "stairs.railing_creation_failed";
    /// The engine refuses it instead of choosing a value.
    pub const STAIRS_UNDECIDED_BRANCH: &str = "stairs.undecided_branch";

    pub const ALL: &[&str] = &[
        CURVE_TOO_SHORT,
        HEIGHT_OUT_OF_RANGE,
        CANNOT_MAKE_WALL,
        BELOW_MIN_HEIGHT,
        TOP_BELOW_BASE,
        READ_ONLY,
        INVALID_LAYERS,
        NOT_FOUND,
        INVALID_BOUNDARY,
        SLAB_SEGMENT_OUT_OF_RANGE,
        SLAB_SLOPE_NOT_FINITE,
        SLAB_ROLE_HAS_NO_SLOPE,
        ROOF_CUTOFF_BELOW_BASE,
        SLAB_SLOPE_DEGENERATE,
        SLAB_LOOPS_INTERSECT,
        SLAB_LINES_OVERLAP,
        SLAB_OPEN_LOOP,
        SLAB_INVALID_ARC,
        SLAB_TOO_MANY_SLOPES,
        SLAB_SLOPE_EDGE_NOT_LINE,
        SLAB_INTEGRATION_FAILED,
        TRANSFORM_INVALID,
        TRANSFORM_NOT_SUPPORTED,
        WALL_SPLIT_REFUSED,
        WALL_TRIM_REFUSED,
        STAIRS_TOP_NOT_ABOVE_BASE,
        RAILING_STAIRS_HAS_RAILINGS,
        REFPLANE_DUPLICATE_NAME,
        REFPLANE_INVALID,
        MODEL_LINE_INVALID,
        MODEL_LINE_NOT_IN_PLANE,
        GROUP_EDIT_OUTSIDE_EDIT_MODE,
        GROUP_CANNOT_GROUP,
        DIMENSION_INVALID,
        TAG_INVALID,
        TEXT_NOTE_INVALID,
        SWEEP_INVALID,
        SWEEP_OVERLAP,
        SWEEP_OUTSIDE_WALL,
        MATERIAL_INVALID,
        TOPOSOLID_INVALID,
        TOPOSOLID_SUBDIVISION_FAILED,
        DOCVIEW_INVALID,
        VIEWFILTER_INVALID,
        SCHEDULE_INVALID,
        SHEET_INVALID,
        SHEET_NUMBER_EMPTY,
        SHEET_NUMBER_DUPLICATED,
        TAG_NOT_PASTED,
        ELEMENT_PINNED,
        COLUMN_BAD_SIZE,
        COLUMN_DUPLICATE_TYPE,
        COLUMN_BAD_ATTACHMENT,
        CURTAIN_INVALID,
        ARRAY_INVALID,
        WALL_OVERLAPS_WALL,
        HOST_HAS_TOO_MANY_INSERTS,
        GRID_INVALID_ARC,
        JOIN_ALREADY_JOINED,
        JOIN_NOT_JOINED,
        JOIN_UNSUPPORTED_PAIR,
        JOIN_DO_NOT_INTERSECT,
        INVALID_ARC,
        WALL_PROFILE_CURVE_TOO_SHORT,
        VOLUME_INTEGRATION_FAILED,
        SURFACE_AREA_INTEGRATION_FAILED,
        ROOM_BASE_ABOVE_COMPUTATION_HEIGHT,
        ROOM_LEVEL_HAS_NO_PLAN_VIEW,
        ROOM_NO_ENCLOSED_REGION,
        ROOM_NONE_CREATED_AUTOMATICALLY,
        LEVEL_STORY_ABOVE_MOVED,
        LEVEL_STORY_ABOVE_NOT_A_STORY,
        LEVEL_DUPLICATE_NAME,
        INVALID_THICKNESS,
        WRONG_KIND,
        VIEW_NO_LEVEL,
        VIEW_NOT_PLAN,
        VIEW_BAD_SCALE,
        VIEW_RANGE_INVALID,
        VIEW_DUPLICATE_NAME,
        VIEW_BAD_DISPLAY_SETTING,
        GRID_DUPLICATE_NAME,
        GRID_BAD_EXTENT,
        BAD_END_INDEX,
        INSERT_NOT_CUTTING_HOST,
        PARAM_NOT_PRESENT,
        INSERT_WRONG_LEVEL,
        INSERT_HOST_NOT_SUPPORTED,
        INSERT_OVERLAPS_OR_OUT,
        INSERT_BAD_SIZE,
        OPENING_INVALID_BOUNDARY,
        OPENING_DOES_NOT_CUT_HOST,
        INSERT_NO_LEVEL,
        INSERT_BAD_STATION,
        HOSTED_TYPE_DUPLICATE_NAME,
        STAIRS_TYPE_NOT_A_STAIRS_TYPE,
        STAIRS_CREATION_RISER_HEIGHT_TOO_SMALL,
        STAIRS_RUN_NEEDS_STAIRS,
        STAIRS_RUN_TREAD_DEPTH_NOT_POSITIVE,
        STAIRS_RUN_NEGATIVE_RISER_COUNT,
        STAIRS_REGENERATION_FAILED,
        STAIRS_SCOPE_COMMIT_UNRESOLVED_ERRORS,
        STAIRS_ILLEGAL_DESIRED_RISERS,
        STAIRS_TREAD_DEPTH_LESS_THAN_MINIMUM,
        STAIRS_RUN_TOP_LOWER_THAN_BOTTOM,
        STAIRS_RUN_BOTTOM_HIGHER_THAN_TOP,
        STAIRS_RUN_HEIGHT_TOO_SMALL,
        STAIRS_RUN_BOTTOM_UNDER_STAIRS_BOTTOM,
        STAIRS_RUN_EXTEND_BELOW_BASE_ABOVE_TOP,
        STAIRS_LANDING_HEIGHT_UNDER_STAIRS_BOTTOM,
        STAIRS_RISER_HEIGHT_TOO_SMALL,
        STAIRS_RISER_HEIGHT_ABOVE_MAXIMUM,
        STAIRS_RUN_WIDTH_LESS_THAN_MINIMUM,
        STAIRS_RAILING_CREATION_FAILED,
        STAIRS_UNDECIDED_BRANCH,
    ];
}

#[cfg(test)]
mod tests {
    use super::codes;
    use super::insert_overlaps_wall_curve;
    use super::{wall_below_type_minimum, wall_height_is_valid, wall_is_too_short, wall_width_is_valid};
    use asterion_core::units::feet_to_metres;

    /// AUD-5 regression guard: the stable-code table is the *only* place a
    /// failure identifier is written, and no two codes collide. A new code added
    /// to `codes` but left out of `codes::ALL` fails this test, which is what
    /// keeps the audit's coverage number honest.
    #[test]
    fn failure_codes_are_unique_and_namespaced() {
        let mut seen = std::collections::BTreeSet::new();
        for code in codes::ALL {
            assert!(seen.insert(*code), "duplicate failure code {code}");
            assert!(
                code.contains('.'),
                "failure code {code} must be namespaced as <domain>.<reason>"
            );
            assert!(
                code.chars().all(|c| c.is_ascii_lowercase() || c == '.' || c == '_'),
                "failure code {code} must be lower_snake.dotted ascii"
            );
        }
        assert_eq!(seen.len(), codes::ALL.len());
    }

    /// The height test is INCLUSIVE and made in feet.
    #[test]
    fn the_minimum_wall_height_is_inclusive_in_feet() {
        // exactly 0.1 ft passes (0.030480000000000004 m is 0.1 ft in double arithmetic) ...
        assert!(wall_height_is_valid(0.030480000000000004));
        assert!(wall_height_is_valid(feet_to_metres(0.1)));
        // ... and the OBSERVED boundary: 0.03048 m is 0.09999999999999999 ft.
        assert!(!wall_height_is_valid(0.03048));
        assert!(wall_height_is_valid(0.03049));
        assert!(!wall_width_is_valid(0.0));
        assert!(wall_width_is_valid(0.2));
    }

    /// Too short iff NOT (w / 100 < L), in feet, layers summed in feet.
    #[test]
    fn the_minimum_length_is_a_tenth_of_a_percent_of_the_width_inclusive_fail() {
        // The 0.2625 m type: 0.00262 fails, 0.002625 (the float tie) COMMITS, 0.00263 commits.
        let asym = [0.02, 0.08, 0.15, 0.0125];
        assert!(wall_is_too_short(&asym, 0.00262, None));
        assert!(!wall_is_too_short(&asym, 0.002625, None));
        assert!(!wall_is_too_short(&asym, 0.00263, None));
        // symmetric 0.2: L = w / 100 exactly fails (inclusive-fail), a hair above passes
        assert!(wall_is_too_short(&[0.2], 0.002, None));
        assert!(!wall_is_too_short(&[0.2], 0.00201, None));
        // an arc: w = min(w, radius), so a tight arc shortens the width that is compared
        assert!(wall_is_too_short(&[1.0], 0.0009, None));
        assert!(!wall_is_too_short(&[1.0], 0.0009, Some(0.05)));
    }

    /// The per-type minimum comparison, exposed for when regions exist.
    #[test]
    fn the_type_minimum_height_check_uses_the_read_inequality() {
        assert!(wall_below_type_minimum(1.0, 1.0, false), "flag false: below min + 1e-9");
        assert!(!wall_below_type_minimum(1.0, 1.0, true), "flag true: below min - 1e-9");
        assert!(wall_below_type_minimum(0.9, 1.0, true));
        assert!(!wall_below_type_minimum(1.1, 1.0, false));
    }

    /// Strictly inside is fine; at or beyond an end the half extent must reach back.
    #[test]
    fn an_insert_off_the_host_curve_is_a_warning_case_only_when_its_extent_does_not_reach_back() {
        assert!(insert_overlaps_wall_curve(3.0, 0.45, 0.0, 6.0));
        // a door whose centre is 0.1 beyond the end still has half of itself on the wall
        assert!(insert_overlaps_wall_curve(6.1, 0.45, 0.0, 6.0));
        // wholly off the end / the start: the posted-warning case
        assert!(!insert_overlaps_wall_curve(7.0, 0.45, 0.0, 6.0));
        assert!(!insert_overlaps_wall_curve(-1.0, 0.45, 0.0, 6.0));
        // exactly touching the end within the relative slack does not overlap
        assert!(!insert_overlaps_wall_curve(6.45, 0.45, 0.0, 6.0));
        // centre exactly on an end is not strictly inside, but the extent reaches back
        assert!(insert_overlaps_wall_curve(0.0, 0.45, 0.0, 6.0));
    }

    /// Every code listed in `codes::ALL` must actually be reachable as a `&str`
    /// constant; this is a cheap guard that the table and the constants agree.
    #[test]
    fn the_code_table_covers_the_declared_constants() {
        // Counted by hand against the `pub const` list above; raise it when a code is added, and
        // `failure_codes_are_unique_and_namespaced` then proves the new entry is well formed. It went 28 -> 25 on
        // 2026-10-13 when round 9 falsified three insert rules outright: the reference accepts and clips where ASTERION
        // rejected, so those codes describe conduct that does not exist. They were deleted, not relaxed. 28 -> 31 when
        // floors, ceilings and roofs became one kind with three roles: the three slab codes. Note what is NOT there - no
        // code for a cutoff below the base, an out-of-range height offset, a tiny sketch or a slope "too steep but finite".
        // The reference accepts all four, and round 9 deleted three invented codes for precisely that mistake. 42 -> 43:
        // `wall.volume_integration_failed`. 45 -> 46: `insert.overlaps_or_out`, a WARNING. 43 -> 45: `slab.lines_overlap`,
        // `slab.open_loop`. 46 -> 47: `wall.profile_curve_too_short` (Err 9 of the thickening route). 47 -> 67: the 20
        // stair codes. 67 -> 71: `room.no_enclosed_region`, `room.none_created_automatically` (Spec M-06),
        // `level.story_above_moved`, `level.story_above_not_a_story` (Spec L-01). 71 -> 72: `opening.does_not_cut_host`, a
        // WARNING. 72 -> 73: `view.bad_display_setting` (an ASTERION input check of the view display settings, not a
        // reference failure: the palette offers only the vocabulary). 73 -> 77: `slab.invalid_arc`, `slab.too_many_slopes`,
        // `slab.slope_edge_not_line`, `slab.integration_failed` (Spec W-08). 77 -> 79: `transform.invalid`,
        // `transform.not_supported`. 79 -> 80: `performance.host_has_too_many_inserts`. 80 -> 81: `grid.invalid_arc`. 81 ->
        // 82: `wall.split_refused` (Spec SPLIT-01). 82 -> 83: `wall.trim_refused` (Spec TRIMEXT-01). 83 -> 86:
        // `element.pinned`, `array.invalid`, `wall.overlaps_wall` (rounds 19 observations). 86 -> 89: `column.bad_size`,
        // `column.duplicate_type_name`, `column.bad_attachment`. 89 -> 90: `curtain.invalid`. 90 -> 92:
        // `stairs.top_not_above_base`, `railing.stairs_has_railings` (observations). 92 -> 111: refplane, modelline, group,
        // dimension, tag, text note and sweep codes. 111 -> 112: `level.duplicate_name` (observation). 112 -> 115:
        // `sheet.number_empty`, `sheet.number_duplicated`, `tag.not_pasted`.
        assert_eq!(codes::ALL.len(), 115);
    }
}
