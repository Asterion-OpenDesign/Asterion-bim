//! Document-level settings that a document stores and the engine keeps beside its elements:
//! the active browser organisation of the Views node and its folder definitions, the starting
//! view, the document's default types, the project units element and the stored facts of the
//! views and grids that the engine's own types have no field for (view type, family, template,
//! discipline, raw detail level; grid type and head family).
//!
//! A document authored in ASTERION has `DocumentSettings::default` - nothing is invented for it.
//!
//! What the rules leave open is NOT turned into engine behaviour here: for example the label of the Views node ("Views
//! (<name>)") is still driven by [`BimModel::browser_organization`], which an import does not set, because the root label
//! of the node is the open.

use crate::model::BimModel;
use asterion_core::ElementId;
use asterion_geom::Vec2;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The groups of element types for which a document keeps a default. The engine uses the wall group: the type of a wall
/// created by the "curve + level" path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultTypeGroup {
    Wall,
}

/// One folder definition of a browser organisation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrowserFolder {
    /// The parameter id that selects the folder text (-1012106 = the view type's "family and type" folder, -1012109 = its
    /// family folder, -1001717 = a category name, any other parameter = its formatted value).
    pub parameter_id: Option<i64>,
    /// The truncation it drives is not specified.
    pub characters: Option<i64>,
}

/// One filter of a browser organisation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrowserFilter {
    /// The parameter id the filter tests (-1 = always satisfied; -1012106 the view's type,
    /// -1012109 its view family, -1012107 its print set).
    pub parameter_id: i64,
    /// 0 equals, 1 not equals.
    pub relation: i64,
    /// The kind of the value: 0 none, 1 int, 3 string, 4 element id.
    pub value_kind: i64,
    /// The integer value.
    pub value_int: i64,
    /// The string value.
    pub value_str: String,
    /// The element-id value.
    pub value_elem_id: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrowserOrganisation {
    /// The organisation element's id in the source document.
    pub source_id: i64,
    /// The organisation's name as the element stores it. An OBSERVATION: where the reference product keeps the name that
    /// labels the Views node is not specified.
    pub name: Option<String>,
    pub folders: Vec<BrowserFolder>,
    /// How many filters the organisation holds.
    pub filters: usize,
    /// The organisation's filters in stored order. A view is listed in the organisation only when every filter is
    /// satisfied; see [`crate::browser::view_listed`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filter_list: Vec<BrowserFilter>,
    pub sort_parameter: Option<i64>,
    pub sort_ascending: Option<bool>,
}

/// The document's starting view.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StartingView {
    pub source_id: i64,
    /// The engine view it is, when the engine holds that view. `None` when the stored view is not
    /// one the engine models (a sheet, a schedule, ...): the document then opens on its first
    /// plan view ([`BimModel::opening_view`]) and the import reports the loss.
    pub view: Option<ElementId>,
    /// Class of the stored starting view when the engine does not hold it (for the report).
    pub unrepresented_class: Option<String>,
}

/// What an imported view stored that the engine's [`crate::View`] has no field for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewSource {
    /// The view's id in the source document.
    pub source_id: i64,
    /// A dependent view answers with its primary's.
    pub type_source_id: Option<i64>,
    /// The type's name.
    pub type_name: Option<String>,
    pub family: Option<i64>,
    /// None when -1.
    pub template_source_id: Option<i64>,
    /// The int parameter -1005163 (1 when absent).
    pub discipline: i64,
    /// The int parameter -1011002 as stored (0 when absent). The names of 1, 2 and 3 are not specified, so the engine's
    /// detail level is not set from it.
    pub detail_level_raw: i64,
}

/// What an imported door / window TYPE stored that the engine's [`crate::HostedType`] has no field for: the family it
/// belongs to and where its category came from.
///
/// The engine's type name is the symbol's own name; the reference product lists a type as family + type, and two families
/// may hold types of the same name, so the family is kept here, by engine type id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostedTypeSource {
    /// The symbol's id in the source document.
    pub source_id: i64,
    pub family_source_id: Option<i64>,
    pub family_name: Option<String>,
    /// The category id the type was classified by (-2000023 doors, -2000014 windows).
    pub category_id: Option<i64>,
    /// Whether an instance of the type exists in the document (an unplaced type is a loaded family's type that no element
    /// uses).
    pub placed: bool,
}

