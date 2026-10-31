// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! A client for one OpenTelemetry destination, speaking OTAP (Arrow streams)
//! or OTLP (unary requests) over a few gRPC connections.
//!
//! Each payload is handed over as a [`Job`] with a oneshot that the client
//! settles once the destination has answered for it: a `BatchStatus` on the
//! OTAP stream, the response to an OTLP export. A connection that ends before
//! the answer drops the oneshot, which the sender reads as "retry".

use crate::common::tls;
use crate::config::sink::OtelSinkMode;
use crate::config::{OtapCompression, Tls};
use crate::payload::{self};
use anyhow::{Result, anyhow};
use arrow_ipc::CompressionType;
use flume::{Receiver, Sender};
use otel_arrow_dfe_pdata::encode::producer::ProducerOptions;
use otel_arrow_dfe_pdata::otlp::logs::LogsProtoBytesEncoder;
use otel_arrow_dfe_pdata::otlp::metrics::MetricsProtoBytesEncoder;
use otel_arrow_dfe_pdata::otlp::traces::TracesProtoBytesEncoder;
use otel_arrow_dfe_pdata::otlp::{ProtoBuffer, ProtoBytesEncoder};
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::{
    BatchArrowRecords, BatchStatus, StatusCode, arrow_logs_service_client::ArrowLogsServiceClient,
    arrow_metrics_service_client::ArrowMetricsServiceClient,
    arrow_traces_service_client::ArrowTracesServiceClient,
};
use otel_arrow_dfe_pdata::proto::opentelemetry::collector::logs::v1::{
    ExportLogsServiceRequest, logs_service_client::LogsServiceClient,
};
use otel_arrow_dfe_pdata::proto::opentelemetry::collector::metrics::v1::{
    ExportMetricsServiceRequest, metrics_service_client::MetricsServiceClient,
};
use otel_arrow_dfe_pdata::proto::opentelemetry::collector::trace::v1::{
    ExportTraceServiceRequest, trace_service_client::TraceServiceClient,
};
use otel_arrow_dfe_pdata::{OtapArrowRecords, Producer};
use parking_lot::Mutex;
use prost::Message;
use rustc_hash::FxHashMap as HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tonic::transport::{Channel, Endpoint};
use tonic::{Code, Request, Response, Status, Streaming};
use tracing::{Instrument, Span, error, info, trace, warn};

/// Buffer between our producer and tonic's HTTP/2 writer. Bounded so a slow peer
/// applies backpressure on `produce_bar` rather than letting BARs pile up.
const STREAM_BUFFER: usize = 64;

/// How often tonic sends an HTTP/2 keepalive PING to detect a dead peer.
const HTTP2_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(20);

/// How long [`OtelClient::close`] lets each connection flush the requests it
/// already queued to tonic before giving up on them.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

pub struct OtelClientConfig {
    pub compression: OtapCompression,
    pub tls: Option<Tls>,
    /// Unlike the pure-TCP patrolli client (which uses this as `TCP_USER_TIMEOUT`),
    /// here it bounds how long we wait for an HTTP/2 keepalive PING ack before
    /// declaring the connection dead. Zero leaves hyper's default in place.
    pub write_timeout: Duration,
    pub sink_mode: OtelSinkMode,
}

/// Maps our wire compression knob onto Arrow IPC compression. Unlike the patrolli
/// protocol (which compresses the whole frame), OTAP compresses each Arrow IPC
/// batch, so this is handed straight to the [`Producer`].
fn ipc_compression(compression: OtapCompression) -> Option<CompressionType> {
    match compression {
        OtapCompression::None => None,
        OtapCompression::Lz4 => Some(CompressionType::LZ4_FRAME),
        OtapCompression::Zstd => Some(CompressionType::ZSTD),
    }
}

pub use crate::common::internal_metrics::NodeMetrics;

/// What the destination made of one [`Job`]'s payload.
pub enum Answer {
    Delivered,
    /// Nothing landed, and the destination may take it later. A dropped
    /// oneshot (the connection ended before an answer) means the same.
    /// `after` is the wait the destination asked for, if it did.
    Retry {
        error: anyhow::Error,
        after: Option<Duration>,
    },
    /// `items` of the payload were refused for good; the rest landed.
    Rejected {
        items: u64,
        error: anyhow::Error,
    },
}

/// A payload to send, and where to answer for it.
pub struct Job {
    pub payload: payload::Payload,
    pub answer: oneshot::Sender<Answer>,
}

/// The answer still owed for a payload handed to tonic.
struct Owed {
    answer: oneshot::Sender<Answer>,
    items: u64,
    bytes: u64,
}

