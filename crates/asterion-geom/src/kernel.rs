//! The geometry-kernel boundary (ADR-0007, action item 9).
//!
//! ADR-0007 keeps ASTERION's own analytic kernel in the modelling path and
//! names `truck` as the upgrade path for genuinely free-form needs. This module
//! is the seam that makes that upgrade safe, built **before** anything external
//! exists, so that:
//!
//! 1. **The boundary is written in ASTERION's types.** Every method takes and
//!    returns `Region`, `ExtrudedRegion`, `Sweep`, `SolidMetrics` — never a
//!    third-party type. A future adapter converts at its own edge; nothing of
//!    another kernel can leak into `asterion-bim` or into persisted data.
//! 2. **The analytic kernel is the oracle.** [`cross_check`] runs a candidate
//!    kernel and the oracle over a fixed corpus of cases with known closed
//!    forms (boxes, discs, lenses, the measured opening sequence, curved and
//!    sloped sweeps) and reports every quantity that disagrees beyond a
//!    relative tolerance. A kernel that tessellates arcs for its quantities
//!    fails this gate by construction — that is tested below.
//!
//! The trait is object-safe (`&dyn GeometryKernel`), so the choice of kernel
//! can be made at run time without generics spreading through the engine.
//!
//! Status: `asterion-bim` still calls the free functions of this crate
//! directly, which is exactly what [`AnalyticKernel`] wraps; moving it onto the
//! trait is a mechanical change for the engine lane whenever a second kernel is
//! actually introduced.

use crate::boolean::{boolean, total_area, union_all, BoolOp};
use crate::curve::{Arc, Curve};
use crate::error::GeomResult;
use crate::offset::{offset_region, JoinStyle};
use crate::prism::Aabb;
use crate::region::{Polycurve, Region};
use crate::solid::ExtrudedRegion;
use crate::sweep::{SlopeConvention, Sweep};
use crate::tolerance::Tolerances;
use crate::vec::Vec2;

/// Everything the engine reads from a solid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolidMetrics {
    pub volume: f64,
    pub surface_area: f64,
    pub aabb: Aabb,
    /// Face count under the reference's measured convention.
    pub face_count: usize,
}

impl SolidMetrics {
    fn bits(&self) -> [u64; 8] {
        [
            self.volume.to_bits(),
            self.surface_area.to_bits(),
            self.aabb.min.x.to_bits(),
            self.aabb.min.y.to_bits(),
            self.aabb.min.z.to_bits(),
            self.aabb.max.x.to_bits(),
            self.aabb.max.y.to_bits(),
            self.aabb.max.z.to_bits(),
        ]
    }
}

/// The operations the engine needs from a geometry kernel.
pub trait GeometryKernel {
    /// Stable identifier, recorded in comparison reports.
    fn name(&self) -> &'static str;
    fn region_boolean(
        &self,
        a: &Region,
        b: &Region,
        op: BoolOp,
        tol: &Tolerances,
    ) -> GeomResult<Vec<Region>>;
    fn region_offset(
        &self,
        r: &Region,
        distance: f64,
        style: JoinStyle,
        tol: &Tolerances,
    ) -> GeomResult<Vec<Region>>;
    fn region_area(&self, r: &Region) -> f64;
    fn extrusion_metrics(&self, e: &ExtrudedRegion, tol: &Tolerances) -> GeomResult<SolidMetrics>;
    fn sweep_metrics(&self, s: &Sweep, tol: &Tolerances) -> GeomResult<SolidMetrics>;
}

/// ASTERION's own closed-form kernel: the default and the oracle.
#[derive(Clone, Copy, Debug, Default)]
pub struct AnalyticKernel;

impl GeometryKernel for AnalyticKernel {
    fn name(&self) -> &'static str {
        "asterion-analytic"
    }
    fn region_boolean(
        &self,
        a: &Region,
        b: &Region,
        op: BoolOp,
        tol: &Tolerances,
    ) -> GeomResult<Vec<Region>> {
        boolean(a, b, op, tol)
    }
    fn region_offset(
        &self,
        r: &Region,
        distance: f64,
        style: JoinStyle,
        tol: &Tolerances,
    ) -> GeomResult<Vec<Region>> {
        offset_region(r, distance, style, tol)
    }
    fn region_area(&self, r: &Region) -> f64 {
        r.area()
    }
    fn extrusion_metrics(&self, e: &ExtrudedRegion, tol: &Tolerances) -> GeomResult<SolidMetrics> {
        Ok(SolidMetrics {
            volume: e.volume(),
            surface_area: e.surface_area(),
            aabb: e.aabb(),
            face_count: e.face_count_merged(tol),
        })
    }
    fn sweep_metrics(&self, s: &Sweep, tol: &Tolerances) -> GeomResult<SolidMetrics> {
        Ok(SolidMetrics {
            volume: s.volume(tol)?,
            surface_area: s.surface_area(tol)?,
            aabb: s.aabb(tol)?,
            face_count: s.profile().distinct_support_count(tol) + 2,
        })
    }
}

