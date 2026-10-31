// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod visit;

/// Reused rather than duplicated: the loader already counts row rejections by
/// reason, and a second copy of the same seven fields would only drift.
pub use crate::cache::lookup_table::immutable::SkipCounts;

use arc_swap::ArcSwap;
use rustc_hash::FxHashMap as HashMap;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use utoipa::ToSchema;
use uuid::Uuid;

pub static INTERNAL_METRICS: LazyLock<Arc<InternalMetrics>> =
    LazyLock::new(|| Arc::new(InternalMetrics::default()));

pub struct InternalMetrics {
    pub sink_channel: Arc<ArcSwap<(Instant, Channel)>>,
    pub processor_channel: Arc<ArcSwap<(Instant, Channel)>>,
    pub forwarder_channel: Arc<ArcSwap<(Instant, Channel)>>,
    pub sink: Arc<ArcSwap<(Instant, Vec<Sink>)>>,
    pub listener: Arc<ArcSwap<(Instant, Vec<Listener>)>>,
    pub monitor: Arc<ArcSwap<(Instant, Monitor)>>,
    pub processor: Arc<ArcSwap<(Instant, Processor)>>,
    pub forwarder: Arc<ArcSwap<(Instant, Forwarder)>>,
    pub forwarder_server: Arc<ArcSwap<(Instant, ForwarderServer)>>,
    pub lookup_table: Arc<ArcSwap<(Instant, Vec<LookupTable>)>>,
}

