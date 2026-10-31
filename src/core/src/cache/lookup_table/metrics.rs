// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Live counters for the lookup tables, and the task that publishes them.
//!
//! They live here rather than in [`crate::common::internal_metrics`] because
//! [`TableMetrics`] hangs off [`Meta`]: a table's counters are created and
//! dropped with it, so they survive a reconfiguration exactly when the loaded
//! data does and leave no dead entity id behind.
//!
//! Per-processor counters are handed out as an `Arc` shared by every worker
//! thread, as the forwarder's per-peer counters are. The read and write guards
//! in the parent module accumulate into non-atomic cells and touch these once
//! per batch, so the per-record path contends on no shared cache line.

use super::immutable::{LoadStats, SkipCounts};
use crate::cache::Cache;
use crate::common::internal_metrics::{
    INTERNAL_METRICS, LookupTable as LookupTableMetrics, LookupTableProcessor as ProcessorSnapshot,
};
use rustc_hash::{FxBuildHasher, FxHashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

type PHashMap<K, V> = papaya::HashMap<K, V, FxBuildHasher>;

/// Counters for one processor's use of one table, shared by every worker thread
/// and registered in the table's `processors` map for snapshots.
#[derive(Debug, Default)]
pub struct ProcessorMetrics {
    lookups_hit: AtomicU64,
    lookups_missed: AtomicU64,
    lookups_errored: AtomicU64,
    writes_inserted: AtomicU64,
    writes_updated: AtomicU64,
    writes_rejected: AtomicU64,
    writes_errored: AtomicU64,
    removes: AtomicU64,
}

impl ProcessorMetrics {
    /// The snapshot, with the processor's id stamped in — the map key is the
    /// only place the id is stored.
    pub fn get(&self, id: Uuid) -> ProcessorSnapshot {
        let lookups_hit = self.lookups_hit.load(Ordering::Relaxed);
        let lookups_missed = self.lookups_missed.load(Ordering::Relaxed);
        let lookups_errored = self.lookups_errored.load(Ordering::Relaxed);
        let writes_inserted = self.writes_inserted.load(Ordering::Relaxed);
        let writes_updated = self.writes_updated.load(Ordering::Relaxed);
        let writes_rejected = self.writes_rejected.load(Ordering::Relaxed);
        let writes_errored = self.writes_errored.load(Ordering::Relaxed);
        // A batch can flush while these loads run. Deriving totals from the
        // values actually sampled preserves the outcome sums in every snapshot.
        ProcessorSnapshot {
            id,
            lookups: lookups_hit + lookups_missed + lookups_errored,
            lookups_hit,
            lookups_missed,
            lookups_errored,
            writes: writes_inserted + writes_updated + writes_rejected + writes_errored,
            writes_inserted,
            writes_updated,
            writes_rejected,
            writes_errored,
            removes: self.removes.load(Ordering::Relaxed),
        }
    }

    /// Folds a read guard's batch into the shared counters. `lookups` is the
    /// sum of the three outcomes, so callers never have to keep it themselves.
    pub(super) fn add_reads(&self, counts: &ReadCounts) {
        add(&self.lookups_hit, counts.hit);
        add(&self.lookups_missed, counts.missed);
        add(&self.lookups_errored, counts.errored);
    }

    /// Folds a write guard's batch into the shared counters. `writes` counts
    /// the upserts, so a `remove` does not inflate it.
    pub(super) fn add_writes(&self, counts: &WriteCounts) {
        add(&self.writes_inserted, counts.inserted);
        add(&self.writes_updated, counts.updated);
        add(&self.writes_rejected, counts.rejected);
        add(&self.writes_errored, counts.errored);
        add(&self.removes, counts.removes);
    }
}

/// One read guard's tally, flushed by [`ProcessorMetrics::add_reads`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct ReadCounts {
    pub hit: u64,
    pub missed: u64,
    pub errored: u64,
}

