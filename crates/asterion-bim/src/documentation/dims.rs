//! Linear dimensions, text notes and tags (OBSERVED and).
//!
//! * A dimension is derived from its references: the dimension line meets each reference (a wall face or location line,
//!   a grid) and a segment is the distance between neighbouring meeting points. One segment: a value and a string in
//!   millimetres ("4000"), the origin at the middle of the two points; several: no value of its own, one value per
//!   segment, the text of each at the middle of its segment 0.175 m (1.75 mm of paper at 1:100) off the line. The text
//!   properties of a segment, once any is set, read as empty strings instead of null. Deleting a referenced wall drops
//!   its reference; a dimension left with one segment takes the text of the first. A copy of a dimension is an exact
//!   duplicate (the copy vector is ignored). A dimension with several segments cannot be locked.
//! * A text note keeps its text with a carriage return as every line's end. A mirror turns Right into Left.
//! * A tag follows its host when the host moves and goes with it when it is deleted; a tag alone cannot be copied (a
//!   warning, nothing is made). The text of a door tag is the door's mark.

use crate::annot::{DimRef, DimText, Dimension, HAlign, Tag, TextNote, TextNoteType, V3};
use crate::model::BimModel;
use crate::spec::codes;
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use asterion_geom::Vec2;

/// 1.75 mm of paper at the view's scale of 1:100 (OBSERVED: the text sits 0.175 m off the dimension line).
pub const TEXT_OFFSET_M: f64 = 0.175;
pub const DEFAULT_DIMENSION_TYPE: &str = "Linear - 3mm Arial";

fn bad(code: &str, msg: &str) -> Failure {
    Failure::error(code, msg.to_owned())
}

/// A reference as an infinite line in plan: a point and a direction.
fn ref_line(m: &BimModel, r: &DimRef) -> Option<(Vec2, Vec2)> {
    match r {
        DimRef::Wall { wall, side } => {
            let w = m.walls.get(wall)?;
            let (d, orient, _, width) = crate::regen::wall_frame(m, w).ok()?;
            Some((w.start + orient * (f64::from(*side) * width / 2.0), d))
        }
        DimRef::Grid { grid } => {
            let g = m.grids.get(grid)?;
            Some((g.start, (g.end - g.start).normalized()?))
        }
    }
}

/// The meeting points of the references with the dimension line, as parameters along it, in the order of the
/// references sorted by position.
pub fn stations(m: &BimModel, d: &Dimension) -> Option<(Vec2, Vec2, Vec<f64>)> {
    let p0 = Vec2::new(d.line.0[0], d.line.0[1]);
    let p1 = Vec2::new(d.line.1[0], d.line.1[1]);
    let u = (p1 - p0).normalized()?;
    let mut ts = Vec::new();
    // OBSERVED: a dimension line parallel to every reference is made, with the value 0 and no segments (all the stations at
    // the line's start). ENGINE CHOICE: one parallel reference among others is refused.
    let all_parallel = d.refs.iter().all(|r| ref_line(m, r).map_or(false, |(_, dir)| u.cross(dir).abs() < 1e-9));
    if all_parallel {
        return Some((p0, u, vec![0.0; d.refs.len()]));
    }
    for r in &d.refs {
        let (p, dir) = ref_line(m, r)?;
        let c = u.cross(dir);
        if c.abs() < 1e-9 {
            return None;
        }
        // p0 + u t = p + dir s
        let t = (p - p0).cross(dir) / c;
        ts.push(t);
    }
    ts.sort_by(f64::total_cmp);
    Some((p0, u, ts))
}

/// The value in metres of every segment.
///
/// A segment's value is |(P_b - P_a) . d|, the distance between the two reference points that bound it MEASURED ALONG THE
/// DIMENSION DIRECTION d, never negative; the references are sorted along d first. Here the points are the meeting points
/// of the references with the dimension line, so the distance along the line is the same number.
pub fn segment_values(m: &BimModel, d: &Dimension) -> Vec<f64> {
    match stations(m, d) {
        Some((_, _, ts)) => ts.windows(2).map(|w| (w[1] - w[0]).abs()).collect(),
        None => Vec::new(),
    }
}