impl Default for InternalMetrics {
    fn default() -> Self {
        Self {
            sink_channel: Arc::new(ArcSwap::from_pointee((Instant::now(), Channel::default()))),
            processor_channel: Arc::new(ArcSwap::from_pointee((
                Instant::now(),
                Channel::default(),
            ))),
            forwarder_channel: Arc::new(ArcSwap::from_pointee((
                Instant::now(),
                Channel::default(),
            ))),
            sink: Arc::new(ArcSwap::from_pointee((Instant::now(), Vec::new()))),
            listener: Arc::new(ArcSwap::from_pointee((Instant::now(), Vec::new()))),
            monitor: Arc::new(ArcSwap::from_pointee((Instant::now(), Monitor::default()))),
            processor: Arc::new(ArcSwap::from_pointee((
                Instant::now(),
                Processor::default(),
            ))),
            forwarder: Arc::new(ArcSwap::from_pointee((
                Instant::now(),
                Forwarder::default(),
            ))),
            forwarder_server: Arc::new(ArcSwap::from_pointee((
                Instant::now(),
                ForwarderServer::default(),
            ))),
            lookup_table: Arc::new(ArcSwap::from_pointee((Instant::now(), Vec::new()))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    Memory,
    Disk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelState {
    pub kind: ChannelKind,
    pub capacity: u64,
    pub length: u64,
}

#[derive(Default)]
pub struct Channel {
    pub channels: HashMap<String, ChannelState>,
    pub total_messages: usize,
    pub total_batches: usize,
    pub total_time_spent: Duration,
    pub messages_sent: HashMap<String, usize>,
    pub messages_dropped: HashMap<String, usize>,
    pub batches_sent: HashMap<String, usize>,
    pub batches_dropped: HashMap<String, usize>,
    pub messages_lost: HashMap<String, usize>,
    pub batches_lost: HashMap<String, usize>,
    pub send_wait_time: HashMap<String, Duration>,
}

#[derive(Default)]
pub struct Sink {
    pub id: Uuid,
    pub name: String,
    pub total_workers: usize,
    pub total_time_spent: Duration,
    pub engine_time_spent: Duration,
    pub receive_wait_time: Duration,
    pub send_wait_time: Duration,
    pub total_records: u64,
    pub total_records_sent: u64,
    pub total_records_filtered: u64,
    pub total_records_errored: u64,
    /// Records sent again after a failed attempt, once per attempt.
    pub total_records_retried: u64,
    /// Records the workers hold for their next attempt.
    pub records_pending_retry: u64,
    /// One per destination endpoint, for sinks that track their health.
    pub endpoints: Vec<SinkEndpoint>,
}

pub struct SinkEndpoint {
    pub endpoint: String,
    pub up: bool,
}

#[derive(Default)]
pub struct Listener {
    pub id: Uuid,
    pub name: String,
    pub total_workers: usize,
    pub total_time_spent: Duration,
    pub engine_time_spent: Duration,
    pub receive_wait_time: Duration,
    pub send_wait_time: Duration,
    pub total_bytes: u64,
    pub total_bytes_succeeded: u64,
    pub total_bytes_dropped: u64,
    pub total_bytes_errored: u64,
    pub total_items: u64,
    pub total_items_succeeded: u64,
    pub total_items_dropped: u64,
    pub total_items_errored: u64,
    pub total_records: u64,
    pub total_records_sent: u64,
    pub total_records_forwarded: u64,
    pub total_records_forward_failed: u64,
    pub total_records_annotate_failed: u64,
    pub total_records_dropped: u64,
    pub total_records_errored: u64,
}

#[derive(Default)]
pub struct Monitor {
    pub total_workers: usize,
    pub total_time_spent: Duration,
    pub engine_time_spent: Duration,
    pub send_wait_time: Duration,
    pub receive_wait_time: Duration,
    pub total_items: u64,
    pub total_items_dropped: u64,
    pub total_items_errored: u64,
    pub total_items_retry_dropped: u64,
}

#[derive(Default)]
pub struct ProcessorNode {
    pub id: Uuid,
    pub total_time_spent: Duration,
    pub engine_time_spent: Duration,
    pub send_wait_time: Duration,
    pub total_records: u64,
    pub total_records_filtered: u64,
    pub total_records_errored: u64,
}

#[derive(Default)]
pub struct ProcessorEdge {
    pub source: Uuid,
    pub target: Uuid,
    pub target_is_sink: bool,
    pub total_records: u64,
}

#[derive(Default)]
pub struct Processor {
    pub total_workers: usize,
    pub total_time_spent: Duration,
    pub engine_time_spent: Duration,
    pub send_wait_time: Duration,
    pub receive_wait_time: Duration,
    pub total_records: u64,
    pub total_records_sent: u64,
    pub total_records_filtered: u64,
    pub total_records_errored: u64,
    pub total_records_unrouted: u64,
    pub nodes: Vec<ProcessorNode>,
    pub edges: Vec<ProcessorEdge>,
}

/// Per-peer counters shared between whichever tasks talk to that node; the
/// snapshot side is [`ForwarderNode`].
#[derive(Default)]
pub struct NodeMetrics {
    pub total_connections: AtomicUsize,
    pub total_time_spent: AtomicU64,
    pub engine_time_spent: AtomicU64,
    pub send_wait_time: AtomicU64,
    pub receive_wait_time: AtomicU64,
    pub total_records: AtomicU64,
    pub total_records_sent: AtomicU64,
    pub total_records_dropped: AtomicU64,
    pub total_records_errored: AtomicU64,
    pub total_bytes: AtomicU64,
    pub total_bytes_sent: AtomicU64,
    pub total_bytes_dropped: AtomicU64,
    pub total_bytes_errored: AtomicU64,
}

impl NodeMetrics {
    pub fn get(&self, id: Uuid) -> ForwarderNode {
        ForwarderNode {
            id,
            total_connections: self.total_connections.load(Ordering::Relaxed),
            total_time_spent: Duration::from_nanos(self.total_time_spent.load(Ordering::Relaxed)),
            engine_time_spent: Duration::from_nanos(self.engine_time_spent.load(Ordering::Relaxed)),
            send_wait_time: Duration::from_nanos(self.send_wait_time.load(Ordering::Relaxed)),
            receive_wait_time: Duration::from_nanos(self.receive_wait_time.load(Ordering::Relaxed)),
            total_records: self.total_records.load(Ordering::Relaxed),
            total_records_sent: self.total_records_sent.load(Ordering::Relaxed),
            total_records_dropped: self.total_records_dropped.load(Ordering::Relaxed),
            total_records_errored: self.total_records_errored.load(Ordering::Relaxed),
            total_bytes: self.total_bytes.load(Ordering::Relaxed),
            total_bytes_sent: self.total_bytes_sent.load(Ordering::Relaxed),
            total_bytes_dropped: self.total_bytes_dropped.load(Ordering::Relaxed),
            total_bytes_errored: self.total_bytes_errored.load(Ordering::Relaxed),
        }
    }
}

#[derive(Default)]
pub struct ForwarderNode {
    pub id: Uuid,
    pub total_connections: usize,
    pub total_time_spent: Duration,
    pub engine_time_spent: Duration,
    pub send_wait_time: Duration,
    pub receive_wait_time: Duration,
    pub total_records: u64,
    pub total_records_sent: u64,
    pub total_records_dropped: u64,
    pub total_records_errored: u64,
    pub total_bytes: u64,
    pub total_bytes_sent: u64,
    pub total_bytes_dropped: u64,
    pub total_bytes_errored: u64,
}

#[derive(Default)]
pub struct Forwarder {
    pub total_time_spent: Duration,
    pub engine_time_spent: Duration,
    pub send_wait_time: Duration,
    pub receive_wait_time: Duration,
    pub total_records: u64,
    pub total_records_sent: u64,
    pub total_records_dropped: u64,
    pub total_records_errored: u64,
    pub nodes: Vec<ForwarderNode>,
}

#[derive(Default)]
pub struct ForwarderServer {
    pub engine_time_spent: Duration,
    pub send_wait_time: Duration,
    pub total_records: u64,
    pub total_records_sent: u64,
    pub total_records_errored: u64,
    pub total_bytes: u64,
}

/// One lookup table: occupancy, load history, and one entry per processor that
/// reads or writes it.
///
/// File-backed (`kind == "immutable"`) and processor-fed (`"mutable"`) tables
/// use disjoint halves of this struct, each leaving the other's fields zero.
/// The renderers filter by `kind` rather than emitting those zeros.
#[derive(Default)]
pub struct LookupTable {
    pub id: Uuid,
    pub name: String,
    pub kind: &'static str,

    // occupancy
    pub rows: u64,
    pub index_entries: u64,
    /// hard row cap; mutable only
    pub max_rows: u64,
    /// distinct strings held: an immutable table's per-generation interner or a
    /// mutable table's shared pools — different populations, so compare a table
    /// only with itself over time
    pub pooled_strings: u64,
    /// Current generation; zero before an immutable table first loads.
    pub generation: u64,

    // load health, immutable only
    /// Unix seconds of the last successful load; zero if none has succeeded.
    pub last_success_timestamp: f64,
    /// Whether the most recent load attempt failed, even if old data remains.
    pub load_error: bool,

    // the generation currently in memory, immutable only
    pub generation_rows_read: u64,
    pub generation_rows_skipped: SkipCounts,
    pub generation_load_duration: Duration,

    // cumulative
    pub loads: u64,
    pub load_failures: u64,
    pub load_duration: Duration,
    pub rows_read: u64,
    pub rows_skipped: SkipCounts,
    /// writes turned away by `max_rows`; mutable only
    pub rows_rejected: u64,
    /// rows dropped by the TTL sweep; mutable only
    pub rows_expired: u64,
    /// pool entries released by compaction; mutable only
    pub strings_released: u64,

    pub processors: Vec<LookupTableProcessor>,
}

/// One processor's use of one table; `id` is the processor node uuid, 1:1 with
/// a processor.
///
/// `lookups == lookups_hit + lookups_missed + lookups_errored`, the
/// total-plus-disjoint-outcomes shape every other section uses. The write
/// fields stay zero for an immutable table, which cannot be written.
///
/// Derives serde and `ToSchema` because the status API reports these rows
/// verbatim: an eleven-field duplicate would drift, the reason [`SkipCounts`]
/// is re-exported above rather than re-declared.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct LookupTableProcessor {
    pub id: Uuid,
    pub lookups: u64,
    pub lookups_hit: u64,
    pub lookups_missed: u64,
    /// the probe key could not be built from the record at all
    pub lookups_errored: u64,
    pub writes: u64,
    pub writes_inserted: u64,
    pub writes_updated: u64,
    /// would have exceeded `max_rows`
    pub writes_rejected: u64,
    pub writes_errored: u64,
    pub removes: u64,
}
