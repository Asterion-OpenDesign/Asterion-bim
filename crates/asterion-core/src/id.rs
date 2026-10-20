//! Element identity.
//!
//! Ids are allocated from a monotonically increasing counter that lives *inside* the model state. Because the transaction
//! system restores the whole model state on rollback, whether an id consumed by a rolled-back transaction is handed out
//! again is decided by where the allocator lives. Both behaviours are offered so that the policy can follow observed
//! reference behaviour.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Persistent identity of an element inside one document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ElementId(pub u64);

impl ElementId {
    pub const INVALID: ElementId = ElementId(0);

    pub fn is_well_formed(self) -> bool {
        self.0 != 0
    }
}

impl fmt::Display for ElementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Monotonic id allocator. Never reuses an id it has handed out, including ids of
/// deleted elements.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdAllocator {
    next: u64,
}

impl IdAllocator {
    pub fn starting_at(first: u64) -> Self {
        assert!(first > 0, "0 is reserved for ElementId::INVALID");
        IdAllocator { next: first }
    }

    pub fn allocate(&mut self) -> ElementId {
        let id = ElementId(self.next);
        self.next += 1;
        id
    }

    pub fn peek(&self) -> ElementId {
        ElementId(self.next)
    }

    /// Reserve `n` consecutive ids and return the first (Spec P-01: the block is taken from the same counter, which only
    /// advances). `n = 0` reserves nothing and returns the next id.
    pub fn allocate_sequential_ids(&mut self, n: u64) -> ElementId {
        let first = ElementId(self.next);
        self.next += n;
        first
    }

    /// Make sure the next id is strictly greater than `id` (used after loading). Spec P-01: the counter is RAISED to cover
    /// ids it did not hand out, never lowered.
    pub fn reserve_through(&mut self, id: ElementId) {
        if id.0 >= self.next {
            self.next = id.0 + 1;
        }
    }
}

impl Default for IdAllocator {
    fn default() -> Self {
        IdAllocator::starting_at(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_is_monotonic_and_never_returns_invalid() {
        let mut a = IdAllocator::default();
        let x = a.allocate();
        let y = a.allocate();
        assert!(x.is_well_formed() && y.is_well_formed());
        assert!(y > x);
    }

    #[test]
    fn reserve_through_skips_loaded_ids() {
        let mut a = IdAllocator::default();
        a.reserve_through(ElementId(41));
        assert_eq!(a.allocate(), ElementId(42));
        a.reserve_through(ElementId(10));
        assert_eq!(a.allocate(), ElementId(43));
    }
}