/// The text a value reads as: millimetres, no decimals. The template's style is millimetres at accuracy 1 (OBSERVED);
/// UNKNOWN: the style's units of a document that changed them.
pub fn value_text(v: f64) -> String {
    let mm = crate::schedules::round_to_accuracy(v * 1000.0, 1.0, crate::schedules::RoundMethod::Nearest);
    format!("{}", mm as i64)
}

/// The text of a dimension segment. The non-empty fragments are joined with one space, in this order: the ABOVE text, the
/// segment PREFIX, (the style's prefix, the label and the symbols: none in the engine), the VALUE, the segment SUFFIX, and
/// - after everything else - the BELOW text. The VALUE is the segment's value override when it has one, and then the
/// segment's prefix and suffix are left out (above and below stay); else the measured value, written in millimetres.
/// OBSERVED: "AB P0 4000 S0", "5000 m" (a suffix that itself starts with a space). UNKNOWN: the unit formatting of the
/// measured value, the alternate units and the label / equality fragments; the sentinel -1e30 reading `IDS_QUESTIONS` is
/// not modelled.
fn compose(text: &DimText, value: f64) -> String {
    let nonempty = |o: &Option<String>| o.as_deref().filter(|p| !p.is_empty()).map(str::to_owned);
    let mut parts: Vec<String> = Vec::new();
    parts.extend(nonempty(&text.above));
    match nonempty(&text.value_override) {
        Some(o) => parts.push(o),
        None => {
            parts.extend(nonempty(&text.prefix));
            parts.push(value_text(value));
            parts.extend(nonempty(&text.suffix));
        }
    }
    parts.extend(nonempty(&text.below));
    parts.join(" ")
}

/// `compose` for the tests of other modules.
#[cfg(test)]
pub(crate) fn compose_for_test(text: &DimText, value: f64) -> String {
    compose(text, value)
}

