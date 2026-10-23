//! Spec S-01h.
//!
//! The solids of a stairs are derived ([`crate::stair_solids`]) from the stairs' run type, its stringers, its runs' location lines and its
//! landings' boundary loops; the solids of a railing from its path and its type. A stairs imported through `import_ops::import_stairs`,
//! `import_stairs_run`, `import_stairs_landing` and `import_stairs_railing` has the run, the landing elevation and the railing's order and
//! no more: the functions below add the rest, one stored object each, with no rule re-judged (a stored document is not re-validated).
//!
//! * a landing's boundary loop.
//! * a run's justification.
//! * A railing along a path ([`import_path_railing`]).

use crate::model::BimModel;
use crate::spec::codes;
use crate::stairs_scene::{BalusterInstance, Justification, PathRailing, RailingType, RunExt, RunType, StairsTypeExt};
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use asterion_geom::Vec2;

/// A stored run type, in engine terms; metres.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportedRunType {
    pub name: String,
    pub tread_thickness: f64,
    pub riser_thickness: f64,
    pub nosing_length: f64,
    pub has_treads: bool,
    pub has_risers: bool,
    pub riser_slanted: bool,
    pub is_monolithic: bool,
    pub structural_depth: f64,
}

fn bad(what: &str) -> Failure {
    Failure::error(codes::INSERT_BAD_SIZE, format!("An imported {what} must be finite and not negative."))
}

/// Insert a stored run type. The lengths must be finite and not negative; no name test (a stored document is not re-judged).
pub fn import_run_type(m: &mut BimModel, cx: &mut TxnContext<'_>, t: ImportedRunType) -> Result<ElementId, Failure> {
    let lengths = [t.tread_thickness, t.riser_thickness, t.nosing_length, t.structural_depth];
    if !lengths.iter().all(|v| v.is_finite() && *v >= 0.0) {
        return Err(bad("run type's thicknesses"));
    }
    let id = cx.new_id();
    m.stairs_scene.run_types.insert(
        id,
        RunType {
            id,
            name: t.name,
            tread_thickness: t.tread_thickness,
            riser_thickness: t.riser_thickness,
            nosing_length: t.nosing_length,
            has_risers: t.has_risers,
            has_treads: t.has_treads,
            riser_slanted: t.riser_slanted,
            is_monolithic: t.is_monolithic,
            structural_depth: t.structural_depth,
            total_depth: t.structural_depth,
        },
    );
    Ok(id)
}

/// What a stored stairs type points at besides its own numbers.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportedTypeParts {
    pub run_type: ElementId,
    pub left_support: bool,
    pub right_support: bool,
    pub construction_method: String,
    pub landing_type: String,
    pub landing_thickness: f64,
    pub landing_monolithic: bool,
}

/// Attach a run type, stringers and a landing type to a stairs type.
pub fn attach_run_type(m: &mut BimModel, _cx: &mut TxnContext<'_>, stairs_type: ElementId, p: ImportedTypeParts) -> Result<(), Failure> {
    m.stairs.stairs_type(stairs_type)?;
    if !m.stairs_scene.run_types.contains_key(&p.run_type) {
        return Err(Failure::error(codes::NOT_FOUND, format!("No run type with id {}.", p.run_type)).on(&[p.run_type]));
    }
    if !(p.landing_thickness.is_finite() && p.landing_thickness >= 0.0) {
        return Err(bad("landing thickness"));
    }
    let support = |on: bool| on.then(|| "Stringer".to_owned());
    m.stairs_scene.types.insert(
        stairs_type,
        StairsTypeExt {
            construction_method: p.construction_method,
            left_support: support(p.left_support),
            right_support: support(p.right_support),
            run_type: p.run_type,
            landing_type: p.landing_type,
            landing_thickness: p.landing_thickness,
            landing_monolithic: p.landing_monolithic,
        },
    );
    Ok(())
}

