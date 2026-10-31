// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::cell::{
    CellOwned, CellRef, KeyOwned, KeyRef, Lookup, RangePoint, hash_key, hash_key_owned, keys_eq,
};
use super::index::{RangeKey, RangeReject, normalize};
use super::metrics::{ProcessorMetrics, WriteCounts};
use super::schema::{ColumnIdx, KeyPlan, Schema};
use crate::config::lookup_table::{RangeType, ValueType};
use anyhow::{Result, bail};
use hashbrown::{HashSet, HashTable};
use parking_lot::RwLock;
use rangemap::RangeInclusiveMap;
use smallvec::SmallVec;
use smol_str::SmolStr;
use std::cell::Cell;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Unreferenced pool entries are swept no more often than this.
const COMPACT_INTERVAL: Duration = Duration::from_secs(60);
/// `SmolStr` stores up to this many bytes inline, so shorter strings never
/// reach the pool.
const INLINE_LEN: usize = 23;
const MAX_SHARDS: usize = 64;

/// Milliseconds since process start, used for row ageing.
pub fn now_millis() -> u64 {
    crate::common::uptime().as_millis() as u64
}

/// What an upsert did. `Rejected` means the write would exceed `max_rows`.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upsert {
    Inserted,
    Updated,
    Rejected,
}

/// A row of a mutable table. Handed out behind an `Arc` so readers never hold
/// a shard lock.
#[derive(Debug)]
pub struct MutableRow {
    cells: Box<[CellOwned]>,
    last_seen: AtomicU64,
}

impl MutableRow {
    #[inline]
    pub fn cell(&self, column: ColumnIdx) -> CellRef<'_> {
        self.cells
            .get(column as usize)
            .map_or(CellRef::Null, CellOwned::as_ref)
    }

    pub fn last_seen(&self) -> u64 {
        self.last_seen.load(Ordering::Relaxed)
    }
}

/// `Arc<MutableRow>` compared by identity, which is exactly the coalescing rule
/// `rangemap` needs: merge adjacent ranges only when they name the same row.
#[derive(Debug, Clone)]
struct RowSlot(Arc<MutableRow>);

impl PartialEq for RowSlot {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for RowSlot {}

#[derive(Debug)]
enum RangeStore {
    Integer(RangeInclusiveMap<i64, RowSlot>),
    Ip {
        v4: RangeInclusiveMap<u32, RowSlot>,
        v6: RangeInclusiveMap<u128, RowSlot>,
    },
}

impl RangeStore {
    fn new(range_type: RangeType) -> Self {
        match range_type {
            RangeType::Integer => RangeStore::Integer(RangeInclusiveMap::new()),
            RangeType::Ip => RangeStore::Ip {
                v4: RangeInclusiveMap::new(),
                v6: RangeInclusiveMap::new(),
            },
        }
    }

    fn len(&self) -> usize {
        match self {
            RangeStore::Integer(m) => m.len(),
            RangeStore::Ip { v4, v6 } => v4.len() + v6.len(),
        }
    }

    fn get(&self, point: RangePoint) -> Option<&RowSlot> {
        match (self, point) {
            (RangeStore::Integer(m), RangePoint::Integer(v)) => m.get(&v),
            (RangeStore::Ip { v4, .. }, RangePoint::Ip(IpAddr::V4(v))) => v4.get(&v.to_bits()),
            (RangeStore::Ip { v6, .. }, RangePoint::Ip(IpAddr::V6(v))) => v6.get(&v.to_bits()),
            _ => None,
        }
    }

    fn covers_exactly(&self, lo: RangePoint, hi: RangePoint) -> bool {
        match (self, lo, hi) {
            (RangeStore::Integer(m), RangePoint::Integer(a), RangePoint::Integer(b)) => m
                .get_key_value(&a)
                .is_some_and(|(r, _)| *r.start() == a && *r.end() == b),
            (
                RangeStore::Ip { v4, .. },
                RangePoint::Ip(IpAddr::V4(a)),
                RangePoint::Ip(IpAddr::V4(b)),
            ) => v4
                .get_key_value(&a.to_bits())
                .is_some_and(|(r, _)| *r.start() == a.to_bits() && *r.end() == b.to_bits()),
            (
                RangeStore::Ip { v6, .. },
                RangePoint::Ip(IpAddr::V6(a)),
                RangePoint::Ip(IpAddr::V6(b)),
            ) => v6
                .get_key_value(&a.to_bits())
                .is_some_and(|(r, _)| *r.start() == a.to_bits() && *r.end() == b.to_bits()),
            _ => false,
        }
    }

    /// The change in stored entry count when inserting a fresh `RowSlot`.
    fn insert_delta(&self, lo: RangePoint, hi: RangePoint) -> i64 {
        match (self, lo, hi) {
            (RangeStore::Integer(m), RangePoint::Integer(a), RangePoint::Integer(b)) => {
                range_insert_delta(m, a, b)
            }
            (
                RangeStore::Ip { v4, .. },
                RangePoint::Ip(IpAddr::V4(a)),
                RangePoint::Ip(IpAddr::V4(b)),
            ) => range_insert_delta(v4, a.to_bits(), b.to_bits()),
            (
                RangeStore::Ip { v6, .. },
                RangePoint::Ip(IpAddr::V6(a)),
                RangePoint::Ip(IpAddr::V6(b)),
            ) => range_insert_delta(v6, a.to_bits(), b.to_bits()),
            _ => unreachable!("normalized bounds must match the range store"),
        }
    }

    fn insert(&mut self, lo: RangePoint, hi: RangePoint, row: RowSlot) {
        match (self, lo, hi) {
            (RangeStore::Integer(m), RangePoint::Integer(a), RangePoint::Integer(b)) => {
                m.insert(a..=b, row)
            }
            (
                RangeStore::Ip { v4, .. },
                RangePoint::Ip(IpAddr::V4(a)),
                RangePoint::Ip(IpAddr::V4(b)),
            ) => v4.insert(a.to_bits()..=b.to_bits(), row),
            (
                RangeStore::Ip { v6, .. },
                RangePoint::Ip(IpAddr::V6(a)),
                RangePoint::Ip(IpAddr::V6(b)),
            ) => v6.insert(a.to_bits()..=b.to_bits(), row),
            _ => {}
        }
    }

    /// Drops the range covering `point`. Returns how many stored ranges went
    /// away (removal can also split, hence the signed accounting at the call
    /// site).
    fn remove_at(&mut self, point: RangePoint) -> i64 {
        let before = self.len() as i64;
        match (&mut *self, point) {
            (RangeStore::Integer(m), RangePoint::Integer(v)) => {
                if let Some((r, _)) = m.get_key_value(&v) {
                    let r = r.clone();
                    m.remove(r);
                }
            }
            (RangeStore::Ip { v4, .. }, RangePoint::Ip(IpAddr::V4(v))) => {
                if let Some((r, _)) = v4.get_key_value(&v.to_bits()) {
                    let r = r.clone();
                    v4.remove(r);
                }
            }
            (RangeStore::Ip { v6, .. }, RangePoint::Ip(IpAddr::V6(v))) => {
                if let Some((r, _)) = v6.get_key_value(&v.to_bits()) {
                    let r = r.clone();
                    v6.remove(r);
                }
            }
            _ => {}
        }
        before - self.len() as i64
    }

