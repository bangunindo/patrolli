// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! In-memory lookup tables.
//!
//! Two kinds share one read API:
//!
//! * **immutable** tables are built from files and swapped wholesale when the
//!   source changes, so reads are lock-free;
//! * **mutable** tables are written by the pipeline and read by processors,
//!   sharded behind `RwLock`s.
//!
//! Reads are synchronous throughout: processor workers run on dedicated threads
//! and `Worker::run_processor` is a sync fn, so an async read API would force
//! a `block_on` on the hot path.

pub mod cell;
pub mod column;
pub mod csv;
pub mod immutable;
pub mod index;
pub mod loader;
pub mod metrics;
pub mod mutable;
pub mod schema;
pub mod status;

use crate::config::lookup_table as cfg;
use crate::payload::Value;
use anyhow::{Result, bail};
use arc_swap::ArcSwap;
use cell::{CellRef, Lookup};
use immutable::ImmutableData;
use immutable::LoadStats;
use loader::SourceStamp;
use metrics::{ProcessorMetrics, ReadCounts, TableMetrics, Totals};
use mutable::{Mutable, MutableRow, TableWrite};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use schema::{ColumnIdx, ColumnLayout, KeyPlan, RowId, Schema};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::cell::Cell;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant, SystemTime};
use tracing::{debug, info, warn};
use utoipa::ToSchema;
use uuid::Uuid;

pub(super) static LOOKUP_TABLE_CACHE: LazyLock<Arc<LookupTable>> =
    LazyLock::new(|| Arc::new(LookupTable::new()));

/// The identity, configuration and counters of one table.
///
/// Behind an `Arc` so the counters live exactly as long as the table: a `remove`
/// takes them with it rather than leaving a dead entity id in a map.
#[derive(Debug)]
pub struct Meta {
    pub id: Uuid,
    pub name: SmolStr,
    pub config: cfg::LookupTable,
    pub metrics: Arc<TableMetrics>,
    /// The epoch `metrics` counts from. Created and dropped with the counters,
    /// so neither can be graphed across a silent reset. A processor-fed table
    /// survives any change that leaves its rows valid — see [`reuses_rows`] —
    /// and the counters, this epoch and those rows still move as one.
    pub created_at: SystemTime,
}

#[derive(Debug, Default, Clone)]
struct LoadState {
    stamp: Option<SourceStamp>,
    last_check: Option<Instant>,
    loaded_at: Option<SystemTime>,
    generation: u64,
    /// completed load attempts, successful or not
    attempts: u64,
    last_error: Option<String>,
    /// An unstable read must be retried on the next maintenance tick.
    retry_pending: bool,
}

/// A file-backed table. The data behind the `ArcSwap` is replaced on reload, so
/// a caller holding this handle always sees the current generation.
#[derive(Debug)]
pub struct ImmutableTable {
    meta: Arc<Meta>,
    data: Arc<ArcSwap<ImmutableData>>,
    load: Arc<Mutex<LoadState>>,
}

impl ImmutableTable {
    fn new(meta: Arc<Meta>) -> Self {
        Self {
            data: Arc::new(ArcSwap::from_pointee(ImmutableData::not_loaded(
                &meta.config.key,
            ))),
            meta,
            load: Arc::new(Mutex::new(LoadState::default())),
        }
    }

    /// Metadata edits share live load state, including any in-flight reload.
    /// Source/key edits get an independent replacement seeded with the last
    /// good generation, so an old loader cannot publish into the new config.
    fn reconfigured(&self, config: &cfg::LookupTable, file: &cfg::File) -> Self {
        let mut previous_file = self.file().expect("immutable source").clone();
        previous_file.reload_interval = file.reload_interval;
        let unchanged = previous_file == *file && self.meta.config.key == config.key;
        let (data, load) = if unchanged {
            (Arc::clone(&self.data), Arc::clone(&self.load))
        } else {
            // Pair the snapshot and its generation under the install lock.
            let state = self.load.lock();
            let data = self.data();
            let mut replacement = state.clone();
            replacement.stamp = None;
            replacement.last_check = None;
            replacement.retry_pending = true;
            (
                Arc::new(ArcSwap::from(data)),
                Arc::new(Mutex::new(replacement)),
            )
        };
        Self {
            meta: Arc::new(Meta {
                id: config.id,
                name: SmolStr::new(&config.name),
                config: config.clone(),
                metrics: Arc::clone(&self.meta.metrics),
                created_at: self.meta.created_at,
            }),
            data,
            load,
        }
    }

    pub fn meta(&self) -> &Arc<Meta> {
        &self.meta
    }

    pub fn data(&self) -> Arc<ImmutableData> {
        self.data.load_full()
    }

    pub fn is_loaded(&self) -> bool {
        self.load.lock().generation > 0
    }

    pub fn generation(&self) -> u64 {
        self.load.lock().generation
    }

    pub fn last_error(&self) -> Option<String> {
        self.load.lock().last_error.clone()
    }

    /// Parses actually performed; static files should not make this grow.
    pub fn attempts(&self) -> u64 {
        self.load.lock().attempts
    }

    pub fn file(&self) -> Option<&cfg::File> {
        match &self.meta.config.source {
            cfg::Source::File(f) => Some(f),
            cfg::Source::Processor(_) => None,
        }
    }

    pub fn due_for_check(&self, interval: std::time::Duration) -> bool {
        let load = self.load.lock();
        load.retry_pending || load.last_check.is_none_or(|t| t.elapsed() >= interval)
    }

    /// Records that the sources were checked and reports whether they changed;
    /// never-stamped counts as changed.
    ///
    /// Keyed on the stamp, not `generation`: that stays zero until a load
    /// *succeeds*, so a source that stays broken would be re-parsed on every
    /// tick forever, defeating [`Self::note_error`].
    pub fn note_check(&self, stamp: &SourceStamp) -> bool {
        let mut load = self.load.lock();
        load.last_check = Some(Instant::now());
        load.retry_pending || load.stamp.is_none() || load.stamp.as_ref() != Some(stamp)
    }

