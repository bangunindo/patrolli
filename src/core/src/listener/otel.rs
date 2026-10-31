// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::chan::SenderChannel;
use crate::listener::InternalMetrics;
use crate::{cache, common, config, payload};
use async_trait::async_trait;
use otel_arrow_dfe_pdata::encode::{
    encode_logs_otap_batch, encode_metrics_otap_batch, encode_spans_otap_batch,
};
use otel_arrow_dfe_pdata::otap::from_record_messages;
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::{
    BatchArrowRecords, BatchStatus, StatusCode,
    arrow_logs_service_server::{ArrowLogsService, ArrowLogsServiceServer},
    arrow_metrics_service_server::{ArrowMetricsService, ArrowMetricsServiceServer},
    arrow_traces_service_server::{ArrowTracesService, ArrowTracesServiceServer},
};
use otel_arrow_dfe_pdata::proto::opentelemetry::collector::{
    logs::v1::logs_service_server::{LogsService, LogsServiceServer},
    metrics::v1::metrics_service_server::{MetricsService, MetricsServiceServer},
    trace::v1::trace_service_server::{TraceService, TraceServiceServer},
};
use otel_arrow_dfe_pdata::proto::opentelemetry::collector::{
    logs::v1::{ExportLogsServiceRequest, ExportLogsServiceResponse},
    metrics::v1::{ExportMetricsServiceRequest, ExportMetricsServiceResponse},
    trace::v1::{ExportTraceServiceRequest, ExportTraceServiceResponse},
};
use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::LogsData;
use otel_arrow_dfe_pdata::proto::opentelemetry::metrics::v1::MetricsData;
use otel_arrow_dfe_pdata::proto::opentelemetry::trace::v1::TracesData;

use otel_arrow_dfe_pdata::{Consumer, OtapArrowRecords};
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio::time::{MissedTickBehavior, interval};
use tokio_stream::Stream;
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status, Streaming};
use tracing::{Instrument, Span, debug, error, info, warn};
use uuid::Uuid;

/// `serve_with_shutdown` binds the socket internally with no synchronous
/// "started" signal, so `init` waits this long for a startup (bind) error to
/// surface; if none arrives, the listener is treated as up.
const STARTUP_GRACE: Duration = Duration::from_millis(300);
/// The status message of a batch no worker took.
const NOT_HANDED_OVER: &str = "listener closing";

#[derive(Clone)]
struct OtelReceiver {
    channel: flume::Sender<payload::Payload>,
    internal_metrics: Arc<InternalMetrics>,
    ip_whitelist: Vec<IpAddr>,
    ip_blacklist: Vec<IpAddr>,
    /// Cancelled when the listener closes, which ends the OTAP streams so the
    /// server's graceful shutdown need not wait for their clients.
    intake: CancellationToken,
}

type ArrowStream = Pin<Box<dyn Stream<Item = Result<BatchStatus, Status>> + Send>>;

enum BatchType {
    Logs,
    Metrics,
    Traces,
}

fn check_allowed<T>(
    request: &Request<T>,
    ip_whitelist: &[IpAddr],
    ip_blacklist: &[IpAddr],
) -> Result<(), Status> {
    // IP allow/deny happens here, after the TCP+TLS handshake, using the peer
    // address tonic records in the request extensions. This is cheaper code
    // than running our own accept loop, at the cost of spending a handshake on
    // rejected peers.
    let allowed = match request.remote_addr() {
        Some(addr) => common::ip_allowed(&addr.ip(), ip_whitelist, ip_blacklist),
        // Peer address unknown (shouldn't happen over TCP/TLS): fail closed
        // only when a whitelist is configured.
        None => ip_whitelist.is_empty(),
    };
    if !allowed {
        debug!(peer = ?request.remote_addr(), "connection rejected by ip filter");
        return Err(Status::permission_denied("ip not allowed"));
    }
    Ok(())
}

