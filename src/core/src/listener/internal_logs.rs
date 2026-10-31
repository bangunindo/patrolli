// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Patrolli's own `tracing` events as log records.
//!
//! The capture side (`common::log_export`) copies matching events into a
//! bounded channel on the logging thread; this listener drains that channel,
//! turns each event into a log row and sends batches down the normal listener
//! path (annotate → processor, forwarding included).
//!
//! One registration serves every worker of a listener: `Worker::adjust` calls
//! `listen` once per worker on a shared `Arc`, and registering per call would
//! deliver each event once per worker. The first worker registers, the others
//! share its receiver (flume is MPMC), and the last one out unregisters.

use super::{InternalMetrics, Metrics};
use crate::chan::SenderChannel;
use crate::common::log_export::{self, FieldValue, LOG_EXPORT, LogEvent, LogExport, Registration};
use crate::config::listener::ListenerKind;
use crate::payload::{LogRow, Scope, Value, ZString};
use crate::{cache, config, payload};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use opentelemetry_proto::tonic::logs::v1::SeverityNumber;
use parking_lot::Mutex;
use smallvec::SmallVec;
use smol_str::SmolStr;
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::time::{Instant, sleep_until};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use tracing_subscriber::filter::Targets;
use uuid::Uuid;

const CODE_NAMESPACE: &str = "code.namespace";
const CODE_FILEPATH: &str = "code.filepath";
const CODE_LINENO: &str = "code.lineno";

pub struct InternalLogsListener {
    conf: config::listener::Listener,
    channel: SenderChannel<&'static str, payload::ProcessorPayload>,
    forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
    conf_cache: cache::Cache,
    node_id: Option<Uuid>,

    export: Arc<LogExport>,
    filter: Targets,
    buffer_size: usize,
    batch_size: usize,
    batch_timeout: Duration,
    capture: Mutex<Capture>,
    internal_metrics: InternalMetrics,
}

#[derive(Default)]
struct Capture {
    workers: usize,
    active: Option<(Registration, flume::Receiver<Arc<LogEvent>>)>,
    /// Drops booked by registrations already released, so the reported
    /// counter stays monotonic across a worker count going to 0 and back.
    retired_dropped: u64,
}

/// Held by a worker for as long as it drains; releasing the last one
/// unregisters, including when the worker is cancelled or panics.
struct CaptureGuard<'a> {
    capture: &'a Mutex<Capture>,
}

impl Drop for CaptureGuard<'_> {
    fn drop(&mut self) {
        let mut capture = self.capture.lock();
        capture.workers -= 1;
        if capture.workers == 0
            && let Some((registration, _)) = capture.active.take()
        {
            capture.retired_dropped += registration.dropped();
        }
    }
}

impl InternalLogsListener {
    pub fn new(
        conf: config::listener::Listener,
        channel: SenderChannel<&'static str, payload::ProcessorPayload>,
        forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
        conf_cache: cache::Cache,
        node_id: Option<Uuid>,
    ) -> Self {
        Self::with_export(
            conf,
            channel,
            forwarder,
            conf_cache,
            node_id,
            LOG_EXPORT.clone(),
        )
    }

    fn with_export(
        conf: config::listener::Listener,
        channel: SenderChannel<&'static str, payload::ProcessorPayload>,
        forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
        conf_cache: cache::Cache,
        node_id: Option<Uuid>,
        export: Arc<LogExport>,
    ) -> Self {
        let config::listener::Config::InternalLogs(logs) = &conf.config else {
            panic!("not internal_logs config")
        };
        // Never export this module: every batch this listener sends would
        // otherwise be able to produce another event for it to send.
        let mut exclude = logs.exclude.clone();
        exclude.push(module_path!().to_string());
        let filter = log_export::targets(logs.level, &logs.include, &exclude);
        Self {
            buffer_size: logs.buffer_size,
            batch_size: conf.batch.batch_size,
            batch_timeout: conf.batch.batch_timeout,
            conf,
            channel,
            forwarder,
            conf_cache,
            node_id,
            export,
            filter,
            capture: Mutex::new(Capture::default()),
            internal_metrics: InternalMetrics::default(),
        }
    }

