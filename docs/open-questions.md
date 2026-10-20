# Open questions

What the engine does not know, or knows only by assumption. Nothing here is filled in with a guess: each item is either
left unimplemented, refused with a message, or implemented as a named engine choice. A question is closed by a measurement
or a public source, never by an opinion.

## Named in engine messages

These identifiers appear in the text the engine gives when it refuses or limits something (`OQ-...`).

* **OQ-ROOM-SECTION** (asterion-bim, limit): A room is collected by a section but the engine draws nothing of it there.
* **OQ-SLAB-ARC-LENGTH** (asterion-bim, assumed): Perimeter: exact arc length.
* **OQ-SLAB-SHAPE** (asterion-bim, engine choice): A sloped slab cannot be shape edited.
* **OQ-SLAB-SHAPE-BODY** (asterion-bim, limit): A shape-edited slab: the engine keeps its quantities, not its body.
* **OQ-SPLIT-ARC** (asterion-bim, limit): Arc walls are not split by the engine.
* **OQ-STAIR-GEOM** (asterion-bim, limit): The stairs module has no solid (treads, risers, stringers): only run location lines and quantities exist.
* **OQ-TOPOSOLID-BODY** (asterion-bim, limit): A toposolid with surface points has a triangulated top: the engine reports its quantities, not a body.
* **OQ-TRIMEXT-ARC** (asterion-bim, limit): An arc wall is not trimmed or extended by the engine.

## Marked UNKNOWN in the code

Grouped by source file; the text is the comment as it stands in the code.


### `asterion-bim/src/documentation/dims.rs`

* Not known: The style's units of a document that changed them.
* Not known: The unit formatting of the measured value, the alternate units and the label / equality fragments; the sentinel -1e30 reading `IDS_QUESTIONS` is not modelled.
* Not known: What the sketch solver does to the other references of an equal or locked dimension and the failure `CannotToggleDimensionEquality`: the engine stores the flag and moves nothing.

### `asterion-bim/src/documentation/docs.rs`

* Not known: Whether the comparison ignores case for the element path; the engine compares exactly as observed.
* Not known: The constructor's values live in the reference product).

### `asterion-bim/src/documentation/schedules.rs`

* Not known: The natural alphanumeric order of text and the order of values that could not be read; the engine compares text by code-unit order).

### `asterion-bim/src/documentation/sheets.rs`

* Not known: The generator and the seed text; the engine counts up from the number of sheets (OBSERVED: the first sheet reads "1").
* Not known: The placement of the box centre at the given point and the label.
* Not known: Which corner of the schedule table the point is.
* Not known: Which revisions a sheet shows besides the ones set on it (issued, additional, clouds) and the per-sheet order.

### `asterion-bim/src/engine/failures.rs`