/// Turns one OTAP message into records, transport-optimized ids resolved.
fn decode_bar(
    consumer: &mut Consumer,
    batch_type: &BatchType,
    bar: &mut BatchArrowRecords,
) -> Result<OtapArrowRecords, Status> {
    let msgs = consumer
        .consume_bar(bar)
        .map_err(|e| Status::internal(e.to_string()))?;
    let mut records = match batch_type {
        BatchType::Logs => OtapArrowRecords::Logs(
            from_record_messages(msgs).map_err(|e| Status::internal(e.to_string()))?,
        ),
        BatchType::Metrics => OtapArrowRecords::Metrics(
            from_record_messages(msgs).map_err(|e| Status::internal(e.to_string()))?,
        ),
        BatchType::Traces => OtapArrowRecords::Traces(
            from_record_messages(msgs).map_err(|e| Status::internal(e.to_string()))?,
        ),
    };
    records
        .decode_transport_optimized_ids()
        .map_err(|e| Status::internal(e.to_string()))?;
    Ok(records)
}

impl OtelReceiver {
    async fn handle_arrow_records(
        &self,
        batch_type: BatchType,
        request: Request<Streaming<BatchArrowRecords>>,
    ) -> Result<Response<ArrowStream>, Status> {
        check_allowed(&request, &self.ip_whitelist, &self.ip_blacklist)?;

        let mut inbound = request.into_inner();
        let chan = self.channel.clone();
        let im = self.internal_metrics.clone();
        let intake = self.intake.clone();
        let mut consumer = Consumer::default();

        let out = async_stream::try_stream! {
            let mut n_recv = Instant::now();
            let mut n_total = Instant::now();
            loop {
                let next = tokio::select! {
                    biased;

                    _ = intake.cancelled() => break,
                    next = inbound.message() => next,
                };
                let Some(mut bar) = next? else {
                    break;
                };
                im.receive_wait_time.fetch_add(n_recv.elapsed().as_nanos() as u64, Ordering::Relaxed);
                im.total_items.fetch_add(1, Ordering::Relaxed);
                let bytes = bar.arrow_payloads.iter().map(|pl| pl.record.len()).sum::<usize>() as u64;
                im.total_bytes.fetch_add(bytes, Ordering::Relaxed);
                im.total_time_spent.fetch_add(n_total.elapsed().as_nanos() as u64, Ordering::Relaxed);
                n_total = Instant::now();
                let n = Instant::now();
                let batch_id = bar.batch_id;
                let decoded = decode_bar(&mut consumer, &batch_type, &mut bar);
                im.engine_time_spent.fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                // A message that cannot be decoded is the item's error; `?`
                // then ends the stream with the status, as before.
                let records = decoded.inspect_err(|_| {
                    im.total_items_errored.fetch_add(1, Ordering::Relaxed);
                    im.total_bytes_errored.fetch_add(bytes, Ordering::Relaxed);
                })?;
                let num_items = records.num_items() as u64;
                im.total_records.fetch_add(num_items, Ordering::Relaxed);
                let n = Instant::now();
                let sent = chan.send_async(payload::Payload::new(records)).await.is_ok();
                im.send_wait_time.fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                if !sent {
                    // Only fails once every worker is gone: a close that ran
                    // past its drain timeout. The records never reach a
                    // pipeline, so the client must not count them delivered;
                    // `Unavailable` makes it retry them elsewhere or later.
                    // Every later batch would fail the same way.
                    im.total_items_errored.fetch_add(1, Ordering::Relaxed);
                    im.total_bytes_errored.fetch_add(bytes, Ordering::Relaxed);
                    im.total_records_errored.fetch_add(num_items, Ordering::Relaxed);
                    yield BatchStatus { batch_id, status_code: StatusCode::Unavailable as i32,
                                        status_message: NOT_HANDED_OVER.into() };
                    break;
                }
                im.total_items_succeeded.fetch_add(1, Ordering::Relaxed);
                im.total_bytes_succeeded.fetch_add(bytes, Ordering::Relaxed);
                yield BatchStatus { batch_id, status_code: StatusCode::Ok as i32,
                                    status_message: String::new() };
                n_recv = Instant::now();
            }
        };
        Ok(Response::new(Box::pin(out)))
    }

