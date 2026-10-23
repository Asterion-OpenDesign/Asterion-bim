
use super::{def, Def, Dumped, Listed, Raw, Storage};
use crate::docviews::{self, DocViewKind};
use crate::model::BimModel;
use asterion_core::ElementId;
use Listed::{Class, Generic};
use Storage::{Double, Integer, String as Str};

pub const VIEW_PLAN: &[Def] = &[
    def(-1154613, "Show Hidden Lines", Integer, false, Class { class: "DBView", conditional: true }, "(DBView)"),
    def(-1150170, "Column Symbolic Offset", Double, false, Class { class: "DBViewPlan", conditional: true }, "(DBViewPlan)"),
    def(-1013200, "Design Option", Str, true, Generic, ""),
    def(-1011003, "Parts Visibility", Integer, false, Class { class: "DBView", conditional: true }, "(DBView)"),
    def(-1005333, "Sun Path", Integer, false, Class { class: "DBView", conditional: true }, "(DBView)"),
    def(-1005183, "Color Scheme Location", Integer, false, Class { class: "DBView", conditional: true }, "(DBView)"),
    def(-1005182, "Dependency", Str, true, Class { class: "DBView", conditional: true }, "(DBView)"),
    def(-1005177, "Underlay Orientation", Integer, false, Class { class: "DBViewPlan", conditional: true }, "(DBViewPlan)"),
    def(-1005168, "Orientation", Integer, false, Class { class: "DBViewPlan", conditional: true }, "(DBViewPlan)"),
    def(-1005166, "Associated Level", Str, true, Class { class: "DBViewPlan", conditional: true }, "(DBViewPlan)"),
    def(-1005161, "Display Model", Integer, false, Class { class: "DBView", conditional: true }, "(DBView)"),
    def(-1005158, "Wall Join Display", Integer, false, Class { class: "DBViewPlan", conditional: true }, "(DBViewPlan)"),
];

pub const VIEW_SECTION: &[Def] = &[
    def(-1154613, "Show Hidden Lines", Integer, false, Class { class: "DBView", conditional: true }, "(DBView)"),
    def(-1013200, "Design Option", Str, true, Generic, ""),
    def(-1012202, "Scope Box", Integer, false, Class { class: "Viewer", conditional: true }, "(Viewer)"),
    def(-1011003, "Parts Visibility", Integer, false, Class { class: "DBView", conditional: true }, "(DBView)"),
    def(-1005333, "Sun Path", Integer, false, Class { class: "DBView", conditional: true }, "(DBView)"),
    def(-1005183, "Color Scheme Location", Integer, false, Class { class: "DBView", conditional: true }, "(DBView)"),
    def(-1005182, "Dependency", Str, true, Class { class: "DBView", conditional: true }, "(DBView)"),
    def(-1005161, "Display Model", Integer, false, Class { class: "DBView", conditional: true }, "(DBView)"),
];

pub const TEMPLATE_CONTROLLED: &[i32] = &[
    -1005163, -1154613, -1005151, -1005152, -1005150, -1005161, -1011002, -1011003, -1006961, -1006962, -1006963, -1006966, -1006969,
    -1006970, -1005165, -1005173, -1005131, -1005132, -1154615, -1005133, -1005137, -1005135, -1005136, -1005183, -1005148, -1133900,
];

pub const TEMPLATE_CONTROLLED_PLAN: &[i32] = &[-1005162, -1005181, -1005153, -1005335, -1005168, -1005177];

/// The dump of a view of the plan or section class; `None` for any other element.
pub fn view_dump(m: &BimModel, id: ElementId) -> Option<Vec<Dumped>> {
    let (table, plan, level_name): (&'static [Def], bool, Option<String>) = if let Some(v) = m.views.get(&id) {
        (VIEW_PLAN, true, v.gen_level.and_then(|l| m.levels.get(&l)).map(|l| l.name.clone()))
    } else if let Some(v) = m.docs.views.get(&id) {
        match v.kind {
            DocViewKind::CalloutPlan => (VIEW_PLAN, true, v.level.and_then(|l| m.levels.get(&l)).map(|l| l.name.clone())),
            _ => (VIEW_SECTION, false, None),
        }
    } else {
        return None;
    };
    // A parameter controlled by the assigned view template is not modifiable. The ids a template can control are the
    // application lists; WHICH of them a given template includes is stored in the template and not modelled.
    let templated = docviews::view_has_template(m, id);
    let independent = crate::view_dup::primary_of(m, id).is_none() && crate::view_dup::dependents_of(m, id).is_empty();
    Some(
        table
            .iter()
            .map(|d| {
                let controlled = templated && (TEMPLATE_CONTROLLED.contains(&d.id) || (plan && TEMPLATE_CONTROLLED_PLAN.contains(&d.id)));
                let read_only = Some(d.registered_read_only || controlled);
                let raw = match d.id {
                    -1005166 => level_name.clone().map_or(Raw::Unknown, |n| Raw::Text(Some(n))),
                    -1005182 if independent => Raw::Text(Some("Independent".to_owned())),
                    // ENGINE FACT: the engine draws no sun path
                    -1005333 => Raw::Int(0),
                    _ => Raw::Unknown,
                };
                Dumped { def: d, read_only, raw }
            })
            .collect(),
    )
}