    pub fn install(&self, mut data: ImmutableData, stamp: SourceStamp) -> u64 {
        let mut load = self.load.lock();
        self.meta.metrics.note_load(data.stats());
        load.generation += 1;
        load.attempts += 1;
        load.stamp = Some(stamp);
        load.loaded_at = Some(SystemTime::now());
        load.last_error = None;
        load.retry_pending = false;
        let generation = load.generation;
        data.set_generation(generation);
        self.data.store(Arc::new(data));
        generation
    }

    /// Records a failed load, keeping the data already in place. Stamps the
    /// files it failed on, so the retry waits until they change — which is also
    /// when they could have been fixed.
    pub fn note_error(&self, stamp: SourceStamp, error: String) {
        self.note_load_error(stamp, error, false);
    }

    pub(super) fn note_load_error(&self, stamp: SourceStamp, error: String, retry: bool) {
        let mut load = self.load.lock();
        self.meta.metrics.note_load_failure();
        load.attempts += 1;
        load.stamp = Some(stamp);
        load.last_error = Some(error);
        load.retry_pending = retry;
    }
}

#[derive(Debug)]
pub struct MutableTable {
    meta: Arc<Meta>,
    /// Behind an `Arc` so a config edit that cannot invalidate the rows can hand
    /// them to a new [`Meta`] instead of starting over — see [`reuses_rows`].
    table: Arc<Mutable>,
}

impl MutableTable {
    pub fn meta(&self) -> &Arc<Meta> {
        &self.meta
    }

    pub fn table(&self) -> &Mutable {
        &self.table
    }

    /// A write handle attributing every write to processor node `processor`.
    /// Acquire once per batch: the outcomes reach the shared counters on drop.
    pub fn write_as(&self, processor: Uuid) -> TableWrite<'_> {
        TableWrite::attributed(&self.table, self.meta.metrics.processor(processor))
    }
}

/// Which storage a table uses. The rendered strings are load-bearing:
/// [`metrics::snapshot`] feeds them to the `kind` Prometheus label.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
    strum_macros::IntoStaticStr,
)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TableKind {
    /// built from files and swapped wholesale when the source changes
    Immutable,
    /// written by the pipeline
    Mutable,
}

/// A handle to one table. Cloning is cheap: everything is behind an `Arc`.
#[derive(Debug, Clone)]
pub enum Table {
    Immutable(Arc<ImmutableTable>),
    Mutable(Arc<MutableTable>),
}

impl Table {
    fn build(config: &cfg::LookupTable) -> Result<Self> {
        let meta = Arc::new(Meta {
            id: config.id,
            name: SmolStr::new(&config.name),
            config: config.clone(),
            metrics: Arc::new(TableMetrics::default()),
            created_at: SystemTime::now(),
        });
        Ok(match &config.source {
            cfg::Source::File(_) => Table::Immutable(Arc::new(ImmutableTable::new(meta))),
            cfg::Source::Processor(processor) => {
                let source: Vec<(SmolStr, cfg::ValueType)> = processor
                    .columns
                    .iter()
                    .map(|c| (SmolStr::new(&c.name), c.value_type))
                    .collect();
                let (_, schema, plan) = ColumnLayout::build(&source, &config.key)?;
                let ttl = (!processor.ttl.is_zero()).then_some(processor.ttl);
                Table::Mutable(Arc::new(MutableTable {
                    meta,
                    table: Arc::new(Mutable::new(
                        Arc::new(schema),
                        plan,
                        ttl,
                        processor.max_rows,
                    )),
                }))
            }
        })
    }

    /// Preserves serving data across compatible edits. File-backed replacements
    /// keep the last good generation until the new source and key load together;
    /// processor-fed tables can retain rows only when their schema/key agree.
    fn build_reusing(config: &cfg::LookupTable, existing: Option<&Table>) -> Result<Self> {
        if let (Some(Table::Immutable(previous)), cfg::Source::File(file)) =
            (existing, &config.source)
        {
            return Ok(Table::Immutable(Arc::new(
                previous.reconfigured(config, file),
            )));
        }
        let (Some(Table::Mutable(previous)), cfg::Source::Processor(processor)) =
            (existing, &config.source)
        else {
            return Self::build(config);
        };
        if !reuses_rows(&previous.meta.config, config) {
            let rows_dropped = previous.table.len();
            let table = Self::build(config)?;
            info!(
                lookup_table_id = %config.id,
                lookup_table = %config.name,
                rows_dropped,
                "mutable lookup table rebuilt, rows discarded"
            );
            return Ok(table);
        }
        if processor.max_rows < previous.table.len() {
            warn!(
                lookup_table_id = %config.id,
                lookup_table = %config.name,
                rows = previous.table.len(),
                max_rows = processor.max_rows,
                "max_rows lowered below current row count"
            );
        }
        previous.table.reconfigure(
            (!processor.ttl.is_zero()).then_some(processor.ttl),
            processor.max_rows,
        );
        Ok(Table::Mutable(Arc::new(MutableTable {
            meta: Arc::new(Meta {
                id: config.id,
                name: SmolStr::new(&config.name),
                config: config.clone(),
                // The rows survive, so the counters describing them must too,
                // and `created_at` with them: it is the epoch they count from.
                metrics: Arc::clone(&previous.meta.metrics),
                created_at: previous.meta.created_at,
            }),
            table: Arc::clone(&previous.table),
        })))
    }

    pub fn id(&self) -> Uuid {
        self.meta().id
    }

    pub fn name(&self) -> SmolStr {
        self.meta().name.clone()
    }

    pub fn meta(&self) -> &Arc<Meta> {
        match self {
            Table::Immutable(t) => &t.meta,
            Table::Mutable(t) => &t.meta,
        }
    }

    pub fn kind(&self) -> TableKind {
        match self {
            Table::Immutable(_) => TableKind::Immutable,
            Table::Mutable(_) => TableKind::Mutable,
        }
    }

    pub fn as_immutable(&self) -> Option<&Arc<ImmutableTable>> {
        match self {
            Table::Immutable(t) => Some(t),
            Table::Mutable(_) => None,
        }
    }

    pub fn as_mutable(&self) -> Option<&Arc<MutableTable>> {
        match self {
            Table::Mutable(t) => Some(t),
            Table::Immutable(_) => None,
        }
    }