    /// The unary half of the listener: books the request as one item, resolves
    /// the encoded batch, hands it to the listener loop and answers.
    ///
    /// `n` is when the request began, so the item's engine time covers the
    /// OTLP -> OTAP encoding done by the caller. Bytes are not counted here:
    /// tonic has already decoded the protobuf, and sizing it again would mean
    /// a second walk over the whole request.
    async fn handle_otlp_request(
        &self,
        batch_type: BatchType,
        n: Instant,
        encoded: Result<OtapArrowRecords, Status>,
    ) -> Result<OtlpResponse, Status> {
        let im = &self.internal_metrics;
        im.total_items.fetch_add(1, Ordering::Relaxed);

        let decoded = encoded.and_then(|mut records| {
            records.decode_transport_optimized_ids().map_err(|e| {
                warn!(error = %e, "failed to decode transport optimized ids");
                Status::internal("failed to decode transport optimized ids")
            })?;
            Ok(records)
        });
        im.engine_time_spent
            .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
        let records = match decoded {
            Ok(records) => records,
            Err(status) => {
                im.total_items_errored.fetch_add(1, Ordering::Relaxed);
                im.total_time_spent
                    .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                return Err(status);
            }
        };

        let num_items = records.num_items() as u64;
        im.total_records.fetch_add(num_items, Ordering::Relaxed);
        let n_send = Instant::now();
        let sent = self
            .channel
            .send_async(payload::Payload::new(records))
            .await
            .is_ok();
        im.send_wait_time
            .fetch_add(n_send.elapsed().as_nanos() as u64, Ordering::Relaxed);
        im.total_time_spent
            .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
        if !sent {
            // As in the OTAP stream: every worker is gone, so the client
            // must not be told the records were delivered.
            im.total_items_errored.fetch_add(1, Ordering::Relaxed);
            im.total_records_errored
                .fetch_add(num_items, Ordering::Relaxed);
            return Err(Status::unavailable(NOT_HANDED_OVER));
        }
        im.total_items_succeeded.fetch_add(1, Ordering::Relaxed);

        match batch_type {
            BatchType::Metrics => Ok(OtlpResponse::Metrics(Response::new(
                ExportMetricsServiceResponse {
                    partial_success: None,
                },
            ))),
            BatchType::Traces => Ok(OtlpResponse::Traces(Response::new(
                ExportTraceServiceResponse {
                    partial_success: None,
                },
            ))),
            BatchType::Logs => Ok(OtlpResponse::Logs(Response::new(
                ExportLogsServiceResponse {
                    partial_success: None,
                },
            ))),
        }
    }
}

#[async_trait]
impl ArrowLogsService for OtelReceiver {
    type ArrowLogsStream = Pin<Box<dyn Stream<Item = Result<BatchStatus, Status>> + Send>>;

    async fn arrow_logs(
        &self,
        request: Request<Streaming<BatchArrowRecords>>,
    ) -> Result<Response<Self::ArrowLogsStream>, Status> {
        self.handle_arrow_records(BatchType::Logs, request).await
    }
}

#[async_trait]
impl ArrowMetricsService for OtelReceiver {
    type ArrowMetricsStream = Pin<Box<dyn Stream<Item = Result<BatchStatus, Status>> + Send>>;

    async fn arrow_metrics(
        &self,
        request: Request<Streaming<BatchArrowRecords>>,
    ) -> Result<Response<Self::ArrowMetricsStream>, Status> {
        self.handle_arrow_records(BatchType::Metrics, request).await
    }
}

#[async_trait]
impl ArrowTracesService for OtelReceiver {
    type ArrowTracesStream = Pin<Box<dyn Stream<Item = Result<BatchStatus, Status>> + Send>>;

    async fn arrow_traces(
        &self,
        request: Request<Streaming<BatchArrowRecords>>,
    ) -> Result<Response<Self::ArrowTracesStream>, Status> {
        self.handle_arrow_records(BatchType::Traces, request).await
    }
}

enum OtlpResponse {
    Metrics(Response<ExportMetricsServiceResponse>),
    Traces(Response<ExportTraceServiceResponse>),
    Logs(Response<ExportLogsServiceResponse>),
}