/// What an imported grid stored that the engine's [`crate::Grid`] has no field for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GridSource {
    /// The grid's id in the source document.
    pub source_id: i64,
    /// The grid type element.
    pub type_source_id: Option<i64>,
    /// The type's name.
    pub type_name: Option<String>,
    pub head_family_source_id: Option<i64>,
}

/// The relative names a plan view range can give its levels: the plan's own (associated) level, the level above, the level
/// below, or no limit. What elevation "above" and "below" stand for is NOT specified, so the range plane of an imported view
/// is computed as unbounded for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeLevelName {
    /// The plan's own level.
    Current,
    Above,
    Below,
    Unlimited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeLevelRef {
    Named(RangeLevelName),
    Level(i64),
}

impl RangeLevelName {

    /// Name for reports.
    pub fn name(self) -> &'static str {
        match self {
            RangeLevelName::Current => "current",
            RangeLevelName::Above => "level_above",
            RangeLevelName::Below => "level_below",
            RangeLevelName::Unlimited => "unlimited",
        }
    }
}

/// An arc grid (a cylindrical datum surface) as a document stored it.
///
/// The engine's [`crate::Grid`] is a straight line and is not changed for this: the arc is kept as data read from the
/// document, in metres and radians, with the parameters the reference product evaluates (`P(u) = center + radius (cos u
/// x_axis + sin u y_axis)`, from the angle of the envelope's u-min corner to its u-max corner counter-clockwise, each
/// normalised to `[0, 2 pi)`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArcGridSource {
    /// The grid's id in the source document.
    pub source_id: i64,
    pub name: String,
    pub type_source_id: Option<i64>,
    /// Arc centre, plan, metres.
    pub center: Vec2,
    /// Arc radius, metres.
    pub radius: f64,
    /// Plan end points and the point at the middle parameter, metres.
    pub start: Vec2,
    /// The point at the middle parameter.
    pub through: Vec2,
    /// The end point.
    pub end: Vec2,
    /// Start angle (radians, the cylinder parameter), in `[0, 2 pi)`.
    pub start_angle: f64,
    /// End angle (radians), not smaller than `start_angle`.
    pub end_angle: f64,
    /// Vertical extent `(bottom, top)`, metres.
    pub vertical_extent: (f64, f64),
}

/// The settings a document stores (module documentation).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DocumentSettings {
    /// The active organisation of the Views node and its folder definitions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub views_browser: Option<BrowserOrganisation>,
    /// The starting view; `None` = the document does not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starting_view: Option<StartingView>,
    /// The document's default element types: group -> element id in the SOURCE document, as stored. Empty = not stored.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub default_types: BTreeMap<DefaultTypeGroup, i64>,
    /// The engine wall type that group [`DefaultTypeGroup::Wall`] names, when the document stores it and the engine holds
    /// that type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_wall_type: Option<ElementId>,
    /// The project units element. Which entry of its units is the length spec is not specified, so only the element id is
    /// kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units_element: Option<i64>,
    /// Stored facts of each imported view, by engine view id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub view_sources: BTreeMap<ElementId, ViewSource>,
    /// Stored facts of each imported grid, by engine grid id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub grid_sources: BTreeMap<ElementId, GridSource>,
    /// Stored facts of each imported door / window type, by engine type id: its family, and whether any instance uses it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hosted_type_sources: BTreeMap<ElementId, HostedTypeSource>,
    /// Only the id is kept: the table that maps a unit id to its factor is not specified, so no display text is derived
    /// from it. `None` = not stored (the reference product then uses its library default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_unit_type_id: Option<String>,
    /// The stored level of each of the five range slots (cut, top clip, bottom clip, view depth, underlay bottom) of each
    /// imported plan view, by engine view id: a relative name ([`RangeLevelName`]) or the id of a level; the engine's range
    /// plane is computed as unbounded for every name but the plan's own level.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub view_range_level_ids: BTreeMap<ElementId, [RangeLevelRef; 5]>,
    /// Arc grids the document stores, which the engine's straight [`crate::Grid`] cannot hold.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arc_grids: Vec<ArcGridSource>,
    /// The document's default family symbol (type) per category, here per hosted kind: the type a door or window tool
    /// starts from. Written by a family load (the first type of the loaded family) and by Create Similar (the type of the
    /// instance used). Empty = none stored.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub default_hosted_types: BTreeMap<crate::model::HostedKind, ElementId>,
    /// The stored viewer box of each imported section, elevation and 3D view, by engine view id: the frame and the bounds
    /// [`crate::docviews::doc_view`] derives the view's drawing frame from.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub view_boxes: BTreeMap<ElementId, StoredViewBox>,
}