/// The measured state of a dimension.
#[derive(Clone, Debug, PartialEq)]
pub struct DimState {
    pub value: Option<f64>,
    pub value_string: Option<String>,
    pub segments: Vec<SegState>,
    pub number_of_segments: usize,
    pub origin: Option<V3>,
    pub text_position: Option<V3>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SegState {
    pub value: f64,
    pub value_string: String,
    pub text: DimText,
    pub text_position: V3,
}

pub fn dim_state(m: &BimModel, d: &Dimension) -> Option<DimState> {
    let (p0, u, ts) = stations(m, d)?;
    let n = Vec2::new(-u.y, u.x);
    let at = |t: f64| p0 + u * t;
    let off = n * TEXT_OFFSET_M;
    if ts.len() == 2 {
        let v = (ts[1] - ts[0]).abs();
        let mid = at(0.5 * (ts[0] + ts[1]));
        let tp = match d.text_position {
            Some(o) => [mid.x + o[0], mid.y + o[1], o[2]],
            None => [mid.x + off.x, mid.y + off.y, 0.0],
        };
        return Some(DimState {
            value: Some(v),
            value_string: Some(value_text(v)),
            segments: Vec::new(),
            number_of_segments: 0,
            origin: Some([mid.x, mid.y, 0.0]),
            text_position: Some(tp),
        });
    }
    let segments = ts
        .windows(2)
        .enumerate()
        .map(|(i, w)| {
            let v = (w[1] - w[0]).abs();
            let mid = at(0.5 * (w[0] + w[1]));
            let text = d.segment_text.get(&i).cloned().unwrap_or_default();
            SegState { value: v, value_string: compose(&text, v), text, text_position: [mid.x + off.x, mid.y + off.y, 0.0] }
        })
        .collect();
    Some(DimState { value: None, value_string: None, segments, number_of_segments: ts.len().saturating_sub(1), origin: None, text_position: None })
}

/// `NewDimension(view, line, references)`.
pub fn create_dimension(m: &mut BimModel, cx: &mut TxnContext<'_>, view: ElementId, line: ([f64; 2], [f64; 2]), refs: Vec<DimRef>) -> Result<ElementId, Failure> {
    if refs.len() < 2 {
        return Err(bad(codes::DIMENSION_INVALID, "A dimension needs at least two references."));
    }
    m.view(view)?;
    let d = Dimension { id: ElementId(0), view, refs, line, text: DimText::default(), segment_text: Default::default(), text_position: None, locked: false, equal: false };
    if stations(m, &d).is_none() {
        return Err(bad(codes::DIMENSION_INVALID, "The references cannot be dimensioned along that line."));
    }
    let id = cx.new_id();
    m.annot.dims.insert(id, Dimension { id, ..d });
    Ok(id)
}

/// One text property of a dimension (`ValueOverride`, `Prefix`, `Suffix`, `Above`, `Below`).
pub fn set_dimension_text(m: &mut BimModel, id: ElementId, segment: Option<usize>, property: &str, value: &str) -> Result<(), Failure> {
    let d = m.annot.dims.get_mut(&id).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such dimension.").on(&[id]))?;
    let text = match segment {
        None => &mut d.text,
        Some(i) => d.segment_text.entry(i).or_default(),
    };
    // a segment's properties read as empty strings once any is set
    if segment.is_some() {
        for f in [&mut text.value_override, &mut text.prefix, &mut text.suffix, &mut text.above, &mut text.below] {
            f.get_or_insert_with(String::new);
        }
    }
    let field = match property {
        "ValueOverride" => &mut text.value_override,
        "Prefix" => &mut text.prefix,
        "Suffix" => &mut text.suffix,
        "Above" => &mut text.above,
        "Below" => &mut text.below,
        _ => return Err(bad(codes::DIMENSION_INVALID, "Unknown dimension text property.")),
    };
    *field = Some(value.to_owned());
    Ok(())
}

pub fn set_dimension_lock(m: &mut BimModel, id: ElementId, locked: bool) -> Result<(), Failure> {
    let segs = {
        let d = m.annot.dims.get(&id).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such dimension.").on(&[id]))?;
        d.refs.len().saturating_sub(1)
    };
    if segs > 1 {
        return Err(bad(codes::DIMENSION_INVALID, "Cannot access this property if this dimension is linear shape with more than one segments."));
    }
    m.annot.dims.get_mut(&id).expect("checked").locked = locked;
    Ok(())
}

/// The equality toggle of a dimension. When segment 0 is currently among the equal segments every segment's equal flag is
/// CLEARED, otherwise every segment's flag is SET - one toggle for the whole dimension. Returns the new state. UNKNOWN:
/// what the sketch solver does to the other references of an equal or locked dimension and the failure
/// `CannotToggleDimensionEquality`: the engine stores the flag and moves nothing. A dimension with a single segment has
/// nothing to equalise.
pub fn toggle_dimension_equality(m: &mut BimModel, id: ElementId) -> Result<bool, Failure> {
    let d = m.annot.dims.get_mut(&id).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such dimension.").on(&[id]))?;
    if d.refs.len().saturating_sub(1) < 2 {
        return Err(bad(codes::DIMENSION_INVALID, "Only a dimension with several segments can be made equal."));
    }
    d.equal = !d.equal;
    Ok(d.equal)
}

pub fn set_dimension_text_position(m: &mut BimModel, id: ElementId, p: V3) -> Result<(), Failure> {
    let segs = m.annot.dims.get(&id).map(|d| d.refs.len().saturating_sub(1)).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such dimension.").on(&[id]))?;
    if segs > 1 {
        return Err(bad(codes::DIMENSION_INVALID, "The text position of a dimension with several segments is not modelled."));
    }
    // stored as an offset from the origin, so the text follows the references
    let origin = {
        let d = &m.annot.dims[&id];
        dim_state(m, d).and_then(|s| s.origin).unwrap_or([0.0; 3])
    };
    m.annot.dims.get_mut(&id).expect("checked").text_position = Some([p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]]);
    Ok(())
}

/// `Dimension.ResetTextPosition`.
pub fn reset_dimension_text(m: &mut BimModel, id: ElementId) -> Result<(), Failure> {
    m.annot.dims.get_mut(&id).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such dimension.").on(&[id]))?.text_position = None;
    Ok(())
}