    fn acquire(&self) -> (CaptureGuard<'_>, flume::Receiver<Arc<LogEvent>>) {
        let mut capture = self.capture.lock();
        capture.workers += 1;
        let rx = match &capture.active {
            Some((_, rx)) => rx.clone(),
            None => {
                let (registration, rx) =
                    self.export.register(self.filter.clone(), self.buffer_size);
                capture.active = Some((registration, rx.clone()));
                rx
            }
        };
        let guard = CaptureGuard {
            capture: &self.capture,
        };
        (guard, rx)
    }

    fn set_scope(&self, batch: &mut payload::LogsBuilder) {
        batch.scope(Scope {
            name: self.conf.name.clone(),
            listener_id: Some(self.conf.id),
            listener_type: Some(SmolStr::new_static(
                ListenerKind::from(&self.conf.config).into(),
            )),
            listener_name: Some(self.conf.name.clone()),
            monitor_type: None,
            node_id: config::NODE_ID.get().copied(),
        });
    }

    fn push(&self, batch: &mut payload::LogsBuilder, ev: &LogEvent) {
        let n = std::time::Instant::now();
        push_event(batch, ev);
        self.internal_metrics
            .engine_time_spent
            .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
        self.internal_metrics
            .total_items
            .fetch_add(1, Ordering::Relaxed);
        self.internal_metrics
            .total_records
            .fetch_add(1, Ordering::Relaxed);
        self.internal_metrics
            .total_items_succeeded
            .fetch_add(1, Ordering::Relaxed);
    }

    async fn send_batch(&self, batch: &mut payload::LogsBuilder) {
        if batch.is_empty() {
            return;
        }
        let pl = batch.finish();
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
        // `finish` resets the builder (clearing the scope), so re-arm the scope
        // before the next batch starts collecting rows.
        self.set_scope(batch);
    }

    /// Sends what was already captured when the worker was told to stop, so
    /// events logged just before a shutdown still reach the processor. Bounded
    /// by the channel length at that moment: a process that keeps logging
    /// would otherwise keep this loop alive.
    async fn drain(&self, batch: &mut payload::LogsBuilder, rx: &flume::Receiver<Arc<LogEvent>>) {
        let pending = rx.len();
        for ev in rx.try_iter().take(pending) {
            self.push(batch, &ev);
            if batch.num_rows() >= self.batch_size {
                self.send_batch(batch).await;
            }
        }
        self.send_batch(batch).await;
    }
}

fn severity(level: &tracing::Level) -> SeverityNumber {
    match *level {
        tracing::Level::TRACE => SeverityNumber::Trace,
        tracing::Level::DEBUG => SeverityNumber::Debug,
        tracing::Level::INFO => SeverityNumber::Info,
        tracing::Level::WARN => SeverityNumber::Warn,
        tracing::Level::ERROR => SeverityNumber::Error,
    }
}

fn value(value: &FieldValue) -> Value<'_> {
    match value {
        FieldValue::Bool(b) => Value::Bool(*b),
        FieldValue::I64(i) => Value::Int64(*i),
        FieldValue::U64(u) => match i64::try_from(*u) {
            Ok(i) => Value::Int64(i),
            Err(_) => Value::String(ZString::from_owned(u.to_string())),
        },
        FieldValue::F64(f) => Value::Double(*f),
        FieldValue::Str(s) => Value::String(ZString::from_ref(s.as_str())),
    }
}

