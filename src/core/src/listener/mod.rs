// SPDX-FileCopyrightText: 2025 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::chan::{Error as ChanError, SenderChannel};
use crate::common::HasLength;
use crate::common::backoff::RestartBackoff;
use crate::common::internal_metrics::{INTERNAL_METRICS, Listener as Metrics};
use crate::common::shutdown::Deadline;
use crate::{cache, config, payload};
use anyhow::{Error, Result};
use async_trait::async_trait;
use rustc_hash::FxHashMap as HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use uuid::Uuid;

pub mod dummy;
pub mod netflow;
pub mod syslog;

mod annotate;
mod internal_logs;
mod internal_metrics;
mod otel;
mod snmp_trap;
#[cfg(test)]
mod test_util;

pub use annotate::{ListenerAnnotate, item_listener_annotate};

/// How often the supervisor looks for workers that exited on their own.
const SUPERVISE_INTERVAL: Duration = Duration::from_secs(1);
/// Longest a close (or a scale-down) waits for the transport to hand over what
/// it holds and for the workers to deliver it. Past it the workers are aborted
/// and the rest is dropped, so a stuck pipeline cannot hold shutdown.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Error, Debug)]
pub enum ListenerError {
    #[error("Listener already exists")]
    ListenerAlreadyExists,
    #[error("Listener doesn't exist")]
    ListenerDoesNotExist,
}

#[derive(Default, Debug)]
struct InternalMetrics {
    total_workers: AtomicUsize,
    total_time_spent: AtomicU64,
    engine_time_spent: AtomicU64,
    receive_wait_time: AtomicU64,
    send_wait_time: AtomicU64,
    total_bytes: AtomicU64,
    total_bytes_succeeded: AtomicU64,
    total_bytes_dropped: AtomicU64,
    total_bytes_errored: AtomicU64,
    total_items: AtomicU64,
    total_items_succeeded: AtomicU64,
    total_items_dropped: AtomicU64,
    total_items_errored: AtomicU64,
    total_records: AtomicU64,
    total_records_sent: AtomicU64,
    total_records_forwarded: AtomicU64,
    total_records_forward_failed: AtomicU64,
    total_records_annotate_failed: AtomicU64,
    total_records_dropped: AtomicU64,
    total_records_errored: AtomicU64,
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
            total_bytes: self.total_bytes.load(Ordering::Relaxed),
            total_bytes_succeeded: self.total_bytes_succeeded.load(Ordering::Relaxed),
            total_bytes_dropped: self.total_bytes_dropped.load(Ordering::Relaxed),
            total_bytes_errored: self.total_bytes_errored.load(Ordering::Relaxed),
            total_items: self.total_items.load(Ordering::Relaxed),
            total_items_succeeded: self.total_items_succeeded.load(Ordering::Relaxed),
            total_items_dropped: self.total_items_dropped.load(Ordering::Relaxed),
            total_items_errored: self.total_items_errored.load(Ordering::Relaxed),
            total_records: self.total_records.load(Ordering::Relaxed),
            total_records_sent: self.total_records_sent.load(Ordering::Relaxed),
            total_records_forwarded: self.total_records_forwarded.load(Ordering::Relaxed),
            total_records_forward_failed: self.total_records_forward_failed.load(Ordering::Relaxed),
            total_records_annotate_failed: self
                .total_records_annotate_failed
                .load(Ordering::Relaxed),
            total_records_dropped: self.total_records_dropped.load(Ordering::Relaxed),
            total_records_errored: self.total_records_errored.load(Ordering::Relaxed),
        }
    }
}

#[async_trait]
pub trait Listener: Send + Sync {
    /// One worker. Returns once `token` is cancelled, or once its input ends:
    /// after [`stop`](Self::stop), or when the transport died, in which case
    /// the supervisor restarts it.
    async fn listen(&self, token: CancellationToken);
    async fn metrics(&self) -> Metrics;
    /// Stops the intake the workers read, producers first: once it returns,
    /// nothing is left to feed the workers but what is already queued, the
    /// listener's sockets are released, and no later `listen` starts them
    /// again. The workers are still running meanwhile, so the hand-over can
    /// complete. Called once, when the listener is closed.
    async fn stop(&self) {}
}

struct WorkerUnit {
    token: CancellationToken,
    handle: JoinHandle<()>,
}

