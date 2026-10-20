//! Walls: joins and their order, junctions, crossings, curved ends, profiles, vertices, stacked walls, sweeps, cuts.
//!
//! Every module keeps its crate-root path too (`crate::joins` is `crate::walls::joins`): see the re-exports in `lib.rs`.

pub mod joins;
pub mod join_end;
pub mod join_order;
pub mod junctions;
pub mod crossings;
pub mod curved_ends;
pub mod curved_joins;
pub mod wall_profiles;
pub mod wall_vertices;
pub mod stacked;
pub mod sweeps;
pub mod arc_sweeps;
pub mod cut;
