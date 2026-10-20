//! Levels, grids, reference planes: datum extents and item numbering.
//!
//! Every module keeps its crate-root path too (`crate::datums` is `crate::levels_grids::datums`): see the re-exports in `lib.rs`.

pub mod datums;
pub mod datum_extents;
pub mod item_number;