/// The running units of a listener and the count the supervisor keeps them at.
#[derive(Default)]
struct Units {
    live: Vec<WorkerUnit>,
    desired: usize,
    restart: RestartBackoff,
}

impl Units {
    /// Takes out the units whose task already returned. Units leave `live`
    /// before they are cancelled, so every one of these exited on its own.
    fn reap(&mut self) -> Vec<WorkerUnit> {
        self.live
            .extract_if(.., |unit| unit.handle.is_finished())
            .collect()
    }
}

struct Worker {
    listener: Arc<dyn Listener>,
    units: Mutex<Units>,
    conf: config::listener::Listener,
    /// [`DRAIN_TIMEOUT`]; tests shorten it.
    drain_timeout: Duration,
}

impl Worker {
    async fn new(
        conf: config::listener::Listener,
        channel: SenderChannel<&'static str, payload::ProcessorPayload>,
        forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
        conf_cache: cache::Cache,
        node_id: Option<Uuid>,
    ) -> Result<Self> {
        let listener: Arc<dyn Listener> = match &conf.config {
            config::listener::Config::Dummy(_) => Arc::new(dummy::Dummy::new(
                conf.clone(),
                channel,
                forwarder,
                conf_cache,
                node_id,
            )),
            config::listener::Config::Netflow(_) => Arc::new(netflow::Netflow::new(
                conf.clone(),
                channel,
                forwarder,
                conf_cache,
                node_id,
            )),
            config::listener::Config::SnmpTrap(_) => Arc::new(snmp_trap::SnmpTrap::new(
                conf.clone(),
                channel,
                forwarder,
                conf_cache,
                node_id,
            )),
            config::listener::Config::Syslog(_) => Arc::new(syslog::SyslogListener::new(
                conf.clone(),
                channel,
                forwarder,
                conf_cache,
                node_id,
            )?),
            config::listener::Config::OpenTelemetry(_) => Arc::new(otel::OtelListener::new(
                conf.clone(),
                channel,
                forwarder,
                conf_cache,
                node_id,
            )),
            config::listener::Config::InternalLogs(_) => {
                Arc::new(internal_logs::InternalLogsListener::new(
                    conf.clone(),
                    channel,
                    forwarder,
                    conf_cache,
                    node_id,
                ))
            }
            config::listener::Config::InternalMetrics(_) => {
                Arc::new(internal_metrics::InternalMetricsListener::new(
                    conf.clone(),
                    channel,
                    forwarder,
                    conf_cache,
                    node_id,
                ))
            }
        };
        Ok(Self::with_listener(conf, listener))
    }

    fn with_listener(conf: config::listener::Listener, listener: Arc<dyn Listener>) -> Self {
        Self {
            listener,
            units: Mutex::new(Units::default()),
            conf,
            drain_timeout: DRAIN_TIMEOUT,
        }
    }

    async fn metrics(&self) -> Metrics {
        self.listener.metrics().await
    }

    fn spawn_unit(&self) -> WorkerUnit {
        let token = CancellationToken::new();
        let token_clone = token.clone();
        let listener = self.listener.clone();
        let handle = tokio::spawn(async move {
            listener.listen(token_clone).await;
        });
        WorkerUnit { token, handle }
    }

    /// Cancels `units` and waits for them to deliver what they hold. With
    /// `stop`, the listener's intake is stopped first and hands over what it
    /// holds while the units still run. The whole of it is bounded by
    /// `drain_timeout`, or `deadline` when that comes first, past which the
    /// units are aborted; a `stop` cut short drops its transport, which
    /// aborts the transport's tasks.
    async fn close_units(&self, mut units: Vec<WorkerUnit>, stop: bool, deadline: Deadline) {
        let limit = deadline.budget(self.drain_timeout);
        let drained = tokio::time::timeout(limit, async {
            if stop {
                self.listener.stop().await;
            }
            for unit in units.iter() {
                unit.token.cancel();
            }
            for unit in units.iter_mut() {
                if let Err(e) = (&mut unit.handle).await {
                    debug!(error = %e, "listener worker panicked");
                }
            }
        })
        .await;
        if drained.is_err() {
            warn!(
                timeout_ms = limit.as_millis() as u64,
                "listener drain timed out, aborting workers"
            );
            for unit in units {
                unit.handle.abort();
            }
        }
    }