/// One event as one row. Event fields and span fields are flattened into
/// record attributes under their own names; an event field shadows a span
/// field of the same name.
fn push_event(batch: &mut payload::LogsBuilder, ev: &LogEvent) {
    let mut attrs: SmallVec<[(ZString<'_>, Value<'_>); 16]> = SmallVec::new();
    attrs.push((
        ZString::from_static(CODE_NAMESPACE),
        Value::String(ZString::from_ref(ev.target.as_str())),
    ));
    if let Some(file) = &ev.file {
        attrs.push((
            ZString::from_static(CODE_FILEPATH),
            Value::String(ZString::from_ref(file.as_str())),
        ));
    }
    if let Some(line) = ev.line {
        attrs.push((ZString::from_static(CODE_LINENO), Value::Int64(line.into())));
    }
    for (name, v) in &ev.fields {
        attrs.push((ZString::from_static(name), value(v)));
    }
    for (name, v) in &ev.span_fields {
        if !ev.fields.iter().any(|(n, _)| n == name) {
            attrs.push((ZString::from_static(name), value(v)));
        }
    }
    batch.push_row(LogRow {
        observed_time: Some(DateTime::<Utc>::from(ev.timestamp)),
        severity: Some(severity(&ev.level)),
        body: (!ev.message.is_empty()).then(|| Cow::Borrowed(ev.message.as_str())),
        source_ip: None,
        source_port: None,
        attrs: &attrs,
    });
}

