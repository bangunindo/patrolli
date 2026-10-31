// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The lookup-table status report, as the frontend sees it.
//!
//! The half of a table's state the metric path loses:
//! [`crate::common::internal_metrics::LookupTable`] collapses a load error to a
//! boolean, has no room for the schema, and dates its counters by nothing.
//!
//! Layered like `cluster::inter::item_stats`: [`local_status`] reads the registry
//! and owns the clock, [`merge_local`] holds the rules so they can be tested
//! against a hand-built `Vec`. Everything maps an already-taken [`TableStatus`];
//! nothing here takes a lock. It lives under `lookup_table` so it can read
//! `LoadState`'s private fields — anywhere else `checked_elapsed` would cost a
//! second lock acquisition per table.

use super::schema::{ColumnIdx, KeyPlan, Schema};
use super::{LookupTable, Meta, Table, TableKind, TableStatus};
use crate::common::internal_metrics::LookupTableProcessor;
use crate::config::lookup_table as cfg;
use chrono::{DateTime, FixedOffset, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::fmt::Display;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use utoipa::ToSchema;
use uuid::Uuid;

/// Coarse health of a table, derived on read.
///
/// Derived rather than stored because no writer can set it, as with
/// [`crate::cache::item::ItemState::Stale`]: `Degraded` means the *latest*
/// reload failed while an *earlier* one succeeded, and nothing on the load path
/// knows both at once. Discriminants are pinned for a future
/// `v1::LookupTableHealth` whose proto3 zero value maps to `Unknown`.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
    strum_macros::IntoStaticStr,
)]
#[repr(u8)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TableHealth {
    /// Never produced locally: what an unrecognised peer value becomes, so a
    /// version skew degrades to "cannot say" rather than to a wrong state.
    #[default]
    Unknown = 0,
    /// Serving the current data.
    Ready = 1,
    /// Registered but holding no rows: a first load still running, one that
    /// succeeded against an empty or wholly rejected file, or a processor-fed
    /// table nothing has written. Every lookup misses.
    NeverPopulated = 2,
    /// Serving answers, some of them wrong or missing: a file-backed table
    /// whose latest reload failed and is still serving an older generation, or
    /// a processor-fed table at its row cap, turning away every new key.
    Degraded = 3,
    /// Every load has failed and none ever succeeded; `load.last_error` says
    /// what happened. Unreachable for a processor-fed table, which has no source.
    Failed = 4,
}

impl TableHealth {
    /// Total order for a cluster fan-out merge: the worst state wins.
    ///
    /// Ranked by how much a user should care, not by discriminant.
    /// `NeverPopulated` sits below `Degraded` because paging on a boot is worse
    /// than not paging — the placement
    /// [`crate::cache::item::ItemState::severity`] gives `NeverRan`.
    ///
    /// TODO(cluster): for a *roll-up badge* only. A lookup table exists on every
    /// node with independently loaded data, so a node whose file is broken must
    /// stay visible: key the merged result by `(table id, node id)` rather than
    /// keeping one node's row the way `collect_item_stats` does.
    pub fn severity(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Ready => 1,
            Self::NeverPopulated => 2,
            Self::Degraded => 3,
            Self::Failed => 4,
        }
    }
}

/// One lookup table as the frontend sees it, on one node.
///
/// Final consumable form, following [`crate::cache::item::StatsSnapshot`]:
/// timestamps in the configured zone, durations as milliseconds, and a field
/// that does not apply to this `kind` **absent rather than zero**, so "no row
/// cap" reads differently from "a cap of zero".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct LookupTableStatus {
    /// Stable across a reload and a no-op `replace`, and the same on every node.
    pub id: Uuid,
    /// The configured name. **Not unique**, so never use it as a key.
    #[schema(value_type = String)]
    pub name: SmolStr,
    pub kind: TableKind,
    /// Derived from the fields below on every read; see [`TableHealth`].
    pub health: TableHealth,
    /// The epoch this table's cumulative counters and generation numbering start
    /// from. A `replace` that keeps the table keeps this; a structural change
    /// rebuilds both together, so a graph of `usage` cannot span a reset
    /// silently. `None` only for an instant no datetime can represent.
    pub created_at: Option<DateTime<FixedOffset>>,

    pub rows: u64,
    /// Index entries currently stored. Exceeds `rows` when several keys or
    /// ranges resolve to one deduplicated row.
    pub index_entries: u64,
    /// Distinct strings held: the nearest available proxy for what they cost,
    /// there being no accurate byte estimate. The two kinds pool different
    /// populations, so this is comparable across time for one table but not
    /// between kinds.
    pub pooled_strings: u64,

    /// The columns and key *as actually resolved*. For a CSV table the header
    /// row is the schema, so this is knowable only after a load.
    pub schema: TableSchema,

    /// Every processor's counters folded into one, so a caller does not have to
    /// sum `processors` itself. Dated by `created_at`.
    pub usage: Usage,
    /// One entry per processor node that has read or written this table, sorted
    /// by id. Only nodes using the attributed handles (`Table::read_as`,
    /// `MutableTable::write_as`) appear, so for a `NeverPopulated` processor-fed
    /// table, empty here is the diagnosis.
    pub processors: Vec<LookupTableProcessor>,

    /// `None` for a processor-fed table rather than a zeroed struct: "loaded zero
    /// times" and "cannot be loaded" are different answers.
    pub load: Option<LoadStatus>,
    /// `None` for a file-backed table, which has neither.
    pub capacity: Option<CapacityStatus>,
}

/// The load history of a file-backed table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct LoadStatus {
    /// Bumped by each load that installed a new generation; zero before the
    /// first success. Unlike the `lookup_table.generation` metric this whole
    /// section is *absent* for a processor-fed table rather than reporting a
    /// constant `1`, which would mean nothing to a reader.
    pub generation: u64,
    /// When the current generation was installed. `None` before the first
    /// success, so "never loaded" stays distinguishable from "loaded at the
    /// epoch".
    pub loaded_at: Option<DateTime<FixedOffset>>,
    /// When the sources were last stat-ed, whether or not they had changed.
    /// `None` before the first check.
    ///
    /// Derived from the loader's monotonic `Instant`, which no wall-clock jump
    /// can fool, re-expressed against the sweep's single clock reading — so it
    /// can lag by the length of the sweep, always backwards.
    pub checked_at: Option<DateTime<FixedOffset>>,
    /// `checked_at` plus the reload interval, or immediately due after an
    /// unstable read. `None` before the first check, when the table is due at
    /// the next tick. An estimate: a slow load pushes the whole sweep back.
    pub next_check_at: Option<DateTime<FixedOffset>>,
    /// Why the most recent load failed — the string the `lookup_table.load_error`
    /// metric can only collapse to a boolean. Present alongside a served
    /// `generation` is exactly [`TableHealth::Degraded`].
    pub last_error: Option<String>,
    /// How the generation now in memory was built. Replaced outright by each
    /// reload, so it describes the data being served rather than a running
    /// total. Its `unique_rows`/`index_entries` repeat the parent's `rows` and
    /// `index_entries`, an artefact of reporting the struct verbatim.
    pub generation_stats: super::LoadStats,
    /// Loads that installed a generation, for the life of this table.
    pub loads: u64,
    /// Climbing while `loads` stands still is a file that keeps failing to parse.
    pub load_failures: u64,
    /// Time spent in successful loads, for the life of this table.
    #[serde(with = "crate::common::serde::duration_millis")]
    #[schema(value_type = u64)]
    pub load_duration: Duration,
    /// Source rows read across every load.
    pub rows_read: u64,
    /// Source rows rejected across every load, by reason.
    pub rows_skipped: super::immutable::SkipCounts,
}