    async fn close(&self, deadline: Deadline) {
        let units = {
            let mut units = self.units.lock().await;
            units.desired = 0;
            std::mem::take(&mut units.live)
        };
        self.close_units(units, true, deadline).await;
    }

    async fn adjust(&self, count: usize) {
        let mut units = self.units.lock().await;
        units.desired = count;
        // Dead units are replaced here rather than counted as live; their exit
        // was already reported by `supervise` or is reported on its next tick.
        let dead = units.reap();
        let units_len = units.live.len();
        if count > units_len {
            for _ in 0..(count - units_len) {
                let unit = self.spawn_unit();
                units.live.push(unit);
            }
        } else if count < units_len {
            let units_to_close = units.live.drain(..(units_len - count)).collect();
            self.close_units(units_to_close, false, Deadline::NONE)
                .await;
        }
        drop(units);
        Self::report_exits(dead, Duration::ZERO).await;
    }

    /// Logs the units that exited on their own, once each.
    async fn report_exits(dead: Vec<WorkerUnit>, restart_in: Duration) {
        for unit in dead {
            match unit.handle.await {
                Ok(()) => warn!(restart_in = ?restart_in, "listener worker exited unexpectedly"),
                // Reported by the panic hook, which also started the
                // shutdown: see `common::panic`.
                Err(e) => debug!(error = %e, "listener worker panicked"),
            }
        }
    }

    /// Replaces units that returned without being cancelled, pacing restarts
    /// with [`RestartBackoff`]. Skips the tick when `adjust` or `close` holds
    /// the units.
    #[tracing::instrument(skip_all, fields(listener_id = %self.conf.id, listener = %self.conf.name))]
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
        // Units sharing a socket tend to die together; that is one failure,
        // not one per unit, so it advances the backoff once.
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
            info!(restarted, attempt, "listener workers restarted");
        }
    }
}

#[async_trait]
pub trait WorkerManager: Send + Sync {
    async fn close_all_listener(&self);
    async fn add(&self, conf: config::listener::Listener) -> Result<()>;
    async fn close_listener(&self, id: Uuid) -> Result<()>;
}

pub struct WorkerGroup {
    listeners: RwLock<HashMap<Uuid, Worker>>,
    conf_cache: cache::Cache,
    channel: SenderChannel<&'static str, payload::ProcessorPayload>,
    forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
    node_id: Option<Uuid>,
    bg_token: CancellationToken,
}

#[async_trait]
impl WorkerManager for WorkerGroup {
    async fn close_all_listener(&self) {
        WorkerGroup::close_all_listener(self).await
    }

    async fn add(&self, conf: config::listener::Listener) -> Result<()> {
        WorkerGroup::add(self, conf).await
    }

    async fn close_listener(&self, id: Uuid) -> Result<()> {
        WorkerGroup::close_listener(self, id).await
    }
}