#[async_trait]
impl super::Listener for InternalLogsListener {
    #[tracing::instrument(skip_all, fields(listener_id = %self.conf.id, listener = %self.conf.name))]
    async fn listen(&self, token: CancellationToken) {
        let (_guard, rx) = self.acquire();
        self.internal_metrics
            .total_workers
            .fetch_add(1, Ordering::Relaxed);
        info!("starting listener");
        let mut batch = payload::LogsBuilder::new();
        self.set_scope(&mut batch);
        // armed by the first row of a batch, cleared by every flush
        let mut deadline: Option<Instant> = None;

        loop {
            let n = Instant::now();
            tokio::select! {
                biased;
                _ = token.cancelled() => {
                    debug!("stopping listener");
                    self.drain(&mut batch, &rx).await;
                    break;
                }
                _ = sleep_until(deadline.unwrap_or(n)), if deadline.is_some() => {
                    self.send_batch(&mut batch).await;
                    deadline = None;
                }
                res = rx.recv_async() => {
                    self.internal_metrics
                        .receive_wait_time
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    let Ok(ev) = res else {
                        // the sender lives in the registration this worker holds
                        warn!("capture channel closed unexpectedly");
                        break;
                    };
                    self.push(&mut batch, &ev);
                    if batch.num_rows() >= self.batch_size {
                        self.send_batch(&mut batch).await;
                        deadline = None;
                    } else if deadline.is_none() {
                        deadline = Some(Instant::now() + self.batch_timeout);
                    }
                }
            }
            self.internal_metrics
                .total_time_spent
                .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        info!("listener stopped");
        self.internal_metrics
            .total_workers
            .fetch_sub(1, Ordering::Relaxed);
    }

    async fn metrics(&self) -> Metrics {
        let mut metrics = self
            .internal_metrics
            .get(self.conf.id, self.conf.name.to_string());
        let capture = self.capture.lock();
        let active = capture
            .active
            .as_ref()
            .map_or(0, |(registration, _)| registration.dropped());
        metrics.total_items_dropped += capture.retired_dropped + active;
        metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::internal_metrics;
    use crate::config::global::{Level, LogModule};
    use crate::config::listener::{Config, InternalLogs};
    use crate::listener::Listener as _;
    use crate::processor::PROCESSOR_CHANNEL_NAME;
    use arc_swap::ArcSwap;
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::any_value::Value as AnyValue;
    use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{LogRecord, LogsData};
    use otel_arrow_dfe_pdata::{OtlpProtoBytes, TryFromWithOptions};
    use prost::Message as _;
    use std::collections::BTreeSet;
    use tokio::task::JoinHandle;
    use tracing::info_span;
    use tracing::subscriber::DefaultGuard;
    use tracing_subscriber::prelude::*;

    struct Harness {
        listener: Arc<InternalLogsListener>,
        export: Arc<LogExport>,
        processor_rx: flume::Receiver<payload::ProcessorPayload>,
        _admin: crate::chan::AdminChannel<&'static str, payload::ProcessorPayload>,
        _subscriber: DefaultGuard,
    }

    impl Harness {
        async fn new(logs: InternalLogs) -> Self {
            Self::with_batch(logs, config::Batch::default()).await
        }

        async fn with_batch(logs: InternalLogs, batch: config::Batch) -> Self {
            let export = LogExport::new();
            // thread-local default: the current-thread test runtime runs the
            // listener tasks on this same thread, so their events see it too
            let subscriber = tracing::subscriber::set_default(tracing_subscriber::registry().with(
                log_export::layer(export.clone()).with_filter(log_export::filter(export.clone())),
            ));
            let (admin, sender) = crate::chan::new(
                Duration::from_secs(3600),
                Arc::new(ArcSwap::from_pointee((
                    std::time::Instant::now(),
                    internal_metrics::Channel::default(),
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
                "internal-logs".into(),
                1,
                batch,
                Config::InternalLogs(logs),
                false,
            );
            let listener = Arc::new(InternalLogsListener::with_export(
                conf,
                sender,
                None,
                cache::Cache::new(),
                None,
                export.clone(),
            ));
            Self {
                listener,
                export,
                processor_rx,
                _admin: admin,
                _subscriber: subscriber,
            }
        }

        /// Spawns a worker and waits until it is counted, so events emitted
        /// afterwards are captured.
        async fn spawn(&self) -> (CancellationToken, JoinHandle<()>) {
            let token = CancellationToken::new();
            let before = self.listener.capture.lock().workers;
            let listener = self.listener.clone();
            let child = token.clone();
            let handle = tokio::spawn(async move { listener.listen(child).await });
            while self.listener.capture.lock().workers == before {
                tokio::task::yield_now().await;
            }
            (token, handle)
        }

        fn payloads(&self) -> Vec<payload::Payload> {
            self.processor_rx.try_iter().map(|pl| pl.payload).collect()
        }
    }

    async fn stop(worker: (CancellationToken, JoinHandle<()>)) {
        worker.0.cancel();
        worker.1.await.unwrap();
    }

    fn records(pl: &payload::Payload) -> Vec<LogRecord> {
        let OtlpProtoBytes::ExportLogsRequest(bytes) =
            OtlpProtoBytes::try_from_with_default(pl.data.clone()).unwrap()
        else {
            panic!("not logs");
        };
        LogsData::decode(bytes)
            .unwrap()
            .resource_logs
            .into_iter()
            .flat_map(|r| r.scope_logs)
            .flat_map(|s| s.log_records)
            .collect()
    }

    fn all_records(pls: &[payload::Payload]) -> Vec<LogRecord> {
        pls.iter().flat_map(records).collect()
    }

    fn attr<'a>(record: &'a LogRecord, key: &str) -> Option<&'a AnyValue> {
        record
            .attributes
            .iter()
            .find(|kv| kv.key == key)
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| v.value.as_ref())
    }

    fn body(record: &LogRecord) -> Option<&str> {
        match record.body.as_ref()?.value.as_ref()? {
            AnyValue::StringValue(s) => Some(s),
            _ => None,
        }
    }

    fn str_attr(s: &str) -> AnyValue {
        AnyValue::StringValue(s.to_string())
    }

    #[test]
    fn an_event_maps_to_one_row() {
        let export = LogExport::new();
        let (_reg, rx) = export.register(log_export::targets(Level::Info, &[], &[]), 16);
        let subscriber = tracing_subscriber::registry().with(
            log_export::layer(export.clone()).with_filter(log_export::filter(export.clone())),
        );
        tracing::subscriber::with_default(subscriber, || {
            let span = info_span!(target: "patrolli::sink", "run", sink_id = 7u64, sink = "span");
            let _e = span.enter();
            tracing::warn!(
                target: "patrolli::sink::kafka",
                sink = "event",
                ok = true,
                signed = -2i64,
                ratio = 0.25f64,
                big = u64::MAX,
                "delivery failed"
            );
        });
        let ev = rx.try_recv().unwrap();
        let mut batch = payload::LogsBuilder::new();
        push_event(&mut batch, &ev);
        let records = records(&batch.finish());
        assert_eq!(records.len(), 1);
        let r = &records[0];

        assert_eq!(body(r), Some("delivery failed"));
        assert_eq!(r.severity_number, SeverityNumber::Warn as i32);
        assert_eq!(
            r.observed_time_unix_nano,
            DateTime::<Utc>::from(ev.timestamp)
                .timestamp_nanos_opt()
                .unwrap() as u64
        );
        assert_eq!(
            attr(r, CODE_NAMESPACE),
            Some(&str_attr("patrolli::sink::kafka"))
        );
        assert_eq!(attr(r, CODE_FILEPATH), Some(&str_attr(file!())));
        assert!(matches!(attr(r, CODE_LINENO), Some(AnyValue::IntValue(_))));
        assert_eq!(attr(r, "ok"), Some(&AnyValue::BoolValue(true)));
        assert_eq!(attr(r, "signed"), Some(&AnyValue::IntValue(-2)));
        assert_eq!(attr(r, "ratio"), Some(&AnyValue::DoubleValue(0.25)));
        assert_eq!(attr(r, "big"), Some(&str_attr(&u64::MAX.to_string())));
        // span field kept, and shadowed by the event's own field of that name
        assert_eq!(attr(r, "sink_id"), Some(&AnyValue::IntValue(7)));
        assert_eq!(attr(r, "sink"), Some(&str_attr("event")));
        assert_eq!(r.attributes.iter().filter(|kv| kv.key == "sink").count(), 1);
    }

    #[tokio::test]
    async fn include_exclude_apply_and_the_listener_never_exports_itself() {
        let h = Harness::new(InternalLogs {
            include: vec![
                LogModule {
                    name: "patrolli".into(),
                    level: Level::Info,
                },
                LogModule {
                    name: "patrolli::cluster".into(),
                    level: Level::Debug,
                },
            ],
            exclude: vec!["patrolli::api".into()],
            ..Default::default()
        })
        .await;
        let worker = h.spawn().await;
        tracing::info!(target: "patrolli::sink::file", "included");
        tracing::debug!(target: "patrolli::sink::file", "below level");
        tracing::debug!(target: "patrolli::cluster::node_lock", "debug include");
        tracing::info!(target: "patrolli::api::audit", "excluded");
        tracing::info!(target: "patrolli::listener::internal_logs", "self");
        stop(worker).await;

        let records = all_records(&h.payloads());
        let bodies: Vec<_> = records.iter().filter_map(body).collect();
        // also absent: the worker's own "starting listener" / "listener stopped"
        assert_eq!(bodies, ["included", "debug include"]);
    }

    #[tokio::test(start_paused = true)]
    async fn batches_flush_on_size_and_on_timeout() {
        let h = Harness::with_batch(
            InternalLogs::default(),
            config::Batch {
                batch_size: 3,
                batch_timeout: Duration::from_millis(500),
            },
        )
        .await;
        let worker = h.spawn().await;
        let start = Instant::now();
        for i in 0..7u64 {
            tracing::info!(target: "patrolli::x", i, "event");
        }
        let mut sizes = Vec::new();
        for _ in 0..3 {
            let pl = h.processor_rx.recv_async().await.unwrap();
            sizes.push(pl.payload.num_rows());
        }
        assert_eq!(sizes, [3, 3, 1]);
        // the remainder waited for the timeout, not for a full batch
        assert!(start.elapsed() >= Duration::from_millis(500));
        stop(worker).await;
        assert!(h.payloads().is_empty());
    }

    #[tokio::test]
    async fn workers_share_one_registration_and_deliver_once() {
        let h = Harness::new(InternalLogs::default()).await;
        let a = h.spawn().await;
        let b = h.spawn().await;
        assert_eq!(h.export.subscriptions(), 1);
        for i in 0..10i64 {
            tracing::info!(target: "patrolli::x", i, "event");
        }
        // let both workers pick up events before stopping them
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        stop(a).await;
        assert_eq!(h.export.subscriptions(), 1, "b still holds it");
        stop(b).await;
        assert_eq!(h.export.subscriptions(), 0);

        let seen: Vec<i64> = all_records(&h.payloads())
            .iter()
            .filter_map(|r| match attr(r, "i") {
                Some(AnyValue::IntValue(i)) => Some(*i),
                _ => None,
            })
            .collect();
        assert_eq!(seen.len(), 10);
        assert_eq!(seen.into_iter().collect::<BTreeSet<_>>(), (0..10).collect());
    }

    #[tokio::test]
    async fn drops_are_counted_across_re_registration() {
        let h = Harness::new(InternalLogs {
            buffer_size: 1,
            ..Default::default()
        })
        .await;
        let worker = h.spawn().await;
        // no await in between: the worker can't drain, so the buffer of one
        // keeps the first event and drops the rest
        for _ in 0..5 {
            tracing::info!(target: "patrolli::x", "burst");
        }
        assert_eq!(h.listener.metrics().await.total_items_dropped, 4);
        stop(worker).await;
        assert_eq!(h.export.subscriptions(), 0);
        assert_eq!(h.listener.metrics().await.total_items_dropped, 4);

        // nothing is captured, or counted, while no worker runs
        tracing::info!(target: "patrolli::x", "unheard");

        let worker = h.spawn().await;
        assert_eq!(h.export.subscriptions(), 1);
        for _ in 0..3 {
            tracing::info!(target: "patrolli::x", "burst");
        }
        assert_eq!(h.listener.metrics().await.total_items_dropped, 6);
        stop(worker).await;

        let metrics = h.listener.metrics().await;
        assert_eq!(metrics.total_items_dropped, 6);
        assert_eq!(metrics.total_items_succeeded, 2);
        assert_eq!(metrics.total_records_sent, 2);
        assert_eq!(metrics.total_workers, 0);
    }

    #[tokio::test]
    async fn events_captured_before_a_stop_are_still_sent() {
        let h = Harness::new(InternalLogs::default()).await;
        let worker = h.spawn().await;
        for _ in 0..4 {
            tracing::info!(target: "patrolli::x", "last words");
        }
        // cancelled before the worker ever ran: only the stop-time drain can
        // send these
        stop(worker).await;
        let records = all_records(&h.payloads());
        assert_eq!(records.len(), 4);
    }

    #[tokio::test]
    async fn records_carry_the_listener_scope() {
        let h = Harness::new(InternalLogs::default()).await;
        let worker = h.spawn().await;
        tracing::info!(target: "patrolli::x", "scoped");
        stop(worker).await;
        let pls = h.payloads();
        let OtlpProtoBytes::ExportLogsRequest(bytes) =
            OtlpProtoBytes::try_from_with_default(pls[0].data.clone()).unwrap()
        else {
            panic!("not logs");
        };
        let data = LogsData::decode(bytes).unwrap();
        let scope = data.resource_logs[0].scope_logs[0].scope.as_ref().unwrap();
        let listener_type = scope
            .attributes
            .iter()
            .find(|kv| kv.key == "patrolli.listener.type")
            .and_then(|kv| kv.value.as_ref())
            .and_then(|v| v.value.as_ref());
        assert_eq!(listener_type, Some(&str_attr("internal_logs")));
    }
}