    /// A snapshot to read from. Acquire once per batch, then probe it many
    /// times. Probes are not counted; use [`Table::read_as`] from a processor.
    pub fn read(&self) -> TableRead {
        TableRead {
            data: self.snapshot(),
            tally: None,
        }
    }

    /// As [`Table::read`], attributing every probe to processor node
    /// `processor`. Outcomes are tallied without atomics and reach the shared
    /// counters when the guard drops.
    pub fn read_as(&self, processor: Uuid) -> TableRead {
        TableRead {
            data: self.snapshot(),
            tally: Some(ReadTally::new(self.meta().metrics.processor(processor))),
        }
    }

    fn snapshot(&self) -> Snapshot {
        match self {
            Table::Immutable(t) => Snapshot::Immutable(t.data()),
            Table::Mutable(t) => Snapshot::Mutable(Arc::clone(t)),
        }
    }

    /// Everything countable about this table, in one pass.
    ///
    /// Cheap enough that an async caller needs no `spawn_blocking`, and must
    /// **stay** sync: no guard taken below is meant to cross an `.await`.
    pub fn status(&self) -> TableStatus {
        let meta = self.meta();
        match self {
            Table::Immutable(t) => {
                // Order matters: `install` stores the generation and calls
                // `note_load` under this same lock, so everything below has to
                // be read inside it or the report can pair generation N's load
                // stats with N+1's row count — or report `loads < generation`,
                // which the DTO says cannot happen.
                let load = t.load.lock();
                let totals = meta.metrics.totals();
                let data = t.data();
                TableStatus {
                    id: meta.id,
                    name: meta.name.clone(),
                    kind: TableKind::Immutable,
                    rows: data.rows(),
                    index_entries: data.index_entries(),
                    max_rows: 0,
                    pooled_strings: data.interned_strings(),
                    shards: 0,
                    generation: load.generation,
                    generation_stats: data.stats().clone(),
                    loaded_at: load.loaded_at,
                    // Read under the lock we already hold. `Instant::elapsed` is
                    // monotonic, so unlike a stored wall clock this stays
                    // meaningful across a clock change; a caller anchors it to
                    // its own single reading of `now`.
                    checked_elapsed: load.last_check.map(|at| at.elapsed()),
                    retry_pending: load.retry_pending,
                    last_error: load.last_error.clone(),
                    rejected: 0,
                    created_at: meta.created_at,
                    schema: Arc::clone(data.schema()),
                    key_plan: data.key_plan().clone(),
                    totals,
                }
            }
            // No lock to pair with: a mutable table has no generation, so its
            // gauges and its counters cannot describe different loads.
            Table::Mutable(t) => TableStatus {
                totals: meta.metrics.totals(),
                id: meta.id,
                name: meta.name.clone(),
                kind: TableKind::Mutable,
                rows: t.table.len(),
                index_entries: t.table.len(),
                max_rows: t.table.max_rows(),
                pooled_strings: t.table.pooled_strings(),
                shards: t.table.shards(),
                // One, not zero: the metric renderers document it that way, and a
                // processor-fed table has exactly one live set of rows.
                generation: 1,
                generation_stats: LoadStats::default(),
                loaded_at: None,
                checked_elapsed: None,
                retry_pending: false,
                last_error: None,
                rejected: t.table.rejected(),
                created_at: meta.created_at,
                schema: Arc::clone(t.table.schema()),
                key_plan: t.table.key_plan().clone(),
            },
        }
    }
}

/// One table's live state, sampled in a single pass by [`Table::status`].
///
/// Internal, not a wire type: the shared input to two very different reports.
/// [`metrics::snapshot`] wants zeros and `&'static str` labels;
/// [`status::local_status`] wants absence and real timestamps.
#[derive(Debug, Clone)]
pub struct TableStatus {
    pub id: Uuid,
    pub name: SmolStr,
    pub kind: TableKind,
    pub rows: usize,
    pub index_entries: usize,
    /// hard row cap; zero for an immutable table, which has none
    pub max_rows: usize,
    /// An immutable table's per-generation interner, or a mutable table's
    /// per-shard pools — different populations, so comparable across time for
    /// one table but not between kinds.
    pub pooled_strings: usize,
    /// independently locked shards; zero for an immutable table, which is
    /// lock-free
    pub shards: usize,
    pub generation: u64,
    /// How the generation currently in memory was built. Default for a mutable
    /// table, which is not built from a source at all.
    pub generation_stats: LoadStats,
    pub loaded_at: Option<SystemTime>,
    /// How long ago the reloader last stat-ed the sources. Monotonic, so the
    /// caller anchors it to its own clock. `None` for a mutable table, and
    /// before the first check.
    pub checked_elapsed: Option<Duration>,
    /// A source changed during loading; it is due again on the next tick.
    pub retry_pending: bool,
    pub last_error: Option<String>,
    pub rejected: u64,
    /// The epoch `totals` counts from; see [`Meta::created_at`].
    pub created_at: SystemTime,
    /// Read from the same snapshot as `rows`, so the two cannot disagree.
    pub schema: Arc<Schema>,
    /// The key resolved against `schema`.
    pub key_plan: KeyPlan,
    /// Cumulative counters for the life of this table.
    pub totals: Totals,
}

/// Which kind of table a [`TableRead`] is reading.
#[derive(Debug)]
enum Snapshot {
    Immutable(Arc<ImmutableData>),
    Mutable(Arc<MutableTable>),
}

/// Probe outcomes accumulated without atomics, flushed when the guard drops.
///
/// Every worker thread shares one processor node id, so a `fetch_add` per probe
/// would ping-pong one cache line between cores on the per-record path.
#[derive(Debug)]
struct ReadTally {
    metrics: Arc<ProcessorMetrics>,
    hit: Cell<u64>,
    missed: Cell<u64>,
    errored: Cell<u64>,
}

impl ReadTally {
    fn new(metrics: Arc<ProcessorMetrics>) -> Self {
        Self {
            metrics,
            hit: Cell::new(0),
            missed: Cell::new(0),
            errored: Cell::new(0),
        }
    }

    #[inline]
    fn record(&self, found: bool) {
        let slot = if found { &self.hit } else { &self.missed };
        slot.set(slot.get() + 1);
    }

    #[inline]
    fn record_key_error(&self) {
        self.errored.set(self.errored.get() + 1);
    }
}