* Not known: Which inserts counts (here: the wall's doors, windows and rectangular openings) and when the adviser runs its rules.

### `asterion-bim/src/engine/instance_params.rs`

* Not known: The default answer of the confirmation when no UI is attached: the engine answers yes, as measured.

### `asterion-bim/src/engine/model.rs`

* Not known: What API function the internal code 0 is - none of this engine's functions is mapped to it, so only `Membrane` (100) may be zero; structural deck (200) and non-rectangular regions are not represented by this engine's `Layer`.
* Not known: Not specified, so the metric member is the harness's observation).

### `asterion-bim/src/engine/modify.rs`

* Not known: How positions 1 and 2 place the members - the spacing equal to the vector for `Second` and the vector spread over the count for `Last` are the OBSERVED rule below.

### `asterion-bim/src/engine/ops.rs`

* Not known: Which UI path writes the parameter and which element ids the API accepts; the id is stored as given and validated, so an id that is not a level is cleared.
* Not known: Which atoms "the stairs elevation-span atoms" are - the rule does not say they are the levels' position atoms - and how the reference product derives the stairs bottom and top from the levels and the offsets (the engine uses the levels' elevations).
* Not known: The Stair parent overrides: whether the runs, landings and railings of a stairs travel with a moved base level is not specified, so only the regeneration of the stairs element itself is run here.
* Not known: The rule says the join at a control is SEPARATED when the squared displacement of the control (projected on the plane perpendicular to the driver normal) exceeds `(width / 2)^2 - 1e-9`, and kept (the neighbours re-trimmed by regeneration) otherwise.
* Not known: Whether the stored station is a raw curve parameter or a normalised one.
* Not known: The z reference of an opening corner ("curve z") and which edits re-run the corner-to-parameter derivation; the engine keeps the absolute elevation.
* Not known: What a registered failures processor does with the pending error (the default processor's module was not opened).
* Not known: That special category and the group-member rule: the engine refuses what it has no collection for.
* Not known: Whether the base step still has an edge with that history pair after the cut-off clip (a face-history question of the geometry boolean) is not modelled, so which branch a given roof takes is not decided.
* Not known: The branch for a roof with no computable outline (the up-to id becomes the constant, whose value was not specified, and the offset 0): the offset 0 is applied and the engine uses the roof's base level for the constant.
* Not known: The deletion of the BASE level of a roof, the same rule for a face roof and the thickness of a curtain roof (a unit-converted constant; the engine has no curtain roof).
* Not known: What deleting a level does to the stairs that name it as base or top level.
* Not known: The reference product's cascade for a stairs.
* Not known: The list of steps the dependency graph re-runs after the type atom is touched (it lives in the reference product / the reference product).
* Not known: The increment that turns the remembered Number into the next one (101 to 102), what a deleted room's number (the remembered set is held by the symbol-id manager, which only stores and looks it up - no operation of it changes a value - and the numeric-string incrementer has only the new-item-number operations as callers, so the 101 -> 102 step is not a parameter-set operation) does to the cursor, and whether API-created rooms go through that remembered set - the "last assigned + 1, starting at 1" rule here is the harness's observation and nothing else.
* Not known: The point of the transaction at which regeneration delivers it (commit versus an explicit regenerate), and the writer of the level's computation offset (written per level since round 36: `crate::datum_extents::computation_height`).
* Not known: For a CEILING plan the same operation reads "top and view depth = the level above the associated level, offset 0; cut and bottom offsets 2.3 above the associated level", but the harness's ceiling plan reports the floor-plan values on the associated level, and runs after the constructor and may replace the range - so the ceiling plan keeps the floor defaults here.
* Not known: The derivation of the next name from the stored one (`Grid::next_name` is the harness's observation: a trailing number or letter run incremented), the text of the first name (IDS_FIRST_GRID_NAME, "1" here), and whether skips a name that is already taken (the engine refuses it).
* Not known: The default grid type a project TEMPLATE carries is persisted data - the code default applies only to a document without a default grid type).
* Not known: The duplicate-name refusal (delegates to a operation that was not specified, so the unique-name rule stays the engine's OBSERVED / DOCUMENTED one), and the record's first-write condition (the cursor is written only when the document already has a stored record).
* Not known: Which view types count as "extent view types" for the override to apply, and how a grid without a per-view record appears in plan views of other classes.
* Not known: So no other edit does.
* Not known: The producers of that (tag, code) result - which geometric condition yields each code - so the arc rules below (`radius <= half thickness`, the thickened-profile curve too short) are the engine's earlier OBSERVED behaviour and the rule does not decide them; and bodies.
* Not known: The placement path read there stores the level passed in and refuses nothing, and derives the position from that level's elevation.
* Not known: Why an instance placed on another level stops cutting (the reference product predicates and the extent test were not specified).
* Not known: Why an instance on another level stops cutting.
* Not known: Raw versus normalised station, so the value is pinned only as "unchanged".
* Not known: The geometric extent test; the engine keeps its own threshold.
* Not known: The z reference of the corners; absolute z is kept.
* Not known: Whether a warning is posted there).

### `asterion-bim/src/engine/phases.rs`

* Not known: The trimming and case rules.
* Not known: The order of the four ids inside the guard.
* Not known: The Demolish tool itself (which view phase it uses, what it does with an element created in that phase).

### `asterion-bim/src/engine/spec.rs`

* Not known: The exact comparison.
* Not known: Which geometric condition yields each regeneration result code, so the arc-radius guard is not decided).
* Not known: What a wall joined to an element at either end does with the same condition (the rule lists that join among the exceptions of the post; the engine does not model the exception because the effect on the commit was not specified).
* Not known: The same operation for the footprint roof, which is the kind this engine models, so the guard here is the engine's own OBSERVED one.
* Not known: What a registered processor does with the error.
* Not known: The geometric extent test that decides "cuts nothing".
* Not known: Why an instance on another level stops cutting.
* Not known: (severity code semantics).

