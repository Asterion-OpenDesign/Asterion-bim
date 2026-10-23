# Engine rules

Behavioural statements that the engine crates cite as `Spec <id>` in their comments and tests. Each statement says in plain
words what the engine does, for the case it names. It is the specification of that rule; the code and its tests are the
implementation. A statement is listed here only if the engine cites it.

How each statement was established is not part of this document. What the project does and does not claim about the
origin of its code is stated in [PROVENANCE_POLICY.md](../PROVENANCE_POLICY.md) and in [LEDGER.md](../../provenance/LEDGER.md).
Where a rule is an engine choice rather than a reference behaviour, the comment at the code says so (`ENGINE CHOICE`), and
behaviour that has been observed or only assumed is labelled `OBSERVED`, `ASSUMED` or `DOCUMENTED` there.

Open questions are listed in [open-questions.md](../open-questions.md).

59 rules.

## Spec BRW-01

Project Browser view groups are labelled with the view family's plural resource string, plus ' (type name)' when the view type's name differs from the family's singular name, and are ordered by a fixed per-family sort number (Structural Plans, Floor Plans, Ceiling Plans, 3D Views, Elevations, Sections, Detail Views, Renderings, Drafting Views, Walkthroughs, Area Plans, then Schedules, Reports, Sheets, Legends)

What the code says under this id:

* The Project Browser's view groups (Spec BRW-01). (`asterion-bim/src/viewing/browser.rs`)
* Spec BRW-01 (the reference product, the static view-family map and): every view family has a singular name, a plural browser name and a sort number. (`asterion-bim/src/viewing/browser.rs`)
* Spec BRW-01: the families of the table, as read (key, singular, plural, sort number). (`asterion-bim/src/viewing/browser.rs`)

Cited in the code 8 times.

## Spec C-02

Ceiling quantities follow the floor rules; 2 mm sketch commits with `Area` 0; bow-tie raises at the call

What the code says under this id:

* Spec C-02 not specified: "which operation turns the failure id into a refusal at the API call". (`asterion-bim/src/engine/spec.rs`)
* UNKNOWN: the parameter layer outside the reference product that turns the error into the value a user sees; the engine reports 0 (Spec C-02: a 2 mm sketch commits with Area 0 - that one is 4e-6 m2, far above the threshold, so it is a rounding observation and not this branch). (`asterion-bim/src/geometry/regen.rs`)
* Spec C-02: a self hit is a pair, the same failure as two loops that cross. (`asterion-bim/src/geometry/regen.rs`)

Cited in the code 4 times.

## Spec COL-01

When an architectural (or structural) column is a room-bounding element of a level

What the code says under this id:

* Rules specified since: Spec COL-01 (the reference product: an ARCHITECTURAL column is a room-bounding element type 3, a 2D bounding type only when the area settings enable it, and bounds a room only while its own Room Bounding parameter is on), Spec COL-02 (the reference product: a column is a structural-type-3 instance without a curve driver; (`asterion-bim/src/vertical/columns.rs`)
* Spec COL-01: the instance parameter `Room Bounding`; a column with it off never bounds a room. (`asterion-bim/src/vertical/columns.rs`)
* Spec COL-01 + (3): an architectural column is a 2D room-bounding element only when its own Room Bounding parameter is on AND the document's area settings enable the column classification. (`asterion-bim/src/vertical/columns.rs`)

Cited in the code 5 times.

## Spec COL-02

Column as a FamilyInstance: base / top level and offset parameters, Moves With Grids availability, and the column attachment record (offset, base or top, cut style, justification)

What the code says under this id:

* Rules specified since: Spec COL-01 (the reference product: an ARCHITECTURAL column is a room-bounding element type 3, a 2D bounding type only when the area settings enable it, and bounds a room only while its own Room Bounding parameter is on), Spec COL-02 (the reference product: a column is a structural-type-3 instance without a curve driver; (`asterion-bim/src/vertical/columns.rs`)
* Spec COL-02 (`ColumnAttachment`): the record holds the offset (a double), the base-or-top selector, the cut style and the justification; the numeric meaning of the cut style and justification enums is not specified, the engine keeps the names the API shows. (`asterion-bim/src/vertical/columns.rs`)
* Spec COL-02: the stored flag `Moves With Grids` (the parameter exists for the column categories only; an architectural column always has it). (`asterion-bim/src/vertical/columns.rs`)

Cited in the code 6 times.

## Spec CW-01

Curtain grid line placement: how a grid line of a curtain wall gets its curve from its direction flag and stored parameter, and the extent of that curve

What the code says under this id:

* Spec CW-01: the stored form of a grid line - one direction flag and ONE surface parameter on the reference face of its wall. (`asterion-bim/src/vertical/curtains.rs`)
* Lines: Spec CW-01 (layouts); panels: Spec CW-03; removed segments: Spec CW-05. (`asterion-bim/src/vertical/curtains.rs`)
* the API U line is HORIZONTAL: the flag "keeps v constant" (Spec CW-01: flag 1 fixes v and runs along u), its parameter the height (`asterion-bim/src/vertical/curtains.rs`)

Cited in the code 4 times.

## Spec CW-03

Curtain panels: how the curtain grid divides the driver face into panel faces and how each panel face is tagged

What the code says under this id:

* Lines: Spec CW-01 (layouts); panels: Spec CW-03; removed segments: Spec CW-05. (`asterion-bim/src/vertical/curtains.rs`)
* Spec CW-03: the panels are the CELLS of the lines of both directions, (n + 1)(m + 1) of them before removed segments merge two cells (Spec CW-05); in the order of the bounding box minimum (x, y, z) to the millimetre (OBSERVED) (`asterion-bim/src/vertical/curtains.rs`)

Cited in the code 3 times.

## Spec CW-04

Curtain mullion geometry: where a mullion lies, how its solid is made from the mullion type profile, and how its ends are trimmed or mitered

What the code says under this id:

* Spec CW-04: a mullion is the swept solid of its profile along the path curve of a grid edge, and an end that meets a neighbouring mullion is MITRED (one neighbour) or TRIMMED (two) to it. (`asterion-bim/src/vertical/curtains.rs`)
* the VERTICAL mullion runs the whole segment between the horizontal lines (it continues through a crossing) and the HORIZONTAL one is cut back to the face of the vertical mullion at each end (Spec CW-04 says only that the join pass decides which continues). (`asterion-bim/src/vertical/curtains.rs`)
* mullions: the automatic ones, then the added ones (Spec CW-04 gives no geometry: see [`MullionRec`]) (`asterion-bim/src/vertical/curtains.rs`)

Cited in the code 3 times.

## Spec CW-05

Removed curtain grid segments: how a grid line stores its removed segments, how they are normalised, what happens when a crossing line is added, and the empty-line warning

What the code says under this id:

* Spec CW-05: the reference product stores the removed stretch as the pair of crossing lines that bound it (an invalid id stands for the face border), ordered by parameter; (`asterion-bim/src/vertical/curtains.rs`)
* `span` is that stretch (lo, hi) along the line, so a crossing line added later leaves BOTH halves removed (Spec CW-05). (`asterion-bim/src/vertical/curtains.rs`)
* Spec CW-05: the empty-grid warning of the line - removed segments and no remaining segment. (`asterion-bim/src/vertical/curtains.rs`)

Cited in the code 8 times.

## Spec CW-06

Corner mullions and the curtain grid error poster: when an edge can carry a corner mullion, and how curtain grid errors are posted (mullion reference failures)

What the code says under this id:

* Spec CW-06: an edge of the grid can take a CORNER mullion only when both values the driver answers for it (the distances of the edge to the face border) are greater than -1e-9; an interior edge answers a negative value. (`asterion-bim/src/vertical/curtains.rs`)

Cited in the code 2 times.

## Spec F-02a

Floor `Area` is the larger of the summed top faces and the summed bottom faces

What the code says under this id:

* Source: the specification rules (Ceiling and Floor share), Spec F-02a (Floor Area), Spec F-02b (Floor Volume) and (a slope arrow or defining edge does not move the sketch loops; the solid comes). (`asterion-bim/src/engine/quantity.rs`)
* Area: Spec F-02a - the larger of the top and bottom face areas, each the exact area of the face (`asterion_geom::stokes`): for a flat slab the plan area, for an inclined one the plan area times `sqrt(1 + g^2)`. (`asterion-bim/src/geometry/regen.rs`)
* Quantities Spec F-02a, F-02b, F-02c, W-08 and slope-defining edges of floors and ceilings. (`asterion-bim/src/geometry/regen.rs`)

Cited in the code 3 times.

## Spec F-02b

Floor `Volume` is the volume of the solid (area x thickness only in a special case)

What the code says under this id:

* Source: the specification rules (Ceiling and Floor share), Spec F-02a (Floor Area), Spec F-02b (Floor Volume) and (a slope arrow or defining edge does not move the sketch loops; the solid comes). (`asterion-bim/src/engine/quantity.rs`)
* Volume: Spec F-02b - the volume of the solid (no shape-edit step, constant offset true), exact: the plan area times the vertical thickness. (`asterion-bim/src/geometry/regen.rs`)
* Quantities Spec F-02a, F-02b, F-02c, W-08 and slope-defining edges of floors and ceilings. (`asterion-bim/src/geometry/regen.rs`)

Cited in the code 3 times.

## Spec F-02c

Floor `Perimeter` is the plan perimeter and includes inner loops

What the code says under this id:

* Perimeter: Spec F-02c - the sum of the lengths of every loop, inner loops included, in the sketch plane. (`asterion-bim/src/geometry/regen.rs`)
* ASSUMED (Spec F-02c not specified: the per-curve rule): an arc's length is its exact `r |sweep|`. (`asterion-bim/src/geometry/regen.rs`)
* Every loop of the sketch takes part: material or hole is the parity of nesting, the Area and the Volume are those of the net region, the Perimeter includes the inner loops (Spec F-02c). (`asterion-bim/src/geometry/regen.rs`)

Cited in the code 6 times.

## Spec F-05a

Join Geometry between two elements is a stored relation (a cut-pair step), and IsCuttingElementInJoin is the existence of that step

What the code says under this id:

* Spec F-05a: the reference product stores the relation as ONE cut-pair step held by the CUT element and naming the CUTTER (plus a symmetric clean-pair step on both). (`asterion-bim/src/engine/model.rs`)
* # The relation (Spec F-05a) (`asterion-bim/src/walls/join_order.rs`)

Cited in the code 2 times.

## Spec F-06

The document default floor type is obtained get-or-create (separate default for foundation slabs)

What the code says under this id:

* The document's default FLOOR type, got or created (Spec F-06: -> : when the document has none it is CREATED; foundation slabs have a separate default, which the engine does not model). (`asterion-bim/src/engine/ops.rs`)
* UNKNOWN (Spec F-06; which existing type the getter picks and what it creates), questions. (`asterion-bim/src/engine/ops.rs`)
* Rules of specification rules that the engine already followed, pinned (Spec F-06). (`asterion-bim/src/lib.rs`)

Cited in the code 4 times.

## Spec FILTER-02

How a view hides an element through its filters: the first enabled filter with visibility off whose filter the element (or its owner chain, assembly, or host) passes hides it

What the code says under this id:

* Spec FILTER-02: a hiding entry is visibility off AND enabled; (`asterion-bim/src/documentation/docs_rows_tests.rs`)
* The graphics of a view as the reference product reads them: the template's when the view has one; a dependent view reads the hidden categories and the filters of its primary view (Spec FILTER-02). (`asterion-bim/src/viewing/docviews.rs`)

Cited in the code 2 times.

## Spec FILTER-03

Which graphic overrides a view applies to an element that several filters match: the matching filters are visited from the bottom of the list to the top and the set fields of the upper filter supersede

What the code says under this id:

* Tests of the documentation rules that follow specification rules (Spec FILTER-03). (`asterion-bim/src/documentation/docs_rows_tests.rs`)
* Spec FILTER-03-(4): visited from the last filter to the first; the second match replaces the first-visited settings; every further match supersedes field by field; the filter nearest the start has the last word (`asterion-bim/src/documentation/docs_rows_tests.rs`)
* (the stored fields) + Spec FILTER-03: background patterns and colours are copied when set; the visible flags always (`asterion-bim/src/documentation/docs_rows_tests.rs`)

Cited in the code 8 times.

## Spec G-01

Grid name cursor = last name assigned (create or rename)

What the code says under this id:

* Spec G-01: the cursor is a per-document record keyed by the grid name class and overwritten by every name assigned (create and rename alike). (`asterion-bim/src/engine/ops.rs`)
* UNKNOWN (Spec G-01): the derivation of the next name from the stored one (`Grid::next_name` is the harness's observation: a trailing number or letter run incremented), the text of the first name (IDS_FIRST_GRID_NAME, "1" here), and whether skips a name that is already taken (the engine refuses it). (`asterion-bim/src/engine/ops.rs`)
* UNKNOWN (Spec G-01): the duplicate-name refusal (delegates to a operation that was not specified, so the unique-name rule stays the engine's OBSERVED / DOCUMENTED one), and the record's first-write condition (the cursor is written only when the document already has a stored record). (`asterion-bim/src/engine/ops.rs`)

Cited in the code 5 times.

## Spec G-02

Default grid bubbles: End1 only; bubble visibility per view

What the code says under this id:

* Spec G-02: the default lives on the grid TYPE, not the grid (initialises "bubble at end 1" to false and "bubble at end 2" to true. (`asterion-bim/src/engine/model.rs`)
* OBSERVED (five grids) and Spec G-02: a new grid has no bubble flag of its own; the default comes from the grid TYPE (bubble at end 1 false, at end 2 true), so a new grid shows its bubble at the second end only ([`Grid::DEFAULT_BUBBLE`]. (`asterion-bim/src/engine/ops.rs`)
* UNKNOWN (Spec G-02): the default grid type a project TEMPLATE carries is persisted data - the code default applies only to a document without a default grid type). (`asterion-bim/src/engine/ops.rs`)

Cited in the code 13 times.

## Spec L-01

Level move regenerates dependents; delete cascades; "top below base" fails at commit

What the code says under this id:

* levels - Spec L-01 (literal 1) (`asterion-bim/src/engine/failures.rs`)
* Spec L-01: it is validated at regeneration of the level and cleared when it names a level that is not above this one or is not a building story. (`asterion-bim/src/engine/model.rs`)
* Which UI path writes it is UNKNOWN (Spec L-01). (`asterion-bim/src/engine/model.rs`)

Cited in the code 23 times.

## Spec L-02

Creating a Level element does not create a plan view; a plan view is a separate element found by association

What the code says under this id:

* Spec L-02: creating a Level creates no plan view, and a new level has no story above and is a building story (Spec L-01: the flag is true at construction). (`asterion-bim/src/engine/ops.rs`)

Cited in the code 1 time.

## Spec M-01

Room `Area` / `Perimeter` at the Finish faces; Center boundary on the centrelines

What the code says under this id:

* Spec M-01, M-01b: the Finish boundary is each room-bounding vertical wall's path curve (its centreline of the total width, so it does NOT depend on the Location Line parameter) offset by half the total wall width toward the room; the Center boundary is the path curves themselves. (`asterion-bim/src/geometry/regen.rs`)
* UNKNOWN (Spec M-01): the two core locations (CoreCenter, CoreBoundary) are not modelled; the sign convention of the offset; the corner cleaning, so the boundary here is the hole of the union of the wall bodies (equal for single-layer rectangular rooms - the only case corroborated); the non-vertical walls (wall-shape path); and the rejection of a loop that is not continuous within the short-curve tolerance. (`asterion-bim/src/geometry/regen.rs`)

Cited in the code 1 time.

## Spec M-01b

A Finish room-boundary segment of a vertical wall is NOT the wall's finish-face geometry looked up through the side-face id: it is the wall driver's curve offset sideways...

What the code says under this id:

* Spec M-01, M-01b: the Finish boundary is each room-bounding vertical wall's path curve (its centreline of the total width, so it does NOT depend on the Location Line parameter) offset by half the total wall width toward the room; the Center boundary is the path curves themselves. (`asterion-bim/src/geometry/regen.rs`)

Cited in the code 1 time.

## Spec M-02

`Volume` = Area x (upper-limit elevation + limit offset - base offset); gated by a project switch (ON by default)

What the code says under this id:

* Spec M-02: a room's upper limit that names an element that is missing or is not a Level reads as the room's own base level - the fall-back written just above. (`asterion-bim/src/engine/ops.rs`)
* Spec M-02: the Volume is that of the room's own prism (boundary loops extruded by the stored height = top elevation - bottom elevation); when the height is not above 1e-9 the geometry step creates no geometry and Volume reads 0 - it is never negative. (`asterion-bim/src/geometry/regen.rs`)
* Spec M-02: with volumes computed the prism is trimmed by the room-bounding slabs (`crate::room_trim`) (`asterion-bim/src/geometry/regen.rs`)

Cited in the code 6 times.

## Spec M-03

Enclosure: placed room recomputed on wall change; bounding flag; separation lines; doors do not bound

What the code says under this id:

* The level's room computation elevation offset, in metres: the height above the level at which the plane that decides what bounds a room is taken (Spec M-03: z = level elevation + computation offset; (`asterion-bim/src/geometry/regen.rs`)
* Whether a wall bounds the rooms of `level` (Spec M-03, wall branch). (`asterion-bim/src/geometry/regen.rs`)
* Where the plane is clearly inside or clearly outside the span the rule decides; within the vertex-tolerance band of a face the engine keeps its previous test (the wall is based on that level): UNKNOWN (Spec M-03) - the rule says "inside ... (`asterion-bim/src/geometry/regen.rs`)

Cited in the code 9 times.

## Spec M-04

Redundant rooms: one keeps the area, the rest report 0 (the older one)

What the code says under this id:

* Spec M-04: rooms that share one enclosed circuit form a group ordered by ElementId; (`asterion-bim/src/geometry/regen.rs`)
* UNKNOWN (Spec M-04): what makes two rooms share a Space beyond one circuit (design options, phases), and the phase / level / design-option skip conditions of the group loop. (`asterion-bim/src/geometry/regen.rs`)
* The compute-room-volumes switch gates the value (`computing`); the redundant case reaches here with `area` 0 (Spec M-04). and the value of the switch in a shipped template (file data): ON is the harness's observation, the code default of a document with no area settings is OFF. (`asterion-bim/src/geometry/regen.rs`)

Cited in the code 6 times.

## Spec M-05

Room number is a cursor (delete 101 -> next 102); `Name` reads "name number"

What the code says under this id:

* Spec M-05 says only where the cursor lives, not how it advances: no operation of the reference product writes a room's Number; the UI placement path copies a Number remembered per category in the document's symbol-id manager (a parameter value, not a counter field of the room) and writes the placed element's value back to it after a placement. (`asterion-bim/src/engine/ops.rs`)

Cited in the code 2 times.

## Spec M-06

`NewRooms2` needs a plan view; not idempotent (creates redundant rooms)

What the code says under this id:

* rooms - Spec M-07 (literal 2, resolution type 6), Spec M-06 (literal 1) (`asterion-bim/src/engine/failures.rs`)
* The batch room creator of the placement command (Spec M-06). (`asterion-bim/src/engine/ops.rs`)
* The batch room creator created no room (Spec M-06: a failure of severity code 1 is posted). (`asterion-bim/src/engine/spec.rs`)

Cited in the code 6 times.

## Spec M-07

Base offset above the computation height refused at commit

What the code says under this id:

* rooms - Spec M-07 (literal 2, resolution type 6), Spec M-06 (literal 1) (`asterion-bim/src/engine/failures.rs`)
* Spec M-07: `base offset > computation offset + 1e-9`, in feet (internal units). (`asterion-bim/src/engine/ops.rs`)
* Spec M-07: the failure is posted by the regeneration step of the room (`RoomElemValidHeightStep`), not by the setter, for a placed room whose base offset is greater than the level's room computation offset plus 1e-9 (feet; both relative to the level elevation), offering the resolution "AdjustLimits"; severity code 2 (error). (`asterion-bim/src/engine/ops.rs`)

Cited in the code 10 times.

## Spec MIR-01

Mirroring a wall about a vertical plane reflects its location curve without swapping its ends, flips the wall (its exterior side) unless it is a host panel, flips the sides of its joins and swaps the materials of its two side faces

What the code says under this id:

* **Mirror of a wall (Spec MIR-01).** About a vertical plane the location curve is reflected WITHOUT swapping its ends (the start stays the start), the wall is flipped (its exterior side) unless it is a host panel, the sides of its joins are flipped and the materials of its two side faces are swapped. (`asterion-bim/src/engine/transform.rs`)
* Not specified (Spec MIR-01): non-vertical planes (the engine's mirror planes are always vertical) and a hosted instance's own mirror. (`asterion-bim/src/engine/transform.rs`)
* Spec MIR-01: the curve is reflected with no end swap and the wall is flipped. (`asterion-bim/src/engine/transform.rs`)

Cited in the code 6 times.

## Spec ML-03

GEOMETRY, KIND AND VALIDITY OF A MODEL / DETAIL CURVE ELEMENT (C-14 / D-06 templates model_curves/*/start, end, center, radius, length, curve_kind, class). (1) GEOMETRY....

What the code says under this id:

* Spec ML-03: the ends and the length of the element are those of its geometry curve in model coordinates, and a curve with none of the detail / view-specific / reference-line / by-points flags is the model-curve kind (1); the engine's model lines are straight, so their curve kind is a line. (`asterion-bim/src/levels_grids/datums.rs`)

Cited in the code 1 time.

## Spec P-01

Element id allocation: monotonic, never reused, unaffected by rollback

What the code says under this id:

* A loaded document raises its id counter above it (Spec P-01: the counter is raised to cover ids it did not hand out, never lowered). (`asterion-bim/src/engine/model.rs`)
* Written specification of the specification rules and Spec P-01 (id allocation): (`asterion-bim/src/engine/transform.rs`)
* **Copy is a different mechanism.** The ids are deep-copied: one sequential block of new ids is reserved and handed out in the order of the copy array (Spec P-01); every id stored inside a copied element is rewritten through the old -> new map, so a copied door whose wall is copied too lands on the copy of the wall. (`asterion-bim/src/engine/transform.rs`)

Cited in the code 11 times.

## Spec R-01a

Roof `Area` is the area of the actual faces of the roof (INCLINED, not projected): the larger of the two sides

What the code says under this id:

* Spec R-01a: the Area is the LARGER of the two sides' summed face areas (real, inclined faces). (`asterion-bim/src/slabs/roof.rs`)
* Spec R-01a: the larger of the two sides (`asterion-bim/src/slabs/roof.rs`)
* Spec R-01a: the Area is the larger side. On the clipped roof the underside (13.4164) is larger than the top faces that survive the cut; on an unclipped single slope both sides are the same inclined area. (`asterion-bim/src/slabs/roof.rs`)

Cited in the code 3 times.

## Spec R-01b2

How the footprint roof solid applies the roof type thickness: measured normal to the slope or vertically, and how the top and bottom shells are separated

What the code says under this id:

* # Chains of unequal slopes (Spec R-01b2, R-04b) (`asterion-bim/src/slabs/roof.rs`)
* How far the bottom lies below the edge's own plane (Spec R-01b2: the chain's smallest slope sets the vertical thickness of every curve of the chain, the steeper facets keep their perpendicular thickness). (`asterion-bim/src/slabs/roof.rs`)
* Spec R-01b2: the vertical separation of the two shells at a slope edge is `t / cos(a_min)`, where `a_min` is the SMALLEST slope angle of the edge's chain - the same for every curve of the chain (the plumb-cut eave type puts all of it on the top shell), so the top surface is the envelope of the edge planes lifted by that one distance. (`asterion-bim/src/slabs/roof.rs`)

Cited in the code 5 times.

## Spec R-01c

Roof `Volume` comes from the solid (`Volume = Area x t` only when unclipped)

What the code says under this id:

* Spec R-01c: the Volume is the solid's; `Area x t` only when the offset is not constant, which no footprint roof without a shape-edit step meets. (`asterion-bim/src/slabs/roof.rs`)
* The Area parameter is the area of the reference surface and the Volume parameter `Area x t` (Spec R-01c: constant thickness, non-constant offset), except with a VARIABLE LAYER, where the bottom stays flat at the nominal thickness under the level and the Volume is the solid's. (`asterion-bim/src/slabs/slab_shape.rs`)

Cited in the code 3 times.

## Spec R-02

Cut-off clips the SOLID with a horizontal plane (`Volume != Area x t`)

What the code says under this id:

* Spec R-02: the cut-off cuts the solid IN PLACE with a horizontal plane at the up-to level elevation plus its offset, and the volume comes from the cut solid, so it is not `Area x t`. (`asterion-bim/src/slabs/roof.rs`)

Cited in the code 1 time.

## Spec R-04b

Where two roof slopes of unequal angle meet in a footprint roof: how the hip / ridge line between two slope-defining edges is computed

What the code says under this id:

* # Chains of unequal slopes (Spec R-01b2, R-04b) (`asterion-bim/src/slabs/roof.rs`)
* **The join between facets of different slopes** is resolved in `asterion_bim::roof` (Spec R-01b2, R-04b; (`asterion-geom/src/roof.rs`)

Cited in the code 2 times.

## Spec R-05

Deleting the cut-off level: roof survives, cut-off becomes (base level, +1.0) - ONE sample

What the code says under this id:

* Spec R-05: deleting a roof's CUT-OFF (up-to) level does not delete the roof. (`asterion-bim/src/engine/ops.rs`)
* UNKNOWN (Spec R-05; branch 1 edge existence): whether the base step still has an edge with that history pair after the cut-off clip (a face-history question of the geometry boolean) is not modelled, so which branch a given roof takes is not decided. (`asterion-bim/src/engine/ops.rs`)
* UNKNOWN (Spec R-05): the branch for a roof with no computable outline (the up-to id becomes the constant, whose value was not specified, and the offset 0): the offset 0 is applied and the engine uses the roof's base level for the constant. (`asterion-bim/src/engine/ops.rs`)

Cited in the code 6 times.

## Spec S-01c

LANDING ELEVATION: a landing's base elevation is always snapped to a whole number of the stairs' actual riser heights above the stairs bottom: n = round half away from ze...

What the code says under this id:

* AT PARAMETER SET (Spec S-01c): a landing base elevation that rounds to no riser above the stairs bottom. (`asterion-bim/src/engine/spec.rs`)
* Spec S-01c: the absolute guard of the stair operations, in internal units (feet). (`asterion-bim/src/vertical/stairs.rs`)
* Spec S-01c: "stores the value only when it differs from the stored one by more than 1e-9 relative". (`asterion-bim/src/vertical/stairs.rs`)

Cited in the code 25 times.

## Spec S-01f

WHAT THE NATIVE STAIRS ELEMENT ANSWERS FOR THE COMPARED READ-OUTS (elevations, height, riser and tread data, stories, supports, type). (0) Binding: the managed layer Stai...

What the code says under this id:

* The number of treads of a run from its riser count (Spec S-01f: the risers, less one when it ends with a riser, plus one when it does not begin with one). (`asterion-bim/src/vertical/stair_solids.rs`)
* Spec S-01f (`asterion-bim/src/vertical/stair_solids.rs`)

Cited in the code 2 times.

## Spec S-01h

WHAT A STAIRS RUN TYPE ANSWERS (has risers / has treads / is monolithic, nosing length, riser and tread thickness, structural depth, total depth). Persisted fields of the...

What the code says under this id:

* Spec S-01h. (`asterion-bim/src/vertical/stair_import.rs`)

Cited in the code 1 time.

## Spec SCALE-01

What Scale does to a wall's location curve: the line is scaled about the picked origin keeping its middle point scaled and its length multiplied by the factor; a result shorter than the tolerance posts the 'too short' failure

What the code says under this id:

* Written specification of the specification rules Spec SPLIT-01, SCALE-01 (the reference product, the wall's split operation and). (`asterion-bim/src/engine/modify.rs`)
* # Scale (Spec SCALE-01) (`asterion-bim/src/engine/modify.rs`)
* A wall's location curve is scaled in the plan about the picked origin (Spec SCALE-01): a straight wall keeps its middle point scaled (`f x mid + (1 - f) x origin`) and its length multiplied by the factor, which is the uniform scaling of its two end points about the origin; a curved wall's curve is transformed by the uniform scaling about the origin (Spec SCALE-01: its three defining points are). (`asterion-bim/src/engine/modify.rs`)

Cited in the code 10 times.

## Spec SPLIT-01

What Split Element does to a straight wall: refusals, the two parts, what is carried to the new part

What the code says under this id:

* Written specification of the specification rules Spec SPLIT-01, SCALE-01 (the reference product, the wall's split operation and). (`asterion-bim/src/engine/modify.rs`)
* # Split (Spec SPLIT-01) (`asterion-bim/src/engine/modify.rs`)
* **Refusals.** The element must be a wall (Spec SPLIT-01: "error 1 when the element is not a wall" -> `WRONG_KIND`). (`asterion-bim/src/engine/modify.rs`)

Cited in the code 10 times.

## Spec STK-01

Stacked wall members: how the member heights, the variable member, the member boundaries and the member offsets of a stacked wall type are computed

What the code says under this id:

* Spec STK-01: the members are records bottom first (height, offset, member type, member index, flip); a member whose height is below -1e-9 is the VARIABLE member; the minimum height of the wall is the sum of the fixed heights; the boundary at the bottom of a member is the sum of the fixed heights below it, or, when the variable member lies below, the wall height less the heights of the members above. (`asterion-bim/src/walls/stacked.rs`)
* One member of a stacked wall type (Spec STK-01). (`asterion-bim/src/walls/stacked.rs`)
* Spec STK-01: the sum of the fixed heights and the index of the variable member (the last one found); (`asterion-bim/src/walls/stacked.rs`)

Cited in the code 8 times.

## Spec STK-02

Stacked wall type validity and minimum height

What the code says under this id:

* Spec STK-02: member indices are non-negative and distinct, every fixed height is at least 1e-9, exactly one member is variable, and the fixed heights add up to at most the type's sample height. (`asterion-bim/src/walls/stacked.rs`)
* The sample height of the type: the fixed heights may add up to at most this (Spec STK-02). (`asterion-bim/src/walls/stacked.rs`)
* Spec STK-02. (`asterion-bim/src/walls/stacked.rs`)

Cited in the code 5 times.

## Spec SWP-03

Overlapping sweeps and reveals on one wall, and sweeps at wall corners

What the code says under this id:

* Spec SWP-03: the pair is compared only when both are on the same side with the same vertical flag and NEITHER is a reveal - overlapping reveals, and a reveal over a sweep, are never reported (not measured: ENGINE CHOICE for the profile overlap, the square profile of the template types). (`asterion-bim/src/walls/sweeps.rs`)

Cited in the code 1 time.

## Spec T-02

A slab type change keeps the sketch and re-derives the body from the new type total layer width

What the code says under this id:

* ACCEPTANCE is the separate of the element (Spec T-02): for a Floor the candidate must pass the floor symbol test (a footing slab type the footing symbol test); for a Ceiling a compound ceiling type (a soffit-type rule applies to a roof soffit); anything else a generic symbol test. (`asterion-bim/src/engine/ops.rs`)
* Spec T-02: the same id is a no-op - no touch, hence no regeneration and no new post. (`asterion-bim/src/engine/ops.rs`)
* Regeneration consequences of the touched atom (Spec T-02: the dependency graph re-runs the steps that list it; a regeneration recomputes the roof cut-off): posted, not refused. (`asterion-bim/src/engine/ops.rs`)

Cited in the code 4 times.

## Spec TEMPDIM-02

The references a straight wall dimensions from when it is selected as a whole: its centre (or core centre) or its two side faces according to the document's dimension-sides preference, plus the end faces that are not joined, plus the corner references

What the code says under this id:

* Written specification of the specification rule Spec TEMPDIM-02 (the reference product wall reference provider): for a pick of the whole wall, (`asterion-bim/src/documentation/tempdim.rs`)
* The document settings Spec TEMPDIM-02 states. (`asterion-bim/src/documentation/tempdim.rs`)
* The references of straight wall `wall` selected as a whole, in the order of Spec TEMPDIM-02. (`asterion-bim/src/documentation/tempdim.rs`)

Cited in the code 4 times.

## Spec TOPO-03

Toposolid subdivisions and splits

What the code says under this id:

* Spec TOPO-03: a subdivision of a subdivision is refused (the host carries a host id), and the host level must be a Level; both before the sketch is made. (`asterion-bim/src/slabs/toposolid.rs`)
* Spec TOPO-03: a subdivision of a subdivision is refused with the reference product's text. (`asterion-bim/src/slabs/toposolid.rs`)

Cited in the code 2 times.

## Spec TRIMEXT-01

Which end Trim/Extend moves, how a candidate end result is decided from the intersection with the boundary, and which elements may be trimmed

What the code says under this id:

* # Trim / Extend to a boundary (Spec TRIMEXT-01) (`asterion-bim/src/engine/modify.rs`)
* **Which end moves (Spec TRIMEXT-01).** The end of the wall's curve that is FARTHER from the clicked point; the click selects the part to keep, the other end stays. (`asterion-bim/src/engine/modify.rs`)
* **Where it goes (Spec TRIMEXT-01).** To the parameter `t` of the intersection of the wall's curve with the boundary's curve (a wall's code 2: "the moved end goes to t"). (`asterion-bim/src/engine/modify.rs`)

Cited in the code 7 times.

## Spec V-01

Default metric plan view range (top 2.3, cut 1.2, bottom 0, depth 0) + fifth plane UnderlayBottom

What the code says under this id:

* Spec V-01: for a floor plan of a non-structural view family the five slots start at the associated level with offset 0, then cut = (4.0 ft | 1.2 m), top = (7.5 ft | 2.3 m), bottom and view depth stay 0; the pair member is chosen inside (UNKNOWN (Spec V-01): not specified, so the metric member is the harness's observation). (`asterion-bim/src/engine/model.rs`)
* Spec V-01: the default plan view range is filled when the plan view is constructed, by view class. (`asterion-bim/src/engine/ops.rs`)
* UNKNOWN (Spec V-01; open question): for a CEILING plan the same operation reads "top and view depth = the level above the associated level, offset 0; cut and bottom offsets 2.3 above the associated level", but the harness's ceiling plan reports the floor-plan values on the associated level, and runs after the constructor and may replace the range - so the ceiling plan keeps the floor defaults here. (`asterion-bim/src/engine/ops.rs`)

Cited in the code 5 times.

## Spec V-02

`Options.View` clips a cut element at the cut plane; classification cut / projection / beyond

What the code says under this id:

* Spec V-02 decides which of these a part is ([`crate::views::classify_plan_condition`]). (`asterion-bim/src/vertical/stair_solids.rs`)
* The range planes of a plan view as the classifier of Spec V-02 takes them; (`asterion-bim/src/vertical/stair_solids.rs`)
* The runs a plan view draws: a run whose steps appear in the view's range (Spec V-02, [`plan_linework`]) and, for a run the engine has no solid for (a spiral or sketched run, a monolithic one), the plan of its base level - the presentation the plan had before the stairs had a body. (`asterion-bim/src/vertical/stair_solids.rs`)

Cited in the code 23 times.

## Spec V-03

Datum visibility in a plan decided by the PRIMARY range; grid curve drawn reversed on the cut plane

What the code says under this id:

* Arc grids (Spec V-03, G-02). (`asterion-bim/src/engine/transform.rs`)
* Rows followed: (maximise box, default extents) (the model outline), Spec V-03 (a datum's visibility in a plan by the primary range; the plan curve of a straight grid runs reversed on the cut plane) and Spec G-02 (bubble per view, end 1 by default). (`asterion-bim/src/levels_grids/datum_extents.rs`)
* In a view the datum lies on one line (a grid in a plan: its line at the cut plane, Spec V-03; a level in a section or an elevation: the trace of the view plane at the level's elevation, running along the view's right direction). (`asterion-bim/src/levels_grids/datum_extents.rs`)

Cited in the code 16 times.

## Spec VIEWTYPE-01

THE NATIVE VIEW TYPE OF A VIEW AND WHY THE API NAMES (view_type: FloorPlan / AreaPlan / Section / Elevation / Detail; class: ViewPlan / ViewSection) ARE NOT A FUNCTION OF...

What the code says under this id:

* Spec VIEWTYPE-01: the third integer of the family table, so a floor plan and an area plan are both 1 and a section, an elevation and a detail view are all 3 - the API names are NOT a function of it: 1 floor, structural and area plans; (`asterion-bim/src/viewing/view_dup.rs`)

Cited in the code 1 time.

## Spec W-05a

At a wall join each wall end is bounded by a TARGET face chosen by the join type (butt: a side face of the neighbour; miter; square-off)

What the code says under this id:

* | row | what | here | |---|---|---| | | the join angle value and its flags, the integer-to-name map of join types | [`join_angle_flags`], [`WallJoinType`] | | Spec W-05a | which target the end of the first or second wall gets | [`first_or_second_wall_target`] | | | the third and later walls of a join | [`later_wall_target`] and friends | | Spec W-07c | the routing priority of a layer function | [`routing_priority`] | (`asterion-bim/src/walls/join_end.rs`)
* Spec W-05a: type 2 on the second wall is read as type 0. (`asterion-bim/src/walls/joins.rs`)
* Type 2 read for the first wall: Spec W-05a, a plane square to the wall through an intersection of the offset side curves, at outward offset `ext` from the location end. (`asterion-bim/src/walls/joins.rs`)

Cited in the code 14 times.

## Spec W-07c

Layer interlock at a wall join is driven by the layer function: every layer with width >= 1e-9 gets a routing priority (layer function 100 -> 0, function 200 treated as 1...

What the code says under this id:

* | row | what | here | |---|---|---| | | the join angle value and its flags, the integer-to-name map of join types | [`join_angle_flags`], [`WallJoinType`] | | Spec W-05a | which target the end of the first or second wall gets | [`first_or_second_wall_target`] | | | the third and later walls of a join | [`later_wall_target`] and friends | | Spec W-07c | the routing priority of a layer function | [`routing_priority`] | (`asterion-bim/src/walls/join_end.rs`)
* The routing priority of a layer (Spec W-07c), and whether the function code was one the operation does not know (a debug warning in the reference product). (`asterion-bim/src/walls/join_end.rs`)
* Spec W-07c: layer function 100 gives 0; function 200 is treated as 1; functions 1, 2, 3, 4, 5 give 5, 4, 3, 2, 1; (`asterion-bim/src/walls/join_end.rs`)

Cited in the code 7 times.

## Spec W-08

Arc wall: `Area` = convex face; topology 6/12/8; `Volume` deviates from analytic up to 2.27e-4

What the code says under this id:

* 73 -> 77: `slab.invalid_arc`, `slab.too_many_slopes`, `slab.slope_edge_not_line`, `slab.integration_failed` (Spec W-08). (`asterion-bim/src/engine/spec.rs`)
* Quantities Spec F-02a, F-02b, F-02c, W-08 and slope-defining edges of floors and ceilings. (`asterion-bim/src/geometry/regen.rs`)
* OBSERVED (j: a 0.3 line meeting a 0.2 arc at 22.6 degrees): the line ends on the OUTER cylinder of the arc (bbox max x 6.510681686 = the outer circle at the line's left face), the arc ends on the line's left face (y 180.15); the volumes 5.604896018 / 4.380416335 and the surface areas 43.910500753 / 48.620791457 of the two walls are reproduced to 3e-7 / 1e-10 by the Stokes sums of `asterion_geom::stokes` (Spec W-08). (`asterion-bim/src/walls/curved_ends.rs`)

Cited in the code 4 times.

## Spec W-10b

When the host wall curve changes (location line edit, end drag, move), the hosted instance is NOT re-hosted and its station is NOT recomputed: its transform is re-derived...

What the code says under this id:

* Spec W-10b: editing the host curve (location edit, end drag, move) neither re-hosts the instances it carries nor recomputes their station, and no station is clamped: each hosted instance keeps its stored station and travels with the new curve. (`asterion-bim/src/engine/ops.rs`)
* Spec W-10b says the instance "follows the new curve at the same stored station", which is what that does for a translation or a rotation. (`asterion-bim/src/engine/ops.rs`)
* The edit itself is applied; the instance keeps its host and its stored station (Spec W-10b). (`asterion-bim/src/engine/ops.rs`)

Cited in the code 9 times.

## Spec W-10g

Moving a host wall so that a hosted door/window is left past the new curve posts a failure of severity Error and the transaction is rolled back (lab harness round 9 step 27: Error, RolledBack)

What the code says under this id:

* Spec W-10g: an instance that no longer cuts its host wall is not refused at the edit call: the cut step of the instance posts the instance-not-cutting-anything failure for the commit (the step itself does not fail), see [`report_inserts_left_behind`]. (`asterion-bim/src/engine/ops.rs`)
* Spec W-10g: a door or window instance that no longer cuts its host wall is refused by the cut step of the instance on the wall, which posts the instance-not-cutting-anything failure. (`asterion-bim/src/engine/ops.rs`)
* The severity is [`asterion_core::Severity::Error`]: the failure definition is registered with the severity literal 2 (Spec W-10g) and literal 2 of the failure severity enumeration is Error; it is posted with the error call, never as a warning. (`asterion-bim/src/engine/ops.rs`)

Cited in the code 15 times.

## Spec W-11b

Edges and vertices of a wall after a rectangular opening: which Boolean options the opening step uses, the imprint / merge rule they switch on, and the resulting topology

What the code says under this id:

* Spec W-11b (the Boolean of the wall opening step, merge pass always on): the faces of the cut wall are the faces of the wall that survive plus the faces of the cutters (`cutter_face_count`). (`asterion-bim/src/geometry/regen.rs`)
* Spec W-11b, round 10: a 10 x 3 wall, openings made one at a time; the reference product reports 6, 10, 13, 16, 18, 18, 18, 21, 24 faces and 20 once the first one is deleted again. (`asterion-bim/src/geometry/regen.rs`)
* Spec W-11b: a door cuts its host; its notch adds the corner points of the net region on both faces (16 points for a 1.0 x 2.0 door at station 2.0) (`asterion-bim/src/walls/wall_vertices.rs`)

Cited in the code 5 times.
