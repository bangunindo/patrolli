// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

// End-to-end test: OTLP telemetry flows through the OpenTelemetry Arrow pipeline.
//
// This also shows that `otelarrowreceiver` and `otelarrowexporter` from `opentelemetry-collector-contrib` can work with OTLP
// Meaning our OTLP listener can receive payload from an `otelarrowexporter`
// because `otelarrowexporter` will downgrade the arrow payload to OTLP protobuf if the receiver does not support OTAP.
// Also, our OTLP sink can send payload to an `otelarrowreceiver` without us having to set separate OTAP mode sink.
//
// Architecture:
//
//   telemetrygen (metrics, logs, traces)
//   → collector-a (OTLP receiver)
//     ├── otelarrow/otap exporter  ──→ patrolli otap-listener  (mode=otap)
//     ├── otelarrow/otlp exporter  ──→ patrolli otlp-listener  (mode=otlp)
//     └── otlp exporter            ──→ patrolli both-listener  (mode=both)
//
//   Patrolli pipelines:
//     otap-listener  → otap-sink (otap mode)   → collector-b:4317 (otelarrow receiver)
//     otlp-listener  → otlp-arrow-sink (otlp)  → collector-c:4317 (otelarrow receiver)
//     both-listener  → otlp-std-sink (otlp)    → collector-d:4317 (otlp receiver)
//
// Patrolli is the binary cargo built for this test, run on the host; the collectors
// and telemetrygen are containers (so Docker is required). Patrolli reaches
// collectors b, c and d through their published ports, and collector-a reaches
// patrolli through the Docker host gateway, so patrolli's listeners bind 0.0.0.0 and a
// host firewall must let container → host traffic through.

mod setup;

