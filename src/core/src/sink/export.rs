// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The batching and retry loop shared by sinks that implement [`Exporter`].
//!
//! [`ExportSink`] takes payloads off the sink's topic into batches, has the
//! exporter encode each batch once and export it until the destination has
//! taken or refused all of it, and only then acks the payloads. A batch the
//! destination could not take is retried in the worker that holds it, paced by
//! the sink's `retry:` config, for as long as that allows. A worker cancelled
//! mid-retry (scale-down, a replacing sink, a shutdown past its drain timeout)
//! nacks what it holds, so the topic delivers it again.

use super::{InternalMetrics, Sink};
use crate::common::backoff::RetryBackoff;
use crate::common::internal_metrics::{Sink as Metrics, SinkEndpoint};
use crate::{chan, config, payload};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::time::{Instant, MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// What became of one attempt at exporting an encoded batch.
pub enum Export<E> {
    /// Everything landed.
    Delivered,
    /// Nothing landed, and trying again may help: the destination is down or
    /// overloaded. `after`, when set, is the shortest wait before the next
    /// attempt (the backoff still applies when it is longer).
    Retry {
        error: anyhow::Error,
        after: Option<Duration>,
    },
    /// Nothing landed, and trying again would fail the same way: the data
    /// itself is bad, or does not fit the destination.
    Reject { error: anyhow::Error },
    /// Some of it landed. `retry` is what is left to try again, `rejected`
    /// the items dropped for good; the rest was delivered.
    Partial {
        retry: Option<(E, anyhow::Error)>,
        rejected: Option<(u64, anyhow::Error)>,
    },
}

/// A batch as the exporter sends it, with the items encoding already settled.
pub struct Encoded<E> {
    /// `None` when nothing is left to send.
    pub data: Option<E>,
    /// Items left out on purpose.
    pub filtered: u64,
    /// Items that could not be encoded; the encoder logs why.
    pub errored: u64,
}

/// A destination as [`ExportSink`] drives it.
///
/// Items are the unit of the sink's counters: for each batch,
/// `filtered + errored + items(data)` must equal the batch's `num_items`.
#[async_trait]
pub trait Exporter: Send + Sync + 'static {
    type Encoded: Send + Sync;

    /// Called at the start of every worker; an error stops the worker, for
    /// the supervisor to restart it.
    async fn start(&self) -> Result<()> {
        Ok(())
    }

    /// Turns a batch into what [`export`](Self::export) sends. Runs once per
    /// batch, however many attempts the export takes.
    fn encode(&self, batch: payload::PayloadBatch) -> Encoded<Self::Encoded>;

    /// Items held by an encoded batch, or by what is left of it.
    fn items(&self, encoded: &Self::Encoded) -> u64;

    /// One attempt at sending `encoded`. Must be cancel-safe: a cancelled
    /// worker drops it mid-flight and requeues the batch's payloads.
    async fn export(&self, encoded: &Self::Encoded) -> Export<Self::Encoded>;

    /// Health of each endpoint, for the sink's metrics.
    fn endpoints(&self) -> Vec<SinkEndpoint> {
        Vec::new()
    }

    /// Called once, after every worker has stopped.
    async fn close(&self) {}
}

pub struct ExportSink<X> {
    conf: config::sink::Sink,
    channel: chan::Consumer<payload::Payload>,
    metrics: InternalMetrics,
    exporter: X,
    /// Set while some worker waits to retry a batch; only so the sink logs
    /// once when its destination goes away and once when it comes back.
    retrying: AtomicBool,
    /// Off in tests that check the delays.
    jitter: bool,
}

impl<X: Exporter> ExportSink<X> {
    pub fn with_exporter(
        conf: config::sink::Sink,
        channel: chan::Consumer<payload::Payload>,
        exporter: X,
    ) -> Self {
        Self {
            conf,
            channel,
            metrics: InternalMetrics::default(),
            exporter,
            retrying: AtomicBool::new(false),
            jitter: true,
        }
    }

    fn add(counter: &std::sync::atomic::AtomicU64, n: u64) {
        if n > 0 {
            counter.fetch_add(n, Ordering::Relaxed);
        }
    }

