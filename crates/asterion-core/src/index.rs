//! Derived indexes that follow an [`IdMap`] incrementally.
//!
//! A derived index (walls by grid cell, inserts by host, ...) is a pure function of one map. It is never part of
//! the model: it lives in a cache beside it, and [`IndexCache::with`] brings it up to date with the map it is asked
//! about before answering.
//!
//! * The map is compared with the snapshot the index was built from, chunk by chunk and by pointer
//!   ([`Snapshot::advance`]); only the chunks that changed are removed from / added to the index. An edit that
//!   touches one wall therefore costs O(one chunk), not O(model).
//! * Because the snapshot keeps its chunks shared, a write can never change a chunk in place behind the index's
//!   back, so a stale answer is impossible by construction (see [`Snapshot`]).
//! * Undo and redo hand the model an older (or newer) set of chunks; the same comparison brings the index to that
//!   state, again touching only the chunks that differ.
//! * Several lineages (two documents, or the same one before and after a failed experiment) get separate entries,
//!   up to [`ENTRIES`]; a map unrelated to every entry rebuilds the least recently used one.
//!
//! The cache is meant to be a `thread_local!`: no locking, and a thread's index can only describe maps the thread
//! has asked about. [`IndexCache::clear`] drops every entry (a closed document must not stay alive through its
//! index).

use crate::id::ElementId;
use crate::idmap::{IdMap, Snapshot};

/// Lineages kept per cache.
pub const ENTRIES: usize = 4;

struct Entry<V, I> {
    snapshot: Snapshot<V>,
    index: I,
}

pub struct IndexCache<V, I> {
    entries: Vec<Entry<V, I>>,
}

impl<V, I> Default for IndexCache<V, I> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V, I> IndexCache<V, I> {
    pub const fn new() -> Self {
        IndexCache { entries: Vec::new() }
    }

    /// Drops every index (and the chunks their snapshots keep alive).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Runs `answer` on the index of `map`, first updating the index with the chunks of `map` that differ from
    /// the ones it last saw. `empty` makes a new index; `add` / `remove` put one element in / take one out.
    /// `remove` is always given the element exactly as the index saw it, `add` as the map has it now.
    pub fn with<R>(
        &mut self,
        map: &IdMap<V>,
        empty: impl FnOnce() -> I,
        mut add: impl FnMut(&mut I, &ElementId, &V),
        mut remove: impl FnMut(&mut I, &ElementId, &V),
        answer: impl FnOnce(&I) -> R,
    ) -> R {
        let mut best: Option<(usize, usize, usize)> = None;
        for (i, e) in self.entries.iter().enumerate() {
            // equal versions mean identical content (see `IdMap::version`): no need to walk the chunk table
            let (d, t) = if e.snapshot.version() == map.version() { (0, 1) } else { e.snapshot.distance(map) };
            if best.map_or(true, |(_, bd, _)| d < bd) {
                best = Some((i, d, t));
            }
            if d == 0 {
                break;
            }
        }
        let at = match best {
            // close to an entry we already have: keep following that lineage
            Some((i, d, total)) if d < total || total == 0 => i,
            // unrelated to all of them: a new entry, or the least recently used one rebuilt from nothing
            _ if self.entries.len() < ENTRIES => {
                self.entries.insert(0, Entry { snapshot: Snapshot::default(), index: empty() });
                0
            }
            _ => {
                let last = self.entries.len() - 1;
                self.entries[last] = Entry { snapshot: Snapshot::default(), index: empty() };
                last
            }
        };
        let e = &mut self.entries[at];
        let Entry { snapshot, index } = e;
        snapshot.advance(map, |old, new| {
            if let Some(c) = old {
                for (k, v) in c.iter() {
                    remove(index, k, v);
                }
            }
            if let Some(c) = new {
                for (k, v) in c.iter() {
                    add(index, k, v);
                }
            }
        });
        if at != 0 {
            let entry = self.entries.remove(at);
            self.entries.insert(0, entry);
        }
        answer(&self.entries[0].index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    /// index: value -> ids having that value
    type ByValue = BTreeMap<u32, BTreeSet<u64>>;

    fn ask(cache: &mut IndexCache<u32, ByValue>, m: &IdMap<u32>) -> ByValue {
        cache.with(
            m,
            ByValue::new,
            |ix, k, v| {
                ix.entry(*v).or_default().insert(k.0);
            },
            |ix, k, v| {
                if let Some(s) = ix.get_mut(v) {
                    s.remove(&k.0);
                    if s.is_empty() {
                        ix.remove(v);
                    }
                }
            },
            |ix| ix.clone(),
        )
    }

    fn brute(m: &IdMap<u32>) -> ByValue {
        let mut out = ByValue::new();
        for (k, v) in m.iter() {
            out.entry(*v).or_default().insert(k.0);
        }
        out
    }

    /// Whatever sequence of writes, clones, rollbacks to older states and unrelated maps, the answer equals a
    /// from-scratch rebuild.
    #[test]
    fn the_index_always_equals_a_rebuild() {
        let mut cache: IndexCache<u32, ByValue> = IndexCache::new();
        let mut live: IdMap<u32> = (1..=6_000u64).map(|i| (ElementId(i), (i % 17) as u32)).collect();
        let mut history: Vec<IdMap<u32>> = vec![live.clone()];
        let mut x = 12345u64;
        for step in 0..600u64 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let k = ElementId((x >> 33) % 8_000 + 1);
            match (x >> 12) % 6 {
                0 => {
                    live.insert(k, (step % 23) as u32);
                }
                1 => {
                    live.remove(&k);
                }
                2 => {
                    if let Some(v) = live.get_mut(&k) {
                        *v += 100;
                    }
                }
                3 => history.push(live.clone()),
                4 if history.len() > 1 => {
                    // undo / redo: go back to an older state
                    let i = (x >> 20) as usize % history.len();
                    live = history[i].clone();
                }
                _ => {}
            }
            if step % 7 == 0 {
                assert_eq!(ask(&mut cache, &live), brute(&live), "step {step}");
            }
        }
        // an unrelated map and back
        let other: IdMap<u32> = (1..=100u64).map(|i| (ElementId(i + 50_000), 1)).collect();
        assert_eq!(ask(&mut cache, &other), brute(&other));
        assert_eq!(ask(&mut cache, &live), brute(&live));
        assert!(cache.len() <= ENTRIES);
        cache.clear();
        assert!(cache.is_empty());
    }

    /// A one-element edit of a big map touches one chunk's worth of index work.
    #[test]
    fn an_edit_costs_one_chunk_not_the_model() {
        let mut cache: IndexCache<u32, ByValue> = IndexCache::new();
        let mut m: IdMap<u32> = (1..=100_000u64).map(|i| (ElementId(i), (i % 5) as u32)).collect();
        let _ = ask(&mut cache, &m);
        let touched = std::cell::Cell::new(0usize);
        m.insert(ElementId(77_777), 99);
        cache.with(
            &m,
            ByValue::new,
            |_, _, _| touched.set(touched.get() + 1),
            |_, _, _| touched.set(touched.get() + 1),
            |_| (),
        );
        // the old chunk is removed and the new one added: at most two chunks of elements, never 100 000
        assert!(touched.get() <= 2 * crate::idmap::CHUNK_IDS as usize, "touched {}", touched.get());
        assert!(touched.get() > 0);
    }
}
