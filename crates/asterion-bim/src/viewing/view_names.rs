//! Default names of views and schedules.
//!
//! Rows followed: (the first default name is the base name, the second "<name> 2", the counter starts at 2: the default 3D
//! view is "{3D}" ("{3D - <user>}" in a workshared document), a 3D view made by the API creators is "3D View 1", "3D View
//! 2", ... counted against the names of all existing 3D views) and (the pool of names a new name is tested against: every
//! view and every view template, exact case, sheets also by number).

use crate::model::{BimModel, ViewKind};

/// The default 3D view, resource text "{3D}".
pub const DEFAULT_3D_NAME: &str = "{3D}";

/// The base text of a 3D view made by the API creators.
pub const API_3D_BASE: &str = "3D View";

/// The base text of a walkthrough.
pub const WALKTHROUGH_BASE: &str = "Walkthrough";

/// . In a workshared document the name carries the user ("{3D - user}"), otherwise it is "{3D}". The engine has no
/// worksharing: pass `None`.
pub fn default_3d_view_name(worksharing_user: Option<&str>) -> String {
    match worksharing_user {
        Some(u) => format!("{{3D - {u}}}"),
        None => DEFAULT_3D_NAME.to_owned(),
    }
}

/// : "<base> 1", then "<base> 2", ... while the candidate equals the name of an existing 3D view (every 3D view, not only
/// the default ones).
pub fn unique_3d_view_name(m: &BimModel, walkthrough: bool) -> String {
    let base = if walkthrough { WALKTHROUGH_BASE } else { API_3D_BASE };
    let taken = |n: &str| m.views.values().any(|v| v.kind == ViewKind::ThreeD && v.name == n);
    (1u32..).map(|n| format!("{base} {n}")).find(|c| !taken(c)).expect("unbounded")
}

/// The DEFAULT 3D view, isometric, on no level, named "{3D}" (`{3D - user}` in a workshared document: the engine has none).
/// The reference product marks the name as NOT user-given; the engine keeps no such flag. A second call is refused with the
/// view-name duplicate failure (the reference product's on an existing name is not specified). `ops::create_view` gives
/// every 3D view that camera.
pub fn create_default_3d_view(m: &mut BimModel, cx: &mut asterion_core::txn::TxnContext<'_>) -> Result<asterion_core::ElementId, asterion_core::Failure> {
    crate::ops::create_view(m, cx, &default_3d_view_name(None), ViewKind::ThreeD, None, None)
}

/// An isometric 3D view made by the API creators: named "3D View 1", "3D View 2", ... (the lowest number no existing 3D
/// view carries) and marked user-named.
pub fn create_isometric_3d_view(m: &mut BimModel, cx: &mut asterion_core::txn::TxnContext<'_>) -> Result<asterion_core::ElementId, asterion_core::Failure> {
    let name = unique_3d_view_name(m, false);
    crate::ops::create_view(m, cx, &name, ViewKind::ThreeD, None, None)
}

/// The name equals either default text ("{3D}" or "{3D - <user>}").
pub fn is_default_3d_name(name: &str) -> bool {
    name == DEFAULT_3D_NAME || (name.starts_with("{3D - ") && name.ends_with('}'))
}

/// . The base name when it is free; else "<base> 2", "<base> 3", ... (the counter starts at 2, NOT at 1). `taken` answers
/// whether a name is in use in the shared view-name space.
pub fn unique_default_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    (2u32..).map(|n| format!("{base} {n}")).find(|c| !taken(c)).expect("unbounded")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_counter_of_a_default_name_starts_at_two_and_a_3d_view_name_at_one() {
        let used = ["Walls Schedule", "Walls Schedule 2"];
        assert_eq!(unique_default_name("Walls Schedule", |n| used.contains(&n)), "Walls Schedule 3");
        assert_eq!(unique_default_name("Rooms Schedule", |n| used.contains(&n)), "Rooms Schedule");
        assert_eq!(unique_default_name("Walls Schedule", |n| n == "Walls Schedule"), "Walls Schedule 2");
        assert_eq!(default_3d_view_name(None), "{3D}");
        assert_eq!(default_3d_view_name(Some("ana")), "{3D - ana}");
        let mut d = crate::new_document();
        assert_eq!(unique_3d_view_name(d.model(), false), "3D View 1");
        d.transact("v", false, |m, cx| crate::ops::create_view(m, cx, "{3D}", ViewKind::ThreeD, None, None));
        assert_eq!(unique_3d_view_name(d.model(), false), "3D View 1", "the default {{3D}} view does not take a number");
        d.transact("v", false, |m, cx| crate::ops::create_view(m, cx, "3D View 1", ViewKind::ThreeD, None, None));
        assert_eq!(unique_3d_view_name(d.model(), false), "3D View 2");
        assert_eq!(unique_3d_view_name(d.model(), true), "Walkthrough 1");
        // The creators
        let mut e = crate::new_document();
        let a = e.transact("a", false, |m, cx| create_default_3d_view(m, cx)).value.unwrap();
        let b = e.transact("b", false, |m, cx| create_isometric_3d_view(m, cx)).value.unwrap();
        let c = e.transact("c", false, |m, cx| create_isometric_3d_view(m, cx)).value.unwrap();
        let names: Vec<&str> = [a, b, c].iter().map(|i| e.model().views[i].name.as_str()).collect();
        assert_eq!(names, ["{3D}", "3D View 1", "3D View 2"]);
        assert!(e.transact("twice", false, |m, cx| create_default_3d_view(m, cx)).value.is_none());
        assert!(is_default_3d_name("{3D}") && is_default_3d_name("{3D - ana}") && !is_default_3d_name("3D View 1"));
    }
}