    fn add_time(counter: &std::sync::atomic::AtomicU64, since: Instant) {
        counter.fetch_add(since.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }

    async fn nack_all(&self, deliveries: Vec<chan::Delivery<payload::Payload>>) {
        for delivery in deliveries {
            self.channel.nack(delivery).await;
        }
    }

    /// Encodes and exports `deliveries` as one batch, then acks them. Returns
    /// false when cancelled first, with every delivery nacked instead.
    async fn flush(
        &self,
        token: &CancellationToken,
        backoff: &mut RetryBackoff,
        deliveries: Vec<chan::Delivery<payload::Payload>>,
    ) -> bool {
        if deliveries.is_empty() {
            return true;
        }
        // Encoding consumes the batch, while a nack needs the payloads back:
        // encode from clones, which only share the Arrow buffers.
        let mut batch = payload::PayloadBatch::new();
        for delivery in &deliveries {
            batch.push(delivery.msg.clone());
        }
        Self::add(&self.metrics.total_records, batch.num_items() as u64);
        let started = Instant::now();
        let encoded = self.exporter.encode(batch);
        Self::add_time(&self.metrics.engine_time_spent, started);
        Self::add(&self.metrics.total_records_filtered, encoded.filtered);
        Self::add(&self.metrics.total_records_errored, encoded.errored);
        if let Some(data) = encoded.data
            && !self.export(token, backoff, data).await
        {
            self.nack_all(deliveries).await;
            return false;
        }
        self.channel
            .ack_all(deliveries.into_iter().map(|d| d.into_parts().1));
        true
    }

    /// Exports `data` until all of it is delivered or rejected, or the retries
    /// run out. Returns false when cancelled first.
    async fn export(
        &self,
        token: &CancellationToken,
        backoff: &mut RetryBackoff,
        mut data: X::Encoded,
    ) -> bool {
        let max_elapsed = self.conf.retry.max_elapsed;
        let started = Instant::now();
        let mut pending = PendingRetry::new(&self.metrics);
        let mut attempt = 0u32;
        loop {
            let items = self.exporter.items(&data);
            if attempt > 0 {
                Self::add(&self.metrics.total_records_retried, items);
            }
            attempt += 1;
            let sent_at = Instant::now();
            let outcome = tokio::select! {
                biased;

                _ = token.cancelled() => None,
                outcome = self.exporter.export(&data) => Some(outcome),
            };
            Self::add_time(&self.metrics.send_wait_time, sent_at);
            let Some(outcome) = outcome else {
                return false;
            };
            let (left, error, after) = match outcome {
                Export::Delivered => {
                    Self::add(&self.metrics.total_records_sent, items);
                    self.recovered();
                    backoff.reset();
                    return true;
                }
                Export::Reject { error } => {
                    Self::add(&self.metrics.total_records_errored, items);
                    error!(error = %format!("{error:#}"), records = items, "batch rejected");
                    backoff.reset();
                    return true;
                }
                Export::Retry { error, after } => (data, error, after),
                Export::Partial { retry, rejected } => {
                    let rejected = match rejected {
                        Some((n, error)) => {
                            Self::add(&self.metrics.total_records_errored, n);
                            error!(error = %format!("{error:#}"), records = n, "batch partly rejected");
                            n
                        }
                        None => 0,
                    };
                    let left = retry.as_ref().map_or(0, |(e, _)| self.exporter.items(e));
                    Self::add(
                        &self.metrics.total_records_sent,
                        items.saturating_sub(rejected + left),
                    );
                    let Some((left, error)) = retry else {
                        self.recovered();
                        backoff.reset();
                        return true;
                    };
                    (left, error, None)
                }
            };
            data = left;
            let items = self.exporter.items(&data);
            let mut delay = backoff.next_delay();
            if let Some(after) = after {
                delay = delay.max(after);
            }
            if !max_elapsed.is_zero() {
                let remaining = max_elapsed.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    Self::add(&self.metrics.total_records_errored, items);
                    warn!(
                        error = %format!("{error:#}"),
                        records = items,
                        attempts = attempt,
                        "retries exhausted, batch dropped"
                    );
                    return true;
                }
                // The last attempt lands on the deadline rather than past it.
                delay = delay.min(remaining);
            }
            pending.set(items);
            if !self.retrying.swap(true, Ordering::Relaxed) {
                warn!(
                    error = %format!("{error:#}"),
                    retry_in = ?delay,
                    "destination unavailable, retrying"
                );
            }
            // Waiting out the backoff is waiting on the destination too.
            let slept_at = Instant::now();
            let cancelled = tokio::select! {
                biased;

                _ = token.cancelled() => true,
                _ = tokio::time::sleep(delay) => false,
            };
            Self::add_time(&self.metrics.send_wait_time, slept_at);
            if cancelled {
                return false;
            }
        }
    }

