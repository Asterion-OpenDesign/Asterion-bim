//! Right prisms over a planar polygon, with exact metrics.
//!
//! The generalised form (arcs, holes) is `solid::ExtrudedRegion`, which reproduces the same numbers for a polygonal single
//! loop — see `solid::tests::prism_and_extruded_region_agree_on_every_metric`.
//!
//! The plan-area and perimeter formulas are shared with `region` so there is a
//! single implementation of the shoelace sum.

use crate::region::{polygon_perimeter, polygon_signed_area};
use crate::vec::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

/// A vertical right prism: a convex polygon in plan (counter-clockwise or
/// clockwise, at least 3 vertices) extruded from `z0` to `z1` (`z1 > z0`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prism {
    pub footprint: Vec<Vec2>,
    pub z0: f64,
    pub z1: f64,
}

impl Prism {
    pub fn new(footprint: Vec<Vec2>, z0: f64, z1: f64) -> Self {
        assert!(footprint.len() >= 3, "a prism needs at least 3 footprint vertices");
        assert!(z1 > z0, "prism height must be positive");
        Prism { footprint, z0, z1 }
    }

    pub fn height(&self) -> f64 {
        self.z1 - self.z0
    }

    /// Unsigned footprint area (shoelace formula).
    pub fn footprint_area(&self) -> f64 {
        polygon_signed_area(&self.footprint).abs()
    }

    pub fn footprint_perimeter(&self) -> f64 {
        polygon_perimeter(&self.footprint)
    }

    pub fn volume(&self) -> f64 {
        self.footprint_area() * self.height()
    }

    pub fn surface_area(&self) -> f64 {
        2.0 * self.footprint_area() + self.footprint_perimeter() * self.height()
    }

    pub fn face_count(&self) -> usize {
        self.footprint.len() + 2
    }

    pub fn edge_count(&self) -> usize {
        self.footprint.len() * 3
    }

    pub fn vertices(&self) -> Vec<Vec3> {
        let mut v: Vec<Vec3> = self
            .footprint
            .iter()
            .flat_map(|p| [Vec3::from_xy(*p, self.z0), Vec3::from_xy(*p, self.z1)])
            .collect();
        v.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)).then(a.z.total_cmp(&b.z)));
        v
    }

    pub fn aabb(&self) -> Aabb {
        let (mut lo, mut hi) = (self.footprint[0], self.footprint[0]);
        for p in &self.footprint {
            lo = Vec2::new(lo.x.min(p.x), lo.y.min(p.y));
            hi = Vec2::new(hi.x.max(p.x), hi.y.max(p.y));
        }
        Aabb { min: Vec3::from_xy(lo, self.z0), max: Vec3::from_xy(hi, self.z1) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_box() -> Prism {
        Prism::new(
            vec![Vec2::new(0.0, 0.0), Vec2::new(5.0, 0.0), Vec2::new(5.0, 0.2), Vec2::new(0.0, 0.2)],
            0.0,
            3.0,
        )
    }

    #[test]
    fn box_metrics_match_closed_form() {
        let b = unit_box();
        assert!((b.volume() - 3.0).abs() < 1e-12);
        assert!((b.surface_area() - 33.2).abs() < 1e-12);
        assert_eq!((b.face_count(), b.edge_count(), b.vertices().len()), (6, 12, 8));
    }

    #[test]
    fn orientation_of_footprint_does_not_change_metrics() {
        let mut b = unit_box();
        b.footprint.reverse();
        assert!((b.volume() - 3.0).abs() < 1e-12);
    }
}
