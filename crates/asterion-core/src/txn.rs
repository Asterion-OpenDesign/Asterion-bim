//! Transactional editing.
//!
//! A [`Document`] owns a model value `M`. Every modification happens inside a named
//! transaction through [`Document::transact`]. A transaction that reports an
//! [`crate::diag::Severity::Error`] failure (or returns `Err`) is rolled back; warnings are kept
//! with the committed result. Committed transactions can be undone and redone.
//!
//! Implementation note (v0.1): rollback/undo restore a full snapshot of `M`
//! (`M: Clone`). This is O(model size) per transaction and is a deliberate first
//! step; ADR-0005 records the plan to move to a change journal once models grow.
//!
//! Element ids are allocated through [`TxnContext::new_id`] from an allocator held
//! by the document, outside `M`, so the reuse policy on rollback is explicit
//! ([`IdRollbackPolicy`]).

use crate::diag::Failure;
use crate::id::{ElementId, IdAllocator};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// The source of [`Document::generation`] values. Every value it hands out is used once, by one document at one moment, so two
/// states of the process's models never share a generation (a document that is dropped and another allocated at the same address
/// cannot be mistaken for each other either).
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

/// The code of the failure a transaction carries when an operation panicked (the transaction is rolled back).
pub const INTERNAL_ERROR: &str = "engine.internal_error";

fn fresh_generation() -> u64 {
    NEXT_GENERATION.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionStatus {
    Committed,
    RolledBack,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TransactionError {
    NothingToUndo,
    NothingToRedo,
}

/// What happens to ids consumed by a transaction that is rolled back.
///
/// Spec P-01: ids come from one counter that the reference product only ADVANCES - no operation sets, lowers or resets it;
/// a rollback deletes the elements created in the transaction and leaves the counter untouched, and undo / redo keep an
/// element under the same id. [`IdRollbackPolicy::Keep`] is that rule; `Restore` exists only to probe the opposite
/// behaviour in tests and is not the reference product's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdRollbackPolicy {
    /// The allocator is restored: the next creation reuses the rolled-back id.
    Restore,
    /// The allocator keeps advancing: rolled-back ids are never reused.
    Keep,
}

/// Mutable access granted to code running inside a transaction.
pub struct TxnContext<'a> {
    ids: &'a mut IdAllocator,
    failures: &'a mut Vec<Failure>,
}

impl TxnContext<'_> {
    pub fn new_id(&mut self) -> ElementId {
        self.ids.allocate()
    }

    pub fn report(&mut self, failure: Failure) {
        self.failures.push(failure);
    }

    /// True when this transaction already posted a failure with `code` on exactly `elements`
    /// (order-insensitive): lets a check that can run several times in one transaction post once.
    pub fn has_failure(&self, code: &str, elements: &[ElementId]) -> bool {
        self.failures.iter().any(|f| {
            f.code == code && f.elements.len() == elements.len() && elements.iter().all(|e| f.elements.contains(e))
        })
    }
}

/// Result of one transaction.
#[derive(Debug)]
pub struct Outcome<R> {
    pub name: String,
    pub status: TransactionStatus,
    /// Value returned by the edit closure (`None` when it returned `Err`).
    pub value: Option<R>,
    /// Ids allocated while the transaction was open, in allocation order.
    pub allocated: Vec<ElementId>,
    pub failures: Vec<Failure>,
}

impl<R> Outcome<R> {
    pub fn committed(&self) -> bool {
        self.status == TransactionStatus::Committed
    }
}

struct Committed<M> {
    name: String,
    before: M,
}

pub struct Document<M: Clone> {
    model: M,
    ids: IdAllocator,
    policy: IdRollbackPolicy,
    undo: Vec<Committed<M>>,
    redo: Vec<Committed<M>>,
    undo_limit: usize,
    /// A number that names the state of `model`: renewed (from a process-wide counter, never repeated) by every operation that can
    /// change the model - a transaction (committed or rolled back), an undo, a redo - and when the document is made. The model is
    /// private and mutable only through those operations, so two reads that see the same generation see the same model.
    generation: u64,
}

impl<M: Clone> Document<M> {
    pub fn new(model: M, ids: IdAllocator, policy: IdRollbackPolicy) -> Self {
        Document { model, ids, policy, undo: Vec::new(), redo: Vec::new(), undo_limit: 256, generation: fresh_generation() }
    }

    pub fn model(&self) -> &M {
        &self.model
    }

