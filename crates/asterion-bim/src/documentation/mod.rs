//! Annotation, dimensions, schedules, sheets and the documentation rows.
//!
//! Every module keeps its crate-root path too (`crate::annot` is `crate::documentation::annot`): see the re-exports in `lib.rs`.

pub mod annot;
pub mod dims;
pub mod tempdim;
pub mod schedules;
pub mod sheets;
pub mod docs;
#[cfg(test)]
mod docs_rows_tests;
