// SPDX-FileCopyrightText: 2025 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

// Public, unlike its sibling sinks, so integration tests can drive the schema DDL and the
// Payload → Arrow conversion directly. Both are self-contained, sink-independent pieces.
pub mod clickhouse;
pub mod export;
mod file;
// Public so `tests/kafka_sink_test.rs` can run `KafkaSink` and read its final metrics after
// `close`, which `WorkerGroup::close_all_sink` drops along with the sink.
pub mod kafka;
mod otap;

use crate::common::backoff::RestartBackoff;
use crate::common::internal_metrics::{INTERNAL_METRICS, Sink as Metrics};
use crate::common::shutdown::Deadline;
use crate::{chan, config, payload};
use anyhow::{Error, Result};
use async_trait::async_trait;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// How often the supervisor looks for workers that exited on their own.
const SUPERVISE_INTERVAL: Duration = Duration::from_secs(1);
/// Longest each phase of a sink's close takes: the workers delivering what
/// they hold, then the sink's own close (flushing its transport). Past it the
/// phase is cut short, so an unreachable destination cannot hold shutdown.
/// Matches the otap client's destination drain and the Kafka flush.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
/// How long workers cancelled after [`DRAIN_TIMEOUT`] get to requeue what
/// they hold before they are aborted.
const CANCEL_GRACE: Duration = Duration::from_secs(1);

#[derive(Error, Debug)]
pub enum SinkError {
    #[error("Sink already exists")]
    SinkAlreadyExists,
    #[error("Sink doesn't exist")]
    SinkDoesNotExist,
}

#[derive(Default)]
struct InternalMetrics {
    total_workers: AtomicUsize,
    total_time_spent: AtomicU64,
    engine_time_spent: AtomicU64,
    receive_wait_time: AtomicU64,
    send_wait_time: AtomicU64,
    total_records: AtomicU64,
    total_records_sent: AtomicU64,
    total_records_filtered: AtomicU64,
    total_records_errored: AtomicU64,
    total_records_retried: AtomicU64,
    records_pending_retry: AtomicU64,
}

impl InternalMetrics {
    fn get(&self, id: Uuid, name: String) -> Metrics {
        Metrics {
            id,
            name,
            total_workers: self.total_workers.load(Ordering::Relaxed),
            total_time_spent: Duration::from_nanos(self.total_time_spent.load(Ordering::Relaxed)),
            engine_time_spent: Duration::from_nanos(self.engine_time_spent.load(Ordering::Relaxed)),
            receive_wait_time: Duration::from_nanos(self.receive_wait_time.load(Ordering::Relaxed)),
            send_wait_time: Duration::from_nanos(self.send_wait_time.load(Ordering::Relaxed)),
            total_records: self.total_records.load(Ordering::Relaxed),
            total_records_sent: self.total_records_sent.load(Ordering::Relaxed),
            total_records_filtered: self.total_records_filtered.load(Ordering::Relaxed),
            total_records_errored: self.total_records_errored.load(Ordering::Relaxed),
            total_records_retried: self.total_records_retried.load(Ordering::Relaxed),
            records_pending_retry: self.records_pending_retry.load(Ordering::Relaxed),
            endpoints: Vec::new(),
        }
    }
}

/// Items to attribute to the `idx`-th of `blob_count` blobs that
/// [`payload::Payload::serialize_as`] produced from a payload of `items` items.
///
/// A batch format yields one blob for the whole payload, a denormalized one a
/// blob per root record; for metrics a record is not an item (one row may
/// carry many data points, or none), so the items are spread evenly with the
/// remainder on the last blob. Per payload the attributions sum to `items`,
/// which is what keeps `records_sent + records_errored == records`.
fn blob_items(items: u64, blob_count: usize, idx: usize) -> u64 {
    let blob_count = blob_count as u64;
    if blob_count == 0 {
        return 0;
    }
    let each = items / blob_count;
    if idx as u64 + 1 == blob_count {
        items - each * (blob_count - 1)
    } else {
        each
    }
}

#[async_trait]
pub trait Sink: Send + Sync {
    async fn run(&self, token: CancellationToken);
    fn metrics(&self) -> Metrics;
    fn get_channel(&self) -> chan::Consumer<payload::Payload>;

    /// Called once, after every worker of this sink has stopped, for sinks that
    /// own background tasks of their own and have to drain and flush them.
    /// Scaling workers down does not call it — only tearing the sink down does.
    async fn close(&self) {}
}

struct WorkerUnit {
    token: CancellationToken,
    handle: JoinHandle<()>,
}

/// The running units of a sink and the count the supervisor keeps them at.
#[derive(Default)]
struct Units {
    live: Vec<WorkerUnit>,
    desired: usize,
    restart: RestartBackoff,
}