impl Drop for ReadTally {
    fn drop(&mut self) {
        self.metrics.add_reads(&ReadCounts {
            hit: self.hit.get(),
            missed: self.missed.get(),
            errored: self.errored.get(),
        });
    }
}

/// A snapshot of one table, cheap to hold for the length of a batch. Not
/// `Sync`: the tally is unsynchronized, matching [`Table::read_as`]'s
/// once-per-batch, one-thread contract.
#[derive(Debug)]
pub struct TableRead {
    data: Snapshot,
    tally: Option<ReadTally>,
}

impl TableRead {
    pub fn schema(&self) -> &Schema {
        match &self.data {
            Snapshot::Immutable(d) => d.schema(),
            Snapshot::Mutable(t) => t.table.schema(),
        }
    }

    pub fn column(&self, name: &str) -> Option<ColumnIdx> {
        self.schema().index_of(name)
    }

    pub fn key_plan(&self) -> &KeyPlan {
        match &self.data {
            Snapshot::Immutable(d) => d.key_plan(),
            Snapshot::Mutable(t) => t.table.key_plan(),
        }
    }

    /// Bumps whenever the schema may have changed, so a caller can cache
    /// resolved column indices. Zero means an immutable table that has never
    /// loaded; a mutable table's schema is fixed by config and always reports 1.
    pub fn generation(&self) -> u64 {
        match &self.data {
            Snapshot::Immutable(d) => d.generation(),
            Snapshot::Mutable(_) => 1,
        }
    }

    pub fn len(&self) -> usize {
        match &self.data {
            Snapshot::Immutable(d) => d.rows(),
            Snapshot::Mutable(t) => t.table.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn lookup(&self, key: Lookup<'_>) -> Option<Row<'_>> {
        let row = match &self.data {
            Snapshot::Immutable(data) => data.lookup(key).map(|row| Row::Immutable {
                data: data.as_ref(),
                row,
            }),
            Snapshot::Mutable(t) => t.table.get(key).map(Row::Mutable),
        };
        if let Some(tally) = &self.tally {
            tally.record(row.is_some());
        }
        row
    }

    /// A record whose probe key could not be built at all — a missing or
    /// uncastable source attribute. Counts toward `lookups` and
    /// `lookups_errored`, never toward hits or misses.
    pub fn note_key_error(&self) {
        if let Some(tally) = &self.tally {
            tally.record_key_error();
        }
    }
}

/// One matched row. A mutable row owns its `Arc`, so no lock is held while it
/// is read.
#[derive(Debug, Clone)]
pub enum Row<'a> {
    Immutable { data: &'a ImmutableData, row: RowId },
    Mutable(Arc<MutableRow>),
}

impl Row<'_> {
    #[inline]
    pub fn cell(&self, column: ColumnIdx) -> CellRef<'_> {
        match self {
            Row::Immutable { data, row } => data.cell(*row, column),
            Row::Mutable(r) => r.cell(column),
        }
    }

    /// The cell as a telemetry value, borrowing where it can.
    #[inline]
    pub fn value(&self, column: ColumnIdx) -> Value<'_> {
        self.cell(column).to_value()
    }
}

/// Whether a config change leaves a processor-fed table's rows valid. Only the
/// columns and the key shape the stored rows; the name, `ttl` and `max_rows`
/// are applied in place by [`Mutable::reconfigure`].
fn reuses_rows(old: &cfg::LookupTable, new: &cfg::LookupTable) -> bool {
    match (&old.source, &new.source) {
        (cfg::Source::Processor(a), cfg::Source::Processor(b)) => {
            a.columns == b.columns && old.key == new.key
        }
        _ => false,
    }
}

/// The process-wide set of lookup tables.
///
/// Reads go through an `ArcSwap`, so resolving a table never blocks. Mutations
/// rebuild the map under a mutex — rare, and the expensive part of a reload
/// happens outside it.
#[derive(Debug)]
pub struct LookupTable {
    registry: ArcSwap<FxHashMap<Uuid, Table>>,
    admin: Mutex<()>,
}

impl Default for LookupTable {
    fn default() -> Self {
        Self::new()
    }
}

impl LookupTable {
    pub fn new() -> Self {
        Self {
            registry: ArcSwap::from_pointee(FxHashMap::default()),
            admin: Mutex::new(()),
        }
    }

    pub fn get(&self, id: &Uuid) -> Option<Table> {
        self.registry.load().get(id).cloned()
    }