use anyhow::{Result, bail};
use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use setup::patrolli::{Patrolli, free_tcp_port};
use std::sync::Arc;
use testcontainers::core::logs::LogFrame;
use testcontainers::core::logs::consumer::LogConsumer;
use testcontainers::core::{ContainerPort, Host, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use tokio::sync::Mutex;
use tracing::info;
use uuid::Uuid;

/// Pinned, and both from one collector-contrib release: the assertions read the
/// debug exporter's output, whose format can change between releases. Bump them
/// together.
const COLLECTOR_IMAGE: (&str, &str) = ("otel/opentelemetry-collector-contrib", "0.156.0");
const TELEMETRYGEN_IMAGE: (&str, &str) = (
    "ghcr.io/open-telemetry/opentelemetry-collector-contrib/telemetrygen",
    "v0.156.0",
);

/// Captures every stdout line from a container into a shared Vec.
#[derive(Clone)]
struct LineCollector {
    lines: Arc<Mutex<Vec<String>>>,
}

impl LineCollector {
    fn new() -> Self {
        Self {
            lines: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl LogConsumer for LineCollector {
    fn accept<'a>(&'a self, record: &'a LogFrame) -> BoxFuture<'a, ()> {
        async move {
            match record {
                LogFrame::StdOut(line) | LogFrame::StdErr(line) => {
                    self.lines
                        .lock()
                        .await
                        .push(String::from_utf8_lossy(line).to_string());
                }
            }
        }
        .boxed()
    }
}

/// The patrolli listener ports, as `[otap, otlp, both]`.
type ListenerPorts = [u16; 3];

/// `collectors` are the `host:port` addresses of collectors b, c and d.
fn generate_patrolli_config(listener_ports: ListenerPorts, collectors: [&str; 3]) -> String {
    let [otap_port, otlp_port, both_port] = listener_ports;
    let [collector_b, collector_c, collector_d] = collectors;
    let host_id = Uuid::now_v7();
    let processor_otap_id = Uuid::now_v7();
    let processor_otlp_id = Uuid::now_v7();
    let processor_both_id = Uuid::now_v7();

    let otap_listener_id = Uuid::now_v7();
    let otlp_listener_id = Uuid::now_v7();
    let both_listener_id = Uuid::now_v7();

    let otap_item_id = Uuid::now_v7();
    let otlp_item_id = Uuid::now_v7();
    let both_item_id = Uuid::now_v7();

    let otap_sink_id = Uuid::now_v7();
    let otlp_arrow_sink_id = Uuid::now_v7();
    let otlp_std_sink_id = Uuid::now_v7();

    format!(
        r#"
global:
  logger:
    level: info
    disable_color: true
  mib:
    disabled: true
  disk_channel:
    data_dir: ./data
monitor:
  worker_count: 10
listeners:
  - id: {otap_listener_id}
    name: otap-listener
    worker_count: 4
    type: "open_telemetry"
    config:
      mode: otap
      listener:
        address: 0.0.0.0:{otap_port}
  - id: {otlp_listener_id}
    name: otlp-listener
    worker_count: 4
    type: "open_telemetry"
    config:
      mode: otlp
      listener:
        address: 0.0.0.0:{otlp_port}
  - id: {both_listener_id}
    name: both-listener
    worker_count: 4
    type: "open_telemetry"
    config:
      mode: both
      listener:
        address: 0.0.0.0:{both_port}
hosts:
  - id: {host_id}
    name: test-host
processors:
  - id: {processor_otap_id}
    name: passthrough-otap
    type: "null"
  - id: {processor_otlp_id}
    name: passthrough-otlp
    type: "null"
  - id: {processor_both_id}
    name: passthrough-both
    type: "null"
items:
  - type: listener
    id: {otap_item_id}
    name: otap-item
    host_id: {host_id}
    listener_id: {otap_listener_id}
    processors:
      - {processor_otap_id}
    config:
      match_strategy:
        method: none
  - type: listener
    id: {otlp_item_id}
    name: otlp-item
    host_id: {host_id}
    listener_id: {otlp_listener_id}
    processors:
      - {processor_otlp_id}
    config:
      match_strategy:
        method: none
  - type: listener
    id: {both_item_id}
    name: both-item
    host_id: {host_id}
    listener_id: {both_listener_id}
    processors:
      - {processor_both_id}
    config:
      match_strategy:
        method: none
sinks:
  - type: open_telemetry
    name: otap-sink
    id: {otap_sink_id}
    worker_count: 2
    inputs:
      - {processor_otap_id}
    config:
      mode: otap
      destinations:
        - "{collector_b}"
      compression: none
    batch_size: 1000
    batch_timeout: 1000
  - type: open_telemetry
    name: otlp-arrow-sink
    id: {otlp_arrow_sink_id}
    worker_count: 2
    inputs:
      - {processor_otlp_id}
    config:
      mode: otlp
      destinations:
        - "{collector_c}"
      compression: none
    batch_size: 1000
    batch_timeout: 1000
  - type: open_telemetry
    name: otlp-std-sink
    id: {otlp_std_sink_id}
    worker_count: 2
    inputs:
      - {processor_both_id}
    config:
      mode: otlp
      destinations:
        - "{collector_d}"
      compression: none
    batch_size: 1000
    batch_timeout: 1000
"#
    )
}

/// Collector-a runs in a container, so it reaches patrolli on the host through
/// the `host.docker.internal` alias for the host gateway.
fn generate_collector_a_config(listener_ports: ListenerPorts) -> String {
    let [otap_port, otlp_port, both_port] = listener_ports;
    format!(
        r#"
receivers:
  otlp:
    protocols:
      grpc:
        endpoint: 0.0.0.0:4317
exporters:
  otelarrow/otap:
    endpoint: host.docker.internal:{otap_port}
    tls:
      insecure: true
    compression: none
    arrow:
      zstd:
        level: 3
  otelarrow/otlp:
    endpoint: host.docker.internal:{otlp_port}
    tls:
      insecure: true
    compression: none
    arrow:
      zstd:
        level: 3
  otlp:
    endpoint: host.docker.internal:{both_port}
    tls:
      insecure: true
    compression: none
service:
  pipelines:
    logs:
      receivers: [otlp]
      exporters: [otlp, otelarrow/otap, otelarrow/otlp]
    traces:
      receivers: [otlp]
      exporters: [otlp, otelarrow/otap, otelarrow/otlp]
    metrics:
      receivers: [otlp]
      exporters: [otlp, otelarrow/otap, otelarrow/otlp]
"#
    )
}

fn generate_collector_b_config() -> String {
    r#"
receivers:
  otelarrow:
    protocols:
      grpc:
        endpoint: 0.0.0.0:4317
exporters:
  debug:
    verbosity: detailed
service:
  pipelines:
    logs:
      receivers: [otelarrow]
      exporters: [debug]
    traces:
      receivers: [otelarrow]
      exporters: [debug]
    metrics:
      receivers: [otelarrow]
      exporters: [debug]
"#
    .to_string()
}

fn generate_collector_c_config() -> String {
    r#"
receivers:
  otelarrow:
    protocols:
      grpc:
        endpoint: 0.0.0.0:4317
exporters:
  debug:
    verbosity: detailed
service:
  pipelines:
    logs:
      receivers: [otelarrow]
      exporters: [debug]
    traces:
      receivers: [otelarrow]
      exporters: [debug]
    metrics:
      receivers: [otelarrow]
      exporters: [debug]
"#
    .to_string()
}

fn generate_collector_d_config() -> String {
    r#"
receivers:
  otlp:
    protocols:
      grpc:
        endpoint: 0.0.0.0:4317
exporters:
  debug:
    verbosity: detailed
service:
  pipelines:
    logs:
      receivers: [otlp]
      exporters: [debug]
    traces:
      receivers: [otlp]
      exporters: [debug]
    metrics:
      receivers: [otlp]
      exporters: [debug]
"#
    .to_string()
}

/// Start a receiving collector (b, c or d), with its 4317 published on the host.
async fn start_receiver(config: String, logs: LineCollector) -> ContainerAsync<GenericImage> {
    GenericImage::new(COLLECTOR_IMAGE.0, COLLECTOR_IMAGE.1)
        .with_exposed_port(ContainerPort::Tcp(4317))
        .with_wait_for(WaitFor::message_on_stderr("Everything is ready"))
        .with_copy_to("/etc/otelcol/config.yaml", config.into_bytes())
        .with_log_consumer(logs)
        .with_cmd(vec!["--config=/etc/otelcol/config.yaml"])
        .start()
        .await
        .expect("failed to start a receiving collector container")
}

/// The `host:port` patrolli's sink dials to reach `collector`.
async fn receiver_address(collector: &ContainerAsync<GenericImage>) -> String {
    let host = collector.get_host().await.unwrap();
    let port = collector.get_host_port_ipv4(4317).await.unwrap();
    format!("{host}:{port}")
}

#[tokio::test]
async fn test_otel_arrow_e2e() -> Result<()> {
    setup::setup_logging();

    // Per-run suffix keeps container names unique even on repeated runs.
    let run_id = Uuid::now_v7().simple().to_string()[..8].to_string();
    let network = format!("otel-e2e-{run_id}");
    let collector_a_name = format!("otel-e2e-{run_id}-col-a");
    let listener_ports: ListenerPorts = [free_tcp_port(), free_tcp_port(), free_tcp_port()];

    // ---- Start collectors b, c and d (the sinks' destinations) ---------
    // b and c: otelarrow receiver + debug exporter; d: otlp receiver + debug.
    let collector_b_logs = LineCollector::new();
    let collector_c_logs = LineCollector::new();
    let collector_d_logs = LineCollector::new();
    let collector_b = start_receiver(generate_collector_b_config(), collector_b_logs.clone()).await;
    let collector_c = start_receiver(generate_collector_c_config(), collector_c_logs.clone()).await;
    let collector_d = start_receiver(generate_collector_d_config(), collector_d_logs.clone()).await;

    // ---- Start patrolli ---------------------------------------------------
    let patrolli_config = generate_patrolli_config(
        listener_ports,
        [
            &receiver_address(&collector_b).await,
            &receiver_address(&collector_c).await,
            &receiver_address(&collector_d).await,
        ],
    );
    let mut patrolli = Patrolli::start("patrolli", &patrolli_config, &[]);

    // Collector-a only starts sending once patrolli listens.
    setup::eventually(async || {
        for port in listener_ports {
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_err()
            {
                bail!("patrolli is not listening on {port} yet");
            }
        }
        Ok(())
    })
    .await;

    // ---- Start collector-a (otlp receiver + otelarrow/otlp exporters) ---
    // Its exporters retry, so it needs no wait past its own readiness.
    let collector_a_config = generate_collector_a_config(listener_ports);
    let collector_a = GenericImage::new(COLLECTOR_IMAGE.0, COLLECTOR_IMAGE.1)
        .with_wait_for(WaitFor::message_on_stderr("Everything is ready"))
        .with_network(&network)
        .with_container_name(&collector_a_name)
        .with_host("host.docker.internal", Host::HostGateway)
        .with_copy_to("/etc/otelcol/config.yaml", collector_a_config.into_bytes())
        .with_cmd(vec!["--config=/etc/otelcol/config.yaml"])
        .start()
        .await
        .expect("failed to start collector-a container");

    // ---- Start telemetry generators ------------------------------------
    let gen_duration: &str = "5s";
    let gen_rate = 1000;
    let batch_size = 1000;

    let start_generator = async |signal: &str| {
        GenericImage::new(TELEMETRYGEN_IMAGE.0, TELEMETRYGEN_IMAGE.1)
            .with_network(&network)
            .with_cmd([
                signal,
                "--otlp-insecure",
                &format!("--otlp-endpoint={collector_a_name}:4317"),
                &format!("--duration={gen_duration}"),
                &format!("--rate={gen_rate}"),
                &format!("--batch-size={batch_size}"),
            ])
            .start()
            .await
            .unwrap_or_else(|e| panic!("failed to start {signal} telemetrygen: {e}"))
    };
    let metrics_gen = start_generator("metrics").await;
    let logs_gen = start_generator("logs").await;
    let traces_gen = start_generator("traces").await;

    // ---- Assertions ----------------------------------------------------
    // Each collector's debug exporter tags what it received with its signal.
    info!("waiting for telemetry to flow through the pipeline");
    let receivers = [
        ("collector-b (otap path)", &collector_b_logs),
        ("collector-c (otlp-arrow path)", &collector_c_logs),
        ("collector-d (otlp-std path)", &collector_d_logs),
    ];
    setup::eventually(async || {
        let mut missing = Vec::new();
        for (name, logs) in receivers {
            let lines = logs.lines.lock().await;
            for signal in ["metrics", "logs", "traces"] {
                let pattern = format!("\"otelcol.signal\": \"{signal}\"");
                if !lines.iter().any(|l| l.contains(&pattern)) {
                    missing.push(format!("{name} has no {signal} in {} lines", lines.len()));
                }
            }
        }
        if !missing.is_empty() {
            bail!("{}", missing.join("; "));
        }
        Ok(())
    })
    .await;

    // ---- Cleanup -------------------------------------------------------
    traces_gen.stop().await.ok();
    logs_gen.stop().await.ok();
    metrics_gen.stop().await.ok();
    collector_a.stop().await.ok();
    patrolli.stop().await;
    collector_b.stop().await.ok();
    collector_c.stop().await.ok();
    collector_d.stop().await.ok();

    Ok(())
}