#[async_trait]
impl MetricsService for OtelReceiver {
    async fn export(
        &self,
        request: Request<ExportMetricsServiceRequest>,
    ) -> Result<Response<ExportMetricsServiceResponse>, Status> {
        check_allowed(&request, &self.ip_whitelist, &self.ip_blacklist)?;

        let n = Instant::now();
        let res: ExportMetricsServiceRequest = request.into_inner();
        let metrics_data = MetricsData {
            resource_metrics: res.resource_metrics,
        };
        let encoded = encode_metrics_otap_batch(&metrics_data).map_err(|e| {
            warn!(error = %e, "failed to encode metrics batch");
            Status::internal("failed to encode metrics batch")
        });

        let resp = self
            .handle_otlp_request(BatchType::Metrics, n, encoded)
            .await?;

        match resp {
            OtlpResponse::Metrics(resp) => return Ok(resp),
            _ => unreachable!("handle_otlp_request returned unexpected response type"),
        }
    }
}

#[async_trait]
impl TraceService for OtelReceiver {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        check_allowed(&request, &self.ip_whitelist, &self.ip_blacklist)?;

        let n = Instant::now();
        let res: ExportTraceServiceRequest = request.into_inner();
        let traces_data = TracesData {
            resource_spans: res.resource_spans,
        };
        let encoded = encode_spans_otap_batch(&traces_data).map_err(|e| {
            warn!(error = %e, "failed to encode traces batch");
            Status::internal("failed to encode traces batch")
        });

        let resp = self
            .handle_otlp_request(BatchType::Traces, n, encoded)
            .await?;

        match resp {
            OtlpResponse::Traces(resp) => return Ok(resp),
            _ => unreachable!("handle_otlp_request returned unexpected response type"),
        }
    }
}

#[async_trait]
impl LogsService for OtelReceiver {
    async fn export(
        &self,
        request: Request<ExportLogsServiceRequest>,
    ) -> Result<Response<ExportLogsServiceResponse>, Status> {
        check_allowed(&request, &self.ip_whitelist, &self.ip_blacklist)?;

        let n = Instant::now();
        let res: ExportLogsServiceRequest = request.into_inner();
        let logs_data = LogsData {
            resource_logs: res.resource_logs,
        };
        let encoded = encode_logs_otap_batch(&logs_data).map_err(|e| {
            warn!(error = %e, "failed to encode logs batch");
            Status::internal("failed to encode logs batch")
        });

        let resp = self
            .handle_otlp_request(BatchType::Logs, n, encoded)
            .await?;

        match resp {
            OtlpResponse::Logs(resp) => return Ok(resp),
            _ => unreachable!("handle_otlp_request returned unexpected response type"),
        }
    }
}

/// What the transport is doing. Workers start it; only `stop` ends it.
enum State {
    Idle,
    Running(Running),
    /// Stopped by the listener's close; never started again.
    Closed,
}

/// A started server. Its services hold the only senders of `rx`'s channel,
/// so the channel disconnects once the server has returned. Dropping it
/// aborts the server.
struct Running {
    rx: flume::Receiver<payload::Payload>,
    /// Shuts the server down and ends its OTAP streams. The server cancels it
    /// too if it returns on its own, which marks the transport dead.
    intake: CancellationToken,
    server: JoinSet<()>,
}

impl Running {
    /// Stops the server and waits until its requests have handed over.
    async fn shutdown(mut self) {
        self.intake.cancel();
        while let Some(res) = self.server.join_next().await {
            if let Err(e) = res {
                error!(error = %e, "server task failed");
            }
        }
    }
}

struct OtelTransport {
    conf: config::listener::OpenTelemetry,
    state: Mutex<State>,
}

impl OtelTransport {
    fn new(conf: config::listener::OpenTelemetry) -> Self {
        Self {
            conf,
            state: Mutex::new(State::Idle),
        }
    }

