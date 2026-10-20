//! Sheets, viewports, schedule instances and revisions (OBSERVED, public API).
//!
//! * A sheet number is unique ("Sheet number is already in use"). Without a title block the sheet has no outline until a
//!   viewport gives it one.
//! * `Viewport.Create(sheet, view, centre)`: a view goes on ONE sheet, once; a sheet, a template or a view already placed is
//!   refused ("This view cannot be placed on the sheet").
//! * Revisions are numbered after the template's own first revision (the first one made reads number 2).
//!
//! NOT modelled: the box and label outlines of a viewport, its centre after the view's scale changes, the label length and
//! the outline of a sheet without title block: they follow from the extent of the view's content.

use crate::model::BimModel;
use crate::spec::codes;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use serde::{Deserialize, Serialize};

pub const TITLE_BLOCK: &str = "A0 metric";
pub const TITLE_BLOCK_SIZE_M: [f64; 2] = [1.188, 0.84];
pub const VIEWPORT_TYPE: &str = "Title w Line";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sheet {
    pub id: ElementId,
    pub number: String,
    pub name: String,
    pub title_block: Option<String>,
    pub revisions: Vec<ElementId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    pub id: ElementId,
    pub sheet: ElementId,
    pub view: ElementId,
    pub center: [f64; 2],
    pub rotation: String,
    pub label_offset: Option<[f64; 2]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScheduleInstance {
    pub id: ElementId,
    pub sheet: ElementId,
    pub schedule: ElementId,
    pub point: [f64; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Revision {
    pub id: ElementId,
    pub description: String,
    pub date: String,
    pub issued_by: String,
    pub issued_to: String,
    pub issued: bool,
    pub sequence: u32,
    /// `RevisionVisibility`: 0 Hidden, 1 CloudAndTagVisible, 2 TagVisible. A new revision is 1.
    #[serde(default = "cloud_and_tag")]
    pub visibility: u8,
}

fn cloud_and_tag() -> u8 {
    1
}

fn bad(msg: &str) -> Failure {
    Failure::error(codes::SHEET_INVALID, msg.to_owned())
}

fn not_found(id: ElementId, what: &str) -> Failure {
    Failure::error(codes::NOT_FOUND, format!("No such {what}.")).on(&[id])
}

/// The sheet number checks of the number parameter: the value is trimmed of white space at both ends; a text that is empty
/// then is refused (`SheetNumberEmpty`, an error); otherwise a number another sheet already has is refused
/// (`SheetNumberDuplicated`, an error). Returns the trimmed number. the engine compares every sheet of the document.
pub fn check_sheet_number(m: &BimModel, except: Option<ElementId>, number: &str) -> Result<String, Failure> {
    let n = number.trim();
    if n.is_empty() {
        return Err(Failure::error(codes::SHEET_NUMBER_EMPTY, "Sheet number is empty."));
    }
    if let Some(other) = m.docs.sheets.values().find(|s| Some(s.id) != except && s.number == n) {
        return Err(Failure::error(codes::SHEET_NUMBER_DUPLICATED, "Sheet number is already in use.").on(&[other.id]));
    }
    Ok(n.to_owned())
}

/// `ViewSheet.Create`: `with_title_block` takes the template's title block.
pub fn create_sheet(m: &mut BimModel, cx: &mut TxnContext<'_>, number: Option<&str>, name: Option<&str>, with_title_block: bool) -> Result<ElementId, Failure> {
    let number = match number {
        Some(n) => check_sheet_number(m, None, n)?,
        // The next number from the generator, repeated until it is not used by another sheet. UNKNOWN: the generator and
        // the seed text; the engine counts up from the number of sheets (OBSERVED: the first sheet reads "1").
        None => (m.docs.sheets.len() as u64 + 1..).map(|n| n.to_string()).find(|c| !m.docs.sheets.values().any(|s| &s.number == c)).expect("unbounded"),
    };
    let id = cx.new_id();
    m.docs.sheets.insert(
        id,
        Sheet { id, number, name: name.unwrap_or("Unnamed").to_owned(), title_block: with_title_block.then(|| TITLE_BLOCK.to_owned()), revisions: Vec::new() },
    );
    Ok(id)
}

pub fn set_sheet(m: &mut BimModel, id: ElementId, number: Option<&str>, name: Option<&str>) -> Result<(), Failure> {
    let number = match number {
        Some(n) => Some(check_sheet_number(m, Some(id), n)?),
        None => None,
    };
    let s = m.docs.sheets.get_mut(&id).ok_or_else(|| not_found(id, "sheet"))?;
    if let Some(n) = number {
        s.number = n;
    }
    if let Some(n) = name {
        s.name = n.to_owned();
    }
    Ok(())
}

/// Whether a view can be placed on a sheet: a plan, section, elevation or callout that is not yet on any sheet.
pub fn can_add_view(m: &BimModel, sheet: ElementId, view: ElementId) -> bool {
    if !m.docs.sheets.contains_key(&sheet) {
        return false;
    }
    let kind_ok = crate::view_dup::view_type_of(m, view).map_or(false, crate::view_dup::viewport_capable);
    kind_ok && !m.docs.viewports.values().any(|v| v.view == view)
}

/// `Viewport.Create`. A sheet id that is not a sheet is refused first ("The sheet id does not refer to a sheet."), then a
/// view that cannot be placed is refused ("This view cannot be placed on the sheet."): a template, a view already on a
/// sheet (the views the engine has are not allowed on several sheets). UNKNOWN: the placement of the box centre at the
/// given point and the label.
pub fn create_viewport(m: &mut BimModel, cx: &mut TxnContext<'_>, sheet: ElementId, view: ElementId, center: [f64; 2]) -> Result<ElementId, Failure> {
    if !m.docs.sheets.contains_key(&sheet) {
        return Err(bad("The sheet id does not refer to a sheet."));
    }
    if !can_add_view(m, sheet, view) {
        return Err(bad("This view cannot be placed on the sheet."));
    }
    let id = cx.new_id();
    m.docs.viewports.insert(id, Viewport { id, sheet, view, center, rotation: "None".into(), label_offset: None });
    Ok(id)
}

pub fn create_schedule_instance(m: &mut BimModel, cx: &mut TxnContext<'_>, sheet: ElementId, schedule: ElementId, point: [f64; 2]) -> Result<ElementId, Failure> {
    if !m.docs.sheets.contains_key(&sheet) {
        return Err(not_found(sheet, "sheet"));
    }
    if !m.docs.schedules.contains_key(&schedule) {
        return Err(not_found(schedule, "schedule"));
    }
    if m.docs.schedule_instances.values().any(|s| s.sheet == sheet && s.schedule == schedule) {
        return Err(bad("The schedule is already on the sheet."));
    }
    let id = cx.new_id();
    m.docs.schedule_instances.insert(id, ScheduleInstance { id, sheet, schedule, point });
    Ok(id)
}

/// `ScheduleInstance.Point = ...`: the point is stored only when it differs from the stored one by more than a relative
/// 1e-9 (the squared distance against 1e-18 of the squared magnitudes). UNKNOWN: which corner of the schedule table the
/// point is.
pub fn set_schedule_instance_point(m: &mut BimModel, id: ElementId, point: [f64; 2]) -> Result<bool, Failure> {
    let s = m.docs.schedule_instances.get_mut(&id).ok_or_else(|| not_found(id, "schedule instance"))?;
    let d2 = (point[0] - s.point[0]).powi(2) + (point[1] - s.point[1]).powi(2);
    let mag = (point[0].powi(2) + point[1].powi(2)).max(s.point[0].powi(2) + s.point[1].powi(2));
    if d2 > 1e-18 * mag {
        s.point = point;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn set_viewport(m: &mut BimModel, id: ElementId, center: Option<[f64; 2]>, rotation: Option<&str>, label_offset: Option<[f64; 2]>) -> Result<(), Failure> {
    let v = m.docs.viewports.get_mut(&id).ok_or_else(|| not_found(id, "viewport"))?;
    if let Some(c) = center {
        v.center = c;
    }
    if let Some(r) = rotation {
        if !["None", "Clockwise", "Counterclockwise"].contains(&r) {
            return Err(bad("Unknown rotation."));
        }
        v.rotation = r.to_owned();
    }
    if let Some(o) = label_offset {
        v.label_offset = Some(o);
    }
    Ok(())
}

/// `Revision.Create`.
pub fn create_revision(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    description: Option<&str>,
    date: Option<&str>,
    issued_by: Option<&str>,
    issued_to: Option<&str>,
    issued: Option<bool>,
) -> ElementId {
    let id = cx.new_id();
    let sequence = m.docs.revisions.values().map(|r| r.sequence).max().unwrap_or(1) + 1;
    m.docs.revisions.insert(
        id,
        Revision {
            id,
            // : the resources "Revision %1!d!" and "Date %1!d!" formatted with the sequence number of the new revision
            description: description.map_or_else(|| format!("Revision {sequence}"), str::to_owned),
            date: date.map_or_else(|| format!("Date {sequence}"), str::to_owned),
            issued_by: issued_by.unwrap_or("").to_owned(),
            issued_to: issued_to.unwrap_or("").to_owned(),
            issued: issued.unwrap_or(false),
            sequence,
            visibility: 1,
        },
    );
    id
}

/// Edit one field of a revision: `description`, `date`, `issued_by`, `issued_to` (text) or `issued` ("1" / "true").
pub fn set_revision(m: &mut BimModel, id: ElementId, field: &str, value: &str) -> Result<(), Failure> {
    let r = m.docs.revisions.get_mut(&id).ok_or_else(|| not_found(id, "revision"))?;
    match field {
        "description" => r.description = value.to_owned(),
        "date" => r.date = value.to_owned(),
        "issued_by" => r.issued_by = value.to_owned(),
        "issued_to" => r.issued_to = value.to_owned(),
        "issued" => r.issued = matches!(value, "1" | "true" | "True" | "yes"),
        _ => return Err(bad(&format!("A revision has no field named {field}."))),
    }
    Ok(())
}

pub fn set_sheet_revisions(m: &mut BimModel, sheet: ElementId, revisions: Vec<ElementId>) -> Result<(), Failure> {
    for r in &revisions {
        if !m.docs.revisions.contains_key(r) {
            return Err(not_found(*r, "revision"));
        }
    }
    let s = m.docs.sheets.get_mut(&sheet).ok_or_else(|| not_found(sheet, "sheet"))?;
    let mut v = revisions;
    v.sort();
    v.dedup();
    s.revisions = v;
    Ok(())
}

/// The sequence number of a revision. The 1-based position of the revision in the document's revision list - which begins
/// with the template's own revision (OBSERVED: the first one a scenario makes reads 2).
pub fn revision_sequence(m: &BimModel, id: ElementId) -> Option<u32> {
    revision_list(m).iter().position(|k| *k == id).map(|p| p as u32 + 2)
}

/// The revisions in the order of the document's revision list, after the template's own first revision: the order the
/// dialog gave, the revisions it did not list after them in id order.
pub fn revision_list(m: &BimModel) -> Vec<ElementId> {
    let mut v: Vec<ElementId> = m.docs.revision_order.iter().copied().filter(|r| m.docs.revisions.contains_key(r)).collect();
    let rest: Vec<ElementId> = m.docs.revisions.keys().copied().filter(|r| !v.contains(r)).collect();
    v.extend(rest);
    v
}

fn locked(msg: &str, id: ElementId) -> Failure {
    Failure::error(codes::SHEET_INVALID, msg.to_owned()).on(&[id])
}

/// `Revision.Issued = ...`: the flag is stored (the API setter is not subject to the issued lock).
pub fn set_revision_issued(m: &mut BimModel, id: ElementId, issued: bool) -> Result<(), Failure> {
    m.docs.revisions.get_mut(&id).ok_or_else(|| not_found(id, "revision"))?.issued = issued;
    Ok(())
}

/// The editable properties of a revision.
#[derive(Default)]
pub struct RevisionEdit<'a> {
    pub description: Option<&'a str>,
    pub date: Option<&'a str>,
    pub issued_by: Option<&'a str>,
    pub issued_to: Option<&'a str>,
    /// `RevisionVisibility`: 0 Hidden, 1 CloudAndTagVisible, 2 TagVisible.
    pub visibility: Option<u8>,
}

/// Edit a revision's description, date, issuer, recipient or visibility. An ISSUED revision refuses every parameter edit; a
/// visibility outside 0..=2 is out of range (`RevisionVisibility`'s name table).
pub fn edit_revision(m: &mut BimModel, id: ElementId, e: RevisionEdit<'_>) -> Result<(), Failure> {
    let r = m.docs.revisions.get_mut(&id).ok_or_else(|| not_found(id, "revision"))?;
    if r.issued {
        return Err(locked("The revision is issued: its parameters are read-only.", id));
    }
    if e.visibility.map_or(false, |v| v > 2) {
        return Err(Failure::error(codes::SHEET_INVALID, "The revision visibility is out of range.").on(&[id]));
    }
    if let Some(x) = e.description {
        r.description = x.to_owned();
    }
    if let Some(x) = e.date {
        r.date = x.to_owned();
    }
    if let Some(x) = e.issued_by {
        r.issued_by = x.to_owned();
    }
    if let Some(x) = e.issued_to {
        r.issued_to = x.to_owned();
    }
    if let Some(v) = e.visibility {
        r.visibility = v;
    }
    Ok(())
}

/// The new sequence must hold exactly the current revision ids - the same count, every current id present (a permutation);
/// it does not test the issued flag. The template's own first revision is not part of the engine's list and stays first
/// (ENGINE CHOICE).
pub fn reorder_revisions(m: &mut BimModel, ids: &[ElementId]) -> Result<(), Failure> {
    let mut a: Vec<ElementId> = ids.to_vec();
    let mut b = revision_list(m);
    a.sort();
    b.sort();
    if a != b {
        return Err(Failure::error(codes::SHEET_INVALID, "The new revision sequence must contain exactly the current revisions."));
    }
    m.docs.revision_order = ids.to_vec();
    Ok(())
}

/// Whether a revision can be merged into the NEXT one: it is not issued, it is not the last, and the next revision is not
/// issued.
pub fn may_merge_with_next_revision(m: &BimModel, id: ElementId) -> bool {
    let list = revision_list(m);
    let Some(i) = list.iter().position(|r| *r == id) else { return false };
    i + 1 < list.len() && !m.docs.revisions[&id].issued && !m.docs.revisions[&list[i + 1]].issued
}

/// Whether a revision can be merged into the PREVIOUS one: it is not issued, it has a previous revision, and that one is
/// not issued. The template's own first revision is not in the engine's list, so the first revision the engine holds has no
/// previous one here (ENGINE CHOICE).
pub fn can_combine_with_previous(m: &BimModel, id: ElementId) -> bool {
    let list = revision_list(m);
    let Some(i) = list.iter().position(|r| *r == id) else { return false };
    i >= 1 && !m.docs.revisions[&id].issued && !m.docs.revisions[&list[i - 1]].issued
}

/// : the revision is merged into its neighbour and deleted with its children. Returns the neighbour.
pub fn combine_revision(m: &mut BimModel, id: ElementId, with_next: bool) -> Result<ElementId, Failure> {
    let ok = if with_next { may_merge_with_next_revision(m, id) } else { can_combine_with_previous(m, id) };
    if !ok {
        return Err(locked("The revision cannot be combined with its neighbour.", id));
    }
    let list = revision_list(m);
    let i = list.iter().position(|r| *r == id).expect("checked");
    let into = if with_next { list[i + 1] } else { list[i - 1] };
    for s in m.docs.sheets.values_mut() {
        if let Some(p) = s.revisions.iter().position(|r| *r == id) {
            s.revisions.remove(p);
            if !s.revisions.contains(&into) {
                s.revisions.push(into);
                s.revisions.sort();
            }
        }
    }
    m.docs.revision_order.retain(|r| *r != id);
    delete_sheet_element(m, id);
    Ok(into)
}

/// Whether the dialog can delete a revision: the document has at least two revisions (the template's own counts) and this
/// one is not issued.
pub fn can_delete_revision(m: &BimModel, id: ElementId) -> bool {
    m.docs.revisions.get(&id).map_or(false, |r| !r.issued) && m.docs.revisions.len() + 1 >= 2
}

/// Delete a revision the way the dialog does: refused when it cannot be deleted. The revisions after it move up in the
/// list.
pub fn delete_revision(m: &mut BimModel, id: ElementId) -> Result<Vec<ElementId>, Failure> {
    if !can_delete_revision(m, id) {
        return Err(locked("The revision cannot be deleted.", id));
    }
    m.docs.revision_order.retain(|r| *r != id);
    Ok(delete_sheet_element(m, id))
}

/// The number of a revision numbered by project. 1 plus the number of earlier revisions of the same numbering sequence,
/// written with the numeric settings (n - 1) x increment + start, padded to the minimum digits, with prefix and suffix; the
/// defaults are increment 1, start 1, one digit, no prefix or suffix. Every revision of the engine (and the template's)
/// uses the built-in numeric sequence, so the number is the sequence number.
pub fn revision_number(m: &BimModel, id: ElementId) -> Option<String> {
    revision_sequence(m, id).map(|n| n.to_string())
}

/// The revision number a sheet shows for a revision. No number when the revision is not one of the sheet's revisions; with
/// revisions numbered BY PROJECT (the only mode the engine has: the numbering mode is not a setting yet) the project
/// revision's own number. UNKNOWN: which revisions a sheet shows besides the ones set on it (issued, additional, clouds)
/// and the per-sheet order.
pub fn revision_number_on_sheet(m: &BimModel, sheet: ElementId, revision: ElementId) -> Option<String> {
    if !m.docs.sheets.get(&sheet)?.revisions.contains(&revision) {
        return None;
    }
    revision_number(m, revision)
}

/// Delete an element of this slice (sheet, viewport, schedule instance, revision, schedule): returns the ids removed.
pub fn delete_sheet_element(m: &mut BimModel, id: ElementId) -> Vec<ElementId> {
    let mut gone = Vec::new();
    if m.docs.sheets.remove(&id).is_some() {
        gone.push(id);
        let vps: Vec<ElementId> = m.docs.viewports.values().filter(|v| v.sheet == id).map(|v| v.id).collect();
        let sis: Vec<ElementId> = m.docs.schedule_instances.values().filter(|v| v.sheet == id).map(|v| v.id).collect();
        for v in vps {
            m.docs.viewports.remove(&v);
            gone.push(v);
        }
        for s in sis {
            m.docs.schedule_instances.remove(&s);
            gone.push(s);
        }
    } else if m.docs.viewports.remove(&id).is_some() || m.docs.schedule_instances.remove(&id).is_some() {
        gone.push(id);
    } else if m.docs.revisions.remove(&id).is_some() {
        gone.push(id);
        for s in m.docs.sheets.values_mut() {
            s.revisions.retain(|r| *r != id);
        }
    } else if m.docs.schedules.remove(&id).is_some() {
        gone.push(id);
        let sis: Vec<ElementId> = m.docs.schedule_instances.values().filter(|v| v.schedule == id).map(|v| v.id).collect();
        for s in sis {
            m.docs.schedule_instances.remove(&s);
            gone.push(s);
        }
    }
    gone
}

/// The placed views of a sheet in the order of their viewports (the views of the viewports, then the schedules).
pub fn placed_views(m: &BimModel, sheet: ElementId) -> Vec<ElementId> {
    m.docs.viewports.values().filter(|v| v.sheet == sheet).map(|v| v.view).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops;

    #[test]
    fn sheets_number_uniquely_and_a_view_goes_on_one_sheet_once() {
        let mut d = crate::new_document();
        let (l, v) = d
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let v = ops::create_view(m, cx, "P1", crate::model::ViewKind::FloorPlan, Some(l), None)?;
                Ok((l, v))
            })
            .value
            .unwrap();
        let _ = l;
        let s1 = d.transact("s1", false, |m, cx| create_sheet(m, cx, Some("A101"), Some("Plans"), true)).value.unwrap();
        let s2 = d.transact("s2", false, |m, cx| create_sheet(m, cx, Some("A102"), None, false)).value.unwrap();
        assert!(d.transact("dup", false, |m, cx| create_sheet(m, cx, Some("A101"), None, true)).value.is_none());
        assert!(d.transact("vp", false, |m, cx| create_viewport(m, cx, s1, v, [0.2, 0.15])).committed());
        assert!(d.transact("vp2", false, |m, cx| create_viewport(m, cx, s2, v, [0.2, 0.15])).value.is_none(), "one sheet only");
        assert!(d.transact("vp3", false, |m, cx| create_viewport(m, cx, s2, s1, [0.2, 0.15])).value.is_none(), "a sheet is not a view");
        let r1 = d.transact("r1", false, |m, cx| Ok(create_revision(m, cx, Some("First"), None, None, None, None))).value.unwrap();
        assert_eq!(d.model().docs.revisions[&r1].sequence, 2);
        d.transact("sr", false, |m, _| set_sheet_revisions(m, s1, vec![r1]));
        assert!(d.transact("e", false, |m, _| set_revision(m, r1, "description", "Issued for tender")).committed());
        assert!(d.transact("e", false, |m, _| set_revision(m, r1, "issued", "1")).committed());
        assert_eq!((d.model().docs.revisions[&r1].description.as_str(), d.model().docs.revisions[&r1].issued), ("Issued for tender", true));
        assert!(d.transact("e", false, |m, _| set_revision(m, r1, "colour", "red")).value.is_none(), "an unknown field is refused");
        assert!(d.transact("e", false, |m, _| set_revision(m, s1, "date", "x")).value.is_none(), "a sheet is not a revision");
        assert_eq!(d.model().docs.sheets[&s1].revisions, vec![r1]);
        d.transact("del", false, |m, _| Ok(delete_sheet_element(m, r1)));
        assert!(d.model().docs.sheets[&s1].revisions.is_empty());
    }
}
