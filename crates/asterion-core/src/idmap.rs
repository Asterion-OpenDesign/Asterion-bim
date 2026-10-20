//! `IdMap`: an ordered map keyed by [`ElementId`] with structural sharing.
//!
//! Why it exists: a transaction used to snapshot the model by deep-copying every `BTreeMap` in it, and the undo stack kept
//! up to 256 such snapshots. At 100 000 walls one snapshot is 24 MB and 30 edits held 728 MB. The ids of a document are
//! dense and monotonic, so the map is split into chunks of [`CHUNK_IDS`] consecutive ids, each behind an `Arc`:
//!
//! * `clone` copies one `Arc` per chunk (`n / 1024` pointers), not the elements;
//! * a write copies ONE chunk (copy-on-write, `Arc::make_mut`), so an edit costs O(chunk), not O(model);
//! * iteration order is identical to `BTreeMap<ElementId, V>` (chunks are ordered by `id >> SHIFT`), so every
//!   "ascending element id" rule of the engine holds unchanged;
//! * the serialized form is the same JSON object a `BTreeMap` writes, so existing files load unchanged.
//!
//! Each map also carries a [`version`](IdMap::version): a number taken from a process-wide counter by every
//! operation that may change the content. Two maps with the same version have the same content (a clone shares
//! its source's version until either is written), so a derived structure cached against `(map, version)` can
//! never be stale - this is the invalidation rule of the engine's spatial caches.

use crate::id::ElementId;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{btree_map, BTreeMap};
use std::fmt;
use std::marker::PhantomData;
use std::ops::Index;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const SHIFT: u32 = 10;
/// Consecutive ids that share one chunk.
pub const CHUNK_IDS: u64 = 1 << SHIFT;

static NEXT_VERSION: AtomicU64 = AtomicU64::new(1);

fn fresh_version() -> u64 {
    NEXT_VERSION.fetch_add(1, Ordering::Relaxed)
}

type Chunk<V> = BTreeMap<ElementId, V>;

pub struct IdMap<V> {
    chunks: BTreeMap<u64, Arc<Chunk<V>>>,
    len: usize,
    version: u64,
}

fn chunk_of(id: &ElementId) -> u64 {
    id.0 >> SHIFT
}

impl<V> Default for IdMap<V> {
    fn default() -> Self {
        IdMap { chunks: BTreeMap::new(), len: 0, version: 0 }
    }
}

impl<V> Clone for IdMap<V> {
    /// O(chunks): shares every chunk with `self`.
    fn clone(&self) -> Self {
        IdMap { chunks: self.chunks.clone(), len: self.len, version: self.version }
    }
}

impl<V> IdMap<V> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Identifies the content: see the module documentation.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Number of chunks and how many of them are shared with another map (their `Arc` has other owners).
    pub fn chunk_stats(&self) -> (usize, usize) {
        (self.chunks.len(), self.chunks.values().filter(|c| Arc::strong_count(c) > 1).count())
    }

    pub fn get(&self, id: &ElementId) -> Option<&V> {
        self.chunks.get(&chunk_of(id))?.get(id)
    }

    pub fn contains_key(&self, id: &ElementId) -> bool {
        self.get(id).is_some()
    }

    pub fn iter(&self) -> Iter<'_, V> {
        Iter { outer: self.chunks.values(), front: None, back: None, remaining: self.len }
    }

    pub fn keys(&self) -> Keys<'_, V> {
        Keys(self.iter())
    }

    pub fn values(&self) -> Values<'_, V> {
        Values(self.iter())
    }

    pub fn first_key_value(&self) -> Option<(&ElementId, &V)> {
        self.chunks.values().next()?.first_key_value()
    }

    pub fn last_key_value(&self) -> Option<(&ElementId, &V)> {
        self.chunks.values().next_back()?.last_key_value()
    }
}

impl<V: Clone> IdMap<V> {
    pub fn get_mut(&mut self, id: &ElementId) -> Option<&mut V> {
        let ck = chunk_of(id);
        if !self.chunks.get(&ck).is_some_and(|c| c.contains_key(id)) {
            return None;
        }
        self.version = fresh_version();
        Arc::make_mut(self.chunks.get_mut(&ck)?).get_mut(id)
    }

