# ASTERION BIM alpha - testing guide

This guide says what you can try in the first alpha build (`0.1.0-alpha.1`) and what you should not expect to work.
It is a snapshot of 2026-10-20. If something here and the application disagree, the application is right: please
[open an issue](https://github.com/Asterion-OpenDesign/Asterion-bim/issues) and tell us.

> **Alpha.** Features are incomplete, files may change format between builds, and crashes are possible. Save often and do not use it
> for real project deliverables.

## 1. Install

1. Download `AsterionBim-0.1.0-setup.exe` from the [latest release](https://github.com/Asterion-OpenDesign/Asterion-bim/releases/latest).
2. Check its SHA-256 (listed in the release notes): `Get-FileHash .\AsterionBim-0.1.0-setup.exe -Algorithm SHA256`.
3. Run it. The installer is **unsigned**, so Windows SmartScreen asks for confirmation ("More info" > "Run anyway").
4. Requirements: Windows 11, 64-bit, a GPU that supports DirectX 12 or Vulkan (there is no software fallback in this alpha: without one the window opens and says why). Nothing else needs to be installed.
5. Uninstall from Windows "Installed apps". On a machine that had earlier test builds, the Start-menu shortcut can stay behind.

## 2. The 10-minute tour

File > New, then with the real tools: draw four walls and an arc wall; place a door and a window; add a floor, a ceiling and a roof;
create a room; place a column; add a curtain wall; add a stair with a railing; add a level and a grid; add a dimension, a tag and a
text note; make a section and an elevation; open the 3D view; make a schedule of walls; save the project and open it again.
Every count should come back identical after reopening.

The same script runs as an automatic self-check: `asterion-shell.exe --selftest-demo` drives the real window and prints one `PASS` or
`FAIL` line per step (about 20 seconds, off-screen).

## 3. What you can test, by ribbon tab

### Architecture tab
- **Build:** Wall (straight and arc), Door, Window, Column, Roof (footprint), Ceiling, Floor, Curtain Grid, Mullion.
- **Circulation:** Railing, Stair. Stairs and railings are placed with correct counts, but **have no 3D body yet**: you see the run path in plan only.
- **Model:** Model Line, Model Group.
- **Room & Area:** Room, Room Separator, Tag Room.
- **Opening:** By Face, Shaft, Wall, Vertical.
- **Datum:** Level, Grid, Reference Plane.
- *Disabled with a reason in the tooltip:* Component, Curtain System, Ramp, Model Text, Area and area tools, Dormer, Reference Line, Work Plane tools.

### Modify tab
- **Properties:** Properties palette, Type Properties.
- **Clipboard:** Copy, Paste, Match Type.
- **Geometry:** Join, Wall Joins, Demolish, Paint.
- **Modify:** Move, Copy, Rotate, Scale, Mirror (pick axis and draw axis), Offset, Array, Trim / Extend (corner, single, multiple), Split Element, Pin, Unpin, Delete.
- **Measure and Create:** Measure Between Two References, Aligned Dimension, Create Group, Create Similar, Hide in View.
- *Disabled with a reason:* Align, Cut, Cut geometry, Cope, Repeat, Split with Gap, Displace Elements, Override Graphics in View, Linework, Create Assembly / Parts.

### Annotate tab
- Aligned and Linear dimensions, Text, Find / Replace, Tag by Category, Tag All.
- Most other annotation kinds are disabled: angular, radial, diameter and arc-length dimensions, detail lines and regions, legends, symbols, keynotes.
- Spot elevations are not verified in this build: try them and tell us what happens.

### View tab
- **Create:** 3D View, Section, Callout, Elevation, Floor Plan, Plan Region, Drafting View, Duplicate View, Schedules.
- **Graphics:** View Templates, Visibility / Graphics, Filters, Thin Lines.
- **Sheet Composition:** Sheet, Place View, Revisions.
- **Windows:** Switch Windows, Close Inactive, User Interface.
- *Disabled with a reason:* Render, Camera, Legends, Scope Box, Hidden-line display, Title Block tool, Matchline.

## 4. Selection, properties and types

- **Selection** works in plan, section, elevation, callout and 3D views: click, hover pre-selection, `Tab` to cycle through the elements behind, `Ctrl` / `Shift` to add or remove, window selection (left to right) and crossing selection (right to left). In 3D, window / crossing selection ignores occlusion.
- **Properties palette** follows the selection and edits in place for: walls, doors, windows, floors, ceilings, roofs, rooms, columns, curtain walls, grids, levels and stairs. With nothing selected it shows the properties of the active view (scale, detail level, visual style, crop, and so on). Values the engine cannot change stay read-only with the reason in the tooltip.
- **Types:** Duplicate, Rename and Edit Type work for wall, curtain wall, floor, ceiling, roof, door, window, column and stair types. Railing types have no properties page yet.
- **Project Browser:** one node per view, double-click opens it, the active view is bold, the context menu works or is disabled with a reason.
- Undo / Redo keep the selection and the palette values in step.

## 5. What not to expect

- Stairs and railings are not drawn in 3D or in sections.
- Doors and windows show as openings, without the geometry of the family.
- Sections and elevations do not draw rooms, grids, tags, dimensions or text, and there is no hidden-line removal or fills.
- No MEP, structure, steel, precast, analysis or collaboration. Architecture only.
- No IFC, DWG, DXF or PDF import or export yet. The native `.asterion` format may change between alpha builds.
- Viewports on a sheet cannot be dragged yet; there are no selection filters.
- Windows only.

## 6. Reporting a problem

Open an issue with: the build (shown in the window title), what you did, what you expected, what you saw, and a screenshot if it helps.
Please do not attach files you have no right to share, and never attach models that belong to a client or employer.

See the [README](../../README.md) for the measured numbers and the legal position.
