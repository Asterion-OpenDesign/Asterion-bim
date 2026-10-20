//! The rafter cut of a footprint roof's eaves (`ROOF_EAVE_CUT_PARAM`, "Rafter Cut", with "Fascia Depth").
//!
//! OBSERVED (a 6 x 4 gable of 0.3 at slope 0.5).
//!
//! * the values are 33615 "Plumb Cut" (the default: the eave face is vertical, the roof is the plain slab), 33619
//!   "Two Cut - Plumb" and 33618 "Two Cut - Square"; any other integer is accepted by the parameter but the roof cannot
//!   regenerate: "Can't make footprint roof." is posted and the transaction rolls back;
//! * Two Cut - Plumb, depth `d`: the vertical eave face is kept from the top edge down `d`, below that the eave is cut
//!   horizontally until the underside (Volume 8.0498 -> 7.8298 for `d` 0.2; `d` 0: the cut is level with the top edge,
//!   6.6998, no extra face; `d` deeper than the plumb face `t sqrt(1 + g^2)`: nothing is cut);
//! * Two Cut - Square, depth `d`: a face perpendicular to the slope, `d` long from the top edge, then the horizontal cut
//!   (7.6598 for 0.2, 7.0298 for 0.05);
//! * Area does not change; a cut eave gains one face, three edges and two vertices; the box starts at the cut level.
//!
//! ENGINE CHOICE: the cut is applied to slope-defining edges whose two neighbours do not define a slope (the eaves of a
//! gable, the only case measured); a hip's eaves are left plumb.

use crate::model::{BimModel, Slab, SlabRole};
use asterion_core::txn::TxnContext;
use asterion_core::{ElementId, Failure};
use serde::{Deserialize, Serialize};

pub const PLUMB_CUT: i64 = 33615;
pub const TWO_CUT_SQUARE: i64 = 33618;
pub const TWO_CUT_PLUMB: i64 = 33619;

