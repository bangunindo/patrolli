// SPDX-FileCopyrightText: 2025 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

mod dummy;
mod prometheus;
mod snmp;

use crate::cache::item::{Failure, FailureKind, Stats};
use crate::chan::SenderChannel;
use crate::common;
use crate::common::internal_metrics::{INTERNAL_METRICS, Monitor as MonitorMetrics};
use crate::config::endpoint::{Endpoint, EndpointKind};
use crate::config::host::Host;
use crate::config::{Batch, MonitorConfig, item};
use crate::payload::Scope;
use crate::{cache, config, payload};
use async_trait::async_trait;
use flume::{Receiver, Sender, bounded};
use futures_util::StreamExt;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use smol_str::SmolStr;
use std::fmt::Display;
use std::mem::{Discriminant, discriminant};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant, sleep};
use tokio_util::sync::CancellationToken;
use tokio_util::time::{DelayQueue, delay_queue};
use tracing::{Span, debug, error, info, warn};
use uuid::Uuid;

static MAX_ITEMS_PER_WORKER: usize = 10;
static MAX_RETRIES: u32 = 3;
static RETRY_INTERVAL: Duration = Duration::from_secs(30);
enum MonitorItem {
    Single(cache::item::Entry),
    Batch(Vec<cache::item::Entry>),
}

impl MonitorItem {
    fn timeout(&self) -> Duration {
        match self {
            MonitorItem::Single(unit) => unit.item.timeout,
            MonitorItem::Batch(units) => units
                .iter()
                .map(|unit| unit.item.timeout)
                .max()
                .unwrap_or(Duration::ZERO),
        }
    }

    fn first(&self) -> &Arc<item::Item> {
        match self {
            MonitorItem::Single(unit) => &unit.item,
            MonitorItem::Batch(units) => &units[0].item,
        }
    }

    /// Every item this check covers, so the single and batch cases share one
    /// accounting path instead of duplicating it per outcome.
    fn units(&self) -> &[cache::item::Entry] {
        match self {
            MonitorItem::Single(unit) => std::slice::from_ref(unit),
            MonitorItem::Batch(units) => units,
        }
    }
}

/// Rows produced per item id, read off each record's resource.
///
/// Only needed for batch checks: they share one result across their members, so
/// the only way to attribute rows is to look at which item each row was built
/// for. A single item owns everything its collector produced, streamed rows
/// included, and uses [`Collected::records`] instead.
fn rows_by_item(record: &MonitorRecord) -> FxHashMap<Uuid, u64> {
    let mut counts: FxHashMap<Uuid, u64> = FxHashMap::default();
    match record {
        MonitorRecord::Null => {}
        MonitorRecord::SingleLog(r) => *counts.entry(r.resource.item_id).or_default() += 1,
        MonitorRecord::SingleMetric(r) => *counts.entry(r.resource.item_id).or_default() += 1,
        MonitorRecord::BatchLog(rs) => {
            for r in rs {
                *counts.entry(r.resource.item_id).or_default() += 1;
            }
        }
        MonitorRecord::BatchMetric(rs) => {
            for r in rs {
                *counts.entry(r.resource.item_id).or_default() += 1;
            }
        }
    }
    counts
}

struct MonitorLog {
    row: payload::LogRowOwned,
    resource: payload::Resource,
    kind: item::ItemKind,
    processors: SmallVec<[Uuid; 4]>,
    /// The producing item's stats, so the batcher can attribute a dropped row
    /// back to the item that collected it.
    stats: Arc<Stats>,
}

struct MonitorMetric {
    row: payload::MetricRowOwned,
    resource: payload::Resource,
    kind: item::ItemKind,
    processors: SmallVec<[Uuid; 4]>,
    stats: Arc<Stats>,
}

enum MonitorRecord {
    Null,
    SingleLog(Box<MonitorLog>),
    BatchLog(Vec<MonitorLog>),
    SingleMetric(Box<MonitorMetric>),
    BatchMetric(Vec<MonitorMetric>),
}

/// What a successful check produced.
///
/// `records` counts **rows** — one per [`MonitorLog`]/[`MonitorMetric`] — and has
/// to be reported alongside `record` rather than derived from it: the streaming
/// collectors (SNMP walk in table mode, Prometheus with `stream = true`) push
/// their rows straight down `sender` and return [`MonitorRecord::Null`], so
/// counting the return value alone would report zero for exactly the items that
/// produce the most data.
///
/// `failed` lets a batch check succeed for some members and fail others: an
/// SNMP GET whose request went through but whose OID the agent doesn't have.
/// Those members are recorded as failed; the rest as succeeded.
struct Collected {
    record: MonitorRecord,
    records: u64,
    failed: FxHashMap<Uuid, Error>,
}

impl Collected {
    fn new(record: MonitorRecord, records: u64) -> Self {
        Self::partial(record, records, FxHashMap::default())
    }

    fn partial(record: MonitorRecord, records: u64, failed: FxHashMap<Uuid, Error>) -> Self {
        Self {
            record,
            records,
            failed,
        }
    }

    /// A check that succeeded and produced nothing.
    fn empty() -> Self {
        Self::new(MonitorRecord::Null, 0)
    }
}

/// The outcome of one check attempt.
///
/// `Cancelled` is deliberately not an [`Error`]: shutdown used to arrive as
/// `Unrecoverable("cancelled")`, which would now write a failure to every
/// in-flight item on every deploy and poison its activity map.
enum Attempt {
    Done(Collected),
    Failed(Error),
    Cancelled,
}

/// A failed check, carrying both the classification the stats page displays and
/// the retry decision derived from it.
///
/// One struct rather than a `Recoverable`/`Unrecoverable` pair: retryability is a
/// function of the kind, and keeping them as separate variants let the two
/// disagree at each of the ~40 construction sites.
struct Error {
    kind: FailureKind,
    msg: String,
}

impl Error {
    fn new(kind: FailureKind, msg: impl Into<String>) -> Self {
        Self {
            kind,
            msg: msg.into(),
        }
    }

    /// Whether waiting `RETRY_INTERVAL` and trying again could plausibly help.
    ///
    /// `Config` and `Internal` need a human. `Auth` and `NotSupported` are the
    /// judgement calls: a rejected community or an OID outside the agent's VACM
    /// view will not start working within 90 seconds, so retrying them only
    /// delays the item's next real check and hides the actionable state behind
    /// `Retrying`.
    fn retryable(&self) -> bool {
        !matches!(
            self.kind,
            FailureKind::Config
                | FailureKind::Internal
                | FailureKind::Auth
                | FailureKind::NotSupported
        )
    }

    fn timeout(msg: impl Into<String>) -> Self {
        Self::new(FailureKind::Timeout, msg)
    }

    fn unreachable(msg: impl Into<String>) -> Self {
        Self::new(FailureKind::Unreachable, msg)
    }

    fn auth(msg: impl Into<String>) -> Self {
        Self::new(FailureKind::Auth, msg)
    }

    fn not_supported(msg: impl Into<String>) -> Self {
        Self::new(FailureKind::NotSupported, msg)
    }

    fn target(msg: impl Into<String>) -> Self {
        Self::new(FailureKind::TargetError, msg)
    }

    fn malformed(msg: impl Into<String>) -> Self {
        Self::new(FailureKind::Malformed, msg)
    }

    fn config(msg: impl Into<String>) -> Self {
        Self::new(FailureKind::Config, msg)
    }

