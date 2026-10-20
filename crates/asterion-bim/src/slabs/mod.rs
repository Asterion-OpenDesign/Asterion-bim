//! Floors, roofs and toposolids: shapes, rafter cuts.
//!
//! Every module keeps its crate-root path too (`crate::roof` is `crate::slabs::roof`): see the re-exports in `lib.rs`.

pub mod roof;
pub mod slab_shape;
pub mod rafter_cut;
pub mod toposolid;
