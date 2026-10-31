// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

mod attr_input;
mod attr_write;
mod attribute_collect;
mod attribute_filter;
mod attribute_lookup;
mod attribute_transform;
#[cfg(test)]
mod test_alloc;
#[cfg(test)]
mod test_payloads;
mod transform;
#[cfg(test)]
mod worker_tests;

use crate::cache::processor::{Capture, Node, Step};
use crate::chan::{self, SenderChannel};
use crate::cluster::inter::v1::CaptureMessage;
use crate::common::HasLength;
use crate::common::internal_metrics::{
    INTERNAL_METRICS, Processor as ProcessorMetrics, ProcessorEdge as ProcessorEdgeMetrics,
    ProcessorNode as ProcessorNodeMetrics,
};
use crate::common::shutdown::Deadline;
use crate::config::Format;
use crate::config::processor::{
    AddAttributes, Config, DropAttributes, Processor, RenameAttributes,
};
use crate::payload::PayloadBatch;
use crate::{cache, common, payload};
use anyhow::Result;
use flume::{Receiver, RecvTimeoutError};
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use std::cell::Cell;
use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, trace, warn};
use uuid::Uuid;

type PHashMap<K, V> = papaya::HashMap<K, V, FxBuildHasher>;

pub static PROCESSOR_CHANNEL_NAME: &str = "processor";

/// How long [`WorkerGroup::close`] waits for the workers to finish what their
/// input holds before they stop waiting on full sink topics.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
/// How long workers that stopped waiting get to finish. Not cut by the
/// shutdown deadline: it is what lets them hand a disk topic what they hold,
/// and the shutdown watchdog allows for it.
const STOP_GRACE: Duration = Duration::from_secs(1);

/// `PATROLLI_TEST_PANIC=processor` makes the worker that gets the first batch
/// panic, once, for `tests/shutdown_test.rs` to see what a panic does to the
/// process. Debug builds only: a release binary does not read it.
#[cfg(debug_assertions)]
static TEST_PANIC: std::sync::LazyLock<std::sync::atomic::AtomicBool> =
    std::sync::LazyLock::new(|| {
        std::sync::atomic::AtomicBool::new(
            std::env::var_os("PATROLLI_TEST_PANIC").is_some_and(|v| v == "processor"),
        )
    });

#[derive(Default)]
struct NodeMetrics {
    total_time_spent: AtomicU64,
    engine_time_spent: AtomicU64,
    send_wait_time: AtomicU64,
    total_records: AtomicU64,
    total_records_filtered: AtomicU64,
    total_records_errored: AtomicU64,
}

#[derive(Default)]
struct EdgeMetrics {
    target_is_sink: bool,
    total_records: AtomicU64,
}

#[derive(Default)]
struct InternalMetrics {
    total_workers: AtomicUsize,
    total_time_spent: AtomicU64,
    engine_time_spent: AtomicU64,
    send_wait_time: AtomicU64,
    receive_wait_time: AtomicU64,
    total_records: AtomicU64,
    total_records_sent: AtomicU64,
    total_records_filtered: AtomicU64,
    total_records_errored: AtomicU64,
    /// Records dropped because nothing was downstream: a processor id the
    /// cache does not hold, no processor id and no input-less sink, or a
    /// processor without a next step. Counted per drop, so output fanned out
    /// to two dead ends counts twice, the way `total_records_sent` counts
    /// once per sink.
    total_records_unrouted: AtomicU64,
    nodes: PHashMap<Uuid, NodeMetrics>,
    edges: PHashMap<(Uuid, Uuid), EdgeMetrics>,
}

impl InternalMetrics {
    /// Drops the counters of nodes and edges the processor cache no longer
    /// holds. Entries are created on first use, and every recreated processor
    /// or sink mints a new id, so without this each delete-and-recreate left
    /// a frozen set of series behind forever.
    ///
    /// An empty cache means standby, not "nothing is live": the counters are
    /// kept so they carry on when the node is main again, the rule the lookup
    /// tables follow as well.
    fn retain_live(&self, cache: &cache::Cache) {
        let (live_nodes, live_edges): (FxHashSet<Uuid>, FxHashSet<(Uuid, Uuid)>) = {
            let processor = cache.processor.read();
            (
                processor.node_ids().chain(processor.sink_ids()).collect(),
                processor.edge_ids().collect(),
            )
        };
        if live_nodes.is_empty() {
            return;
        }
        self.nodes.pin().retain(|id, _| live_nodes.contains(id));
        self.edges.pin().retain(|edge, _| live_edges.contains(edge));
    }