    /// Drops every range whose row last changed before `cutoff`.
    fn expire(&mut self, cutoff: u64) -> i64 {
        let before = self.len() as i64;
        match self {
            RangeStore::Integer(m) => {
                let stale: Vec<_> = m
                    .iter()
                    .filter(|(_, slot)| slot.0.last_seen() < cutoff)
                    .map(|(r, _)| r.clone())
                    .collect();
                for r in stale {
                    m.remove(r);
                }
            }
            RangeStore::Ip { v4, v6 } => {
                let stale: Vec<_> = v4
                    .iter()
                    .filter(|(_, slot)| slot.0.last_seen() < cutoff)
                    .map(|(r, _)| r.clone())
                    .collect();
                for r in stale {
                    v4.remove(r);
                }
                let stale: Vec<_> = v6
                    .iter()
                    .filter(|(_, slot)| slot.0.last_seen() < cutoff)
                    .map(|(r, _)| r.clone())
                    .collect();
                for r in stale {
                    v6.remove(r);
                }
            }
        }
        before - self.len() as i64
    }
}

fn range_insert_delta<K: RangeKey>(map: &RangeInclusiveMap<K, RowSlot>, lo: K, hi: K) -> i64 {
    // The new row has a fresh Arc identity, so it cannot coalesce with an
    // existing interval. Count its entry, subtract every overwritten entry,
    // and add back any surviving fragments at the two boundaries.
    map.overlapping(lo..=hi).fold(1, |delta, (range, _)| {
        delta - 1 + i64::from(*range.start() < lo) + i64::from(*range.end() > hi)
    })
}

/// Keyed by the owned key tuple. A `HashTable` rather than a `HashMap` so a
/// read can probe with borrowed `KeyRef`s and never allocate.
type HashEntry = (Box<[KeyOwned]>, Arc<MutableRow>);
type RangeEntry = (Box<[KeyOwned]>, RangeStore);

#[derive(Debug, Default)]
struct Shard {
    hash_rows: HashTable<HashEntry>,
    ranges: HashTable<RangeEntry>,
    /// long strings shared between rows; short ones live inline in `SmolStr`
    strings: HashSet<Arc<str>>,
    last_compact: Option<Instant>,
}

impl Shard {
    fn intern(&mut self, s: &str) -> SmolStr {
        if s.len() <= INLINE_LEN {
            SmolStr::new_inline(s)
        } else {
            // `SmolStr::from(Arc<str>)` keeps this very `Arc` for strings past
            // the inline threshold, so `strong_count` below stays meaningful
            SmolStr::from(self.strings.get_or_insert_with(s, |s| Arc::from(s)).clone())
        }
    }

    fn own_cell(&mut self, cell: CellRef<'_>) -> CellOwned {
        match cell {
            CellRef::String(s) => CellOwned::String(self.intern(s)),
            other => other.to_owned_cell(),
        }
    }

    fn own_key(&mut self, key: KeyRef<'_>) -> KeyOwned {
        match key {
            KeyRef::String(s) => KeyOwned::String(self.intern(s)),
            other => other.to_owned_key(),
        }
    }

    fn compact(&mut self, force: bool) -> usize {
        if !force
            && self
                .last_compact
                .is_some_and(|t| t.elapsed() < COMPACT_INTERVAL)
        {
            return 0;
        }
        let before = self.strings.len();
        // only the pool holds a reference to an unused string
        self.strings.retain(|s| Arc::strong_count(s) > 1);
        self.last_compact = Some(Instant::now());
        before - self.strings.len()
    }
}

/// A table written by the pipeline and read by processors.
///
/// Sharded by key hash so writers rarely contend. Reads clone an `Arc` and drop
/// the lock immediately, so a reader never holds one across a lookup.
#[derive(Debug)]
pub struct Mutable {
    schema: Arc<Schema>,
    key: KeyPlan,
    shards: Box<[RwLock<Shard>]>,
    shard_mask: usize,
    /// millis a row survives untouched; zero disables expiry. Atomic because
    /// [`Self::reconfigure`] retunes it without rebuilding the table.
    ttl: AtomicU64,
    max_rows: AtomicUsize,
    len: AtomicUsize,
    rejected: AtomicU64,
    /// `rejected` as of the last [`Self::take_rejected_delta`]; log-only.
    rejected_reported: AtomicU64,
}

impl Mutable {
    pub fn new(schema: Arc<Schema>, key: KeyPlan, ttl: Option<Duration>, max_rows: usize) -> Self {
        let shards = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .next_power_of_two()
            .clamp(1, MAX_SHARDS);
        Self {
            schema,
            key,
            shards: (0..shards)
                .map(|_| RwLock::new(Shard::default()))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            shard_mask: shards - 1,
            ttl: AtomicU64::new(ttl_millis(ttl)),
            max_rows: AtomicUsize::new(max_rows.max(1)),
            len: AtomicUsize::new(0),
            rejected: AtomicU64::new(0),
            rejected_reported: AtomicU64::new(0),
        }
    }

    pub fn schema(&self) -> &Arc<Schema> {
        &self.schema
    }

    pub fn key_plan(&self) -> &KeyPlan {
        &self.key
    }

    pub fn len(&self) -> usize {
        self.len.load(Ordering::Relaxed)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn max_rows(&self) -> usize {
        self.max_rows.load(Ordering::Relaxed)
    }

    /// Retunes the settings that do not describe the rows themselves, which is
    /// what lets a processor-fed table survive a rename or a retuned TTL — see
    /// `super::reuses_rows` for what counts as structural.
    ///
    /// Lowering `max_rows` below the current count evicts nothing: new keys are
    /// turned away until the sweep drains it, as the cap already does.
    pub fn reconfigure(&self, ttl: Option<Duration>, max_rows: usize) {
        self.ttl.store(ttl_millis(ttl), Ordering::Relaxed);
        self.max_rows.store(max_rows.max(1), Ordering::Relaxed);
    }

    /// How many writes were turned away because they would exceed `max_rows`.
    pub fn rejected(&self) -> u64 {
        self.rejected.load(Ordering::Relaxed)
    }

    /// Writes rejected since the previous call. Only feeds a log line.
    pub fn take_rejected_delta(&self) -> u64 {
        let now = self.rejected();
        now.saturating_sub(self.rejected_reported.swap(now, Ordering::Relaxed))
    }

    pub fn shards(&self) -> usize {
        self.shards.len()
    }

    /// The shard one key belongs to, taken from the *high* half of the hash.
    ///
    /// Not the low bits, even though they are the obvious choice: the shard's
    /// own `HashTable` is handed this very same hash, and hashbrown places an
    /// entry at `h1(hash) & bucket_mask` where `h1(hash)` is just
    /// `hash as usize`. Sharding on the low bits therefore fixes those bits for
    /// every key in a shard, so its probe positions are all congruent to the
    /// shard index modulo the shard count — leaving only `Group::WIDTH` buckets
    /// reachable on the first probe out of every `shards`, and pushing the rest
    /// of the table behind extra probe groups long before hashbrown would
    /// otherwise grow. Bits 32.. are read by neither the bucket index nor the
    /// top-7 control tag, so they are free.
    ///
    /// Shifting by a constant rather than by `shard_mask.count_ones()` keeps the
    /// single-shard case (`shard_mask == 0`) from becoming a shift by 64.
    #[inline]
    fn shard_of(&self, hash: u64) -> &RwLock<Shard> {
        &self.shards[((hash >> 32) as usize) & self.shard_mask]
    }

    /// Reserves additional entries against `max_rows`, atomically, so writers on
    /// different shards cannot together overshoot the cap.
    fn reserve(&self, additional: usize) -> bool {
        self.len
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                n.checked_add(additional)
                    .filter(|&next| next <= self.max_rows())
            })
            .is_ok()
    }

