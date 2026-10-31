// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::common::internal_metrics::{InternalMetrics, visit};
use crate::config::global::Prometheus;
use anyhow::{Context, Result};
use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum_server::tls_rustls::RustlsConfig;
use bytes::{Bytes, BytesMut};
use prometheus_client::collector::Collector;
use prometheus_client::encoding::text::encode;
use prometheus_client::encoding::{DescriptorEncoder, EncodeMetric};
use prometheus_client::metrics::MetricType;
use prometheus_client::metrics::counter::ConstCounter;
use prometheus_client::metrics::gauge::ConstGauge;
use prometheus_client::registry::{Registry, Unit};
use std::borrow::Cow;
use std::fmt;
use std::future::Future;
use std::io;
use std::net::TcpListener;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

const OPENMETRICS_CONTENT_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";
const SCRAPE_CHUNK_SIZE: usize = 16 * 1024;
const SCRAPE_BUFFERED_CHUNKS: usize = 2;
const SECONDS: Unit = Unit::Seconds;
const BYTES: Unit = Unit::Bytes;

type ServeFuture = Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>;

pub struct Server {
    task: Option<JoinHandle<Result<()>>>,
}

impl Server {
    pub async fn wait(mut self) -> Result<()> {
        self.task
            .take()
            .expect("Prometheus server task is missing")
            .await
            .context("Prometheus server task failed")?
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

#[derive(Clone)]
struct AppState {
    registry: Arc<Registry>,
    expected_authorization: Option<Arc<[u8]>>,
}

pub fn spawn(
    config: &Prometheus,
    metrics: Arc<InternalMetrics>,
    include_forwarder: bool,
    shutdown: CancellationToken,
) -> Result<Server> {
    let tls_config = config
        .tls
        .as_ref()
        .map(|tls| {
            crate::common::tls::build_rustls_server_config(tls)
                .context("failed to build Prometheus server TLS configuration")
                .map(|config| RustlsConfig::from_config(Arc::new(config)))
        })
        .transpose()?;

    let listener = TcpListener::bind(config.address)
        .with_context(|| format!("failed to bind Prometheus endpoint to {}", config.address))?;
    listener
        .set_nonblocking(true)
        .context("failed to configure Prometheus listener as non-blocking")?;
    let address = listener
        .local_addr()
        .context("failed to get Prometheus listener address")?;

    let app = router(metrics, include_forwarder, config.bearer_token.as_deref());
    let handle = axum_server::Handle::new();
    let serve: ServeFuture = match tls_config {
        Some(tls_config) => Box::pin(
            axum_server::from_tcp_rustls(listener, tls_config)
                .context("failed to initialize Prometheus TLS listener")?
                .handle(handle.clone())
                .serve(app.into_make_service()),
        ),
        None => Box::pin(
            axum_server::from_tcp(listener)
                .context("failed to initialize Prometheus listener")?
                .handle(handle.clone())
                .serve(app.into_make_service()),
        ),
    };

    if !address.ip().is_loopback() {
        if config.tls.is_none() && config.bearer_token.is_none() {
            warn!(
                %address,
                "prometheus endpoint is exposed on a non-loopback address without authentication or TLS"
            );
        } else if config.tls.is_none() && config.bearer_token.is_some() {
            warn!(
                %address,
                "prometheus bearer token will be transmitted over plaintext HTTP"
            );
        }
    }

    info!(
        %address,
        tls = config.tls.is_some(),
        mtls = config
            .tls
            .as_ref()
            .is_some_and(|tls| tls.ca_cert_path.is_some()),
        authenticated = config.bearer_token.is_some(),
        "prometheus endpoint listening"
    );

    let task = tokio::spawn(async move {
        let mut serve = serve;
        let result = tokio::select! {
            result = &mut serve => result,
            _ = shutdown.cancelled() => {
                handle.graceful_shutdown(Some(Duration::from_secs(10)));
                serve.await
            }
        }
        .context("Prometheus server failed");

        if !shutdown.is_cancelled() {
            // `Ok` here means the server returned without being asked to, which is just as wrong
            // as an error — one event either way, with the cause when there is one.
            match &result {
                Ok(()) => error!("prometheus server stopped unexpectedly"),
                Err(error) => {
                    error!(error = %format!("{error:#}"), "prometheus server stopped unexpectedly")
                }
            }
            shutdown.cancel();
            return result
                .and_then(|_| Err(anyhow::anyhow!("Prometheus server stopped unexpectedly")));
        }

        result
    });

    Ok(Server { task: Some(task) })
}

fn router(
    metrics: Arc<InternalMetrics>,
    include_forwarder: bool,
    bearer_token: Option<&str>,
) -> Router {
    let mut registry = Registry::with_prefix("patrolli");
    registry.register_collector(Box::new(InternalMetricsCollector {
        metrics,
        include_forwarder,
    }));

    let expected_authorization =
        bearer_token.map(|token| Arc::<[u8]>::from(format!("Bearer {token}").into_bytes()));
    let state = AppState {
        registry: Arc::new(registry),
        expected_authorization,
    };

    // Axum automatically serves HEAD through the GET route and strips the body.
    Router::new()
        .route("/metrics", get(scrape))
        .with_state(state)
}

async fn scrape(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !is_authorized(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            [(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"))],
        )
            .into_response();
    }

    (
        [(
            CONTENT_TYPE,
            HeaderValue::from_static(OPENMETRICS_CONTENT_TYPE),
        )],
        streaming_body(state.registry),
    )
        .into_response()
}

fn streaming_body(registry: Arc<Registry>) -> Body {
    let (sender, receiver) = mpsc::channel::<Result<Bytes, io::Error>>(SCRAPE_BUFFERED_CHUNKS);

    tokio::task::spawn_blocking(move || {
        let mut writer = ChunkedBodyWriter::new(sender, SCRAPE_CHUNK_SIZE);
        match encode(&mut writer, &registry) {
            Ok(()) => {
                // A failed final flush means Hyper dropped the response body.
                let _ = writer.finish();
            }
            Err(_) if writer.receiver_closed() => {
                // Client disconnects are expected and should stop encoding quietly.
            }
            Err(error) => {
                error!(error = %error, "failed to encode prometheus metrics");
                writer.send_error();
            }
        }
    });

    Body::from_stream(ReceiverStream::new(receiver))
}

struct ChunkedBodyWriter {
    sender: mpsc::Sender<Result<Bytes, io::Error>>,
    buffer: BytesMut,
    chunk_size: usize,
    receiver_closed: bool,
}

impl ChunkedBodyWriter {
    fn new(sender: mpsc::Sender<Result<Bytes, io::Error>>, chunk_size: usize) -> Self {
        assert!(
            chunk_size > 0,
            "Prometheus scrape chunk size must be non-zero"
        );
        Self {
            sender,
            buffer: BytesMut::with_capacity(chunk_size),
            chunk_size,
            receiver_closed: false,
        }
    }

    fn send_chunk(&mut self, chunk: Bytes) -> fmt::Result {
        if self.sender.blocking_send(Ok(chunk)).is_err() {
            self.receiver_closed = true;
            return Err(fmt::Error);
        }
        Ok(())
    }

    fn flush(&mut self) -> fmt::Result {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let chunk = self.buffer.split().freeze();
        self.send_chunk(chunk)
    }

    fn finish(mut self) -> fmt::Result {
        self.flush()
    }

    fn receiver_closed(&self) -> bool {
        self.receiver_closed
    }

    fn send_error(mut self) {
        self.buffer.clear();
        let _ = self
            .sender
            .blocking_send(Err(io::Error::other("failed to encode Prometheus metrics")));
    }
}

impl fmt::Write for ChunkedBodyWriter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let mut remaining = value.as_bytes();
        while !remaining.is_empty() {
            let available = self.chunk_size - self.buffer.len();
            let amount = available.min(remaining.len());
            self.buffer.extend_from_slice(&remaining[..amount]);
            remaining = &remaining[amount..];

            if self.buffer.len() == self.chunk_size {
                let chunk = self.buffer.split_to(self.chunk_size).freeze();
                self.send_chunk(chunk)?;
            }
        }
        Ok(())
    }
}

fn is_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(expected) = &state.expected_authorization else {
        return true;
    };
    let Some(actual) = headers.get(AUTHORIZATION) else {
        return false;
    };