    fn get(&self) -> ProcessorMetrics {
        let nodes = self
            .nodes
            .pin()
            .iter()
            .map(|(id, metrics)| ProcessorNodeMetrics {
                id: *id,
                total_time_spent: Duration::from_nanos(
                    metrics.total_time_spent.load(Ordering::Relaxed),
                ),
                engine_time_spent: Duration::from_nanos(
                    metrics.engine_time_spent.load(Ordering::Relaxed),
                ),
                send_wait_time: Duration::from_nanos(
                    metrics.send_wait_time.load(Ordering::Relaxed),
                ),
                total_records: metrics.total_records.load(Ordering::Relaxed),
                total_records_filtered: metrics.total_records_filtered.load(Ordering::Relaxed),
                total_records_errored: metrics.total_records_errored.load(Ordering::Relaxed),
            })
            .collect();
        let edges = self
            .edges
            .pin()
            .iter()
            .map(|((source, target), metrics)| ProcessorEdgeMetrics {
                source: *source,
                target: *target,
                target_is_sink: metrics.target_is_sink,
                total_records: metrics.total_records.load(Ordering::Relaxed),
            })
            .collect();

        ProcessorMetrics {
            total_workers: self.total_workers.load(Ordering::Relaxed),
            total_time_spent: Duration::from_nanos(self.total_time_spent.load(Ordering::Relaxed)),
            engine_time_spent: Duration::from_nanos(self.engine_time_spent.load(Ordering::Relaxed)),
            receive_wait_time: Duration::from_nanos(self.receive_wait_time.load(Ordering::Relaxed)),
            send_wait_time: Duration::from_nanos(self.send_wait_time.load(Ordering::Relaxed)),
            total_records: self.total_records.load(Ordering::Relaxed),
            total_records_sent: self.total_records_sent.load(Ordering::Relaxed),
            total_records_filtered: self.total_records_filtered.load(Ordering::Relaxed),
            total_records_errored: self.total_records_errored.load(Ordering::Relaxed),
            total_records_unrouted: self.total_records_unrouted.load(Ordering::Relaxed),
            nodes,
            edges,
        }
    }
}

struct Worker {
    idx: usize,
    internal_metrics: Arc<InternalMetrics>,
    sender: SenderChannel<Uuid, payload::Payload>,
    receiver: Receiver<payload::ProcessorPayload>,
    config_cache: cache::Cache,
    /// Cancelled when shutdown stops waiting on sinks that do not consume:
    /// see [`WorkerGroup::close`].
    stop_waiting: CancellationToken,
    /// Records a full sink topic turned away after `stop_waiting`, reported
    /// once as the worker stops.
    dropped_on_stop: Cell<u64>,
    transforms: transform::Transforms,
}

impl Worker {
    fn log_edge_total_records(&self, prev_step: Option<&Step>, step: &Step, total: u64) {
        let Some(source_id) = prev_step.map(|s| s.id()) else {
            return;
        };
        let target_id = step.id();
        let target_is_sink = matches!(&step.node, Node::Sink(_));
        self.internal_metrics
            .edges
            .pin()
            .get_or_insert_with((source_id, target_id), || EdgeMetrics {
                target_is_sink,
                ..Default::default()
            })
            .total_records
            .fetch_add(total, Ordering::Relaxed);
    }

