//! The references a selected straight wall dimensions from (its temporary dimension's own side).
//!
//! Written specification of the specification rule Spec TEMPDIM-02 (the reference product wall reference provider): for a
//! pick of the whole wall,
//!
//! 1. one ANGULAR reference first;
//! 2. with the document's "clear temporary dimensions to walls" preference OFF (or a curtain wall):
//!    the wall's CENTRE face, or its CORE CENTRE face when the dimension-sides type is one of the two
//!    "core" values (the centre face when the wall has no core) - the same face added twice, once per
//!    direction (the second reversed), so the dimension can be made to either side;
//! 3. with the preference ON: the two SIDE faces (side 0 in one direction, side 1 in the other), or
//!    the two CORE side faces for the core values (each falling back to the ordinary side face when
//!    the wall has no core);
//! 4. each wall END that is not joined to a neighbour: its end face;
//! 5. the corner references last.
//! 6. each reference is the plane of a planar face, with that plane's normal as the direction of the
//!    dimension (flipped for the reversed entry); a non-planar face gives nothing.
//!
//! Not specified (named, ENGINE CHOICE until a row answers):
//! * Which preference states the two "core" dimension-sides values are. The engine takes the two settings as inputs
//!   ([`DimSettings`]) rather than inventing the document's defaults; its default is the preference off and the non-core
//!   type (the centre face).
//! * Which face is side 0. The engine takes side 0 as the face on the wall's orientation side (its exterior) and side 1 the
//!   opposite.
//! * The corner references (not produced).
//! * When a wall "has no core": the engine says a wall without shell layers (its core is its whole width) has none distinct
//!   from its faces, so the core faces fall back to the centre / side faces.
//! * An arc wall: its centre and side faces are cylinders (not planar: nothing is added), its end faces are planar and are
//!   not produced here; only the angular reference is.

use crate::model::{BimModel, LocationLine};
use asterion_core::{ElementId, Failure};
use asterion_geom::Vec2;

/// The document settings Spec TEMPDIM-02 states.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DimSettings {
    /// "Clear temporary dimensions to walls": off dimensions from the wall's centre (or core centre),
    /// on from its side faces.
    pub to_wall_faces: bool,
    /// The dimension-sides type is one of the two "core" values: the core centre / core side faces.
    pub core: bool,
}

/// What a reference is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DimRefKind {
    /// The wall's direction (the angular reference, always first for a pick of the whole wall).
    Angular,
    /// The wall's centre face.
    Centre,
    /// The wall's core centre face.
    CoreCentre,
    /// A side face; `side` 0 or 1.
    SideFace,
    /// A core side face; `side` 0 or 1.
    CoreSideFace,
    /// The end face of an end that no neighbour determines; `side` 0 start, 1 end.
    EndFace,
}

impl DimRefKind {
    pub fn name(&self) -> &'static str {
        match self {
            DimRefKind::Angular => "angular",
            DimRefKind::Centre => "centre",
            DimRefKind::CoreCentre => "core_centre",
            DimRefKind::SideFace => "side_face",
            DimRefKind::CoreSideFace => "core_side_face",
            DimRefKind::EndFace => "end_face",
        }
    }
}

/// One reference: a point on the face's plane (the middle of the face along the wall), the normal
/// of the plane as the direction of the dimension, and whether that direction is the reversed one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WallDimRef {
    pub kind: DimRefKind,
    /// 0 / 1 for the side and end faces, 0 otherwise.
    pub side: u8,
    pub point: Vec2,
    pub normal: Vec2,
    pub reversed: bool,
}

