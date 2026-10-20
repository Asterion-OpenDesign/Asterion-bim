//! Every rule is OBSERVED through the public API and is an ENGINE CHOICE that fits the named observation; the question that
//! would validate it is named next to it. This module holds the stored state and the materials; the rest lives in
//! [`crate::toposolid`], [`crate::docviews`], [`crate::schedules`] and [`crate::sheets`].

use crate::model::BimModel;
use crate::spec::codes;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type Rgb = [u8; 3];

/// A material (`Material.Create`). OBSERVED defaults: colour black, transparency 0, shininess 64, smoothness 50, class and
/// category "Unassigned", no pattern, render appearance not used for shading.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub id: ElementId,
    pub name: String,
    pub color: Rgb,
    pub transparency: i32,
    pub shininess: i32,
    pub smoothness: i32,
    pub class: String,
    pub category: String,
    pub use_render_appearance: bool,
    pub surface_pattern: Option<String>,
    pub surface_color: Rgb,
    pub cut_pattern: Option<String>,
    pub cut_color: Rgb,
    pub appearance_asset: Option<String>,
}

/// The fill and line patterns of the template (OBSERVED `probe_documentation`; ENGINE CHOICE: the engine knows no pattern
/// beyond these).
pub const FILL_PATTERNS: [&str; 49] = ["100mm horizontal", "150 x 150 mm Squares", "250mm Squares", "50mm horizontal", "600 x 1200mm", "600 x 600mm", "<Solid fill>", "Aluminum", "Block 200x200", "Block 200x400", "Block 225x450", "Brick - US Standard", "Brick 75x225", "Brick 80x240 CSR", "Brick Soldier Course", "Brickwork", "Concrete", "Crosshatch", "Crosshatch 1.5mm", "Diagonal cross-hatch", "Diagonal crosshatch", "Diagonal crosshatch 1.5mm", "Diagonal down", "Diagonal down 1.5mm", "Diagonal up", "Diagonal up 1.5mm", "Diagonal up 3mm", "Earth", "Gypsum-Plaster", "Horizontal", "Horizontal 1.5mm", "Insulation - Rigid", "Masonry - Brick", "Masonry - Concrete  Block", "Plastic", "Plywood", "Sand", "Sand - Dense", "Shake 150mm", "Shingle", "Steel", "Triangles", "Vertical", "Vertical 1.5mm", "Wood - Finish", "Wood 1", "Wood 2", "Wood Board", "Wood Board Wide"];
pub const LINE_PATTERNS: [&str; 18] = ["Aligning Line", "Center", "Centre", "Dash", "Dash dot", "Dash dot dot", "Demolished", "Dot", "Dot 1mm", "Dot 2mm", "Double dash", "Grid Line", "Hidden", "Long dash", "Loose dash", "Overhead", "Rebar Cover Lines", "Triple dash"];

/// A fill pattern the document has (the solid fill is `<Solid fill>`).
pub fn fill_pattern_exists(name: &str) -> bool {
    FILL_PATTERNS.contains(&name)
}

/// A line pattern the document has (`solid` is always there).
pub fn line_pattern_exists(name: &str) -> bool {
    name == "solid" || LINE_PATTERNS.contains(&name)
}

/// The material a layer without one reads as (the template's default wall material; ENGINE CHOICE: only the name is known,
/// and the engine does not model the template's other 90 materials).
pub const DEFAULT_WALL_MATERIAL: &str = "Default Wall";

/// The NAMES are template data. ENGINE CHOICE: the engine holds no template types, so the names stand in until the document
/// has a default of its own.
pub const TEMPLATE_DEFAULT_DOOR_TYPE: &str = "0915 x 2134mm";
pub const TEMPLATE_DEFAULT_WINDOW_TYPE: &str = "0406 x 0610mm";