    fn log_node(
        &self,
        step: &Step,
        dur: Duration,
        total: u64,
        filtered: Option<u64>,
        errored: Option<u64>,
    ) {
        let internal_metrics = self.internal_metrics.nodes.pin();
        let node_metrics = internal_metrics.get_or_insert_with(step.id(), NodeMetrics::default);
        node_metrics
            .total_time_spent
            .fetch_add(dur.as_nanos() as u64, Ordering::Relaxed);
        node_metrics
            .engine_time_spent
            .fetch_add(dur.as_nanos() as u64, Ordering::Relaxed);
        node_metrics
            .total_records
            .fetch_add(total, Ordering::Relaxed);
        // The processor-wide counters are the sum over its nodes, the way
        // `total_records_sent` is the sum over its sinks.
        if let Some(errored) = errored {
            node_metrics
                .total_records_errored
                .fetch_add(errored, Ordering::Relaxed);
            self.internal_metrics
                .total_records_errored
                .fetch_add(errored, Ordering::Relaxed);
        }
        if let Some(filtered) = filtered {
            node_metrics
                .total_records_filtered
                .fetch_add(filtered, Ordering::Relaxed);
            self.internal_metrics
                .total_records_filtered
                .fetch_add(filtered, Ordering::Relaxed);
        }
    }

    #[instrument(level = "debug", skip_all, fields(processor_id = %step.id(), processor = %processor.name))]
    fn run_processor(
        &self,
        step: &Step,
        pl: payload::Payload,
        processor: &Arc<Processor>,
        n: Instant,
    ) -> Option<payload::Payload> {
        let pl_len = pl.num_items() as u64;
        match &processor.config {
            Config::Null => {
                self.log_node(step, n.elapsed(), pl_len, None, None);
                Some(pl)
            }
            Config::AttributeFilter(filter) => {
                let res = attribute_filter::attribute_filter(pl, filter);
                match res {
                    Ok(pl) => {
                        self.log_node(
                            step,
                            n.elapsed(),
                            pl_len,
                            Some(pl_len.saturating_sub(pl.num_items() as u64)),
                            None,
                        );
                        Some(pl)
                    }
                    Err(e) => {
                        warn!(error = %format!("{e:#}"), "attribute filter failed, batch dropped");
                        self.log_node(step, n.elapsed(), pl_len, None, Some(pl_len));
                        None
                    }
                }
            }
            Config::DropAttributes(DropAttributes { transforms, .. })
            | Config::AddAttributes(AddAttributes { transforms, .. })
            | Config::RenameAttributes(RenameAttributes { transforms, .. }) => {
                match transforms {
                    Some(transforms) => {
                        let res = attribute_transform::attribute_transform(pl, transforms);
                        match res {
                            Ok(pl) => {
                                self.log_node(step, n.elapsed(), pl_len, None, None);
                                Some(pl)
                            }
                            Err(e) => {
                                warn!(
                                    error = %format!("{e:#}"),
                                    "attribute transform failed, batch dropped"
                                );
                                self.log_node(step, n.elapsed(), pl_len, None, Some(pl_len));
                                None
                            }
                        }
                    }
                    // Not errored: the batch goes on unchanged, nothing is lost.
                    None => {
                        error!("processor transforms not compiled, passing batch through");
                        self.log_node(step, n.elapsed(), pl_len, None, None);
                        Some(pl)
                    }
                }
            }
            // Neither drops on error, so neither reports
            // `total_records_errored`: that counter means "records this node
            // lost", and one misconfigured rule out of ten would mark every
            // record in the batch errored, pinning an error-rate alert at 100%
            // while nothing is lost. The skip surfaces in each processor's
            // `warn!`, per-record failures in the table's own counters.
            Config::AttributeLookup(config) => {
                let (pl, _errored) =
                    attribute_lookup::attribute_lookup(pl, config, step.id(), &self.config_cache);
                self.log_node(step, n.elapsed(), pl_len, None, None);
                Some(pl)
            }
            Config::AttributeCollect(config) => {
                let (pl, _errored) =
                    attribute_collect::attribute_collect(pl, config, step.id(), &self.config_cache);
                self.log_node(step, n.elapsed(), pl_len, None, None);
                Some(pl)
            }
            Config::Transform(program) => match self.transforms.run(processor, program, pl) {
                Ok(pl) => {
                    self.log_node(
                        step,
                        n.elapsed(),
                        pl_len,
                        Some(pl_len.saturating_sub(pl.num_items() as u64)),
                        None,
                    );
                    Some(pl)
                }
                Err(e) => {
                    warn!(error = %format!("{e:#}"), "transform failed, batch dropped");
                    self.log_node(step, n.elapsed(), pl_len, None, Some(pl_len));
                    None
                }
            },
        }
    }