/// One quantity on which two kernels disagree.
#[derive(Clone, Debug, PartialEq)]
pub struct Discrepancy {
    pub case: &'static str,
    pub quantity: &'static str,
    pub oracle: f64,
    pub candidate: f64,
}

impl Discrepancy {
    pub fn relative_error(&self) -> f64 {
        (self.candidate - self.oracle).abs() / self.oracle.abs().max(1e-300)
    }
}

/// A case of the comparison corpus: a name and the solids it exercises.
pub struct KernelCase {
    pub name: &'static str,
    pub extrusions: Vec<ExtrudedRegion>,
    pub sweeps: Vec<Sweep>,
    /// Region operations whose net area is compared: `(a, b, op)`.
    pub booleans: Vec<(Region, Region, BoolOp)>,
}

/// The fixed corpus: every case has a closed form pinned elsewhere in this
/// crate, so the oracle's own values are already verified.
pub fn standard_corpus(tol: &Tolerances) -> Vec<KernelCase> {
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64| {
        Region::from_loop(Polycurve::rectangle(Vec2::new(x0, y0), Vec2::new(x1, y1), tol).unwrap())
    };
    let disc = |cx: f64, cy: f64, r: f64| {
        Region::from_loop(Polycurve::circle(Vec2::new(cx, cy), r, tol).unwrap())
    };
    let mut corpus: Vec<KernelCase> = Vec::new();

    // Measured floor: 6 x 4 slab, 0.2 thick.
    corpus.push(KernelCase {
        name: "floor-6x4",
        extrusions: vec![ExtrudedRegion::new(rect(0.0, 0.0, 6.0, 4.0), -0.2, 0.0).unwrap()],
        sweeps: vec![],
        booleans: vec![],
    });

    // Cylinder and hollow tube: the arc cases a tessellating kernel gets wrong.
    let ring = Region::new(
        Polycurve::circle(Vec2::new(0.0, 0.0), 1.0, tol).unwrap(),
        vec![Polycurve::circle(Vec2::new(0.0, 0.0), 0.5, tol).unwrap()],
        tol,
    )
    .unwrap();
    corpus.push(KernelCase {
        name: "cylinder-and-tube",
        extrusions: vec![
            ExtrudedRegion::new(disc(0.0, 0.0, 2.0), 0.0, 3.0).unwrap(),
            ExtrudedRegion::new(ring, 0.0, 10.0).unwrap(),
        ],
        sweeps: vec![],
        booleans: vec![],
    });

    // Lens of two unit discs; window and door cut from the measured host.
    corpus.push(KernelCase {
        name: "booleans",
        extrusions: vec![],
        sweeps: vec![],
        booleans: vec![
            (disc(0.0, 0.0, 1.0), disc(1.0, 0.0, 1.0), BoolOp::Intersection),
            (disc(0.0, 0.0, 1.0), disc(1.0, 0.0, 1.0), BoolOp::Union),
            (rect(0.0, 0.0, 6.0, 3.0), rect(3.0, 0.0, 3.9, 2.1), BoolOp::Difference),
        ],
    });

    // Sweeps: curved wall body, torus, 1:12 ramp under the working convention.
    let arc = |r: f64, s: f64| vec![Curve::Arc(Arc::new(Vec2::new(0.0, 0.0), r, 0.0, s).unwrap())];
    let full_circle = vec![
        Curve::Arc(Arc::new(Vec2::new(0.0, 0.0), 3.0, 0.0, std::f64::consts::PI).unwrap()),
        Curve::Arc(
            Arc::new(Vec2::new(0.0, 0.0), 3.0, std::f64::consts::PI, std::f64::consts::PI).unwrap(),
        ),
    ];
    corpus.push(KernelCase {
        name: "sweeps",
        extrusions: vec![],
        sweeps: vec![
            Sweep::new(rect(-0.1, 0.0, 0.1, 3.0), arc(5.0, std::f64::consts::FRAC_PI_2), 0.0, tol)
                .unwrap(),
            Sweep::new(disc(0.0, 1.5, 1.0), full_circle, 0.0, tol).unwrap(),
            Sweep::new_sloped(
                rect(-0.6, -0.2, 0.6, 0.0),
                vec![Curve::line(Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0))],
                0.0,
                1.0 / 12.0,
                SlopeConvention::VerticalProfile,
                tol,
            )
            .unwrap(),
        ],
        booleans: vec![],
    });
    corpus
}