### `asterion-bim/src/engine/transform.rs`

* Not known: Recorded as questions (no value is guessed for them; the engine choice is named): * Which ids the per-class helpers add.

### `asterion-bim/src/engine/type_names.rs`

* Not known: The type-specific validator and which properties a copy carries per class; "trimmed" is read as both ends (the trimming operation is not transcribed in the rule).
* Not known: The message says "such as".

### `asterion-bim/src/engine/type_ops.rs`

* Not known: Which properties a copy carries per class: the engine copies every stored field of the kind.

### `asterion-bim/src/geometry/regen.rs`

* Not known: The two core locations (CoreCenter, CoreBoundary) are not modelled; the sign convention of the offset; the corner cleaning, so the boundary here is the hole of the union of the wall bodies (equal for single-layer rectangular rooms - the only case corroborated); the non-vertical walls (wall-shape path); and the rejection of a loop that is not continuous within the short-curve tolerance.
* Not known: The writer of the offset and the value shipped templates carry; the engine's levels hold no such field, and 0 is the value the harness reports for every level it measured (OBSERVED).
* Not known: The rule says "inside .
* Not known: What makes two rooms share a Space beyond one circuit (design options, phases), and the phase / level / design-option skip conditions of the group loop.
* Not known: The parameter layer outside the reference product that turns the error into the value a user sees; the engine reports 0 (Spec C-02: a 2 mm sketch commits with Area 0 - that one is 4e-6 m2, far above the threshold, so it is a rounding observation and not this branch).
* Not known: Instances are left out.
* Not known: The direction of the tolerance), i.e.

### `asterion-bim/src/geometry/sketch.rs`

* Not known: * `asterion_geom::loop_point` documents the choice taken.
* Not known: The envelope pre-test (loops with a computed envelope whose envelopes do not intersect are skipped by the reference); skipping only removes pairs that cannot touch, so it is not modelled.

### `asterion-bim/src/levels_grids/datum_extents.rs`

* Not known: Which operation asks for the nearest level of a pasted wall and with which elevation, and the levels of the reference product template, which the engine does not carry as elements: the copy of a wall therefore keeps its level.

### `asterion-bim/src/levels_grids/datums.rs`

* Not known: The creation operation the API uses for a model line.

### `asterion-bim/src/levels_grids/item_number.rs`

* Not known: The key class `NameEquivalenceClass` and the base string each caller passes: the engine keeps ONE cursor per kind of item (for grids [`crate::model::BimModel::last_grid_name`]) and falls back to "1" where the reference product stores the base string of its caller.

### `asterion-bim/src/vertical/stairs.rs`

* Not known: Each item is either an `Option` input that defaults to "not evaluated", an explicit argument, or a refusal with [`codes::STAIRS_UNDECIDED_BRANCH`].
* Not known: The default failures processor.
* Not known: What a registered failures processor does with the error.
* Not known: The base of the relative test.
* Not known: Which special case wins when both hold.
* Not known: The rule gives no other branch.
* Not known: Writer and initial value.
* Not known: Whose thickness and in which frame the elevation is taken; the caller supplies both, and `None` means the check is not evaluated.
* Not known: Which run elevation "above" compares (the bottom is used).
* Not known: And any common offset leaves the comparisons unchanged).
* Not known: The formula of that second branch is not evaluated.
* Not known: The minimum run height.
* Not known: The run/landing tread thickness of the instance check.
* Not known: Writer, meaning, initial value; `None` = never written.
* Not known: How the reference product derives the stairs bottom and top (base and top offsets); the levels' elevations are used.
* Not known: The relative order of the two steps is not specified.
* Not known: Whether the reference product posts one warning per run or per stairs.
* Not known: The tread depth a new stairs holds before "the actual tread depth is raised to the type minimum when it is smaller".
* Not known: How the entry point reports it; the engine's rejection channel is `Err`.
* Not known: The defaults of a new run.
* Not known: Which run type code assigns.
* Not known: Which failures count as "unresolved"; error severity is used.
* Not known: How a landing element is created (sketched and automatic landing creation are not specified).
* Not known: Which dependents the reference product's deletion algorithm removes for a stairs and which failures it posts.
* Not known: So the branch exists only when it is supplied.