/// Drop the references to a deleted element; a dimension left with fewer than two is deleted. Returns the deleted ids.
pub fn element_deleted(m: &mut BimModel, gone: ElementId) -> Vec<ElementId> {
    let mut removed = Vec::new();
    let ids: Vec<ElementId> = m.annot.dims.values().filter(|d| d.refs.iter().any(|r| r.element() == gone)).map(|d| d.id).collect();
    for id in ids {
        let d = m.annot.dims.get_mut(&id).expect("listed");
        d.refs.retain(|r| r.element() != gone);
        if d.refs.len() < 2 {
            m.annot.dims.remove(&id);
            removed.push(id);
        } else if d.refs.len() == 2 && !d.segment_text.is_empty() {
            // collapsed to one segment: it carries the text of the first
            if let Some(t) = d.segment_text.get(&0).cloned() {
                d.text = t;
            }
            d.segment_text.clear();
        }
    }
    // tags of a deleted host go with it
    let tags: Vec<ElementId> = m.annot.tags.values().filter(|t| t.host == gone).map(|t| t.id).collect();
    for t in tags {
        m.annot.tags.remove(&t);
        removed.push(t);
    }
    removed
}

// ------------------------------------------------------------------ text notes

pub const TEMPLATE_NOTE_TYPES: [(&str, f64); 8] = [
    ("2.5mm Arial", 0.0025),
    ("3.5mm Arial", 0.0035),
    ("4mm Arial", 0.004),
    ("5mm Arial", 0.005),
    ("6mm Arial", 0.006),
    ("7mm Arial", 0.007),
    ("8mm Arial", 0.008),
    ("Schedule Default", 0.0035),
];

/// The template's text note types, in name order, created on first use.
pub fn note_types(m: &mut BimModel, cx: &mut TxnContext<'_>) -> Vec<ElementId> {
    for (name, size) in TEMPLATE_NOTE_TYPES {
        if !m.annot.note_types.values().any(|t| t.name == name) {
            let id = cx.new_id();
            m.annot.note_types.insert(id, TextNoteType { id, name: name.to_owned(), size });
        }
    }
    let mut v: Vec<&TextNoteType> = m.annot.note_types.values().collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v.iter().map(|t| t.id).collect()
}

pub fn use_note_type(m: &mut BimModel, cx: &mut TxnContext<'_>, index: usize) -> Result<ElementId, Failure> {
    note_types(m, cx).get(index).copied().ok_or_else(|| bad(codes::TEXT_NOTE_INVALID, "No text note type at that index."))
}

pub fn duplicate_note_type(m: &mut BimModel, cx: &mut TxnContext<'_>, source: ElementId, name: &str) -> Result<ElementId, Failure> {
    let t = m.annot.note_types.get(&source).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such text note type.").on(&[source]))?.clone();
    // The duplicate's name checks
    let name = &crate::type_names::check_duplicate_name(name, |n| m.annot.note_types.values().any(|x| x.name == n))?;
    let id = cx.new_id();
    m.annot.note_types.insert(id, TextNoteType { id, name: name.to_owned(), size: t.size });
    Ok(id)
}

pub fn set_note_type_size(m: &mut BimModel, id: ElementId, size: f64) -> Result<(), Failure> {
    let t = m.annot.note_types.get_mut(&id).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such text note type.").on(&[id]))?;
    t.size = size;
    Ok(())
}

/// The text as the reference product keeps it: every line ends with a carriage return.
pub fn normalise_text(text: &str) -> String {
    let mut out = String::new();
    for line in text.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        out.push_str(line);
        out.push('\r');
    }
    out
}

/// The text of a note replaced (the same normalisation as at creation; an empty text is refused as at creation).
pub fn set_text_note_text(m: &mut BimModel, id: ElementId, text: &str) -> Result<(), Failure> {
    if text.is_empty() {
        return Err(bad(codes::TEXT_NOTE_INVALID, "A text note needs text."));
    }
    let n = m.annot.notes.get_mut(&id).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such text note.").on(&[id]))?;
    n.text = normalise_text(text);
    Ok(())
}