    pub fn insert(&mut self, id: ElementId, value: V) -> Option<V> {
        self.version = fresh_version();
        let chunk = self.chunks.entry(chunk_of(&id)).or_insert_with(|| Arc::new(BTreeMap::new()));
        let old = Arc::make_mut(chunk).insert(id, value);
        if old.is_none() {
            self.len += 1;
        }
        old
    }

    pub fn remove(&mut self, id: &ElementId) -> Option<V> {
        let ck = chunk_of(id);
        if !self.chunks.get(&ck).is_some_and(|c| c.contains_key(id)) {
            return None;
        }
        self.version = fresh_version();
        let chunk = self.chunks.get_mut(&ck)?;
        let old = Arc::make_mut(chunk).remove(id);
        if chunk.is_empty() {
            self.chunks.remove(&ck);
        }
        self.len -= 1;
        old
    }

    pub fn clear(&mut self) {
        self.version = fresh_version();
        self.chunks.clear();
        self.len = 0;
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&ElementId, &mut V) -> bool) {
        self.version = fresh_version();
        let mut len = 0;
        self.chunks.retain(|_, c| {
            Arc::make_mut(c).retain(|k, v| keep(k, v));
            len += c.len();
            !c.is_empty()
        });
        self.len = len;
    }

    /// Mutable iteration copies every chunk that is still shared: use it only for a change that really
    /// touches every element.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&ElementId, &mut V)> {
        self.version = fresh_version();
        self.chunks.values_mut().flat_map(|c| Arc::make_mut(c).iter_mut())
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.iter_mut().map(|(_, v)| v)
    }

    pub fn entry(&mut self, id: ElementId) -> Entry<'_, V> {
        Entry { map: self, id }
    }
}

/// The chunk table of an [`IdMap`] at one moment, kept by a derived structure (a spatial index, a cache) to learn
/// later WHICH chunks changed. Holding the snapshot keeps its chunks shared, so a write to the map always copies
/// the chunk it touches: a chunk whose `Arc` is still the same pointer is therefore guaranteed unchanged, and no
/// change log is needed.
pub struct Snapshot<V> {
    chunks: BTreeMap<u64, Arc<Chunk<V>>>,
    version: u64,
}

impl<V> Default for Snapshot<V> {
    fn default() -> Self {
        Snapshot { chunks: BTreeMap::new(), version: 0 }
    }
}

/// Read access to one chunk's elements, in ascending id.
pub struct ChunkRef<'a, V>(&'a Chunk<V>);

impl<'a, V> ChunkRef<'a, V> {
    pub fn iter(&self) -> btree_map::Iter<'a, ElementId, V> {
        self.0.iter()
    }
}

impl<V> IdMap<V> {
    pub fn snapshot(&self) -> Snapshot<V> {
        Snapshot { chunks: self.chunks.clone(), version: self.version }
    }
}

impl<V> Snapshot<V> {
    /// The [`IdMap::version`] this snapshot was taken at; equal to the map's current version means no change.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Number of chunks that differ between this snapshot and `current` (by pointer), and the number of chunks
    /// of `current`: how far a derived structure built from this snapshot is from `current`.
    pub fn distance(&self, current: &IdMap<V>) -> (usize, usize) {
        let mut differing = 0;
        for (k, c) in &current.chunks {
            if !self.chunks.get(k).is_some_and(|o| Arc::ptr_eq(o, c)) {
                differing += 1;
            }
        }
        differing += self.chunks.keys().filter(|k| !current.chunks.contains_key(k)).count();
        (differing, current.chunks.len().max(self.chunks.len()))
    }

