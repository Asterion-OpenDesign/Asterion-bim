//! ASTERION Core: services shared by every ASTERION engine.
//!
//! * [`id`] element identity and id allocation policy;
//! * [`txn`] transactional editing of a model value (commit, rollback, undo, redo);
//! * [`diag`] structured failures (warnings and errors) attached to a transaction;
//! * [`units`] SI internal units and exact conversions to foreign unit systems.
//!
//! The core is domain-agnostic: it never knows what a wall is. Domain engines
//! (BIM, drafting, mechanical, ...) put their own model type inside a
//! [`txn::Document`].

pub mod diag;
pub mod id;
pub mod idmap;
pub mod index;
pub mod txn;
pub mod units;

pub use diag::{Failure, Severity};
pub use id::{ElementId, IdAllocator};
pub use idmap::IdMap;
pub use index::IndexCache;
pub use txn::{Document, TransactionError, TransactionStatus};