    fn recovered(&self) {
        if self.retrying.swap(false, Ordering::Relaxed) {
            info!("destination recovered");
        }
    }
}

/// This worker's share of `records_pending_retry`, given back when dropped.
struct PendingRetry<'a> {
    metrics: &'a InternalMetrics,
    items: u64,
}

impl<'a> PendingRetry<'a> {
    fn new(metrics: &'a InternalMetrics) -> Self {
        Self { metrics, items: 0 }
    }

    fn set(&mut self, items: u64) {
        let gauge = &self.metrics.records_pending_retry;
        if items > self.items {
            gauge.fetch_add(items - self.items, Ordering::Relaxed);
        } else {
            gauge.fetch_sub(self.items - items, Ordering::Relaxed);
        }
        self.items = items;
    }
}

impl Drop for PendingRetry<'_> {
    fn drop(&mut self) {
        self.set(0);
    }
}

#[async_trait]
impl<X: Exporter> Sink for ExportSink<X> {
    #[tracing::instrument(skip_all, fields(sink_id = %self.conf.id, sink = %self.conf.name))]
    async fn run(&self, token: CancellationToken) {
        self.metrics.total_workers.fetch_add(1, Ordering::Relaxed);
        if let Err(e) = self.exporter.start().await {
            error!(error = %format!("{e:#}"), "failed to start sink");
            self.metrics.total_workers.fetch_sub(1, Ordering::Relaxed);
            return;
        }
        info!("sink started");
        let mut backoff = RetryBackoff::new(&self.conf.retry);
        if !self.jitter {
            backoff = backoff.without_jitter();
        }
        let mut deliveries = Vec::new();
        let mut items = 0usize;
        let mut batch_tick = interval(self.conf.batch.batch_timeout);
        batch_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            let n = Instant::now();
            let flush = tokio::select! {
                biased;

                _ = token.cancelled() => {
                    Self::add_time(&self.metrics.receive_wait_time, n);
                    Self::add_time(&self.metrics.total_time_spent, n);
                    break;
                }
                _ = batch_tick.tick() => true,
                msg = self.channel.recv() => match msg {
                    Some(delivery) => {
                        items += delivery.msg.num_items();
                        deliveries.push(delivery);
                        items >= self.conf.batch.batch_size
                    }
                    None => {
                        Self::add_time(&self.metrics.receive_wait_time, n);
                        Self::add_time(&self.metrics.total_time_spent, n);
                        break;
                    }
                },
            };
            Self::add_time(&self.metrics.receive_wait_time, n);
            if flush {
                let delivered = self
                    .flush(&token, &mut backoff, std::mem::take(&mut deliveries))
                    .await;
                items = 0;
                batch_tick.reset();
                if !delivered {
                    Self::add_time(&self.metrics.total_time_spent, n);
                    break;
                }
            }
            Self::add_time(&self.metrics.total_time_spent, n);
        }
        // The input ended: deliver the rest. Cancelled: hand it back.
        let n = Instant::now();
        if token.is_cancelled() {
            self.nack_all(deliveries).await;
        } else {
            self.flush(&token, &mut backoff, deliveries).await;
        }
        Self::add_time(&self.metrics.total_time_spent, n);
        self.metrics.total_workers.fetch_sub(1, Ordering::Relaxed);
        info!("sink stopped");
    }

    fn metrics(&self) -> Metrics {
        let mut metrics = self.metrics.get(self.conf.id, self.conf.name.clone());
        metrics.endpoints = self.exporter.endpoints();
        metrics
    }

    fn get_channel(&self) -> chan::Consumer<payload::Payload> {
        self.channel.clone()
    }

    async fn close(&self) {
        self.exporter.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Input, Worker};
    use super::*;
    use otel_arrow_dfe_pdata::encode::encode_logs_otap_batch;
    use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
        LogRecord, LogsData, ResourceLogs, ScopeLogs,
    };
    use parking_lot::Mutex;
    use std::collections::VecDeque;
    use std::sync::Arc;

    #[derive(Clone, Copy, Debug)]
    enum Step {
        Deliver,
        Retry,
        RetryAfter(Duration),
        Reject,
        /// Never answers.
        Hang,
        /// Retry `retry` items, drop `rejected`, deliver the rest.
        Partial {
            retry: u64,
            rejected: u64,
        },
    }

    /// A destination that answers each attempt with the next scripted step,
    /// then with `then` forever. Its encoded batch is just the item count.
    struct Fake {
        script: Mutex<VecDeque<Step>>,
        then: Step,
        attempts: Mutex<Vec<Instant>>,
    }

    #[async_trait]
    impl Exporter for Fake {
        type Encoded = u64;

        fn encode(&self, batch: payload::PayloadBatch) -> Encoded<u64> {
            let items = batch.num_items() as u64;
            Encoded {
                data: (items > 0).then_some(items),
                filtered: 0,
                errored: 0,
            }
        }

        fn items(&self, items: &u64) -> u64 {
            *items
        }

        async fn export(&self, items: &u64) -> Export<u64> {
            self.attempts.lock().push(Instant::now());
            let step = self.script.lock().pop_front().unwrap_or(self.then);
            match step {
                Step::Deliver => Export::Delivered,
                Step::Retry => Export::Retry {
                    error: anyhow::anyhow!("down"),
                    after: None,
                },
                Step::RetryAfter(after) => Export::Retry {
                    error: anyhow::anyhow!("down"),
                    after: Some(after),
                },
                Step::Reject => Export::Reject {
                    error: anyhow::anyhow!("bad"),
                },
                Step::Hang => std::future::pending().await,
                Step::Partial { retry, rejected } => {
                    assert!(retry + rejected <= *items);
                    Export::Partial {
                        retry: (retry > 0).then(|| (retry, anyhow::anyhow!("down"))),
                        rejected: (rejected > 0).then(|| (rejected, anyhow::anyhow!("bad"))),
                    }
                }
            }
        }
    }

    fn logs(records: usize) -> payload::Payload {
        let logs = LogsData {
            resource_logs: vec![ResourceLogs {
                resource: None,
                scope_logs: vec![ScopeLogs {
                    scope: None,
                    log_records: vec![LogRecord::default(); records],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        payload::Payload::new(encode_logs_otap_batch(&logs).unwrap())
    }

    /// Batches of 5 items that only `batch_size` closes, and retries from 1s
    /// doubling up to 60s, for at most `max_elapsed` seconds (0: forever).
    fn conf(max_elapsed: u64) -> config::sink::Sink {
        let mut conf: config::sink::Sink = serde_yaml_ng::from_str(&format!(
            r#"
id: 00000000-0000-0000-0000-000000000003
name: fake
worker_count: 1
batch_size: 5
batch_timeout: 3600000
retry:
  initial_interval: 1000
  max_interval: 60000
  multiplier: 2.0
  max_elapsed: {}
type: file
config:
  path: /dev/null
"#,
            max_elapsed * 1000
        ))
        .unwrap();
        conf.validate().unwrap();
        conf
    }

    struct Harness {
        sink: Arc<ExportSink<Fake>>,
        tx: flume::Sender<payload::Payload>,
        rx: flume::Receiver<payload::Payload>,
    }

    fn harness(script: &[Step], then: Step, max_elapsed: u64) -> Harness {
        let (tx, rx) = flume::bounded(16);
        let consumer = chan::Consumer::from_flume(&tx, rx.clone());
        let fake = Fake {
            script: Mutex::new(script.iter().copied().collect()),
            then,
            attempts: Mutex::new(Vec::new()),
        };
        let mut sink = ExportSink::with_exporter(conf(max_elapsed), consumer, fake);
        sink.jitter = false;
        Harness {
            sink: Arc::new(sink),
            tx,
            rx,
        }
    }

    impl Harness {
        /// Seconds from the first attempt to each attempt.
        fn attempts(&self) -> Vec<u64> {
            let attempts = self.sink.exporter.attempts.lock();
            attempts
                .iter()
                .map(|at| at.duration_since(attempts[0]).as_secs())
                .collect()
        }

        /// Sends `payloads`, ends the input and runs one worker until it returns.
        async fn run_to_end(self, payloads: Vec<payload::Payload>) -> (Self, Metrics) {
            for pl in payloads {
                self.tx.send(pl).unwrap();
            }
            let Self { sink, tx, rx } = self;
            drop(tx);
            sink.run(CancellationToken::new()).await;
            let metrics = sink.metrics();
            let (tx, _) = flume::bounded(1);
            (Self { sink, tx, rx }, metrics)
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_batch_is_retried_until_it_lands() {
        let h = harness(&[Step::Retry, Step::Retry, Step::Retry], Step::Deliver, 0);
        let (h, m) = h.run_to_end(vec![logs(5)]).await;
        assert_eq!(h.attempts(), [0, 1, 3, 7]);
        assert_eq!(m.total_records, 5);
        assert_eq!(m.total_records_sent, 5);
        assert_eq!(m.total_records_errored, 0);
        assert_eq!(m.total_records_retried, 15);
        assert_eq!(m.records_pending_retry, 0);
        assert_eq!(h.sink.channel.stashed(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_out_the_backoff_counts_as_send_wait() {
        // the full batch lands at 3s, then the 3 left over when the input
        // ends go out at 3s and, after one retry, at 4s
        let script = [Step::Retry, Step::Retry, Step::Deliver, Step::Retry];
        let h = harness(&script, Step::Deliver, 0);
        let (h, m) = h.run_to_end(vec![logs(5), logs(3)]).await;
        assert_eq!(h.attempts(), [0, 1, 3, 3, 4]);
        assert_eq!(m.send_wait_time, Duration::from_secs(4));
        // the flush after the input ended is part of the total too
        assert_eq!(m.total_time_spent, m.send_wait_time);
    }

    #[tokio::test(start_paused = true)]
    async fn an_export_cut_short_by_cancel_still_counts_as_send_wait() {
        let h = harness(&[], Step::Hang, 0);
        h.tx.send(logs(5)).unwrap();
        let token = CancellationToken::new();
        let worker = tokio::spawn({
            let sink = h.sink.clone();
            let token = token.clone();
            async move { sink.run(token).await }
        });
        tokio::time::sleep(Duration::from_secs(2)).await;
        token.cancel();
        worker.await.unwrap();
        assert_eq!(h.rx.len(), 1);
        let m = h.sink.metrics();
        assert_eq!(m.send_wait_time, Duration::from_secs(2));
        assert_eq!(m.total_time_spent, m.send_wait_time);
    }

    #[tokio::test(start_paused = true)]
    async fn the_backoff_starts_over_after_a_success() {
        let script = [Step::Retry, Step::Retry, Step::Deliver, Step::Retry];
        let h = harness(&script, Step::Deliver, 0);
        let (h, m) = h.run_to_end(vec![logs(5), logs(5)]).await;
        // the second batch goes out as the first lands, and waits 1s again
        assert_eq!(h.attempts(), [0, 1, 3, 3, 4]);
        assert_eq!(m.total_records_sent, 10);
    }

    #[tokio::test(start_paused = true)]
    async fn a_destination_hint_longer_than_the_backoff_wins() {
        let script = [Step::RetryAfter(Duration::from_secs(10)), Step::Retry];
        let h = harness(&script, Step::Deliver, 0);
        let (h, _) = h.run_to_end(vec![logs(5)]).await;
        // the hint, then the backoff carrying on from where it was
        assert_eq!(h.attempts(), [0, 10, 12]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_rejected_batch_is_counted_and_the_next_one_goes_out() {
        let h = harness(&[Step::Reject], Step::Deliver, 0);
        let (h, m) = h.run_to_end(vec![logs(5), logs(5)]).await;
        assert_eq!(h.attempts(), [0, 0]);
        assert_eq!(m.total_records, 10);
        assert_eq!(m.total_records_errored, 5);
        assert_eq!(m.total_records_sent, 5);
        assert_eq!(m.total_records_retried, 0);
        assert_eq!(h.sink.channel.stashed(), 0);
        assert!(h.rx.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_partial_export_retries_only_what_is_left() {
        let script = [Step::Partial {
            retry: 2,
            rejected: 1,
        }];
        let h = harness(&script, Step::Deliver, 0);
        let (h, m) = h.run_to_end(vec![logs(5)]).await;
        assert_eq!(h.attempts(), [0, 1]);
        assert_eq!(m.total_records_sent, 4);
        assert_eq!(m.total_records_errored, 1);
        assert_eq!(m.total_records_retried, 2);
        assert_eq!(
            m.total_records,
            m.total_records_sent + m.total_records_errored + m.total_records_filtered
        );
    }

    #[tokio::test(start_paused = true)]
    async fn retries_past_max_elapsed_drop_the_batch() {
        let h = harness(&[], Step::Retry, 10);
        let (h, m) = h.run_to_end(vec![logs(5)]).await;
        // the wait due at 7s would end at 15s, so it is cut to the 10s deadline
        assert_eq!(h.attempts(), [0, 1, 3, 7, 10]);
        assert_eq!(m.total_records_errored, 5);
        assert_eq!(m.total_records_sent, 0);
        assert_eq!(m.records_pending_retry, 0);
        assert_eq!(h.sink.channel.stashed(), 0);
        assert!(h.rx.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_worker_cancelled_mid_retry_requeues_its_batch() {
        let h = harness(&[], Step::Retry, 0);
        h.tx.send(logs(5)).unwrap();
        let token = CancellationToken::new();
        let worker = tokio::spawn({
            let sink = h.sink.clone();
            let token = token.clone();
            async move { sink.run(token).await }
        });
        tokio::time::sleep(Duration::from_millis(3500)).await;
        assert_eq!(h.attempts(), [0, 1, 3]);
        assert_eq!(h.sink.metrics().records_pending_retry, 5);
        token.cancel();
        worker.await.unwrap();
        // the input is still open, so the payload goes straight back on it
        assert_eq!(h.rx.len(), 1);
        assert_eq!(h.sink.channel.stashed(), 0);
        let m = h.sink.metrics();
        // 1s and 2s of backoff, and half of the 4s one
        assert_eq!(m.send_wait_time, Duration::from_millis(3500));
        assert_eq!(m.records_pending_retry, 0);
        assert_eq!(m.total_records_sent + m.total_records_errored, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_cancelled_worker_requeues_its_unsent_batch() {
        let h = harness(&[], Step::Deliver, 0);
        // 3 items do not fill a batch of 5, and the timeout is an hour away
        h.tx.send(logs(3)).unwrap();
        let token = CancellationToken::new();
        let worker = tokio::spawn({
            let sink = h.sink.clone();
            let token = token.clone();
            async move { sink.run(token).await }
        });
        tokio::time::sleep(Duration::from_secs(1)).await;
        token.cancel();
        worker.await.unwrap();
        assert!(h.attempts().is_empty());
        assert_eq!(h.rx.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn closing_with_the_destination_down_requeues_instead_of_losing_the_batch() {
        let h = harness(&[], Step::Retry, 0);
        h.tx.send(logs(5)).unwrap();
        let mut worker = Worker::with_sink(conf(0), h.sink.clone());
        worker.drain_timeout = Duration::from_secs(5);
        worker.adjust(1).await;
        let Harness { sink, tx, rx } = h;
        // the topic is closed: the worker drains what it has, which never lands
        drop(tx);
        drop(rx);
        let started = Instant::now();
        worker
            .close(Input::Ends, crate::common::shutdown::Deadline::NONE)
            .await;
        // cancelled at the drain timeout, it requeues right away instead of
        // using up its grace
        assert_eq!(started.elapsed(), worker.drain_timeout);
        // with the input gone the payload waits in the stash, which a disk
        // topic would have written back to its keyspace instead
        assert_eq!(sink.channel.stashed(), 1);
        assert_eq!(sink.metrics().total_records_errored, 0);
    }
}