    /// Visits every chunk that differs between this snapshot and `current`: `f(old, new)` with the chunk as the
    /// snapshot saw it (`None` if it did not exist) and as `current` has it (`None` if it is gone). Unchanged chunks
    /// are skipped without being read. Afterwards this snapshot IS `current`.
    pub fn advance(&mut self, current: &IdMap<V>, mut f: impl FnMut(Option<ChunkRef<'_, V>>, Option<ChunkRef<'_, V>>)) {
        if self.version == current.version && self.chunks.len() == current.chunks.len() {
            return;
        }
        for (k, old) in &self.chunks {
            match current.chunks.get(k) {
                Some(new) if Arc::ptr_eq(old, new) => {}
                Some(new) => f(Some(ChunkRef(old)), Some(ChunkRef(new))),
                None => f(Some(ChunkRef(old)), None),
            }
        }
        for (k, new) in &current.chunks {
            if !self.chunks.contains_key(k) {
                f(None, Some(ChunkRef(new)));
            }
        }
        self.chunks = current.chunks.clone();
        self.version = current.version;
    }
}

/// The part of the `BTreeMap` entry API the engine uses.
pub struct Entry<'a, V> {
    map: &'a mut IdMap<V>,
    id: ElementId,
}

impl<'a, V: Clone> Entry<'a, V> {
    pub fn or_insert_with(self, make: impl FnOnce() -> V) -> &'a mut V {
        if !self.map.contains_key(&self.id) {
            self.map.insert(self.id, make());
        }
        self.map.get_mut(&self.id).expect("present")
    }

    pub fn or_insert(self, value: V) -> &'a mut V {
        self.or_insert_with(|| value)
    }

    pub fn or_default(self) -> &'a mut V
    where
        V: Default,
    {
        self.or_insert_with(V::default)
    }

    pub fn and_modify(self, f: impl FnOnce(&mut V)) -> Self {
        if let Some(v) = self.map.get_mut(&self.id) {
            f(v);
        }
        self
    }
}

/// Ascending-id iterator, double-ended and exact-sized like the `BTreeMap` one it replaces.
pub struct Iter<'a, V> {
    outer: btree_map::Values<'a, u64, Arc<Chunk<V>>>,
    front: Option<btree_map::Iter<'a, ElementId, V>>,
    back: Option<btree_map::Iter<'a, ElementId, V>>,
    remaining: usize,
}

impl<'a, V> Iterator for Iter<'a, V> {
    type Item = (&'a ElementId, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(it) = &mut self.front {
                if let Some(x) = it.next() {
                    self.remaining -= 1;
                    return Some(x);
                }
            }
            match self.outer.next() {
                Some(c) => self.front = Some(c.iter()),
                None => {
                    let x = self.back.as_mut()?.next()?;
                    self.remaining -= 1;
                    return Some(x);
                }
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<V> DoubleEndedIterator for Iter<'_, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(it) = &mut self.back {
                if let Some(x) = it.next_back() {
                    self.remaining -= 1;
                    return Some(x);
                }
            }
            match self.outer.next_back() {
                Some(c) => self.back = Some(c.iter()),
                None => {
                    let x = self.front.as_mut()?.next_back()?;
                    self.remaining -= 1;
                    return Some(x);
                }
            }
        }
    }
}

impl<V> ExactSizeIterator for Iter<'_, V> {}

/// Ascending ids, like `BTreeMap::keys`.
pub struct Keys<'a, V>(Iter<'a, V>);

impl<'a, V> Iterator for Keys<'a, V> {
    type Item = &'a ElementId;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, _)| k)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<V> DoubleEndedIterator for Keys<'_, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(k, _)| k)
    }
}

impl<V> ExactSizeIterator for Keys<'_, V> {}

/// Values in ascending id order, like `BTreeMap::values`.
pub struct Values<'a, V>(Iter<'a, V>);

impl<'a, V> Iterator for Values<'a, V> {
    type Item = &'a V;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(_, v)| v)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<V> DoubleEndedIterator for Values<'_, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(_, v)| v)
    }
}

impl<V> ExactSizeIterator for Values<'_, V> {}

impl<'a, V> IntoIterator for &'a IdMap<V> {
    type Item = (&'a ElementId, &'a V);
    type IntoIter = Iter<'a, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<V: Clone> IntoIterator for IdMap<V> {
    type Item = (ElementId, V);
    type IntoIter = std::vec::IntoIter<(ElementId, V)>;
    fn into_iter(self) -> Self::IntoIter {
        let mut out = Vec::with_capacity(self.len);
        for (_, c) in self.chunks {
            let chunk = Arc::try_unwrap(c).unwrap_or_else(|shared| (*shared).clone());
            out.extend(chunk);
        }
        out.into_iter()
    }
}