impl Units {
    /// Takes out the units whose task already returned. Units leave `live`
    /// before they are cancelled, and a sink leaves the group's map before its
    /// channel is removed, so every one of these exited on its own.
    fn reap(&mut self) -> Vec<WorkerUnit> {
        self.live
            .extract_if(.., |unit| unit.handle.is_finished())
            .collect()
    }
}

/// What becomes of a sink's input topic as the sink closes.
#[derive(Clone, Copy)]
enum Input {
    /// The caller closed or removed the topic: the workers deliver what is
    /// queued and return once it runs out.
    Ends,
    /// The topic stays for the sink that replaces this one: the workers are
    /// cancelled and leave what is queued to it.
    HandedOver,
}

struct Worker {
    conf: config::sink::Sink,
    sink: Arc<dyn Sink>,
    units: Mutex<Units>,
    /// [`DRAIN_TIMEOUT`]; tests shorten it.
    drain_timeout: Duration,
    /// [`CANCEL_GRACE`]; tests shorten it.
    cancel_grace: Duration,
}

/// Whether keyspace `name` is the disk data of a sink not among `ids`: a sink
/// topic's keyspace is named by its id, written the way `Uuid` prints it.
fn is_orphaned(ids: &HashSet<Uuid>, name: &str) -> bool {
    Uuid::try_parse(name).is_ok_and(|id| id.to_string() == name && !ids.contains(&id))
}

/// A sink's exporter, built before the sink has a consumer: everything that
/// can fail to build (a file to open, client settings) is done. `add` builds
/// it before it touches the running sink or its topic, so a config it cannot
/// build leaves both as they were.
enum Prepared {
    File(file::File),
    Kafka(kafka::Kafka),
    Otel(otap::Otel),
    Clickhouse(Box<clickhouse::Clickhouse>),
}

impl Prepared {
    async fn new(conf: &config::sink::Sink) -> Result<Self> {
        Ok(match &conf.config {
            config::sink::Config::File(_) => Self::File(file::File::new(conf).await?),
            config::sink::Config::Kafka(_) => Self::Kafka(kafka::Kafka::new(conf)?),
            config::sink::Config::OpenTelemetry(otel) => Self::Otel(otap::Otel::new(otel.clone())),
            config::sink::Config::Clickhouse(_) => {
                Self::Clickhouse(Box::new(clickhouse::Clickhouse::new(conf)))
            }
        })
    }
}

impl Worker {
    fn new(
        conf: config::sink::Sink,
        prepared: Prepared,
        channel: chan::Consumer<payload::Payload>,
    ) -> Self {
        let c = conf.clone();
        let sink: Arc<dyn Sink> = match prepared {
            Prepared::File(x) => Arc::new(file::FileSink::with_exporter(c, channel, x)),
            Prepared::Kafka(x) => Arc::new(kafka::KafkaSink::with_exporter(c, channel, x)),
            Prepared::Otel(x) => Arc::new(otap::PatrolliSink::with_exporter(c, channel, x)),
            Prepared::Clickhouse(x) => {
                Arc::new(clickhouse::ClickhouseSink::with_exporter(c, channel, *x))
            }
        };
        Self::with_sink(conf, sink)
    }

    fn with_sink(conf: config::sink::Sink, sink: Arc<dyn Sink>) -> Self {
        Self {
            conf,
            sink,
            units: Mutex::new(Units::default()),
            drain_timeout: DRAIN_TIMEOUT,
            cancel_grace: CANCEL_GRACE,
        }
    }

    fn metrics(&self) -> Metrics {
        self.sink.metrics()
    }

    fn spawn_unit(&self) -> WorkerUnit {
        let token = CancellationToken::new();
        let token_clone = token.clone();
        let sink = self.sink.clone();
        let handle = tokio::spawn(async move {
            sink.run(token_clone).await;
        });
        WorkerUnit { token, handle }
    }

    /// Waits for `units` to return, cancelling them first unless their input
    /// `Ends`, for at most `drain_timeout` or until `deadline`. Workers still
    /// draining an ending input then (a destination that is down) are
    /// cancelled, so they requeue what they hold, and get `cancel_grace` for
    /// it. The grace is not cut by `deadline`: it is what keeps a disk
    /// topic's payloads, and the shutdown watchdog allows for it. Whatever
    /// still runs after that is aborted.
    async fn close_units(&self, mut units: Vec<WorkerUnit>, input: Input, deadline: Deadline) {
        if let Input::HandedOver = input {
            for unit in units.iter() {
                unit.token.cancel();
            }
        }
        let limit = deadline.budget(self.drain_timeout);
        if Self::join_units(&mut units, limit).await {
            return;
        }
        if let Input::Ends = input {
            warn!(
                timeout_ms = limit.as_millis() as u64,
                "sink drain timed out, cancelling workers"
            );
            for unit in units.iter() {
                unit.token.cancel();
            }
            if Self::join_units(&mut units, self.cancel_grace).await {
                return;
            }
        }
        warn!(workers = units.len(), "sink workers did not stop, aborting");
        for unit in units {
            unit.handle.abort();
        }
    }