    /// Returns the channel the workers read, starting the server unless it
    /// runs. Every worker calls this, so a worker restarted after a failed
    /// start retries it, and one restarted after the server died starts it
    /// afresh. `None` once the listener is closed.
    async fn ensure_started(
        &self,
        internal_metrics: &Arc<InternalMetrics>,
    ) -> anyhow::Result<Option<flume::Receiver<payload::Payload>>> {
        let mut state = self.state.lock().await;
        match &*state {
            State::Running(running) if !running.intake.is_cancelled() => {
                return Ok(Some(running.rx.clone()));
            }
            State::Closed => return Ok(None),
            State::Running(_) | State::Idle => {}
        }
        if let State::Running(dead) = std::mem::replace(&mut *state, State::Idle) {
            warn!("server stopped unexpectedly, restarting");
            dead.shutdown().await;
        }
        let running = self.start(internal_metrics).await?;
        let rx = running.rx.clone();
        *state = State::Running(running);
        Ok(Some(rx))
    }

    async fn start(&self, internal_metrics: &Arc<InternalMetrics>) -> anyhow::Result<Running> {
        let (sender, rx) = flume::bounded(0);
        let intake = CancellationToken::new();
        let otap_receiver = OtelReceiver {
            channel: sender,
            internal_metrics: internal_metrics.clone(),
            ip_whitelist: self.conf.listener.ip_whitelist.clone(),
            ip_blacklist: self.conf.listener.ip_blacklist.clone(),
            intake: intake.clone(),
        };

        // Hand the whole TCP + TLS transport to tonic. Its server advertises ALPN
        // `h2`, so OTAP peers (ours and OTel's) negotiate HTTP/2 during the
        // handshake. IP allow/deny moves into the service (see `arrow_logs`).
        let mut builder = tonic::transport::Server::builder();
        if let Some(tls_conf) = &self.conf.tls {
            let server_tls = common::tls::build_tonic_server_tls(tls_conf)?;
            builder = builder.tls_config(server_tls)?;
        }

        let router = match self.conf.mode {
            config::listener::ListenerOtelMode::Both => builder
                .add_service(ArrowLogsServiceServer::new(otap_receiver.clone()))
                .add_service(ArrowMetricsServiceServer::new(otap_receiver.clone()))
                .add_service(ArrowTracesServiceServer::new(otap_receiver.clone()))
                .add_service(LogsServiceServer::new(otap_receiver.clone()))
                .add_service(MetricsServiceServer::new(otap_receiver.clone()))
                .add_service(TraceServiceServer::new(otap_receiver)),
            config::listener::ListenerOtelMode::Otap => builder
                .add_service(ArrowLogsServiceServer::new(otap_receiver.clone()))
                .add_service(ArrowMetricsServiceServer::new(otap_receiver.clone()))
                .add_service(ArrowTracesServiceServer::new(otap_receiver)),
            config::listener::ListenerOtelMode::Otlp => builder
                .add_service(LogsServiceServer::new(otap_receiver.clone()))
                .add_service(MetricsServiceServer::new(otap_receiver.clone()))
                .add_service(TraceServiceServer::new(otap_receiver)),
        };

        let addr = self.conf.listener.address;
        // Forward the serve outcome over a oneshot. A bind failure (e.g. port in
        // use) makes `serve_with_shutdown` return almost immediately, so if no
        // error arrives within the grace window we treat the bind as successful.
        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<anyhow::Result<()>>();
        let mut server = JoinSet::new();
        let shutdown = intake.clone();
        server.spawn(
            async move {
                // However this returns, the transport is done.
                let _intake_guard = shutdown.clone().drop_guard();
                let res = router
                    .serve_with_shutdown(addr, async move { shutdown.cancelled().await })
                    .await;
                // Receiver is gone once `start` accepted the startup, so a failure
                // that can't be handed back is a runtime serve error: log it here.
                // A startup failure is returned by `start` and logged by its caller.
                if let Err(Err(e)) = started_tx.send(res.map_err(anyhow::Error::from)) {
                    error!(error = %format!("{e:#}"), "server failed");
                }
            }
            .instrument(Span::current()),
        );

        match tokio::time::timeout(STARTUP_GRACE, started_rx).await {
            // Serve returned within the grace window → startup failed.
            Ok(Ok(Err(e))) => return Err(e.context("OtapArrow server failed to start")),
            Ok(Ok(Ok(()))) => {
                return Err(anyhow::anyhow!(
                    "OtapArrow server stopped immediately after start"
                ));
            }
            Ok(Err(_)) => return Err(anyhow::anyhow!("OtapArrow server task ended unexpectedly")),
            // Still serving after the grace window → bind succeeded.
            Err(_) => info!(addr = %addr, "server listening"),
        }

        Ok(Running { rx, intake, server })
    }