impl Owed {
    fn settle(self, metrics: &NodeMetrics, answer: Answer) {
        match &answer {
            Answer::Delivered => {
                metrics
                    .total_records_sent
                    .fetch_add(self.items, Ordering::Relaxed);
                metrics
                    .total_bytes_sent
                    .fetch_add(self.bytes, Ordering::Relaxed);
            }
            Answer::Rejected { items, .. } => {
                metrics
                    .total_records_errored
                    .fetch_add(*items, Ordering::Relaxed);
                metrics
                    .total_bytes_errored
                    .fetch_add(self.bytes, Ordering::Relaxed);
            }
            Answer::Retry { .. } => {}
        }
        // The sender stopped waiting (a cancelled worker): nobody to tell.
        let _ = self.answer.send(answer);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Signal {
    Logs,
    Traces,
    Metrics,
}

impl Signal {
    fn of(data: &OtapArrowRecords) -> Self {
        match data {
            OtapArrowRecords::Logs(_) => Self::Logs,
            OtapArrowRecords::Traces(_) => Self::Traces,
            OtapArrowRecords::Metrics(_) => Self::Metrics,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Logs => "logs",
            Self::Traces => "traces",
            Self::Metrics => "metrics",
        }
    }
}

/// What a connection's stream tasks report to its main loop.
enum Event {
    /// The destination's answer to one OTAP batch.
    Status(Signal, BatchStatus),
    /// A stream ended: `Ok` when the destination closed it.
    Closed(Result<()>),
}

pub struct OtelClient {
    config: Arc<OtelClientConfig>,
    dst: String,
    units: Mutex<Vec<JoinHandle<()>>>,
    receiver: Receiver<Job>,
    token: CancellationToken,
    pub metrics: Arc<NodeMetrics>,
}

impl OtelClient {
    pub fn new(
        config: Arc<OtelClientConfig>,
        dst: String,
        metrics: Arc<NodeMetrics>,
    ) -> (Arc<Self>, Sender<Job>) {
        let (sender, receiver) = flume::bounded(0);
        (
            Arc::new(Self {
                config,
                dst,
                units: Mutex::new(Vec::new()),
                receiver,
                token: CancellationToken::new(),
                metrics,
            }),
            sender,
        )
    }

    pub fn dst(&self) -> &str {
        &self.dst
    }

    /// Whether some connection to the destination is up.
    pub fn is_connected(&self) -> bool {
        self.metrics.total_connections.load(Ordering::Relaxed) > 0
    }

    /// Builds the tonic endpoint. We let tonic own the TCP/TLS/HTTP-2 transport
    /// (connection, reconnect-on-new-RPC, keepalive); gRPC's HTTP/2 keepalive
    /// replaces the TCP_USER_TIMEOUT the patrolli client had to set by hand. TLS,
    /// including mTLS, comes from our shared `tls` module so the OTAP forwarder
    /// and the rest of the app share one TLS surface.
    fn endpoint(&self) -> Result<Endpoint> {
        // Scheme drives whether tonic applies TLS: `https` → configured TLS,
        // `http` → plaintext h2c.
        let scheme = if self.config.tls.is_some() {
            "https"
        } else {
            "http"
        };
        let mut endpoint = Endpoint::from_shared(format!("{scheme}://{}", self.dst))?
            .tcp_nodelay(true)
            .tcp_keepalive(Some(Duration::from_secs(60)))
            .tcp_keepalive_interval(Some(Duration::from_secs(20)))
            .tcp_keepalive_retries(Some(4))
            .http2_keep_alive_interval(HTTP2_KEEPALIVE_INTERVAL)
            .keep_alive_while_idle(true);
        if !self.config.write_timeout.is_zero() {
            endpoint = endpoint.keep_alive_timeout(self.config.write_timeout);
        }
        if let Some(tls_conf) = &self.config.tls {
            endpoint = match tls::build_tonic_client_tls(tls_conf)? {
                tls::TonicClientTls::Verified(config) => endpoint.tls_config(*config)?,
                tls::TonicClientTls::Insecure(config, verifier) => {
                    endpoint.tls_config_with_verifier(*config, verifier)?
                }
            };
        }
        Ok(endpoint)
    }

    /// Returns `Ok` only once the client is shutting down: [`Self::close`] was
    /// called or our sender was dropped. Errors are logged once, by the retry
    /// loop in [`Self::run`].
    async fn connect(&self, failed_count: &mut u32) -> Result<()> {
        let endpoint = match self.endpoint() {
            Ok(e) => e,
            Err(e) => return Err(e.context("invalid destination endpoint config")),
        };
        let connected = tokio::select! {
            _ = self.token.cancelled() => return Ok(()),
            connected = endpoint.connect() => connected,
        };
        let channel = match connected {
            Ok(c) => c,
            Err(e) => return Err(anyhow::Error::new(e).context("failed to connect")),
        };
        match self.config.sink_mode {
            OtelSinkMode::Otap => self.stream_otap(channel, failed_count).await,
            OtelSinkMode::Otlp => self.stream_otlp(channel, failed_count).await,
        }
    }

    /// Opens the OTAP Arrow streams over the connected channel and pumps jobs
    /// until the peer drops, an error occurs, our sender is closed, or the
    /// client is closed — in which case it first drains what tonic still holds.
    /// A job is settled when its batch's `BatchStatus` comes back.
    async fn stream_otap(&self, channel: Channel, failed_count: &mut u32) -> Result<()> {
        let Streams {
            mut events,
            logs_tx,
            traces_tx,
            metrics_tx,
            tasks,
        } = setup_otap_clients(channel);

        self.metrics
            .total_connections
            .fetch_add(1, Ordering::Relaxed);
        *failed_count = 0;
        info!(protocol = "otap", "destination connected");

        // One producer per stream so dictionaries/schema are sent once on the wire
        // and referenced by later batches. Each numbers its batches from 0, so
        // an answer is matched on the signal and the batch id.
        let producer = || {
            Producer::new_with_options(ProducerOptions {
                ipc_compression: ipc_compression(self.config.compression),
            })
        };
        let mut logs_producer = producer();
        let mut traces_producer = producer();
        let mut metrics_producer = producer();
        // Dropped with the connection, which tells every sender still waiting
        // to retry.
        let mut owed: HashMap<(Signal, i64), Owed> = HashMap::default();

        let result = loop {
            let n = Instant::now();
            tokio::select! {
                _ = self.token.cancelled() => {
                    self.metrics
                        .receive_wait_time
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    break Ok(());
                }

                event = events.recv() => {
                    self.metrics
                        .receive_wait_time
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    match event {
                        Some(Event::Status(signal, status)) => {
                            // An answer for a batch nobody waits on any more
                            // is a protocol slip of the peer's; nothing to do.
                            if let Some(o) = owed.remove(&(signal, status.batch_id)) {
                                let answer = status_answer(&status, o.items);
                                o.settle(&self.metrics, answer);
                            }
                        }
                        Some(Event::Closed(Ok(()))) => break Err(anyhow!("destination closed the stream")),
                        Some(Event::Closed(Err(e))) => break Err(e),
                        None => break Err(anyhow!("otap stream tasks ended unexpectedly")),
                    }
                }

                job = self.receiver.recv_async() => {
                    self.metrics
                        .receive_wait_time
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    let Ok(Job { payload, answer }) = job else {
                        // Our sender was dropped → graceful shutdown.
                        break Ok(());
                    };
                    let n = Instant::now();
                    let signal = Signal::of(&payload.data);
                    // produce_bar rewrites the batch (transport-optimized encoding);
                    // the job's payload is ours to change.
                    let mut data = payload.data;
                    let items = data.num_items() as u64;
                    self.metrics
                        .total_records
                        .fetch_add(items, Ordering::Relaxed);
                    let produced = match data.encode_transport_optimized() {
                        Ok(()) => match signal {
                            Signal::Logs => logs_producer.produce_bar(&mut data),
                            Signal::Traces => traces_producer.produce_bar(&mut data),
                            Signal::Metrics => metrics_producer.produce_bar(&mut data),
                        }
                        .map_err(|e| anyhow!("failed to produce arrow batch: {e}")),
                        Err(e) => Err(anyhow!("failed to encode transport optimized payload: {e}")),
                    };
                    self.metrics
                        .engine_time_spent
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    let bar = match produced {
                        Ok(bar) => bar,
                        Err(error) => {
                            Owed { answer, items, bytes: 0 }
                                .settle(&self.metrics, Answer::Rejected { items, error });
                            self.metrics
                                .total_time_spent
                                .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                            continue;
                        }
                    };
                    let bytes = bar
                        .arrow_payloads
                        .iter()
                        .map(|pl| pl.record.len())
                        .sum::<usize>() as u64;
                    self.metrics
                        .total_bytes
                        .fetch_add(bytes, Ordering::Relaxed);
                    let batch_id = bar.batch_id;

                    let n = Instant::now();
                    let sent = match signal {
                        Signal::Logs => logs_tx.send(bar).await.is_ok(),
                        Signal::Traces => traces_tx.send(bar).await.is_ok(),
                        Signal::Metrics => metrics_tx.send(bar).await.is_ok(),
                    };
                    self.metrics
                        .send_wait_time
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    if !sent {
                        Owed { answer, items, bytes }.settle(
                            &self.metrics,
                            Answer::Retry {
                                error: anyhow!("otap request stream closed"),
                                after: None,
                            },
                        );
                        break Err(anyhow!("otap request stream closed"));
                    }
                    owed.insert((signal, batch_id), Owed { answer, items, bytes });
                }
            }
            self.metrics
                .total_time_spent
                .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
        };

        self.metrics
            .total_connections
            .fetch_sub(1, Ordering::Relaxed);
        drop(owed);
        // Drop the request senders so tonic flushes and closes the outbound
        // streams, and the event receiver so no stream task waits on it.
        drop(logs_tx);
        drop(traces_tx);
        drop(metrics_tx);
        drop(events);
        if self.token.is_cancelled() {
            drain(tasks).await;
        }
        result
    }

    /// Encodes jobs into OTLP export requests for the per-signal tasks, which
    /// send them one at a time and settle each job with its response.
    async fn stream_otlp(&self, channel: Channel, failed_count: &mut u32) -> Result<()> {
        let Streams {
            mut events,
            logs_tx,
            traces_tx,
            metrics_tx,
            tasks,
        } = setup_otlp_clients(channel, self.metrics.clone());

        self.metrics
            .total_connections
            .fetch_add(1, Ordering::Relaxed);
        *failed_count = 0;
        info!(protocol = "otlp", "destination connected");

        // Reuse buffers
        let mut logs_encoder = LogsProtoBytesEncoder::new();
        let mut traces_encoder = TracesProtoBytesEncoder::new();
        let mut metrics_encoder = MetricsProtoBytesEncoder::new();
        let mut buffer = ProtoBuffer::default();

        let result = loop {
            let n = Instant::now();
            tokio::select! {
                _ = self.token.cancelled() => {
                    self.metrics
                        .receive_wait_time
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    break Ok(());
                }

                event = events.recv() => {
                    self.metrics
                        .receive_wait_time
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    match event {
                        // Only OTAP streams answer with a status.
                        Some(Event::Status(..)) => {}
                        Some(Event::Closed(Ok(()))) => break Err(anyhow!("destination closed the stream")),
                        Some(Event::Closed(Err(e))) => break Err(e),
                        None => break Err(anyhow!("otlp stream tasks ended unexpectedly")),
                    }
                }

                job = self.receiver.recv_async() => {
                    self.metrics
                        .receive_wait_time
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    let Ok(Job { payload, answer }) = job else {
                        // Our sender was dropped → graceful shutdown.
                        break Ok(());
                    };
                    let n = Instant::now();
                    let signal = Signal::of(&payload.data);
                    let mut data = payload.data;
                    let items = data.num_items() as u64;
                    self.metrics
                        .total_records
                        .fetch_add(items, Ordering::Relaxed);

                    buffer.clear();
                    let encoded = match signal {
                        Signal::Logs => logs_encoder.encode(&mut data, &mut buffer),
                        Signal::Traces => traces_encoder.encode(&mut data, &mut buffer),
                        Signal::Metrics => metrics_encoder.encode(&mut data, &mut buffer),
                    }
                    .map_err(|e| anyhow!("failed to encode otlp request: {e}"));
                    let bytes = buffer.len() as u64;
                    let owed = Owed { answer, items, bytes };
                    let request = encoded.and_then(|()| {
                        let decoded = match signal {
                            Signal::Logs => ExportLogsServiceRequest::decode(buffer.as_ref())
                                .map(|r| Otlp::Logs(Request::new(r))),
                            Signal::Traces => ExportTraceServiceRequest::decode(buffer.as_ref())
                                .map(|r| Otlp::Traces(Request::new(r))),
                            Signal::Metrics => ExportMetricsServiceRequest::decode(buffer.as_ref())
                                .map(|r| Otlp::Metrics(Request::new(r))),
                        };
                        decoded.map_err(|e| anyhow!("failed to decode otlp request: {e}"))
                    });
                    self.metrics
                        .engine_time_spent
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    let request = match request {
                        Ok(request) => request,
                        Err(error) => {
                            owed.settle(&self.metrics, Answer::Rejected { items, error });
                            self.metrics
                                .total_time_spent
                                .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                            continue;
                        }
                    };
                    self.metrics
                        .total_bytes
                        .fetch_add(bytes, Ordering::Relaxed);

                    let n = Instant::now();
                    // A failed send hands the job back, so it can be settled.
                    let unsent = match request {
                        Otlp::Logs(r) => logs_tx.send((r, owed)).await.err().map(|e| e.0.1),
                        Otlp::Traces(r) => traces_tx.send((r, owed)).await.err().map(|e| e.0.1),
                        Otlp::Metrics(r) => metrics_tx.send((r, owed)).await.err().map(|e| e.0.1),
                    };
                    self.metrics
                        .send_wait_time
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    if let Some(owed) = unsent {
                        owed.settle(
                            &self.metrics,
                            Answer::Retry {
                                error: anyhow!("otlp request stream closed"),
                                after: None,
                            },
                        );
                        break Err(anyhow!("otlp request stream closed"));
                    }
                }
            }
            self.metrics
                .total_time_spent
                .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
        };

        self.metrics
            .total_connections
            .fetch_sub(1, Ordering::Relaxed);
        // Drop the request senders so the stream tasks send what they hold
        // and stop, and the event receiver so no stream task waits on it.
        drop(logs_tx);
        drop(traces_tx);
        drop(metrics_tx);
        drop(events);
        if self.token.is_cancelled() {
            drain(tasks).await;
        }
        result
    }

    #[tracing::instrument(skip_all, fields(dst = %self.dst))]
    pub fn run(self: &Arc<Self>, mut count: usize) {
        // Each connection task carries this span, so its events and the stream
        // tasks it spawns are attributed to the destination.
        if count == 0 {
            count = 1;
        }
        let mut units = self.units.lock();
        for _ in 0..count {
            let this = self.clone();
            let task = tokio::spawn(
                async move {
                    let mut failed_count: u32 = 0;
                    loop {
                        let err = match this.connect(&mut failed_count).await {
                            Ok(()) => break,
                            Err(e) => e,
                        };
                        // Nobody left to send for, e.g. a sink dropped without close.
                        if this.token.is_cancelled() || this.receiver.is_disconnected() {
                            break;
                        }
                        let mut seconds_to_wait = 2u64.saturating_pow(failed_count);
                        if seconds_to_wait > 30 {
                            seconds_to_wait = 30;
                        }
                        warn!(
                            error = %format!("{err:#}"),
                            retry_in_secs = seconds_to_wait,
                            "destination connection failed, retrying"
                        );
                        tokio::select! {
                            _ = this.token.cancelled() => break,
                            _ = sleep(Duration::from_secs(seconds_to_wait)) => {}
                        }
                        failed_count = failed_count.saturating_add(1);
                    }
                }
                .instrument(Span::current()),
            );
            units.push(task);
        }
    }

    /// Stops every connection task. A connected one first flushes what it
    /// already queued to tonic, for up to [`DRAIN_TIMEOUT`]. Call it once
    /// nothing sends into the client any more.
    #[tracing::instrument(skip_all, fields(dst = %self.dst))]
    pub async fn close(&self) {
        self.token.cancel();
        let units = {
            let mut units = self.units.lock();
            std::mem::take(&mut *units)
        };
        for unit in units {
            if let Err(e) = unit.await {
                error!(error = %e, "destination connection task panicked");
            }
        }
    }
}

/// A non-ok status answers for the whole batch. Its codes are gRPC's, but it
/// carries no `RetryInfo`, so RESOURCE_EXHAUSTED is not retried.
fn status_answer(status: &BatchStatus, items: u64) -> Answer {
    if status.status_code == StatusCode::Ok as i32 {
        return Answer::Delivered;
    }
    let code = StatusCode::try_from(status.status_code).map_or_else(
        |_| status.status_code.to_string(),
        |c| c.as_str_name().to_string(),
    );
    let error = anyhow!(
        "destination answered the batch with {code}: {}",
        status.status_message
    );
    if is_retryable(Code::from(status.status_code), false) {
        Answer::Retry { error, after: None }
    } else {
        Answer::Rejected { items, error }
    }
}

/// Whether a destination that turned a batch away with `code` may take it if
/// sent again: the codes the OTLP spec lists as retryable. RESOURCE_EXHAUSTED
/// only when the server attached `RetryInfo` (`recovers`), its sign of an
/// overload it gets over; without it, the batch may be too large to ever fit,
/// and retries are unbounded by default.
fn is_retryable(code: Code, recovers: bool) -> bool {
    match code {
        Code::Cancelled
        | Code::DeadlineExceeded
        | Code::Aborted
        | Code::OutOfRange
        | Code::Unavailable
        | Code::DataLoss => true,
        Code::ResourceExhausted => recovers,
        _ => false,
    }
}

/// The parts of `google.rpc.Status` that carry `google.rpc.RetryInfo`, the
/// wait a server asks for.
mod rpc {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Status {
        #[prost(message, repeated, tag = "3")]
        pub details: Vec<Any>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Any {
        #[prost(string, tag = "1")]
        pub type_url: String,
        #[prost(bytes = "vec", tag = "2")]
        pub value: Vec<u8>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct RetryInfo {
        #[prost(message, optional, tag = "1")]
        pub retry_delay: Option<Duration>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Duration {
        #[prost(int64, tag = "1")]
        pub seconds: i64,
        #[prost(int32, tag = "2")]
        pub nanos: i32,
    }

    pub const RETRY_INFO: &str = "type.googleapis.com/google.rpc.RetryInfo";
}

/// The wait in the status's `RetryInfo` (zero when it names none), or `None`
/// without one.
fn retry_info(status: &Status) -> Option<Duration> {
    let details = rpc::Status::decode(status.details()).ok()?;
    let any = details
        .details
        .iter()
        .find(|any| any.type_url == rpc::RETRY_INFO)?;
    let delay = rpc::RetryInfo::decode(any.value.as_slice())
        .ok()?
        .retry_delay
        .unwrap_or_default();
    Some(Duration::new(
        delay.seconds.max(0) as u64,
        delay.nanos.clamp(0, 999_999_999) as u32,
    ))
}

/// Waits for the per-signal stream tasks of a connection whose request senders
/// were dropped, so what they still hold reaches the destination.
async fn drain(tasks: Vec<JoinHandle<()>>) {
    let joined = tokio::time::timeout(DRAIN_TIMEOUT, async {
        for task in tasks {
            if let Err(e) = task.await {
                error!(error = %e, "destination stream task panicked");
            }
        }
    })
    .await;
    if joined.is_err() {
        warn!(timeout = ?DRAIN_TIMEOUT, "destination drain timed out");
    }
}

/// One connection's per-signal request senders, the channel its stream tasks
/// report answers and the end of the connection on, and those tasks.
///
/// `events` is unbounded so a stream task never waits on the main loop: that
/// loop can be blocked handing tonic a request, which the peer may only take
/// once we have read its answers. It holds at most one answer per request in
/// flight.
struct Streams<L, T, M> {
    events: mpsc::UnboundedReceiver<Event>,
    logs_tx: mpsc::Sender<L>,
    traces_tx: mpsc::Sender<T>,
    metrics_tx: mpsc::Sender<M>,
    tasks: Vec<JoinHandle<()>>,
}

fn setup_otap_clients(
    channel: Channel,
) -> Streams<BatchArrowRecords, BatchArrowRecords, BatchArrowRecords> {
    let (events_tx, events) = mpsc::unbounded_channel();

    let mut logs_client = ArrowLogsServiceClient::new(channel.clone());
    let (logs_tx, logs_rx) = mpsc::channel::<BatchArrowRecords>(STREAM_BUFFER);
    let logs_stream = spawn_otap_stream(
        Signal::Logs,
        async move { logs_client.arrow_logs(ReceiverStream::new(logs_rx)).await },
        events_tx.clone(),
    );

    let mut traces_client = ArrowTracesServiceClient::new(channel.clone());
    let (traces_tx, traces_rx) = mpsc::channel::<BatchArrowRecords>(STREAM_BUFFER);
    let traces_stream = spawn_otap_stream(
        Signal::Traces,
        async move {
            traces_client
                .arrow_traces(ReceiverStream::new(traces_rx))
                .await
        },
        events_tx.clone(),
    );

    let mut metrics_client = ArrowMetricsServiceClient::new(channel);
    let (metrics_tx, metrics_rx) = mpsc::channel::<BatchArrowRecords>(STREAM_BUFFER);
    let metrics_stream = spawn_otap_stream(
        Signal::Metrics,
        async move {
            metrics_client
                .arrow_metrics(ReceiverStream::new(metrics_rx))
                .await
        },
        events_tx,
    );

    Streams {
        events,
        logs_tx,
        traces_tx,
        metrics_tx,
        tasks: vec![logs_stream, traces_stream, metrics_stream],
    }
}

/// Opens one signal's Arrow stream and forwards every `BatchStatus` on it.
fn spawn_otap_stream<F>(
    signal: Signal,
    open: F,
    events: mpsc::UnboundedSender<Event>,
) -> JoinHandle<()>
where
    F: Future<Output = Result<Response<Streaming<BatchStatus>>, Status>> + Send + 'static,
{
    tokio::spawn(
        async move {
            let end: Result<()> = async {
                let mut responses = open.await?.into_inner();
                trace!(signal = signal.as_str(), "arrow stream opened");
                // Read to the end of the stream even once the main loop has
                // quit, so a closing connection still delivers what it queued.
                while let Some(status) = responses.message().await? {
                    let _ = events.send(Event::Status(signal, status));
                }
                Ok(())
            }
            .await;
            let _ = events.send(Event::Closed(end));
        }
        .in_current_span(),
    )
}

/// An OTLP export request, by signal.
enum Otlp {
    Logs(Request<ExportLogsServiceRequest>),
    Traces(Request<ExportTraceServiceRequest>),
    Metrics(Request<ExportMetricsServiceRequest>),
}

/// What an OTLP export's `partial_success` says was refused: the count and
/// the destination's message.
type Refused = Option<(i64, String)>;

type OtlpStreams = Streams<
    (Request<ExportLogsServiceRequest>, Owed),
    (Request<ExportTraceServiceRequest>, Owed),
    (Request<ExportMetricsServiceRequest>, Owed),
>;

fn setup_otlp_clients(channel: Channel, metrics: Arc<NodeMetrics>) -> OtlpStreams {
    let (events_tx, events) = mpsc::unbounded_channel();

    let logs_client = LogsServiceClient::new(channel.clone());
    let (logs_tx, logs_rx) = mpsc::channel(STREAM_BUFFER);
    let logs_stream = spawn_otlp_stream(logs_rx, events_tx.clone(), metrics.clone(), move |req| {
        let mut client = logs_client.clone();
        async move {
            let resp = client.export(req).await?.into_inner();
            Ok(resp
                .partial_success
                .map(|p| (p.rejected_log_records, p.error_message)))
        }
    });

    let traces_client = TraceServiceClient::new(channel.clone());
    let (traces_tx, traces_rx) = mpsc::channel(STREAM_BUFFER);
    let traces_stream =
        spawn_otlp_stream(traces_rx, events_tx.clone(), metrics.clone(), move |req| {
            let mut client = traces_client.clone();
            async move {
                let resp = client.export(req).await?.into_inner();
                Ok(resp
                    .partial_success
                    .map(|p| (p.rejected_spans, p.error_message)))
            }
        });

    let metrics_client = MetricsServiceClient::new(channel);
    let (metrics_tx, metrics_rx) = mpsc::channel(STREAM_BUFFER);
    let metrics_stream = spawn_otlp_stream(metrics_rx, events_tx, metrics, move |req| {
        let mut client = metrics_client.clone();
        async move {
            let resp = client.export(req).await?.into_inner();
            Ok(resp
                .partial_success
                .map(|p| (p.rejected_data_points, p.error_message)))
        }
    });

    Streams {
        events,
        logs_tx,
        traces_tx,
        metrics_tx,
        tasks: vec![logs_stream, traces_stream, metrics_stream],
    }
}

/// Sends one signal's requests one at a time and settles each with its
/// response. `Unavailable` (the transport failed) is retried and ends the
/// connection, so it reconnects with backoff; any other error status
/// refuses the request.
fn spawn_otlp_stream<R, F, Fut>(
    mut requests: mpsc::Receiver<(R, Owed)>,
    events: mpsc::UnboundedSender<Event>,
    metrics: Arc<NodeMetrics>,
    mut export: F,
) -> JoinHandle<()>
where
    R: Send + 'static,
    F: FnMut(R) -> Fut + Send + 'static,
    Fut: Future<Output = Result<Refused, Status>> + Send,
{
    tokio::spawn(
        async move {
            let end = loop {
                // `None`: the main loop dropped its sender, graceful shutdown.
                let Some((req, owed)) = requests.recv().await else {
                    break Ok(());
                };
                let items = owed.items;
                match export(req).await {
                    Ok(Some((refused, message))) if refused > 0 => {
                        let refused = (refused as u64).min(items);
                        owed.settle(
                            &metrics,
                            Answer::Rejected {
                                items: refused,
                                error: anyhow!("destination refused {refused} items: {message}"),
                            },
                        );
                    }
                    Ok(_) => owed.settle(&metrics, Answer::Delivered),
                    // The connection may be what is down: start over on a new one.
                    Err(status) if status.code() == Code::Unavailable => {
                        let answer = Answer::Retry {
                            error: status.clone().into(),
                            after: retry_info(&status).filter(|d| !d.is_zero()),
                        };
                        owed.settle(&metrics, answer);
                        break Err(anyhow::Error::new(status).context("otlp export failed"));
                    }
                    Err(status) => {
                        let after = retry_info(&status);
                        let answer = if is_retryable(status.code(), after.is_some()) {
                            Answer::Retry {
                                error: status.into(),
                                after: after.filter(|d| !d.is_zero()),
                            }
                        } else {
                            Answer::Rejected {
                                items,
                                error: status.into(),
                            }
                        };
                        owed.settle(&metrics, answer);
                    }
                }
            };
            let _ = events.send(Event::Closed(end));
        }
        .in_current_span(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_retry_info(code: Code, delay: Option<rpc::Duration>) -> Status {
        let info = rpc::RetryInfo { retry_delay: delay };
        let details = rpc::Status {
            details: vec![rpc::Any {
                type_url: rpc::RETRY_INFO.to_string(),
                value: info.encode_to_vec(),
            }],
        };
        Status::with_details(code, "slow down", details.encode_to_vec().into())
    }

    #[test]
    fn resource_exhausted_is_retried_only_with_retry_info() {
        let plain = Status::resource_exhausted("too big");
        assert_eq!(retry_info(&plain), None);
        assert!(!is_retryable(plain.code(), false));

        let asked = with_retry_info(
            Code::ResourceExhausted,
            Some(rpc::Duration {
                seconds: 2,
                nanos: 500_000_000,
            }),
        );
        assert_eq!(retry_info(&asked), Some(Duration::from_millis(2500)));
        assert!(is_retryable(asked.code(), true));
        let no_delay = with_retry_info(Code::ResourceExhausted, None);
        assert_eq!(retry_info(&no_delay), Some(Duration::ZERO));
    }

    #[test]
    fn the_otlp_retryable_codes_are_retried_and_the_rest_are_not() {
        for code in [
            Code::Cancelled,
            Code::DeadlineExceeded,
            Code::Aborted,
            Code::OutOfRange,
            Code::Unavailable,
            Code::DataLoss,
        ] {
            assert!(is_retryable(code, false), "{code:?}");
        }
        for code in [
            Code::InvalidArgument,
            Code::PermissionDenied,
            Code::Unauthenticated,
            Code::Internal,
            Code::Unimplemented,
            Code::ResourceExhausted,
        ] {
            assert!(!is_retryable(code, false), "{code:?}");
        }
    }

    #[test]
    fn an_otap_status_maps_onto_the_same_codes() {
        let status = |code: StatusCode| BatchStatus {
            batch_id: 1,
            status_code: code as i32,
            status_message: String::new(),
        };
        assert!(matches!(
            status_answer(&status(StatusCode::Ok), 3),
            Answer::Delivered
        ));
        for code in [
            StatusCode::Unavailable,
            StatusCode::DeadlineExceeded,
            StatusCode::Aborted,
            StatusCode::Canceled,
        ] {
            assert!(
                matches!(
                    status_answer(&status(code), 3),
                    Answer::Retry { after: None, .. }
                ),
                "{code:?}"
            );
        }
        for code in [
            StatusCode::ResourceExhausted,
            StatusCode::InvalidArgument,
            StatusCode::Internal,
        ] {
            assert!(
                matches!(
                    status_answer(&status(code), 3),
                    Answer::Rejected { items: 3, .. }
                ),
                "{code:?}"
            );
        }
    }

    /// Refuses connections straight away, so the tasks sit in their backoff.
    const UNREACHABLE: &str = "127.0.0.1:1";

    fn client() -> (Arc<OtelClient>, Sender<Job>) {
        let config = Arc::new(OtelClientConfig {
            compression: OtapCompression::None,
            tls: None,
            write_timeout: Duration::ZERO,
            sink_mode: OtelSinkMode::Otap,
        });
        OtelClient::new(config, UNREACHABLE.into(), Arc::default())
    }

    #[tokio::test]
    async fn close_stops_connection_tasks_in_backoff() {
        let (client, _sender) = client();
        client.run(2);
        sleep(Duration::from_millis(200)).await;
        tokio::time::timeout(Duration::from_millis(500), client.close())
            .await
            .expect("close waited out the reconnect backoff");
        assert!(client.units.lock().is_empty());
    }

    #[tokio::test]
    async fn dropped_sender_stops_connection_tasks() {
        let (client, sender) = client();
        client.run(2);
        drop(sender);
        tokio::time::timeout(Duration::from_secs(5), async {
            while !client.units.lock().iter().all(|unit| unit.is_finished()) {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("connection tasks kept retrying for a dropped sink");
    }
}