    /// Rows were collected but couldn't be handed downstream.
    fn dropped(msg: impl Into<String>) -> Self {
        Self::new(FailureKind::Dropped, msg)
    }

    fn internal(msg: impl Into<String>) -> Self {
        Self::new(FailureKind::Internal, msg)
    }

    /// Build the [`Failure`] recorded against the item's stats.
    fn to_failure(&self) -> Failure {
        Failure::new(common::now().fixed_offset(), self.kind, &self.msg, None)
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} error: {}", self.kind, self.msg)
    }
}

fn create_resource(
    host: &Host,
    endpoint: Option<&Endpoint>,
    item: &item::Item,
) -> payload::Resource {
    payload::Resource {
        host_id: host.id,
        host_name: host.name.clone(),
        host_timezone: host.timezone.clone(),
        endpoint_id: endpoint.map(|e| e.id),
        endpoint_name: endpoint.map(|e| e.name.clone()),
        endpoint_type: endpoint.map(|e| SmolStr::new_static(EndpointKind::from(&e.config).into())),
        item_id: item.id,
        item_name: item.name.clone(),
    }
}

#[derive(Default)]
struct InternalMetrics {
    total_workers: AtomicUsize,
    total_time_spent: AtomicU64,
    engine_time_spent: AtomicU64,
    send_wait_time: AtomicU64,
    receive_wait_time: AtomicU64,
    total_items: AtomicU64,
    total_items_dropped: AtomicU64,
    total_items_errored: AtomicU64,
    total_items_retry_dropped: AtomicU64,
}

impl InternalMetrics {
    fn get(&self) -> MonitorMetrics {
        MonitorMetrics {
            total_workers: self.total_workers.load(Ordering::Relaxed),
            total_time_spent: Duration::from_nanos(self.total_time_spent.load(Ordering::Relaxed)),
            engine_time_spent: Duration::from_nanos(self.engine_time_spent.load(Ordering::Relaxed)),
            receive_wait_time: Duration::from_nanos(self.receive_wait_time.load(Ordering::Relaxed)),
            send_wait_time: Duration::from_nanos(self.send_wait_time.load(Ordering::Relaxed)),
            total_items: self.total_items.load(Ordering::Relaxed),
            total_items_dropped: self.total_items_dropped.load(Ordering::Relaxed),
            total_items_errored: self.total_items_errored.load(Ordering::Relaxed),
            total_items_retry_dropped: self.total_items_retry_dropped.load(Ordering::Relaxed),
        }
    }
}

struct Worker {
    idx: usize,
    internal_metrics: Arc<InternalMetrics>,
    token: CancellationToken,
    monitor_channel: Receiver<MonitorItem>,
    sender: Sender<MonitorRecord>,
    config_cache: cache::Cache,
}

impl Worker {
    async fn execute(&self, item: &MonitorItem) -> Result<Collected, Error> {
        let host = self
            .config_cache
            .host
            .read()
            .get(&item.first().host_id)
            .cloned()
            .ok_or_else(|| Error::config("host not found"))?;
        let endpoint = item
            .first()
            .endpoint_id
            .and_then(|id| self.config_cache.endpoint.read().get(&id).cloned());
        match item {
            MonitorItem::Single(unit) => match &unit.item.config {
                item::Config::SnmpWalk(_) => {
                    let endpoint = endpoint.ok_or_else(|| Error::config("endpoint not found"))?;
                    snmp::process_snmp_walk(&unit.item, &endpoint, &host, &unit.stats, &self.sender)
                        .await
                }
                item::Config::Dummy(_) => dummy::generate_data(&unit.item, &host, &unit.stats),
                item::Config::PrometheusScrape(_) => {
                    let endpoint = endpoint.ok_or_else(|| Error::config("endpoint not found"))?;
                    prometheus::process_prometheus_scrape(
                        &unit.item,
                        &endpoint,
                        &host,
                        &unit.stats,
                        &self.sender,
                    )
                    .await
                }
                _ => {
                    error!("internal error, incorrect item type");
                    self.internal_metrics
                        .total_items_dropped
                        .fetch_add(1, Ordering::Relaxed);
                    Err(Error::internal("incorrect item type"))
                }
            },
            MonitorItem::Batch(units) => match &units[0].item.config {
                item::Config::Snmp(_) => {
                    let endpoint = endpoint.ok_or_else(|| Error::config("endpoint not found"))?;
                    snmp::process_snmp(units, &endpoint, &host, item.timeout()).await
                }
                _ => {
                    error!("internal error, incorrect item type");
                    self.internal_metrics
                        .total_items_dropped
                        .fetch_add(units.len() as u64, Ordering::Relaxed);
                    Err(Error::internal("incorrect item type"))
                }
            },
        }
    }

