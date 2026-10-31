// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Patrolli reporting on itself.
//!
//! Every `interval` the listener renders this node's
//! [`crate::common::internal_metrics::INTERNAL_METRICS`] snapshots and sends
//! them down the normal listener path (annotate → processor, forwarding
//! included). Every main node runs its listeners, so each node reports itself,
//! told apart by the scope's `patrolli.node.id`.
//!
//! Two renderers, selected by [`InternalMetricsSignal`]:
//!
//! - [`log`] — one log record whose attributes are a nested document of every
//!   subsystem, optionally reported as per-second rates.
//! - [`metric`] — one OTLP metric per family, cumulative, with identity in
//!   data-point attributes. Shares its catalogue with the Prometheus exporter.
//!
//! One worker emits per listener: `Worker::adjust` calls `listen` once per
//! worker on a shared `Arc`, and ticking in each would report every snapshot
//! once per worker. The workers queue on [`Emitter`]; the one holding it ticks,
//! and when it is cancelled the next one takes over the schedule and the
//! throughput baseline where it left off.

mod log;
mod metric;

use super::{InternalMetrics, Metrics};
use crate::chan::SenderChannel;
use crate::common::internal_metrics::{self as snapshots, INTERNAL_METRICS};
use crate::config::listener::{InternalMetricsSignal, ListenerKind};
use crate::payload::{LogRowOwned, Scope};
use crate::{cache, config, payload};
use async_trait::async_trait;
use chrono::Utc;
use smol_str::SmolStr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::{Instant, MissedTickBehavior, interval_at};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};
use uuid::Uuid;

pub struct InternalMetricsListener {
    conf: config::listener::Listener,
    channel: SenderChannel<&'static str, payload::ProcessorPayload>,
    forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
    conf_cache: cache::Cache,
    node_id: Option<Uuid>,

    snapshots: Arc<snapshots::InternalMetrics>,
    signal: InternalMetricsSignal,
    report_throughput: bool,
    interval: Duration,
    emitter: Mutex<Emitter>,
    internal_metrics: InternalMetrics,
}

/// What the emitting worker hands over to the next one.
#[derive(Default)]
struct Emitter {
    /// When the next report is due; `None` until the first one.
    next: Option<Instant>,
    /// Snapshots the previous throughput pass diffed against.
    baseline: Option<log::PrevValue>,
}

impl InternalMetricsListener {
    pub fn new(
        conf: config::listener::Listener,
        channel: SenderChannel<&'static str, payload::ProcessorPayload>,
        forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
        conf_cache: cache::Cache,
        node_id: Option<Uuid>,
    ) -> Self {
        Self::with_snapshots(
            conf,
            channel,
            forwarder,
            conf_cache,
            node_id,
            INTERNAL_METRICS.clone(),
        )
    }

    fn with_snapshots(
        conf: config::listener::Listener,
        channel: SenderChannel<&'static str, payload::ProcessorPayload>,
        forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
        conf_cache: cache::Cache,
        node_id: Option<Uuid>,
        snapshots: Arc<snapshots::InternalMetrics>,
    ) -> Self {
        let config::listener::Config::InternalMetrics(metrics) = &conf.config else {
            panic!("not internal_metrics config")
        };
        Self {
            signal: metrics.signal,
            report_throughput: metrics.report_throughput,
            interval: metrics.interval,
            conf,
            channel,
            forwarder,
            conf_cache,
            node_id,
            snapshots,
            emitter: Mutex::new(Emitter::default()),
            internal_metrics: InternalMetrics::default(),
        }
    }

    fn scope(&self) -> Scope {
        Scope {
            name: self.conf.name.clone(),
            listener_id: Some(self.conf.id),
            listener_type: Some(SmolStr::new_static(
                ListenerKind::from(&self.conf.config).into(),
            )),
            listener_name: Some(self.conf.name.clone()),
            monitor_type: None,
            node_id: config::NODE_ID.get().copied(),
        }
    }

    /// The payload for one report and its row count, or `None` when there is
    /// nothing to send: a throughput pass that only primed its baseline, or
    /// one whose snapshots have not refreshed since the previous pass.
    fn render(&self, baseline: &mut Option<log::PrevValue>) -> Option<payload::Payload> {
        match self.signal {
            InternalMetricsSignal::Log => {
                let attrs = log::render(&self.snapshots, self.report_throughput, baseline)?;
                if attrs.is_empty() {
                    return None;
                }
                let mut batch = payload::LogsBuilder::new();
                batch.scope(self.scope());
                batch.push_row_owned(LogRowOwned {
                    observed_time: Some(Utc::now()),
                    severity: None,
                    body: None,
                    source_ip: None,
                    source_port: None,
                    attrs,
                });
                Some(batch.finish())
            }
            InternalMetricsSignal::Metrics => {
                let rows = metric::render(&self.snapshots);
                if rows.is_empty() {
                    return None;
                }
                let mut batch = payload::MetricsBuilder::new();
                batch.scope(self.scope());
                for row in rows {
                    batch.push_row_owned(row);
                }
                Some(batch.finish())
            }
        }
    }

