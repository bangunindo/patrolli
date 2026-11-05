// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::cell::{
    CellOwned, CellRef, Delete, KeyOwned, KeyRef, Lookup, hash_key, hash_key_owned, keys_eq,
};
use super::metrics::{ProcessorMetrics, WriteCounts};
use super::prefix::{Family, PrefixSet};
use super::schema::{ColumnIdx, KeyPlan, Schema};
use crate::config::lookup_table::ValueType;
use anyhow::{Result, bail};
use hashbrown::{HashSet, HashTable};
use ipnet::IpNet;
use parking_lot::RwLock;
use smallvec::SmallVec;
use smol_str::SmolStr;
use std::cell::Cell;
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

    /// Every cell in column order, the key columns included.
    pub fn cells(&self) -> impl ExactSizeIterator<Item = CellRef<'_>> {
        self.cells.iter().map(CellOwned::as_ref)
    }
}

/// Keyed by the owned key tuple. A `HashTable` rather than a `HashMap` so a
/// read can probe with borrowed `KeyRef`s and never allocate.
type HashEntry = (Box<[KeyOwned]>, Arc<MutableRow>);
/// One partition of a prefix table (the only one, under an empty key, when
/// the table is unpartitioned).
type PrefixEntry = (Box<[KeyOwned]>, PrefixSet<Arc<MutableRow>>);

#[derive(Debug, Default)]
struct Shard {
    hash_rows: HashTable<HashEntry>,
    prefixes: HashTable<PrefixEntry>,
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
            bail!("this table is prefix-keyed; use upsert_prefix");
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