    // returns true if capture was sent
    fn send_capture(
        &self,
        capture: &Capture,
        capture_in: &payload::Payload,
        capture_out: Option<&payload::Payload>,
    ) -> Result<bool> {
        let Some(message_in) = capture_in
            .serialize_as(Format::JsonOtlpBatch)?
            .into_iter()
            .next()
        else {
            anyhow::bail!("empty json");
        };
        let message_out = if let Some(capture_out) = capture_out {
            capture_out
                .serialize_as(Format::JsonOtlpBatch)?
                .into_iter()
                .next()
        } else {
            None
        };
        let ok = capture
            .sender
            .send(CaptureMessage {
                message_in,
                message_out,
            })
            .is_ok();
        Ok(ok)
    }

    fn processor_loop(
        &self,
        out: &mut FxHashMap<Uuid, payload::PayloadBatch>,
        pl: payload::Payload,
        step: &Step,
        prev_step: Option<&Step>,
    ) {
        let pl_len = pl.num_items() as u64;
        self.log_edge_total_records(prev_step, step, pl_len);
        let n = Instant::now();
        match &step.node {
            Node::Sink(sink) => {
                out.entry(sink.id).or_default().push(pl);
            }
            Node::Processor(processor) => {
                // A captured batch is processed as two pieces, the sample and
                // the rest, then rebatched. That is one payload, unless together
                // they hold more items than one payload can.
                let (pl, rebatched) = if let capture_guard =
                    self.config_cache.processor.captures.read()
                    && let Some(capture) = capture_guard.get(&processor.id)
                    && let ts = capture.last_send.load(Ordering::Relaxed)
                    && common::uptime().saturating_sub(Duration::from_nanos(ts))
                        >= Duration::from_secs(1)
                    && capture
                        .last_send
                        .compare_exchange(
                            ts,
                            common::uptime().as_nanos() as u64,
                            Ordering::Relaxed,
                            Ordering::Relaxed,
                        )
                        .is_ok()
                {
                    let (pl, rest, delivered) = match pl.split_items(capture.req.max_count as usize)
                    {
                        Ok((Some(capture_pl_in), rest)) => {
                            let capture_pl_out =
                                self.run_processor(step, capture_pl_in.clone(), processor, n);
                            let delivered = match self.send_capture(
                                capture,
                                &capture_pl_in,
                                capture_pl_out.as_ref(),
                            ) {
                                Ok(delivered) => delivered,
                                Err(e) => {
                                    debug!(
                                        processor_id = %processor.id,
                                        error = %format!("{e:#}"),
                                        "capture serialization failed"
                                    );
                                    true
                                }
                            };
                            let rest = rest.and_then(|pl| {
                                self.run_processor(step, pl, processor, Instant::now())
                            });
                            (capture_pl_out, rest, delivered)
                        }
                        Ok((None, Some(pl))) => {
                            (self.run_processor(step, pl, processor, n), None, true)
                        }
                        Ok((None, None)) => (None, None, true),
                        Err(e) => {
                            warn!(
                                processor_id = %processor.id,
                                error = %format!("{e:#}"),
                                "capture split failed, batch dropped"
                            );
                            (None, None, true)
                        }
                    };
                    let capture = Arc::clone(capture);
                    drop(capture_guard);
                    if !delivered {
                        self.config_cache
                            .processor
                            .remove_capture_if_inactive(processor.id, &capture);
                    }
                    match (pl, rest) {
                        (Some(pl), Some(rest)) => {
                            let mut batch = PayloadBatch::new();
                            batch.push(rest);
                            batch.push(pl);
                            (None, Some(batch.batch(None)))
                        }
                        (pl, rest) => (pl.or(rest), None),
                    }
                } else {
                    (self.run_processor(step, pl, processor, n), None)
                };
                for pl in pl.into_iter().chain(rebatched.into_iter().flatten()) {
                    if pl.is_empty() {
                        continue;
                    }
                    if step.next.is_empty() {
                        self.internal_metrics
                            .total_records_unrouted
                            .fetch_add(pl.num_items() as u64, Ordering::Relaxed);
                        continue;
                    }
                    for next_step in &step.next {
                        self.processor_loop(out, pl.clone(), next_step, Some(step));
                    }
                }
            }
        }
    }