    // Batch items share one host but not one item, so item identity is only
    // recorded on the span for single items; batch events carry `item_id`.
    #[tracing::instrument(
        skip_all,
        fields(
            host_id = %item.first().host_id,
            item_id = tracing::field::Empty,
            item = tracing::field::Empty,
            items = item.units().len(),
        )
    )]
    async fn process_item(&mut self, item: MonitorItem) {
        if let MonitorItem::Single(unit) = &item {
            let span = Span::current();
            span.record("item_id", tracing::field::display(unit.item.id));
            span.record("item", tracing::field::display(&unit.item.name));
        }
        let count = item.units().len() as u64;
        self.internal_metrics
            .total_items
            .fetch_add(count, Ordering::Relaxed);

        // Lag is queue wait: the item's due time against when a worker actually
        // picked it up. Both are monotonic instants, so it survives a wall-clock
        // adjustment; the floor is the 100 ms scheduler tick plus time spent
        // queued on `monitor_channel`.
        let started = Instant::now();
        let attempt_ms = common::now().timestamp_millis();
        for unit in item.units() {
            let lag = started.saturating_duration_since(unit.monitor.next_check);
            unit.stats.record_attempt(attempt_ms, lag);
        }

        let attempt = tokio::select! {
            biased;

            _ = self.token.cancelled() => Attempt::Cancelled,
            _ = sleep(item.timeout()) => Attempt::Failed(Error::timeout("check deadline elapsed")),
            result = self.execute(&item) => match result {
                Ok(collected) => Attempt::Done(collected),
                Err(e) => Attempt::Failed(e),
            },
        };
        let elapsed = started.elapsed();

        // Monitors are updated and stats recorded first, then requeued in one
        // pass, so the cache write lock is held only for the requeues.
        let mut pending = Vec::with_capacity(item.units().len());

        match attempt {
            Attempt::Done(Collected {
                record,
                records,
                failed,
            }) => {
                // Read the per-item counts before the record moves downstream.
                let per_item = match &item {
                    MonitorItem::Single(_) => None,
                    MonitorItem::Batch(_) => Some(rows_by_item(&record)),
                };
                // Raced against the token: a sink that stopped consuming
                // backs up the processors, then the batcher, then this send,
                // and shutdown waits for this worker to return.
                let handed_over = matches!(record, MonitorRecord::Null)
                    || tokio::select! {
                        biased;

                        res = self.sender.send_async(record) => {
                            if res.is_err() {
                                error!("monitor batcher channel closed");
                            }
                            res.unwrap();
                            true
                        }
                        _ = self.token.cancelled() => false,
                    };
                if !handed_over {
                    // Shutdown, as in the `Cancelled` arm: requeued untouched,
                    // nothing recorded.
                    debug!("monitor record dropped during shutdown");
                    pending.extend(item.units().iter().map(|unit| unit.monitor));
                }

                let now_ms = common::now().timestamp_millis();
                for unit in item.units().iter().filter(|_| handed_over) {
                    if let Some(e) = failed.get(&unit.item.id) {
                        self.internal_metrics
                            .total_items_errored
                            .fetch_add(1, Ordering::Relaxed);
                        pending.push(self.fail_unit(unit, e, &e.to_failure(), elapsed));
                        continue;
                    }
                    let rows = match &per_item {
                        None => records,
                        Some(counts) => counts.get(&unit.item.id).copied().unwrap_or(0),
                    };
                    if let Some(t) = unit.stats.record_success(now_ms, elapsed, rows) {
                        t.log(unit.item.id, &unit.item.name, None);
                    }
                    let mut monitor = unit.monitor;
                    monitor.retries = 0;
                    pending.push(monitor);
                }
            }
            Attempt::Failed(e) => {
                self.internal_metrics
                    .total_items_errored
                    .fetch_add(count, Ordering::Relaxed);

                let failure = e.to_failure();
                for unit in item.units() {
                    pending.push(self.fail_unit(unit, &e, &failure, elapsed));
                }
            }
            Attempt::Cancelled => {
                // Shutdown, not a fault. Requeue untouched and record nothing:
                // this arm fires for every in-flight item on every deploy, so
                // writing a failure here would put a red dot on healthy items and
                // punch a hole in their activity maps.
                debug!("monitor check cancelled during shutdown");
                pending.extend(item.units().iter().map(|unit| unit.monitor));
            }
        }

        let mut item_cache = self.config_cache.item.write();
        for monitor in pending {
            item_cache.requeue(monitor);
        }
    }

    /// Record `e` against one unit of a check and decide when it runs next: in
    /// `RETRY_INTERVAL` while retries remain and the kind is worth retrying,
    /// otherwise on its own schedule.
    fn fail_unit(
        &self,
        unit: &cache::item::Entry,
        e: &Error,
        failure: &Failure,
        elapsed: Duration,
    ) -> item::ItemMonitor {
        let retryable = e.retryable();
        if retryable {
            debug!(item_id = %unit.item.id, error = %e, "monitor item failed");
        } else {
            warn!(item_id = %unit.item.id, error = %e, "monitor item failed");
        }

        let mut monitor = unit.monitor;
        let retrying = if !retryable {
            // Nothing to gain from waiting — fall straight back to the
            // item's own schedule so the next real check isn't delayed.
            monitor.retries = 0;
            false
        } else {
            monitor.retries += 1;
            if monitor.retries < MAX_RETRIES {
                monitor.next_check = Instant::now() + RETRY_INTERVAL;
                true
            } else {
                warn!(
                    item_id = %unit.item.id,
                    error = %e,
                    retries = MAX_RETRIES,
                    "monitor item retries exhausted"
                );
                self.internal_metrics
                    .total_items_retry_dropped
                    .fetch_add(1, Ordering::Relaxed);
                monitor.retries = 0;
                false
            }
        };
        if let Some(t) = unit.stats.record_failure(
            failure.clone(),
            elapsed,
            monitor.retries.min(u8::MAX as u32) as u8,
            retrying,
        ) {
            t.log(unit.item.id, &unit.item.name, Some(failure));
        }
        monitor
    }

    #[tracing::instrument(skip_all, fields(worker = self.idx))]
    async fn run(&mut self) {
        self.internal_metrics
            .total_workers
            .fetch_add(1, Ordering::Relaxed);
        debug!("monitor worker started");
        loop {
            let n = Instant::now();
            tokio::select! {
                msg = self.monitor_channel.recv_async() => {
                    self.internal_metrics.receive_wait_time.fetch_add(
                        n.elapsed().as_nanos() as u64,
                        Ordering::Relaxed,
                    );
                    match msg {
                        Ok(item) => {
                            let n2 = Instant::now();
                            self.process_item(item).await;
                            self.internal_metrics.engine_time_spent.fetch_add(
                                n2.elapsed().as_nanos() as u64,
                                Ordering::Relaxed,
                            );
                        }
                        Err(_) => {
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    self.internal_metrics.receive_wait_time.fetch_add(
                        n.elapsed().as_nanos() as u64,
                        Ordering::Relaxed,
                    );
                }
                _ = self.token.cancelled() => {
                    self.internal_metrics
                        .total_time_spent
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    break
                },
            }
            self.internal_metrics
                .total_time_spent
                .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        self.internal_metrics
            .total_workers
            .fetch_sub(1, Ordering::Relaxed);
        debug!("monitor worker stopped");
    }
}

struct WorkerUnit {
    token: CancellationToken,
    handle: JoinHandle<()>,
}

#[async_trait]
pub trait WorkerManager: Send + Sync {
    /// Apply a re-read monitor config in place; `true` when anything changed.
    async fn apply_config(&self, config: MonitorConfig) -> bool;
}

pub struct WorkerGroup {
    internal_metrics: Arc<InternalMetrics>,
    metrics_token: CancellationToken,
    scheduler_token: CancellationToken,
    batch_token: CancellationToken,
    units: Mutex<Vec<WorkerUnit>>,
    channel: SenderChannel<&'static str, payload::ProcessorPayload>,
    monitor_channel: (Sender<MonitorItem>, Receiver<MonitorItem>),
    metric_channel: (Sender<MonitorRecord>, Receiver<MonitorRecord>),
    /// Watched by the batcher, so a config change reaches it without a restart.
    batch: watch::Sender<Batch>,
    config_cache: cache::Cache,
}

#[async_trait]
impl WorkerManager for WorkerGroup {
    async fn apply_config(&self, config: MonitorConfig) -> bool {
        WorkerGroup::apply_config(self, &config).await
    }
}

impl WorkerGroup {
    pub fn new(
        channel: SenderChannel<&'static str, payload::ProcessorPayload>,
        batch: Batch,
        config_cache: cache::Cache,
    ) -> Arc<Self> {
        Arc::new(Self {
            internal_metrics: Arc::new(InternalMetrics::default()),
            metrics_token: CancellationToken::new(),
            scheduler_token: CancellationToken::new(),
            batch_token: CancellationToken::new(),
            units: Mutex::new(Vec::new()),
            channel,
            monitor_channel: bounded(10),
            metric_channel: bounded(0),
            batch: watch::Sender::new(batch),
            config_cache,
        })
    }

    // should guarantee the items sent will have the same endpoint and item type
    // And for now, only batch snmp items until we see other items that support batch check.
    // Batch check: a single round-trip to the endpoint to check multiple items
    async fn scheduler_loop(
        sender: &Sender<MonitorItem>,
        item_cache: &RwLock<cache::item::Item>,
        token: &CancellationToken,
    ) {
        let items = item_cache.write().pop_due();
        let mut item_by_endpoint: FxHashMap<
            (Option<Uuid>, Discriminant<item::Config>),
            Vec<cache::item::Entry>,
        > = FxHashMap::default();
        for unit in items {
            if !unit.item.supports_batch_check() {
                tokio::select! {
                    _ = sender.send_async(MonitorItem::Single(unit)) => {},
                    _ = token.cancelled() => return,
                }
                continue;
            }
            let key = (unit.item.endpoint_id, discriminant(&unit.item.config));
            if let Some(batch) = item_by_endpoint.get_mut(&key) {
                batch.push(unit);
                if batch.len() >= MAX_ITEMS_PER_WORKER {
                    tokio::select! {
                        _ = sender.send_async(MonitorItem::Batch(std::mem::take(batch))) => {},
                        _ = token.cancelled() => return,
                    }
                }
            } else {
                let mut batch = Vec::with_capacity(MAX_ITEMS_PER_WORKER);
                batch.push(unit);
                item_by_endpoint.insert(key, batch);
            }
        }
        for (_, batch) in item_by_endpoint {
            if !batch.is_empty() {
                tokio::select! {
                    _ = sender.send_async(MonitorItem::Batch(batch)) => {},
                    _ = token.cancelled() => return,
                }
            }
        }
    }

    async fn scheduler_worker(
        sender: Sender<MonitorItem>,
        item_cache: Arc<RwLock<cache::item::Item>>,
        token: CancellationToken,
    ) {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    Self::scheduler_loop(&sender, &item_cache, &token).await;
                },
                _ = token.cancelled() => break,
            }
        }
    }

    async fn batch_worker(
        batch_conf: watch::Receiver<Batch>,
        sender: SenderChannel<&'static str, payload::ProcessorPayload>,
        receiver: Receiver<MonitorRecord>,
        internal_metrics: Arc<InternalMetrics>,
        token: CancellationToken,
    ) {
        let mut batcher = Batcher::new(batch_conf, sender, receiver, internal_metrics, token);
        batcher.batch_loop().await
    }

    async fn main_loop(self: Arc<Self>) {
        let scheduler_handle = tokio::spawn(Self::scheduler_worker(
            self.monitor_channel.0.clone(),
            self.config_cache.item.clone(),
            self.scheduler_token.clone(),
        ));
        let batch_handle = tokio::spawn(Self::batch_worker(
            self.batch.subscribe(),
            self.channel.clone(),
            self.metric_channel.1.clone(),
            self.internal_metrics.clone(),
            self.batch_token.clone(),
        ));
        info!("monitor scheduler started");
        self.scheduler_token.cancelled().await;
        // Panics are reported by the hook, as in `close_units`.
        if let Err(e) = scheduler_handle.await {
            debug!(error = %e, "monitor scheduler task panicked");
        }
        info!("monitor scheduler stopped");
        self.adjust(0).await;
        self.batch_token.cancel();
        if let Err(e) = batch_handle.await {
            debug!(error = %e, "monitor batcher task panicked");
        }
        info!("monitor batcher stopped");
    }

    pub async fn run(self: Arc<Self>) -> JoinHandle<()> {
        tokio::spawn(self.clone().main_loop())
    }

    pub async fn spawn_metrics_reporter(
        self: Arc<Self>,
        metrics_report_duration: Duration,
    ) -> JoinHandle<()> {
        let token_clone = self.metrics_token.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(metrics_report_duration) => {
                        let metrics = self.internal_metrics.get();
                        INTERNAL_METRICS.monitor.store(Arc::new((Instant::now().into_std(), metrics)));
                    }
                    _ = token_clone.cancelled() => break,
                }
            }
        })
    }

    pub fn close_scheduler(&self) {
        self.scheduler_token.cancel();
    }

    pub fn close_metrics_reporter(&self) {
        self.metrics_token.cancel();
    }

    async fn close_units(units: Vec<WorkerUnit>) {
        for unit in units.iter() {
            unit.token.cancel();
        }
        // A panic was reported by the hook and has started the shutdown;
        // re-raising it here would only cut the teardown short.
        for unit in units {
            if let Err(e) = unit.handle.await {
                debug!(error = %e, "monitor worker task panicked");
            }
        }
    }

    /// Apply `config` to the running group: the batcher flushes what it holds
    /// and continues with the new batch, and workers are added or stopped to
    /// match `worker_count`. Returns whether anything changed.
    pub async fn apply_config(&self, config: &MonitorConfig) -> bool {
        let batch_changed = self.batch.send_if_modified(|batch| {
            let changed = *batch != config.batch;
            *batch = config.batch;
            changed
        });
        let workers_changed = self.units.lock().await.len() != config.worker_count;
        self.adjust(config.worker_count).await;
        batch_changed || workers_changed
    }

    pub async fn adjust(&self, count: usize) {
        let mut units = self.units.lock().await;
        let units_len = units.len();
        if count != units_len {
            info!(from = units_len, to = count, "monitor workers adjusted");
        }
        if count > units_len {
            for idx in 0..(count - units_len) {
                let token = CancellationToken::new();
                let mut worker = Worker {
                    idx: units_len + idx,
                    internal_metrics: self.internal_metrics.clone(),
                    sender: self.metric_channel.0.clone(),
                    token: token.clone(),
                    monitor_channel: self.monitor_channel.1.clone(),
                    config_cache: self.config_cache.clone(),
                };
                let handle = tokio::spawn(async move {
                    worker.run().await;
                });
                units.push(WorkerUnit { token, handle });
            }
        } else if count < units_len {
            let units_to_close = units.drain(..(units_len - count)).collect();
            Self::close_units(units_to_close).await;
        }
    }
}