    /// Inserts or updates one prefix in a prefix-keyed table. The partition is
    /// taken from the row; the prefix is supplied separately because it is
    /// not stored as a column. Each stored prefix is one entry against
    /// `max_rows`, however many others it nests in or covers.
    pub fn upsert_prefix(&self, cells: &[CellRef<'_>], net: IpNet) -> Result<Upsert> {
        let KeyPlan::Prefix { partition } = &self.key else {
            bail!("this table is hash-keyed; use upsert_hash");
        };
        self.check_cells(cells)?;
        let net = super::prefix::canonical(net);
        let parts = self.key_parts(cells, partition)?;
        let hash = hash_key(parts.iter().copied());
        let now = now_millis();

        // the lock is held across the capacity check and the write, so a
        // concurrent writer cannot slip a prefix in between the two
        let mut shard = self.shard_of(hash).write();
        let existing = shard
            .prefixes
            .find(hash, |(k, _)| keys_eq(k, &parts))
            .is_some_and(|(_, set)| set.contains(&net));
        if !existing && !self.reserve(1) {
            self.rejected.fetch_add(1, Ordering::Relaxed);
            return Ok(Upsert::Rejected);
        }

        let owned: Vec<CellOwned> = cells.iter().map(|c| shard.own_cell(*c)).collect();
        let row = Arc::new(MutableRow {
            cells: owned.into_boxed_slice(),
            last_seen: AtomicU64::new(now),
        });
        if shard
            .prefixes
            .find(hash, |(k, _)| keys_eq(k, &parts))
            .is_none()
        {
            let key: Box<[KeyOwned]> = parts.iter().map(|k| shard.own_key(*k)).collect();
            shard
                .prefixes
                .insert_unique(hash, (key, PrefixSet::new()), |(k, _)| hash_key_owned(k));
        }
        let set = &mut shard
            .prefixes
            .find_mut(hash, |(k, _)| keys_eq(k, &parts))
            .expect("just inserted")
            .1;
        // rows are shared behind an `Arc`, so an update swaps in a new one
        // rather than mutating what readers may already hold
        let replaced = set.insert(net, row);
        debug_assert_eq!(replaced.is_some(), existing);
        Ok(if existing {
            Upsert::Updated
        } else {
            Upsert::Inserted
        })
    }

    /// The row `key` names, and for a prefix table the prefix that matched.
    pub fn get(&self, key: Lookup<'_>) -> Option<(Arc<MutableRow>, Option<IpNet>)> {
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
                    .map(|(_, row)| (Arc::clone(row), None))
            }
            (KeyPlan::Prefix { partition }, Lookup::Prefix { partition: p, addr }) => {
                if p.len() != partition.len() {
                    return None;
                }
                let hash = hash_key(p.iter().copied());
                let shard = self.shard_of(hash).read();
                shard
                    .prefixes
                    .find(hash, |(k, _)| keys_eq(k, p))
                    .and_then(|(_, set)| set.lpm(addr))
                    .map(|(net, row)| (Arc::clone(row), Some(net)))
            }
            _ => None,
        }
    }

    /// The row stored under exactly `net` in one partition of a prefix table;
    /// unlike [`Self::get`], a covering prefix does not answer.
    pub fn get_prefix(&self, partition: &[KeyRef<'_>], net: IpNet) -> Option<Arc<MutableRow>> {
        let KeyPlan::Prefix { partition: cols } = &self.key else {
            return None;
        };
        if partition.len() != cols.len() {
            return None;
        }
        let net = super::prefix::canonical(net);
        let hash = hash_key(partition.iter().copied());
        let shard = self.shard_of(hash).read();
        shard
            .prefixes
            .find(hash, |(k, _)| keys_eq(k, partition))
            .and_then(|(_, set)| set.get(&net))
            .map(Arc::clone)
    }

    /// Drops the row stored under exactly `key`: a hash key, or a prefix in
    /// one partition. A wider or narrower prefix is never touched, so removing
    /// a /24 exposes the /16 it was nested in.
    pub fn remove(&self, key: Delete<'_>) -> bool {
        match (&self.key, key) {
            (KeyPlan::Hash { columns }, Delete::Hash(parts)) => {
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
            (KeyPlan::Prefix { partition }, Delete::Prefix { partition: p, net }) => {
                if p.len() != partition.len() {
                    return false;
                }
                let net = super::prefix::canonical(net);
                let hash = hash_key(p.iter().copied());
                let mut shard = self.shard_of(hash).write();
                let Ok(mut entry) = shard.prefixes.find_entry(hash, |(k, _)| keys_eq(k, p)) else {
                    return false;
                };
                let removed = entry.get_mut().1.remove(&net).is_some();
                // An empty partition must release its owned key too, or the
                // string pool cannot reclaim it on the next sweep.
                if entry.get().1.is_empty() {
                    let _ = entry.remove();
                }
                if removed {
                    self.adjust_len(-1);
                }
                removed
            }
            _ => false,
        }
    }

    /// Drops every row of one partition of a prefix table (a BGP peer going
    /// down), returning how many went. Unlinking is O(1); the partition's
    /// trie is freed as it drops. Zero for a missing partition, a hash table,
    /// or a partition tuple of the wrong arity.
    pub fn remove_partition(&self, partition: &[KeyRef<'_>]) -> usize {
        let KeyPlan::Prefix { partition: cols } = &self.key else {
            return 0;
        };
        if partition.len() != cols.len() {
            return 0;
        }
        let hash = hash_key(partition.iter().copied());
        let mut shard = self.shard_of(hash).write();
        let Ok(entry) = shard
            .prefixes
            .find_entry(hash, |(k, _)| keys_eq(k, partition))
        else {
            return 0;
        };
        // removing the entry frees its owned key, so the pool can reclaim it
        let ((_, set), _) = entry.remove();
        let removed = set.len();
        self.adjust_len(-(removed as i64));
        drop(shard);
        drop(set);
        removed
    }

    /// Drops the rows of one partition last written before `cutoff` (in
    /// [`now_millis`] time), returning how many went. With a `family`, only
    /// that family's prefixes are considered; `cutoff = u64::MAX` drops all of
    /// them.
    ///
    /// The resync sweep of a BGP session: a re-sent route is a fresh row
    /// stamped by its upsert, so after End-of-RIB whatever still predates the
    /// session's start was not re-announced. End-of-RIB comes per address
    /// family, hence the filter. An emptied partition is unlinked, as in
    /// [`Self::remove`]. Zero for a missing partition, a hash table, or a
    /// partition tuple of the wrong arity.
    pub fn sweep_partition(
        &self,
        partition: &[KeyRef<'_>],
        family: Option<Family>,
        cutoff: u64,
    ) -> usize {
        let KeyPlan::Prefix { partition: cols } = &self.key else {
            return 0;
        };
        if partition.len() != cols.len() {
            return 0;
        }
        let hash = hash_key(partition.iter().copied());
        let mut shard = self.shard_of(hash).write();
        let Ok(mut entry) = shard
            .prefixes
            .find_entry(hash, |(k, _)| keys_eq(k, partition))
        else {
            return 0;
        };
        // Collected rather than dropped in place, so the rows are freed after
        // the lock is released: a full table can stale out a million routes.
        let mut stale = Vec::new();
        entry.get_mut().1.retain(|net, row| {
            let keep = family.is_some_and(|f| f != Family::of(&net)) || row.last_seen() >= cutoff;
            if !keep {
                stale.push(Arc::clone(row));
            }
            keep
        });
        let emptied = entry.get().1.is_empty().then(|| entry.remove().0);
        let removed = stale.len();
        self.adjust_len(-(removed as i64));
        drop(shard);
        drop(stale);
        drop(emptied);
        removed
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
            for (_, set) in shard.prefixes.iter_mut() {
                let before = set.len();
                set.retain(|_, row| row.last_seen() >= cutoff);
                shard_dropped += (before - set.len()) as i64;
            }
            // as in `remove`: an emptied partition releases its owned key
            shard.prefixes.retain(|(_, set)| !set.is_empty());
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
            shard.prefixes.clear();
            shard.strings.clear();
        }
        // hash rows and prefix entries alike, so zero rather than a decrement
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
    /// rows removed, so a partition delete adds every row it dropped
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

    pub fn upsert_prefix(&self, cells: &[CellRef<'_>], net: IpNet) -> Result<Upsert> {
        self.record(self.table.upsert_prefix(cells, net))
    }

    /// Removes exactly `key`; see [`Mutable::remove`].
    pub fn remove(&self, key: Delete<'_>) -> bool {
        let removed = self.table.remove(key);
        if removed && let Some(tally) = &self.tally {
            tally.record(&tally.removes);
        }
        removed
    }

    /// Removes a whole partition; see [`Mutable::remove_partition`].
    pub fn remove_partition(&self, partition: &[KeyRef<'_>]) -> usize {
        let removed = self.table.remove_partition(partition);
        if let Some(tally) = &self.tally {
            tally.removes.set(tally.removes.get() + removed as u64);
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
        for prefix in [false, true] {
            let t = if prefix {
                table(
                    &[
                        ("host", ValueType::String),
                        ("network", ValueType::String),
                        ("site", ValueType::String),
                    ],
                    prefix_key(&["host"]),
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
                            if prefix {
                                // nested prefixes, so a partition holds several
                                let n = net(["10.0.0.0/8", "10.1.0.0/16"][i % 2]);
                                let _ = t.upsert_prefix(&cells, n).unwrap();
                            } else {
                                let _ = t.upsert_hash(&cells).unwrap();
                            }
                            let parts = [KeyRef::String(&host)];
                            if prefix && i % 7 == 0 {
                                t.remove_partition(&parts);
                            } else if i % 3 == 0 {
                                let key = if prefix {
                                    Delete::Prefix {
                                        partition: &parts,
                                        net: net("10.1.0.0/16"),
                                    }
                                } else {
                                    Delete::Hash(&parts)
                                };
                                t.remove(key);
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
                            .prefixes
                            .iter()
                            .map(|(_, set)| set.len())
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
        let (snapshot, _) = t.get(Lookup::Hash(&[KeyRef::String(&host)])).unwrap();
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

    fn prefix_key(partition: &[&str]) -> cfg::Key {
        cfg::Key::Prefix(cfg::Prefix {
            prefix_column: "network".into(),
            partition_columns: partition.iter().map(|s| s.to_string()).collect(),
        })
    }

    fn prefix_table(max_rows: usize, ttl: Option<Duration>) -> Mutable {
        table(
            &[("network", ValueType::String), ("label", ValueType::String)],
            prefix_key(&[]),
            ttl,
            max_rows,
        )
    }

    /// Routes per BGP peer: partitioned by `peer`.
    fn peer_table(max_rows: usize) -> Mutable {
        table(
            &[
                ("peer", ValueType::String),
                ("network", ValueType::String),
                ("label", ValueType::String),
            ],
            prefix_key(&["peer"]),
            None,
            max_rows,
        )
    }

    fn net(s: &str) -> IpNet {
        super::super::prefix::parse_prefix(s).unwrap()
    }

    fn put(t: &Mutable, cidr: &str, label: &str) -> Upsert {
        t.upsert_prefix(&[CellRef::String(label)], net(cidr))
            .unwrap()
    }

    fn put_peer(t: &Mutable, peer: &str, cidr: &str, label: &str) -> Upsert {
        t.upsert_prefix(&[CellRef::String(peer), CellRef::String(label)], net(cidr))
            .unwrap()
    }

    /// The label and matched prefix for `addr`.
    fn at(t: &Mutable, partition: &[KeyRef<'_>], addr: &str) -> Option<(SmolStr, IpNet)> {
        let label = t.schema().index_of("label").unwrap();
        t.get(Lookup::Prefix {
            partition,
            addr: addr.parse().unwrap(),
        })
        .map(|(r, n)| {
            (
                SmolStr::new(r.cell(label).as_str().unwrap()),
                n.expect("a prefix match"),
            )
        })
    }

    fn at_label(t: &Mutable, addr: &str) -> Option<SmolStr> {
        at(t, &[], addr).map(|(l, _)| l)
    }

    fn stored_prefixes(t: &Mutable) -> usize {
        t.shards
            .iter()
            .map(|shard| {
                shard
                    .read()
                    .prefixes
                    .iter()
                    .map(|(_, set)| set.len())
                    .sum::<usize>()
            })
            .sum()
    }

    fn row(t: &Mutable, host: &str) -> Option<SmolStr> {
        let site = t.schema().index_of("site").unwrap();
        t.get(Lookup::Hash(&[KeyRef::String(host)])).map(|(r, n)| {
            assert_eq!(n, None, "a hash match has no prefix");
            SmolStr::new(r.cell(site).as_str().unwrap())
        })
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
        let (held, _) = t.get(Lookup::Hash(&[KeyRef::String("a")])).unwrap();
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
        assert!(t.remove(Delete::Hash(&[KeyRef::String("a")])));
        assert_eq!(t.len(), 0);
        assert!(
            !t.remove(Delete::Hash(&[KeyRef::String("a")])),
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
        t.remove(Delete::Hash(&[KeyRef::String("a")]));
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
    fn prefix_upsert_and_longest_match() {
        let t = prefix_table(10, None);
        assert_eq!(put(&t, "10.0.0.0/8", "wide"), Upsert::Inserted);
        assert_eq!(
            at(&t, &[], "10.1.2.3"),
            Some(("wide".into(), net("10.0.0.0/8")))
        );
        assert_eq!(at(&t, &[], "11.0.0.0"), None);

        // the same prefix again is an update, not a second row
        assert_eq!(put(&t, "10.0.0.0/8", "still-wide"), Upsert::Updated);
        assert_eq!(t.len(), 1);
        assert_eq!(at_label(&t, "10.1.2.3").as_deref(), Some("still-wide"));
        // a host-bits-free v4-mapped spelling is the same prefix
        assert_eq!(put(&t, "::ffff:10.0.0.0/104", "mapped"), Upsert::Updated);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn nested_prefixes_resolve_to_the_narrowest_in_either_insert_order() {
        let order = ["10.0.0.0/8", "10.1.0.0/16", "10.1.2.0/24"];
        for reversed in [false, true] {
            let t = prefix_table(10, None);
            let mut cidrs = order.to_vec();
            if reversed {
                cidrs.reverse();
            }
            for cidr in cidrs {
                assert_eq!(put(&t, cidr, cidr), Upsert::Inserted);
            }
            assert_eq!(t.len(), 3, "nesting neither splits nor merges");
            assert_eq!(at_label(&t, "10.1.2.3").as_deref(), Some("10.1.2.0/24"));
            assert_eq!(at_label(&t, "10.1.3.3").as_deref(), Some("10.1.0.0/16"));
            assert_eq!(at_label(&t, "10.2.0.0").as_deref(), Some("10.0.0.0/8"));
        }
    }

    #[test]
    fn an_exact_delete_exposes_the_parent_and_touches_nothing_else() {
        let t = prefix_table(10, None);
        let _ = put(&t, "10.0.0.0/8", "a");
        let _ = put(&t, "10.1.0.0/16", "b");
        let _ = put(&t, "10.1.2.0/24", "c");
        let delete = |cidr| {
            t.remove(Delete::Prefix {
                partition: &[],
                net: net(cidr),
            })
        };
        // an address inside a stored prefix is not that prefix
        assert!(!delete("10.1.2.3"));
        assert!(!delete("10.1.0.0/17"));
        assert_eq!(t.len(), 3);

        assert!(delete("10.1.0.0/16"));
        assert_eq!(t.len(), 2);
        assert_eq!(
            at_label(&t, "10.1.9.9").as_deref(),
            Some("a"),
            "parent exposed"
        );
        assert_eq!(at_label(&t, "10.1.2.9").as_deref(), Some("c"), "child kept");
        assert!(!delete("10.1.0.0/16"), "already gone");
    }

    #[test]
    fn max_rows_counts_one_entry_per_prefix() {
        let t = prefix_table(2, None);
        assert_eq!(put(&t, "10.0.0.0/8", "x"), Upsert::Inserted);
        assert_eq!(put(&t, "10.1.0.0/16", "x"), Upsert::Inserted);
        assert_eq!(
            put(&t, "10.1.2.0/24", "x"),
            Upsert::Rejected,
            "one prefix is one row against the cap"
        );
        assert_eq!(t.rejected(), 1);
        // an existing prefix still updates at the cap
        assert_eq!(put(&t, "10.1.0.0/16", "y"), Upsert::Updated);
        assert_eq!(t.len(), 2);
        assert_eq!(stored_prefixes(&t), 2);

        assert!(t.remove(Delete::Prefix {
            partition: &[],
            net: net("10.0.0.0/8"),
        }));
        assert_eq!(put(&t, "10.1.2.0/24", "x"), Upsert::Inserted);
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn families_share_the_cap() {
        let t = prefix_table(2, None);
        assert_eq!(put(&t, "0.0.0.0/0", "v4"), Upsert::Inserted);
        assert_eq!(put(&t, "::/0", "v6"), Upsert::Inserted);
        assert_eq!(put(&t, "2001:db8::/32", "v6"), Upsert::Rejected);
        assert_eq!(at_label(&t, "192.0.2.1").as_deref(), Some("v4"));
        assert_eq!(at_label(&t, "2001:db8::1").as_deref(), Some("v6"));
    }

    #[test]
    fn without_a_ttl_prefixes_never_expire_but_the_cap_still_holds() {
        // `ttl: 0` in the config arrives as `None`
        for ttl in [None, Some(Duration::ZERO)] {
            let t = prefix_table(2, ttl);
            let _ = put(&t, "10.0.0.0/8", "a");
            let _ = put(&t, "10.1.0.0/16", "b");
            assert_eq!(t.expire(now_millis() + 1_000_000_000), 0, "{ttl:?}");
            assert_eq!(t.len(), 2);
            assert_eq!(put(&t, "10.2.0.0/16", "c"), Upsert::Rejected, "{ttl:?}");
            assert_eq!(t.len(), 2);
        }
    }

    #[test]
    fn expiry_drops_stale_prefixes_and_empty_partitions() {
        let t = peer_table(10);
        t.reconfigure(Some(Duration::from_millis(50)), 10);
        let _ = put_peer(&t, "peer-a", "10.0.0.0/8", "a");
        let _ = put_peer(&t, "peer-a", "10.1.0.0/16", "b");
        let _ = put_peer(&t, "peer-b", "10.0.0.0/8", "c");
        let now = now_millis();
        assert_eq!(t.expire(now), 0);
        assert_eq!(t.expire(now + 51), 3);
        assert_eq!(t.len(), 0);
        let partitions: usize = t.shards.iter().map(|s| s.read().prefixes.len()).sum();
        assert_eq!(partitions, 0, "emptied partitions release their keys");
    }

    #[test]
    fn removing_a_partition_drops_only_that_partition() {
        let t = peer_table(5);
        let a = [KeyRef::String("peer-a")];
        let b = [KeyRef::String("peer-b")];
        for cidr in ["10.0.0.0/8", "10.1.0.0/16", "192.0.2.0/24"] {
            assert_eq!(put_peer(&t, "peer-a", cidr, "a"), Upsert::Inserted);
        }
        assert_eq!(put_peer(&t, "peer-b", "10.0.0.0/8", "b"), Upsert::Inserted);
        assert_eq!(put_peer(&t, "peer-b", "10.1.0.0/16", "b"), Upsert::Inserted);
        assert_eq!(t.len(), 5);
        assert_eq!(put_peer(&t, "peer-b", "10.2.0.0/16", "b"), Upsert::Rejected);

        assert_eq!(t.remove_partition(&a), 3);
        assert_eq!(t.len(), 2);
        assert_eq!(at(&t, &a, "10.1.2.3"), None);
        assert_eq!(
            at(&t, &b, "10.1.2.3"),
            Some(("b".into(), net("10.1.0.0/16")))
        );
        // the freed capacity is usable again
        assert_eq!(put_peer(&t, "peer-b", "10.2.0.0/16", "b"), Upsert::Inserted);
        assert_eq!(t.len(), 3);
        assert_eq!(stored_prefixes(&t), 3);

        // a missing partition, or one of the wrong arity, removes nothing
        assert_eq!(t.remove_partition(&a), 0);
        assert_eq!(t.remove_partition(&[KeyRef::String("peer-z")]), 0);
        assert_eq!(t.remove_partition(&[]), 0);
        assert_eq!(t.len(), 3);
        // and a hash table has no partitions to remove
        assert_eq!(hash_table(10, None).remove_partition(&a), 0);
    }

    #[test]
    fn a_partition_sweep_drops_only_its_own_stale_rows() {
        let t = peer_table(10);
        let a = [KeyRef::String("peer-a")];
        let b = [KeyRef::String("peer-b")];
        let _ = put_peer(&t, "peer-a", "10.0.0.0/8", "old");
        let _ = put_peer(&t, "peer-a", "10.1.0.0/16", "old");
        let _ = put_peer(&t, "peer-b", "10.0.0.0/8", "old");
        // the session restarts: everything written so far predates it
        std::thread::sleep(Duration::from_millis(2));
        let cutoff = now_millis();
        // and re-announces one route
        let _ = put_peer(&t, "peer-a", "10.1.0.0/16", "new");

        assert_eq!(t.sweep_partition(&a, None, cutoff), 1);
        assert_eq!(t.len(), 2);
        assert_eq!(at(&t, &a, "10.0.0.1"), None);
        assert_eq!(
            at(&t, &a, "10.1.0.1"),
            Some(("new".into(), net("10.1.0.0/16")))
        );
        assert_eq!(
            at(&t, &b, "10.0.0.1"),
            Some(("old".into(), net("10.0.0.0/8"))),
            "another partition is untouched"
        );
        assert_eq!(
            t.sweep_partition(&a, None, cutoff),
            0,
            "nothing left to sweep"
        );

        // a partition swept empty is unlinked, releasing its key
        assert_eq!(t.sweep_partition(&b, None, cutoff), 1);
        assert_eq!(t.len(), 1);
        let partitions: usize = t.shards.iter().map(|s| s.read().prefixes.len()).sum();
        assert_eq!(partitions, 1);

        // a missing partition, a wrong arity, or a hash table sweeps nothing
        assert_eq!(
            t.sweep_partition(&[KeyRef::String("peer-z")], None, u64::MAX),
            0
        );
        assert_eq!(t.sweep_partition(&[], None, u64::MAX), 0);
        assert_eq!(hash_table(10, None).sweep_partition(&a, None, u64::MAX), 0);
        assert_eq!(t.len(), 1);
        assert_eq!(stored_prefixes(&t), 1);
    }

    #[test]
    fn a_family_sweep_leaves_the_other_family_alone() {
        let t = peer_table(10);
        let a = [KeyRef::String("peer-a")];
        let _ = put_peer(&t, "peer-a", "10.0.0.0/8", "v4");
        let _ = put_peer(&t, "peer-a", "2001:db8::/32", "v6");
        let _ = put_peer(&t, "peer-a", "2001:db8:1::/48", "v6");

        assert_eq!(t.sweep_partition(&a, Some(Family::V6), u64::MAX), 2);
        assert_eq!(t.len(), 1);
        assert_eq!(
            at(&t, &a, "10.0.0.1"),
            Some(("v4".into(), net("10.0.0.0/8")))
        );
        assert_eq!(at(&t, &a, "2001:db8::1"), None);
        assert_eq!(t.sweep_partition(&a, Some(Family::V6), u64::MAX), 0);
        assert_eq!(t.sweep_partition(&a, Some(Family::V4), u64::MAX), 1);
        assert!(t.is_empty());
    }

    #[test]
    fn an_exact_prefix_read_ignores_covering_and_nested_prefixes() {
        let t = peer_table(10);
        let a = [KeyRef::String("peer-a")];
        let _ = put_peer(&t, "peer-a", "10.0.0.0/8", "wide");
        let _ = put_peer(&t, "peer-a", "10.1.2.0/24", "narrow");
        let value = |row: Arc<MutableRow>| row.cells().nth(1).unwrap().to_owned_cell();

        assert_eq!(
            t.get_prefix(&a, net("10.0.0.0/8")).map(value),
            Some(CellOwned::String("wide".into()))
        );
        assert!(t.get_prefix(&a, net("10.1.0.0/16")).is_none());
        assert!(t.get_prefix(&a, net("10.1.2.0/25")).is_none());
        assert!(
            t.get_prefix(&[KeyRef::String("peer-b")], net("10.0.0.0/8"))
                .is_none()
        );
        assert!(t.get_prefix(&[], net("10.0.0.0/8")).is_none());
    }

    #[test]
    fn a_removed_partition_lets_its_key_strings_be_reclaimed() {
        let t = peer_table(10);
        let peer = "a-long-peer-name-that-must-be-pooled".repeat(4);
        let label = "a-long-label-value-that-must-be-pooled".repeat(4);
        for cidr in ["10.0.0.0/8", "10.1.0.0/16"] {
            let _ = t
                .upsert_prefix(
                    &[CellRef::String(&peer), CellRef::String(&label)],
                    net(cidr),
                )
                .unwrap();
        }
        assert_eq!(t.pooled_strings(), 2);
        assert_eq!(t.compact(true), 0, "everything is still referenced");
        assert_eq!(t.remove_partition(&[KeyRef::String(&peer)]), 2);
        assert_eq!(
            t.compact(true),
            2,
            "the key and the row strings are released"
        );
        assert_eq!(t.pooled_strings(), 0);
    }

    #[test]
    fn concurrent_prefix_writers_never_exceed_the_cap() {
        const WRITERS: usize = 8;
        const CAP: usize = 20;
        let t = peer_table(CAP);
        std::thread::scope(|scope| {
            for w in 0..WRITERS {
                let t = &t;
                scope.spawn(move || {
                    let peer = format!("peer-{}", w % 3);
                    for i in 0..64u32 {
                        let cidr = format!("10.{}.0.0/16", (w as u32 * 64 + i) % 256);
                        let _ = put_peer(t, &peer, &cidr, "x");
                        assert!(t.len() <= CAP);
                    }
                });
            }
        });
        assert_eq!(t.len(), CAP);
        assert_eq!(stored_prefixes(&t), CAP);
    }

    #[test]
    fn the_wrong_upsert_method_is_an_error() {
        let hash = hash_table(10, None);
        assert!(
            hash.upsert_prefix(
                &[CellRef::String("a"), CellRef::String("b")],
                net("10.0.0.0/8")
            )
            .is_err()
        );
        let prefix = prefix_table(10, None);
        assert!(prefix.upsert_hash(&[CellRef::String("a")]).is_err());
        // and a delete of the wrong shape removes nothing
        let _ = put(&prefix, "10.0.0.0/8", "a");
        assert!(!prefix.remove(Delete::Hash(&[KeyRef::String("a")])));
        assert_eq!(prefix.len(), 1);
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
            t.remove(Delete::Hash(&[KeyRef::String(&host)]));
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
    fn clearing_a_prefix_table_zeroes_the_row_count() {
        let t = prefix_table(10, None);
        for cidr in ["10.0.0.0/8", "10.1.0.0/16", "2001:db8::/32"] {
            assert_eq!(put(&t, cidr, "z"), Upsert::Inserted);
        }
        assert_eq!(t.len(), 3);

        // prefix entries are not hash rows, so counting only the latter would
        // leave `len` stuck at 3 and the row gauge reporting a table that had
        // just been emptied
        t.clear();
        assert_eq!(t.len(), 0);
        assert!(t.is_empty());

        // and the freed capacity is really usable again
        assert_eq!(put(&t, "10.0.0.0/8", "z"), Upsert::Inserted);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn long_hash_and_partition_keys_share_the_row_string_pool() {
        for prefix in [false, true] {
            let key = "a-long-key-that-must-be-allocated".repeat(100);
            let t = if prefix {
                table(
                    &[("network", ValueType::String), ("host", ValueType::String)],
                    prefix_key(&["host"]),
                    None,
                    100,
                )
            } else {
                hash_table(100, None)
            };
            if prefix {
                let _ = t
                    .upsert_prefix(&[CellRef::String(&key)], net("10.0.0.0/8"))
                    .unwrap();
            } else {
                let _ = t
                    .upsert_hash(&[CellRef::String(&key), CellRef::String(&key)])
                    .unwrap();
            }
            let hash = hash_key([KeyRef::String(&key)]);
            {
                let shard = t.shard_of(hash).read();
                let stored_key = if prefix {
                    &shard.prefixes.iter().next().unwrap().0[0]
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
            let (probe, delete) = if prefix {
                (
                    Lookup::Prefix {
                        partition: &parts,
                        addr: "10.0.0.1".parse().unwrap(),
                    },
                    Delete::Prefix {
                        partition: &parts,
                        net: net("10.0.0.0/8"),
                    },
                )
            } else {
                (Lookup::Hash(&parts), Delete::Hash(&parts))
            };
            assert!(t.get(probe).is_some());
            assert!(t.remove(delete));
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
        assert!(write.remove(Delete::Hash(&[KeyRef::String("a")])));
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
            t.get(Lookup::Prefix {
                partition: &[],
                addr: "10.0.0.1".parse().unwrap(),
            })
            .is_none()
        );
    }
}