/// The name of the template's default type of a hosted kind.
pub fn template_default_type_name(kind: crate::model::HostedKind) -> &'static str {
    match kind {
        crate::model::HostedKind::Window => TEMPLATE_DEFAULT_WINDOW_TYPE,
        _ => TEMPLATE_DEFAULT_DOOR_TYPE,
    }
}

/// A painted face: `Document.Paint(wall, face, material)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Paint {
    pub wall: ElementId,
    /// `1` for the exterior face, `-1` for the interior one.
    pub side: i8,
    pub material: ElementId,
}

/// All the state of this slice (one field of `BimModel`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DocsModel {
    /// The `Comments` text of walls (the filters of a view read it; ENGINE CHOICE).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub comments: BTreeMap<ElementId, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub materials: BTreeMap<ElementId, Material>,
    /// The material of each layer of a wall type that has any, by type (`None`: the layer has none).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub layer_materials: BTreeMap<ElementId, Vec<Option<ElementId>>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paints: Vec<Paint>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub toposolids: BTreeMap<ElementId, crate::toposolid::Toposolid>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub views: BTreeMap<ElementId, crate::docviews::DocView>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub markers: BTreeMap<ElementId, crate::docviews::ElevationMarker>,
    /// A view without an entry has the defaults.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub graphics: BTreeMap<ElementId, crate::docviews::ViewGraphics>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub templates: BTreeMap<ElementId, crate::docviews::ViewTemplate>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub filters: BTreeMap<ElementId, crate::docviews::ViewFilter>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub schedules: BTreeMap<ElementId, crate::schedules::Schedule>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sheets: BTreeMap<ElementId, crate::sheets::Sheet>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub viewports: BTreeMap<ElementId, crate::sheets::Viewport>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub schedule_instances: BTreeMap<ElementId, crate::sheets::ScheduleInstance>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub revisions: BTreeMap<ElementId, crate::sheets::Revision>,
    /// The primary view of each dependent view (dependent -> primary, `crate::view_dup`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub primary_views: BTreeMap<ElementId, ElementId>,
    /// The order of the revisions when the dialog reordered them: the revisions listed first in this order, any other after
    /// them in id order. Empty = id order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revision_order: Vec<ElementId>,
    /// The phase filters made through the API.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub phase_filters: BTreeMap<ElementId, crate::phases::PhaseFilter>,
}

impl DocsModel {
    pub fn is_empty(&self) -> bool {
        *self == DocsModel::default()
    }

    fn ids(&self) -> Vec<ElementId> {
        let mut v: Vec<ElementId> = Vec::new();
        v.extend(self.materials.keys());
        v.extend(self.toposolids.keys());
        v.extend(self.views.keys());
        v.extend(self.markers.keys());
        v.extend(self.templates.keys());
        v.extend(self.filters.keys());
        v.extend(self.schedules.keys());
        v.extend(self.sheets.keys());
        v.extend(self.viewports.keys());
        v.extend(self.schedule_instances.keys());
        v.extend(self.revisions.keys());
        v.extend(self.phase_filters.keys());
        v
    }

    pub fn max_id(&self) -> Option<ElementId> {
        self.ids().into_iter().max()
    }

    pub fn contains(&self, id: ElementId) -> bool {
        self.ids().contains(&id)
    }
}

fn bad(code: &str, msg: &str) -> Failure {
    Failure::error(code, msg.to_owned())
}

fn not_found(id: ElementId, what: &str) -> Failure {
    Failure::error(codes::NOT_FOUND, format!("No such {what}.")).on(&[id])
}

// ------------------------------------------------------------------------------------------------ materials

/// UNKNOWN: whether the comparison ignores case for the element path; the engine compares exactly as observed.
fn check_material_name(m: &BimModel, name: &str) -> Result<(), Failure> {
    if crate::type_names::has_prohibited_char(name) {
        return Err(bad(
            codes::MATERIAL_INVALID,
            "name cannot include prohibited characters, such as \"{, }, [, ], |, ;, less-than sign, greater-than sign, ?, `, ~\".",
        ));
    }
    if name == DEFAULT_WALL_MATERIAL || m.docs.materials.values().any(|x| x.name == name) {
        return Err(bad(codes::MATERIAL_INVALID, "The given value for name is already in use as a material element name."));
    }
    Ok(())
}