/// One write guard's tally, flushed by [`ProcessorMetrics::add_writes`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct WriteCounts {
    pub inserted: u64,
    pub updated: u64,
    pub rejected: u64,
    pub errored: u64,
    pub removes: u64,
}

/// Per-reason skip totals across every load, mirroring [`SkipCounts`] field for
/// field so the two cannot drift.
#[derive(Debug, Default)]
struct SkipAtomics {
    arity: AtomicU64,
    bad_cell: AtomicU64,
    bad_bound: AtomicU64,
    null_key: AtomicU64,
    duplicate_key: AtomicU64,
    overlap: AtomicU64,
    empty_range: AtomicU64,
    mismatch: AtomicU64,
}

impl SkipAtomics {
    fn add(&self, counts: &SkipCounts) {
        add(&self.arity, counts.arity);
        add(&self.bad_cell, counts.bad_cell);
        add(&self.bad_bound, counts.bad_bound);
        add(&self.null_key, counts.null_key);
        add(&self.duplicate_key, counts.duplicate_key);
        add(&self.overlap, counts.overlap);
        add(&self.empty_range, counts.empty_range);
        add(&self.mismatch, counts.mismatch);
    }

    fn get(&self) -> SkipCounts {
        SkipCounts {
            arity: self.arity.load(Ordering::Relaxed),
            bad_cell: self.bad_cell.load(Ordering::Relaxed),
            bad_bound: self.bad_bound.load(Ordering::Relaxed),
            null_key: self.null_key.load(Ordering::Relaxed),
            duplicate_key: self.duplicate_key.load(Ordering::Relaxed),
            overlap: self.overlap.load(Ordering::Relaxed),
            empty_range: self.empty_range.load(Ordering::Relaxed),
            mismatch: self.mismatch.load(Ordering::Relaxed),
        }
    }
}

/// The cumulative counters of one table, sampled once.
///
/// Read through [`TableMetrics::totals`] so the metric snapshot and the status
/// report cannot drift, the same reason [`SkipCounts`] is shared rather than
/// copied. Plain values, not atomics: this is a sample, not the live state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub loads: u64,
    pub load_failures: u64,
    pub load_duration: Duration,
    pub rows_read: u64,
    pub rows_skipped: SkipCounts,
    /// rows dropped by the TTL sweep; mutable only
    pub rows_expired: u64,
    /// pool entries released by compaction; mutable only
    pub strings_released: u64,
}

/// Everything counted about one table, for the life of that table.
#[derive(Debug, Default)]
pub struct TableMetrics {
    loads: AtomicU64,
    load_failures: AtomicU64,
    load_nanos: AtomicU64,
    rows_read: AtomicU64,
    skipped: SkipAtomics,
    rows_expired: AtomicU64,
    strings_released: AtomicU64,
    processors: PHashMap<Uuid, Arc<ProcessorMetrics>>,
}

impl TableMetrics {
    /// The counter handle for one processor, created on first use.
    ///
    /// Acquire it once per batch through [`super::Table::read_as`] or
    /// [`super::MutableTable::write_as`] rather than per record.
    pub fn processor(&self, id: Uuid) -> Arc<ProcessorMetrics> {
        self.processors
            .pin()
            .get_or_insert_with(id, || Arc::new(ProcessorMetrics::default()))
            .clone()
    }

    /// Drops the counters of processor nodes that no longer exist.
    ///
    /// Every `POST /api/v1/processors` mints a fresh node id, so without this each
    /// delete-and-recreate strands ten more `patrolli_lookup_table_processor_*`
    /// series forever — twenty tuning cycles across three tables is 600.
    ///
    /// An empty `live` set means standby, not "nothing is live": that node has
    /// no tables either, so there is nothing to reconcile against.
    pub fn retain_processors(&self, live: &FxHashSet<Uuid>) {
        if live.is_empty() {
            return;
        }
        self.processors.pin().retain(|id, _| live.contains(id));
    }