impl WorkerGroup {
    pub fn new(
        channel: SenderChannel<&'static str, payload::ProcessorPayload>,
        forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
        conf_cache: cache::Cache,
        node_id: Option<Uuid>,
    ) -> Arc<Self> {
        Arc::new(Self {
            listeners: Default::default(),
            channel,
            forwarder,
            conf_cache,
            node_id,
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
                        INTERNAL_METRICS.listener.store(Arc::new((Instant::now(), metrics)));
                    }
                    _ = token_clone.cancelled() => break,
                }
            }
        })
    }

    /// Restores the worker count of every listener whose workers exit on their
    /// own (see [`Worker::supervise`]). Stopped by [`close`](Self::close).
    pub fn spawn_supervisor(self: Arc<Self>) -> JoinHandle<()> {
        let token_clone = self.bg_token.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(SUPERVISE_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        for listener in self.listeners.read().await.values() {
                            listener.supervise().await;
                        }
                    }
                    _ = token_clone.cancelled() => break,
                }
            }
        })
    }

    pub async fn metrics(&self) -> Vec<Metrics> {
        let listeners = self.listeners.read().await;
        let mut metrics = Vec::with_capacity(listeners.len());
        for listener in listeners.values() {
            metrics.push(listener.metrics().await);
        }
        metrics
    }

    pub async fn close_listener(&self, id: Uuid) -> Result<()> {
        if let Some(listener) = self.listeners.write().await.remove(&id) {
            listener.close(Deadline::NONE).await;
            debug!(listener_id = %id, listener = %listener.conf.name, "listener removed");
            Ok(())
        } else {
            Err(Error::new(ListenerError::ListenerDoesNotExist))
        }
    }

    pub async fn close_all_listener(&self) {
        let listeners = {
            let mut listeners = self.listeners.write().await;
            std::mem::take(&mut *listeners)
        };
        let mut set = tokio::task::JoinSet::new();
        for (_, listener) in listeners {
            set.spawn(async move { listener.close(Deadline::NONE).await });
        }
        set.join_all().await;
    }

    pub async fn close(&self, deadline: Deadline) {
        let mut listeners = self.listeners.write().await;
        let mut set = tokio::task::JoinSet::new();
        for (_, listener) in listeners.drain() {
            set.spawn(async move { listener.close(deadline).await });
        }
        self.bg_token.cancel();
        drop(listeners);
        set.join_all().await;
    }

    pub async fn adjust(&self, id: Uuid, count: usize) -> Result<()> {
        if let Some(listener) = self.listeners.read().await.get(&id) {
            listener.adjust(count).await;
            Ok(())
        } else {
            Err(Error::new(ListenerError::ListenerDoesNotExist))
        }
    }

    pub async fn add(&self, conf: config::listener::Listener) -> Result<()> {
        let mut listeners = self.listeners.write().await;
        let id = conf.id;
        let worker_count = conf.worker_count;
        if let Some(existing) = listeners.get(&id) {
            if existing.conf.config != conf.config || existing.conf.batch != conf.batch {
                debug!(listener_id = %id, listener = %conf.name, "listener config changed, restarting");
                existing.close(Deadline::NONE).await;
                let listener = Worker::new(
                    conf,
                    self.channel.clone(),
                    self.forwarder.clone(),
                    self.conf_cache.clone(),
                    self.node_id,
                )
                .await?;
                listener.adjust(worker_count).await;
                listeners.insert(id, listener);
            } else {
                debug!(listener_id = %id, listener = %conf.name, workers = worker_count, "listener workers adjusted");
                existing.adjust(worker_count).await;
            }
        } else {
            debug!(listener_id = %id, listener = %conf.name, "listener added");
            let listener = Worker::new(
                conf,
                self.channel.clone(),
                self.forwarder.clone(),
                self.conf_cache.clone(),
                self.node_id,
            )
            .await?;
            listener.adjust(worker_count).await;
            listeners.insert(id, listener);
        }
        Ok(())
    }
}

struct SendBatch<'a> {
    batch: payload::Payload,
    conf: &'a config::listener::Listener,
    conf_cache: &'a cache::Cache,
    node_id: Option<Uuid>,
    internal_metrics: &'a InternalMetrics,
    channel: &'a SenderChannel<&'static str, payload::ProcessorPayload>,
    forwarder: &'a Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
}

/// Whether a send failed only for lack of room, i.e. the records were dropped
/// rather than lost to a fault.
fn is_buffer_full<M>(e: &ChanError<M>) -> bool {
    match e {
        ChanError::BufferFull(_) => true,
        ChanError::Multiple(errors) => errors.iter().all(is_buffer_full),
        _ => false,
    }
}

/// Hands one payload to the local processor channel and books the outcome:
/// `total_records_sent` once it is accepted, `total_records_dropped` when the
/// channel had no room, `total_records_errored` for any other failure.
async fn send_to_processor(
    internal_metrics: &InternalMetrics,
    channel: &SenderChannel<&'static str, payload::ProcessorPayload>,
    pl: payload::ProcessorPayload,
) {
    let pl_len = pl.len() as u64;
    match channel.broadcast(pl).await {
        Ok(()) => {
            internal_metrics
                .total_records_sent
                .fetch_add(pl_len, Ordering::Relaxed);
        }
        Err(e) if is_buffer_full(&e) => {
            internal_metrics
                .total_records_dropped
                .fetch_add(pl_len, Ordering::Relaxed);
        }
        Err(e) => {
            warn!(error = %e, "processor channel send failed");
            internal_metrics
                .total_records_errored
                .fetch_add(pl_len, Ordering::Relaxed);
        }
    }
}