pub fn create_text_note(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    view: ElementId,
    coord: V3,
    text: &str,
    type_id: Option<ElementId>,
    width: Option<f64>,
    align: HAlign,
) -> Result<ElementId, Failure> {
    m.view(view)?;
    // An empty text, or one of white space only, makes no text note (OBSERVED: the API call raised)
    if text.trim().is_empty() {
        return Err(bad(codes::TEXT_NOTE_INVALID, "A text note needs text."));
    }
    let type_id = match type_id {
        Some(t) => t,
        // the document's default text note type: the first of the template (ENGINE CHOICE)
        None => note_types(m, cx)[0],
    };
    if !m.annot.note_types.contains_key(&type_id) {
        return Err(Failure::error(codes::NOT_FOUND, "No such text note type.").on(&[type_id]));
    }
    let id = cx.new_id();
    m.annot.notes.insert(id, TextNote { id, view, type_id, coord, text: normalise_text(text), width, align, leaders: 0 });
    Ok(id)
}

// ------------------------------------------------------------------ tags

/// The tag type a host gets by default, and its category (OBSERVED: the first type by name of the category).
pub fn tag_type_of(m: &BimModel, host: ElementId) -> (&'static str, &'static str) {
    if let Some(i) = m.hosted_instances.get(&host) {
        if m.hosted_types.get(&i.type_id).map_or(false, |t| t.kind == crate::model::HostedKind::Window) {
            // OBSERVED: a window takes the window tag of the template
            return ("M_Window Tag", "Window Tags");
        }
        ("M_Door Tag", "Door Tags")
    } else if m.rooms.contains_key(&host) {
        ("Room Tag", "Room Tags")
    } else {
        ("12mm", "Wall Tags")
    }
}

/// The text a tag reads: a door's mark; a wall tag reads the type mark, which the template leaves empty.
pub fn tag_text(m: &BimModel, t: &Tag) -> String {
    if m.hosted_instances.contains_key(&t.host) {
        // the reference product numbers the doors as they are made, per kind (ENGINE CHOICE, shared with the schedules;
        // OBSERVED: the second door reads "2" with a window placed between them)
        return crate::schedules::instance_mark(m, t.host);
    }
    String::new()
}

/// A wall or a door. OBSERVED: a room is not tagged this way (no tag type). A tag made WITHOUT a leader has the given point
/// as its head; a tag made WITH a leader does not use the given point - its head is the default head position the reference
/// product computes (view scale, view directions, the tag family's box), which the engine does not model, so the head reads
/// as unknown (`head_known = false`, OBSERVED) until a head is set.
pub fn create_tag(m: &mut BimModel, cx: &mut TxnContext<'_>, view: ElementId, host: ElementId, head: V3, leader: bool, vertical: bool) -> Result<ElementId, Failure> {
    m.view(view)?;
    if !(m.walls.contains_key(&host) || m.hosted_instances.contains_key(&host)) {
        return Err(bad(codes::TAG_INVALID, "There is no loaded tag type that can be used when tagging that element."));
    }
    let id = cx.new_id();
    m.annot.tags.insert(id, Tag { id, view, host, head, leader, vertical, head_known: !leader });
    Ok(id)
}

/// `TextNote.AddLeader`.
pub fn add_text_leader(m: &mut BimModel, note: ElementId) -> Result<(), Failure> {
    m.annot.notes.get_mut(&note).ok_or_else(|| Failure::error(codes::NOT_FOUND, "No such text note.").on(&[note]))?.leaders += 1;
    Ok(())
}