/// Nothing here is derived: the section frame, the crop rectangle and the depth are computed from it on read
/// ([`crate::docviews::doc_view`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredViewBox {
    pub origin: [f64; 3],
    pub basis: [[f64; 3]; 3],
    pub offset: [[f64; 2]; 3],
    pub active: [[bool; 2]; 3],
    pub on: bool,
    pub perspective: Option<bool>,
}

impl DocumentSettings {
    /// True when the document stored nothing of this layer.
    pub fn is_empty(&self) -> bool {
        *self == DocumentSettings::default()
    }
}

impl BimModel {
    /// The type a wall takes when it is placed without an explicit type: the document's stored
    /// default wall type when the import resolved it to a wall type the engine
    /// holds, otherwise the first wall type by id (what the engine always did).
    pub fn default_wall_type(&self) -> Option<ElementId> {
        match self.settings.default_wall_type {
            Some(t) if self.wall_types.contains_key(&t) => Some(t),
            _ => self.wall_types.keys().next().copied(),
        }
    }

    /// The document's default type of a hosted kind, when one is stored and the type still exists with that kind.
    pub fn default_hosted_type(&self, kind: crate::model::HostedKind) -> Option<ElementId> {
        let t = *self.settings.default_hosted_types.get(&kind)?;
        self.hosted_types.get(&t).filter(|h| h.kind == kind).map(|h| h.id)
    }

    pub fn range_level_name(&self, view: ElementId, slot: usize) -> Option<RangeLevelName> {
        match self.settings.view_range_level_ids.get(&view)?.get(slot)? {
            RangeLevelRef::Named(n) => Some(*n),
            RangeLevelRef::Level(_) => None,
        }
    }

    /// The view the document opens on, when it stored a starting view: that view when the engine holds it, otherwise the
    /// first plan view by id. `None` when the document does not say (the shell then chooses).
    pub fn opening_view(&self) -> Option<ElementId> {
        let sv = self.settings.starting_view.as_ref()?;
        if let Some(v) = sv.view {
            if self.views.contains_key(&v) {
                return Some(v);
            }
        }
        self.views.values().find(|v| v.kind.is_plan()).map(|v| v.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DetailLevel, View, ViewKind, ViewRange, VisualStyle};
    use crate::{ops, CompoundStructure, Layer, LayerFunction};

    fn view(id: u64, kind: ViewKind) -> View {
        View {
            id: ElementId(id),
            name: format!("v{id}"),
            kind,
            scale: 100,
            detail_level: DetailLevel::Coarse,
            gen_level: None,
            range: ViewRange::default(),
            crop: None,
            crop_active: false,
            crop_visible: false,
            origin_z: None,
            visual_style: VisualStyle::default(),
            display: Default::default(),
            hidden_elements: Default::default(),
            camera: None,
        }
    }

    /// The default wall type is the stored one when the engine holds it, else the first.
    #[test]
    fn default_wall_type_follows_the_document() {
        let mut doc = crate::new_document();
        let (a, b) = {
            let out = doc.transact("types", false, |m, cx| {
                let s = || CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0).unwrap();
                let a = ops::create_wall_type(m, cx, "A", s())?;
                let b = ops::create_wall_type(m, cx, "B", s())?;
                Ok((a, b))
            });
            out.value.unwrap()
        };
        let mut m = doc.model().clone();
        assert_eq!(m.default_wall_type(), Some(a), "no setting: the first type by id");
        m.settings.default_wall_type = Some(b);
        assert_eq!(m.default_wall_type(), Some(b));
        m.settings.default_wall_type = Some(ElementId(999));
        assert_eq!(m.default_wall_type(), Some(a), "a type the engine does not hold is ignored");
    }