    fn adjust_len(&self, delta: i64) {
        match delta.cmp(&0) {
            std::cmp::Ordering::Greater => {
                self.len.fetch_add(delta as usize, Ordering::AcqRel);
            }
            std::cmp::Ordering::Less => {
                self.len.fetch_sub((-delta) as usize, Ordering::AcqRel);
            }
            std::cmp::Ordering::Equal => {}
        }
    }

    fn check_cells(&self, cells: &[CellRef<'_>]) -> Result<()> {
        if cells.len() != self.schema.len() {
            bail!("expected {} cells, got {}", self.schema.len(), cells.len());
        }
        for (i, cell) in cells.iter().enumerate() {
            let ty = self.schema.value_type(i as ColumnIdx).unwrap_or_default();
            if !cell_matches(*cell, ty) {
                bail!(
                    "column {:?} expects {ty:?} but got {cell:?}",
                    self.schema.name(i as ColumnIdx).unwrap_or_default()
                );
            }
        }
        Ok(())
    }

    fn key_parts<'a>(
        &self,
        cells: &[CellRef<'a>],
        columns: &[ColumnIdx],
    ) -> Result<SmallVec<[KeyRef<'a>; 8]>> {
        columns
            .iter()
            .map(|c| KeyRef::try_from_cell(cells[*c as usize]))
            .collect()
    }

    /// Inserts or updates a row in an exact-match table. The key is taken
    /// from the row itself, using the table's key columns.
    pub fn upsert_hash(&self, cells: &[CellRef<'_>]) -> Result<Upsert> {
        let KeyPlan::Hash { columns } = &self.key else {
            bail!("this table is range-keyed; use upsert_range");
        };
        self.check_cells(cells)?;
        let parts = self.key_parts(cells, columns)?;
        let hash = hash_key(parts.iter().copied());
        let now = now_millis();

        // the lock is held across the capacity check and the write, so a
        // concurrent writer cannot slip a row in between the two
        let mut shard = self.shard_of(hash).write();
        let existing = shard
            .hash_rows
            .find(hash, |(k, _)| keys_eq(k, &parts))
            .is_some();
        if !existing && !self.reserve(1) {
            self.rejected.fetch_add(1, Ordering::Relaxed);
            return Ok(Upsert::Rejected);
        }

        let owned: Vec<CellOwned> = cells.iter().map(|c| shard.own_cell(*c)).collect();
        let row = Arc::new(MutableRow {
            cells: owned.into_boxed_slice(),
            last_seen: AtomicU64::new(now),
        });
        if let Some(entry) = shard.hash_rows.find_mut(hash, |(k, _)| keys_eq(k, &parts)) {
            // rows are shared behind an `Arc`, so an update swaps in a new one
            // rather than mutating what readers may already hold
            entry.1 = row;
            return Ok(Upsert::Updated);
        }
        let key: Box<[KeyOwned]> = parts.iter().map(|k| shard.own_key(*k)).collect();
        shard
            .hash_rows
            .insert_unique(hash, (key, row), |(k, _)| hash_key_owned(k));
        Ok(Upsert::Inserted)
    }

    /// Inserts or updates a range in a range-keyed table. The partition is
    /// taken from the row; the bounds are supplied separately because they are
    /// not stored as columns. `max_rows` limits the resulting stored intervals,
    /// including fragments left by overwriting part of an existing range.
    pub fn upsert_range(
        &self,
        cells: &[CellRef<'_>],
        start: RangePoint,
        start_inclusive: bool,
        end: RangePoint,
        end_inclusive: bool,
    ) -> Result<Upsert> {
        let KeyPlan::Range {
            partition,
            range_type,
        } = &self.key
        else {
            bail!("this table is hash-keyed; use upsert_hash");
        };
        self.check_cells(cells)?;
        let (lo, hi) = normalize_point(start, start_inclusive, end, end_inclusive, *range_type)
            .map_err(|e| anyhow::anyhow!("range bounds rejected: {e:?}"))?;
        let parts = self.key_parts(cells, partition)?;
        let hash = hash_key(parts.iter().copied());
        let now = now_millis();

        let mut shard = self.shard_of(hash).write();
        let (is_update, delta) = shard
            .ranges
            .find(hash, |(k, _)| keys_eq(k, &parts))
            .map_or((false, 1), |(_, store)| {
                (store.covers_exactly(lo, hi), store.insert_delta(lo, hi))
            });
        // Reserve the full growth before changing the map. Splitting one
        // interval into three needs two slots; shrinking needs none.
        if delta > 0 && !self.reserve(delta as usize) {
            self.rejected.fetch_add(1, Ordering::Relaxed);
            return Ok(Upsert::Rejected);
        }

        let owned: Vec<CellOwned> = cells.iter().map(|c| shard.own_cell(*c)).collect();
        let row = RowSlot(Arc::new(MutableRow {
            cells: owned.into_boxed_slice(),
            last_seen: AtomicU64::new(now),
        }));
        if shard
            .ranges
            .find(hash, |(k, _)| keys_eq(k, &parts))
            .is_none()
        {
            let key: Box<[KeyOwned]> = parts.iter().map(|k| shard.own_key(*k)).collect();
            shard
                .ranges
                .insert_unique(hash, (key, RangeStore::new(*range_type)), |(k, _)| {
                    hash_key_owned(k)
                });
        }
        let store = &mut shard
            .ranges
            .find_mut(hash, |(k, _)| keys_eq(k, &parts))
            .expect("just inserted")
            .1;
        let before = store.len() as i64;
        store.insert(lo, hi, row);
        debug_assert_eq!(store.len() as i64 - before, delta);

        // Release capacity only after the old entries are gone, while still
        // holding the shard lock. Positive deltas were already reserved.
        if delta < 0 {
            self.adjust_len(delta);
        }
        Ok(if is_update {
            Upsert::Updated
        } else {
            Upsert::Inserted
        })
    }

    pub fn get(&self, key: Lookup<'_>) -> Option<Arc<MutableRow>> {
        match (&self.key, key) {
            (KeyPlan::Hash { columns }, Lookup::Hash(parts)) => {
                if parts.len() != columns.len() {
                    return None;
                }
                let hash = hash_key(parts.iter().copied());
                let shard = self.shard_of(hash).read();
                shard
                    .hash_rows
                    .find(hash, |(k, _)| keys_eq(k, parts))
                    .map(|(_, row)| Arc::clone(row))
            }
            (
                KeyPlan::Range { partition, .. },
                Lookup::Range {
                    partition: p,
                    point,
                },
            ) => {
                if p.len() != partition.len() {
                    return None;
                }
                let hash = hash_key(p.iter().copied());
                let shard = self.shard_of(hash).read();
                shard
                    .ranges
                    .find(hash, |(k, _)| keys_eq(k, p))
                    .and_then(|(_, store)| store.get(point))
                    .map(|slot| Arc::clone(&slot.0))
            }
            _ => None,
        }
    }

    /// Drops the row (or the range covering the point) that `key` names.
    pub fn remove(&self, key: Lookup<'_>) -> bool {
        match (&self.key, key) {
            (KeyPlan::Hash { columns }, Lookup::Hash(parts)) => {
                if parts.len() != columns.len() {
                    return false;
                }
                let hash = hash_key(parts.iter().copied());
                let mut shard = self.shard_of(hash).write();
                let removed = shard
                    .hash_rows
                    .find_entry(hash, |(k, _)| keys_eq(k, parts))
                    .map(|entry| entry.remove())
                    .is_ok();
                if removed {
                    self.adjust_len(-1);
                }
                removed
            }
            (
                KeyPlan::Range { partition, .. },
                Lookup::Range {
                    partition: p,
                    point,
                },
            ) => {
                if p.len() != partition.len() {
                    return false;
                }
                let hash = hash_key(p.iter().copied());
                let mut shard = self.shard_of(hash).write();
                let removed =
                    if let Ok(mut entry) = shard.ranges.find_entry(hash, |(k, _)| keys_eq(k, p)) {
                        let removed = entry.get_mut().1.remove_at(point);
                        // An empty partition must release its owned key too, or
                        // the string pool cannot reclaim it on the next sweep.
                        if entry.get().1.len() == 0 {
                            let _ = entry.remove();
                        }
                        removed
                    } else {
                        0
                    };
                self.adjust_len(-removed);
                removed > 0
            }
            _ => false,
        }
    }

    /// Drops rows untouched for longer than the table's TTL. Returns how many
    /// stored entries went away.
    pub fn expire(&self, now: u64) -> usize {
        let ttl = self.ttl.load(Ordering::Relaxed);
        if ttl == 0 {
            return 0;
        }
        let cutoff = now.saturating_sub(ttl);
        let mut dropped = 0i64;
        for shard in &self.shards {
            let mut shard = shard.write();
            let before = shard.hash_rows.len() as i64;
            shard.hash_rows.retain(|(_, row)| row.last_seen() >= cutoff);
            let mut shard_dropped = before - shard.hash_rows.len() as i64;
            for (_, store) in shard.ranges.iter_mut() {
                shard_dropped += store.expire(cutoff);
            }
            shard.ranges.retain(|(_, store)| store.len() > 0);
            self.adjust_len(-shard_dropped);
            dropped += shard_dropped;
        }
        dropped.max(0) as usize
    }

    /// Sweeps unreferenced entries out of the string pools. Rate-limited unless
    /// `force`. Returns how many pooled strings were released.
    pub fn compact(&self, force: bool) -> usize {
        self.shards.iter().map(|s| s.write().compact(force)).sum()
    }

    pub fn pooled_strings(&self) -> usize {
        self.shards.iter().map(|s| s.read().strings.len()).sum()
    }

    pub fn clear(&self) {
        // Lock in shard order, before clearing anything. Every mutation updates
        // capacity under its shard lock, so the final reset cannot erase a
        // concurrent reservation or race a delayed removal decrement.
        let mut shards: Vec<_> = self.shards.iter().map(|s| s.write()).collect();
        for shard in &mut shards {
            shard.hash_rows.clear();
            shard.ranges.clear();
            shard.strings.clear();
        }
        // hash rows and range entries alike, so zero rather than a decrement
        self.len.store(0, Ordering::Release);
    }
}

/// A write handle for one batch, attributing every write to one processor.
///
/// Passes each [`Upsert`] straight back, so a caller can still act on a
/// `Rejected` while it is recorded. Like [`super::TableRead`], the tally is
/// unsynchronized and reaches the shared counters when the handle drops.
#[derive(Debug)]
pub struct TableWrite<'a> {
    table: &'a Mutable,
    tally: Option<WriteTally>,
}

#[derive(Debug)]
struct WriteTally {
    metrics: Arc<ProcessorMetrics>,
    inserted: Cell<u64>,
    updated: Cell<u64>,
    rejected: Cell<u64>,
    errored: Cell<u64>,
    removes: Cell<u64>,
}

impl WriteTally {
    #[inline]
    fn record(&self, slot: &Cell<u64>) {
        slot.set(slot.get() + 1);
    }
}

impl Drop for WriteTally {
    fn drop(&mut self) {
        self.metrics.add_writes(&WriteCounts {
            inserted: self.inserted.get(),
            updated: self.updated.get(),
            rejected: self.rejected.get(),
            errored: self.errored.get(),
            removes: self.removes.get(),
        });
    }
}

impl<'a> TableWrite<'a> {
    /// An unattributed handle, for the loader and for tests.
    pub fn unattributed(table: &'a Mutable) -> Self {
        Self { table, tally: None }
    }

    pub(super) fn attributed(table: &'a Mutable, metrics: Arc<ProcessorMetrics>) -> Self {
        Self {
            table,
            tally: Some(WriteTally {
                metrics,
                inserted: Cell::new(0),
                updated: Cell::new(0),
                rejected: Cell::new(0),
                errored: Cell::new(0),
                removes: Cell::new(0),
            }),
        }
    }

    /// The table underneath, for the reads and gauges that are not writes.
    pub fn table(&self) -> &Mutable {
        self.table
    }

    pub fn upsert_hash(&self, cells: &[CellRef<'_>]) -> Result<Upsert> {
        self.record(self.table.upsert_hash(cells))
    }

    pub fn upsert_range(
        &self,
        cells: &[CellRef<'_>],
        start: RangePoint,
        start_inclusive: bool,
        end: RangePoint,
        end_inclusive: bool,
    ) -> Result<Upsert> {
        self.record(
            self.table
                .upsert_range(cells, start, start_inclusive, end, end_inclusive),
        )
    }

    pub fn remove(&self, key: Lookup<'_>) -> bool {
        let removed = self.table.remove(key);
        if removed && let Some(tally) = &self.tally {
            tally.record(&tally.removes);
        }
        removed
    }

    /// Records a record whose row could not be built at all — a missing or
    /// uncastable source attribute, so there was nothing to write.
    pub fn note_key_error(&self) {
        if let Some(tally) = &self.tally {
            tally.record(&tally.errored);
        }
    }

    fn record(&self, outcome: Result<Upsert>) -> Result<Upsert> {
        if let Some(tally) = &self.tally {
            tally.record(match &outcome {
                Ok(Upsert::Inserted) => &tally.inserted,
                Ok(Upsert::Updated) => &tally.updated,
                Ok(Upsert::Rejected) => &tally.rejected,
                Err(_) => &tally.errored,
            });
        }
        outcome
    }
}

/// Zero means "never expire", which is also how a `None` arrives here — so a
/// sub-millisecond TTL rounds *up*, or truncating it would disable expiry.
fn ttl_millis(ttl: Option<Duration>) -> u64 {
    match ttl.filter(|t| !t.is_zero()) {
        None => 0,
        Some(t) => (t.as_millis() as u64).max(1),
    }
}

fn cell_matches(cell: CellRef<'_>, ty: ValueType) -> bool {
    matches!(
        (cell, ty),
        (CellRef::Null, _)
            | (CellRef::Bool(_), ValueType::Bool)
            | (CellRef::Double(_), ValueType::Double)
            | (CellRef::Integer(_), ValueType::Integer)
            | (CellRef::String(_), ValueType::String)
            | (CellRef::Ip(_), ValueType::Ip)
    )
}

/// Folds arbitrary inclusivity into a closed range, in the point domain.
fn normalize_point(
    start: RangePoint,
    start_inclusive: bool,
    end: RangePoint,
    end_inclusive: bool,
    range_type: RangeType,
) -> Result<(RangePoint, RangePoint), RangeReject> {
    match (range_type, start, end) {
        (RangeType::Integer, RangePoint::Integer(s), RangePoint::Integer(e)) => {
            let (lo, hi) =
                normalize(s, start_inclusive, e, end_inclusive).ok_or(RangeReject::Empty)?;
            Ok((RangePoint::Integer(lo), RangePoint::Integer(hi)))
        }
        (RangeType::Ip, RangePoint::Ip(IpAddr::V4(s)), RangePoint::Ip(IpAddr::V4(e))) => {
            let (lo, hi) = normalize(s.to_bits(), start_inclusive, e.to_bits(), end_inclusive)
                .ok_or(RangeReject::Empty)?;
            Ok((
                RangePoint::Ip(IpAddr::V4(lo.into())),
                RangePoint::Ip(IpAddr::V4(hi.into())),
            ))
        }
        (RangeType::Ip, RangePoint::Ip(IpAddr::V6(s)), RangePoint::Ip(IpAddr::V6(e))) => {
            let (lo, hi) = normalize(s.to_bits(), start_inclusive, e.to_bits(), end_inclusive)
                .ok_or(RangeReject::Empty)?;
            Ok((
                RangePoint::Ip(IpAddr::V6(lo.into())),
                RangePoint::Ip(IpAddr::V6(hi.into())),
            ))
        }
        _ => Err(RangeReject::Mismatch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::lookup_table::schema::ColumnLayout;
    use crate::config::lookup_table as cfg;

    fn table(
        columns: &[(&str, ValueType)],
        key: cfg::Key,
        ttl: Option<Duration>,
        max_rows: usize,
    ) -> Mutable {
        let source: Vec<(SmolStr, ValueType)> = columns
            .iter()
            .map(|(n, t)| (SmolStr::new(*n), *t))
            .collect();
        let (_, schema, plan) = ColumnLayout::build(&source, &key).unwrap();
        Mutable::new(Arc::new(schema), plan, ttl, max_rows)
    }

    fn hash_table(max_rows: usize, ttl: Option<Duration>) -> Mutable {
        table(
            &[("host", ValueType::String), ("site", ValueType::String)],
            cfg::Key::Hash(cfg::Hash {
                key_columns: vec!["host".into()],
            }),
            ttl,
            max_rows,
        )
    }

    /// The shard index must not consume the bits hashbrown places entries on.
    ///
    /// Pinned as a property rather than as a timing test: a regression here is
    /// completely silent — every answer stays correct, only probe chains get
    /// longer. Sharding on the low bits pins every key in a shard to
    /// `pos ≡ shard (mod shards)` inside that shard's own table, because
    /// hashbrown's `h1(hash)` is just `hash as usize`.
    #[test]
    fn the_shard_index_ignores_the_bits_hashbrown_places_on() {
        let t = hash_table(16, None);
        // hashbrown's bucket index lives in the low half; changing all of it
        // must not move a key to a different shard
        let base = 0xdead_beef_0000_0000u64;
        for low in [1u64, 0xff, 0x1234_5678, 0xffff_ffff] {
            assert!(
                std::ptr::eq(t.shard_of(base), t.shard_of(base | low)),
                "low bits {low:#x} moved the shard"
            );
        }
        // and the high half must still reach every shard
        let reached: HashSet<*const RwLock<Shard>> = (0..t.shards() as u64)
            .map(|i| t.shard_of(i << 32) as *const _)
            .collect();
        assert_eq!(reached.len(), t.shards(), "every shard must be reachable");
    }

    #[test]
    fn shared_clear_preserves_accounting_under_concurrent_mutations() {
        for range in [false, true] {
            let t = if range {
                table(
                    &[
                        ("host", ValueType::String),
                        ("start", ValueType::Integer),
                        ("end", ValueType::Integer),
                        ("site", ValueType::String),
                    ],
                    cfg::Key::Range(cfg::Range {
                        range_type: RangeType::Integer,
                        partition_columns: vec!["host".into()],
                        start_column: "start".into(),
                        end_column: "end".into(),
                        start_inclusive: true,
                        end_inclusive: true,
                    }),
                    Some(Duration::from_millis(1)),
                    64,
                )
            } else {
                hash_table(64, Some(Duration::from_millis(1)))
            };
            std::thread::scope(|scope| {
                for worker in 0..4 {
                    let t = &t;
                    scope.spawn(move || {
                        for i in 0..1000 {
                            let host = format!("{worker}-{}", i % 32);
                            let cells = [CellRef::String(&host), CellRef::String("value")];
                            if range {
                                let _ = t
                                    .upsert_range(
                                        &cells,
                                        RangePoint::Integer(1),
                                        true,
                                        RangePoint::Integer(10),
                                        true,
                                    )
                                    .unwrap();
                            } else {
                                let _ = t.upsert_hash(&cells).unwrap();
                            }
                            if i % 3 == 0 {
                                let parts = [KeyRef::String(&host)];
                                let probe = if range {
                                    Lookup::Range {
                                        partition: &parts,
                                        point: RangePoint::Integer(5),
                                    }
                                } else {
                                    Lookup::Hash(&parts)
                                };
                                t.remove(probe);
                            }
                        }
                    });
                }
                scope.spawn(|| {
                    for _ in 0..200 {
                        t.clear();
                        std::thread::yield_now();
                    }
                });
                scope.spawn(|| {
                    for _ in 0..200 {
                        t.expire(u64::MAX);
                        std::thread::yield_now();
                    }
                });
            });
            let actual: usize = t
                .shards
                .iter()
                .map(|shard| {
                    let shard = shard.read();
                    shard.hash_rows.len()
                        + shard
                            .ranges
                            .iter()
                            .map(|(_, ranges)| ranges.len())
                            .sum::<usize>()
                })
                .sum();
            assert_eq!(t.len(), actual);
            assert!(t.len() <= t.max_rows());
            t.clear();
            assert_eq!(t.len(), 0);
            assert_eq!(t.pooled_strings(), 0);
        }
    }

    #[test]
    fn clear_excludes_writers_until_every_shard_has_been_cleared() {
        let t = hash_table(64, None);
        if t.shards.len() < 2 {
            return;
        }
        let host = (0..10_000)
            .map(|i| format!("host-{i}"))
            .find(|host| std::ptr::eq(t.shard_of(hash_key([KeyRef::String(host)])), &t.shards[0]))
            .unwrap();
        let cells = [CellRef::String(&host), CellRef::String("old")];
        let _ = t.upsert_hash(&cells).unwrap();
        let snapshot = t.get(Lookup::Hash(&[KeyRef::String(&host)])).unwrap();
        std::thread::scope(|scope| {
            // Hold the last shard so clear has acquired the first but cannot finish.
            let last = t.shards.last().unwrap().write();
            let clear = scope.spawn(|| t.clear());
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while t.shards[0].try_read().is_some() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "clear did not acquire the first shard"
                );
                std::thread::yield_now();
            }
            let writer = scope.spawn(|| t.upsert_hash(&cells).unwrap());
            drop(last);
            clear.join().unwrap();
            assert_eq!(writer.join().unwrap(), Upsert::Inserted);
        });
        assert_eq!(t.len(), 1);
        assert_eq!(snapshot.cell(1), CellRef::String("old"));
    }

    fn range_table(max_rows: usize) -> Mutable {
        table(
            &[
                ("start", ValueType::Integer),
                ("end", ValueType::Integer),
                ("label", ValueType::String),
            ],
            cfg::Key::Range(cfg::Range {
                range_type: RangeType::Integer,
                partition_columns: vec![],
                start_column: "start".into(),
                start_inclusive: true,
                end_column: "end".into(),
                end_inclusive: true,
            }),
            None,
            max_rows,
        )
    }

    fn row(t: &Mutable, host: &str) -> Option<SmolStr> {
        let site = t.schema().index_of("site").unwrap();
        t.get(Lookup::Hash(&[KeyRef::String(host)]))
            .map(|r| SmolStr::new(r.cell(site).as_str().unwrap()))
    }

    #[test]
    fn upsert_inserts_then_updates() {
        let t = hash_table(10, None);
        assert_eq!(
            t.upsert_hash(&[CellRef::String("a"), CellRef::String("jkt")])
                .unwrap(),
            Upsert::Inserted
        );
        assert_eq!(t.len(), 1);
        assert_eq!(row(&t, "a").as_deref(), Some("jkt"));

        assert_eq!(
            t.upsert_hash(&[CellRef::String("a"), CellRef::String("bdg")])
                .unwrap(),
            Upsert::Updated
        );
        assert_eq!(t.len(), 1, "an update adds no row");
        assert_eq!(row(&t, "a").as_deref(), Some("bdg"));
        assert_eq!(row(&t, "b"), None);
    }

    #[test]
    fn a_reader_holding_a_row_is_unaffected_by_a_later_update() {
        let t = hash_table(10, None);
        let _ = t
            .upsert_hash(&[CellRef::String("a"), CellRef::String("jkt")])
            .unwrap();
        let site = t.schema().index_of("site").unwrap();
        let held = t.get(Lookup::Hash(&[KeyRef::String("a")])).unwrap();
        let _ = t
            .upsert_hash(&[CellRef::String("a"), CellRef::String("bdg")])
            .unwrap();
        assert_eq!(held.cell(site).as_str(), Some("jkt"), "snapshot is stable");
        assert_eq!(row(&t, "a").as_deref(), Some("bdg"));
    }

    #[test]
    fn remove_frees_the_row_and_the_slot() {
        let t = hash_table(10, None);
        let _ = t
            .upsert_hash(&[CellRef::String("a"), CellRef::String("jkt")])
            .unwrap();
        assert!(t.remove(Lookup::Hash(&[KeyRef::String("a")])));
        assert_eq!(t.len(), 0);
        assert!(
            !t.remove(Lookup::Hash(&[KeyRef::String("a")])),
            "already gone"
        );
        assert_eq!(row(&t, "a"), None);
    }

    #[test]
    fn max_rows_rejects_new_keys_but_still_accepts_updates() {
        let t = hash_table(2, None);
        assert_eq!(
            t.upsert_hash(&[CellRef::String("a"), CellRef::String("1")])
                .unwrap(),
            Upsert::Inserted
        );
        assert_eq!(
            t.upsert_hash(&[CellRef::String("b"), CellRef::String("2")])
                .unwrap(),
            Upsert::Inserted
        );
        assert_eq!(t.len(), 2);

        // full: a new key is turned away and nothing changes
        assert_eq!(
            t.upsert_hash(&[CellRef::String("c"), CellRef::String("3")])
                .unwrap(),
            Upsert::Rejected
        );
        assert_eq!(t.len(), 2, "a rejected insert must not grow the table");
        assert_eq!(row(&t, "c"), None);
        assert_eq!(t.rejected(), 1);

        // ...but an existing key still updates in place
        assert_eq!(
            t.upsert_hash(&[CellRef::String("a"), CellRef::String("9")])
                .unwrap(),
            Upsert::Updated
        );
        assert_eq!(row(&t, "a").as_deref(), Some("9"));
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn removing_a_row_makes_room_again() {
        let t = hash_table(1, None);
        let _ = t
            .upsert_hash(&[CellRef::String("a"), CellRef::String("1")])
            .unwrap();
        assert_eq!(
            t.upsert_hash(&[CellRef::String("b"), CellRef::String("2")])
                .unwrap(),
            Upsert::Rejected
        );
        t.remove(Lookup::Hash(&[KeyRef::String("a")]));
        assert_eq!(
            t.upsert_hash(&[CellRef::String("b"), CellRef::String("2")])
                .unwrap(),
            Upsert::Inserted
        );
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn concurrent_writers_never_exceed_the_cap() {
        const CAP: usize = 64;
        let t = Arc::new(hash_table(CAP, None));
        std::thread::scope(|scope| {
            for w in 0..8 {
                let t = Arc::clone(&t);
                scope.spawn(move || {
                    for i in 0..200 {
                        let host = format!("host-{w}-{i}");
                        let _ = t.upsert_hash(&[CellRef::String(&host), CellRef::String("x")]);
                    }
                });
            }
        });
        assert_eq!(t.len(), CAP, "the cap is exact, not approximate");
        assert!(t.rejected() > 0);
    }

    #[test]
    fn expiry_drops_rows_older_than_the_ttl() {
        let ttl = Duration::from_millis(50);
        let t = hash_table(10, Some(ttl));
        let _ = t
            .upsert_hash(&[CellRef::String("a"), CellRef::String("1")])
            .unwrap();
        let now = now_millis();
        assert_eq!(t.expire(now), 0, "nothing is stale yet");
        assert_eq!(t.len(), 1);

        assert_eq!(t.expire(now + 51), 1);
        assert_eq!(t.len(), 0);
        assert_eq!(row(&t, "a"), None);
    }

    #[test]
    fn a_table_without_a_ttl_never_expires() {
        let t = hash_table(10, None);
        let _ = t
            .upsert_hash(&[CellRef::String("a"), CellRef::String("1")])
            .unwrap();
        assert_eq!(t.expire(now_millis() + 1_000_000), 0);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn expiry_makes_room_again() {
        let ttl = Duration::from_millis(10);
        let t = hash_table(1, Some(ttl));
        let _ = t
            .upsert_hash(&[CellRef::String("a"), CellRef::String("1")])
            .unwrap();
        assert_eq!(
            t.upsert_hash(&[CellRef::String("b"), CellRef::String("2")])
                .unwrap(),
            Upsert::Rejected
        );
        t.expire(now_millis() + 11);
        assert_eq!(t.len(), 0);
        assert_eq!(
            t.upsert_hash(&[CellRef::String("b"), CellRef::String("2")])
                .unwrap(),
            Upsert::Inserted
        );
    }

    #[test]
    fn range_upsert_and_point_lookup() {
        let t = range_table(10);
        let label = t.schema().index_of("label").unwrap();
        assert_eq!(
            t.upsert_range(
                &[CellRef::String("low")],
                RangePoint::Integer(0),
                true,
                RangePoint::Integer(9),
                true
            )
            .unwrap(),
            Upsert::Inserted
        );
        let at = |p: i64| {
            t.get(Lookup::Range {
                partition: &[],
                point: RangePoint::Integer(p),
            })
            .map(|r| SmolStr::new(r.cell(label).as_str().unwrap()))
        };
        assert_eq!(at(5).as_deref(), Some("low"));
        assert_eq!(at(10), None);

        // the same bounds again is an update, not a second row
        assert_eq!(
            t.upsert_range(
                &[CellRef::String("still-low")],
                RangePoint::Integer(0),
                true,
                RangePoint::Integer(9),
                true
            )
            .unwrap(),
            Upsert::Updated
        );
        assert_eq!(t.len(), 1);
        assert_eq!(at(5).as_deref(), Some("still-low"));
    }

    #[test]
    fn range_removal_and_cap_are_counted_per_range() {
        let t = range_table(2);
        for i in 0..2i64 {
            assert_eq!(
                t.upsert_range(
                    &[CellRef::String("x")],
                    RangePoint::Integer(i * 100),
                    true,
                    RangePoint::Integer(i * 100 + 10),
                    true
                )
                .unwrap(),
                Upsert::Inserted
            );
        }
        assert_eq!(t.len(), 2);
        assert_eq!(
            t.upsert_range(
                &[CellRef::String("x")],
                RangePoint::Integer(500),
                true,
                RangePoint::Integer(510),
                true
            )
            .unwrap(),
            Upsert::Rejected,
            "one range is one row against the cap"
        );
        assert!(t.remove(Lookup::Range {
            partition: &[],
            point: RangePoint::Integer(5)
        }));
        assert_eq!(t.len(), 1);
        assert!(
            t.get(Lookup::Range {
                partition: &[],
                point: RangePoint::Integer(5)
            })
            .is_none()
        );
    }

    #[test]
    fn range_insert_delta_matches_overlaps_gaps_and_domain_boundaries() {
        let fresh_row = || {
            RowSlot(Arc::new(MutableRow {
                cells: Box::new([]),
                last_seen: AtomicU64::new(0),
            }))
        };
        let bounds = [i64::MIN, -6, -5, -4, -1, 0, 1, 4, 5, 6, 7, 10, 11, i64::MAX];
        for populated in [false, true] {
            let mut original = RangeInclusiveMap::new();
            if populated {
                let old = fresh_row();
                for range in [i64::MIN..=-5, 0..=4, 6..=10, i64::MAX..=i64::MAX] {
                    original.insert(range, old.clone());
                }
            }
            for lo in bounds {
                for hi in bounds.into_iter().filter(|&hi| hi >= lo) {
                    let delta = range_insert_delta(&original, lo, hi);
                    let mut updated = original.clone();
                    updated.insert(lo..=hi, fresh_row());
                    assert_eq!(
                        delta,
                        updated.len() as i64 - original.len() as i64,
                        "populated={populated}, bounds={lo}..={hi}"
                    );
                }
            }
        }
    }

    fn upsert_integer_range(t: &Mutable, lo: i64, hi: i64, label: &str) -> Upsert {
        t.upsert_range(
            &[CellRef::String(label)],
            RangePoint::Integer(lo),
            true,
            RangePoint::Integer(hi),
            true,
        )
        .unwrap()
    }

    #[test]
    fn range_splits_reserve_both_extra_entries_before_writing() {
        let replacement = "a-new-long-label-that-must-not-be-pooled-on-rejection";
        for cap in [2, 3] {
            let t = range_table(cap);
            assert_eq!(upsert_integer_range(&t, 0, 10, "old"), Upsert::Inserted);
            let at = |point| {
                t.get(Lookup::Range {
                    partition: &[],
                    point: RangePoint::Integer(point),
                })
                .unwrap()
            };
            let held = at(5);
            let result = upsert_integer_range(&t, 3, 7, replacement);
            if cap == 2 {
                assert_eq!(result, Upsert::Rejected);
                assert_eq!(t.len(), 1);
                assert_eq!(t.rejected(), 1);
                assert_eq!(t.pooled_strings(), 0);
                assert!(Arc::ptr_eq(&held, &at(5)));
            } else {
                assert_eq!(result, Upsert::Inserted);
                assert_eq!(t.len(), 3);
                assert_eq!(t.rejected(), 0);
                let label = t.schema().index_of("label").unwrap();
                assert_eq!(at(5).cell(label).as_str(), Some(replacement));
                // An exact replacement still succeeds when all three slots are used.
                assert_eq!(upsert_integer_range(&t, 3, 7, "updated"), Upsert::Updated);
                assert_eq!(t.len(), 3);
            }
            for point in [0, 2, 8, 10] {
                assert!(Arc::ptr_eq(&held, &at(point)));
            }
        }
    }

    #[test]
    fn range_overwrites_at_capacity_can_preserve_or_free_slots() {
        let t = range_table(2);
        assert_eq!(upsert_integer_range(&t, 0, 9, "first"), Upsert::Inserted);
        assert_eq!(upsert_integer_range(&t, 20, 29, "second"), Upsert::Inserted);

        // Extending an existing interval does not need another slot.
        assert_eq!(
            upsert_integer_range(&t, -5, 9, "extended"),
            Upsert::Inserted
        );
        assert_eq!(t.len(), 2);

        // Covering both intervals replaces two entries with one at capacity.
        assert_eq!(
            upsert_integer_range(&t, -5, 29, "combined"),
            Upsert::Inserted
        );
        assert_eq!(t.len(), 1);
        let label = t.schema().index_of("label").unwrap();
        for point in [-5, 0, 15, 29] {
            let row = t
                .get(Lookup::Range {
                    partition: &[],
                    point: RangePoint::Integer(point),
                })
                .unwrap();
            assert_eq!(row.cell(label).as_str(), Some("combined"));
        }
        assert_eq!(upsert_integer_range(&t, 40, 49, "new"), Upsert::Inserted);
        assert_eq!(t.len(), 2);
        assert_eq!(t.rejected(), 0);
    }

    #[test]
    fn ip_range_splits_share_the_cap_across_address_families() {
        let t = table(
            &[
                ("start", ValueType::Ip),
                ("end", ValueType::Ip),
                ("label", ValueType::String),
            ],
            cfg::Key::Range(cfg::Range {
                range_type: RangeType::Ip,
                partition_columns: vec![],
                start_column: "start".into(),
                start_inclusive: true,
                end_column: "end".into(),
                end_inclusive: true,
            }),
            None,
            3,
        );
        let put = |lo: &str, hi: &str, inclusive, label| {
            t.upsert_range(
                &[CellRef::String(label)],
                RangePoint::Ip(lo.parse().unwrap()),
                inclusive,
                RangePoint::Ip(hi.parse().unwrap()),
                inclusive,
            )
            .unwrap()
        };
        assert_eq!(
            put("0.0.0.0", "255.255.255.255", true, "v4"),
            Upsert::Inserted
        );
        assert_eq!(
            put("::", "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", true, "v6"),
            Upsert::Inserted
        );
        // Exclusive bounds normalize to an interior interval, needing two slots.
        assert_eq!(
            put("0.0.0.0", "255.255.255.255", false, "inner-v4"),
            Upsert::Rejected
        );
        assert_eq!(
            put(
                "::",
                "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
                false,
                "inner-v6"
            ),
            Upsert::Rejected
        );
        assert_eq!(t.len(), 2);
        assert_eq!(t.rejected(), 2);
        for (point, expected) in [("127.0.0.1", "v4"), ("::1", "v6")] {
            let row = t
                .get(Lookup::Range {
                    partition: &[],
                    point: RangePoint::Ip(point.parse().unwrap()),
                })
                .unwrap();
            assert_eq!(
                row.cell(t.schema().index_of("label").unwrap()).as_str(),
                Some(expected)
            );
        }
    }

    #[test]
    fn concurrent_range_splits_never_exceed_the_cap() {
        const WRITERS: usize = 8;
        // Four splits fit, leaving one spare slot that cannot fit a fifth.
        const CAP: usize = WRITERS + 2 * 4 + 1;
        let t = table(
            &[
                ("partition", ValueType::Integer),
                ("start", ValueType::Integer),
                ("end", ValueType::Integer),
            ],
            cfg::Key::Range(cfg::Range {
                range_type: RangeType::Integer,
                partition_columns: vec!["partition".into()],
                start_column: "start".into(),
                start_inclusive: true,
                end_column: "end".into(),
                end_inclusive: true,
            }),
            None,
            CAP,
        );
        let put = |partition, lo, hi| {
            t.upsert_range(
                &[CellRef::Integer(partition)],
                RangePoint::Integer(lo),
                true,
                RangePoint::Integer(hi),
                true,
            )
            .unwrap()
        };
        for partition in 0..WRITERS {
            assert_eq!(put(partition as i64, 0, 10), Upsert::Inserted);
        }
        let barrier = std::sync::Barrier::new(WRITERS);
        std::thread::scope(|scope| {
            for partition in 0..WRITERS {
                let put = &put;
                let barrier = &barrier;
                let t = &t;
                scope.spawn(move || {
                    barrier.wait();
                    let _ = put(partition as i64, 3, 7);
                    assert!(t.len() <= CAP);
                });
            }
        });
        assert_eq!(t.len(), CAP - 1);
        assert_eq!(t.rejected(), 4);
        let stored: usize = t
            .shards
            .iter()
            .map(|shard| {
                shard
                    .read()
                    .ranges
                    .iter()
                    .map(|(_, store)| store.len())
                    .sum::<usize>()
            })
            .sum();
        assert_eq!(stored, CAP - 1);
    }

    #[test]
    fn exclusive_range_bounds_are_folded() {
        let t = range_table(10);
        let label = t.schema().index_of("label").unwrap();
        let _ = t
            .upsert_range(
                &[CellRef::String("a")],
                RangePoint::Integer(0),
                true,
                RangePoint::Integer(10),
                false,
            )
            .unwrap();
        let at = |p: i64| {
            t.get(Lookup::Range {
                partition: &[],
                point: RangePoint::Integer(p),
            })
            .map(|r| SmolStr::new(r.cell(label).as_str().unwrap()))
        };
        assert_eq!(at(9).as_deref(), Some("a"));
        assert_eq!(at(10), None, "end was exclusive");
    }

    #[test]
    fn the_wrong_upsert_method_is_an_error() {
        let hash = hash_table(10, None);
        assert!(
            hash.upsert_range(
                &[CellRef::String("a"), CellRef::String("b")],
                RangePoint::Integer(0),
                true,
                RangePoint::Integer(1),
                true
            )
            .is_err()
        );
        let range = range_table(10);
        assert!(range.upsert_hash(&[CellRef::String("a")]).is_err());
    }

    #[test]
    fn cells_are_type_checked_against_the_schema() {
        let t = hash_table(10, None);
        // wrong arity
        assert!(t.upsert_hash(&[CellRef::String("a")]).is_err());
        // wrong type
        assert!(
            t.upsert_hash(&[CellRef::String("a"), CellRef::Integer(1)])
                .is_err()
        );
        // null is accepted for any column
        assert!(
            t.upsert_hash(&[CellRef::String("a"), CellRef::Null])
                .is_ok()
        );
    }

    #[test]
    fn long_strings_are_pooled_and_swept_once_unreferenced() {
        let long = "a-really-quite-long-site-name-well-past-the-inline-limit";
        assert!(long.len() > INLINE_LEN);
        let t = hash_table(10, None);
        for i in 0..5 {
            let host = format!("host-{i}");
            let _ = t
                .upsert_hash(&[CellRef::String(&host), CellRef::String(long)])
                .unwrap();
        }
        // one pooled entry shared by every row in a shard
        let pooled = t.pooled_strings();
        assert!(pooled >= 1 && pooled <= t.shards(), "pooled={pooled}");

        t.clear();
        assert_eq!(t.pooled_strings(), 0, "clear drops the pools outright");

        // and the interval-driven sweep also releases them
        for i in 0..5 {
            let host = format!("host-{i}");
            let _ = t
                .upsert_hash(&[CellRef::String(&host), CellRef::String(long)])
                .unwrap();
        }
        assert!(t.pooled_strings() >= 1);
        for i in 0..5 {
            let host = format!("host-{i}");
            t.remove(Lookup::Hash(&[KeyRef::String(&host)]));
        }
        let released = t.compact(true);
        assert!(released >= 1, "released={released}");
        assert_eq!(t.pooled_strings(), 0);
    }

    #[test]
    fn short_strings_never_reach_the_pool() {
        let t = hash_table(10, None);
        let _ = t
            .upsert_hash(&[CellRef::String("a"), CellRef::String("short")])
            .unwrap();
        assert_eq!(t.pooled_strings(), 0);
    }

    #[test]
    fn compaction_is_rate_limited_unless_forced() {
        let t = hash_table(10, None);
        // the first sweep sets the clock; the second is inside the interval
        assert_eq!(t.compact(false), 0);
        assert_eq!(t.compact(false), 0);
        assert_eq!(t.compact(true), 0);
    }

    #[test]
    fn clearing_a_range_table_zeroes_the_row_count() {
        let t = range_table(10);
        for (lo, hi) in [(0, 9), (20, 29), (40, 49)] {
            assert_eq!(
                t.upsert_range(
                    &[CellRef::String("z")],
                    RangePoint::Integer(lo),
                    true,
                    RangePoint::Integer(hi),
                    true,
                )
                .unwrap(),
                Upsert::Inserted
            );
        }
        assert_eq!(t.len(), 3);

        // range entries are not hash rows, so counting only the latter left
        // `len` stuck at 3 here and the row gauge reporting a table that had
        // just been emptied
        t.clear();
        assert_eq!(t.len(), 0);
        assert!(t.is_empty());

        // and the freed capacity is really usable again
        assert_eq!(
            t.upsert_range(
                &[CellRef::String("z")],
                RangePoint::Integer(0),
                true,
                RangePoint::Integer(9),
                true,
            )
            .unwrap(),
            Upsert::Inserted
        );
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn long_hash_and_partition_keys_share_the_row_string_pool() {
        for range in [false, true] {
            let key = "a-long-key-that-must-be-allocated".repeat(100);
            let t = if range {
                table(
                    &[
                        ("start", ValueType::Integer),
                        ("end", ValueType::Integer),
                        ("host", ValueType::String),
                    ],
                    cfg::Key::Range(cfg::Range {
                        range_type: RangeType::Integer,
                        partition_columns: vec!["host".into()],
                        start_column: "start".into(),
                        end_column: "end".into(),
                        start_inclusive: true,
                        end_inclusive: true,
                    }),
                    None,
                    100,
                )
            } else {
                hash_table(100, None)
            };
            if range {
                let _ = t
                    .upsert_range(
                        &[CellRef::String(&key)],
                        RangePoint::Integer(0),
                        true,
                        RangePoint::Integer(10),
                        true,
                    )
                    .unwrap();
            } else {
                let _ = t
                    .upsert_hash(&[CellRef::String(&key), CellRef::String(&key)])
                    .unwrap();
            }
            let hash = hash_key([KeyRef::String(&key)]);
            {
                let shard = t.shard_of(hash).read();
                let stored_key = if range {
                    &shard.ranges.iter().next().unwrap().0[0]
                } else {
                    &shard.hash_rows.iter().next().unwrap().0[0]
                };
                let KeyOwned::String(stored_key) = stored_key else {
                    panic!("expected a string key")
                };
                let pooled = shard.strings.iter().next().unwrap();
                assert_eq!(
                    stored_key.as_ptr(),
                    pooled.as_ptr(),
                    "key must share the pool allocation"
                );
                assert_eq!(shard.strings.len(), 1);
            }
            let parts = [KeyRef::String(&key)];
            let probe = if range {
                Lookup::Range {
                    partition: &parts,
                    point: RangePoint::Integer(0),
                }
            } else {
                Lookup::Hash(&parts)
            };
            assert!(t.get(probe).is_some());
            assert!(t.remove(probe));
            assert_eq!(
                t.compact(true),
                1,
                "removing the row also releases the pooled key"
            );
        }
    }

    #[test]
    fn an_unattributed_write_handle_behaves_like_the_table() {
        let t = hash_table(10, None);
        let write = TableWrite::unattributed(&t);
        assert_eq!(
            write
                .upsert_hash(&[CellRef::String("a"), CellRef::String("jkt")])
                .unwrap(),
            Upsert::Inserted
        );
        // no processor to attribute to, so these are no-ops rather than panics
        write.note_key_error();
        assert!(write.remove(Lookup::Hash(&[KeyRef::String("a")])));
        assert_eq!(write.table().len(), 0);
    }

    #[test]
    fn probing_with_the_wrong_key_shape_is_a_miss() {
        let t = hash_table(10, None);
        let _ = t
            .upsert_hash(&[CellRef::String("a"), CellRef::String("1")])
            .unwrap();
        assert!(
            t.get(Lookup::Hash(&[KeyRef::String("a"), KeyRef::String("b")]))
                .is_none()
        );
        assert!(
            t.get(Lookup::Range {
                partition: &[],
                point: RangePoint::Integer(1)
            })
            .is_none()
        );
    }
}