impl<V: Clone> FromIterator<(ElementId, V)> for IdMap<V> {
    fn from_iter<I: IntoIterator<Item = (ElementId, V)>>(iter: I) -> Self {
        let mut m = IdMap::new();
        m.extend(iter);
        m
    }
}

impl<V: Clone> Extend<(ElementId, V)> for IdMap<V> {
    fn extend<I: IntoIterator<Item = (ElementId, V)>>(&mut self, iter: I) {
        for (k, v) in iter {
            self.insert(k, v);
        }
    }
}

impl<V> Index<&ElementId> for IdMap<V> {
    type Output = V;
    fn index(&self, id: &ElementId) -> &V {
        self.get(id).expect("no entry found for key")
    }
}

impl<V: PartialEq> PartialEq for IdMap<V> {
    fn eq(&self, other: &Self) -> bool {
        if self.len != other.len {
            return false;
        }
        // chunks shared with the other map (an unmodified clone) are equal without looking inside
        let mut a = self.chunks.iter();
        let mut b = other.chunks.iter();
        loop {
            match (a.next(), b.next()) {
                (None, None) => return true,
                (Some((ka, ca)), Some((kb, cb))) => {
                    if ka != kb || !(Arc::ptr_eq(ca, cb) || ca == cb) {
                        return false;
                    }
                }
                _ => return false,
            }
        }
    }
}

impl<V: Eq> Eq for IdMap<V> {}

impl<V: fmt::Debug> fmt::Debug for IdMap<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<V: Serialize> Serialize for IdMap<V> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_map(self.iter())
    }
}