/// The cap and expiry of a processor-fed table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CapacityStatus {
    /// Hard row cap. Once `rows` reaches it, writes that would introduce a new
    /// row are rejected; existing rows can still be updated.
    pub max_rows: u64,
    /// Cumulative, so a table that overflowed once and has since drained reports
    /// a non-zero count here and [`TableHealth::Ready`].
    pub rejected: u64,
    /// Rows dropped by the TTL sweep, for the life of this table.
    pub rows_expired: u64,
    /// Pool entries released by compaction, for the life of this table. Mutable
    /// only: an immutable interner is replaced wholesale with its generation.
    pub strings_released: u64,
    /// How many independently locked shards the rows are spread over. Fixed for
    /// the table's life; the only number that explains write contention.
    pub shards: u32,
    /// Rows untouched for this long are dropped by the maintenance sweep. `None`
    /// is expiry disabled, not "expire instantly". Seconds rather than a
    /// `Duration` because `serde(with = ...)` does not apply through an `Option`.
    pub ttl_seconds: Option<u64>,
}

/// The columns and key of a table, as resolved against the data in memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TableSchema {
    /// Materialized columns, in row order. **Empty until a file-backed table has
    /// loaded once**, because a CSV table's columns come from its header row.
    /// A range table's bound columns are not here — they live in `key`.
    pub columns: Vec<ColumnInfo>,
    pub key: KeyInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ColumnInfo {
    #[schema(value_type = String)]
    pub name: SmolStr,
    /// The cast actually applied, which for a CSV column with no `type_cast`
    /// entry is the default rather than anything the config names.
    pub value_type: cfg::ValueType,
    /// Derived from `key`, so a header renders without a second pass.
    pub is_key: bool,
}

/// The key configuration as resolved against the columns beside it.
///
/// `type` is spelled the same as [`cfg::Key`], so the config API and the status
/// API name a key the same way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum KeyInfo {
    Hash(HashKeyInfo),
    Range(RangeKeyInfo),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HashKeyInfo {
    /// Key columns in probe order. Empty until the schema resolves.
    #[schema(value_type = Vec<String>)]
    pub columns: Vec<SmolStr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RangeKeyInfo {
    /// Partition columns, empty for an unpartitioned table — which is also what
    /// an unresolved schema reports.
    #[schema(value_type = Vec<String>)]
    pub partition_columns: Vec<SmolStr>,
    pub range_type: cfg::RangeType,
    pub bounds: RangeBounds,
}

/// The range bound columns, and whether each end is inclusive.
///
/// The one part of a key taken from config rather than resolved data: the bounds
/// never appear in [`TableSchema::columns`], so there is no runtime name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RangeBounds {
    pub start_column: String,
    pub start_inclusive: bool,
    pub end_column: String,
    pub end_inclusive: bool,
}

/// Every processor's counters for one table, folded into one.
///
/// Summed from the rows reported beside them, never from a table-level atomic
/// that could be sampled across a batch flush and disagree with those rows —
/// which is what makes `lookups == hit + missed + errored` hold.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Usage {
    pub lookups: u64,
    pub lookups_hit: u64,
    pub lookups_missed: u64,
    /// The probe key could not be built from the record at all — a missing or
    /// uncastable source attribute, so there was nothing to look up. Not a miss.
    pub lookups_errored: u64,
    pub writes: u64,
    pub writes_inserted: u64,
    pub writes_updated: u64,
    /// Would have exceeded `capacity.max_rows`.
    pub writes_rejected: u64,
    pub writes_errored: u64,
    pub removes: u64,
}

impl Usage {
    /// Fold the per-processor rows into one; see the type's note on why.
    fn sum(rows: &[LookupTableProcessor]) -> Self {
        rows.iter().fold(Self::default(), |mut acc, row| {
            acc.lookups += row.lookups;
            acc.lookups_hit += row.lookups_hit;
            acc.lookups_missed += row.lookups_missed;
            acc.lookups_errored += row.lookups_errored;
            acc.writes += row.writes;
            acc.writes_inserted += row.writes_inserted;
            acc.writes_updated += row.writes_updated;
            acc.writes_rejected += row.writes_rejected;
            acc.writes_errored += row.writes_errored;
            acc.removes += row.removes;
            acc
        })
    }
}

/// One node's answer to a lookup-table status query.
///
/// Derives serde, unlike `cluster::inter::comm::ItemStats`, which is folded into
/// an existing DTO; there is nothing to fold this into. `error` set means the
/// answer is incomplete and the other fields carry whatever did arrive.
/// `node_id` comes from the cluster config, so even an unreachable node is
/// named.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct NodeLookupTableStatus {
    /// Sorted by name, then by id, because names are not unique.
    pub tables: Vec<LookupTableStatus>,
    pub node_id: Uuid,
    /// The single clock reading the whole sweep was taken at. `None` only when a
    /// node never answered — inventing one would misdate every derived timestamp.
    pub at: Option<DateTime<FixedOffset>>,
    /// Requested ids this node holds no table for; empty when no filter was
    /// given. "Not configured here" is a different answer from "filtered out".
    pub unknown_ids: Vec<Uuid>,
    pub error: Option<String>,
}

impl NodeLookupTableStatus {
    /// A node that could not be reached, or gave up with nothing usable. `pub`
    /// because the caller will be the cluster layer, outside this module tree.
    pub fn failed(node_id: Uuid, error: impl Display) -> Self {
        Self {
            tables: Vec::new(),
            node_id,
            at: None,
            unknown_ids: Vec::new(),
            error: Some(error.to_string()),
        }
    }
}

/// Which tables a status query wants.
///
/// Empty versus absent is load-bearing, as in `item_stats::StateFilter`: an
/// absent filter matches every table, one that names nothing matches none.
#[derive(Debug, Clone, Default)]
pub struct StatusFilter {
    ids: Option<Vec<Uuid>>,
}

impl StatusFilter {
    /// Only these ids, in registry order. An empty slice matches no table.
    pub fn by_ids(ids: &[Uuid]) -> Self {
        Self {
            ids: Some(ids.to_vec()),
        }
    }

    fn ids(&self) -> Option<&[Uuid]> {
        self.ids.as_deref()
    }
}

/// This node's own lookup-table status, read straight from the registry.
///
/// Takes the registry rather than a `Cache`, and a native [`StatusFilter`]
/// rather than a request message: `cache` must not grow a wire dependency for an
/// RPC that does not exist yet. The gRPC layer will build the filter, and reject
/// a malformed id.
///
/// Infallible and sync, and must stay sync: no guard taken below is meant to
/// cross an `.await`. A peer's failure is `error` on the envelope, never a
/// variant.
///
/// TODO(cluster): this answers for **one** node, which may legitimately hold
/// zero tables and `error: None` — it may hold no lock, or be on standby, which
/// clears the registry by design. Render that as "no tables on this node", never
/// as a failure, and fan out to the other nodes for the whole picture.
pub fn local_status(
    node_id: Uuid,
    registry: &LookupTable,
    filter: &StatusFilter,
) -> NodeLookupTableStatus {
    let (tables, unknown_ids) = gather(registry, filter.ids());
    let snapshots = tables
        .iter()
        .map(|table| (Arc::clone(table.meta()), table.status()))
        .collect();
    // One clock for all of them: every derived timestamp is anchored to it, so
    // reading it per table would date tables against different nows.
    let now = crate::common::now().fixed_offset();
    merge_local(node_id, now, snapshots, unknown_ids)
}