    fn run_processors(
        &self,
        pl: payload::ProcessorPayload,
    ) -> FxHashMap<Uuid, payload::PayloadBatch> {
        let mut out: FxHashMap<Uuid, payload::PayloadBatch> = FxHashMap::default();
        let processor_guard = self.config_cache.processor.read();
        self.transforms
            .prune(Arc::as_ptr(&*processor_guard) as usize, || {
                processor_guard.node_ids().collect()
            });
        match pl.processor_id {
            Some(processor_id) => match processor_guard.get_step(processor_id) {
                Some(step) if !pl.payload.is_empty() => {
                    self.processor_loop(&mut out, pl.payload, step, None);
                }
                Some(_) => {}
                None => {
                    warn!(processor_id = %processor_id, "no route for processor, records dropped");
                    self.internal_metrics
                        .total_records_unrouted
                        .fetch_add(pl.payload.num_items() as u64, Ordering::Relaxed);
                }
            },
            None => {
                let mut routed = false;
                for step in processor_guard.get_no_input_steps() {
                    let Node::Sink(sink) = &step.node else {
                        continue;
                    };
                    out.entry(sink.id).or_default().push(pl.payload.clone());
                    routed = true;
                }
                if !routed {
                    self.internal_metrics
                        .total_records_unrouted
                        .fetch_add(pl.payload.num_items() as u64, Ordering::Relaxed);
                }
            }
        }
        out
    }

    fn process_records(&self, payloads: payload::ProcessorPayload) {
        trace!(length = payloads.len(), "received records");
        #[cfg(debug_assertions)]
        if TEST_PANIC.swap(false, Ordering::Relaxed) {
            panic!("processor panic requested by PATROLLI_TEST_PANIC");
        }
        self.internal_metrics
            .total_records
            .fetch_add(payloads.len() as u64, Ordering::Relaxed);
        let mut n = Instant::now();
        let record_bucket = self.run_processors(payloads);
        self.internal_metrics
            .engine_time_spent
            .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
        n = Instant::now();
        for (sink_id, mut payloads) in record_bucket.into_iter() {
            let payloads = payloads.batch(None);
            for payloads in payloads {
                let n1 = Instant::now();
                let records_len = payloads.num_items() as u64;
                let sent = match self
                    .sender
                    .send_to_until(&sink_id, payloads, &self.stop_waiting)
                {
                    Ok(_) => true,
                    Err(chan::Error::BufferFull(_)) if self.stop_waiting.is_cancelled() => {
                        self.dropped_on_stop
                            .set(self.dropped_on_stop.get() + records_len);
                        false
                    }
                    Err(e) => {
                        warn!(sink_id = %sink_id, error = %e, "send to sink failed, records dropped");
                        false
                    }
                };
                if sent {
                    self.internal_metrics
                        .total_records_sent
                        .fetch_add(records_len, Ordering::Relaxed);
                } else {
                    self.internal_metrics
                        .total_records_errored
                        .fetch_add(records_len, Ordering::Relaxed);
                }
                let dur = n1.elapsed();
                {
                    // The sink's own node entry: what it was handed and what
                    // it lost, alongside the time spent handing it over.
                    let guard = self.internal_metrics.nodes.pin();
                    let internal_metrics = guard.get_or_insert_with(sink_id, NodeMetrics::default);
                    internal_metrics
                        .total_records
                        .fetch_add(records_len, Ordering::Relaxed);
                    if !sent {
                        internal_metrics
                            .total_records_errored
                            .fetch_add(records_len, Ordering::Relaxed);
                    }
                    internal_metrics
                        .send_wait_time
                        .fetch_add(dur.as_nanos() as u64, Ordering::Relaxed);
                    internal_metrics
                        .total_time_spent
                        .fetch_add(dur.as_nanos() as u64, Ordering::Relaxed);
                }
            }
        }
        self.internal_metrics
            .send_wait_time
            .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }

    #[instrument(skip_all, fields(worker = self.idx))]
    fn run(self) {
        info!("processor worker started");
        let _counted = WorkerCount::new(&self.internal_metrics);
        loop {
            let n = Instant::now();
            let res = self.receiver.recv_timeout(Duration::from_millis(300));
            self.internal_metrics
                .receive_wait_time
                .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
            match res {
                Err(RecvTimeoutError::Disconnected) => {
                    break;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Ok(records) => self.process_records(records),
            }
            self.internal_metrics
                .total_time_spent
                .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        let dropped = self.dropped_on_stop.get();
        if dropped > 0 {
            warn!(
                records = dropped,
                "records dropped on shutdown, sink not consuming"
            );
        }
        info!("processor worker stopped");
    }
}

/// Counts a running worker in `total_workers`, and stops counting it however
/// the worker ends, a panic's unwinding included.
struct WorkerCount<'a>(&'a InternalMetrics);

impl<'a> WorkerCount<'a> {
    fn new(metrics: &'a InternalMetrics) -> Self {
        metrics.total_workers.fetch_add(1, Ordering::Relaxed);
        Self(metrics)
    }
}

impl Drop for WorkerCount<'_> {
    fn drop(&mut self) {
        self.0.total_workers.fetch_sub(1, Ordering::Relaxed);
    }
}

pub struct WorkerGroup {
    internal_metrics: Arc<InternalMetrics>,
    sender: SenderChannel<Uuid, payload::Payload>,
    receiver: Receiver<payload::ProcessorPayload>,
    units: Vec<JoinHandle<()>>,
    token: CancellationToken,
    /// See [`Worker::stop_waiting`].
    stop_waiting: CancellationToken,
    config_cache: cache::Cache,
}

impl WorkerGroup {
    pub fn new(
        sender: SenderChannel<Uuid, payload::Payload>,
        receiver: Receiver<payload::ProcessorPayload>,
        config_cache: cache::Cache,
    ) -> Self {
        Self {
            internal_metrics: Arc::new(InternalMetrics::default()),
            sender,
            receiver,
            config_cache,
            units: Vec::new(),
            token: CancellationToken::new(),
            stop_waiting: CancellationToken::new(),
        }
    }

    pub async fn run(&mut self) {
        let workers = std::thread::available_parallelism()
            .unwrap_or(NonZero::<usize>::new(8).unwrap())
            .get();
        info!(workers, "starting processor workers");
        for i in 0..workers {
            let sender = self.sender.clone();
            let receiver = self.receiver.clone();
            let internal_metrics = self.internal_metrics.clone();
            let config_cache = self.config_cache.clone();
            let stop_waiting = self.stop_waiting.clone();

            let (done_tx, done_rx) = tokio::sync::oneshot::channel();
            let thread = std::thread::Builder::new().name(format!("processor-{i}"));
            let spawned = thread.spawn(move || {
                let w = Worker {
                    idx: i,
                    internal_metrics,
                    sender,
                    receiver,
                    config_cache,
                    stop_waiting,
                    dropped_on_stop: Cell::new(0),
                    transforms: Default::default(),
                };
                w.run();
                let _ = done_tx.send(());
            });
            // As `std::thread::spawn` does: no thread is a fatal panic.
            spawned.expect("failed to spawn processor worker thread");
            // No respawn: a panic is fatal, reported by the hook, which also
            // started the shutdown (see `common::panic`).
            let task = tokio::spawn(async move {
                if done_rx.await.is_err() {
                    debug!(worker = i, "processor worker panicked");
                }
            });
            self.units.push(task);
        }
    }