    pub fn note_load(&self, stats: &LoadStats) {
        self.loads.fetch_add(1, Ordering::Relaxed);
        add(&self.load_nanos, stats.elapsed.as_nanos() as u64);
        add(&self.rows_read, stats.rows_read);
        self.skipped.add(&stats.skipped);
    }

    /// The rows read before failing are not counted: nothing was installed.
    pub fn note_load_failure(&self) {
        self.load_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_sweep(&self, expired: usize, released: usize) {
        add(&self.rows_expired, expired as u64);
        add(&self.strings_released, released as u64);
    }

    /// One entry per processor that has touched this table, sorted by id so the
    /// rendered label sets are stable between ticks.
    pub fn processor_snapshots(&self) -> Vec<ProcessorSnapshot> {
        let mut all: Vec<_> = self
            .processors
            .pin()
            .iter()
            .map(|(id, metrics)| metrics.get(*id))
            .collect();
        all.sort_unstable_by_key(|p| p.id);
        all
    }

    /// This table's cumulative counters, sampled once.
    ///
    /// Nanosecond precision is preserved in `load_duration`: the counter is kept
    /// in nanos and only the serialized form of a status report rounds to millis.
    pub fn totals(&self) -> Totals {
        Totals {
            loads: self.loads.load(Ordering::Relaxed),
            load_failures: self.load_failures.load(Ordering::Relaxed),
            load_duration: Duration::from_nanos(self.load_nanos.load(Ordering::Relaxed)),
            rows_read: self.rows_read.load(Ordering::Relaxed),
            rows_skipped: self.skipped.get(),
            rows_expired: self.rows_expired.load(Ordering::Relaxed),
            strings_released: self.strings_released.load(Ordering::Relaxed),
        }
    }

    /// Fills the cumulative half of a snapshot; the caller fills the gauges from
    /// [`super::TableStatus`].
    fn fill(&self, out: &mut LookupTableMetrics) {
        let totals = self.totals();
        out.loads = totals.loads;
        out.load_failures = totals.load_failures;
        out.load_duration = totals.load_duration;
        out.rows_read = totals.rows_read;
        out.rows_skipped = totals.rows_skipped;
        out.rows_expired = totals.rows_expired;
        out.strings_released = totals.strings_released;
        out.processors = self.processor_snapshots();
    }
}

/// A relaxed `fetch_add` that skips the atomic entirely for a zero delta, which
/// is the common case for most of these fields.
#[inline]
fn add(slot: &AtomicU64, delta: u64) {
    if delta > 0 {
        slot.fetch_add(delta, Ordering::Relaxed);
    }
}

/// One snapshot per registered table, sorted by id so label sets are stable.
///
/// Reconciles each table's per-processor counters against the processor cache
/// first: this runs on the metrics tick, which is exactly where a label set
/// that has stopped existing needs to stop being rendered.
pub fn snapshot(cache: &Cache) -> Vec<LookupTableMetrics> {
    let live: FxHashSet<Uuid> = cache.processor.read().node_ids().collect();
    let mut all: Vec<_> = cache
        .lookup_table
        .tables()
        .into_iter()
        .inspect(|table| table.meta().metrics.retain_processors(&live))
        .map(|table| {
            let status = table.status();
            let mut out = LookupTableMetrics {
                id: status.id,
                name: status.name.to_string(),
                kind: status.kind.into(),
                rows: status.rows as u64,
                index_entries: status.index_entries as u64,
                max_rows: status.max_rows as u64,
                pooled_strings: status.pooled_strings as u64,
                generation: status.generation,
                last_success_timestamp: status
                    .loaded_at
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map_or(0.0, |elapsed| elapsed.as_secs_f64()),
                load_error: status.last_error.is_some(),
                generation_rows_read: status.generation_stats.rows_read,
                generation_rows_skipped: status.generation_stats.skipped,
                generation_load_duration: status.generation_stats.elapsed,
                rows_rejected: status.rejected,
                ..Default::default()
            };
            table.meta().metrics.fill(&mut out);
            out
        })
        .collect();
    all.sort_unstable_by_key(|t| t.id);
    all
}

/// Publishes the lookup-table snapshots until `shutdown` fires.
pub fn spawn_metrics_reporter(
    cache: Cache,
    interval: Duration,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = tokio::time::sleep(interval) => {
                    let metrics = snapshot(&cache);
                    INTERNAL_METRICS
                        .lookup_table
                        .store(Arc::new((Instant::now(), metrics)));
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Counters are created on first use and keyed by processor node id, and
    /// every recreated node mints a new one — so without reconciliation each
    /// tuning cycle leaves a frozen set of series behind forever.
    #[test]
    fn counters_of_deleted_processor_nodes_are_dropped() {
        let m = TableMetrics::default();
        let (alive, dead) = (Uuid::now_v7(), Uuid::now_v7());
        for id in [alive, dead] {
            m.processor(id).add_reads(&ReadCounts {
                hit: 1,
                ..Default::default()
            });
        }
        assert_eq!(m.processor_snapshots().len(), 2);

        // a standby node has no processors and no tables: there is nothing to
        // reconcile against, so an empty set must not wipe the counters
        m.retain_processors(&FxHashSet::default());
        assert_eq!(m.processor_snapshots().len(), 2);

        m.retain_processors(&[alive].into_iter().collect());
        let left = m.processor_snapshots();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, alive);
    }
    use crate::cache::lookup_table::immutable::SkipCounts;

    fn stats(rows_read: u64, skipped: SkipCounts, elapsed_ms: u64) -> LoadStats {
        LoadStats {
            rows_read,
            skipped,
            elapsed: Duration::from_millis(elapsed_ms),
            ..Default::default()
        }
    }

    #[test]
    fn a_processor_handle_is_created_once_and_then_shared() {
        let metrics = TableMetrics::default();
        let node = Uuid::now_v7();
        let first = metrics.processor(node);
        let second = metrics.processor(node);
        assert!(
            Arc::ptr_eq(&first, &second),
            "every worker thread must reach the same counters"
        );
        assert!(!Arc::ptr_eq(&first, &metrics.processor(Uuid::now_v7())));
        assert_eq!(metrics.processor_snapshots().len(), 2);
    }

    #[test]
    fn processor_snapshots_are_sorted_by_id() {
        let metrics = TableMetrics::default();
        let mut ids: Vec<_> = (0..4).map(|_| Uuid::now_v7()).collect();
        for id in ids.iter().rev() {
            metrics.processor(*id);
        }
        ids.sort_unstable();
        let seen: Vec<_> = metrics
            .processor_snapshots()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(seen, ids, "label sets have to be stable between ticks");
    }

    #[test]
    fn read_and_write_tallies_accumulate() {
        let metrics = TableMetrics::default();
        let node = Uuid::now_v7();
        let handle = metrics.processor(node);

        handle.add_reads(&ReadCounts {
            hit: 7,
            missed: 2,
            errored: 1,
        });
        handle.add_reads(&ReadCounts {
            hit: 3,
            ..Default::default()
        });
        handle.add_writes(&WriteCounts {
            inserted: 4,
            updated: 1,
            rejected: 2,
            errored: 0,
            removes: 5,
        });

        let snap = handle.get(node);
        assert_eq!(snap.id, node);
        assert_eq!(snap.lookups_hit, 10);
        assert_eq!(snap.lookups_missed, 2);
        assert_eq!(snap.lookups_errored, 1);
        assert_eq!(snap.lookups, 13);
        assert_eq!(snap.writes, 7, "the three upsert outcomes, not the removes");
        assert_eq!(snap.removes, 5);
    }

    #[test]
    fn concurrent_snapshots_keep_totals_equal_to_their_outcomes() {
        let metrics = Arc::new(ProcessorMetrics::default());
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let writer_metrics = metrics.clone();
        let writer_barrier = barrier.clone();
        let writer = std::thread::spawn(move || {
            writer_barrier.wait();
            for _ in 0..50_000 {
                writer_metrics.add_reads(&ReadCounts {
                    hit: 3,
                    missed: 2,
                    errored: 1,
                });
                writer_metrics.add_writes(&WriteCounts {
                    inserted: 4,
                    updated: 3,
                    rejected: 2,
                    errored: 1,
                    removes: 1,
                });
            }
        });
        let node = Uuid::now_v7();
        barrier.wait();
        for _ in 0..100_000 {
            let sample = metrics.get(node);
            assert_eq!(
                sample.lookups,
                sample.lookups_hit + sample.lookups_missed + sample.lookups_errored
            );
            assert_eq!(
                sample.writes,
                sample.writes_inserted
                    + sample.writes_updated
                    + sample.writes_rejected
                    + sample.writes_errored
            );
        }
        writer.join().unwrap();
        assert_eq!(metrics.get(node).lookups, 300_000);
        assert_eq!(metrics.get(node).writes, 500_000);
    }

    #[test]
    fn loads_accumulate_across_generations() {
        let metrics = TableMetrics::default();
        metrics.note_load(&stats(
            100,
            SkipCounts {
                bad_cell: 3,
                overlap: 1,
                ..Default::default()
            },
            40,
        ));
        metrics.note_load(&stats(
            50,
            SkipCounts {
                bad_cell: 2,
                ..Default::default()
            },
            60,
        ));
        metrics.note_load_failure();
        metrics.note_sweep(9, 4);

        let mut out = LookupTableMetrics::default();
        metrics.fill(&mut out);
        assert_eq!(out.loads, 2);
        assert_eq!(out.load_failures, 1, "a failure installs no generation");
        assert_eq!(out.load_duration, Duration::from_millis(100));
        assert_eq!(out.rows_read, 150);
        assert_eq!(out.rows_skipped.bad_cell, 5);
        assert_eq!(out.rows_skipped.overlap, 1);
        assert_eq!(out.rows_skipped.total(), 6);
        assert_eq!(out.rows_expired, 9);
        assert_eq!(out.strings_released, 4);
    }

    #[test]
    fn the_snapshot_pairs_gauges_with_counters_per_table() {
        let cache = Cache::new();
        assert!(snapshot(&cache).is_empty(), "no tables, no samples");

        let config = crate::config::lookup_table::LookupTable {
            id: Uuid::now_v7(),
            name: "hosts".into(),
            source: crate::config::lookup_table::Source::Processor(
                crate::config::lookup_table::Processor {
                    columns: vec![
                        crate::config::lookup_table::ColumnSpec {
                            name: "host".into(),
                            value_type: crate::config::lookup_table::ValueType::String,
                        },
                        crate::config::lookup_table::ColumnSpec {
                            name: "site".into(),
                            value_type: crate::config::lookup_table::ValueType::String,
                        },
                    ],
                    ttl: Duration::from_secs(60),
                    max_rows: 25,
                },
            ),
            key: crate::config::lookup_table::Key::Hash(crate::config::lookup_table::Hash {
                key_columns: vec!["host".into()],
            }),
        };
        let table = cache.lookup_table.insert(&config).unwrap();
        let node = Uuid::now_v7();
        {
            let write = table.as_mutable().unwrap().write_as(node);
            let _ = write
                .upsert_hash(&[
                    crate::cache::lookup_table::cell::CellRef::String("a"),
                    crate::cache::lookup_table::cell::CellRef::String("jkt"),
                ])
                .unwrap();
        }

        let all = snapshot(&cache);
        assert_eq!(all.len(), 1);
        let table = &all[0];
        assert_eq!(table.id, config.id);
        assert_eq!(table.name, "hosts");
        assert_eq!(table.kind, "mutable");
        assert_eq!(table.rows, 1);
        assert_eq!(table.max_rows, 25);
        assert_eq!(table.processors.len(), 1);
        assert_eq!(table.processors[0].id, node);
        assert_eq!(table.processors[0].writes_inserted, 1);
    }
}