    /// Stops the server for good: it stops accepting, its streams end, and
    /// its requests in flight hand over to the workers, which must keep
    /// receiving meanwhile; the caller bounds the wait.
    async fn stop(&self) {
        let state = std::mem::replace(&mut *self.state.lock().await, State::Closed);
        if let State::Running(running) = state {
            running.shutdown().await;
            debug!("server stopped");
        }
    }
}

pub struct OtelListener {
    conf: config::listener::Listener,
    internal_metrics: Arc<InternalMetrics>,
    channel: SenderChannel<&'static str, payload::ProcessorPayload>,
    forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
    conf_cache: cache::Cache,
    node_id: Option<Uuid>,

    transport: Arc<OtelTransport>,
}

impl OtelListener {
    pub fn new(
        conf: config::listener::Listener,
        channel: SenderChannel<&'static str, payload::ProcessorPayload>,
        forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
        conf_cache: cache::Cache,
        node_id: Option<Uuid>,
    ) -> Self {
        let config::listener::Config::OpenTelemetry(otap_conf) = conf.config.clone() else {
            panic!("OtelListener configured with non-OtapArrow config type");
        };
        Self {
            conf,
            internal_metrics: Arc::new(InternalMetrics::default()),
            channel,
            forwarder,
            conf_cache,
            node_id,
            transport: Arc::new(OtelTransport::new(otap_conf)),
        }
    }

    async fn send_batch(&self, batch: &mut payload::PayloadBatch) {
        for payload in batch.batch(Some(self.conf.batch.batch_size)) {
            super::send_batch(super::SendBatch {
                batch: payload,
                conf: &self.conf,
                conf_cache: &self.conf_cache,
                node_id: self.node_id,
                internal_metrics: &self.internal_metrics,
                channel: &self.channel,
                forwarder: &self.forwarder,
            })
            .await;
        }
    }
}