    /// Waits up to `limit` for `units` to return, taking out each one that
    /// did. Whether all of them did.
    async fn join_units(units: &mut Vec<WorkerUnit>, limit: Duration) -> bool {
        tokio::time::timeout(limit, async {
            while let Some(unit) = units.last_mut() {
                if let Err(e) = (&mut unit.handle).await {
                    debug!(error = %e, "sink worker panicked");
                }
                units.pop();
            }
        })
        .await
        .is_ok()
    }

    async fn close(&self, input: Input, deadline: Deadline) {
        let units = {
            let mut units = self.units.lock().await;
            units.desired = 0;
            std::mem::take(&mut units.live)
        };
        self.close_units(units, input, deadline).await;
        // Producers before consumers: the workers are gone, so whatever the
        // sink still holds can now be drained and flushed. A worker stuck on
        // a destination was aborted above, which is also what lets this
        // close reach that destination.
        let limit = deadline.budget(self.drain_timeout);
        if tokio::time::timeout(limit, self.sink.close())
            .await
            .is_err()
        {
            warn!(
                timeout_ms = limit.as_millis() as u64,
                "sink close timed out"
            );
        }
    }

    async fn adjust(&self, count: usize) {
        let mut units = self.units.lock().await;
        units.desired = count;
        // Dead units are replaced here rather than counted as live.
        let dead = units.reap();
        let units_len = units.live.len();
        if count != units_len {
            debug!(
                sink_id = %self.conf.id,
                sink = %self.conf.name,
                from = units_len,
                to = count,
                "sink workers adjusted"
            );
        }
        if count > units_len {
            for _ in 0..(count - units_len) {
                let unit = self.spawn_unit();
                units.live.push(unit);
            }
        } else if count < units_len {
            let units_to_close = units.live.drain(..(units_len - count)).collect();
            self.close_units(units_to_close, Input::HandedOver, Deadline::NONE)
                .await;
        }
        drop(units);
        Self::report_exits(dead, Duration::ZERO).await;
    }

    /// Logs the units that exited on their own, once each.
    async fn report_exits(dead: Vec<WorkerUnit>, restart_in: Duration) {
        for unit in dead {
            match unit.handle.await {
                Ok(()) => warn!(restart_in = ?restart_in, "sink worker exited unexpectedly"),
                // Reported by the panic hook, which also started the
                // shutdown: see `common::panic`.
                Err(e) => debug!(error = %e, "sink worker panicked"),
            }
        }
    }

    /// Replaces units that returned without being cancelled, pacing restarts
    /// with [`RestartBackoff`]. Skips the tick when `adjust` or `close` holds
    /// the units.
    #[tracing::instrument(skip_all, fields(sink_id = %self.conf.id, sink = %self.conf.name))]
    async fn supervise(&self) {
        // A panic is fatal and the process is shutting down: a unit restarted
        // now would run on whatever state the panic left behind.
        if crate::common::panic::panicked() {
            return;
        }
        let Ok(mut units) = self.units.try_lock() else {
            return;
        };
        let now = tokio::time::Instant::now();
        let dead = units.reap();
        // Units sharing a connection tend to die together; that is one
        // failure, not one per unit, so it advances the backoff once.
        let restart_in = if dead.is_empty() {
            Duration::ZERO
        } else {
            units.restart.on_exit(now)
        };
        let missing = units.desired.saturating_sub(units.live.len());
        let restarted = if missing > 0 && units.restart.ready(now) {
            for _ in 0..missing {
                let unit = self.spawn_unit();
                units.live.push(unit);
            }
            missing
        } else {
            0
        };
        let attempt = units.restart.attempts();
        drop(units);
        Self::report_exits(dead, restart_in).await;
        if restarted > 0 {
            info!(restarted, attempt, "sink workers restarted");
        }
    }
}

#[async_trait]
pub trait WorkerManager: Send + Sync {
    async fn close_all_sink(&self);
    async fn add(&self, conf: config::sink::Sink) -> Result<()>;
    async fn close_sink(&self, id: Uuid) -> Result<()>;
    async fn remove_orphaned_data(&self);
}