### `asterion-bim/src/viewing/docviews.rs`

* Not known: (detail level and the default name inherited from the parent) and the order.

### `asterion-bim/src/viewing/views.rs`

* Not known: The actual vector d for plan views (only the sign of its z component enters), so "reversed in every plan view" stays the harness's observation.
* Not known: The bounds stored in the template file (only the defaults are read), and that a later scale edit does not apply the table.
* Not known: The cut plane being unlimited (the native reads a warning and an unspecified elevation) - it is treated as an unbounded end here; and the other view classes (section, elevation, 3D) take their range from the bounded space and are not modelled.
* Not known: The mode `m` of the element category (category lists not specified) matters to every other branch, which therefore stay the ASSUMED / OBSERVED rules below.
* Not known: What the helper collapses 5 to, and the per-type draw callbacks of conditions 3, 5 and 2.
* Not known: And therefore answered as [`PlanCondition::Unknown`]: * All three modes are evaluated and a value is returned only when they agree, so a result never depends on a mode the engine cannot know.

### `asterion-bim/src/walls/join_end.rs`

* Not known: The engine does NOT act on this flag.
* Not known: The helper that may flip it to the opposite side was not specified.
* Not known: The rule's legacy clause "(10 degrees and exactly 180 for step version 0)" is for steps loaded from old files; this engine creates only current steps.
* Not known: (the branch structure after the neighbour selection).
* Not known: Which API function name each of 1..5 is, so this engine's [`crate::LayerFunction`] is not mapped here.

### `asterion-bim/src/walls/join_order.rs`

* Not known: The three virtual predicates other DLLs answer (the "concrete" test at the reference product, the coping rank test, the stairs test at the reference product), the wall-sweep internals and the exact membership of "Floor-class" and "Toposolid-class".
* Not known: The membership beyond floors.
* Not known: The order of the list and where the predicate is applied.
* Not known: What the boolean means; only its equality with the processed wall's own value is used.
* Not known: The tolerance of the reference product's parallel test.
* Not known: Whether the builder is selected with the index taken before or after the removal.
* Not known: Whether runs counter-clockwise or clockwise.
* Not known: Its numeric value, so it is an input).
* Not known: A tolerance, none is applied); and finally the LOWER element id wins in both passes.
* Not known: The names of the values; only 0..3 are read).

### `asterion-bim/src/walls/joins.rs`

* Not known: As a product rule) is the *through* wall and extends by `h_q Â· tan(Î³/2)`; the later wall *butts* and trims by `h_q Â· cot(Î³/2)`.
* Not known: (the branch structure after the neighbour selection; the array order that needs to know which walls precede the wall is UNKNOWN).
* Not known: (the branch structure after the neighbour selection).

### `asterion-core/src/txn.rs`

* Not known: The default failures processor of a session, which decides what the user sees for a Warning or an Error.

### `asterion-geom/src/loop_point.rs`

* Not known: Never filled with a guess) * Here: the distance is the unsigned Euclidean one; a zero-length segment is skipped by [`pnt_on_polygon`]; the grown box is closed (a point exactly on it is not rejected by the envelope test, then the distance test decides).
* Not known: The reference operation's answer 1).

### `asterion-geom/src/wall_profile.rs`

* Not known: Never filled with a guess): * slanted and tapered walls (swept solid from a cross-section profile: not specified) and a wall with an elevation sketch (the generic extrusion builder).