/// Rows pending in a builder, per item, so a failed send can be attributed back
/// to the items whose rows were lost.
///
/// Keyed by item id with the item's `Stats` alongside, rather than looking the id
/// up in the item cache afterwards: by the time a send fails the rows are already
/// inside an Arrow builder, and the batcher would otherwise need the cache lock on
/// the drop path.
type RowTally = FxHashMap<Uuid, (Arc<Stats>, u64)>;

struct BatchEntry {
    log_builder: payload::LogsBuilder,
    metric_builder: payload::MetricsBuilder,
    key: Option<delay_queue::Key>,
    /// Tallies are per signal because the two payloads are broadcast separately
    /// and only one of them may fail.
    log_rows: RowTally,
    metric_rows: RowTally,
}

/// Charge every row still in `tally` to its item as dropped, and reset it.
fn charge_dropped(tally: &mut RowTally) {
    for (_, (stats, rows)) in tally.drain() {
        stats.add_dropped(rows);
    }
}

impl BatchEntry {
    fn new() -> Self {
        Self {
            log_builder: payload::LogsBuilder::new(),
            metric_builder: payload::MetricsBuilder::new(),
            key: None,
            log_rows: RowTally::default(),
            metric_rows: RowTally::default(),
        }
    }

    /// Whether the pending builders have reached `batch_size` and should be
    /// flushed.
    ///
    /// `batch_size` counts **items** — data points for metrics — so that it means
    /// the same thing here as it does at the listeners and sinks, which compare
    /// it against `PayloadBatch::num_items` and hand it to `make_item_batches`.
    ///
    /// Rows are checked too, and not just for tidiness: a metric row carrying few
    /// or no data points advances the row count without advancing the item count,
    /// and metric rows are addressed by a `u16` parent id. Since `batch_size` is
    /// validated to be `<= u16::MAX`, flushing on the row total keeps every
    /// builder's row id below the overflow panic in `Base::next_row_id`.
    fn should_flush(&self, batch_size: usize) -> bool {
        let items = self.log_builder.num_items() + self.metric_builder.num_items();
        let rows = self.log_builder.num_rows() + self.metric_builder.num_rows();
        items.max(rows) >= batch_size
    }
}
struct Batcher {
    /// The current value of `batch_rx`.
    batch_conf: Batch,
    batch_rx: watch::Receiver<Batch>,
    sender: SenderChannel<&'static str, payload::ProcessorPayload>,
    receiver: Receiver<MonitorRecord>,
    internal_metrics: Arc<InternalMetrics>,
    token: CancellationToken,
    batch: FxHashMap<Option<Uuid>, BatchEntry>,
    deadlines: DelayQueue<Option<Uuid>>,
}