pub struct WorkerGroup {
    sinks: RwLock<HashMap<Uuid, Worker>>,
    channel: chan::AdminChannel<Uuid, payload::Payload>,
    bg_token: CancellationToken,
}

#[async_trait]
impl WorkerManager for WorkerGroup {
    async fn close_all_sink(&self) {
        WorkerGroup::close_all_sink(self).await;
    }

    async fn add(&self, conf: config::sink::Sink) -> Result<()> {
        WorkerGroup::add(self, conf).await
    }

    async fn close_sink(&self, id: Uuid) -> Result<()> {
        WorkerGroup::close_sink(self, id).await
    }

    async fn remove_orphaned_data(&self) {
        WorkerGroup::remove_orphaned_data(self).await;
    }
}

impl WorkerGroup {
    pub fn new(channel: chan::AdminChannel<Uuid, payload::Payload>) -> Arc<Self> {
        Arc::new(Self {
            sinks: Default::default(),
            channel,
            bg_token: CancellationToken::new(),
        })
    }

    pub async fn spawn_metrics_reporter(
        self: Arc<Self>,
        metrics_report_duration: Duration,
    ) -> JoinHandle<()> {
        let token_clone = self.bg_token.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(metrics_report_duration) => {
                        let metrics = self.metrics().await;
                        INTERNAL_METRICS.sink.store(Arc::new((Instant::now(), metrics)));
                    }
                    _ = token_clone.cancelled() => break,
                }
            }
        })
    }

    /// Restores the worker count of every sink whose workers exit on their own
    /// (see [`Worker::supervise`]). Stopped by [`close`](Self::close).
    pub fn spawn_supervisor(self: Arc<Self>) -> JoinHandle<()> {
        let token_clone = self.bg_token.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(SUPERVISE_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        for sink in self.sinks.read().await.values() {
                            sink.supervise().await;
                        }
                    }
                    _ = token_clone.cancelled() => break,
                }
            }
        })
    }

    pub async fn metrics(&self) -> Vec<Metrics> {
        let sinks = self.sinks.read().await;
        let mut metrics = Vec::with_capacity(sinks.len());
        for sink in sinks.values() {
            metrics.push(sink.metrics());
        }
        metrics
    }

    pub async fn close_sink(&self, id: Uuid) -> Result<()> {
        // Out of the map before its channel goes: removing the topic
        // disconnects the workers, which then drain and return on their own,
        // and the supervisor must not see that as an exit to restart.
        if let Some(sink) = self.sinks.write().await.remove(&id) {
            self.channel.remove(&id).await?;
            sink.close(Input::Ends, Deadline::NONE).await;
            info!(sink_id = %id, sink = %sink.conf.name, "sink removed");
            Ok(())
        } else {
            Err(Error::new(SinkError::SinkDoesNotExist))
        }
    }

    /// Closes every sink while the node keeps running (it lost its role, or
    /// re-reads its config). Same order as `close_sink`: out of the map, then
    /// disconnect. The topics are closed, not removed: a disk backlog is this
    /// node's alone, and is picked up again when the sinks are added back.
    pub async fn close_all_sink(&self) {
        let sinks = {
            let mut sinks = self.sinks.write().await;
            std::mem::take(&mut *sinks)
        };
        self.channel.close_all().await;
        let mut set = tokio::task::JoinSet::new();
        for (_, sink) in sinks {
            set.spawn(async move { sink.close(Input::Ends, Deadline::NONE).await });
        }
        set.join_all().await;
    }

    /// Deletes the disk data of sinks that are gone. `close_sink` deletes it
    /// when a sink is removed while this node runs and watches the config;
    /// one removed while the node was down, or not main, leaves its keyspace
    /// behind. Call it once every configured sink is added: a keyspace named
    /// by a UUID is a sink's (other topics have fixed names), and one whose
    /// sink is not here is deleted. A sink switched to a memory channel keeps
    /// its keyspace, for a switch back.
    pub async fn remove_orphaned_data(&self) {
        let ids: HashSet<Uuid> = self.sinks.read().await.keys().copied().collect();
        match chan::remove_keyspaces(move |name| is_orphaned(&ids, name)).await {
            Ok(removed) if removed.is_empty() => {}
            Ok(removed) => {
                info!(sinks = removed.len(), "disk data of removed sinks deleted");
                debug!(sink_ids = ?removed, "deleted keyspaces of removed sinks");
            }
            Err(e) => {
                warn!(error = %format!("{e:#}"), "failed to delete disk data of removed sinks")
            }
        }
    }

    /// Closes every sink at shutdown, once nothing feeds them any more: their
    /// topics are closed, so each delivers what is queued and returns, and a
    /// disk topic keeps its backlog for the next start. Each sink's close
    /// phases end by `deadline`.
    pub async fn close(&self, deadline: Deadline) {
        let mut sinks = self.sinks.write().await;
        self.channel.close_all().await;
        let mut set = tokio::task::JoinSet::new();
        for (_, sink) in sinks.drain() {
            set.spawn(async move { sink.close(Input::Ends, deadline).await });
        }
        self.bg_token.cancel();
        drop(sinks);
        set.join_all().await;
    }

    pub async fn adjust(&self, id: Uuid, count: usize) -> Result<()> {
        if let Some(sink) = self.sinks.read().await.get(&id) {
            sink.adjust(count).await;
            Ok(())
        } else {
            Err(Error::new(SinkError::SinkDoesNotExist))
        }
    }

    pub async fn add(&self, conf: config::sink::Sink) -> Result<()> {
        let mut sinks = self.sinks.write().await;
        let id = conf.id;
        let worker_count = conf.worker_count;
        // Each branch builds the exporter first: a config it cannot build
        // fails there, with the running sink and its topic untouched.
        if let Some(existing) = sinks.get(&id) {
            if existing.conf.channel != conf.channel {
                let prepared = Prepared::new(&conf).await?;
                // Producers move to the new topic at once; the old one only
                // drains into the old consumer from here on.
                let consumer = self.channel.replace(&id, conf.channel).await?;
                let old = existing.sink.get_channel();
                existing.close(Input::HandedOver, Deadline::NONE).await;
                let sink = Worker::new(conf, prepared, consumer.clone());
                sink.adjust(worker_count).await;
                // What the old topic still held, and what its workers
                // requeued on their way out. Ends once the old topic's
                // producers (its disk bridge included) are gone; a disk
                // backlog stays in the keyspace the new topic reads.
                let mut moved = 0;
                while let Some(delivery) = old.recv().await {
                    consumer.adopt(&old, delivery).await;
                    moved += 1;
                }
                info!(sink_id = %id, sink = %sink.conf.name, payloads_moved = moved, "sink channel replaced");
                sinks.insert(id, sink);
            } else if existing.conf.config != conf.config
                || existing.conf.retry != conf.retry
                || existing.conf.batch != conf.batch
            {
                let prepared = Prepared::new(&conf).await?;
                // The consumer carries what the old workers requeue on their
                // way out over to the new ones.
                let consumer = existing.sink.get_channel();
                existing.close(Input::HandedOver, Deadline::NONE).await;
                let sink = Worker::new(conf, prepared, consumer);
                sink.adjust(worker_count).await;
                info!(sink_id = %id, sink = %sink.conf.name, "sink replaced");
                sinks.insert(id, sink);
            } else {
                existing.adjust(worker_count).await;
            }
        } else {
            let prepared = Prepared::new(&conf).await?;
            let consumer = self.channel.get_consumer(&id, conf.channel).await?;
            let sink = Worker::new(conf, prepared, consumer);
            sink.adjust(worker_count).await;
            info!(sink_id = %id, sink = %sink.conf.name, "sink added");
            sinks.insert(id, sink);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::blob_items;

    mod supervise {
        use super::super::{Metrics, Sink, Worker};
        use crate::{chan, config, payload};
        use async_trait::async_trait;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;
        use tokio_util::sync::CancellationToken;

        /// Returns straight away, as a real sink does once its channel is
        /// disconnected.
        #[derive(Default)]
        struct Fake {
            runs: AtomicUsize,
            closes: AtomicUsize,
        }

        #[async_trait]
        impl Sink for Fake {
            async fn run(&self, _token: CancellationToken) {
                self.runs.fetch_add(1, Ordering::Relaxed);
            }

            fn metrics(&self) -> Metrics {
                unreachable!("not reported in these tests")
            }

            fn get_channel(&self) -> chan::Consumer<payload::Payload> {
                unreachable!("not replaced in these tests")
            }

            async fn close(&self) {
                self.closes.fetch_add(1, Ordering::Relaxed);
            }
        }

        pub(super) fn conf() -> config::sink::Sink {
            serde_yaml_ng::from_str(
                r#"
id: 00000000-0000-0000-0000-000000000001
name: fake
worker_count: 1
type: file
config:
  path: /dev/null
"#,
            )
            .unwrap()
        }

        #[tokio::test(start_paused = true)]
        async fn exited_units_restart_until_the_sink_is_closed() {
            let fake = Arc::new(Fake::default());
            let worker = Worker::with_sink(conf(), fake.clone());
            worker.adjust(1).await;
            tokio::time::sleep(Duration::from_millis(1)).await;
            worker.supervise().await;
            tokio::time::advance(Duration::from_secs(1)).await;
            worker.supervise().await;
            tokio::time::sleep(Duration::from_millis(1)).await;
            assert_eq!(fake.runs.load(Ordering::Relaxed), 2);

            worker
                .close(
                    super::super::Input::Ends,
                    crate::common::shutdown::Deadline::NONE,
                )
                .await;
            tokio::time::advance(Duration::from_secs(5)).await;
            worker.supervise().await;
            tokio::time::sleep(Duration::from_millis(1)).await;
            assert_eq!(fake.runs.load(Ordering::Relaxed), 2);
            assert_eq!(fake.closes.load(Ordering::Relaxed), 1);
        }
    }

    mod replace {
        use super::super::WorkerGroup;
        use crate::{chan, config, payload};
        use arc_swap::ArcSwap;
        use otel_arrow_dfe_pdata::encode::encode_logs_otap_batch;
        use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
            LogRecord, LogsData, ResourceLogs, ScopeLogs,
        };
        use std::time::{Duration, Instant};
        use uuid::Uuid;

        fn logs(n: usize) -> payload::Payload {
            let req = LogsData {
                resource_logs: vec![ResourceLogs {
                    resource: None,
                    scope_logs: vec![ScopeLogs {
                        scope: None,
                        log_records: (0..n).map(|_| LogRecord::default()).collect(),
                        schema_url: String::new(),
                    }],
                    schema_url: String::new(),
                }],
            };
            payload::Payload::new(encode_logs_otap_batch(&req).unwrap())
        }

        fn sink(id: Uuid, channel: &str) -> config::sink::Sink {
            sink_at(id, channel, "/dev/null")
        }

        fn sink_at(id: Uuid, channel: &str, path: &str) -> config::sink::Sink {
            let mut conf: config::sink::Sink = serde_yaml_ng::from_str(&format!(
                "id: {id}\nname: replaced\nworker_count: 1\nchannel:\n{channel}\n\
                 type: file\nconfig:\n  path: {path}\n"
            ))
            .unwrap();
            conf.validate().unwrap();
            conf
        }

        async fn sent(group: &WorkerGroup) -> u64 {
            group.metrics().await[0].total_records_sent
        }

        /// A channel change takes effect on the live sink: what the old
        /// topic still queued is delivered by the new sink, and producers
        /// keep sending to the same id throughout.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn a_channel_change_replaces_the_sink_and_keeps_its_queue() {
            let dir = std::env::temp_dir()
                .join(format!("patrolli-sink-replace-{}", Uuid::now_v7().simple()));
            std::fs::create_dir_all(&dir).unwrap();
            chan::init_database(&config::global::DiskChannel {
                data_dir: dir.to_string_lossy().into_owned(),
                max_journal_size: 0,
                cache_size: 0,
            })
            .unwrap();
            let remote =
                std::sync::Arc::new(ArcSwap::from_pointee((Instant::now(), Default::default())));
            let (admin, sender) = chan::new(Duration::from_secs(60), remote);
            let group = WorkerGroup::new(admin);
            let id = Uuid::now_v7();

            // No workers yet: everything waits in the memory topic.
            let mut idle = sink(id, "  type: memory\n  channel_buffer_size: 16");
            idle.worker_count = 0;
            group.add(idle).await.unwrap();
            for _ in 0..5 {
                sender.send_to(&id, logs(2)).unwrap();
            }
            assert_eq!(sent(&group).await, 0);

            group
                .add(sink(id, "  type: disk\n  write_through: true"))
                .await
                .unwrap();
            sender.send_to(&id, logs(3)).unwrap();
            tokio::time::timeout(Duration::from_secs(10), async {
                while sent(&group).await < 13 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("the old queue was not delivered by the new sink");
            assert_eq!(sent(&group).await, 13);
            group.close_sink(id).await.unwrap();
        }

        /// A channel change whose sink cannot be built fails before it
        /// touches anything: the topic keeps taking and queueing, and the
        /// next good change delivers all of it.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn a_channel_change_that_cannot_build_leaves_the_sink_as_it_was() {
            let remote =
                std::sync::Arc::new(ArcSwap::from_pointee((Instant::now(), Default::default())));
            let (admin, sender) = chan::new(Duration::from_secs(60), remote);
            let group = WorkerGroup::new(admin);
            let id = Uuid::now_v7();

            let mut idle = sink(id, "  type: memory\n  channel_buffer_size: 16");
            idle.worker_count = 0;
            group.add(idle).await.unwrap();
            sender.send_to(&id, logs(2)).unwrap();

            let bad = sink_at(
                id,
                "  type: memory\n  channel_buffer_size: 32",
                "/nonexistent-patrolli-dir/out",
            );
            assert!(group.add(bad).await.is_err());
            sender
                .send_to(&id, logs(3))
                .expect("the topic stopped taking after a failed change");
            assert_eq!(
                group.sinks.read().await[&id].conf.channel,
                sink(id, "  type: memory\n  channel_buffer_size: 16").channel
            );

            group
                .add(sink(id, "  type: memory\n  channel_buffer_size: 32"))
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(10), async {
                while sent(&group).await < 5 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("what the topic queued was not delivered");
            group.close_sink(id).await.unwrap();
        }
    }

    mod close {
        use super::super::{Input, Metrics, Sink, Worker};
        use crate::common::shutdown::Deadline;
        use crate::{chan, config, payload};
        use async_trait::async_trait;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;
        use tokio::time::Instant;
        use tokio_util::sync::CancellationToken;

        /// Takes numbers off its input the way a real sink takes payloads:
        /// biased to its token, done once the input disconnects.
        struct Fake {
            input: flume::Receiver<u32>,
            delivered: AtomicUsize,
            closes: AtomicUsize,
            stuck_run: bool,
            stuck_close: bool,
        }

        #[async_trait]
        impl Sink for Fake {
            async fn run(&self, token: CancellationToken) {
                if self.stuck_run {
                    std::future::pending::<()>().await;
                }
                loop {
                    tokio::select! {
                        biased;

                        _ = token.cancelled() => break,
                        msg = self.input.recv_async() => match msg {
                            Ok(_) => {
                                self.delivered.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(_) => break,
                        },
                    }
                }
            }

            fn metrics(&self) -> Metrics {
                unreachable!("not reported in these tests")
            }

            fn get_channel(&self) -> chan::Consumer<payload::Payload> {
                unreachable!("not replaced in these tests")
            }

            async fn close(&self) {
                self.closes.fetch_add(1, Ordering::Relaxed);
                if self.stuck_close {
                    std::future::pending::<()>().await;
                }
            }
        }

        fn worker(stuck_run: bool, stuck_close: bool) -> (Worker, Arc<Fake>, flume::Sender<u32>) {
            let (tx, input) = flume::bounded(16);
            let fake = Arc::new(Fake {
                input,
                delivered: AtomicUsize::new(0),
                closes: AtomicUsize::new(0),
                stuck_run,
                stuck_close,
            });
            let worker = Worker::with_sink(super::supervise::conf(), fake.clone());
            (worker, fake, tx)
        }

        fn delivered(fake: &Fake) -> usize {
            fake.delivered.load(Ordering::Relaxed)
        }

        // The units are spawned but have not run yet when `close` starts:
        // whatever they deliver, they deliver during the close.

        #[tokio::test(start_paused = true)]
        async fn an_ending_input_is_delivered_before_the_sink_closes() {
            let (worker, fake, tx) = worker(false, false);
            for i in 0..5 {
                tx.send(i).unwrap();
            }
            worker.adjust(2).await;
            drop(tx);
            worker.close(Input::Ends, Deadline::NONE).await;
            assert_eq!(delivered(&fake), 5);
            assert_eq!(fake.closes.load(Ordering::Relaxed), 1);
        }

        #[tokio::test(start_paused = true)]
        async fn a_handed_over_input_keeps_its_queue() {
            let (worker, fake, tx) = worker(false, false);
            for i in 0..5 {
                tx.send(i).unwrap();
            }
            worker.adjust(2).await;
            worker.close(Input::HandedOver, Deadline::NONE).await;
            assert_eq!(delivered(&fake), 0);
            assert_eq!(tx.len(), 5);
        }

        #[tokio::test(start_paused = true)]
        async fn stuck_workers_are_aborted_and_the_sink_still_closes() {
            let (worker, fake, tx) = worker(true, false);
            worker.adjust(2).await;
            drop(tx);
            let started = Instant::now();
            worker.close(Input::Ends, Deadline::NONE).await;
            // a worker stuck past its cancellation gets the grace, then is aborted
            assert_eq!(
                started.elapsed(),
                worker.drain_timeout + worker.cancel_grace
            );
            assert_eq!(fake.closes.load(Ordering::Relaxed), 1);
        }

        #[tokio::test(start_paused = true)]
        async fn a_stuck_sink_close_is_cut_short() {
            let (worker, fake, tx) = worker(false, true);
            worker.adjust(1).await;
            drop(tx);
            let started = Instant::now();
            worker.close(Input::Ends, Deadline::NONE).await;
            assert_eq!(started.elapsed(), worker.drain_timeout);
            assert_eq!(fake.closes.load(Ordering::Relaxed), 1);
        }

        /// A destination that accepts the connection, then stops reading:
        /// once the stream's buffers fill, its connection task blocks
        /// handing tonic a batch and no longer takes from the worker, so the
        /// worker blocks too. Only the sink's close, which waits on that
        /// worker, would stop the connection task.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn a_stalled_otap_destination_does_not_hold_the_close() {
            use otel_arrow_dfe_pdata::encode::encode_logs_otap_batch;
            use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::{AnyValue, any_value};
            use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
                LogRecord, LogsData, ResourceLogs, ScopeLogs,
            };
            let stalled = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = stalled.local_addr().unwrap();
            // Accepts and holds every connection, never reading a byte.
            std::thread::spawn(move || {
                let mut held = Vec::new();
                for stream in stalled.incoming() {
                    held.push(stream);
                }
            });
            let conf: config::sink::Sink = serde_yaml_ng::from_str(&format!(
                r#"
id: 00000000-0000-0000-0000-000000000002
name: otap
worker_count: 1
batch_size: 1
type: open_telemetry
config:
  destinations: ["{addr}"]
  connection_count: 1
"#
            ))
            .unwrap();
            let (tx, rx) = flume::bounded(1);
            let consumer = chan::Consumer::from_flume(&tx, rx);
            let sink = super::super::otap::PatrolliSink::new(conf.clone(), consumer)
                .await
                .unwrap();
            let mut worker = Worker::with_sink(conf, Arc::new(sink));
            worker.drain_timeout = Duration::from_millis(300);
            worker.adjust(1).await;
            let body = "x".repeat(1024);
            let logs = LogsData {
                resource_logs: vec![ResourceLogs {
                    resource: None,
                    scope_logs: vec![ScopeLogs {
                        scope: None,
                        log_records: (0..64)
                            .map(|_| LogRecord {
                                body: Some(AnyValue {
                                    value: Some(any_value::Value::StringValue(body.clone())),
                                }),
                                ..Default::default()
                            })
                            .collect(),
                        schema_url: String::new(),
                    }],
                    schema_url: String::new(),
                }],
            };
            let encoded = encode_logs_otap_batch(&logs).unwrap();
            // Feed it until the sink stops taking: the destination is
            // connected and backed up all the way to the worker.
            let mut stalled_at = None;
            for sent in 0..10_000 {
                let pl = payload::Payload::new(encoded.clone());
                if tokio::time::timeout(Duration::from_millis(500), tx.send_async(pl))
                    .await
                    .is_err()
                {
                    stalled_at = Some(sent);
                    break;
                }
            }
            assert!(stalled_at.is_some(), "the destination never pushed back");
            drop(tx);
            let started = std::time::Instant::now();
            tokio::time::timeout(
                Duration::from_secs(5),
                worker.close(Input::Ends, Deadline::NONE),
            )
            .await
            .expect("close waited on the stalled destination");
            assert!(started.elapsed() < Duration::from_secs(3));
        }
    }

    #[test]
    fn only_the_keyspace_of_a_sink_that_is_gone_is_orphaned() {
        use uuid::Uuid;
        let kept = Uuid::now_v7();
        let gone = Uuid::now_v7();
        let ids = std::iter::once(kept).collect();
        assert!(super::is_orphaned(&ids, &gone.to_string()));
        assert!(!super::is_orphaned(&ids, &kept.to_string()));
        // Other topics' keyspaces, and names that only parse as a UUID.
        for name in [
            "processor".to_string(),
            "forwarder".to_string(),
            gone.simple().to_string(),
            gone.to_string().to_uppercase(),
            format!("replace-disk-{}", gone.simple()),
        ] {
            assert!(!super::is_orphaned(&ids, &name), "{name}");
        }
    }

    #[test]
    fn batch_format_attributes_every_item_to_its_single_blob() {
        assert_eq!(blob_items(12, 1, 0), 12);
        assert_eq!(blob_items(0, 1, 0), 0);
    }

    #[test]
    fn denormalized_blobs_sum_to_the_item_count() {
        for (items, blobs) in [(12u64, 12usize), (7, 3), (3, 7), (0, 4), (1, 1)] {
            let total: u64 = (0..blobs).map(|idx| blob_items(items, blobs, idx)).sum();
            assert_eq!(total, items, "items={items} blobs={blobs}");
        }
        // one item per blob when the counts line up, as for logs and traces
        assert!((0..12).all(|idx| blob_items(12, 12, idx) == 1));
        // the remainder lands on the last blob
        assert_eq!(blob_items(7, 3, 0), 2);
        assert_eq!(blob_items(7, 3, 2), 3);
    }

    #[test]
    fn no_blobs_means_nothing_to_attribute() {
        assert_eq!(blob_items(5, 0, 0), 0);
    }
}