    pub async fn spawn_metrics_reporter(&mut self, metrics_report_duration: Duration) {
        let token_clone = self.token.clone();
        let internal_metrics_clone = self.internal_metrics.clone();
        let config_cache = self.config_cache.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(metrics_report_duration) => {
                        // Reconcile on the tick: this is exactly where a label
                        // set that stopped existing needs to stop being rendered.
                        internal_metrics_clone.retain_live(&config_cache);
                        let metrics = internal_metrics_clone.get();
                        INTERNAL_METRICS.processor.store(Arc::new((Instant::now(), metrics)));
                    }
                    _ = token_clone.cancelled() => break,
                }
            }
        });
        self.units.push(task);
    }

    /// Waits for the workers to finish what their input holds, which they do
    /// once it disconnects, for at most [`DRAIN_TIMEOUT`] or until
    /// `deadline`. A worker still running then is blocked on a sink topic
    /// that is full because its sink stopped consuming (a destination that
    /// is down), and that sink is only closed after this returns. So the
    /// workers stop waiting on full topics: a memory topic turns away what
    /// they send it, a disk topic takes it past its `max_disk_size`. They get
    /// [`STOP_GRACE`] to finish; one still running after that is left behind
    /// for the process exit to end, as a thread cannot be aborted.
    pub async fn close(self, deadline: Deadline) {
        self.token.cancel();
        let mut units = self.units;
        let limit = deadline.budget(DRAIN_TIMEOUT);
        if Self::join_units(&mut units, limit).await {
            return;
        }
        warn!(
            timeout_ms = limit.as_millis() as u64,
            "processor drain timed out, no longer waiting on full sink topics"
        );
        self.stop_waiting.cancel();
        if Self::join_units(&mut units, STOP_GRACE).await {
            return;
        }
        error!(
            workers = units.len(),
            "processor workers did not stop, leaving them behind"
        );
    }

    /// Waits up to `limit` for `units` to return, taking out each one that
    /// did. Whether all of them did.
    async fn join_units(units: &mut Vec<JoinHandle<()>>, limit: Duration) -> bool {
        tokio::time::timeout(limit, async {
            while let Some(unit) = units.last_mut() {
                if let Err(e) = unit.await {
                    debug!(error = %e, "processor task panicked");
                }
                units.pop();
            }
        })
        .await
        .is_ok()
    }
}

#[cfg(test)]
mod retain_tests {
    use super::{EdgeMetrics, InternalMetrics, NodeMetrics};
    use crate::cache;
    use crate::config::processor::{Config, Processor as ProcessorConfig};
    use uuid::Uuid;

    fn null_processor(id: Uuid, inputs: Vec<Uuid>) -> ProcessorConfig {
        ProcessorConfig {
            id,
            name: id.to_string(),
            config: Config::Null,
            inputs,
        }
    }

    fn node_ids(m: &InternalMetrics) -> Vec<Uuid> {
        let mut ids: Vec<Uuid> = m.nodes.pin().keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// Counters are keyed by node id and created on first use, and every
    /// recreated processor mints a new one, so without reconciliation each
    /// tuning cycle leaves a frozen set of series behind forever.
    #[test]
    fn counters_of_deleted_nodes_and_edges_are_dropped() {
        let cache = cache::Cache::new();
        let (a, b, dead) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        {
            let mut draft = cache.processor.write();
            let data = draft.as_mut().unwrap();
            data.insert_processor(null_processor(a, vec![]));
            data.insert_processor(null_processor(b, vec![a]));
            cache.processor.compile(draft).unwrap();
        }
        let m = InternalMetrics::default();
        for id in [a, b, dead] {
            m.nodes.pin().get_or_insert_with(id, NodeMetrics::default);
        }
        m.edges
            .pin()
            .get_or_insert_with((a, b), EdgeMetrics::default);
        m.edges
            .pin()
            .get_or_insert_with((a, dead), EdgeMetrics::default);

        m.retain_live(&cache);
        let mut live = vec![a, b];
        live.sort_unstable();
        assert_eq!(node_ids(&m), live);
        assert_eq!(m.edges.pin().len(), 1);
        assert!(m.edges.pin().contains_key(&(a, b)));

        // a standby node has an empty cache: there is nothing to reconcile
        // against, so an empty set must not wipe the counters
        {
            let mut draft = cache.processor.write();
            draft.as_mut().unwrap().clear();
            cache.processor.compile(draft).unwrap();
        }
        m.retain_live(&cache);
        assert_eq!(node_ids(&m), live);
        assert_eq!(m.edges.pin().len(), 1);
    }
}