    pub fn len(&self) -> usize {
        self.registry.load().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Adds or replaces one table. An existing table with the same id is
    /// dropped, which for an immutable table means the next check reloads it.
    pub fn insert(&self, config: &cfg::LookupTable) -> Result<Table> {
        let table = Table::build(config)?;
        let _guard = self.admin.lock();
        let mut tables = self.registry.load().as_ref().clone();
        tables.insert(config.id, table.clone());
        self.registry.store(Arc::new(tables));
        debug!(lookup_table_id = %config.id, lookup_table = %config.name, "lookup table registered");
        Ok(table)
    }

    /// Adds or replaces one table, keeping the existing one when its
    /// configuration is unchanged.
    ///
    /// The per-row counterpart of [`Self::replace`], for the database delta
    /// path: the `lookup_tables` trigger fires on any row update, including
    /// edits the runtime never reads, and rebuilding on those would drop a
    /// loaded generation and re-parse a large file for nothing. Use
    /// [`Self::insert`] to mean "reload this now".
    ///
    /// Builds under `admin`, unlike `insert`, because whether to build depends
    /// on the current map. Building does no I/O.
    pub fn upsert(&self, config: &cfg::LookupTable) -> Result<Table> {
        let _guard = self.admin.lock();
        if let Some(existing) = self.registry.load().get(&config.id)
            && existing.meta().config == *config
        {
            return Ok(existing.clone());
        }
        let current = self.registry.load();
        let table = Table::build_reusing(config, current.get(&config.id))?;
        let mut tables = current.as_ref().clone();
        tables.insert(config.id, table.clone());
        self.registry.store(Arc::new(tables));
        debug!(lookup_table_id = %config.id, lookup_table = %config.name, "lookup table registered");
        Ok(table)
    }

    pub fn remove(&self, id: &Uuid) -> Option<Table> {
        let _guard = self.admin.lock();
        let mut tables = self.registry.load().as_ref().clone();
        let removed = tables.remove(id)?;
        self.registry.store(Arc::new(tables));
        debug!(lookup_table_id = %id, lookup_table = %removed.meta().name, "lookup table removed");
        Some(removed)
    }

    /// Replaces the whole set, keeping tables whose config is unchanged so a
    /// reconfiguration does not throw away loaded data needlessly.
    pub fn replace(&self, configs: &[cfg::LookupTable]) -> Result<()> {
        let _guard = self.admin.lock();
        let current = self.registry.load();
        let mut tables = FxHashMap::default();
        let mut kept = 0usize;
        for config in configs {
            let table = match current.get(&config.id) {
                Some(existing) if existing.meta().config == *config => {
                    kept += 1;
                    existing.clone()
                }
                existing => Table::build_reusing(config, existing)?,
            };
            if tables.insert(config.id, table).is_some() {
                bail!("duplicate lookup table id {}", config.id);
            }
        }
        let removed = current.keys().filter(|id| !tables.contains_key(id)).count();
        let built = tables.len() - kept;
        self.registry.store(Arc::new(tables));
        debug!(kept, built, removed, "lookup table set replaced");
        Ok(())
    }

    pub fn clear(&self) {
        let _guard = self.admin.lock();
        self.registry.store(Arc::new(FxHashMap::default()));
    }

    pub fn tables(&self) -> Vec<Table> {
        self.registry.load().values().cloned().collect()
    }

    pub fn immutable_tables(&self) -> Vec<Arc<ImmutableTable>> {
        self.registry
            .load()
            .values()
            .filter_map(|t| t.as_immutable().cloned())
            .collect()
    }

    pub fn mutable_tables(&self) -> Vec<Arc<MutableTable>> {
        self.registry
            .load()
            .values()
            .filter_map(|t| t.as_mutable().cloned())
            .collect()
    }

    /// Every table on this node, sorted by name then id — the consumer is a
    /// list a person reads, and names are not unique. [`metrics::snapshot`]
    /// sorts by id instead, where label-set stability between ticks matters.
    ///
    /// **Empty until this node has loaded its configuration**, and a node that
    /// drops to standby clears it again. An empty answer means "not configured
    /// here, or not this node's turn", never a failure.
    pub fn status(&self) -> Vec<TableStatus> {
        let mut all: Vec<TableStatus> = self.registry.load().values().map(Table::status).collect();
        all.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cell::{KeyRef, RangePoint};
    use std::time::Duration;

    fn processor_table(
        name: &str,
        key: cfg::Key,
        columns: &[(&str, cfg::ValueType)],
    ) -> cfg::LookupTable {
        cfg::LookupTable {
            id: Uuid::now_v7(),
            name: name.to_string(),
            source: cfg::Source::Processor(cfg::Processor {
                columns: columns
                    .iter()
                    .map(|(n, t)| cfg::ColumnSpec {
                        name: n.to_string(),
                        value_type: *t,
                    })
                    .collect(),
                ttl: Duration::from_secs(60),
                max_rows: 100,
            }),
            key,
        }
    }

    fn hash_key(columns: &[&str]) -> cfg::Key {
        cfg::Key::Hash(cfg::Hash {
            key_columns: columns.iter().map(|s| s.to_string()).collect(),
        })
    }

    fn file_table(name: &str, key: cfg::Key) -> cfg::LookupTable {
        cfg::LookupTable {
            id: Uuid::now_v7(),
            name: name.to_string(),
            source: cfg::Source::File(cfg::File {
                format: cfg::Format::Csv(cfg::Csv {
                    type_cast: Default::default(),
                    delimiter: ',',
                    quote: '"',
                    escape: None,
                    null_values: vec![String::new()],
                    headers: vec![],
                    has_headers: Some(true),
                }),
                paths: vec!["/nonexistent.csv".into()],
                compression: cfg::Compression::None,
                location: cfg::Location::FileSystem,
                reload_interval: Duration::from_secs(60),
            }),
            key,
        }
    }

    #[test]
    fn tables_resolve_by_id() {
        let registry = LookupTable::new();
        let config = processor_table(
            "hosts",
            hash_key(&["host"]),
            &[
                ("host", cfg::ValueType::String),
                ("site", cfg::ValueType::String),
            ],
        );
        registry.insert(&config).unwrap();

        assert_eq!(registry.len(), 1);
        assert_eq!(registry.get(&config.id).unwrap().name(), "hosts");
        assert!(registry.get(&Uuid::now_v7()).is_none());
    }

    #[test]
    fn removing_a_table_drops_it() {
        let registry = LookupTable::new();
        let config = processor_table(
            "hosts",
            hash_key(&["host"]),
            &[
                ("host", cfg::ValueType::String),
                ("site", cfg::ValueType::String),
            ],
        );
        registry.insert(&config).unwrap();
        assert!(registry.remove(&config.id).is_some());
        assert!(registry.get(&config.id).is_none());
        assert!(registry.is_empty());
        assert!(registry.remove(&config.id).is_none());
    }

    #[test]
    fn replace_keeps_tables_whose_config_did_not_change() {
        let registry = LookupTable::new();
        let a = processor_table(
            "a",
            hash_key(&["k"]),
            &[("k", cfg::ValueType::String), ("v", cfg::ValueType::String)],
        );
        let b = file_table("b", hash_key(&["k"]));
        registry.replace(std::slice::from_ref(&a)).unwrap();

        let before = registry.get(&a.id).unwrap();
        // a table that keeps its config keeps its data too
        registry.replace(&[a.clone(), b.clone()]).unwrap();
        let after = registry.get(&a.id).unwrap();
        match (&before, &after) {
            (Table::Mutable(x), Table::Mutable(y)) => assert!(Arc::ptr_eq(x, y)),
            _ => panic!("expected a mutable table"),
        }
        assert_eq!(registry.len(), 2);

        // ...and one whose config changed is rebuilt
        let mut changed = a.clone();
        if let cfg::Source::Processor(p) = &mut changed.source {
            p.max_rows = 7;
        }
        registry.replace(&[changed.clone()]).unwrap();
        let rebuilt = registry.get(&a.id).unwrap();
        match (&before, &rebuilt) {
            (Table::Mutable(x), Table::Mutable(y)) => assert!(!Arc::ptr_eq(x, y)),
            _ => panic!("expected a mutable table"),
        }
        assert_eq!(registry.len(), 1, "b was dropped");
        assert_eq!(rebuilt.as_mutable().unwrap().table().max_rows(), 7);
    }

    /// The database trigger logs an upsert for any row change, including edits
    /// the runtime never reads. Rebuilding then would drop a loaded generation
    /// and reset the counters, so the delta path compares the config first.
    #[test]
    fn upsert_keeps_the_table_when_the_config_is_unchanged() {
        let registry = LookupTable::new();
        let config = processor_table(
            "a",
            hash_key(&["k"]),
            &[("k", cfg::ValueType::String), ("v", cfg::ValueType::String)],
        );
        registry.upsert(&config).unwrap();
        let before = registry.get(&config.id).unwrap();

        registry.upsert(&config).unwrap();
        let after = registry.get(&config.id).unwrap();
        match (&before, &after) {
            (Table::Mutable(x), Table::Mutable(y)) => assert!(
                Arc::ptr_eq(x, y),
                "an unchanged config must not rebuild the table"
            ),
            _ => panic!("expected a mutable table"),
        }
        // the counters live on `Meta`, so this is what a rebuild would reset
        assert!(Arc::ptr_eq(before.meta(), after.meta()));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn upsert_rebuilds_when_the_config_changed() {
        let registry = LookupTable::new();
        let config = processor_table(
            "a",
            hash_key(&["k"]),
            &[("k", cfg::ValueType::String), ("v", cfg::ValueType::String)],
        );
        registry.upsert(&config).unwrap();
        let before = registry.get(&config.id).unwrap();

        let mut changed = config.clone();
        if let cfg::Source::Processor(p) = &mut changed.source {
            p.max_rows = 7;
        }
        registry.upsert(&changed).unwrap();
        let after = registry.get(&config.id).unwrap();
        match (&before, &after) {
            (Table::Mutable(x), Table::Mutable(y)) => assert!(!Arc::ptr_eq(x, y)),
            _ => panic!("expected a mutable table"),
        }
        assert_eq!(after.as_mutable().unwrap().table().max_rows(), 7);
        assert_eq!(registry.len(), 1, "the id is the identity, so it replaced");
    }

    /// Unlike `replace`, `upsert` leaves everything it was not given alone: the
    /// delta path is told only which ids changed, never the whole set.
    #[test]
    fn upsert_adds_a_table_without_disturbing_the_others() {
        let registry = LookupTable::new();
        let a = processor_table(
            "a",
            hash_key(&["k"]),
            &[("k", cfg::ValueType::String), ("v", cfg::ValueType::String)],
        );
        let b = file_table("b", hash_key(&["k"]));
        registry.upsert(&a).unwrap();
        let before = registry.get(&a.id).unwrap();

        registry.upsert(&b).unwrap();
        assert_eq!(registry.len(), 2);
        let after = registry.get(&a.id).unwrap();
        match (&before, &after) {
            (Table::Mutable(x), Table::Mutable(y)) => assert!(Arc::ptr_eq(x, y)),
            _ => panic!("expected a mutable table"),
        }
    }

    #[test]
    fn replace_rejects_duplicate_ids_but_allows_duplicate_names() {
        let registry = LookupTable::new();
        let a = processor_table(
            "dup",
            hash_key(&["k"]),
            &[("k", cfg::ValueType::String), ("v", cfg::ValueType::String)],
        );
        // the id is the identity; the name is only a label, as everywhere else
        let mut same_name = a.clone();
        same_name.id = Uuid::now_v7();
        registry.replace(&[a.clone(), same_name.clone()]).unwrap();
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.get(&a.id).unwrap().name(), "dup");
        assert_eq!(registry.get(&same_name.id).unwrap().name(), "dup");

        let mut same_id = a.clone();
        same_id.name = "other".into();
        assert!(registry.replace(&[a.clone(), same_id]).is_err());
    }

    #[test]
    fn a_mutable_table_reads_through_the_shared_api() {
        let registry = LookupTable::new();
        let config = processor_table(
            "hosts",
            hash_key(&["host"]),
            &[
                ("host", cfg::ValueType::String),
                ("site", cfg::ValueType::String),
            ],
        );
        let table = registry.insert(&config).unwrap();
        let inserted = table
            .as_mutable()
            .unwrap()
            .table()
            .upsert_hash(&[CellRef::String("a"), CellRef::String("jkt")])
            .unwrap();
        assert_eq!(inserted, mutable::Upsert::Inserted);

        let read = table.read();
        assert_eq!(read.generation(), 1, "a mutable schema is fixed by config");
        assert_eq!(read.len(), 1);
        let site = read.column("site").unwrap();
        let row = read.lookup(Lookup::Hash(&[KeyRef::String("a")])).unwrap();
        assert_eq!(row.cell(site), CellRef::String("jkt"));
        assert!(matches!(row.value(site), Value::String(ref z) if z.as_str() == "jkt"));
        assert!(read.lookup(Lookup::Hash(&[KeyRef::String("b")])).is_none());
    }

    #[test]
    fn a_mutable_range_table_reads_through_the_shared_api() {
        let registry = LookupTable::new();
        let config = processor_table(
            "vlans",
            cfg::Key::Range(cfg::Range {
                range_type: cfg::RangeType::Integer,
                partition_columns: vec![],
                start_column: "lo".into(),
                start_inclusive: true,
                end_column: "hi".into(),
                end_inclusive: true,
            }),
            &[
                ("lo", cfg::ValueType::Integer),
                ("hi", cfg::ValueType::Integer),
                ("zone", cfg::ValueType::String),
            ],
        );
        let table = registry.insert(&config).unwrap();
        let _ = table
            .as_mutable()
            .unwrap()
            .table()
            .upsert_range(
                &[CellRef::String("dmz")],
                RangePoint::Integer(100),
                true,
                RangePoint::Integer(199),
                true,
            )
            .unwrap();
        assert_eq!(table.read().len(), 1);

        let read = table.read();
        let zone = read.column("zone").unwrap();
        let row = read
            .lookup(Lookup::Range {
                partition: &[],
                point: RangePoint::Integer(150),
            })
            .unwrap();
        assert_eq!(row.cell(zone), CellRef::String("dmz"));
    }

    #[test]
    fn an_unloaded_file_table_reads_as_empty_rather_than_failing() {
        let registry = LookupTable::new();
        let config = file_table("geo", hash_key(&["k"]));
        let table = registry.insert(&config).unwrap();
        assert_eq!(table.kind(), TableKind::Immutable);
        assert!(!table.as_immutable().unwrap().is_loaded());

        let read = table.read();
        assert_eq!(read.generation(), 0, "zero means never loaded");
        assert!(read.is_empty());
        assert!(read.column("anything").is_none());
        assert!(read.lookup(Lookup::Hash(&[KeyRef::String("a")])).is_none());
        assert!(
            read.lookup(Lookup::Range {
                partition: &[],
                point: RangePoint::Integer(1)
            })
            .is_none()
        );
    }

    #[test]
    fn status_reports_each_table() {
        let registry = LookupTable::new();
        registry
            .insert(&processor_table(
                "zzz",
                hash_key(&["k"]),
                &[("k", cfg::ValueType::String), ("v", cfg::ValueType::String)],
            ))
            .unwrap();
        registry
            .insert(&file_table("aaa", hash_key(&["k"])))
            .unwrap();
        let status = registry.status();
        assert_eq!(status.len(), 2);
        assert_eq!(status[0].name, "aaa", "sorted by name");
        assert_eq!(status[0].kind, TableKind::Immutable);
        assert_eq!(status[1].kind, TableKind::Mutable);
    }

    /// A hash table with one row, `a -> jkt`.
    fn hosts_table(registry: &LookupTable) -> Table {
        let config = processor_table(
            "hosts",
            hash_key(&["host"]),
            &[
                ("host", cfg::ValueType::String),
                ("site", cfg::ValueType::String),
            ],
        );
        let table = registry.insert(&config).unwrap();
        let _ = table
            .as_mutable()
            .unwrap()
            .table()
            .upsert_hash(&[CellRef::String("a"), CellRef::String("jkt")])
            .unwrap();
        table
    }

    fn processor_metrics(
        table: &Table,
        node: Uuid,
    ) -> crate::common::internal_metrics::LookupTableProcessor {
        table
            .meta()
            .metrics
            .processor_snapshots()
            .into_iter()
            .find(|p| p.id == node)
            .unwrap_or_default()
    }

    #[test]
    fn read_as_attributes_hits_and_misses_to_one_processor() {
        let registry = LookupTable::new();
        let table = hosts_table(&registry);
        let node = Uuid::now_v7();

        let read = table.read_as(node);
        assert!(read.lookup(Lookup::Hash(&[KeyRef::String("a")])).is_some());
        assert!(read.lookup(Lookup::Hash(&[KeyRef::String("a")])).is_some());
        assert!(read.lookup(Lookup::Hash(&[KeyRef::String("b")])).is_none());
        read.note_key_error();

        // the guard tallies without atomics, so nothing has reached the shared
        // counters yet
        assert_eq!(processor_metrics(&table, node).lookups, 0);

        drop(read);
        let m = processor_metrics(&table, node);
        assert_eq!(m.lookups_hit, 2);
        assert_eq!(m.lookups_missed, 1);
        assert_eq!(m.lookups_errored, 1);
        assert_eq!(
            m.lookups, 4,
            "the total covers hits, misses and key errors alike"
        );
        assert_eq!(m.writes, 0, "reads are not writes");
    }

    #[test]
    fn an_unattributed_read_registers_no_processor() {
        let registry = LookupTable::new();
        let table = hosts_table(&registry);

        let read = table.read();
        assert!(read.lookup(Lookup::Hash(&[KeyRef::String("a")])).is_some());
        assert!(read.lookup(Lookup::Hash(&[KeyRef::String("b")])).is_none());
        drop(read);

        assert!(
            table.meta().metrics.processor_snapshots().is_empty(),
            "read() is for status paths and tests, and should stay invisible"
        );
    }

    #[test]
    fn two_processors_are_counted_separately() {
        let registry = LookupTable::new();
        let table = hosts_table(&registry);
        let (one, two) = (Uuid::now_v7(), Uuid::now_v7());

        {
            let read = table.read_as(one);
            read.lookup(Lookup::Hash(&[KeyRef::String("a")]));
        }
        {
            let read = table.read_as(two);
            read.lookup(Lookup::Hash(&[KeyRef::String("nope")]));
            read.lookup(Lookup::Hash(&[KeyRef::String("nope")]));
        }

        let all = table.meta().metrics.processor_snapshots();
        assert_eq!(all.len(), 2);
        assert_eq!(processor_metrics(&table, one).lookups_hit, 1);
        assert_eq!(processor_metrics(&table, one).lookups_missed, 0);
        assert_eq!(processor_metrics(&table, two).lookups_hit, 0);
        assert_eq!(processor_metrics(&table, two).lookups_missed, 2);
    }

    #[test]
    fn write_as_records_each_upsert_outcome() {
        let registry = LookupTable::new();
        let mut config = processor_table(
            "hosts",
            hash_key(&["host"]),
            &[
                ("host", cfg::ValueType::String),
                ("site", cfg::ValueType::String),
            ],
        );
        if let cfg::Source::Processor(p) = &mut config.source {
            p.max_rows = 1;
        }
        let table = registry.insert(&config).unwrap();
        let mutable = table.as_mutable().unwrap();
        let node = Uuid::now_v7();

        {
            let write = mutable.write_as(node);
            // the outcome still reaches the caller, so a saturated table stays
            // actionable rather than silently counted
            assert_eq!(
                write
                    .upsert_hash(&[CellRef::String("a"), CellRef::String("jkt")])
                    .unwrap(),
                mutable::Upsert::Inserted
            );
            assert_eq!(
                write
                    .upsert_hash(&[CellRef::String("a"), CellRef::String("sby")])
                    .unwrap(),
                mutable::Upsert::Updated
            );
            assert_eq!(
                write
                    .upsert_hash(&[CellRef::String("b"), CellRef::String("bdg")])
                    .unwrap(),
                mutable::Upsert::Rejected
            );
            assert!(
                write
                    .upsert_hash(&[CellRef::Integer(1), CellRef::String("x")])
                    .is_err(),
                "a cell of the wrong type is an error, not a rejection"
            );
            write.note_key_error();
            assert!(write.remove(Lookup::Hash(&[KeyRef::String("a")])));
            assert!(!write.remove(Lookup::Hash(&[KeyRef::String("a")])));
        }

        let m = processor_metrics(&table, node);
        assert_eq!(m.writes_inserted, 1);
        assert_eq!(m.writes_updated, 1);
        assert_eq!(m.writes_rejected, 1);
        assert_eq!(m.writes_errored, 2, "the bad cell and the key error");
        assert_eq!(m.writes, 5, "removes are counted apart from writes");
        assert_eq!(m.removes, 1, "only the removal that found a row");
        assert_eq!(m.lookups, 0, "writes are not reads");
    }

    #[test]
    fn removing_a_table_takes_its_counters_with_it() {
        let registry = LookupTable::new();
        let table = hosts_table(&registry);
        let node = Uuid::now_v7();
        {
            let read = table.read_as(node);
            read.lookup(Lookup::Hash(&[KeyRef::String("a")]));
        }
        assert_eq!(processor_metrics(&table, node).lookups, 1);

        // the counters hang off `Meta`, so they cannot outlive the table and
        // leave a dead table id in a snapshot
        registry.remove(&table.id());
        let fresh = registry.insert(&table.meta().config).unwrap();
        assert!(fresh.meta().metrics.processor_snapshots().is_empty());
    }

    #[test]
    fn replace_preserves_counters_exactly_when_it_preserves_data() {
        let registry = LookupTable::new();
        let table = hosts_table(&registry);
        let node = Uuid::now_v7();
        {
            let read = table.read_as(node);
            read.lookup(Lookup::Hash(&[KeyRef::String("a")]));
        }

        let config = table.meta().config.clone();
        registry.replace(std::slice::from_ref(&config)).unwrap();
        assert_eq!(
            processor_metrics(&registry.get(&config.id).unwrap(), node).lookups,
            1,
            "an unchanged config keeps the table, so it keeps the counters"
        );

        // Retuning the cap invalidates no row, and a processor-fed table's rows
        // exist nowhere else, so the table and its counters are kept.
        let mut retuned = config.clone();
        if let cfg::Source::Processor(p) = &mut retuned.source {
            p.max_rows = 7;
        }
        registry.replace(std::slice::from_ref(&retuned)).unwrap();
        let kept = registry.get(&retuned.id).unwrap();
        assert_eq!(kept.status().rows, 1, "the rows are not rebuildable");
        assert_eq!(kept.status().max_rows, 7, "and the new cap took effect");
        assert_eq!(
            processor_metrics(&kept, node).lookups,
            1,
            "the counters still describe those same rows"
        );

        // Changing the columns does invalidate them, so that is a new table.
        let mut restructured = config.clone();
        if let cfg::Source::Processor(p) = &mut restructured.source {
            p.columns.push(cfg::ColumnSpec {
                name: "region".into(),
                value_type: cfg::ValueType::String,
            });
        }
        registry.replace(&[restructured.clone()]).unwrap();
        let rebuilt = registry.get(&restructured.id).unwrap();
        assert_eq!(rebuilt.status().rows, 0);
        assert!(
            rebuilt.meta().metrics.processor_snapshots().is_empty(),
            "a structural change is a new table, and starts from zero"
        );
    }

    #[test]
    fn status_reports_the_occupancy_gauges() {
        let registry = LookupTable::new();
        let table = hosts_table(&registry);
        let status = table.status();
        assert_eq!(status.rows, 1);
        assert_eq!(status.max_rows, 100);
        assert_eq!(
            status.generation_stats.rows_read, 0,
            "a mutable table is not built from a source"
        );

        let file = registry
            .insert(&file_table("geo", hash_key(&["k"])))
            .unwrap();
        let status = file.status();
        assert_eq!(status.max_rows, 0, "an immutable table has no cap");
        assert_eq!(
            status.pooled_strings, 0,
            "and nothing interned yet, because it has not loaded"
        );
        assert!(status.shards == 0, "and no shards; it is lock-free");
    }

    /// Reported as an age off the reloader's monotonic clock, read from the
    /// load lock the immutable arm already holds — so a later refactor cannot
    /// quietly add a second acquisition.
    #[test]
    fn status_reports_the_last_check_as_a_monotonic_age() {
        let registry = LookupTable::new();
        let table = registry
            .insert(&file_table("geo", hash_key(&["k"])))
            .unwrap();
        assert!(
            table.status().checked_elapsed.is_none(),
            "nothing has stat-ed the sources yet"
        );

        let immutable = table.as_immutable().unwrap();
        immutable.note_check(&loader::SourceStamp::never_matches());
        let elapsed = table
            .status()
            .checked_elapsed
            .expect("a checked table reports its age");
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "the age is measured from the check, not from the epoch: {elapsed:?}"
        );
        assert!(
            table.status().checked_elapsed.unwrap() >= elapsed,
            "and it only grows"
        );
    }

    /// `rows` and the reported columns must come from one `ArcSwap` read, or a
    /// reload landing mid-status could pair one generation's schema with
    /// another's row count.
    #[test]
    fn status_reports_the_schema_of_the_generation_it_counted() {
        let registry = LookupTable::new();
        let table = hosts_table(&registry);
        let status = table.status();
        assert_eq!(status.rows, 1);
        assert_eq!(status.schema.names(), ["host", "site"]);
        assert!(matches!(status.key_plan, KeyPlan::Hash { .. }));
    }

    #[test]
    fn the_cache_exposes_an_isolated_registry_per_instance() {
        let a = crate::cache::Cache::new();
        let b = crate::cache::Cache::new();
        a.lookup_table
            .insert(&processor_table(
                "t",
                hash_key(&["k"]),
                &[("k", cfg::ValueType::String), ("v", cfg::ValueType::String)],
            ))
            .unwrap();
        assert_eq!(a.lookup_table.len(), 1);
        assert_eq!(b.lookup_table.len(), 0, "test caches do not share state");
    }
}