/// Map already-gathered snapshots into a per-node answer.
///
/// Split from [`local_status`] so the rules can be tested against a hand-built
/// `Vec` — the same shape a fan-out's receive half will take. `Meta` rides along
/// for the config-derived values (reload interval, TTL, range bound names) and
/// the processor counters; reading those atomics is neither a lock nor I/O.
///
/// The id filter is not re-applied: ids are a registry lookup rather than a
/// predicate, so [`gather`] resolves them and reports what it could not find.
pub(crate) fn merge_local(
    node_id: Uuid,
    now: DateTime<FixedOffset>,
    tables: Vec<(Arc<Meta>, TableStatus)>,
    unknown_ids: Vec<Uuid>,
) -> NodeLookupTableStatus {
    let mut tables: Vec<LookupTableStatus> = tables
        .iter()
        .map(|(meta, status)| map_table(now, meta, status))
        .collect();
    tables.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    NodeLookupTableStatus {
        tables,
        node_id,
        at: Some(now),
        unknown_ids,
        error: None,
    }
}

/// Pull the requested tables out of the registry, reporting the ids it holds no
/// entry for. Mirrors `item_stats::gather`, but with no lock to release before
/// snapshotting: the registry reads through an `ArcSwap`.
fn gather(registry: &LookupTable, ids: Option<&[Uuid]>) -> (Vec<Table>, Vec<Uuid>) {
    match ids {
        None => (registry.tables(), Vec::new()),
        Some(ids) => {
            let mut found = Vec::with_capacity(ids.len());
            let mut unknown = Vec::new();
            for id in ids {
                match registry.get(id) {
                    Some(table) => found.push(table),
                    None => unknown.push(*id),
                }
            }
            (found, unknown)
        }
    }
}

/// One table's snapshot, mapped. Pure, with no clock of its own.
fn map_table(now: DateTime<FixedOffset>, meta: &Meta, status: &TableStatus) -> LookupTableStatus {
    let processors = meta.metrics.processor_snapshots();
    LookupTableStatus {
        id: status.id,
        name: status.name.clone(),
        kind: status.kind,
        health: health_of(status),
        created_at: local_of(status.created_at),
        rows: status.rows as u64,
        index_entries: status.index_entries as u64,
        pooled_strings: status.pooled_strings as u64,
        schema: schema_of(&status.schema, &status.key_plan, &meta.config.key),
        usage: Usage::sum(&processors),
        processors,
        load: match status.kind {
            TableKind::Immutable => Some(load_status(now, meta, status)),
            TableKind::Mutable => None,
        },
        capacity: match status.kind {
            TableKind::Mutable => Some(capacity_status(meta, status)),
            TableKind::Immutable => None,
        },
    }
}

/// The health rules, in priority order: the most actionable fact wins, so an
/// error string beats an empty table — the error explains the emptiness.
fn health_of(status: &TableStatus) -> TableHealth {
    match status.kind {
        // `generation == 0` separates "never served anything" from "still
        // serving the last good generation".
        TableKind::Immutable if status.last_error.is_some() => {
            if status.generation == 0 {
                TableHealth::Failed
            } else {
                TableHealth::Degraded
            }
        }
        // Current occupancy, not the cumulative `rejected` counter: one
        // transient overflow must not latch this table `Degraded` for good.
        TableKind::Mutable if status.max_rows > 0 && status.rows >= status.max_rows => {
            TableHealth::Degraded
        }
        // Keyed on rows, not `generation`: a load that *succeeded* against an
        // empty or wholly rejected file would otherwise read `Ready`.
        _ if status.rows == 0 => TableHealth::NeverPopulated,
        _ => TableHealth::Ready,
    }
}

fn load_status(now: DateTime<FixedOffset>, meta: &Meta, status: &TableStatus) -> LoadStatus {
    let reload_interval = match &meta.config.source {
        cfg::Source::File(file) => Some(if status.retry_pending {
            Duration::ZERO
        } else {
            file.reload_interval
        }),
        cfg::Source::Processor(_) => None,
    };
    let checked_at = status.checked_elapsed.and_then(|ago| ago_of(now, ago));
    LoadStatus {
        generation: status.generation,
        loaded_at: status.loaded_at.and_then(local_of),
        checked_at,
        next_check_at: checked_at
            .zip(reload_interval)
            .and_then(|(at, interval)| after(at, interval)),
        last_error: status.last_error.clone(),
        generation_stats: status.generation_stats.clone(),
        loads: status.totals.loads,
        load_failures: status.totals.load_failures,
        load_duration: status.totals.load_duration,
        rows_read: status.totals.rows_read,
        rows_skipped: status.totals.rows_skipped,
    }
}

fn capacity_status(meta: &Meta, status: &TableStatus) -> CapacityStatus {
    let ttl = match &meta.config.source {
        cfg::Source::Processor(processor) => processor.ttl,
        cfg::Source::File(_) => Duration::ZERO,
    };
    CapacityStatus {
        max_rows: status.max_rows as u64,
        rejected: status.rejected,
        rows_expired: status.totals.rows_expired,
        strings_released: status.totals.strings_released,
        shards: status.shards as u32,
        // Zero disables expiry, so it becomes absence: "never expire" is not
        // "expire instantly".
        ttl_seconds: (!ttl.is_zero()).then_some(ttl.as_secs()),
    }
}

fn schema_of(schema: &Schema, plan: &KeyPlan, key: &cfg::Key) -> TableSchema {
    let key_columns = key_indices(plan);
    let columns = schema
        .names()
        .iter()
        .zip(schema.types())
        .enumerate()
        .map(|(idx, (name, value_type))| ColumnInfo {
            name: name.clone(),
            value_type: *value_type,
            is_key: key_columns.contains(&(idx as ColumnIdx)),
        })
        .collect();
    TableSchema {
        columns,
        key: key_of(schema, plan, key),
    }
}

/// The materialized key columns. For a range key, the partition only.
fn key_indices(plan: &KeyPlan) -> &[ColumnIdx] {
    match plan {
        KeyPlan::Hash { columns } => columns,
        KeyPlan::Range { partition, .. } => partition,
    }
}

fn key_of(schema: &Schema, plan: &KeyPlan, key: &cfg::Key) -> KeyInfo {
    match plan {
        KeyPlan::Hash { columns } => KeyInfo::Hash(HashKeyInfo {
            columns: names_of(schema, columns),
        }),
        KeyPlan::Range {
            partition,
            range_type,
        } => KeyInfo::Range(RangeKeyInfo {
            partition_columns: names_of(schema, partition),
            range_type: *range_type,
            bounds: match key {
                cfg::Key::Range(range) => RangeBounds {
                    start_column: range.start_column.clone(),
                    start_inclusive: range.start_inclusive,
                    end_column: range.end_column.clone(),
                    end_inclusive: range.end_inclusive,
                },
                // Unreachable: a range plan is only built from a range key.
                // Defaulted, not panicked: a panic is fatal (`common::panic`).
                cfg::Key::Hash(_) => RangeBounds::default(),
            },
        }),
    }
}