/// Compare a candidate kernel against the oracle over the corpus. Quantities
/// within `rel_tol` (relative) agree; anything else is reported. Topology
/// counts must agree exactly.
pub fn cross_check(
    candidate: &dyn GeometryKernel,
    oracle: &dyn GeometryKernel,
    corpus: &[KernelCase],
    tol: &Tolerances,
    rel_tol: f64,
) -> GeomResult<Vec<Discrepancy>> {
    let mut out: Vec<Discrepancy> = Vec::new();
    let mut check = |case: &'static str, quantity: &'static str, o: f64, c: f64| {
        let d = Discrepancy { case, quantity, oracle: o, candidate: c };
        if !(d.relative_error() <= rel_tol) {
            out.push(d);
        }
    };
    for case in corpus {
        for e in &case.extrusions {
            let (o, c) = (oracle.extrusion_metrics(e, tol)?, candidate.extrusion_metrics(e, tol)?);
            check(case.name, "volume", o.volume, c.volume);
            check(case.name, "surface_area", o.surface_area, c.surface_area);
            check(case.name, "face_count", o.face_count as f64, c.face_count as f64);
        }
        for s in &case.sweeps {
            let (o, c) = (oracle.sweep_metrics(s, tol)?, candidate.sweep_metrics(s, tol)?);
            check(case.name, "sweep_volume", o.volume, c.volume);
            check(case.name, "sweep_surface", o.surface_area, c.surface_area);
        }
        for (a, b, op) in &case.booleans {
            let o = total_area(&oracle.region_boolean(a, b, *op, tol)?);
            let c: f64 = candidate
                .region_boolean(a, b, *op, tol)?
                .iter()
                .map(|r| candidate.region_area(r))
                .sum();
            check(case.name, "boolean_area", o, c);
        }
    }
    Ok(out)
}

/// Run every case `runs` times and require bit-identical metrics.
pub fn determinism_check(
    kernel: &dyn GeometryKernel,
    corpus: &[KernelCase],
    tol: &Tolerances,
    runs: usize,
) -> GeomResult<Vec<&'static str>> {
    let mut unstable: Vec<&'static str> = Vec::new();
    for case in corpus {
        let snapshot = |k: &dyn GeometryKernel| -> GeomResult<Vec<u64>> {
            let mut v: Vec<u64> = Vec::new();
            for e in &case.extrusions {
                v.extend(k.extrusion_metrics(e, tol)?.bits());
            }
            for s in &case.sweeps {
                v.extend(k.sweep_metrics(s, tol)?.bits());
            }
            for (a, b, op) in &case.booleans {
                let regions = k.region_boolean(a, b, *op, tol)?;
                v.push(regions.len() as u64);
                for r in &regions {
                    v.push(k.region_area(r).to_bits());
                }
            }
            Ok(v)
        };
        let first = snapshot(kernel)?;
        for _ in 1..runs {
            if snapshot(kernel)? != first {
                unstable.push(case.name);
                break;
            }
        }
    }
    Ok(unstable)
}