    actual.as_bytes().ct_eq(expected.as_ref()).into()
}

struct InternalMetricsCollector {
    metrics: Arc<InternalMetrics>,
    include_forwarder: bool,
}

impl fmt::Debug for InternalMetricsCollector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InternalMetricsCollector")
            .field("include_forwarder", &self.include_forwarder)
            .finish_non_exhaustive()
    }
}

impl Collector for InternalMetricsCollector {
    fn encode(&self, mut encoder: DescriptorEncoder) -> fmt::Result {
        visit::walk(
            &self.metrics,
            self.include_forwarder,
            &mut PromVisitor {
                encoder: &mut encoder,
            },
        )
    }
}

/// Renders the shared internal-metrics catalogue as OpenMetrics text.
///
/// The catalogue names families with dots (`sink.time_spent`); Prometheus wants
/// underscores. Everything else about the spelling is added downstream: the
/// `patrolli` prefix by [`Registry::with_prefix`], and the `_seconds`/`_bytes` and
/// `_total` suffixes by `prometheus-client` from the unit and the metric type.
struct PromVisitor<'a, 'b> {
    encoder: &'a mut DescriptorEncoder<'b>,
}

fn prom_unit(unit: visit::Unit) -> Option<&'static Unit> {
    match unit {
        visit::Unit::None => None,
        visit::Unit::Seconds => Some(&SECONDS),
        visit::Unit::Bytes => Some(&BYTES),
    }
}