/// Set the boundary loop of a stored landing (at least three points).
pub fn set_landing_boundary(m: &mut BimModel, _cx: &mut TxnContext<'_>, landing: ElementId, boundary: Vec<Vec2>) -> Result<(), Failure> {
    if !m.stairs.landings.contains_key(&landing) {
        return Err(Failure::error(codes::NOT_FOUND, format!("No landing with id {landing}.")).on(&[landing]));
    }
    if boundary.len() < 3 || !boundary.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
        return Err(Failure::error(codes::INSERT_BAD_SIZE, "A landing boundary needs at least three finite points.").on(&[landing]));
    }
    let slot = m.stairs_scene.landings.entry(landing).or_default();
    slot.boundary = boundary;
    Ok(())
}

/// Set the justification of a stored run (a straight run; the other styles keep their `RunExt`).
pub fn set_run_justification(m: &mut BimModel, _cx: &mut TxnContext<'_>, run: ElementId, justification: Justification) -> Result<(), Failure> {
    if !m.stairs.runs.contains_key(&run) {
        return Err(Failure::error(codes::NOT_FOUND, format!("No run with id {run}.")).on(&[run]));
    }
    m.stairs_scene.runs.entry(run).or_insert_with(RunExt::default).justification = justification;
    Ok(())
}

/// Insert a stored railing type (its name; the engine draws it from [`crate::stair_solids::railing_def`]).
pub fn import_railing_type(m: &mut BimModel, cx: &mut TxnContext<'_>, name: &str) -> ElementId {
    if let Some(t) = m.stairs_scene.railing_types.values().find(|t| t.name == name) {
        return t.id;
    }
    let id = cx.new_id();
    m.stairs_scene.railing_types.insert(id, RailingType { id, name: name.to_owned() });
    id
}

/// Insert a stored railing along a path of straight pieces at a level. No rule of the interactive create is applied (a
/// stored document is not re-judged); the path must not be empty and its points must be finite.
pub fn import_path_railing(
    m: &mut BimModel,
    cx: &mut TxnContext<'_>,
    railing_type: ElementId,
    level: ElementId,
    path: Vec<(Vec2, Vec2)>,
) -> Result<ElementId, Failure> {
    m.level(level)?;
    if path.is_empty() || !path.iter().all(|(a, b)| a.x.is_finite() && a.y.is_finite() && b.x.is_finite() && b.y.is_finite()) {
        return Err(Failure::error(codes::INSERT_BAD_SIZE, "A railing path needs at least one piece, with finite points."));
    }
    let id = cx.new_id();
    m.stairs_scene.path_railings.insert(id, PathRailing { id, railing_type, level, path });
    Ok(id)
}