impl<'de, V: Deserialize<'de> + Clone> Deserialize<'de> for IdMap<V> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct MapVisitor<V>(PhantomData<V>);
        impl<'de, V: Deserialize<'de> + Clone> Visitor<'de> for MapVisitor<V> {
            type Value = IdMap<V>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a map keyed by element id")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<IdMap<V>, A::Error> {
                let mut m = IdMap::new();
                while let Some((k, v)) = access.next_entry::<ElementId, V>()? {
                    m.insert(k, v);
                }
                Ok(m)
            }
        }
        d.deserialize_map(MapVisitor(PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> ElementId {
        ElementId(n)
    }

    /// A deterministic mix of inserts, overwrites, removals and in-place edits, applied to an `IdMap` and to the
    /// `BTreeMap` it replaces: every observable must agree at every step.
    #[test]
    fn behaves_like_a_btreemap_under_a_mixed_workload() {
        let mut m: IdMap<u64> = IdMap::new();
        let mut r: BTreeMap<ElementId, u64> = BTreeMap::new();
        let mut x = 0x9E3779B97F4A7C15u64;
        for step in 0..20_000u64 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let key = id((x >> 33) % 6_000); // several chunks, with collisions
            match (x >> 20) % 4 {
                0 | 1 => assert_eq!(m.insert(key, step), r.insert(key, step)),
                2 => assert_eq!(m.remove(&key), r.remove(&key)),
                _ => {
                    assert_eq!(m.get_mut(&key).map(|v| {
                        *v += 7;
                        *v
                    }), r.get_mut(&key).map(|v| {
                        *v += 7;
                        *v
                    }));
                }
            }
            if step % 997 == 0 {
                assert_eq!(m.len(), r.len());
                assert!(m.iter().map(|(k, v)| (*k, *v)).eq(r.iter().map(|(k, v)| (*k, *v))));
                assert!(m.iter().rev().map(|(k, _)| *k).eq(r.keys().rev().copied()));
                assert_eq!(m.first_key_value().map(|(k, _)| *k), r.first_key_value().map(|(k, _)| *k));
                assert_eq!(m.last_key_value().map(|(k, _)| *k), r.last_key_value().map(|(k, _)| *k));
            }
        }
        assert_eq!(m.len(), r.len());
        assert_eq!(m.iter().len(), r.len());
    }

    /// Mixed front/back consumption of one iterator never repeats or loses an element.
    #[test]
    fn double_ended_iteration_meets_in_the_middle() {
        let m: IdMap<u64> = (0..5_000u64).map(|i| (id(i * 3), i)).collect();
        let mut it = m.iter();
        let (mut front, mut back) = (Vec::new(), Vec::new());
        loop {
            match it.next() {
                Some((k, _)) => front.push(k.0),
                None => break,
            }
            match it.next_back() {
                Some((k, _)) => back.push(k.0),
                None => break,
            }
        }
        back.reverse();
        front.extend(back);
        assert!(front.iter().copied().eq((0..5_000u64).map(|i| i * 3)));
    }

    /// The point of the type: a clone shares storage, and writing one side never shows in the other.
    #[test]
    fn a_clone_shares_chunks_and_a_write_copies_only_one() {
        let mut a: IdMap<String> = (1..=10_000u64).map(|i| (id(i), format!("e{i}"))).collect();
        let (chunks, _) = a.chunk_stats();
        let b = a.clone();
        assert_eq!(a.chunk_stats(), (chunks, chunks), "every chunk is shared right after a clone");
        a.get_mut(&id(5_000)).unwrap().push('!');
        assert_eq!(a.chunk_stats().1, chunks - 1, "one chunk was copied, the rest still shared");
        assert_eq!(b.get(&id(5_000)).unwrap(), "e5000");
        assert_eq!(a.get(&id(5_000)).unwrap(), "e5000!");
        a.insert(id(20_000), "new".into());
        a.remove(&id(1));
        assert!(b.contains_key(&id(1)) && !b.contains_key(&id(20_000)));
        assert_eq!(b.len(), 10_000);
        assert_eq!(a.len(), 10_000);
    }

    /// `version` is the cache-validity rule: equal versions mean equal content, a write always changes it, and
    /// two clones that diverge never end up with the same version.
    #[test]
    fn the_version_identifies_the_content() {
        let mut a: IdMap<u32> = (1..=50u64).map(|i| (id(i), i as u32)).collect();
        let b = a.clone();
        assert_eq!(a.version(), b.version());
        assert_eq!(a, b);
        let v0 = a.version();
        a.insert(id(7), 99);
        assert_ne!(a.version(), v0);
        assert_eq!(b.version(), v0, "the untouched clone keeps its version");
        let mut c = b.clone();
        let mut d = b.clone();
        c.insert(id(1), 1000);
        d.insert(id(1), 2000);
        assert_ne!(c.version(), d.version());
        let before = a.version();
        assert!(a.get_mut(&id(9999)).is_none() && a.remove(&id(9999)).is_none());
        assert_eq!(a.version(), before, "a miss changes nothing");
    }

    #[test]
    fn equality_is_by_content_not_by_sharing() {
        let a: IdMap<u32> = (1..=3_000u64).map(|i| (id(i), i as u32)).collect();
        let mut b: IdMap<u32> = (1..=3_000u64).rev().map(|i| (id(i), i as u32)).collect();
        assert_eq!(a, b);
        b.insert(id(2_000), 0);
        assert_ne!(a, b);
        b.insert(id(2_000), 2_000);
        assert_eq!(a, b);
    }

    /// The file format must not change: the JSON is the object a `BTreeMap<ElementId, _>` writes, and it reads
    /// back, including a map that spans several chunks.
    #[test]
    fn serializes_exactly_like_a_btreemap() {
        let pairs: Vec<(ElementId, f64)> = [1u64, 2, 1023, 1024, 1025, 70_000].iter().map(|&i| (id(i), i as f64 / 3.0)).collect();
        let m: IdMap<f64> = pairs.iter().cloned().collect();
        let r: BTreeMap<ElementId, f64> = pairs.iter().cloned().collect();
        let (jm, jr) = (serde_json::to_string(&m).unwrap(), serde_json::to_string(&r).unwrap());
        assert_eq!(jm, jr);
        let back: IdMap<f64> = serde_json::from_str(&jm).unwrap();
        assert_eq!(back, m);
        let pretty_m = serde_json::to_string_pretty(&m).unwrap();
        assert_eq!(pretty_m, serde_json::to_string_pretty(&r).unwrap());
    }

    /// The incremental-index rule: after any mix of writes to the SAME map (no clone involved), `advance` reports
    /// exactly the chunks whose content changed, so a derived structure that re-reads only those stays exact.
    #[test]
    fn a_snapshot_reports_exactly_the_chunks_that_changed() {
        let mut m: IdMap<u64> = (1..=10_240u64).map(|i| (id(i), i)).collect();
        let mut snap = m.snapshot();
        let mut seen = Vec::new();
        snap.advance(&m, |o, n| seen.push((o.is_some(), n.is_some())));
        assert!(seen.is_empty(), "nothing changed yet");

        *m.get_mut(&id(3_000)).unwrap() += 1; // chunk 2
        m.insert(id(9_000), 0); // chunk 8, an overwrite
        m.insert(id(50_000), 5); // a chunk that did not exist
        m.remove(&id(1)); // chunk 0
        let mut old_ids = Vec::new();
        let mut new_ids = Vec::new();
        snap.advance(&m, |o, n| {
            old_ids.extend(o.into_iter().flat_map(|c| c.iter().map(|(k, _)| k.0).collect::<Vec<_>>()));
            new_ids.extend(n.into_iter().flat_map(|c| c.iter().map(|(k, _)| k.0).collect::<Vec<_>>()));
        });
        assert!(old_ids.contains(&3_000) && old_ids.contains(&9_000) && old_ids.contains(&1));
        assert!(!old_ids.contains(&6_000), "an untouched chunk is not visited");
        assert!(new_ids.contains(&50_000) && !new_ids.contains(&1));
        assert_eq!(snap.version(), m.version());

        // replaying the reported changes onto a mirror reproduces the map exactly
        let mut mirror: BTreeMap<u64, u64> = (1..=10_240u64).map(|i| (i, i)).collect();
        let mut s2 = (1..=10_240u64).map(|i| (id(i), i)).collect::<IdMap<u64>>();
        let mut snap2 = s2.snapshot();
        for step in 0..500u64 {
            let k = (step * 7919) % 12_000 + 1;
            match step % 3 {
                0 => {
                    s2.insert(id(k), step);
                    mirror.insert(k, step);
                }
                1 => {
                    s2.remove(&id(k));
                    mirror.remove(&k);
                }
                _ => {
                    if let Some(v) = s2.get_mut(&id(k)) {
                        *v += 1;
                        *mirror.get_mut(&k).unwrap() += 1;
                    }
                }
            }
            if step % 50 == 0 {
                let mut replay: BTreeMap<u64, u64> = BTreeMap::new();
                // `replay` starts from the snapshot's view, applies the diff, and must equal the live map
                for (k, v) in s2.iter() {
                    replay.insert(k.0, *v);
                }
                snap2.advance(&s2, |_, _| {});
                assert_eq!(replay, mirror);
            }
        }
    }

    #[test]
    fn distance_counts_the_differing_chunks() {
        let a: IdMap<u8> = (1..=5_000u64).map(|i| (id(i), 0)).collect();
        let snap = a.snapshot();
        let mut b = a.clone();
        assert_eq!(snap.distance(&b), (0, 5));
        b.insert(id(10), 1);
        b.insert(id(4_000), 1);
        assert_eq!(snap.distance(&b), (2, 5));
        let unrelated: IdMap<u8> = (1..=5_000u64).map(|i| (id(i), 0)).collect();
        assert_eq!(snap.distance(&unrelated), (5, 5), "equal content in other storage is still 'different'");
    }

    #[test]
    fn entry_retain_and_by_value_iteration() {
        let mut m: IdMap<Vec<u8>> = IdMap::new();
        m.entry(id(5)).or_default().push(1);
        m.entry(id(5)).or_default().push(2);
        m.entry(id(2000)).or_insert_with(|| vec![9]);
        assert_eq!(m[&id(5)], vec![1, 2]);
        m.retain(|k, _| k.0 < 1000);
        assert_eq!(m.len(), 1);
        assert!(m.get(&id(2000)).is_none());
        let shared = m.clone();
        let v: Vec<_> = m.into_iter().collect();
        assert_eq!(v, vec![(id(5), vec![1, 2])]);
        assert_eq!(shared.len(), 1, "consuming one side leaves the other intact");
    }
}