fn prom_type(kind: visit::Kind) -> MetricType {
    match kind {
        visit::Kind::Counter => MetricType::Counter,
        visit::Kind::Gauge => MetricType::Gauge,
    }
}

impl visit::Visitor for PromVisitor<'_, '_> {
    type Error = fmt::Error;

    fn family(&mut self, family: visit::Family, samples: visit::Samples<'_>) -> fmt::Result {
        let name = family.name.replace('.', "_");
        let unit = prom_unit(family.unit);
        let kind = family.kind;
        let mut metric =
            self.encoder
                .encode_descriptor(&name, family.help, unit, prom_type(kind))?;

        match samples {
            // Scalars go out without a label set at all — an empty one would
            // render as `patrolli_monitor_items_total{}`, which is not the same
            // text as `patrolli_monitor_items_total`.
            visit::Samples::U64Scalar(value) => match kind {
                visit::Kind::Counter => ConstCounter::new(value).encode(metric),
                visit::Kind::Gauge => ConstGauge::new(value).encode(metric),
            },
            visit::Samples::F64Scalar(value) => match kind {
                visit::Kind::Counter => ConstCounter::new(value).encode(metric),
                visit::Kind::Gauge => ConstGauge::new(value).encode(metric),
            },
            visit::Samples::U64(samples) => {
                for (labels, value) in samples {
                    let label_set: &[(&'static str, Cow<'static, str>)] = labels;
                    let family_encoder = metric.encode_family(&label_set)?;
                    match kind {
                        visit::Kind::Counter => ConstCounter::new(*value).encode(family_encoder)?,
                        visit::Kind::Gauge => ConstGauge::new(*value).encode(family_encoder)?,
                    }
                }
                Ok(())
            }
            visit::Samples::F64(samples) => {
                for (labels, value) in samples {
                    let label_set: &[(&'static str, Cow<'static, str>)] = labels;
                    let family_encoder = metric.encode_family(&label_set)?;
                    match kind {
                        visit::Kind::Counter => ConstCounter::new(*value).encode(family_encoder)?,
                        visit::Kind::Gauge => ConstGauge::new(*value).encode(family_encoder)?,
                    }
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::internal_metrics::{
        Channel, ChannelKind, ChannelState, Forwarder, ForwarderNode, LookupTable,
        LookupTableProcessor, Sink, SkipCounts,
    };
    use axum::body::to_bytes;
    use std::time::Instant;
    use tokio_stream::StreamExt;
    use uuid::Uuid;

    fn encode_test_metrics(metrics: Arc<InternalMetrics>, include_forwarder: bool) -> String {
        let mut registry = Registry::with_prefix("patrolli");
        registry.register_collector(Box::new(InternalMetricsCollector {
            metrics,
            include_forwarder,
        }));
        let mut output = String::new();
        encode(&mut output, &registry).unwrap();
        output
    }

    #[test]
    fn encodes_counters_gauges_units_and_stable_labels() {
        let metrics = Arc::new(InternalMetrics::default());
        let sink_id = Uuid::now_v7();
        metrics.sink.store(Arc::new((
            Instant::now(),
            vec![Sink {
                id: sink_id,
                name: "primary".to_string(),
                total_workers: 3,
                total_time_spent: Duration::from_millis(1500),
                total_records: 12,
                ..Default::default()
            }],
        )));

        let mut channel = Channel {
            total_messages: 9,
            ..Default::default()
        };
        channel.channels.insert(
            "memory".to_string(),
            ChannelState {
                kind: ChannelKind::Memory,
                capacity: 100,
                length: 7,
            },
        );
        channel.channels.insert(
            "disk".to_string(),
            ChannelState {
                kind: ChannelKind::Disk,
                capacity: 4096,
                length: 512,
            },
        );
        metrics
            .sink_channel
            .store(Arc::new((Instant::now(), channel)));

        let output = encode_test_metrics(metrics, false);
        let parsed: std::result::Result<Vec<_>, _> = prometheus_scraper::parse_payload(
            output.as_bytes(),
            prometheus_scraper::Format::Text(prometheus_scraper::TextFormat::OpenMetrics),
        )
        .collect();
        assert!(!parsed.unwrap().is_empty());
        assert!(output.contains("# TYPE patrolli_channel_messages counter"));
        assert_eq!(
            output
                .matches("# TYPE patrolli_channel_messages counter")
                .count(),
            1
        );
        assert!(output.contains("# TYPE patrolli_channel_queue_length gauge"));
        assert!(output.contains("# TYPE patrolli_channel_disk_usage_bytes gauge"));
        assert!(output.contains("# TYPE patrolli_sink_workers gauge"));
        assert!(output.contains("# TYPE patrolli_sink_records counter"));
        assert!(output.contains("# TYPE patrolli_sink_time_spent_seconds counter"));
        assert!(output.contains("# TYPE patrolli_monitor_workers gauge"));
        assert!(output.contains("# TYPE patrolli_processor_workers gauge"));
        assert!(output.contains("# TYPE patrolli_internal_metrics_snapshot_age_seconds gauge"));
        assert!(output.contains("# UNIT patrolli_sink_time_spent_seconds seconds"));
        // Identity is carried as ordinary labels on every series rather than by a
        // separate Info metric, so that the OTLP metrics renderer — which has no
        // Info equivalent — can share the same catalogue.
        assert!(!output.contains("patrolli_sink_info"));
        assert!(!output.contains("patrolli_listener_info"));
        assert!(output.contains(&format!(
            "patrolli_sink_workers{{sink_id=\"{sink_id}\",sink_name=\"primary\"}} 3"
        )));
        assert!(output.contains(&format!(
            "patrolli_sink_records_total{{sink_id=\"{sink_id}\",sink_name=\"primary\"}} 12"
        )));
        assert!(output.contains(
            "patrolli_channel_queue_length{channel_group=\"sink\",channel=\"memory\"} 7"
        ));
        assert!(output.contains(
            "patrolli_channel_disk_usage_bytes{channel_group=\"sink\",channel=\"disk\"} 512"
        ));
        assert!(
            !output
                .contains("patrolli_channel_queue_length{channel_group=\"sink\",channel=\"disk\"}")
        );
        assert!(!output.contains("patrolli_forwarder_"));
        assert!(output.ends_with("# EOF\n"));
    }

    #[test]
    fn forwarder_connections_are_a_gauge() {
        let metrics = Arc::new(InternalMetrics::default());
        let peer_id = Uuid::now_v7();
        metrics.forwarder.store(Arc::new((
            Instant::now(),
            Forwarder {
                nodes: vec![ForwarderNode {
                    id: peer_id,
                    total_connections: 4,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )));

        let output = encode_test_metrics(metrics, true);
        assert!(output.contains("# TYPE patrolli_forwarder_peer_connections gauge"));
        assert!(output.contains(&format!(
            "patrolli_forwarder_peer_connections{{peer_id=\"{peer_id}\"}} 4"
        )));
    }

    #[test]
    fn bearer_authorization_is_exact() {
        let state = AppState {
            registry: Arc::new(Registry::default()),
            expected_authorization: Some(Arc::from(&b"Bearer secret"[..])),
        };
        let mut headers = HeaderMap::new();
        assert!(!is_authorized(&state, &headers));
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer wrong"));
        assert!(!is_authorized(&state, &headers));
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer secret"));
        assert!(is_authorized(&state, &headers));
    }

    #[test]
    fn chunked_body_writer_emits_bounded_chunks() {
        let (sender, mut receiver) = mpsc::channel(4);
        let mut writer = ChunkedBodyWriter::new(sender, 3);
        fmt::Write::write_str(&mut writer, "abé🙂zz").unwrap();
        writer.finish().unwrap();

        let mut chunks = Vec::new();
        while let Some(chunk) = receiver.blocking_recv() {
            chunks.push(chunk.unwrap());
        }

        assert_eq!(
            chunks.iter().map(Bytes::len).collect::<Vec<_>>(),
            vec![3, 3, 3, 1]
        );
        let output = chunks
            .iter()
            .flat_map(|chunk| chunk.iter().copied())
            .collect::<Vec<_>>();
        assert_eq!(output, "abé🙂zz".as_bytes());
    }

    #[test]
    fn chunked_body_writer_stops_when_receiver_is_dropped() {
        let (sender, receiver) = mpsc::channel(1);
        drop(receiver);

        let mut writer = ChunkedBodyWriter::new(sender, 4);
        assert!(fmt::Write::write_str(&mut writer, "data").is_err());
        assert!(writer.receiver_closed());
    }

    #[tokio::test]
    async fn scrape_enforces_auth_and_returns_openmetrics() {
        let metrics = Arc::new(InternalMetrics::default());
        let mut registry = Registry::with_prefix("patrolli");
        registry.register_collector(Box::new(InternalMetricsCollector {
            metrics,
            include_forwarder: false,
        }));
        let state = AppState {
            registry: Arc::new(registry),
            expected_authorization: Some(Arc::from(&b"Bearer secret"[..])),
        };

        let unauthorized = scrape(State(state.clone()), HeaderMap::new()).await;
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            unauthorized.headers().get(WWW_AUTHENTICATE).unwrap(),
            "Bearer"
        );

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer secret"));
        let response = scrape(State(state), headers).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(CONTENT_TYPE).unwrap(),
            OPENMETRICS_CONTENT_TYPE
        );
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(body.ends_with(b"# EOF\n"));
    }

    #[tokio::test]
    async fn scrape_streams_large_responses_in_bounded_chunks() {
        let metrics = Arc::new(InternalMetrics::default());
        metrics.sink.store(Arc::new((
            Instant::now(),
            vec![Sink {
                id: Uuid::now_v7(),
                name: "x".repeat(SCRAPE_CHUNK_SIZE * 2),
                ..Default::default()
            }],
        )));

        let mut registry = Registry::with_prefix("patrolli");
        registry.register_collector(Box::new(InternalMetricsCollector {
            metrics,
            include_forwarder: false,
        }));
        let state = AppState {
            registry: Arc::new(registry),
            expected_authorization: None,
        };

        let response = scrape(State(state), HeaderMap::new()).await;
        let mut stream = response.into_body().into_data_stream();
        let mut output = BytesMut::new();
        let mut chunk_count = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            assert!(chunk.len() <= SCRAPE_CHUNK_SIZE);
            output.extend_from_slice(&chunk);
            chunk_count += 1;
        }

        assert!(chunk_count > 1);
        assert!(output.ends_with(b"# EOF\n"));
    }

    #[tokio::test]
    async fn bind_failure_is_reported_at_startup() {
        let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
        let config = Prometheus {
            address: occupied.local_addr().unwrap(),
            ..Default::default()
        };

        let result = spawn(
            &config,
            Arc::new(InternalMetrics::default()),
            false,
            CancellationToken::new(),
        );
        assert!(result.is_err());
    }

    /// Records the catalogue as the Prometheus renderer would spell it, so the
    /// two renderers can be diffed against each other. Guards against the shared
    /// walk and the text output drifting apart — the whole point of the split.
    struct RecordingVisitor {
        families: Vec<(String, &'static str)>,
    }

    impl visit::Visitor for RecordingVisitor {
        type Error = std::convert::Infallible;

        fn family(
            &mut self,
            family: visit::Family,
            _samples: visit::Samples<'_>,
        ) -> Result<(), Self::Error> {
            // `prometheus-client` appends the unit to the exposed name.
            let suffix = match family.unit {
                visit::Unit::None => "",
                visit::Unit::Seconds => "_seconds",
                visit::Unit::Bytes => "_bytes",
            };
            let kind = match family.kind {
                visit::Kind::Counter => "counter",
                visit::Kind::Gauge => "gauge",
            };
            self.families.push((
                format!("patrolli_{}{suffix}", family.name.replace('.', "_")),
                kind,
            ));
            Ok(())
        }
    }

    #[test]
    fn the_catalogue_and_the_encoded_output_agree() {
        let metrics = Arc::new(InternalMetrics::default());
        metrics.sink.store(Arc::new((
            Instant::now(),
            vec![Sink {
                id: Uuid::now_v7(),
                name: "primary".to_string(),
                ..Default::default()
            }],
        )));
        let mut channel = Channel::default();
        channel.channels.insert(
            "disk".to_string(),
            ChannelState {
                kind: ChannelKind::Disk,
                capacity: 4096,
                length: 512,
            },
        );
        metrics
            .sink_channel
            .store(Arc::new((Instant::now(), channel)));
        metrics.forwarder.store(Arc::new((
            Instant::now(),
            Forwarder {
                nodes: vec![ForwarderNode {
                    id: Uuid::now_v7(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )));
        // one of each kind, since the two use disjoint families, plus a skip
        // reason and a processor so the labelled families are populated too
        metrics.lookup_table.store(Arc::new((
            Instant::now(),
            vec![
                LookupTable {
                    id: Uuid::now_v7(),
                    name: "geoip".to_string(),
                    kind: "immutable",
                    rows_skipped: SkipCounts {
                        bad_cell: 2,
                        ..Default::default()
                    },
                    generation_rows_skipped: SkipCounts {
                        bad_cell: 1,
                        ..Default::default()
                    },
                    processors: vec![LookupTableProcessor {
                        id: Uuid::now_v7(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                LookupTable {
                    id: Uuid::now_v7(),
                    name: "hosts".to_string(),
                    kind: "mutable",
                    ..Default::default()
                },
            ],
        )));

        let output = encode_test_metrics(Arc::clone(&metrics), true);

        let mut recorder = RecordingVisitor {
            families: Vec::new(),
        };
        visit::walk(&metrics, true, &mut recorder).unwrap();
        assert!(!recorder.families.is_empty());

        for (name, kind) in &recorder.families {
            assert!(
                output.contains(&format!("# TYPE {name} {kind}\n")),
                "encoded output is missing `# TYPE {name} {kind}`"
            );
        }
        // Nothing in the output that the catalogue does not describe, either.
        assert_eq!(output.matches("# TYPE ").count(), recorder.families.len());
    }

    #[test]
    fn lookup_table_skip_series_keep_zero_baselines_and_show_clean_reloads() {
        let metrics = Arc::new(InternalMetrics::default());
        let table_id = Uuid::now_v7();
        let identity = format!("table_id=\"{table_id}\",table_name=\"hosts\"");
        for (cumulative, generation) in [(0, 0), (5, 5), (5, 0)] {
            metrics.lookup_table.store(Arc::new((
                Instant::now(),
                vec![LookupTable {
                    id: table_id,
                    name: "hosts".into(),
                    kind: "immutable",
                    rows_skipped: SkipCounts {
                        bad_cell: cumulative,
                        ..Default::default()
                    },
                    generation_rows_skipped: SkipCounts {
                        bad_cell: generation,
                        ..Default::default()
                    },
                    generation: 2,
                    last_success_timestamp: 1_700_000_000.5,
                    load_error: true,
                    ..Default::default()
                }],
            )));
            let output = encode_test_metrics(metrics.clone(), false);
            assert!(output.contains(&format!(
                "patrolli_lookup_table_rows_skipped_total{{{identity},reason=\"cast\"}} {cumulative}\n"
            )));
            assert!(output.contains(&format!("patrolli_lookup_table_generation_rows_skipped{{{identity},reason=\"cast\"}} {generation}\n")));
            assert!(output.contains(&format!(
                "patrolli_lookup_table_load_error{{{identity},kind=\"immutable\"}} 1\n"
            )));
            assert!(output.contains(&format!(
                "patrolli_lookup_table_generation{{{identity},kind=\"immutable\"}} 2\n"
            )));
            assert!(output.contains(&format!("patrolli_lookup_table_last_success_timestamp_seconds{{{identity},kind=\"immutable\"}} 1700000000.5\n")));
        }
    }

    #[test]
    fn lookup_table_families_carry_their_identity_and_vanish_when_unused() {
        let metrics = Arc::new(InternalMetrics::default());
        let table_id = Uuid::now_v7();
        let node_id = Uuid::now_v7();
        metrics.lookup_table.store(Arc::new((
            Instant::now(),
            vec![LookupTable {
                id: table_id,
                name: "geoip_city".to_string(),
                kind: "immutable",
                rows: 3_000_000,
                pooled_strings: 41_000,
                loads: 4,
                rows_skipped: SkipCounts {
                    bad_cell: 12,
                    ..Default::default()
                },
                processors: vec![LookupTableProcessor {
                    id: node_id,
                    lookups: 100,
                    lookups_hit: 94,
                    lookups_missed: 6,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        )));

        let output = encode_test_metrics(Arc::clone(&metrics), false);
        let identity = format!("table_id=\"{table_id}\",table_name=\"geoip_city\"");

        assert!(output.contains(&format!(
            "patrolli_lookup_table_rows{{{identity},kind=\"immutable\"}} 3000000\n"
        )));
        assert!(output.contains(&format!(
            "patrolli_lookup_table_loads_total{{{identity},kind=\"immutable\"}} 4\n"
        )));
        // the reason is a label, so seven families do not become forty-nine
        assert!(output.contains(&format!(
            "patrolli_lookup_table_rows_skipped_total{{{identity},reason=\"cast\"}} 12\n"
        )));
        assert!(output.contains(&format!(
            "patrolli_lookup_table_rows_skipped_total{{{identity},reason=\"overlap\"}} 0\n"
        )));
        assert!(output.contains(&format!(
            "patrolli_lookup_table_processor_lookups_hit_total{{{identity},node_id=\"{node_id}\"}} 94\n"
        )));
        // an immutable table has no row cap...
        assert!(!output.contains("patrolli_lookup_table_max_rows"));
        // ...but it does hold strings: its generation interner is reported under
        // the same family a mutable table's pools are
        assert!(output.contains(&format!(
            "patrolli_lookup_table_pooled_strings{{{identity},kind=\"immutable\"}} 41000\n"
        )));

        // ...and a mutable table has no load history, so the two kinds never
        // stack permanent zero series on each other
        let mutable = Arc::new(InternalMetrics::default());
        mutable.lookup_table.store(Arc::new((
            Instant::now(),
            vec![LookupTable {
                id: Uuid::now_v7(),
                name: "seen_hosts".to_string(),
                kind: "mutable",
                rows: 12,
                max_rows: 5000,
                ..Default::default()
            }],
        )));
        let output = encode_test_metrics(Arc::clone(&mutable), false);
        assert!(output.contains("patrolli_lookup_table_max_rows"));
        assert!(
            output.contains("patrolli_lookup_table_pooled_strings"),
            "both kinds hold strings, so the family is not one of the disjoint halves"
        );
        assert!(!output.contains("patrolli_lookup_table_loads_total"));
        assert!(!output.contains("patrolli_lookup_table_rows_read_total"));
        assert!(!output.contains("patrolli_lookup_table_generation_rows_read"));
        assert!(!output.contains("patrolli_lookup_table_load_error"));
        assert!(!output.contains("patrolli_lookup_table_last_success_timestamp"));

        // with no tables at all the section disappears entirely, which is what
        // cluster mode looks like
        let empty = encode_test_metrics(Arc::new(InternalMetrics::default()), false);
        assert!(!empty.contains("lookup_table_rows"));
        assert!(!empty.contains("lookup_table_processor"));
        assert!(
            empty.contains("snapshot=\"lookup_table\""),
            "the snapshot age is still reported, so a stalled reporter is visible"
        );
    }
}