fn names_of(schema: &Schema, columns: &[ColumnIdx]) -> Vec<SmolStr> {
    columns
        .iter()
        .filter_map(|idx| schema.name(*idx).map(SmolStr::new))
        .collect()
}

/// A wall-clock instant in the local zone, with the offset resolved per instant
/// — so a `loaded_at` across a DST change carries the offset in force *then*.
fn to_local(t: DateTime<Utc>) -> DateTime<FixedOffset> {
    to_local_in(t, &crate::common::DEFAULT_TZ)
}

/// [`to_local`] against an explicit zone. Split out to be testable: under an
/// unset or UTC `$TZ`, asserting against `DEFAULT_TZ` proves nothing.
fn to_local_in(t: DateTime<Utc>, tz: &chrono_tz::Tz) -> DateTime<FixedOffset> {
    t.with_timezone(tz).fixed_offset()
}

/// A stored `SystemTime` in the local zone, or `None` if it names no
/// representable instant. Fallible rather than `DateTime::<Utc>::from`: a panic
/// is fatal (`common::panic`), and a diagnostics read must not take the process
/// down.
fn local_of(t: SystemTime) -> Option<DateTime<FixedOffset>> {
    let since = t.duration_since(UNIX_EPOCH).ok()?;
    DateTime::from_timestamp(since.as_secs() as i64, since.subsec_nanos()).map(to_local)
}

/// `now` less a monotonic age. Backdated by however long the sweep has been
/// running, so it can never land in the future.
fn ago_of(now: DateTime<FixedOffset>, ago: Duration) -> Option<DateTime<FixedOffset>> {
    now.checked_sub_signed(TimeDelta::from_std(ago).ok()?)
}