    /// The name of the model's present state (see the field): a derived value that is a pure function of the model may be reused
    /// while this number is the same, and never otherwise. Safe to call only outside a transaction, which the borrow rules enforce
    /// (a transaction holds `&mut self`).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn ids(&self) -> &IdAllocator {
        &self.ids
    }

    pub fn id_policy(&self) -> IdRollbackPolicy {
        self.policy
    }

    /// Run `edit` as one transaction. `rollback_requested` forces a rollback after a
    /// successful edit (used to probe reference behaviour).
    ///
    /// The reference product regenerates at the end of the transaction and a failed regeneration rolls the transaction back
    /// (end type 2); then it handles the posted failures. The engine has no stored derived state, so its "regeneration" is
    /// the commit-time checks the edit runs (`cx.report`): an Error among them, or a refusal (`Err`), rolls back; Warnings
    /// are kept with the committed result. UNKNOWN: the default failures processor of a session, which decides what the
    /// user sees for a Warning or an Error.
    pub fn transact<R>(
        &mut self,
        name: &str,
        rollback_requested: bool,
        edit: impl FnOnce(&mut M, &mut TxnContext<'_>) -> Result<R, Failure>,
    ) -> Outcome<R> {
        let before = self.model.clone();
        let ids_before = self.ids.clone();
        let first_id = self.ids.peek();
        self.generation = fresh_generation();   // the model is about to change
        let mut failures = Vec::new();
        let result = {
            let mut ctx = TxnContext { ids: &mut self.ids, failures: &mut failures };
            // A panic inside an operation (a bug, or an input nobody thought of) must not leave the model half edited and outside the
            // undo stack: it is a failure of this transaction, which the rollback below undoes (QA: a wall of height 1e-308 panicked in
            // the middle of its creation; the wall stayed in the model, un-undoable, and every later read of the view panicked too).
            let model = &mut self.model;
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| edit(model, &mut ctx))) {
                Ok(r) => r,
                Err(payload) => {
                    let text = payload
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_owned())
                        .or_else(|| payload.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown cause".to_owned());
                    Err(Failure::error(INTERNAL_ERROR, format!("Internal error, the change was not made: {text}")))
                }
            }
        };
        self.generation = fresh_generation();   // the model changed (or is about to be restored below)
        let allocated: Vec<ElementId> = (first_id.0..self.ids.peek().0).map(ElementId).collect();

        let value = match result {
            Ok(v) => Some(v),
            Err(f) => {
                failures.push(f);
                None
            }
        };
        let has_error = failures.iter().any(|f| f.severity.rolls_back());
        if value.is_none() || has_error || rollback_requested {
            self.model = before;
            self.generation = fresh_generation();
            if self.policy == IdRollbackPolicy::Restore {
                self.ids = ids_before;
            }
            return Outcome { name: name.to_owned(), status: TransactionStatus::RolledBack, value, allocated, failures };
        }

        self.redo.clear();
        self.undo.push(Committed { name: name.to_owned(), before });
        if self.undo.len() > self.undo_limit {
            self.undo.remove(0);
        }
        Outcome { name: name.to_owned(), status: TransactionStatus::Committed, value, allocated, failures }
    }

    /// Undo the last committed transaction. Ids are never handed back on undo.
    pub fn undo(&mut self) -> Result<String, TransactionError> {
        let c = self.undo.pop().ok_or(TransactionError::NothingToUndo)?;
        let after = std::mem::replace(&mut self.model, c.before);
        self.generation = fresh_generation();
        self.redo.push(Committed { name: c.name.clone(), before: after });
        Ok(c.name)
    }

    pub fn redo(&mut self) -> Result<String, TransactionError> {
        let c = self.redo.pop().ok_or(TransactionError::NothingToRedo)?;
        let before = std::mem::replace(&mut self.model, c.before);
        self.generation = fresh_generation();
        self.undo.push(Committed { name: c.name.clone(), before });
        Ok(c.name)
    }

    /// Forgets every undo and redo step: the present state becomes the document's starting point.
    /// A new project, or a document that was just opened and had its session views made, has
    /// nothing to undo - the first Undo must not take the project apart.
    pub fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn undo_names(&self) -> Vec<&str> {
        self.undo.iter().map(|c| c.name.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Severity;

    fn doc(policy: IdRollbackPolicy) -> Document<Vec<(ElementId, i32)>> {
        Document::new(Vec::new(), IdAllocator::default(), policy)
    }

    fn add(d: &mut Document<Vec<(ElementId, i32)>>, v: i32, rollback: bool) -> Outcome<ElementId> {
        d.transact("add", rollback, |m, cx| {
            let id = cx.new_id();
            m.push((id, v));
            Ok(id)
        })
    }

    #[test]
    fn clearing_the_history_keeps_the_model_and_empties_both_stacks() {
        let mut d = doc(IdRollbackPolicy::Keep);
        assert!(add(&mut d, 1, false).committed());
        assert!(add(&mut d, 2, false).committed());
        d.undo().unwrap();
        d.clear_history();
        assert_eq!(d.model().len(), 1, "the model is untouched");
        assert!(d.undo_names().is_empty());
        assert!(d.undo().is_err() && d.redo().is_err());
    }

    /// QA: a panic inside an operation rolls the transaction back (the model is as it was, nothing is pushed on the undo stack) and
    /// is reported as a failure of the transaction instead of unwinding through the caller with the model half edited.
    #[test]
    fn a_panicking_operation_is_rolled_back_and_reported() {
        let mut d = doc(IdRollbackPolicy::Keep);
        assert!(add(&mut d, 1, false).committed());
        let o = d.transact("boom", false, |m, cx| {
            m.push((cx.new_id(), 2));
            if m.len() == 2 {
                panic!("prism height must be positive");
            }
            Ok(())
        });
        assert_eq!(o.status, TransactionStatus::RolledBack);
        assert_eq!(o.failures.len(), 1);
        assert_eq!(o.failures[0].code, INTERNAL_ERROR);
        assert!(o.failures[0].message.contains("prism height must be positive"), "{}", o.failures[0].message);
        assert_eq!(d.model().len(), 1, "the half-made change is gone");
        assert_eq!(d.undo_names(), vec!["add"], "and left no undo step");
        assert!(add(&mut d, 3, false).committed(), "the document goes on working");
    }

    #[test]
    fn commit_keeps_changes_and_rollback_discards_them() {
        let mut d = doc(IdRollbackPolicy::Keep);
        assert!(add(&mut d, 1, false).committed());
        let o = add(&mut d, 2, true);
        assert_eq!(o.status, TransactionStatus::RolledBack);
        assert_eq!(d.model().len(), 1);
    }

    /// A DocumentCorruption failure (literal 3) rolls the transaction back.
    #[test]
    fn a_document_corruption_failure_rolls_back() {
        let mut d = doc(IdRollbackPolicy::Keep);
        let o = d.transact("corrupt", false, |m, cx| {
            m.push((cx.new_id(), 1));
            cx.report(Failure { severity: Severity::DocumentCorruption, code: "x.y".into(), message: String::new(), elements: vec![] });
            Ok(())
        });
        assert_eq!(o.status, TransactionStatus::RolledBack);
        assert!(d.model().is_empty());
        assert_eq!((Severity::Warning.literal(), Severity::Error.literal(), Severity::DocumentCorruption.literal()), (1, 2, 3));
    }

    #[test]
    fn error_failure_rolls_back_but_warning_commits() {
        let mut d = doc(IdRollbackPolicy::Keep);
        let o = d.transact("warn", false, |m, cx| {
            m.push((cx.new_id(), 1));
            cx.report(Failure::warning("w", "just a warning"));
            Ok(())
        });
        assert!(o.committed());
        let o = d.transact("err", false, |m, cx| {
            m.push((cx.new_id(), 2));
            cx.report(Failure::error("e", "blocking"));
            Ok(())
        });
        assert_eq!(o.status, TransactionStatus::RolledBack);
        assert_eq!(d.model().len(), 1);
    }

    #[test]
    fn id_policy_controls_reuse_after_rollback() {
        let mut keep = doc(IdRollbackPolicy::Keep);
        let a = add(&mut keep, 1, true).value.unwrap();
        let b = add(&mut keep, 2, false).value.unwrap();
        assert_eq!(b.0, a.0 + 1);

        let mut restore = doc(IdRollbackPolicy::Restore);
        let a = add(&mut restore, 1, true).value.unwrap();
        let b = add(&mut restore, 2, false).value.unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn undo_redo_round_trip() {
        let mut d = doc(IdRollbackPolicy::Keep);
        add(&mut d, 1, false);
        add(&mut d, 2, false);
        assert_eq!(d.undo().unwrap(), "add");
        assert_eq!(d.model().len(), 1);
        d.redo().unwrap();
        assert_eq!(d.model().len(), 2);
        d.undo().unwrap();
        add(&mut d, 3, false);
        assert_eq!(d.redo(), Err(TransactionError::NothingToRedo));
        // ids are never handed back by undo
        assert_eq!(d.model().last().unwrap().0, ElementId(3));
    }
}