    async fn report(&self, baseline: &mut Option<log::PrevValue>) {
        let n = std::time::Instant::now();
        let rendered = self.render(baseline);
        self.internal_metrics
            .engine_time_spent
            .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
        if let Some(pl) = rendered {
            self.internal_metrics
                .total_items
                .fetch_add(1, Ordering::Relaxed);
            self.internal_metrics
                .total_records
                .fetch_add(pl.num_items() as u64, Ordering::Relaxed);
            self.internal_metrics
                .total_items_succeeded
                .fetch_add(1, Ordering::Relaxed);
            super::send_batch(super::SendBatch {
                batch: pl,
                conf: &self.conf,
                conf_cache: &self.conf_cache,
                node_id: self.node_id,
                internal_metrics: &self.internal_metrics,
                channel: &self.channel,
                forwarder: &self.forwarder,
            })
            .await;
        }
        self.internal_metrics
            .total_time_spent
            .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }

    /// Reports every `interval` until cancelled, continuing the schedule the
    /// previous emitter left in `emitter`.
    async fn emit(&self, token: &CancellationToken, emitter: &mut Emitter) {
        debug!("emitting");
        let start = emitter.next.unwrap_or_else(Instant::now);
        let mut ticker = interval_at(start, self.interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                _ = token.cancelled() => return,
                due = ticker.tick() => {
                    emitter.next = Some(due + self.interval);
                    self.report(&mut emitter.baseline).await;
                }
            }
        }
    }
}

#[async_trait]
impl super::Listener for InternalMetricsListener {
    #[tracing::instrument(skip_all, fields(listener_id = %self.conf.id, listener = %self.conf.name))]
    async fn listen(&self, token: CancellationToken) {
        self.internal_metrics
            .total_workers
            .fetch_add(1, Ordering::Relaxed);
        info!("starting listener");
        tokio::select! {
            biased;
            _ = token.cancelled() => {}
            mut emitter = self.emitter.lock() => self.emit(&token, &mut emitter).await,
        }
        info!("listener stopped");
        self.internal_metrics
            .total_workers
            .fetch_sub(1, Ordering::Relaxed);
    }