#[async_trait]
impl super::Listener for OtelListener {
    #[tracing::instrument(skip_all, fields(listener_id = %self.conf.id, listener = %self.conf.name))]
    async fn listen(&self, token: CancellationToken) {
        let recv = match self.transport.ensure_started(&self.internal_metrics).await {
            Ok(Some(recv)) => recv,
            // Closed meanwhile.
            Ok(None) => return,
            Err(e) => {
                error!(error = %format!("{e:#}"), "transport failed to start");
                return;
            }
        };
        self.internal_metrics
            .total_workers
            .fetch_add(1, Ordering::Relaxed);
        info!("starting listener");
        let mut batch = payload::PayloadBatch::new();
        let mut batch_tick = interval(self.conf.batch.batch_timeout);
        batch_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = token.cancelled() => {
                    break;
                }
                _ = batch_tick.tick() => {
                    if !batch.is_empty() {
                        self.send_batch(&mut batch).await;
                    }
                }
                res = recv.recv_async() => {
                    // Disconnected: the server stopped, or died and the
                    // supervisor restarts this worker.
                    let Ok(res) = res else {
                        debug!("transport closed");
                        break;
                    };
                    batch.push(res);
                    if batch.num_items() >= self.conf.batch.batch_size {
                        self.send_batch(&mut batch).await;
                        batch_tick.reset();
                    }
                }
            }
        }
        // The channel is a rendezvous, so nothing is queued past this batch.
        if !batch.is_empty() {
            self.send_batch(&mut batch).await;
        }
        info!("listener stopped");
        self.internal_metrics
            .total_workers
            .fetch_sub(1, Ordering::Relaxed);
    }

    async fn stop(&self) {
        self.transport.stop().await;
    }

    async fn metrics(&self) -> common::internal_metrics::Listener {
        self.internal_metrics
            .get(self.conf.id, self.conf.name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otel_arrow_dfe_pdata::Producer;
    use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::arrow_logs_service_client::ArrowLogsServiceClient;
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::{AnyValue, any_value};
    use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
        LogRecord, ResourceLogs, ScopeLogs,
    };

    fn logs() -> LogsData {
        LogsData {
            resource_logs: vec![ResourceLogs {
                scope_logs: vec![ScopeLogs {
                    log_records: vec![LogRecord {
                        time_unix_nano: 1,
                        body: Some(AnyValue {
                            value: Some(any_value::Value::StringValue("hello".into())),
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
    }

    /// A receiver and the workers' end of its rendezvous channel. Dropping
    /// that end is what a close past its drain timeout leaves behind.
    fn receiver() -> (OtelReceiver, flume::Receiver<payload::Payload>) {
        let (channel, rx) = flume::bounded(0);
        let recv = OtelReceiver {
            channel,
            internal_metrics: Arc::default(),
            ip_whitelist: Vec::new(),
            ip_blacklist: Vec::new(),
            intake: CancellationToken::new(),
        };
        (recv, rx)
    }

    /// A worker taking every batch.
    fn take_all(rx: flume::Receiver<payload::Payload>) {
        tokio::spawn(async move { while rx.recv_async().await.is_ok() {} });
    }

    /// (items succeeded, items errored, records errored)
    fn outcome(recv: &OtelReceiver) -> (u64, u64, u64) {
        let m = &recv.internal_metrics;
        (
            m.total_items_succeeded.load(Ordering::Relaxed),
            m.total_items_errored.load(Ordering::Relaxed),
            m.total_records_errored.load(Ordering::Relaxed),
        )
    }

    async fn export(recv: &OtelReceiver) -> Result<(), Status> {
        let request = ExportLogsServiceRequest {
            resource_logs: logs().resource_logs,
        };
        LogsService::export(recv, Request::new(request))
            .await
            .map(|_| ())
    }

    /// Serves the OTAP logs service on a free port and sends it one batch;
    /// returns the batch's status.
    async fn send_bar(recv: OtelReceiver) -> BatchStatus {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let incoming = async_stream::stream! {
            loop {
                yield listener.accept().await.map(|(stream, _)| stream);
            }
        };
        tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(ArrowLogsServiceServer::new(recv))
                .serve_with_incoming(incoming),
        );
        let mut client = ArrowLogsServiceClient::connect(format!("http://{addr}"))
            .await
            .unwrap();
        let mut records = encode_logs_otap_batch(&logs()).unwrap();
        let bar = Producer::new().produce_bar(&mut records).unwrap();
        let mut statuses = client
            .arrow_logs(tokio_stream::iter([bar]))
            .await
            .unwrap()
            .into_inner();
        statuses
            .message()
            .await
            .unwrap()
            .expect("no status for the batch")
    }

    // N4: the client was answered `Ok` for a batch no worker took.
    #[tokio::test]
    async fn otlp_export_fails_when_not_handed_over() {
        let (recv, rx) = receiver();
        drop(rx);
        let status = export(&recv).await.unwrap_err();
        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert_eq!(outcome(&recv), (0, 1, 1));
    }

    #[tokio::test]
    async fn otlp_export_handed_over_is_acked() {
        let (recv, rx) = receiver();
        take_all(rx);
        export(&recv).await.unwrap();
        assert_eq!(outcome(&recv), (1, 0, 0));
    }

    // N4, OTAP: the batch status was `Ok` whether or not a worker took it.
    #[tokio::test]
    async fn otap_batch_not_handed_over_is_not_acked() {
        let (recv, rx) = receiver();
        drop(rx);
        let status = send_bar(recv.clone()).await;
        assert_eq!(status.status_code, StatusCode::Unavailable as i32);
        assert_eq!(outcome(&recv), (0, 1, 1));
    }

    #[tokio::test]
    async fn otap_batch_handed_over_is_acked() {
        let (recv, rx) = receiver();
        take_all(rx);
        let status = send_bar(recv.clone()).await;
        assert_eq!(status.status_code, StatusCode::Ok as i32);
        assert_eq!(outcome(&recv), (1, 0, 0));
    }
}