pub mod codes {
    pub const NOT_A_ROOF: &str = "rafter_cut.not_a_roof";
    pub const CANNOT_MAKE_ROOF: &str = "rafter_cut.cannot_make_roof";
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RafterCut {
    pub code: i64,
    pub fascia_depth: f64,
}

impl Default for RafterCut {
    fn default() -> Self {
        RafterCut { code: PLUMB_CUT, fascia_depth: 0.0 }
    }
}

/// The rafter cut of a roof (the default when none was set).
pub fn of(m: &BimModel, roof: ElementId) -> RafterCut {
    m.rafter_cuts.get(&roof).copied().unwrap_or_default()
}

/// Set the rafter cut value and / or the fascia depth of a roof.
pub fn set_rafter_cut(m: &mut BimModel, cx: &mut TxnContext<'_>, roof: ElementId, code: Option<i64>, fascia_depth: Option<f64>) -> Result<(), Failure> {
    let f = m.floors.get(&roof).ok_or_else(|| Failure::error(codes::NOT_A_ROOF, "not a roof"))?;
    if f.role != SlabRole::Roof {
        return Err(Failure::error(codes::NOT_A_ROOF, "not a roof"));
    }
    let mut r = of(m, roof);
    if let Some(c) = code {
        r.code = c;
    }
    if let Some(d) = fascia_depth {
        if !(d.is_finite() && d >= 0.0) {
            return Err(Failure::error(codes::NOT_A_ROOF, "the fascia depth must be a length of at least 0"));
        }
        r.fascia_depth = d;
    }
    m.rafter_cuts.insert(roof, r);
    if ![PLUMB_CUT, TWO_CUT_PLUMB, TWO_CUT_SQUARE].contains(&r.code) {
        cx.report(Failure::error(codes::CANNOT_MAKE_ROOF, "Can't make footprint roof."));
    }
    Ok(())
}

/// The cross-section of one cut eave: `(removed area, change of the boundary length, cut level above the eave base,
/// extra face)`, with `vt` the vertical thickness and `g` the slope; `None` when nothing is cut.
fn eave_cut(code: i64, d: f64, g: f64, vt: f64) -> Option<(f64, f64, f64, bool)> {
    if !(g > 0.0) {
        return None;
    }
    let k = (1.0 + g * g).sqrt();
    match code {
        TWO_CUT_PLUMB => {
            if d >= vt - 1e-12 {
                return None;
            }
            let h = vt - d; // cut level above the eave's underside
            let y2 = h / g;
            let area = 0.5 * h * y2;
            let delta = d + y2 - vt - y2 * k;
            Some((area, delta, h, d > 1e-12))
        }
        TWO_CUT_SQUARE => {
            let (p1y, h) = (d * g / k, vt - d / k);
            if h <= 1e-12 {
                return None;
            }
            let y2 = h / g;
            if p1y > y2 {
                return None;
            }
            // removed quadrilateral: top edge (0, vt), eave base (0, 0), (y2, h), (p1y, h)
            let area = (0.5 * (vt * p1y + h * (y2 - p1y))).abs();
            let delta = d + (y2 - p1y) - vt - y2 * k;
            Some((area, delta, h, true))
        }
        _ => None,
    }
}

/// The rafter cut applied to a roof's quantities (see the module docs).
pub fn apply(m: &BimModel, f: &Slab, s: &mut crate::regen::SlabState) {
    if f.role != SlabRole::Roof {
        return;
    }
    let r = of(m, f.id);
    if r.code == PLUMB_CUT {
        return;
    }
    let n = f.boundary.len();
    let defines = |i: usize| f.segments.get(i).map_or(false, |g| g.defines_slope && g.slope > 0.0);
    if f.segments.len() != n || f.boundary_arcs.iter().any(Option::is_some) {
        return;
    }
    let Ok(t) = m.slab_type(f.type_id).map(|t| t.thickness) else { return };
    let base_z = s.bbox_min.z;
    let mut low = f64::INFINITY;
    let mut all_cut = true;
    let mut any = false;
    for i in 0..n {
        if !defines(i) {
            continue;
        }
        if defines((i + n - 1) % n) || defines((i + 1) % n) {
            all_cut = false;
            continue; // ENGINE CHOICE
        }
        let g = f.segments[i].slope;
        let vt = t * (1.0 + g * g).sqrt();
        let Some((area, delta, h, face)) = eave_cut(r.code, r.fascia_depth, g, vt) else {
            all_cut = false;
            continue;
        };
        let len = (f.boundary[(i + 1) % n] - f.boundary[i]).length();
        any = true;
        s.volume -= area * len;
        s.surface_area += delta * len - 2.0 * area;
        if face {
            s.face_count += 1;
            s.edge_count += 3;
            s.vertex_count += 2;
        }
        if let Some(sl) = s.sloped.as_mut() {
            sl.volume -= area * len;
            sl.surface_area += delta * len - 2.0 * area;
            if face {
                sl.face_count += 1;
                sl.edge_count += 3;
                sl.vertex_count += 2;
            }
        }
        low = low.min(base_z + f.segments[i].offset_from_base + h);
    }
    if any && all_cut && low.is_finite() {
        s.bbox_min.z = low;
        if let Some(sl) = s.sloped.as_mut() {
            sl.bbox_min.z = low;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round 34b: the cut cross-sections of the 0.3 gable at slope 0.5 (vt 0.33541) reproduce the measured Volumes
    /// (8.04984 less 12 x the removed area).
    #[test]
    fn the_eave_cuts_reproduce_round_34b() {
        let vt = 0.3 * 1.25_f64.sqrt();
        let v = |code, d| 8.049844718999248 - 12.0 * eave_cut(code, d, 0.5, vt).map_or(0.0, |c| c.0);
        assert!((v(TWO_CUT_PLUMB, 0.2) - 7.829813662799093).abs() < 1e-9);
        assert!((v(TWO_CUT_SQUARE, 0.2) - 7.659844718999254).abs() < 1e-9);
        assert!((v(TWO_CUT_SQUARE, 0.05) - 7.029844718999248).abs() < 1e-9);
        assert!((v(TWO_CUT_PLUMB, 0.0) - 6.6998447189992465).abs() < 1e-9);
        assert!((v(TWO_CUT_PLUMB, 0.6) - 8.049844718999248).abs() < 1e-12);
    }
}