async fn send_batch<'a>(args: SendBatch<'a>) {
    let n = Instant::now();
    let res = item_listener_annotate(
        &args.batch,
        Some(args.conf),
        args.conf_cache,
        args.node_id,
        args.forwarder.is_none(),
    );
    args.internal_metrics
        .engine_time_spent
        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
    let n = Instant::now();
    match res {
        Err(e) => {
            warn!(error = %e, "error while annotating item listen message");
            args.internal_metrics
                .total_records_annotate_failed
                .fetch_add(args.batch.num_items() as u64, Ordering::Relaxed);
            send_to_processor(
                args.internal_metrics,
                args.channel,
                payload::ProcessorPayload::new(None, args.batch),
            )
            .await;
        }
        Ok(res) => {
            if let Some(forwarder) = &args.forwarder {
                for pl in res.for_forward {
                    let pl_len = pl.len() as u64;
                    match forwarder.broadcast(pl).await {
                        Ok(()) => {
                            args.internal_metrics
                                .total_records_forwarded
                                .fetch_add(pl_len, Ordering::Relaxed);
                        }
                        // The forwarder handed the payload back: process it
                        // here rather than lose it.
                        Err(ChanError::BufferFull(contents))
                        | Err(ChanError::Closed(contents))
                        | Err(ChanError::TopicNotFound(Some(contents))) => {
                            args.internal_metrics
                                .total_records_forward_failed
                                .fetch_add(pl_len, Ordering::Relaxed);
                            send_to_processor(
                                args.internal_metrics,
                                args.channel,
                                payload::ProcessorPayload::new(None, contents.payload),
                            )
                            .await;
                        }
                        // The payload went down with the error.
                        Err(e) => {
                            warn!(error = %e, "error while forwarding item listen message");
                            args.internal_metrics
                                .total_records_forward_failed
                                .fetch_add(pl_len, Ordering::Relaxed);
                            args.internal_metrics
                                .total_records_errored
                                .fetch_add(pl_len, Ordering::Relaxed);
                        }
                    }
                }
            } else {
                for pl in res.for_forward {
                    args.internal_metrics
                        .total_records_forward_failed
                        .fetch_add(pl.len() as u64, Ordering::Relaxed);
                    send_to_processor(
                        args.internal_metrics,
                        args.channel,
                        payload::ProcessorPayload::new(None, pl.payload),
                    )
                    .await;
                }
            };
            for pl in res.for_process {
                send_to_processor(args.internal_metrics, args.channel, pl).await;
            }
        }
    }
    args.internal_metrics
        .send_wait_time
        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::is_buffer_full;
    use crate::chan::Error;

    mod supervise {
        use super::super::{Listener, Metrics, Worker};
        use crate::common::shutdown::Deadline;
        use crate::config;
        use async_trait::async_trait;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;
        use tokio_util::sync::CancellationToken;
        use uuid::Uuid;

        #[derive(Clone, Copy)]
        enum Exit {
            Returns,
            Panics,
            OnCancel,
            /// Ignores its token, like a worker stuck on a pipeline that never
            /// takes its batch.
            Never,
        }

        struct Fake {
            exit: Exit,
            starts: AtomicUsize,
        }

        #[async_trait]
        impl Listener for Fake {
            async fn listen(&self, token: CancellationToken) {
                self.starts.fetch_add(1, Ordering::Relaxed);
                match self.exit {
                    Exit::Returns => {}
                    Exit::Panics => panic!("fake listener panicked"),
                    Exit::OnCancel => token.cancelled().await,
                    Exit::Never => std::future::pending().await,
                }
            }

            async fn metrics(&self) -> Metrics {
                unreachable!("not reported in these tests")
            }
        }

        fn worker(exit: Exit) -> (Worker, Arc<Fake>) {
            let fake = Arc::new(Fake {
                exit,
                starts: AtomicUsize::new(0),
            });
            let conf = config::listener::Listener::new(
                Uuid::nil(),
                "fake".into(),
                1,
                config::Batch::default(),
                config::listener::Config::Dummy(config::listener::Dummy {
                    speed: 5,
                    messages_per_batch: 1,
                }),
                false,
            );
            (Worker::with_listener(conf, fake.clone()), fake)
        }

        /// Lets the spawned units run to their exit. Time is paused, so this
        /// advances the clock by far less than any restart delay.
        async fn settle() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        fn starts(fake: &Fake) -> usize {
            fake.starts.load(Ordering::Relaxed)
        }

        #[tokio::test(start_paused = true)]
        async fn exited_units_restart_with_growing_delay() {
            let (worker, fake) = worker(Exit::Returns);
            worker.adjust(2).await;
            settle().await;
            assert_eq!(starts(&fake), 2);

            // reaped, restart due in 1s
            worker.supervise().await;
            assert_eq!(starts(&fake), 2);
            tokio::time::advance(Duration::from_secs(1)).await;
            worker.supervise().await;
            settle().await;
            assert_eq!(starts(&fake), 4, "both units come back together");

            // second exit in a row: due in 2s
            worker.supervise().await;
            tokio::time::advance(Duration::from_secs(1)).await;
            worker.supervise().await;
            settle().await;
            assert_eq!(starts(&fake), 4);
            tokio::time::advance(Duration::from_secs(1)).await;
            worker.supervise().await;
            settle().await;
            assert_eq!(starts(&fake), 6);
        }

        #[tokio::test(start_paused = true)]
        async fn a_panicked_unit_is_restarted_and_close_does_not_panic() {
            let (worker, fake) = worker(Exit::Panics);
            worker.adjust(1).await;
            settle().await;
            worker.supervise().await;
            tokio::time::advance(Duration::from_secs(1)).await;
            worker.supervise().await;
            settle().await;
            assert_eq!(starts(&fake), 2);
            // the restarted unit panicked too and is still in `live`
            worker.close(Deadline::NONE).await;
        }

        #[tokio::test(start_paused = true)]
        async fn close_and_scale_to_zero_stop_restarts() {
            for scale_to_zero in [false, true] {
                let (worker, fake) = worker(Exit::Returns);
                worker.adjust(1).await;
                settle().await;
                if scale_to_zero {
                    worker.adjust(0).await;
                } else {
                    worker.close(Deadline::NONE).await;
                }
                tokio::time::advance(Duration::from_secs(5)).await;
                worker.supervise().await;
                settle().await;
                assert_eq!(starts(&fake), 1, "scale_to_zero={scale_to_zero}");
            }
        }

        #[tokio::test(start_paused = true)]
        async fn adjust_replaces_dead_units_right_away() {
            let (worker, fake) = worker(Exit::Returns);
            worker.adjust(1).await;
            settle().await;
            worker.adjust(1).await;
            settle().await;
            assert_eq!(starts(&fake), 2);
        }

        #[tokio::test(start_paused = true)]
        async fn close_aborts_units_that_outlive_the_drain_timeout() {
            for scale_down in [false, true] {
                let (worker, fake) = worker(Exit::Never);
                worker.adjust(1).await;
                settle().await;
                let started = tokio::time::Instant::now();
                if scale_down {
                    worker.adjust(0).await;
                } else {
                    worker.close(Deadline::NONE).await;
                }
                assert_eq!(started.elapsed(), worker.drain_timeout);
                assert_eq!(starts(&fake), 1);
            }
        }

        #[tokio::test(start_paused = true)]
        async fn running_units_are_left_alone() {
            let (worker, fake) = worker(Exit::OnCancel);
            worker.adjust(2).await;
            settle().await;
            for _ in 0..3 {
                tokio::time::advance(Duration::from_secs(1)).await;
                worker.supervise().await;
            }
            settle().await;
            assert_eq!(starts(&fake), 2);
            worker.close(Deadline::NONE).await;
        }
    }

    #[test]
    fn only_a_full_buffer_counts_as_dropped() {
        assert!(is_buffer_full(&Error::BufferFull(1u8)));
        assert!(!is_buffer_full(&Error::Closed(1u8)));
        assert!(!is_buffer_full(&Error::<u8>::TopicNotFound(None)));
        assert!(!is_buffer_full(&Error::<u8>::Internal(anyhow::anyhow!(
            "x"
        ))));
    }

    #[test]
    fn a_fan_out_is_dropped_only_when_every_topic_was_full() {
        assert!(is_buffer_full(&Error::Multiple(vec![
            Error::BufferFull(1u8),
            Error::BufferFull(2u8),
        ])));
        assert!(!is_buffer_full(&Error::Multiple(vec![
            Error::BufferFull(1u8),
            Error::Closed(2u8),
        ])));
    }
}