/// The references of straight wall `wall` selected as a whole, in the order of Spec TEMPDIM-02.
pub fn wall_dimension_references(model: &BimModel, wall: ElementId, settings: DimSettings) -> Result<Vec<WallDimRef>, Failure> {
    let w = model.wall(wall)?;
    let t = model.wall_type(w.type_id)?;
    let Some(d) = (w.end - w.start).normalized() else { return Ok(Vec::new()) };
    let n = if w.flipped { -d.perp_ccw() } else { d.perp_ccw() };
    let mid = (w.start + w.end) * 0.5;
    let mut out = vec![WallDimRef { kind: DimRefKind::Angular, side: 0, point: mid, normal: d, reversed: false }];
    if w.through.is_some() {
        // cylinders are not planar faces (Spec TEMPDIM-02): nothing is added
        return Ok(out);
    }
    let s = &t.structure;
    let has_core = s.exterior_shell_layers > 0 || s.interior_shell_layers > 0;
    let at = |offset: f64| mid + n * offset;
    let half = s.width() / 2.0;
    if !settings.to_wall_faces {
        let (kind, offset) = if settings.core && has_core {
            (DimRefKind::CoreCentre, s.offset_for_location_line(LocationLine::CoreCenterline))
        } else {
            (DimRefKind::Centre, 0.0)
        };
        out.push(WallDimRef { kind, side: 0, point: at(offset), normal: n, reversed: false });
        out.push(WallDimRef { kind, side: 0, point: at(offset), normal: -n, reversed: true });
    } else if settings.core && has_core {
        out.push(WallDimRef {
            kind: DimRefKind::CoreSideFace,
            side: 0,
            point: at(s.offset_for_location_line(LocationLine::CoreFaceExterior)),
            normal: n,
            reversed: false,
        });
        out.push(WallDimRef {
            kind: DimRefKind::CoreSideFace,
            side: 1,
            point: at(s.offset_for_location_line(LocationLine::CoreFaceInterior)),
            normal: -n,
            reversed: true,
        });
    } else {
        out.push(WallDimRef { kind: DimRefKind::SideFace, side: 0, point: at(half), normal: n, reversed: false });
        out.push(WallDimRef { kind: DimRefKind::SideFace, side: 1, point: at(-half), normal: -n, reversed: true });
    }
    // the ends no neighbour determines
    for end in 0..2usize {
        let joined = crate::joins::end_effect(model, w, end).partner_dir.is_some();
        if !joined {
            let (p, outward) = if end == 0 { (w.start, -d) } else { (w.end, d) };
            out.push(WallDimRef { kind: DimRefKind::EndFace, side: end as u8, point: p, normal: outward, reversed: false });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ops, CompoundStructure, Layer, LayerFunction};

    fn doc_with(layers: Vec<Layer>, ext: usize, int: usize) -> (crate::BimDocument, ElementId, ElementId) {
        let mut doc = crate::new_document();
        let (a, b) = doc
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L", 0.0)?;
                let wt = ops::create_wall_type(m, cx, "W", CompoundStructure::new(layers, ext, int)?)?;
                let v = Vec2::new;
                let nw = |a: Vec2, b: Vec2| ops::NewWall { wall_type: wt, level: l, start: a, end: b, height: 3.0, base_offset: 0.0, flip: false, structural: false };
                let a = ops::create_wall(m, cx, nw(v(0.0, 0.0), v(6.0, 0.0)))?;
                // a perpendicular wall joined at the end of `a`
                let b = ops::create_wall(m, cx, nw(v(6.0, 0.0), v(6.0, 4.0)))?;
                Ok((a, b))
            })
            .value
            .expect("fixture");
        (doc, a, b)
    }

    fn layer(w: f64) -> Layer {
        Layer { width: w, function: LayerFunction::Structure }
    }

    #[test]
    fn a_plain_wall_gives_the_angular_the_centre_twice_and_the_unjoined_end() {
        let (doc, a, _b) = doc_with(vec![layer(0.2)], 0, 0);
        let r = wall_dimension_references(doc.model(), a, DimSettings::default()).unwrap();
        let kinds: Vec<(DimRefKind, u8, bool)> = r.iter().map(|x| (x.kind, x.side, x.reversed)).collect();
        // the wall's end at (6,0) is joined to the perpendicular wall; its start at (0,0) is free
        assert_eq!(
            kinds,
            vec![
                (DimRefKind::Angular, 0, false),
                (DimRefKind::Centre, 0, false),
                (DimRefKind::Centre, 0, true),
                (DimRefKind::EndFace, 0, false),
            ]
        );
        assert_eq!(r[1].point, Vec2::new(3.0, 0.0));
        assert_eq!(r[1].normal, -r[2].normal);
        assert_eq!(r[3].point, Vec2::new(0.0, 0.0));
    }

    #[test]
    fn the_wall_faces_preference_gives_the_two_side_faces_and_the_core_sides_the_core_faces() {
        // 0.1 finish + 0.2 core + 0.1 finish: shells of one layer each
        let (doc, a, _b) = doc_with(vec![layer(0.1), layer(0.2), layer(0.1)], 1, 1);
        let faces = wall_dimension_references(doc.model(), a, DimSettings { to_wall_faces: true, core: false }).unwrap();
        assert_eq!((faces[1].kind, faces[2].kind), (DimRefKind::SideFace, DimRefKind::SideFace));
        assert!((faces[1].point.y - 0.2).abs() < 1e-12 && (faces[2].point.y + 0.2).abs() < 1e-12, "{:?}", faces);
        let core = wall_dimension_references(doc.model(), a, DimSettings { to_wall_faces: true, core: true }).unwrap();
        assert_eq!((core[1].kind, core[2].kind), (DimRefKind::CoreSideFace, DimRefKind::CoreSideFace));
        assert!((core[1].point.y - 0.1).abs() < 1e-12 && (core[2].point.y + 0.1).abs() < 1e-12, "{:?}", core);
        // preference off, core type: the core centre (here the same plane as the centre)
        let cc = wall_dimension_references(doc.model(), a, DimSettings { to_wall_faces: false, core: true }).unwrap();
        assert_eq!(cc[1].kind, DimRefKind::CoreCentre);
        // a wall without shells has no distinct core: the core settings fall back to the centre / side faces
        let (doc2, a2, _) = doc_with(vec![layer(0.2)], 0, 0);
        let fb = wall_dimension_references(doc2.model(), a2, DimSettings { to_wall_faces: true, core: true }).unwrap();
        assert_eq!(fb[1].kind, DimRefKind::SideFace);
    }

    #[test]
    fn an_arc_wall_gives_only_the_angular_reference_and_a_flip_moves_the_sides() {
        let mut doc = crate::new_document();
        let (arc, flipped) = doc
            .transact("fx", false, |m, cx| {
                let l = ops::create_level(m, cx, "L", 0.0)?;
                let wt = ops::create_wall_type(m, cx, "W", CompoundStructure::new(vec![layer(0.2)], 0, 0)?)?;
                let v = Vec2::new;
                let nw = |a: Vec2, b: Vec2, flip: bool| ops::NewWall { wall_type: wt, level: l, start: a, end: b, height: 3.0, base_offset: 0.0, flip, structural: false };
                let arc = ops::create_arc_wall(m, cx, nw(v(0.0, 0.0), v(4.0, 0.0), false), v(2.0, 1.0))?;
                let flipped = ops::create_wall(m, cx, nw(v(0.0, 10.0), v(6.0, 10.0), true))?;
                Ok((arc, flipped))
            })
            .value
            .expect("fixture");
        let r = wall_dimension_references(doc.model(), arc, DimSettings::default()).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].kind, DimRefKind::Angular);
        let f = wall_dimension_references(doc.model(), flipped, DimSettings { to_wall_faces: true, core: false }).unwrap();
        // side 0 is on the orientation side: a flip puts it on the other side
        assert!(f[1].point.y < 10.0 && f[2].point.y > 10.0, "{:?}", f);
        // not a wall: refused
        assert!(wall_dimension_references(doc.model(), ElementId(9_999_999), DimSettings::default()).is_err());
    }
}