/// Convenience used by tests and by future adapters: union a set of regions
/// through a kernel by folding its boolean.
pub fn kernel_union_all(
    kernel: &dyn GeometryKernel,
    regions: &[Region],
    tol: &Tolerances,
) -> GeomResult<Vec<Region>> {
    if kernel.name() == AnalyticKernel.name() {
        return union_all(regions, tol);
    }
    let mut acc: Vec<Region> = Vec::new();
    for r in regions {
        let mut current = r.clone();
        let mut rest: Vec<Region> = Vec::new();
        for existing in acc.drain(..) {
            let merged = kernel.region_boolean(&current, &existing, BoolOp::Union, tol)?;
            if merged.len() == 1 {
                current = merged.into_iter().next().expect("checked length");
            } else {
                rest.push(existing);
            }
        }
        rest.push(current);
        acc = rest;
    }
    Ok(acc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tol() -> Tolerances {
        Tolerances::default()
    }

    /// A deliberately wrong kernel: it computes every area from a polyline
    /// tessellation of the boundary (sagitta 1 mm) — exactly what ADR-0007
    /// forbids for quantities. It exists to prove the gate catches it.
    struct TessellatingKernel;

    fn tessellated_area(r: &Region) -> f64 {
        let poly_area = |pts: &[Vec2]| crate::region::polygon_signed_area(pts).abs();
        let outer = poly_area(&r.outer().tessellate(0.001));
        let holes: f64 = r.holes().iter().map(|h| poly_area(&h.tessellate(0.001))).sum();
        outer - holes
    }
    fn tessellated_length(r: &Region) -> f64 {
        r.loops()
            .map(|l| crate::region::polygon_perimeter(&l.tessellate(0.001)))
            .sum()
    }

    impl GeometryKernel for TessellatingKernel {
        fn name(&self) -> &'static str {
            "test-tessellating"
        }
        fn region_boolean(
            &self,
            a: &Region,
            b: &Region,
            op: BoolOp,
            tol: &Tolerances,
        ) -> GeomResult<Vec<Region>> {
            boolean(a, b, op, tol)
        }
        fn region_offset(
            &self,
            r: &Region,
            d: f64,
            s: JoinStyle,
            tol: &Tolerances,
        ) -> GeomResult<Vec<Region>> {
            offset_region(r, d, s, tol)
        }
        fn region_area(&self, r: &Region) -> f64 {
            tessellated_area(r)
        }
        fn extrusion_metrics(&self, e: &ExtrudedRegion, tol: &Tolerances) -> GeomResult<SolidMetrics> {
            let a = tessellated_area(e.region());
            let p = tessellated_length(e.region());
            Ok(SolidMetrics {
                volume: a * e.height(),
                surface_area: 2.0 * a + p * e.height(),
                aabb: e.aabb(),
                face_count: e.face_count_merged(tol),
            })
        }
        fn sweep_metrics(&self, s: &Sweep, tol: &Tolerances) -> GeomResult<SolidMetrics> {
            AnalyticKernel.sweep_metrics(s, tol)
        }
    }

    #[test]
    fn the_analytic_kernel_is_its_own_oracle() {
        let t = tol();
        let corpus = standard_corpus(&t);
        let d = cross_check(&AnalyticKernel, &AnalyticKernel, &corpus, &t, 0.0).unwrap();
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn the_analytic_kernel_is_bit_reproducible_over_the_corpus() {
        let t = tol();
        let corpus = standard_corpus(&t);
        let unstable = determinism_check(&AnalyticKernel, &corpus, &t, 5).unwrap();
        assert!(unstable.is_empty(), "unstable cases: {unstable:?}");
    }

    /// The gate does its job: a kernel that tessellates arcs agrees exactly on
    /// the polygonal floor and the polygonal opening, and is caught on every
    /// case with arcs.
    ///
    /// Measured here at a 1 mm sagitta: relative errors of **3.6e-4 to
    /// 2.2e-3** (cylinder volume 6.6e-4, tube volume 8.2e-4, lens area 2.2e-3).
    /// Every one of them is above the comparison harness's 1e-4, i.e. a
    /// tessellating kernel would not merely be "less exact": it would fail
    /// parity on every curved element. That is the quantitative reason behind
    /// ADR-0007's rule that no quantity may come from a tessellation.
    #[test]
    fn a_tessellating_kernel_fails_the_gate_exactly_on_the_arc_cases() {
        let t = tol();
        let corpus = standard_corpus(&t);
        let d = cross_check(&TessellatingKernel, &AnalyticKernel, &corpus, &t, 1e-9).unwrap();
        assert!(!d.is_empty(), "the gate must catch tessellated quantities");
        for x in &d {
            assert!(
                x.relative_error() > 1e-4,
                "{x:?} would slip under the harness tolerance"
            );
        }
        // Polygonal cases agree exactly...
        assert!(d.iter().all(|x| x.case != "floor-6x4"), "{d:?}");
        assert!(
            !d.iter().any(|x| x.case == "booleans" && x.oracle == 18.0 - 0.9 * 2.1),
            "{d:?}"
        );
        // ...and the arc cases are all caught.
        assert!(d.iter().any(|x| x.case == "cylinder-and-tube" && x.quantity == "volume"));
        assert!(d.iter().any(|x| x.case == "booleans" && x.quantity == "boolean_area"));
        // A chord always lies inside its arc, so the tessellated areas are low.
        for x in d.iter().filter(|x| x.quantity == "volume") {
            assert!(x.candidate < x.oracle, "{x:?}");
        }
    }

    #[test]
    fn the_trait_is_object_safe_and_selectable_at_run_time() {
        let t = tol();
        let kernels: Vec<Box<dyn GeometryKernel>> =
            vec![Box::new(AnalyticKernel), Box::new(TessellatingKernel)];
        let names: Vec<&str> = kernels.iter().map(|k| k.name()).collect();
        assert_eq!(names, vec!["asterion-analytic", "test-tessellating"]);
        // Folding union through the trait gives the analytic union's area.
        let rects: Vec<Region> = [(0.0, 2.0), (1.0, 3.0), (5.0, 6.0)]
            .iter()
            .map(|(a, b)| {
                Region::from_loop(
                    Polycurve::rectangle(Vec2::new(*a, 0.0), Vec2::new(*b, 1.0), &t).unwrap(),
                )
            })
            .collect();
        for k in &kernels {
            let u = kernel_union_all(k.as_ref(), &rects, &t).unwrap();
            assert_eq!(u.len(), 2);
            let a: f64 = u.iter().map(|r| k.region_area(r)).sum();
            assert!((a - 4.0).abs() < 1e-12, "{} gave {a}", k.name());
        }
    }
}