/// Keep the baluster placements a stored railing carries: the railing is drawn with exactly these. Replaces any earlier
/// list.
pub fn import_baluster_instances(m: &mut BimModel, _cx: &mut TxnContext<'_>, railing: ElementId, list: Vec<BalusterInstance>) -> Result<(), Failure> {
    if !(m.stairs.railings.contains_key(&railing) || m.stairs_scene.path_railings.contains_key(&railing)) {
        return Err(Failure::error(codes::NOT_FOUND, format!("No railing with id {railing}.")).on(&[railing]));
    }
    if !list.iter().all(|b| b.position.iter().all(|v| v.is_finite()) && b.rotation.is_finite()) {
        return Err(Failure::error(codes::INSERT_BAD_SIZE, "A baluster placement must be finite.").on(&[railing]));
    }
    m.stairs_scene.baluster_instances.insert(railing, list);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element_geometry::{self, Geometry};
    use crate::polysolid::PartRole;
    use crate::{new_document, ops};

    /// An imported stairs with a run type, a stringer on each side and a railing: the engine builds the body from the stored objects.
    #[test]
    fn an_imported_stairs_gets_its_body_from_the_stored_run_type_and_supports() {
        let mut doc = new_document();
        let mut ids = None;
        let out = doc.transact("import", false, |m, cx| {
            let l0 = ops::create_level(m, cx, "L0", 0.0)?;
            let l1 = ops::create_level(m, cx, "L1", 3.0)?;
            let ty = crate::stairs::create_stairs_type(
                m,
                cx,
                "Imported",
                crate::stairs::StairsTypeParams {
                    max_riser_height: 0.18,
                    min_tread_depth: 0.275,
                    min_run_width: 1.0,
                    use_calculator: false,
                    calculator_target: 0.0,
                    calculator_riser_multiplier: 0.0,
                    calculator_tread_multiplier: 0.0,
                    min_run_height: None,
                    tread_thickness: None,
                },
            )?;
            let rt = import_run_type(
                m,
                cx,
                ImportedRunType {
                    name: "Run".into(),
                    tread_thickness: 0.04,
                    riser_thickness: 0.02,
                    nosing_length: 0.03,
                    has_treads: true,
                    has_risers: true,
                    riser_slanted: true,
                    is_monolithic: false,
                    structural_depth: 0.0,
                },
            )?;
            attach_run_type(
                m,
                cx,
                ty,
                ImportedTypeParts {
                    run_type: rt,
                    left_support: true,
                    right_support: false,
                    construction_method: "Assembled".into(),
                    landing_type: "Non-Monolithic Landing".into(),
                    landing_thickness: 0.0,
                    landing_monolithic: false,
                },
            )?;
            let st = crate::stairs::create_stairs(m, cx, crate::stairs::NewStairs { type_id: ty, base_level: l0, top_level: l1, initial_tread_depth: 0.275 })?;
            let run = crate::import_ops::import_stairs_run(
                m,
                cx,
                crate::import_ops::ImportedStairsRun {
                    stairs: st,
                    start: Vec2::new(0.0, 0.0),
                    end: Vec2::new(4.0, 0.0),
                    bottom_elevation: 0.0,
                    width: 1.2,
                    begin_with_riser: true,
                    end_with_riser: true,
                    run_type_code: 3,
                },
            )?;
            set_run_justification(m, cx, run, Justification::Left)?;
            let rail = crate::import_ops::import_stairs_railing(m, cx, st, ElementId(0), 0)?;
            ids = Some((st, run, rail));
            Ok(())
        });
        assert!(out.committed(), "{:?}", out.failures);
        let (st, run, rail) = ids.unwrap();
        let m = doc.model();
        // one stringer only (the type has one, on the left)
        let supports = crate::stair_solids::support_geometry(m, st);
        assert_eq!(supports.len(), 1, "a stringer on the left only");
        // the left-justified run's body lies to the right of its location line: y from -1.2 to 0
        let (lo, hi) = crate::stair_solids::stairs_bbox(m, st).map(|(l, h)| (l, h)).unwrap();
        assert!(hi[1] > 0.0 && lo[1] < -1.0, "{lo:?} {hi:?}");
        assert!(matches!(element_geometry::geometry_of(m, run), Geometry::Parts(_)));
        assert!(matches!(element_geometry::geometry_of(m, rail), Geometry::Parts(_)), "{:?}", element_geometry::geometry_of(m, rail));
    }

    #[test]
    fn stored_balusters_replace_the_laid_out_ones() {
        let mut doc = new_document();
        let mut ids = None;
        let out = doc.transact("import", false, |m, cx| {
            let l0 = ops::create_level(m, cx, "L0", 0.0)?;
            let rt = import_railing_type(m, cx, "900mm Pipe");
            let r = import_path_railing(m, cx, rt, l0, vec![(Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0))])?;
            ids = Some(r);
            Ok(())
        });
        assert!(out.committed());
        let r = ids.unwrap();
        let generated = element_geometry::geometry_of(doc.model(), r).parts().iter().filter(|p| p.role == PartRole::Baluster).count();
        assert!(generated >= 4);
        let out = doc.transact("balusters", false, |m, cx| {
            import_baluster_instances(
                m,
                cx,
                r,
                vec![
                    BalusterInstance { position: [0.5, 0.0, 0.0], rotation: 0.0, symbol: 1 },
                    BalusterInstance { position: [1.7, 0.0, 0.0], rotation: 0.3, symbol: 1 },
                ],
            )
        });
        assert!(out.committed(), "{:?}", out.failures);
        let g = element_geometry::geometry_of(doc.model(), r);
        let b: Vec<_> = g.parts().iter().filter(|p| p.role == PartRole::Baluster).collect();
        assert_eq!(b.len(), 2, "exactly the stored placements");
        let (lo, _) = b[1].solid.aabb();
        assert!(lo[0] > 1.6 && lo[0] < 1.8);
    }
}
