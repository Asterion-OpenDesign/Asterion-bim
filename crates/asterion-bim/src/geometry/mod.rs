//! Regeneration of dependent quantities and the geometry every view draws from (one body source).
//!
//! Every module keeps its crate-root path too (`crate::regen` is `crate::geometry::regen`): see the re-exports in `lib.rs`.

pub mod regen;
pub mod element_geometry;
pub mod mesh3d;
pub mod boxtopo;
pub mod sketch;
pub mod polysolid;