    async fn metrics(&self) -> Metrics {
        self.internal_metrics
            .get(self.conf.id, self.conf.name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::internal_metrics::Sink;
    use crate::config::listener::{Config, InternalMetrics as InternalMetricsConfig};
    use crate::listener::Listener as _;
    use crate::processor::PROCESSOR_CHANNEL_NAME;
    use arc_swap::ArcSwap;
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::KeyValue;
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::any_value::Value as AnyValue;
    use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{LogRecord, LogsData};
    use otel_arrow_dfe_pdata::proto::opentelemetry::metrics::v1::{Metric, MetricsData, metric};
    use otel_arrow_dfe_pdata::{OtlpProtoBytes, TryFromWithOptions};
    use prost::Message as _;
    use tokio::task::JoinHandle;
    use tokio::time::sleep;

    const INTERVAL: Duration = Duration::from_secs(10);

    struct Harness {
        listener: Arc<InternalMetricsListener>,
        snapshots: Arc<snapshots::InternalMetrics>,
        sink_id: Uuid,
        processor_rx: flume::Receiver<payload::ProcessorPayload>,
        _admin: crate::chan::AdminChannel<&'static str, payload::ProcessorPayload>,
    }

    impl Harness {
        async fn new(signal: InternalMetricsSignal, report_throughput: bool) -> Self {
            // isolated from the process-wide snapshots other tests may touch
            let snapshots = Arc::new(snapshots::InternalMetrics::default());
            let (admin, sender) = crate::chan::new(
                Duration::from_secs(3600),
                Arc::new(ArcSwap::from_pointee((
                    std::time::Instant::now(),
                    snapshots::Channel::default(),
                ))),
            );
            let processor_rx = admin
                // the default buffer is 0, a rendezvous channel: every send
                // would wait for the test to receive
                .get_receiver(
                    &PROCESSOR_CHANNEL_NAME,
                    config::Channel::Memory(config::MemoryChannel {
                        non_blocking: false,
                        channel_buffer_size: 1024,
                    }),
                )
                .await
                .unwrap();
            let conf = config::listener::Listener::new(
                Uuid::now_v7(),
                "internal-metrics".into(),
                1,
                config::Batch::default(),
                Config::InternalMetrics(InternalMetricsConfig {
                    signal,
                    report_throughput,
                    interval: INTERVAL,
                }),
                false,
            );
            let listener = Arc::new(InternalMetricsListener::with_snapshots(
                conf,
                sender,
                None,
                cache::Cache::new(),
                None,
                snapshots.clone(),
            ));
            let harness = Self {
                listener,
                snapshots,
                sink_id: Uuid::now_v7(),
                processor_rx,
                _admin: admin,
            };
            harness.store_sink(std::time::Instant::now(), 10);
            harness
        }

        fn store_sink(&self, at: std::time::Instant, total_records: u64) {
            self.snapshots.sink.store(Arc::new((
                at,
                vec![Sink {
                    id: self.sink_id,
                    name: "primary".to_string(),
                    total_records,
                    ..Default::default()
                }],
            )));
        }

        /// A refreshed sink snapshot one second after the current one,
        /// `more` items further along.
        fn advance_sink(&self, more: u64) {
            let current = self.snapshots.sink.load_full();
            let total = current.1[0].total_records + more;
            self.store_sink(current.0 + Duration::from_secs(1), total);
        }

        fn spawn(&self) -> (CancellationToken, JoinHandle<()>) {
            let token = CancellationToken::new();
            let listener = self.listener.clone();
            let child = token.clone();
            (
                token,
                tokio::spawn(async move { listener.listen(child).await }),
            )
        }

        fn payloads(&self) -> Vec<payload::Payload> {
            self.processor_rx.try_iter().map(|pl| pl.payload).collect()
        }
    }

    async fn stop(worker: (CancellationToken, JoinHandle<()>)) {
        worker.0.cancel();
        worker.1.await.unwrap();
    }

    fn logs(pl: &payload::Payload) -> LogsData {
        let OtlpProtoBytes::ExportLogsRequest(bytes) =
            OtlpProtoBytes::try_from_with_default(pl.data.clone()).unwrap()
        else {
            panic!("not logs");
        };
        LogsData::decode(bytes).unwrap()
    }

    fn log_records(pl: &payload::Payload) -> Vec<LogRecord> {
        logs(pl)
            .resource_logs
            .into_iter()
            .flat_map(|r| r.scope_logs)
            .flat_map(|s| s.log_records)
            .collect()
    }

    fn value<'a>(attrs: &'a [KeyValue], key: &str) -> Option<&'a AnyValue> {
        attrs
            .iter()
            .find(|kv| kv.key == key)
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| v.value.as_ref())
    }

    /// Nested attributes arrive flattened into dotted keys (`sink.0.total_records`).
    fn has_section(attrs: &[KeyValue], section: &str) -> bool {
        let prefix = format!("{section}.");
        attrs.iter().any(|kv| kv.key.starts_with(&prefix))
    }

    #[tokio::test(start_paused = true)]
    async fn log_signal_reports_every_subsystem_under_the_listener_scope() {
        let h = Harness::new(InternalMetricsSignal::Log, false).await;
        let worker = h.spawn();
        sleep(INTERVAL / 2).await;
        stop(worker).await;

        let payloads = h.payloads();
        assert_eq!(payloads.len(), 1);
        let data = logs(&payloads[0]);
        let scope = data.resource_logs[0].scope_logs[0].scope.as_ref().unwrap();
        assert_eq!(
            value(&scope.attributes, "patrolli.listener.type"),
            Some(&AnyValue::StringValue("internal_metrics".into()))
        );
        let records = log_records(&payloads[0]);
        assert_eq!(records.len(), 1);
        let attrs = &records[0].attributes;
        // the per-entity sections other than `sink` are empty lists here, and an
        // empty list flattens to no key at all
        for section in [
            "sink_channel",
            "processor_channel",
            "monitor",
            "processor",
            "sink",
        ] {
            assert!(has_section(attrs, section), "missing {section}");
        }
        assert_eq!(
            value(attrs, "sink.0.total_records"),
            Some(&AnyValue::IntValue(10))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn metrics_signal_reports_cumulative_families() {
        let h = Harness::new(InternalMetricsSignal::Metrics, false).await;
        let worker = h.spawn();
        sleep(INTERVAL / 2).await;
        stop(worker).await;

        let payloads = h.payloads();
        assert_eq!(payloads.len(), 1);
        let OtlpProtoBytes::ExportMetricsRequest(bytes) =
            OtlpProtoBytes::try_from_with_default(payloads[0].data.clone()).unwrap()
        else {
            panic!("not metrics");
        };
        let metrics: Vec<Metric> = MetricsData::decode(bytes)
            .unwrap()
            .resource_metrics
            .into_iter()
            .flat_map(|r| r.scope_metrics)
            .flat_map(|s| s.metrics)
            .collect();
        assert!(!metrics.is_empty());
        assert!(metrics.iter().all(|m| m.name.starts_with("patrolli.")));
        let sums: Vec<_> = metrics
            .iter()
            .filter_map(|m| match &m.data {
                Some(metric::Data::Sum(sum)) => Some(sum),
                _ => None,
            })
            .collect();
        assert!(!sums.is_empty());
        let start = common_start_nanos();
        for sum in sums {
            assert!(sum.is_monotonic);
            assert!(
                sum.data_points
                    .iter()
                    .all(|dp| dp.start_time_unix_nano == start)
            );
        }
    }

    fn common_start_nanos() -> u64 {
        crate::common::get_start_datetime()
            .with_timezone(&Utc)
            .timestamp_nanos_opt()
            .unwrap() as u64
    }

    #[tokio::test(start_paused = true)]
    async fn throughput_primes_first_then_reports_rates_and_skips_stale_snapshots() {
        let h = Harness::new(InternalMetricsSignal::Log, true).await;
        let worker = h.spawn();
        sleep(INTERVAL / 2).await;
        assert!(h.payloads().is_empty(), "the first pass only primes");

        h.advance_sink(30);
        sleep(INTERVAL).await;
        let payloads = h.payloads();
        assert_eq!(payloads.len(), 1);
        let records = log_records(&payloads[0]);
        assert_eq!(
            value(&records[0].attributes, "sink.0.total_records"),
            Some(&AnyValue::DoubleValue(30.0))
        );

        // nothing refreshed since the previous pass: nothing to report
        sleep(INTERVAL).await;
        assert!(h.payloads().is_empty());
        stop(worker).await;
    }

    #[tokio::test(start_paused = true)]
    async fn listeners_keep_their_own_baselines() {
        let a = Harness::new(InternalMetricsSignal::Log, true).await;
        let b = Harness::new(InternalMetricsSignal::Log, true).await;
        // both read the same snapshots, as two listeners on one node do
        let b_listener = Arc::new(InternalMetricsListener::with_snapshots(
            b.listener.conf.clone(),
            b.listener.channel.clone(),
            None,
            cache::Cache::new(),
            None,
            a.snapshots.clone(),
        ));
        let spawn_b = || {
            let token = CancellationToken::new();
            let listener = b_listener.clone();
            let child = token.clone();
            (
                token,
                tokio::spawn(async move { listener.listen(child).await }),
            )
        };
        let wa = a.spawn();
        let wb = spawn_b();
        sleep(INTERVAL / 2).await;
        a.advance_sink(20);
        sleep(INTERVAL).await;
        stop(wa).await;
        stop(wb).await;

        for rx in [&a, &b] {
            let payloads = rx.payloads();
            assert_eq!(payloads.len(), 1, "each listener reports its own window");
            assert_eq!(
                value(
                    &log_records(&payloads[0])[0].attributes,
                    "sink.0.total_records"
                ),
                Some(&AnyValue::DoubleValue(20.0))
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn one_worker_emits_and_a_successor_keeps_schedule_and_baseline() {
        let h = Harness::new(InternalMetricsSignal::Log, true).await;
        let first = h.spawn();
        let second = h.spawn();
        let third = h.spawn();

        // ticks at 0 (primes), I and 2I; the test wakes between ticks
        sleep(INTERVAL / 2).await;
        for _ in 0..2 {
            h.advance_sink(10);
            sleep(INTERVAL).await;
        }
        assert_eq!(h.payloads().len(), 2, "one report per tick, not per worker");
        assert_eq!(
            h.listener
                .internal_metrics
                .total_workers
                .load(Ordering::Relaxed),
            3
        );

        // `Worker::adjust` scales down from the oldest unit: the emitter
        // refreshed before the handover at 2.5I, so a successor that reported
        // on taking over would send it right away
        h.advance_sink(10);
        stop(first).await;
        sleep(INTERVAL / 4).await;
        assert!(h.payloads().is_empty(), "the next report stays due at 3I");
        sleep(INTERVAL / 2).await;
        let payloads = h.payloads();
        assert_eq!(payloads.len(), 1);
        assert!(
            value(
                &log_records(&payloads[0])[0].attributes,
                "sink.0.total_records"
            ) == Some(&AnyValue::DoubleValue(10.0)),
            "the successor reports rates instead of priming again"
        );

        stop(second).await;
        stop(third).await;
        assert_eq!(
            h.listener
                .internal_metrics
                .total_workers
                .load(Ordering::Relaxed),
            0
        );
        sleep(INTERVAL * 3).await;
        assert!(
            h.payloads().is_empty(),
            "nothing is sent once every worker stopped"
        );
    }
}