/// `NewRoomTag`: a room (not measured yet: round 29b).
pub fn create_room_tag(m: &mut BimModel, cx: &mut TxnContext<'_>, view: ElementId, room: ElementId, head: V3) -> Result<ElementId, Failure> {
    m.view(view)?;
    if !m.rooms.contains_key(&room) {
        return Err(bad(codes::TAG_INVALID, "Only a room is tagged with a room tag."));
    }
    let id = cx.new_id();
    m.annot.tags.insert(id, Tag { id, view, host: room, head, leader: false, vertical: false, head_known: true });
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ops, CompoundStructure, Layer, LayerFunction};
    use asterion_geom::Vec2;

    fn fixture() -> (crate::BimDocument, [ElementId; 3], ElementId) {
        let mut d = crate::new_document();
        let (w, v) = d
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L0", 0.0)?;
                let wt = ops::create_wall_type(m, cx, "W", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
                let mk = |m: &mut crate::BimModel, cx: &mut TxnContext<'_>, x: f64| {
                    ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l, start: Vec2::new(x, 0.0), end: Vec2::new(x, 5.0), height: 3.0, base_offset: 0.0, flip: false, structural: false })
                };
                let w = [mk(m, cx, 0.0)?, mk(m, cx, 4.0)?, mk(m, cx, 9.0)?];
                let v = ops::create_view(m, cx, "P1", crate::model::ViewKind::FloorPlan, Some(l), None)?;
                Ok((w, v))
            })
            .value
            .unwrap();
        (d, w, v)
    }

    #[test]
    fn dimensions_between_wall_faces_have_the_observed_values_and_texts() {
        // OBSERVED: exterior faces of walls at 0 and 4: 4000, origin x 1.9, text 0.175 above the line; three interior
        // faces: segments 4 and 5; a deleted wall drops its reference
        let (mut d, w, v) = fixture();
        let one = d.transact("d1", false, |m, cx| create_dimension(m, cx, v, ([-3.0, 1.0], [13.0, 1.0]), vec![DimRef::Wall { wall: w[0], side: 1 }, DimRef::Wall { wall: w[1], side: 1 }])).value.unwrap();
        let s = dim_state(d.model(), &d.model().annot.dims[&one]).unwrap();
        assert!((s.value.unwrap() - 4.0).abs() < 1e-9 && s.value_string.as_deref() == Some("4000"));
        let o = s.origin.unwrap();
        assert!((o[0] - 1.9).abs() < 1e-9 && (o[1] - 1.0).abs() < 1e-9, "{o:?}");
        assert!((s.text_position.unwrap()[1] - 1.175).abs() < 1e-9);
        let three = d.transact("d3", false, |m, cx| create_dimension(m, cx, v, ([-3.0, 3.0], [13.0, 3.0]), vec![DimRef::Wall { wall: w[0], side: -1 }, DimRef::Wall { wall: w[1], side: -1 }, DimRef::Wall { wall: w[2], side: -1 }])).value.unwrap();
        let s = dim_state(d.model(), &d.model().annot.dims[&three]).unwrap();
        assert_eq!((s.number_of_segments, s.value), (2, None));
        assert!((s.segments[0].value - 4.0).abs() < 1e-9 && (s.segments[1].value - 5.0).abs() < 1e-9);
        assert!((s.segments[0].text_position[0] - 2.1).abs() < 1e-9);
        d.transact("o", false, |m, _| set_dimension_text(m, three, Some(0), "ValueOverride", "SEG0"));
        d.transact("s", false, |m, _| set_dimension_text(m, three, Some(1), "Suffix", " m"));
        let s = dim_state(d.model(), &d.model().annot.dims[&three]).unwrap();
        assert_eq!((s.segments[0].value_string.as_str(), s.segments[1].value_string.as_str()), ("SEG0", "5000  m"));
        assert_eq!(s.segments[1].text.value_override.as_deref(), Some(""));
        // cannot lock several segments; one can
        assert!(d.transact("l", false, |m, _| set_dimension_lock(m, three, true)).value.is_none());
        assert!(d.transact("l1", false, |m, _| set_dimension_lock(m, one, true)).committed());
        // the last wall deleted: two references left, one segment carrying the first segment's text
        d.transact("del", false, |m, cx| ops::delete(m, cx, w[2]));
        let s = dim_state(d.model(), &d.model().annot.dims[&three]).unwrap();
        assert_eq!((s.number_of_segments, s.value_string.as_deref()), (0, Some("4000")));
        assert_eq!(d.model().annot.dims[&three].text.value_override.as_deref(), Some("SEG0"));
    }

    #[test]
    fn the_equality_toggle_flips_every_segment_at_once() {
        // One toggle for the whole dimension; a one-segment dimension has nothing to equalise
        let (mut d, w, v) = fixture();
        let three = d.transact("d3", false, |m, cx| create_dimension(m, cx, v, ([-3.0, 3.0], [13.0, 3.0]), vec![DimRef::Wall { wall: w[0], side: -1 }, DimRef::Wall { wall: w[1], side: -1 }, DimRef::Wall { wall: w[2], side: -1 }])).value.unwrap();
        assert_eq!(d.transact("e1", false, |m, _| toggle_dimension_equality(m, three)).value, Some(true));
        assert!(d.model().annot.dims[&three].equal);
        assert_eq!(d.transact("e2", false, |m, _| toggle_dimension_equality(m, three)).value, Some(false));
        let one = d.transact("d1", false, |m, cx| create_dimension(m, cx, v, ([-3.0, 1.0], [13.0, 1.0]), vec![DimRef::Wall { wall: w[0], side: 1 }, DimRef::Wall { wall: w[1], side: 1 }])).value.unwrap();
        assert!(d.transact("e3", false, |m, _| toggle_dimension_equality(m, one)).value.is_none());
    }

    #[test]
    fn a_locked_dimension_drags_its_other_wall_and_a_parallel_line_reads_zero() {
        // OBSERVED: moving a wall of a locked dimension moves the other wall by the part of the move along the line; a line
        // parallel to the references makes a dimension of value 0 with no segment
        let (mut d, w, v) = fixture();
        let dim = d.transact("d", false, |m, cx| create_dimension(m, cx, v, ([-3.0, 1.0], [13.0, 1.0]), vec![DimRef::Wall { wall: w[0], side: 1 }, DimRef::Wall { wall: w[1], side: 1 }])).value.unwrap();
        d.transact("l", false, |m, _| set_dimension_lock(m, dim, true));
        d.transact("mv", false, |m, cx| crate::transform::apply_transform_to_elements(m, cx, &[w[1]], crate::transform::PlanTransform::Translate(Vec2::new(1.0, 2.0))));
        let m = d.model();
        assert!((m.walls[&w[1]].start.x - 5.0).abs() < 1e-9 && (m.walls[&w[1]].start.y - 2.0).abs() < 1e-9);
        assert!((m.walls[&w[0]].start.x - 1.0).abs() < 1e-9 && m.walls[&w[0]].start.y.abs() < 1e-9, "dragged along x only");
        let s = dim_state(m, &m.annot.dims[&dim]).unwrap();
        assert!((s.value.unwrap() - 4.0).abs() < 1e-9);
        let par = d.transact("p", false, |m, cx| create_dimension(m, cx, v, ([-1.0, 0.0], [-1.0, 5.0]), vec![DimRef::Wall { wall: w[0], side: 1 }, DimRef::Wall { wall: w[2], side: 1 }])).value.unwrap();
        let s = dim_state(d.model(), &d.model().annot.dims[&par]).unwrap();
        assert_eq!((s.value, s.number_of_segments), (Some(0.0), 0));
    }

    #[test]
    fn a_text_note_keeps_carriage_returns_and_the_template_types_are_in_name_order() {
        let (mut d, _, v) = fixture();
        let n = d
            .transact("n", false, |m, cx| {
                let t = use_note_type(m, cx, 0)?;
                create_text_note(m, cx, v, [10.0, 0.0, 0.0], "Line one\nLine two", Some(t), None, HAlign::Left)
            })
            .value
            .unwrap();
        assert_eq!(d.model().annot.notes[&n].text, "Line one\rLine two\r");
        let t = d.model().annot.note_types[&d.model().annot.notes[&n].type_id].clone();
        assert_eq!((t.name.as_str(), t.size), ("2.5mm Arial", 0.0025));
    }

    #[test]
    fn a_value_override_replaces_the_value_and_the_segment_prefix_and_suffix_only() {
        // With a value override the segment prefix, suffix and symbols are left empty; above and below stay
        let t = DimText { value_override: Some("SEG".into()), prefix: Some("P".into()), suffix: Some("S".into()), above: Some("A".into()), below: Some("B".into()) };
        assert_eq!(compose(&t, 4.0), "A SEG B");
        let none = DimText { prefix: Some("P".into()), suffix: Some("S".into()), above: Some("A".into()), below: Some("B".into()), ..DimText::default() };
        assert_eq!(compose(&none, 4.0), "A P 4000 S B");
        assert_eq!(compose(&DimText { value_override: Some("SEG".into()), ..DimText::default() }, 4.0), "SEG");
    }

    #[test]
    fn a_text_note_of_white_space_only_is_refused_like_an_empty_one() {
        let (mut d, _, v) = fixture();
        for text in ["", " ", "  \t ", "\n", " \r\n "] {
            let r = d.transact("n", false, |m, cx| create_text_note(m, cx, v, [0.0; 3], text, None, None, HAlign::Left));
            assert!(r.value.is_none(), "{text:?} must not make a note");
        }
        assert!(d.model().annot.notes.is_empty());
        let ok = d.transact("n", false, |m, cx| create_text_note(m, cx, v, [0.0; 3], " a ", None, None, HAlign::Left));
        assert!(ok.value.is_some(), "a text with a visible character is kept (spaces included)");
    }

    #[test]
    fn a_dimension_text_moved_by_hand_follows_its_walls() {
        // OBSERVED: the text position is kept as an offset, so moving a wall carries it
        let (mut d, w, v) = fixture();
        let dim = d.transact("d", false, |m, cx| create_dimension(m, cx, v, ([-3.0, 2.0], [13.0, 2.0]), vec![DimRef::Wall { wall: w[0], side: 1 }, DimRef::Wall { wall: w[1], side: 1 }])).value.unwrap();
        d.transact("t", false, |m, _| set_dimension_text_position(m, dim, [3.0, 3.0, 0.0]));
        let before = dim_state(d.model(), &d.model().annot.dims[&dim]).unwrap().text_position.unwrap();
        assert!((before[0] - 3.0).abs() < 1e-9 && (before[1] - 3.0).abs() < 1e-9, "{before:?}");
        d.transact("mv", false, |m, cx| crate::transform::apply_transform_to_elements(m, cx, &[w[1]], crate::transform::PlanTransform::Translate(Vec2::new(1.0, 0.0))));
        let after = dim_state(d.model(), &d.model().annot.dims[&dim]).unwrap().text_position.unwrap();
        assert!((after[0] - 3.5).abs() < 1e-9, "the text keeps its distance from the origin: {after:?}");
        d.transact("reset", false, |m, _| reset_dimension_text(m, dim));
        assert!(d.model().annot.dims[&dim].text_position.is_none());
    }

    #[test]
    fn a_curve_keeps_its_sketch_plane_and_a_copied_wall_keeps_its_sweeps() {
        use crate::annot::SweepKind;
        use crate::transform::{copy_elements, apply_transform_to_elements, PlanTransform};
        let (mut d, w, _) = fixture();
        let (line, sweep) = d
            .transact("c", false, |m, cx| {
                let l = crate::datums::create_model_line(m, cx, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0; 3], [0.0, 0.0, 1.0])?;
                let ty = crate::sweeps::types_of(m, cx, SweepKind::Sweep)[0];
                let s = crate::sweeps::create_wall_sweep(
                    m,
                    cx,
                    w[0],
                    ty,
                    crate::sweeps::SweepInfo { kind: SweepKind::Sweep, vertical: false, distance: 0.5, from_top: false, flipped: false, offset: 0.0, exterior: true, sweep_intersects_wall: true },
                )?;
                Ok((l, s))
            })
            .value
            .unwrap();
        d.transact("mv", false, |m, cx| apply_transform_to_elements(m, cx, &[line], PlanTransform::Translate(Vec2::new(5.0, 5.0))));
        let c = &d.model().annot.curves[&line];
        assert_eq!(c.plane_origin, [0.0; 3], "the sketch plane stays where it was");
        assert!((c.geom.ends().0[0] - 5.0).abs() < 1e-9);
        let copies = d.transact("cp", false, |m, cx| copy_elements(m, cx, &[w[0]], Some(PlanTransform::Translate(Vec2::new(0.0, 20.0))))).value.unwrap();
        let new_wall = copies[0].1;
        assert!(d.model().annot.sweeps.values().any(|x| x.wall == new_wall && x.id != sweep), "the copy carries the sweep");
    }
}