    /// The opening view: the stored one when held, else the first plan view; nothing when the
    /// document says nothing.
    #[test]
    fn opening_view_falls_back_to_the_first_plan_view() {
        let mut m = BimModel::default();
        for (id, k) in [(7, ViewKind::ThreeD), (9, ViewKind::CeilingPlan), (12, ViewKind::FloorPlan)] {
            m.views.insert(ElementId(id), view(id, k));
        }
        assert_eq!(m.opening_view(), None);
        m.settings.starting_view = Some(StartingView { source_id: 850623, view: None, unrepresented_class: Some("Sheet".into()) });
        assert_eq!(m.opening_view(), Some(ElementId(9)), "first plan view by id");
        m.settings.starting_view = Some(StartingView { source_id: 5, view: Some(ElementId(12)), unrepresented_class: None });
        assert_eq!(m.opening_view(), Some(ElementId(12)));
        m.settings.starting_view = Some(StartingView { source_id: 5, view: Some(ElementId(77)), unrepresented_class: None });
        assert_eq!(m.opening_view(), Some(ElementId(9)), "a view that is gone falls back");
    }

    /// An empty settings value is empty; one with data is not (the JSON round trip is tested in
    /// `asterion-persist`).
    #[test]
    fn emptiness() {
        let mut s = DocumentSettings::default();
        assert!(s.is_empty());
        s.default_types.insert(DefaultTypeGroup::Wall, 249);
        assert!(!s.is_empty());
        let mut s = DocumentSettings::default();
        s.length_unit_type_id = Some("unit:feet-fractional-inches".into());
        assert!(!s.is_empty());
    }

    /// A level given by id has none.
    #[test]
    fn range_level_names() {
        use RangeLevelRef::{Level, Named};
        let mut m = BimModel::default();
        let cur = Named(RangeLevelName::Current);
        m.settings.view_range_level_ids.insert(ElementId(7), [cur, cur, cur, Named(RangeLevelName::Above), Named(RangeLevelName::Unlimited)]);
        m.settings.view_range_level_ids.insert(ElementId(9), [Level(208685), cur, cur, cur, cur]);
        assert_eq!(m.range_level_name(ElementId(9), 0), None, "a level by id has no relative name");
        assert_eq!(m.range_level_name(ElementId(7), 3), Some(RangeLevelName::Above));
        assert_eq!(m.range_level_name(ElementId(7), 4).map(|n| n.name()), Some("unlimited"));
        assert_eq!(m.range_level_name(ElementId(7), 5), None);
        assert_eq!(m.range_level_name(ElementId(8), 0), None);
    }

    /// The new fields keep an otherwise empty value non-empty (the JSON round trip, with the
    /// fields skipped when empty, is tested in `asterion-persist`).
    #[test]
    fn new_fields_count_as_content() {
        let mut s = DocumentSettings::default();
        s.arc_grids.push(ArcGridSource {
            source_id: 9,
            name: "A".into(),
            type_source_id: Some(3),
            center: Vec2::new(1.0, 2.0),
            radius: 3.0,
            start: Vec2::new(4.0, 2.0),
            through: Vec2::new(1.0, 5.0),
            end: Vec2::new(-2.0, 2.0),
            start_angle: 0.0,
            end_angle: std::f64::consts::PI,
            vertical_extent: (0.0, 9.0),
        });
        assert!(!s.is_empty());
        let mut s = DocumentSettings::default();
        s.view_range_level_ids.insert(ElementId(1), [RangeLevelRef::Named(RangeLevelName::Current); 5]);
        assert!(!s.is_empty());
    }
}