fn after(at: DateTime<FixedOffset>, interval: Duration) -> Option<DateTime<FixedOffset>> {
    at.checked_add_signed(TimeDelta::from_std(interval).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::Cache;
    use crate::cache::lookup_table::cell::{CellRef, KeyRef, Lookup, RangePoint};
    use crate::cache::lookup_table::loader;
    use std::path::Path;

    const NODE: fn() -> Uuid = Uuid::now_v7;

    fn csv_format() -> cfg::Format {
        cfg::Format::Csv(cfg::Csv {
            type_cast: Default::default(),
            delimiter: ',',
            quote: '"',
            escape: None,
            null_values: vec![String::new()],
            headers: vec![],
            has_headers: Some(true),
        })
    }

    fn file_table(
        name: &str,
        paths: Vec<String>,
        key: cfg::Key,
        reload: Duration,
    ) -> cfg::LookupTable {
        cfg::LookupTable {
            id: Uuid::now_v7(),
            name: name.to_string(),
            source: cfg::Source::File(cfg::File {
                format: csv_format(),
                paths,
                compression: cfg::Compression::None,
                location: cfg::Location::FileSystem,
                reload_interval: reload,
            }),
            key,
        }
    }

    fn processor_table(
        name: &str,
        key: cfg::Key,
        columns: &[(&str, cfg::ValueType)],
        max_rows: usize,
    ) -> cfg::LookupTable {
        cfg::LookupTable {
            id: Uuid::now_v7(),
            name: name.to_string(),
            source: cfg::Source::Processor(cfg::Processor {
                columns: columns
                    .iter()
                    .map(|(name, value_type)| cfg::ColumnSpec {
                        name: name.to_string(),
                        value_type: *value_type,
                    })
                    .collect(),
                ttl: Duration::from_secs(60),
                max_rows,
            }),
            key,
        }
    }

    fn hash_key(columns: &[&str]) -> cfg::Key {
        cfg::Key::Hash(cfg::Hash {
            key_columns: columns.iter().map(|s| s.to_string()).collect(),
        })
    }

    fn hosts(max_rows: usize) -> cfg::LookupTable {
        processor_table(
            "hosts",
            hash_key(&["host"]),
            &[
                ("host", cfg::ValueType::String),
                ("site", cfg::ValueType::String),
            ],
            max_rows,
        )
    }

    fn write(path: &Path, contents: &str) {
        std::fs::write(path, contents).unwrap();
    }

    /// Every table on a node, with no filter — the common shape in these tests.
    fn all(registry: &LookupTable) -> Vec<LookupTableStatus> {
        local_status(NODE(), registry, &StatusFilter::default()).tables
    }

    fn only(registry: &LookupTable) -> LookupTableStatus {
        let mut tables = all(registry);
        assert_eq!(tables.len(), 1, "expected exactly one table");
        tables.pop().unwrap()
    }

    // ---- health derivation ----

    /// Registration alone is not an error. If the startup transient read as a
    /// failure, every node boot would page someone.
    #[test]
    fn a_freshly_registered_file_table_reports_never_populated() {
        let registry = LookupTable::new();
        registry
            .insert(&file_table(
                "geo",
                vec!["/nonexistent.csv".into()],
                hash_key(&["k"]),
                Duration::from_secs(60),
            ))
            .unwrap();

        let status = only(&registry);
        assert_eq!(status.health, TableHealth::NeverPopulated);
        let load = status.load.expect("a file-backed table has a load section");
        assert_eq!(load.generation, 0);
        assert!(load.last_error.is_none(), "nothing has been attempted yet");
    }

    /// "Serving nothing, and we know why" must be distinguishable from the
    /// startup transient, because only one of the two needs a human.
    #[tokio::test]
    async fn a_table_whose_only_load_attempt_failed_reports_failed() {
        let cache = Cache::new();
        let conf = file_table(
            "geo",
            vec!["/nonexistent.csv".into()],
            hash_key(&["k"]),
            Duration::from_nanos(1),
        );
        loader::init(std::slice::from_ref(&conf), &cache)
            .await
            .unwrap();

        let status = only(&cache.lookup_table);
        assert_eq!(status.health, TableHealth::Failed);
        assert_eq!(status.rows, 0);
        let load = status.load.unwrap();
        assert_eq!(load.generation, 0, "no generation was ever installed");
        assert!(load.last_error.is_some(), "and the reason is reported");
        assert_eq!(load.load_failures, 1);
        assert_eq!(load.loads, 0);
    }

    /// The whole point of `Degraded`, and of `note_error` keeping the old data:
    /// the reader must see the served rows *and* the error, not one or the other.
    #[tokio::test]
    async fn a_failed_reload_over_good_data_reports_degraded_and_keeps_serving() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site\na,jkt\nb,bdg\n");

        let cache = Cache::new();
        let conf = file_table(
            "hosts",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            Duration::from_nanos(1),
        );
        loader::init(std::slice::from_ref(&conf), &cache)
            .await
            .unwrap();
        assert_eq!(only(&cache.lookup_table).health, TableHealth::Ready);

        // the key column disappears, so the schema no longer resolves
        write(&path, "other,site\na,jkt\nb,bdg\n");
        loader::check_once(&cache).await;

        let status = only(&cache.lookup_table);
        assert_eq!(status.health, TableHealth::Degraded);
        assert_eq!(status.rows, 2, "the previous generation is still served");
        let load = status.load.unwrap();
        assert_eq!(load.generation, 1, "the failure installed nothing");
        assert!(load.last_error.is_some());
        assert!(load.loaded_at.is_some(), "dated by the load that did work");
        assert_eq!((load.loads, load.load_failures), (1, 1));
    }

    /// A load can succeed and still leave the table serving nothing — a
    /// truncated file is the common cause. A `generation`-based rule would call
    /// this `Ready` and hide it.
    #[tokio::test]
    async fn a_load_that_found_no_rows_reports_never_populated_not_ready() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site\n");

        let cache = Cache::new();
        let conf = file_table(
            "hosts",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            Duration::from_nanos(1),
        );
        loader::init(std::slice::from_ref(&conf), &cache)
            .await
            .unwrap();

        let status = only(&cache.lookup_table);
        assert_eq!(status.health, TableHealth::NeverPopulated);
        let load = status.load.unwrap();
        assert_eq!(load.generation, 1, "the load itself succeeded");
        assert!(load.last_error.is_none(), "and reported no error");
    }

    /// A silently full table rejects every new key while answering for the ones
    /// it already has. Without this rule it reports `Ready`, which is the worst
    /// available failure mode for a lookup.
    #[test]
    fn a_mutable_table_at_its_row_cap_reports_degraded() {
        let registry = LookupTable::new();
        let table = registry.insert(&hosts(1)).unwrap();
        let mutable = table.as_mutable().unwrap();
        let _ = mutable
            .table()
            .upsert_hash(&[CellRef::String("a"), CellRef::String("jkt")])
            .unwrap();

        let status = only(&registry);
        assert_eq!(status.health, TableHealth::Degraded);
        let capacity = status.capacity.expect("a processor-fed table has a cap");
        assert_eq!((capacity.max_rows, status.rows), (1, 1));
    }

    /// A table no processor node references is a config mistake that looks
    /// exactly like a healthy empty table from the outside.
    #[test]
    fn a_mutable_table_no_processor_has_written_reports_never_populated() {
        let registry = LookupTable::new();
        registry.insert(&hosts(100)).unwrap();

        let status = only(&registry);
        assert_eq!(status.health, TableHealth::NeverPopulated);
        assert!(
            status.processors.is_empty(),
            "and the empty processor list is the diagnosis"
        );
        assert!(status.load.is_none(), "there is no source to load from");
    }

    /// Two states sharing a rank would make a cluster roll-up depend on the
    /// order the nodes happened to answer in.
    #[test]
    fn health_severity_is_a_total_order_with_no_ties() {
        let all = [
            TableHealth::Unknown,
            TableHealth::Ready,
            TableHealth::NeverPopulated,
            TableHealth::Degraded,
            TableHealth::Failed,
        ];
        let mut ranks: Vec<u8> = all.iter().map(|h| h.severity()).collect();
        ranks.sort_unstable();
        ranks.dedup();
        assert_eq!(ranks.len(), all.len(), "every state ranks differently");
        assert!(
            TableHealth::NeverPopulated.severity() < TableHealth::Degraded.severity(),
            "a table that has not loaded yet must not outrank a broken one"
        );
        assert_eq!(
            all.iter().map(|h| h.severity()).max(),
            Some(TableHealth::Failed.severity()),
            "Failed is the worst answer a node can give"
        );
    }

    // ---- what the metric path loses ----

    /// `lookup_table.load_error` can only be a boolean. Carrying the message is
    /// the reason this report exists at all.
    #[tokio::test]
    async fn the_load_error_string_survives_into_the_status() {
        let cache = Cache::new();
        let conf = file_table(
            "geo",
            vec!["/nonexistent.csv".into()],
            hash_key(&["k"]),
            Duration::from_nanos(1),
        );
        loader::init(std::slice::from_ref(&conf), &cache)
            .await
            .unwrap();

        let error = only(&cache.lookup_table).load.unwrap().last_error.unwrap();
        assert!(
            error.contains("nonexistent.csv"),
            "the message must name the file: {error}"
        );
    }

    /// Without the first rejection message, "10,000 rows skipped" gives a reader
    /// nothing to act on.
    #[tokio::test]
    async fn the_first_rejection_message_reaches_the_status() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        // the second row is short, so it is skipped rather than fatal
        write(&path, "host,site\na,jkt\nb\n");

        let cache = Cache::new();
        let conf = file_table(
            "hosts",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            Duration::from_nanos(1),
        );
        loader::init(std::slice::from_ref(&conf), &cache)
            .await
            .unwrap();

        let load = only(&cache.lookup_table).load.unwrap();
        assert_eq!(load.generation_stats.skipped.total(), 1);
        assert!(
            load.generation_stats.first_error.is_some(),
            "the count comes with a reason"
        );
        assert_eq!(load.rows_skipped.total(), 1, "and is counted cumulatively");
    }

    /// A file that keeps failing to parse is invisible unless the two counters
    /// are reported separately.
    #[tokio::test]
    async fn load_failures_climb_while_the_generation_stands_still() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site\na,jkt\n");

        let cache = Cache::new();
        let conf = file_table(
            "hosts",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            Duration::from_nanos(1),
        );
        loader::init(std::slice::from_ref(&conf), &cache)
            .await
            .unwrap();

        for contents in ["other,site\na,jkt\n", "another,site\na,jkt\n"] {
            write(&path, contents);
            loader::check_once(&cache).await;
        }

        let load = only(&cache.lookup_table).load.unwrap();
        assert_eq!(load.generation, 1, "no failure installed a generation");
        assert_eq!(load.loads, 1);
        assert_eq!(load.load_failures, 2);
    }

    /// A CSV table's columns and casts come from the file, so reading them from
    /// the config would be a plausible-looking lie.
    #[tokio::test]
    async fn the_resolved_schema_and_key_come_from_the_loaded_generation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site,rack\na,jkt,r1\n");

        let cache = Cache::new();
        let conf = file_table(
            "hosts",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            Duration::from_nanos(1),
        );
        loader::init(std::slice::from_ref(&conf), &cache)
            .await
            .unwrap();

        let schema = only(&cache.lookup_table).schema;
        let names: Vec<&str> = schema.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            ["host", "site", "rack"],
            "the header row is the schema, and the config never named `rack`"
        );
        assert_eq!(
            schema.key,
            KeyInfo::Hash(HashKeyInfo {
                columns: vec![SmolStr::new("host")]
            })
        );
        assert!(schema.columns[0].is_key);
        assert!(!schema.columns[1].is_key);
    }

    /// Absence reported as absence, rather than the configured names presented
    /// as if they had been resolved.
    #[test]
    fn a_table_that_has_never_loaded_reports_an_empty_schema() {
        let registry = LookupTable::new();
        registry
            .insert(&file_table(
                "geo",
                vec!["/nonexistent.csv".into()],
                hash_key(&["k"]),
                Duration::from_secs(60),
            ))
            .unwrap();

        let schema = only(&registry).schema;
        assert!(schema.columns.is_empty());
        assert_eq!(
            schema.key,
            KeyInfo::Hash(HashKeyInfo { columns: vec![] }),
            "the key resolves to nothing until the schema does"
        );
    }

    // ---- the range-key trap ----

    /// The bound columns are consumed by the index and never materialized, so a
    /// reader shown only `columns` would see no key at all.
    #[test]
    fn a_range_keyed_table_names_its_bound_columns_even_though_they_are_not_columns() {
        let registry = LookupTable::new();
        let conf = processor_table(
            "vlans",
            cfg::Key::Range(cfg::Range {
                range_type: cfg::RangeType::Integer,
                partition_columns: vec!["zone".into()],
                start_column: "lo".into(),
                start_inclusive: true,
                end_column: "hi".into(),
                end_inclusive: false,
            }),
            &[
                ("zone", cfg::ValueType::String),
                ("lo", cfg::ValueType::Integer),
                ("hi", cfg::ValueType::Integer),
                ("name", cfg::ValueType::String),
            ],
            100,
        );
        registry.insert(&conf).unwrap();

        let schema = only(&registry).schema;
        let names: Vec<&str> = schema.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            ["zone", "name"],
            "`lo` and `hi` were consumed by the index"
        );
        let KeyInfo::Range(range) = schema.key else {
            panic!("expected a range key");
        };
        assert_eq!(range.partition_columns, [SmolStr::new("zone")]);
        assert_eq!(range.range_type, cfg::RangeType::Integer);
        assert_eq!(
            range.bounds,
            RangeBounds {
                start_column: "lo".into(),
                start_inclusive: true,
                end_column: "hi".into(),
                end_inclusive: false,
            },
            "the only place the bound names survive is the config"
        );
    }

    /// `is_key` is derived convenience; if it drifts, a rendered table header
    /// contradicts the key printed beside it.
    #[test]
    fn column_is_key_agrees_with_the_key_info() {
        let registry = LookupTable::new();
        let conf = processor_table(
            "vlans",
            cfg::Key::Range(cfg::Range {
                range_type: cfg::RangeType::Integer,
                partition_columns: vec!["zone".into()],
                start_column: "lo".into(),
                start_inclusive: true,
                end_column: "hi".into(),
                end_inclusive: false,
            }),
            &[
                ("zone", cfg::ValueType::String),
                ("lo", cfg::ValueType::Integer),
                ("hi", cfg::ValueType::Integer),
                ("name", cfg::ValueType::String),
            ],
            100,
        );
        registry.insert(&conf).unwrap();

        let schema = only(&registry).schema;
        let flagged: Vec<&str> = schema
            .columns
            .iter()
            .filter(|c| c.is_key)
            .map(|c| c.name.as_str())
            .collect();
        let KeyInfo::Range(range) = &schema.key else {
            panic!("expected a range key");
        };
        let named: Vec<&str> = range
            .partition_columns
            .iter()
            .map(SmolStr::as_str)
            .collect();
        assert_eq!(flagged, named);
    }

    // ---- the clock ----

    /// Pins the `now - elapsed` rule. A refactor that reads a second clock, or
    /// reads the monotonic instant as if it were wall clock, breaks this.
    #[tokio::test]
    async fn checked_at_is_derived_from_one_clock_reading_and_never_lands_in_the_future() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site\na,jkt\n");

        let cache = Cache::new();
        let conf = file_table(
            "hosts",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            Duration::from_secs(60),
        );
        loader::init(std::slice::from_ref(&conf), &cache)
            .await
            .unwrap();

        let answer = local_status(NODE(), &cache.lookup_table, &StatusFilter::default());
        let at = answer.at.expect("a local answer always dates itself");
        let load = answer.tables[0].load.clone().unwrap();
        let checked_at = load.checked_at.expect("`init` stats the sources");
        assert!(
            checked_at <= at,
            "a check cannot have happened after the sweep that reported it"
        );
        assert!(load.loaded_at.unwrap() <= at, "and neither can a load");
    }

    /// A "due at" the reloader would not honour is worse than no estimate.
    #[tokio::test]
    async fn next_check_at_is_the_last_check_plus_the_reload_interval() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site\na,jkt\n");

        let cache = Cache::new();
        let interval = Duration::from_secs(90);
        let conf = file_table(
            "hosts",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            interval,
        );
        loader::init(std::slice::from_ref(&conf), &cache)
            .await
            .unwrap();

        let load = only(&cache.lookup_table).load.unwrap();
        let checked_at = load.checked_at.unwrap();
        let next = load.next_check_at.expect("a checked table has a due time");
        assert_eq!(next - checked_at, TimeDelta::from_std(interval).unwrap());
    }

    #[test]
    fn an_unstable_load_is_reported_as_already_due_for_retry() {
        let registry = LookupTable::new();
        let cfg = file_table(
            "retry",
            vec!["unused.csv".into()],
            hash_key(&["host"]),
            Duration::from_secs(3600),
        );
        let table = registry.insert(&cfg).unwrap();
        let immutable = table.as_immutable().unwrap();
        let stamp = loader::SourceStamp::never_matches();
        immutable.note_check(&stamp);
        immutable.note_load_error(stamp, "source changed".into(), true);
        let status = only(&registry);
        let load = status.load.unwrap();
        assert_eq!(load.next_check_at, load.checked_at);
        assert!(load.last_error.is_some());
    }

    /// Before the first check there is nothing to add an interval to, and
    /// inventing one would claim knowledge the loader does not have.
    #[test]
    fn a_table_never_checked_reports_neither_a_check_nor_a_due_time() {
        let registry = LookupTable::new();
        registry
            .insert(&file_table(
                "geo",
                vec!["/nonexistent.csv".into()],
                hash_key(&["k"]),
                Duration::from_secs(60),
            ))
            .unwrap();

        let load = only(&registry).load.unwrap();
        assert!(load.checked_at.is_none());
        assert!(load.next_check_at.is_none());
    }

    /// CI runs with `$TZ` unset, so asserting against `DEFAULT_TZ` would compare
    /// UTC with UTC and prove nothing about whether a zone is applied.
    #[test]
    fn a_timestamp_is_rendered_at_the_offset_in_force_for_that_instant() {
        let ny: chrono_tz::Tz = "America/New_York".parse().unwrap();
        let winter = DateTime::from_timestamp(1_700_000_000, 0).unwrap(); // Nov 2023
        let summer = DateTime::from_timestamp(1_688_000_000, 0).unwrap(); // Jun 2023

        assert_eq!(
            to_local_in(winter, &ny).offset().local_minus_utc(),
            -5 * 3600,
            "standard time"
        );
        assert_eq!(
            to_local_in(summer, &ny).offset().local_minus_utc(),
            -4 * 3600,
            "daylight time, resolved per instant rather than from `now`"
        );
    }

    // ---- aggregation ----

    /// A page whose total disagrees with the rows printed beneath it destroys
    /// trust in every other number on it.
    #[test]
    fn usage_totals_equal_the_sum_of_the_processor_counters() {
        let registry = LookupTable::new();
        let table = registry.insert(&hosts(100)).unwrap();
        let mutable = table.as_mutable().unwrap();
        for _ in 0..2 {
            let write = mutable.write_as(NODE());
            let _ = write
                .upsert_hash(&[CellRef::String("a"), CellRef::String("jkt")])
                .unwrap();
        }

        let status = only(&registry);
        assert_eq!(status.processors.len(), 2, "two distinct processor nodes");
        let summed = Usage::sum(&status.processors);
        assert_eq!(status.usage, summed);
        assert_eq!(status.usage.writes, 2);
        assert_eq!(status.usage.writes_inserted, 1, "the second was an update");
        assert_eq!(status.usage.writes_updated, 1);
    }

    /// The invariant `ProcessorMetrics::get` maintains per row must survive
    /// summation, or the totals contradict themselves.
    #[test]
    fn usage_keeps_lookups_equal_to_its_three_outcomes() {
        let registry = LookupTable::new();
        let table = registry.insert(&hosts(100)).unwrap();
        let _ = table
            .as_mutable()
            .unwrap()
            .table()
            .upsert_hash(&[CellRef::String("a"), CellRef::String("jkt")])
            .unwrap();

        let node = NODE();
        {
            let read = table.read_as(node);
            assert!(read.lookup(Lookup::Hash(&[KeyRef::String("a")])).is_some());
            assert!(read.lookup(Lookup::Hash(&[KeyRef::String("b")])).is_none());
            read.note_key_error();
        }

        let usage = only(&registry).usage;
        assert_eq!(usage.lookups_hit, 1);
        assert_eq!(usage.lookups_missed, 1);
        assert_eq!(usage.lookups_errored, 1);
        assert_eq!(
            usage.lookups,
            usage.lookups_hit + usage.lookups_missed + usage.lookups_errored
        );
    }

    // ---- the counter epoch ----

    /// If the epoch reset while the counters did not, every rate a reader
    /// computed from `usage` would be wrong.
    #[test]
    fn created_at_survives_a_no_op_replace_alongside_the_counters() {
        let registry = LookupTable::new();
        let conf = hosts(100);
        let table = registry.insert(&conf).unwrap();
        table.as_mutable().unwrap().write_as(NODE());
        let before = only(&registry).created_at;

        registry.replace(std::slice::from_ref(&conf)).unwrap();

        let after = only(&registry);
        assert_eq!(
            after.created_at, before,
            "the same table, so the same epoch"
        );
        assert_eq!(
            after.processors.len(),
            1,
            "and the counters it dates are still there"
        );
    }

    /// The other half: a rebuilt table's counters start at zero and must be
    /// dated from zero, or a graph spans a reset without saying so. "Rebuilt"
    /// means the rows were invalidated — the columns or the key changed.
    #[test]
    fn a_structural_change_resets_created_at_with_the_counters() {
        let registry = LookupTable::new();
        let conf = hosts(100);
        let table = registry.insert(&conf).unwrap();
        table.as_mutable().unwrap().write_as(NODE());
        let before = only(&registry).created_at;

        let mut changed = conf.clone();
        changed.source = match changed.source {
            cfg::Source::Processor(mut p) => {
                p.columns.push(cfg::ColumnSpec {
                    name: "region".into(),
                    value_type: cfg::ValueType::String,
                });
                cfg::Source::Processor(p)
            }
            other => other,
        };
        registry.replace(&[changed]).unwrap();

        let after = only(&registry);
        assert!(
            after.created_at >= before,
            "a new table is never older than the one it replaced"
        );
        assert!(
            after.processors.is_empty(),
            "a structural change is a new table, and starts from zero"
        );
    }

    // ---- the filter ----

    /// "Not configured here" is a different answer from "here, but filtered
    /// out", and on a backend node it is the only answer available.
    #[test]
    fn an_id_filter_reports_ids_this_node_holds_no_table_for() {
        let registry = LookupTable::new();
        let conf = hosts(100);
        registry.insert(&conf).unwrap();
        let absent = Uuid::now_v7();

        let answer = local_status(NODE(), &registry, &StatusFilter::by_ids(&[conf.id, absent]));
        assert_eq!(answer.tables.len(), 1);
        assert_eq!(answer.tables[0].id, conf.id);
        assert_eq!(answer.unknown_ids, [absent]);
        assert!(answer.error.is_none(), "a missing id is not a node failure");
    }

    /// Collapsing empty into absent would turn a request for nothing into a
    /// request for everything.
    #[test]
    fn an_empty_id_filter_matches_nothing_and_an_absent_one_matches_everything() {
        let registry = LookupTable::new();
        registry.insert(&hosts(100)).unwrap();

        assert!(
            local_status(NODE(), &registry, &StatusFilter::by_ids(&[]))
                .tables
                .is_empty()
        );
        assert_eq!(
            local_status(NODE(), &registry, &StatusFilter::default())
                .tables
                .len(),
            1
        );
    }

    // ---- shape and wire ----

    /// Catches a rename typo, a missing nested `Deserialize`, and the real
    /// target: a field wired into `Serialize` only. This is why the DTOs derive
    /// `Deserialize` when nothing deserializes them yet.
    #[tokio::test]
    async fn the_status_dto_round_trips_through_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        // one skipped row, so `SkipCounts` and `first_error` are both populated
        write(&path, "host,site\na,jkt\nb\n");

        let cache = Cache::new();
        let file = file_table(
            "afile",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            // tiny, so the `check_once` below actually re-stats the source
            Duration::from_nanos(1),
        );
        loader::init(std::slice::from_ref(&file), &cache)
            .await
            .unwrap();
        // and a failed reload on top, so `last_error` is populated too
        write(&path, "other,site\na,jkt\n");
        loader::check_once(&cache).await;

        let processor = processor_table(
            "bprocessor",
            hash_key(&["host"]),
            &[
                ("host", cfg::ValueType::String),
                ("site", cfg::ValueType::String),
            ],
            50,
        );
        let table = cache.lookup_table.insert(&processor).unwrap();
        {
            let write = table.as_mutable().unwrap().write_as(NODE());
            let _ = write
                .upsert_hash(&[CellRef::String("a"), CellRef::String("jkt")])
                .unwrap();
        }

        let answer = local_status(NODE(), &cache.lookup_table, &StatusFilter::default());
        assert_eq!(answer.tables.len(), 2);
        let immutable = &answer.tables[0];
        assert!(immutable.load.as_ref().unwrap().last_error.is_some());
        assert!(immutable.load.as_ref().unwrap().next_check_at.is_some());
        assert!(
            answer.tables[1]
                .capacity
                .as_ref()
                .unwrap()
                .ttl_seconds
                .is_some()
        );

        let json = serde_json::to_string(&answer).unwrap();
        let back: NodeLookupTableStatus = serde_json::from_str(&json).unwrap();

        // Millis on the wire, so a microsecond load comes back as zero. Round
        // the original the same way rather than weakening the comparison.
        let mut expected = answer.clone();
        for table in &mut expected.tables {
            if let Some(load) = table.load.as_mut() {
                load.load_duration = to_millis(load.load_duration);
                load.generation_stats.elapsed = to_millis(load.generation_stats.elapsed);
            }
        }
        assert_eq!(back, expected, "every field must survive both directions");
        assert!(
            answer.tables[0]
                .load
                .as_ref()
                .unwrap()
                .generation_stats
                .elapsed
                >= expected.tables[0]
                    .load
                    .as_ref()
                    .unwrap()
                    .generation_stats
                    .elapsed,
            "rounding only ever loses sub-millisecond precision"
        );
    }

    fn to_millis(d: Duration) -> Duration {
        Duration::from_millis(d.as_millis() as u64)
    }

    /// A log line, a Prometheus `reason` label and a JSON key must name the same
    /// rejection, or three views of one incident disagree.
    #[test]
    fn the_json_skip_reason_keys_match_the_prometheus_labels() {
        let counts = super::super::immutable::SkipCounts {
            arity: 1,
            bad_cell: 2,
            bad_bound: 3,
            null_key: 4,
            duplicate_key: 5,
            overlap: 6,
            empty_range: 7,
            mismatch: 8,
        };
        let json: serde_json::Value = serde_json::to_value(counts).unwrap();
        let object = json.as_object().unwrap();
        // the spellings `SkipCounts::summary` and `visit::SKIP_REASONS` use
        for reason in [
            "field_count",
            "cast",
            "range_bound",
            "null_key",
            "duplicate_key",
            "overlap",
            "empty_range",
            "family_mismatch",
        ] {
            assert!(object.contains_key(reason), "missing `{reason}`");
        }
        assert_eq!(object.len(), 8, "and nothing else");
    }

    /// The flat-with-`Option` shape can represent an impossible pairing; this is
    /// the discipline that keeps it from happening.
    #[tokio::test]
    async fn each_kind_reports_its_own_half_and_not_the_other() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site\na,jkt\n");

        let cache = Cache::new();
        let file = file_table(
            "afile",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            Duration::from_nanos(1),
        );
        loader::init(std::slice::from_ref(&file), &cache)
            .await
            .unwrap();
        cache.lookup_table.insert(&hosts(100)).unwrap();

        for table in all(&cache.lookup_table) {
            match table.kind {
                TableKind::Immutable => {
                    assert!(table.load.is_some());
                    assert!(table.capacity.is_none(), "no cap to report");
                }
                TableKind::Mutable => {
                    assert!(table.capacity.is_some());
                    assert!(table.load.is_none(), "no source to load from");
                }
            }
        }
    }

    /// A list that reorders between polls is unusable, and names are not unique.
    #[test]
    fn tables_are_sorted_by_name_then_id() {
        let registry = LookupTable::new();
        let mut ids = Vec::new();
        for name in ["b", "a", "a"] {
            let mut conf = hosts(100);
            conf.name = name.to_string();
            ids.push(conf.id);
            registry.insert(&conf).unwrap();
        }
        let mut expected_a = vec![ids[1], ids[2]];
        expected_a.sort_unstable();

        let tables = all(&registry);
        let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["a", "a", "b"]);
        assert_eq!(
            vec![tables[0].id, tables[1].id],
            expected_a,
            "the duplicate name is broken by id, so the order is stable"
        );
    }

    // ---- end to end ----

    /// The executable form of the backend-node TODO: an API node has no lookup
    /// tables, and that is not a failure to report.
    #[test]
    fn an_empty_registry_answers_with_no_tables_rather_than_an_error() {
        let registry = LookupTable::new();
        let node = NODE();

        let answer = local_status(node, &registry, &StatusFilter::default());
        assert!(answer.tables.is_empty());
        assert!(
            answer.error.is_none(),
            "nothing failed; there is nothing here"
        );
        assert_eq!(answer.node_id, node, "the node still names itself");
        assert!(answer.at.is_some(), "and still dates its answer");
    }

    /// Proves the whole producer against a table built through the real write
    /// API, so no assertion here can pass against a shape nothing produces.
    #[test]
    fn the_local_answer_describes_a_table_written_through_the_real_api() {
        let registry = LookupTable::new();
        let conf = hosts(50);
        let table = registry.insert(&conf).unwrap();
        let node = NODE();
        {
            let write = table.as_mutable().unwrap().write_as(node);
            let _ = write
                .upsert_hash(&[CellRef::String("a"), CellRef::String("jkt")])
                .unwrap();
        }

        let status = only(&registry);
        assert_eq!(status.id, conf.id);
        assert_eq!(status.name, "hosts");
        assert_eq!(status.kind, TableKind::Mutable);
        assert_eq!(status.health, TableHealth::Ready);
        assert_eq!(status.rows, 1);
        assert_eq!(status.usage.writes_inserted, 1);
        assert_eq!(status.processors.len(), 1);
        assert_eq!(status.processors[0].id, node);

        let capacity = status.capacity.unwrap();
        assert_eq!(capacity.max_rows, 50);
        assert_eq!(capacity.rejected, 0);
        assert_eq!(capacity.ttl_seconds, Some(60));
        assert!(capacity.shards >= 1, "at least one shard always exists");
    }

    /// An immutable table interns every distinct string-column value, so it
    /// holds strings too. Reporting zero there hid a file-backed table's only
    /// cost proxy.
    #[tokio::test]
    async fn a_loaded_immutable_table_reports_the_strings_it_interned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        // three distinct sites across four rows: the interner holds each once
        write(&path, "host,site\na,jkt\nb,bdg\nc,jkt\nd,surabaya\n");

        let cache = Cache::new();
        let conf = file_table(
            "hosts",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            Duration::from_nanos(1),
        );
        loader::init(std::slice::from_ref(&conf), &cache)
            .await
            .unwrap();

        let status = only(&cache.lookup_table);
        assert_eq!(status.rows, 4);
        assert!(
            status.pooled_strings > 0,
            "a file-backed table's interner is its nearest cost proxy, not zero"
        );
        assert_eq!(
            status.pooled_strings, 7,
            "four hosts plus three distinct sites: `jkt` repeats but interns once"
        );
        assert!(
            status.capacity.is_none(),
            "and it is reported at the top level, not in the mutable-only half"
        );
    }

    /// Pins that the field is shared rather than one of the disjoint halves: a
    /// reader must not have to branch on `kind` to find it.
    #[test]
    fn both_kinds_report_pooled_strings_at_the_top_level() {
        let registry = LookupTable::new();
        let table = registry.insert(&hosts(100)).unwrap();
        let _ = table
            .as_mutable()
            .unwrap()
            .table()
            .upsert_hash(&[
                CellRef::String("a"),
                // longer than the inline threshold, so it reaches the pool
                CellRef::String("a-site-name-far-too-long-to-live-inline"),
            ])
            .unwrap();

        let status = only(&registry);
        assert_eq!(status.kind, TableKind::Mutable);
        assert!(status.pooled_strings > 0, "the long value was pooled");
        // and the compaction counter stays on the mutable half, because only
        // these pools are swept
        assert_eq!(status.capacity.unwrap().strings_released, 0);
    }

    /// Zero disables expiry, so it must read as absence: "rows never expire" is
    /// not "rows expire instantly".
    #[test]
    fn a_disabled_ttl_reports_no_expiry_rather_than_zero() {
        let registry = LookupTable::new();
        let mut conf = hosts(100);
        conf.source = match conf.source {
            cfg::Source::Processor(mut p) => {
                p.ttl = Duration::ZERO;
                cfg::Source::Processor(p)
            }
            other => other,
        };
        registry.insert(&conf).unwrap();

        assert_eq!(only(&registry).capacity.unwrap().ttl_seconds, None);
    }

    /// A range point probe is the other half of the read API; the counters must
    /// attribute it the same way a hash probe is attributed.
    #[test]
    fn a_range_probe_is_counted_like_any_other_lookup() {
        let registry = LookupTable::new();
        let conf = processor_table(
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
            100,
        );
        let table = registry.insert(&conf).unwrap();
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

        {
            let read = table.read_as(NODE());
            assert!(
                read.lookup(Lookup::Range {
                    partition: &[],
                    point: RangePoint::Integer(150),
                })
                .is_some()
            );
        }

        let status = only(&registry);
        assert_eq!(status.usage.lookups_hit, 1);
        assert_eq!(status.health, TableHealth::Ready);
    }
}