/// The name the asset-manager path gives a new or duplicated material: the name itself when it is free (names compared
/// case-insensitively); otherwise a trailing parenthesised number is split off ("Base(n)" -> "Base", n), or the trimmed
/// name is the base with n = 1, and "Base(n)" (no space) is tried with n rising by 1 until a free name is found, at most 10
/// 000 tries (then the name is returned unchanged).
pub fn unique_material_name(m: &BimModel, requested: &str) -> String {
    let free = |n: &str| n.to_lowercase() != DEFAULT_WALL_MATERIAL.to_lowercase() && !m.docs.materials.values().any(|x| x.name.to_lowercase() == n.to_lowercase());
    if free(requested) {
        return requested.to_owned();
    }
    let trimmed = requested.trim();
    let (base, start) = match trimmed.strip_suffix(')').and_then(|t| t.rsplit_once('(')) {
        Some((b, n)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => (b.to_owned(), n.parse::<u32>().unwrap_or(1)),
        _ => (trimmed.to_owned(), 1),
    };
    (start..start.saturating_add(10_000)).map(|n| format!("{base}({n})")).find(|c| free(c)).unwrap_or_else(|| requested.to_owned())
}

/// `Material.Create`: the name checks of [`check_material_name`]; the material is a default-constructed one (OBSERVED
/// defaults, round 31; UNKNOWN: the constructor's values live in the reference product).
pub fn add_material(m: &mut BimModel, cx: &mut TxnContext<'_>, name: &str) -> Result<ElementId, Failure> {
    check_material_name(m, name)?;
    let id = cx.new_id();
    m.docs.materials.insert(
        id,
        Material {
            id,
            name: name.to_owned(),
            color: [0, 0, 0],
            transparency: 0,
            shininess: 64,
            smoothness: 50,
            class: "Unassigned".into(),
            category: "Unassigned".into(),
            use_render_appearance: false,
            surface_pattern: None,
            surface_color: [0, 0, 0],
            cut_pattern: None,
            cut_color: [0, 0, 0],
            appearance_asset: None,
        },
    );
    Ok(id)
}

/// `Material.Duplicate`: the same two name checks, then a clone of the source under the new name ("a duplicate carries
/// every value of the source; only the name differs").
pub fn clone_material(m: &mut BimModel, cx: &mut TxnContext<'_>, from: ElementId, name: &str) -> Result<ElementId, Failure> {
    let src = m.docs.materials.get(&from).ok_or_else(|| not_found(from, "material"))?.clone();
    let id = add_material(m, cx, name)?;
    let mut copy = src;
    copy.id = id;
    copy.name = name.to_owned();
    m.docs.materials.insert(id, copy);
    Ok(id)
}

/// One property of a material. OBSERVED: a shininess outside 0..=128 is refused; a transparency of 120 is accepted and
/// stored (ENGINE CHOICE: no range is enforced for the others); a name in use is refused.
pub fn set_material_property(m: &mut BimModel, id: ElementId, prop: &str, value: &MaterialValue) -> Result<(), Failure> {
    let taken = |name: &str, except: ElementId, m: &BimModel| name == DEFAULT_WALL_MATERIAL || m.docs.materials.values().any(|x| x.id != except && x.name == name);
    if let ("name", MaterialValue::Text(n)) = (prop, value) {
        if taken(n, id, m) {
            return Err(bad(codes::MATERIAL_INVALID, "Name must be unique."));
        }
    }
    let mat = m.docs.materials.get_mut(&id).ok_or_else(|| not_found(id, "material"))?;
    match (prop, value) {
        ("name", MaterialValue::Text(n)) => mat.name = n.clone(),
        ("color", MaterialValue::Color(c)) => mat.color = *c,
        ("transparency", MaterialValue::Int(v)) => mat.transparency = *v,
        ("shininess", MaterialValue::Int(v)) => {
            if !(0..=128).contains(v) {
                return Err(bad(codes::MATERIAL_INVALID, "values for shininess must be between 0 and 128"));
            }
            mat.shininess = *v;
        }
        ("smoothness", MaterialValue::Int(v)) => mat.smoothness = *v,
        ("material_class", MaterialValue::Text(v)) => mat.class = v.clone(),
        ("material_category", MaterialValue::Text(v)) => mat.category = v.clone(),
        ("use_render_appearance", MaterialValue::Flag(v)) => mat.use_render_appearance = *v,
        ("surface_pattern", MaterialValue::Text(v)) => {
            if !fill_pattern_exists(v) {
                return Err(bad(codes::MATERIAL_INVALID, &format!("no fill pattern {v}")));
            }
            mat.surface_pattern = Some(v.clone());
        }
        ("surface_color", MaterialValue::Color(c)) => mat.surface_color = *c,
        ("cut_pattern", MaterialValue::Text(v)) => {
            if !fill_pattern_exists(v) {
                return Err(bad(codes::MATERIAL_INVALID, &format!("no fill pattern {v}")));
            }
            mat.cut_pattern = Some(v.clone());
        }
        ("cut_color", MaterialValue::Color(c)) => mat.cut_color = *c,
        ("appearance_asset", MaterialValue::Text(v)) => mat.appearance_asset = Some(v.clone()),
        _ => return Err(bad(codes::MATERIAL_INVALID, "Unknown material property or value.")),
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub enum MaterialValue {
    Text(String),
    Int(i32),
    Flag(bool),
    Color(Rgb),
}

/// The layers of a wall type as material references: one entry per layer.
pub fn layer_materials_of(m: &BimModel, wall_type: ElementId) -> Vec<Option<ElementId>> {
    let n = m.wall_types.get(&wall_type).map_or(0, |t| t.structure.layers.len());
    let mut v = m.docs.layer_materials.get(&wall_type).cloned().unwrap_or_default();
    v.resize(n, None);
    v
}

/// Give the layers of a wall type their materials (at creation).
pub fn set_wall_type_materials(m: &mut BimModel, wall_type: ElementId, mats: Vec<Option<ElementId>>) {
    if mats.iter().any(Option::is_some) {
        m.docs.layer_materials.insert(wall_type, mats);
    } else {
        m.docs.layer_materials.remove(&wall_type);
    }
}

/// `CompoundStructure.SetMaterialId`: an index out of range is refused (OBSERVED).
pub fn set_layer_material(m: &mut BimModel, wall_type: ElementId, layer: usize, material: Option<ElementId>) -> Result<(), Failure> {
    let n = m.wall_types.get(&wall_type).ok_or_else(|| not_found(wall_type, "wall type"))?.structure.layers.len();
    if layer >= n {
        return Err(bad(codes::MATERIAL_INVALID, "The layer index is out of range."));
    }
    if let Some(mat) = material {
        if !m.docs.materials.contains_key(&mat) {
            return Err(not_found(mat, "material"));
        }
    }
    let mut v = layer_materials_of(m, wall_type);
    v[layer] = material;
    set_wall_type_materials(m, wall_type, v);
    Ok(())
}

/// The material of the layer at one face of a wall (layer 0 is the exterior one).
fn face_material(m: &BimModel, wall: ElementId, side: i8) -> Option<ElementId> {
    let w = m.walls.get(&wall)?;
    let v = layer_materials_of(m, w.type_id);
    if side > 0 {
        v.first().copied().flatten()
    } else {
        v.last().copied().flatten()
    }
}

/// `Document.Paint`: painting a face with the material it already has leaves it unpainted (OBSERVED, the interior of a
/// one-layer wall painted with the layer's own material); painting again replaces the paint.
pub fn paint_face(m: &mut BimModel, wall: ElementId, side: i8, material: ElementId) -> Result<(), Failure> {
    if !m.walls.contains_key(&wall) {
        return Err(not_found(wall, "wall"));
    }
    // An id that is not a material is refused at the call with this text
    if !m.docs.materials.contains_key(&material) {
        return Err(bad(codes::MATERIAL_INVALID, "The material id does not refer to a material."));
    }
    m.docs.paints.retain(|p| !(p.wall == wall && p.side == side));
    if face_material(m, wall, side) != Some(material) {
        m.docs.paints.push(Paint { wall, side, material });
    }
    Ok(())
}

pub fn unpaint_face(m: &mut BimModel, wall: ElementId, side: i8) -> Result<(), Failure> {
    if !m.walls.contains_key(&wall) {
        return Err(not_found(wall, "wall"));
    }
    m.docs.paints.retain(|p| !(p.wall == wall && p.side == side));
    Ok(())
}

/// Deleting a material: its layers and its paints go (OBSERVED: the layers read "no material" afterwards).
pub fn delete_material(m: &mut BimModel, id: ElementId) -> bool {
    if m.docs.materials.remove(&id).is_none() {
        return false;
    }
    for v in m.docs.layer_materials.values_mut() {
        for x in v.iter_mut() {
            if *x == Some(id) {
                *x = None;
            }
        }
    }
    m.docs.layer_materials.retain(|_, v| v.iter().any(Option::is_some));
    m.docs.paints.retain(|p| p.material != id);
    true
}

/// The materials a wall reports (`GetMaterialIds(false)`) with the area and volume of each, summed over its layers:
/// `(material, area, volume)`; `None` is the default wall material. OBSERVED: every layer covers the wall's area. OBSERVED
/// (every round that records `wall_material_quantities`, rounds 30-41, over 3000 wall states, straight, arc, joined and
/// cut): the volumes of the materials SUM to the wall's `Volume`.
///
/// ENGINE CHOICE: the share of each layer is its width over the total width. That equals `area x width` for a free straight
/// wall; how the reference product splits the volume of a joined or an arc wall with several layers among them (each
/// layer's own body, or a proportion) is not specified.
pub fn wall_material_quantities(m: &BimModel, wall: ElementId) -> Option<Vec<(Option<ElementId>, f64, f64)>> {
    let w = m.walls.get(&wall)?;
    let st = crate::regen::wall_state(m, w).ok()?;
    let widths: Vec<f64> = m.wall_types.get(&w.type_id)?.structure.layers.iter().map(|l| l.width).collect();
    let total: f64 = widths.iter().sum();
    let mats = layer_materials_of(m, w.type_id);
    let mut out: Vec<(Option<ElementId>, f64, f64)> = Vec::new();
    for (i, width) in widths.iter().enumerate() {
        let key = mats[i];
        let vol = if total > 0.0 { st.volume * width / total } else { 0.0 };
        match out.iter_mut().find(|e| e.0 == key) {
            Some(e) => {
                e.1 += st.area;
                e.2 += vol;
            }
            None => out.push((key, st.area, vol)),
        }
    }
    Some(out)
}

/// The painted materials of a wall with their areas (`GetMaterialIds(true)` / `GetMaterialArea(id, true)`).
pub fn wall_painted_quantities(m: &BimModel, wall: ElementId) -> Vec<(ElementId, f64)> {
    let area = m.walls.get(&wall).and_then(|w| crate::regen::wall_state(m, w).ok()).map_or(0.0, |s| s.area);
    let mut out: Vec<(ElementId, f64)> = Vec::new();
    for p in m.docs.paints.iter().filter(|p| p.wall == wall) {
        match out.iter_mut().find(|e| e.0 == p.material) {
            Some(e) => e.1 += area,
            None => out.push((p.material, area)),
        }
    }
    out
}

/// Delete an element of this slice and what depends on it; returns every id removed (the element first).
pub fn delete_docs_element(m: &mut BimModel, id: ElementId) -> Vec<ElementId> {
    if m.docs.materials.contains_key(&id) {
        delete_material(m, id);
        return vec![id];
    }
    if m.docs.toposolids.contains_key(&id) {
        let subs: Vec<ElementId> = m.docs.toposolids.values().filter(|t| t.host == Some(id)).map(|t| t.id).collect();
        m.docs.toposolids.remove(&id);
        for s in &subs {
            m.docs.toposolids.remove(s);
        }
        let mut gone = vec![id];
        gone.extend(subs);
        return gone;
    }
    if m.docs.views.contains_key(&id) || m.docs.markers.contains_key(&id) {
        return crate::docviews::delete_doc_view(m, id);
    }
    if m.docs.filters.contains_key(&id) {
        crate::docviews::delete_filter(m, id);
        return vec![id];
    }
    if m.docs.templates.contains_key(&id) {
        crate::docviews::delete_template(m, id);
        return vec![id];
    }
    crate::sheets::delete_sheet_element(m, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ops, CompoundStructure, Layer, LayerFunction};
    use asterion_geom::Vec2;

    #[test]
    fn materials_layers_paint_and_quantities_follow_the_observations() {
        let mut d = crate::new_document();
        let (wt, wall, m1, m2) = d
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let m1 = add_material(m, cx, "M1")?;
                let m2 = add_material(m, cx, "M2")?;
                let wt = ops::create_wall_type(
                    m,
                    cx,
                    "WM",
                    CompoundStructure::new(
                        vec![
                            Layer { width: 0.1, function: LayerFunction::Structure },
                            Layer { width: 0.05, function: LayerFunction::Finish1 },
                            Layer { width: 0.05, function: LayerFunction::Finish2 },
                        ],
                        1,
                        1,
                    )?,
                )?;
                set_wall_type_materials(m, wt, vec![Some(m1), Some(m2), None]);
                let w = ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l, start: Vec2::new(0.0, 0.0), end: Vec2::new(6.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false })?;
                Ok((wt, w, m1, m2))
            })
            .value
            .unwrap();
        // OBSERVED: 18 m2 per layer; volume = area x width; a layer without a material is the default one; same-material layers add
        let q = wall_material_quantities(d.model(), wall).unwrap();
        assert_eq!(q.len(), 3);
        assert!((q[0].1 - 18.0).abs() < 1e-9 && (q[0].2 - 1.8).abs() < 1e-9 && q[0].0 == Some(m1));
        d.transact("l", false, |m, _| set_layer_material(m, wt, 0, Some(m2)));
        let q = wall_material_quantities(d.model(), wall).unwrap();
        let two = q.iter().find(|e| e.0 == Some(m2)).unwrap();
        assert!((two.1 - 36.0).abs() < 1e-9 && (two.2 - 2.7).abs() < 1e-9);
        assert!(d.transact("bad", false, |m, _| set_layer_material(m, wt, 5, Some(m2))).value.is_none());
        // paint: the same material as the face is no paint; another is
        d.transact("p1", false, |m, _| paint_face(m, wall, 1, m2));
        assert!(d.model().docs.paints.is_empty());
        d.transact("p2", false, |m, _| paint_face(m, wall, 1, m1));
        assert_eq!(wall_painted_quantities(d.model(), wall).len(), 1);
        // deleting a material clears its layers and its paints
        d.transact("del", false, |m, _| Ok(delete_material(m, m1)));
        assert!(d.model().docs.paints.is_empty());
        assert!(d.transact("dup", false, |m, cx| add_material(m, cx, "M2")).value.is_none(), "a name in use is refused");
        assert!(d.transact("sh", false, |m, _| set_material_property(m, m2, "shininess", &MaterialValue::Int(200))).value.is_none());
        assert!(d.transact("tr", false, |m, _| set_material_property(m, m2, "transparency", &MaterialValue::Int(120))).committed());
    }
}
