//! ASTERION Geometry Kernel.
//!
//! * **v0.1** (shipped with straight walls and flat slabs): 2D/3D vectors,
//!   documented tolerances, axis-aligned boxes, and right prisms over a planar
//!   polygon with exact volume, surface area and topology counts.
//! * **v0.2** (this slice): analytic planar **curves** (line segment and
//!   circular arc), curve **intersections**, closed **polycurve** loops,
//!   **regions with holes**, and a generalised **extrusion** over them. Every
//!   quantity (length, area, perimeter, volume, bounding box) is closed-form —
//!   tessellation exists only for display (`tessellate`), never for numbers.
//!
//! Invariants the whole crate obeys:
//!
//! * **No stored derived state.** Every metric is a method computed from the
//!   defining numbers, so a geometry value cannot go stale.
//! * **Deterministic.** Pure `f64` arithmetic in a fixed evaluation order; no
//!   threads, no global state, no iteration over hash maps. The same input
//!   produces bit-identical output on any machine with IEEE-754 doubles.
//! * **SI units** (metres, radians) everywhere; conversions belong to adapters.
//! * **Tolerances are explicit**: operations that need one take `&Tolerances`, whose defaults are the values observed in
//!   the reference application.

pub mod band;
pub mod boolean;
pub mod curve;
pub mod error;
pub mod intersect;
pub mod kernel;
pub mod loop_point;
pub mod offset;
pub mod prism;
pub mod region;
pub mod roof;
pub mod slab;
pub mod solid;
pub mod stokes;
pub mod sweep;
pub mod tolerance;
pub mod vec;
pub mod wall_profile;

pub use band::{centred_band, offset_band, offset_band_detailed, offset_chain, BandJoin};
pub use boolean::{boolean, total_area, union_all, BoolOp};
pub use curve::{normalize_angle_positive, normalize_angle_signed, Aabb2, Arc, Curve, LineSegment};
pub use error::{GeomError, GeomResult};
pub use intersect::{
    arc_contains_point, intersect_arcs, intersect_circles, intersect_curves,
    intersect_curves_unbounded, intersect_line_arc_unbounded, intersect_line_circle,
    intersect_lines_unbounded, intersect_segment_arc, intersect_segments, SegmentHit,
};
pub use kernel::{cross_check, determinism_check, standard_corpus, AnalyticKernel, Discrepancy, GeometryKernel, KernelCase, SolidMetrics};
pub use offset::{offset_region, JoinStyle};
pub use prism::{Aabb, Prism};
pub use region::{distinct_support_count_all, polygon_perimeter, polygon_signed_area, Polycurve, Region};
pub use roof::{CutOffRoof, FootprintRoof, RoofEdge, RoofFacet};
pub use slab::{Plane, Slab, ThicknessConvention};
pub use solid::ExtrudedRegion;
pub use sweep::{SegmentLength, SlopeConvention, Sweep};
pub use tolerance::{arc_curve_length_ft, is_too_short_ft, Tolerances, SHORT_CURVE_TOL_FT};
pub use vec::{Vec2, Vec3};