impl Batcher {
    fn new(
        mut batch_rx: watch::Receiver<Batch>,
        sender: SenderChannel<&'static str, payload::ProcessorPayload>,
        receiver: Receiver<MonitorRecord>,
        internal_metrics: Arc<InternalMetrics>,
        token: CancellationToken,
    ) -> Self {
        let batch_conf = *batch_rx.borrow_and_update();
        Self {
            batch_conf,
            batch_rx,
            sender,
            receiver,
            internal_metrics,
            token,
            batch: FxHashMap::default(),
            deadlines: DelayQueue::new(),
        }
    }

    fn gen_scope(kind: item::ItemKind) -> Scope {
        Scope {
            name: SmolStr::new_static("monitor"),
            listener_id: None,
            listener_type: None,
            listener_name: None,
            monitor_type: Some(SmolStr::new_static(kind.into())),
            node_id: config::NODE_ID.get().copied(),
        }
    }

    fn set_log_scope(batch: &mut payload::LogsBuilder, kind: item::ItemKind) {
        batch.scope(Self::gen_scope(kind));
    }

    fn set_metric_scope(batch: &mut payload::MetricsBuilder, kind: item::ItemKind) {
        batch.scope(Self::gen_scope(kind));
    }

    /// Finish the pending builders and broadcast them downstream.
    ///
    /// A failed broadcast is where monitor rows actually go missing: on a
    /// `non_blocking` channel a full buffer means `try_send` drops the payload, and
    /// this used to discard the error, leaving an item looking green while its data
    /// never arrived. Each signal's tally is charged to its items on failure and
    /// cleared on success. A send still waiting when `token` is cancelled is a
    /// failure too: with a sink that stopped consuming the processor channel
    /// stays full, and shutdown waits for the batcher to return.
    async fn send_batch(
        processor_id: Option<Uuid>,
        entry: &mut BatchEntry,
        internal_metrics: &InternalMetrics,
        sender: &SenderChannel<&'static str, payload::ProcessorPayload>,
        token: &CancellationToken,
    ) {
        let mut payloads: [Option<payload::Payload>; 2] = [None, None];
        if !entry.log_builder.is_empty() {
            let n = Instant::now();
            let payload = entry.log_builder.finish();
            internal_metrics
                .engine_time_spent
                .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
            payloads[0] = Some(payload);
        }
        if !entry.metric_builder.is_empty() {
            let n = Instant::now();
            let payload = entry.metric_builder.finish();
            internal_metrics
                .engine_time_spent
                .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
            payloads[1] = Some(payload);
        }
        for (idx, pl) in payloads.into_iter().enumerate() {
            let Some(pl) = pl else { continue };
            let n = Instant::now();
            let result = tokio::select! {
                biased;

                result = sender.broadcast(payload::ProcessorPayload::new(processor_id, pl)) => {
                    Some(result)
                }
                _ = token.cancelled() => None,
            };
            internal_metrics
                .send_wait_time
                .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);

            // Index order matches how `payloads` was filled above: 0 logs, 1
            // metrics.
            let tally = if idx == 0 {
                &mut entry.log_rows
            } else {
                &mut entry.metric_rows
            };
            match result {
                Some(Ok(())) => tally.clear(),
                Some(Err(e)) => {
                    warn!(?processor_id, error = %e, "dropping monitor payload, processor channel unavailable");
                    charge_dropped(tally);
                }
                None => {
                    let rows: u64 = tally.values().map(|(_, rows)| rows).sum();
                    warn!(
                        ?processor_id,
                        rows, "dropping monitor payload on shutdown, processor channel full"
                    );
                    charge_dropped(tally);
                }
            }
        }
    }

    async fn push_log_processor(
        &mut self,
        kind: item::ItemKind,
        resource: payload::Resource,
        row: payload::LogRowOwned,
        stats: Arc<Stats>,
        processor_id: Option<Uuid>,
    ) {
        let e = self
            .batch
            .entry(processor_id)
            .or_insert_with(BatchEntry::new);
        let item_id = resource.item_id;
        Self::set_log_scope(&mut e.log_builder, kind);
        e.log_builder.resource(resource);
        e.log_builder.push_row_owned(row);
        e.log_rows.entry(item_id).or_insert((stats, 0)).1 += 1;
        if e.key.is_none() {
            e.key = Some(
                self.deadlines
                    .insert(processor_id, self.batch_conf.batch_timeout),
            );
        }
        if e.should_flush(self.batch_conf.batch_size) {
            Self::send_batch(
                processor_id,
                e,
                &self.internal_metrics,
                &self.sender,
                &self.token,
            )
            .await;
            if let Some(k) = e.key.take() {
                self.deadlines.remove(&k);
            }
        }
    }

    async fn push_metric_processor(
        &mut self,
        kind: item::ItemKind,
        resource: payload::Resource,
        row: payload::MetricRowOwned,
        stats: Arc<Stats>,
        processor_id: Option<Uuid>,
    ) {
        let e = self
            .batch
            .entry(processor_id)
            .or_insert_with(BatchEntry::new);
        let item_id = resource.item_id;
        Self::set_metric_scope(&mut e.metric_builder, kind);
        e.metric_builder.resource(resource);
        e.metric_builder.push_row_owned(row);
        e.metric_rows.entry(item_id).or_insert((stats, 0)).1 += 1;
        if e.key.is_none() {
            e.key = Some(
                self.deadlines
                    .insert(processor_id, self.batch_conf.batch_timeout),
            );
        }
        if e.should_flush(self.batch_conf.batch_size) {
            Self::send_batch(
                processor_id,
                e,
                &self.internal_metrics,
                &self.sender,
                &self.token,
            )
            .await;
            if let Some(k) = e.key.take() {
                self.deadlines.remove(&k);
            }
        }
    }

    /// Fan one row out to every processor on its item.
    ///
    /// Each processor gets its own copy, which is why the drop tallies count row
    /// copies rather than collected rows.
    async fn push_log(&mut self, record: MonitorLog) {
        if record.processors.is_empty() {
            self.push_log_processor(record.kind, record.resource, record.row, record.stats, None)
                .await;
        } else {
            let (last_processor_id, rest) = record.processors.split_last().unwrap();
            for processor_id in rest {
                self.push_log_processor(
                    record.kind,
                    record.resource.clone(),
                    record.row.clone(),
                    record.stats.clone(),
                    Some(*processor_id),
                )
                .await;
            }
            self.push_log_processor(
                record.kind,
                record.resource,
                record.row,
                record.stats,
                Some(*last_processor_id),
            )
            .await;
        }
    }

    async fn push_metric(&mut self, record: MonitorMetric) {
        if record.processors.is_empty() {
            self.push_metric_processor(
                record.kind,
                record.resource,
                record.row,
                record.stats,
                None,
            )
            .await;
        } else {
            let (last_processor_id, rest) = record.processors.split_last().unwrap();
            for processor_id in rest {
                self.push_metric_processor(
                    record.kind,
                    record.resource.clone(),
                    record.row.clone(),
                    record.stats.clone(),
                    Some(*processor_id),
                )
                .await;
            }
            self.push_metric_processor(
                record.kind,
                record.resource,
                record.row,
                record.stats,
                Some(*last_processor_id),
            )
            .await;
        }
    }

    /// Send every pending batch now, so none of them waits out a size or
    /// timeout that is no longer the config.
    async fn flush_all(&mut self) {
        self.deadlines.clear();
        for (processor_id, mut e) in self.batch.drain() {
            Self::send_batch(
                processor_id,
                &mut e,
                &self.internal_metrics,
                &self.sender,
                &self.token,
            )
            .await;
        }
    }

    async fn batch_loop(&mut self) {
        loop {
            tokio::select! {
                biased;

                _ = self.token.cancelled() => break,
                Some(expired) = self.deadlines.next() => {
                    let processor_id = expired.into_inner();
                    // getting the processor_id expired means the traffic isn't that demanding
                    // so it's ok to remove the builder. this also avoid us some memory leak
                    if let Some(mut e) = self.batch.remove(&processor_id) {
                        Self::send_batch(
                            processor_id,
                            &mut e,
                            &self.internal_metrics,
                            &self.sender,
                            &self.token,
                        ).await;
                    }
                }
                // An `Err` means the group dropped its sender, which it outlives
                // us to do; the pattern just disables the arm then.
                Ok(()) = self.batch_rx.changed() => {
                    self.batch_conf = *self.batch_rx.borrow_and_update();
                    self.flush_all().await;
                }
                record = self.receiver.recv_async() => {
                    if record.is_err() {
                        error!("monitor record channel closed");
                    }
                    let record = record.unwrap();
                    match record {
                        MonitorRecord::SingleLog(record) => {
                            self.push_log(*record).await;
                        }
                        MonitorRecord::BatchLog(records) => {
                            for record in records {
                                self.push_log(record).await;
                            }
                        }
                        MonitorRecord::SingleMetric(record) => {
                            self.push_metric(*record).await;
                        }
                        MonitorRecord::BatchMetric(records) => {
                            for record in records {
                                self.push_metric(record).await;
                            }
                        }
                        MonitorRecord::Null => {},
                    }
                },
            }
        }

        // Cancellation is the first, biased arm above, so anything still batched
        // when we get here is lost. Charge it rather than letting a shutdown
        // silently eat rows the items already counted as collected.
        let pending_rows: u64 = self
            .batch
            .values()
            .flat_map(|entry| entry.log_rows.values().chain(entry.metric_rows.values()))
            .map(|(_, rows)| rows)
            .sum();
        if pending_rows > 0 {
            warn!(
                rows = pending_rows,
                "dropping batched monitor rows on shutdown"
            );
        }
        for entry in self.batch.values_mut() {
            charge_dropped(&mut entry.log_rows);
            charge_dropped(&mut entry.metric_rows);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::host::Host as HostConfig;
    use crate::config::item::{Config as ItemConfig, Dummy, DummyPayload, Schedule};
    use std::time::Duration as StdDuration;

    /// Retryability is derived from the kind, so this is the table that decides
    /// whether a failing item waits `RETRY_INTERVAL` or falls back to its own
    /// schedule. Spelled out per kind because getting it wrong is invisible until
    /// production: a non-retryable kind that retries burns three round trips and
    /// hides the actionable state behind `Retrying`.
    #[test]
    fn retryability_is_a_function_of_the_kind() {
        for kind in [
            FailureKind::Timeout,
            FailureKind::Unreachable,
            FailureKind::TargetError,
            FailureKind::Malformed,
            FailureKind::Dropped,
            FailureKind::Cluster,
        ] {
            assert!(Error::new(kind, "x").retryable(), "{kind} should retry");
        }
        for kind in [
            FailureKind::Auth,
            FailureKind::NotSupported,
            FailureKind::Config,
            FailureKind::Internal,
        ] {
            assert!(
                !Error::new(kind, "x").retryable(),
                "{kind} should not retry"
            );
        }
    }

    struct Harness {
        worker: Worker,
        // Kept alive so the worker's rendezvous send has a counterpart, and so we
        // can assert on what was emitted.
        records: Receiver<MonitorRecord>,
        cache: cache::Cache,
    }

    fn harness() -> Harness {
        let cache = cache::Cache::new();
        let (record_tx, records) = bounded(16);
        let (_item_tx, monitor_channel) = bounded(1);
        Harness {
            worker: Worker {
                idx: 0,
                internal_metrics: Arc::new(InternalMetrics::default()),
                token: CancellationToken::new(),
                monitor_channel,
                sender: record_tx,
                config_cache: cache.clone(),
            },
            records,
            cache,
        }
    }

    /// Register a host plus an item, and hand back the unit the scheduler would.
    fn schedule(cache: &cache::Cache, config: ItemConfig) -> cache::item::Entry {
        let host_id = add_host(cache);
        schedule_on(cache, host_id, None, StdDuration::from_secs(5), config)
    }

    fn add_host(cache: &cache::Cache) -> Uuid {
        let host = HostConfig {
            id: Uuid::now_v7(),
            name: "host".into(),
            timezone: None,
        };
        let host_id = host.id;
        cache.host.write().insert(host);
        host_id
    }

    /// Register an item on an existing host (and endpoint), and hand back the unit
    /// the scheduler would.
    fn schedule_on(
        cache: &cache::Cache,
        host_id: Uuid,
        endpoint_id: Option<Uuid>,
        timeout: StdDuration,
        config: ItemConfig,
    ) -> cache::item::Entry {
        let item = item::Item::new(
            Uuid::now_v7(),
            "test".into(),
            endpoint_id,
            host_id,
            Schedule::Interval(StdDuration::from_secs(60)),
            None,
            timeout,
            vec![],
            config,
        );
        let id = item.id;
        cache.item.write().insert(item);
        cache.item.write().execute_now(&id).unwrap();
        cache.item.write().pop_due().pop().unwrap()
    }

    fn dummy(count: usize) -> ItemConfig {
        ItemConfig::Dummy(Dummy {
            payload: DummyPayload::default(),
            count,
        })
    }

    /// How many buckets each plane of the windowed activity ring has set, as
    /// `(ok, fail)`. Counting rather than naming a bucket keeps these tests off the
    /// wall clock — the check and the snapshot could otherwise straddle a
    /// five-minute boundary.
    fn marks(snap: &cache::item::StatsSnapshot) -> (u32, u32) {
        fn count(plane: &bytes::Bytes) -> u32 {
            plane.iter().map(|b| b.count_ones()).sum()
        }
        (count(&snap.activity_ok), count(&snap.activity_fail))
    }

    #[tokio::test]
    async fn a_successful_check_records_rows_and_goes_ok() {
        let mut h = harness();
        let unit = schedule(&h.cache, dummy(3));
        let stats = unit.stats.clone();

        h.worker.process_item(MonitorItem::Single(unit)).await;

        let snap = stats.snapshot();
        assert_eq!(snap.state, cache::item::ItemState::Ok);
        assert_eq!(snap.total_checks, 1);
        assert_eq!(snap.total_records, 3);
        assert_eq!(snap.last_record_count, Some(3));
        assert!(snap.last_success.is_some());
        assert!(snap.last_failure.is_none());
        assert_eq!(marks(&snap), (1, 0), "one ok bucket, nothing failing");
        // The rows really were handed downstream.
        assert!(matches!(
            h.records.try_recv(),
            Ok(MonitorRecord::BatchLog(rows)) if rows.len() == 3
        ));
    }

    /// A check with no endpoint is a config mistake, and config mistakes must not
    /// be retried — the item goes straight back onto its own schedule.
    #[tokio::test]
    async fn a_config_failure_goes_failing_without_retrying() {
        let mut h = harness();
        // SnmpWalk requires an endpoint; the item has none.
        let unit = schedule(
            &h.cache,
            ItemConfig::SnmpWalk(crate::config::item::SnmpWalk {
                oids: vec![],
                table_format: false,
                use_mib: false,
                disable_bulk: None,
                poll_worker_count: 1,
                max_results: 0,
                chunk_size: 10,
                request_timeout: StdDuration::from_secs(1),
                request_retries: 0,
                max_unknown_oids_per_row: None,
            }),
        );
        let id = unit.item.id;
        let stats = unit.stats.clone();

        h.worker.process_item(MonitorItem::Single(unit)).await;

        let snap = stats.snapshot();
        // `Failing` rather than `Retrying` is the observable proof that the
        // non-retry path was taken: `Retrying` is only reachable when the kind is
        // retryable, and it would have consumed a retry. Asserting on
        // `next_check` instead would be unsound — interval jitter can land
        // inside the 30s retry window, so the two are not distinguishable by
        // timestamp.
        assert_eq!(snap.state, cache::item::ItemState::Failing);
        assert_eq!(snap.total_checks, 1);
        assert_eq!(snap.total_failures, 1);
        assert_eq!(snap.consecutive_failures, 1);
        assert_eq!(snap.retries, 0, "a config error must not accrue retries");
        assert_eq!(marks(&snap), (0, 1), "one failing bucket, nothing ok");
        let failure = snap.last_failure.expect("a failure is recorded");
        assert_eq!(failure.kind, FailureKind::Config);

        // Still scheduled, and still the same stats object the cache holds.
        let cached = h.cache.item.read().stats(&id).unwrap();
        assert!(Arc::ptr_eq(&cached, &stats));
        assert!(cached.snapshot().next_check.is_some());
    }

    /// Shutdown fires for every in-flight item on every deploy. It must leave no
    /// trace: no failure, no activity bit, no consumed retry.
    #[tokio::test]
    async fn cancellation_records_nothing() {
        let mut h = harness();
        let unit = schedule(&h.cache, dummy(1));
        let stats = unit.stats.clone();
        h.worker.token.cancel();

        h.worker.process_item(MonitorItem::Single(unit)).await;

        let snap = stats.snapshot();
        assert_eq!(snap.total_failures, 0);
        assert!(snap.last_failure.is_none());
        assert_eq!(
            marks(&snap),
            (0, 0),
            "shutdown must not punch the sparkline"
        );
        assert_eq!(
            h.worker
                .internal_metrics
                .total_items_errored
                .load(Ordering::Relaxed),
            0
        );
        // The attempt is still counted — we did try.
        assert_eq!(snap.total_checks, 1);
        assert_eq!(snap.state, cache::item::ItemState::NeverRan);
    }

    /// A processor topic the test reads from directly.
    async fn processor_channel() -> (
        crate::chan::AdminChannel<&'static str, payload::ProcessorPayload>,
        SenderChannel<&'static str, payload::ProcessorPayload>,
        Receiver<payload::ProcessorPayload>,
    ) {
        let (admin, sender) = crate::chan::new(
            StdDuration::from_secs(3600),
            Arc::new(arc_swap::ArcSwap::from_pointee((
                std::time::Instant::now(),
                common::internal_metrics::Channel::default(),
            ))),
        );
        let rx = admin
            .get_receiver(
                &crate::processor::PROCESSOR_CHANNEL_NAME,
                config::Channel::Memory(config::MemoryChannel {
                    non_blocking: false,
                    channel_buffer_size: 16,
                }),
            )
            .await
            .unwrap();
        (admin, sender, rx)
    }

    #[tokio::test]
    async fn apply_config_reports_changes_and_scales_workers() {
        let (_admin, sender, _rx) = processor_channel().await;
        let group = WorkerGroup::new(sender, Batch::default(), cache::Cache::new());
        let mut config = MonitorConfig {
            worker_count: 3,
            batch: Batch::default(),
        };

        assert!(group.apply_config(&config).await, "workers 0 -> 3");
        assert_eq!(group.units.lock().await.len(), 3);
        assert!(!group.apply_config(&config).await, "nothing changed");

        config.batch.batch_size = 10;
        assert!(group.apply_config(&config).await, "batch changed");
        assert_eq!(group.batch.borrow().batch_size, 10);

        config.worker_count = 1;
        assert!(group.apply_config(&config).await, "workers 3 -> 1");
        assert_eq!(group.units.lock().await.len(), 1);

        group.adjust(0).await;
    }

    /// Rows batched under the old config go out as soon as it changes, rather
    /// than waiting out a timeout that is no longer the config, and later rows
    /// batch by the new size.
    #[tokio::test]
    async fn a_batch_change_flushes_pending_rows() {
        let mut h = harness();
        // Rendezvous, so a check returns only once the batcher has taken its
        // rows, and they are batched before the config change below is seen.
        let (record_tx, records) = bounded(0);
        h.worker.sender = record_tx;
        let (_admin, sender, processor_rx) = processor_channel().await;
        let hour = StdDuration::from_secs(3600);
        let (batch_tx, batch_rx) = watch::channel(Batch {
            batch_size: 1000,
            batch_timeout: hour,
        });
        let token = CancellationToken::new();
        let mut batcher = Batcher::new(
            batch_rx,
            sender,
            records,
            Arc::new(InternalMetrics::default()),
            token.clone(),
        );
        let batcher = tokio::spawn(async move { batcher.batch_loop().await });
        let recv = |rx: &Receiver<payload::ProcessorPayload>| {
            let rx = rx.clone();
            async move {
                tokio::time::timeout(StdDuration::from_secs(5), rx.recv_async())
                    .await
                    .expect("a payload is flushed")
                    .unwrap()
                    .payload
                    .num_items()
            }
        };

        let unit = schedule(&h.cache, dummy(3));
        h.worker.process_item(MonitorItem::Single(unit)).await;
        assert!(processor_rx.is_empty(), "3 rows stay under a batch of 1000");

        batch_tx.send_replace(Batch {
            batch_size: 2,
            batch_timeout: hour,
        });
        assert_eq!(recv(&processor_rx).await, 3);

        let unit = schedule(&h.cache, dummy(2));
        h.worker.process_item(MonitorItem::Single(unit)).await;
        assert_eq!(recv(&processor_rx).await, 2, "flushed at the new size");

        token.cancel();
        batcher.await.unwrap();
    }

    #[tokio::test]
    async fn a_check_that_yields_nothing_is_ok_empty() {
        let mut h = harness();
        let unit = schedule(&h.cache, dummy(0));
        let stats = unit.stats.clone();

        h.worker.process_item(MonitorItem::Single(unit)).await;

        let snap = stats.snapshot();
        assert_eq!(snap.state, cache::item::ItemState::OkEmpty);
        assert_eq!(snap.last_record_count, Some(0));
        assert_eq!(snap.last_nonempty, None);
    }

    mod snmp_get {
        use super::*;
        use crate::config::endpoint::{Config as EndpointConfig, SnmpV1, SnmpV2C};
        use crate::config::item::Snmp;
        use async_snmp::agent::Agent;
        use async_snmp::handler::{
            BoxFuture, GetNextResult, GetResult, HandlerResult, MibHandler, RequestContext,
        };
        use async_snmp::{Oid, Value};

        use crate::monitor::snmp::TEST_RUNTIME as RUNTIME;

        const PREFIX: &str = "1.3.6.1.4.1.99999";
        const KNOWN: &str = "1.3.6.1.4.1.99999.1.0";
        const UNKNOWN: &str = "1.3.6.1.4.1.99999.2.0";

        fn oid(s: &str) -> Oid {
            Oid::parse(s).unwrap()
        }

        /// Answers `KNOWN` and nothing else.
        struct OneOid;

        impl MibHandler for OneOid {
            fn get<'a>(
                &'a self,
                _ctx: &'a RequestContext,
                req: &'a Oid,
            ) -> BoxFuture<'a, HandlerResult<GetResult>> {
                Box::pin(async move {
                    Ok(if *req == oid(KNOWN) {
                        GetResult::Value(Value::OctetString("up".into()))
                    } else {
                        GetResult::NoSuchObject
                    })
                })
            }

            fn get_next<'a>(
                &'a self,
                _ctx: &'a RequestContext,
                _req: &'a Oid,
            ) -> BoxFuture<'a, HandlerResult<GetNextResult>> {
                Box::pin(async { Ok(GetNextResult::EndOfMibView) })
            }
        }

        struct RunningAgent {
            agent: Arc<Agent>,
            handle: JoinHandle<async_snmp::Result<()>>,
        }

        impl RunningAgent {
            async fn start() -> Self {
                let agent = Agent::builder()
                    .bind("127.0.0.1:0")
                    .community(b"public")
                    .handler(oid(PREFIX), Arc::new(OneOid))
                    .allow_all_access()
                    .build()
                    .await
                    .unwrap();
                let agent = Arc::new(agent);
                let handle = tokio::spawn({
                    let agent = agent.clone();
                    async move { agent.run().await }
                });
                Self { agent, handle }
            }

            fn address(&self) -> String {
                self.agent.local_addr().to_string()
            }

            async fn stop(self) {
                self.agent.cancel();
                let _ = self.handle.await;
            }
        }

        fn v1(address: &str) -> EndpointConfig {
            EndpointConfig::SnmpV1(SnmpV1 {
                address: address.parse().unwrap(),
                community: "public".into(),
            })
        }

        fn v2c(address: &str) -> EndpointConfig {
            EndpointConfig::SnmpV2C(SnmpV2C {
                address: address.parse().unwrap(),
                community: "public".into(),
                max_bulk_repetitions: 25,
                disable_bulk: None,
            })
        }

        fn snmp(s: &str) -> ItemConfig {
            ItemConfig::Snmp(Snmp {
                oid: oid(s),
                use_mib: false,
            })
        }

        /// One host and endpoint, and a batch of items on it, one per oid.
        fn batch(
            cache: &cache::Cache,
            endpoint: EndpointConfig,
            timeout: StdDuration,
            oids: &[&str],
        ) -> Vec<cache::item::Entry> {
            let host_id = add_host(cache);
            let endpoint_id = Uuid::now_v7();
            cache.endpoint.write().insert(Endpoint::new(
                endpoint_id,
                "agent".into(),
                host_id,
                endpoint,
            ));
            oids.iter()
                .map(|o| schedule_on(cache, host_id, Some(endpoint_id), timeout, snmp(o)))
                .collect()
        }

        /// A batch with one oid the agent has and one it hasn't: the first is
        /// collected, the second fails on its own without dragging the first down.
        async fn one_known_one_unknown(endpoint: fn(&str) -> EndpointConfig) {
            let agent = RunningAgent::start().await;
            let mut h = harness();
            let units = batch(
                &h.cache,
                endpoint(&agent.address()),
                StdDuration::from_secs(5),
                &[KNOWN, UNKNOWN],
            );
            let (known, unknown) = (units[0].clone(), units[1].clone());

            h.worker.process_item(MonitorItem::Batch(units)).await;

            let snap = known.stats.snapshot();
            assert_eq!(snap.state, cache::item::ItemState::Ok);
            assert_eq!(snap.last_record_count, Some(1));

            let snap = unknown.stats.snapshot();
            assert_eq!(snap.state, cache::item::ItemState::Failing);
            assert_eq!(snap.retries, 0, "a missing oid must not be retried");
            let failure = snap.last_failure.expect("a failure is recorded");
            assert_eq!(failure.kind, FailureKind::NotSupported);

            let Ok(MonitorRecord::BatchLog(rows)) = h.records.try_recv() else {
                panic!("one batch of rows is emitted");
            };
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].resource.item_id, known.item.id);
            // Walk-shaped: `snmp.oid`, then `snmp.data` as a one-element list.
            use payload::ValueOwned as V;
            let attrs = &rows[0].row.attrs;
            assert_eq!(attrs[0].0.as_ref(), "snmp.oid");
            assert!(matches!(&attrs[0].1, V::String(s) if s.as_ref() == KNOWN));
            assert_eq!(attrs[1].0.as_ref(), "snmp.data");
            let V::Array(data) = &attrs[1].1 else {
                panic!("snmp.data is a list");
            };
            let [V::KvList(fields)] = data.as_slice() else {
                panic!("snmp.data holds one value");
            };
            let value = fields.iter().find(|(k, _)| k.as_ref() == "value");
            assert!(matches!(value, Some((_, V::String(s))) if s.as_ref() == "up"));
            agent.stop().await;
        }

        #[test]
        fn v2c_fails_only_the_item_with_a_missing_oid() {
            RUNTIME.block_on(async {
                one_known_one_unknown(v2c).await;
            });
        }

        /// v1 answers a missing oid by failing the whole PDU with noSuchName, so
        /// this exercises asking again without it.
        #[test]
        fn v1_fails_only_the_item_with_a_missing_oid() {
            RUNTIME.block_on(async {
                one_known_one_unknown(v1).await;
            });
        }

        #[test]
        fn items_on_the_same_oid_are_both_collected() {
            RUNTIME.block_on(async {
                let agent = RunningAgent::start().await;
                let mut h = harness();
                let units = batch(
                    &h.cache,
                    v2c(&agent.address()),
                    StdDuration::from_secs(5),
                    &[KNOWN, KNOWN],
                );
                let stats: Vec<_> = units.iter().map(|u| u.stats.clone()).collect();

                h.worker.process_item(MonitorItem::Batch(units)).await;

                for stats in stats {
                    assert_eq!(stats.snapshot().last_record_count, Some(1));
                }
                assert!(matches!(
                    h.records.try_recv(),
                    Ok(MonitorRecord::BatchLog(rows)) if rows.len() == 2
                ));
                agent.stop().await;
            });
        }

        /// The request itself failing is every member's problem, and worth retrying.
        #[test]
        fn an_unanswered_request_fails_the_whole_batch() {
            RUNTIME.block_on(async {
                // Bound and dropped, so nothing listens there.
                let address = std::net::UdpSocket::bind("127.0.0.1:0")
                    .unwrap()
                    .local_addr()
                    .unwrap()
                    .to_string();
                let mut h = harness();
                let units = batch(
                    &h.cache,
                    v2c(&address),
                    StdDuration::from_millis(300),
                    &[KNOWN, UNKNOWN],
                );
                let stats: Vec<_> = units.iter().map(|u| u.stats.clone()).collect();

                h.worker.process_item(MonitorItem::Batch(units)).await;

                for stats in stats {
                    let snap = stats.snapshot();
                    assert_eq!(snap.state, cache::item::ItemState::Retrying);
                    let kind = snap.last_failure.expect("a failure is recorded").kind;
                    assert!(
                        matches!(kind, FailureKind::Timeout | FailureKind::Unreachable),
                        "{kind}"
                    );
                }
                assert!(h.records.is_empty());
            });
        }
    }
}
