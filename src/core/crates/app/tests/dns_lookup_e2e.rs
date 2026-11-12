// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! End to end through the `patrolli` binary: the `dns_lookup` processor and
//! the resolver behind it.
//!
//! ```text
//!   OTLP logs (tonic client)
//!   → open_telemetry listener (otlp)
//!   → dns_lookup (ptr, forward mode)  ──queue──>  resolver ──udp──> in-test responder
//!   → file sink on stdout (json_otlp_denormalized, one record per line)
//! ```
//!
//! A lookup never waits for DNS, so the first batch of an address passes
//! through unenriched; a later one carries the name. Needs no Docker.

mod setup;

use hickory_resolver::proto::op::{Message, MessageType, OpCode, ResponseCode};
use hickory_resolver::proto::rr::rdata::PTR;
use hickory_resolver::proto::rr::{Name, RData, Record, RecordType};
use otel_arrow_dfe_pdata::proto::opentelemetry::collector::logs::v1::{
    ExportLogsServiceRequest, logs_service_client::LogsServiceClient,
};
use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::any_value::Value as AnyVal;
use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::{AnyValue, KeyValue};
use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use parking_lot::Mutex;
use setup::patrolli::{Patrolli, free_tcp_port};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tonic::transport::Channel;
use uuid::Uuid;

/// A debug build starting on a busy machine.
const STARTUP_LIMIT: Duration = Duration::from_secs(60);

/// Answers PTR questions over UDP from a fixed table, "no such record" for
/// anything else, and counts the questions per name.
struct Responder {
    port: u16,
    asked: Arc<Mutex<BTreeMap<String, usize>>>,
}

impl Responder {
    async fn start(names: &[(&str, &str)]) -> Self {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = socket.local_addr().unwrap().port();
        let names: BTreeMap<Name, Name> = names
            .iter()
            .map(|(ip, name)| {
                (
                    Name::from(ip.parse::<std::net::IpAddr>().unwrap()),
                    Name::from_ascii(name).unwrap(),
                )
            })
            .collect();
        let asked = Arc::new(Mutex::new(BTreeMap::new()));
        let counts = asked.clone();
        tokio::spawn(async move {
            let mut buf = vec![0; 4096];
            loop {
                let Ok((len, peer)) = socket.recv_from(&mut buf).await else {
                    return;
                };
                let Ok(request) = Message::from_vec(&buf[..len]) else {
                    continue;
                };
                let mut response =
                    Message::new(request.metadata.id, MessageType::Response, OpCode::Query);
                response.metadata.recursion_desired = request.metadata.recursion_desired;
                response.metadata.recursion_available = true;
                for query in &request.queries {
                    response.add_query(query.clone());
                    *counts.lock().entry(query.name().to_string()).or_insert(0) += 1;
                    match names.get(query.name()) {
                        Some(name) if query.query_type() == RecordType::PTR => {
                            response.add_answer(Record::from_rdata(
                                query.name().clone(),
                                300,
                                RData::PTR(PTR(name.clone())),
                            ));
                        }
                        _ => response.metadata.response_code = ResponseCode::NXDomain,
                    }
                }
                let _ = socket.send_to(&response.to_vec().unwrap(), peer).await;
            }
        });
        Self { port, asked }
    }

    fn asked(&self, ip: &str) -> usize {
        let name = Name::from(ip.parse::<std::net::IpAddr>().unwrap()).to_string();
        self.asked.lock().get(&name).copied().unwrap_or(0)
    }
}

fn config(port: u16, dns_port: u16, metrics_port: u16) -> String {
    let [listener, host, item, reverse, sink] = [(); 5].map(|_| Uuid::now_v7());
    format!(
        r#"global:
  logger:
    level: info
    disable_color: true
  mib:
    disabled: true
  disk_channel:
    data_dir: ./data
  internal_metrics_interval: 100
  prometheus:
    address: 127.0.0.1:{metrics_port}
  dns:
    mode: forward
    forwarders: [udp://127.0.0.1:{dns_port}]
listeners:
  - id: {listener}
    name: otlp
    type: open_telemetry
    worker_count: 1
    batch_size: 1000
    batch_timeout: 100
    config:
      mode: otlp
      listener:
        address: 127.0.0.1:{port}
hosts:
  - id: {host}
    name: h1
processors:
  - id: {reverse}
    name: reverse
    type: dns_lookup
    config:
      lookups:
        - record: ptr
          table: record
          input: {{key: src}}
          output: src.domain
items:
  - type: listener
    id: {item}
    name: otlp-item
    host_id: {host}
    listener_id: {listener}
    processors: [{reverse}]
    config:
      match_strategy:
        method: none
sinks:
  - type: file
    id: {sink}
    name: stdout
    worker_count: 1
    batch_size: 1000
    batch_timeout: 100
    inputs: [{reverse}]
    config:
      path: stdout
      format: json_otlp_denormalized
"#
    )
}

fn s(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(AnyVal::StringValue(value.to_string())),
        }),
    }
}

async fn export(client: &mut LogsServiceClient<Channel>, records: Vec<Vec<KeyValue>>) {
    let log_records = records
        .into_iter()
        .map(|attributes| LogRecord {
            time_unix_nano: 1_700_000_000_000_000_000,
            attributes,
            ..Default::default()
        })
        .collect();
    let request = ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            scope_logs: vec![ScopeLogs {
                log_records,
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    client.export(request).await.expect("otlp export failed");
}

/// The record attributes of the log on stdout carrying `probe.id == id`.
fn seen(patrolli: &Patrolli, id: &str) -> Option<BTreeMap<String, String>> {
    patrolli.stdout_lines().iter().find_map(|line| {
        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        let attrs: BTreeMap<String, String> = value["record"]["attributes"]
            .as_array()?
            .iter()
            .filter_map(|kv| {
                Some((
                    kv["key"].as_str()?.to_string(),
                    kv["value"]["stringValue"].as_str()?.to_string(),
                ))
            })
            .collect();
        (attrs.get("probe.id").map(String::as_str) == Some(id)).then_some(attrs)
    })
}

#[tokio::test]
async fn names_reach_later_records_without_holding_up_the_first() {
    setup::setup_logging();
    let responder = Responder::start(&[("8.8.8.8", "dns.google.")]).await;
    let (port, metrics_port) = (free_tcp_port(), free_tcp_port());
    let mut patrolli = Patrolli::start("dns", &config(port, responder.port, metrics_port), &[]);
    let started = tokio::time::Instant::now();
    while !patrolli.stderr_contains("startup complete") {
        assert!(
            started.elapsed() < STARTUP_LIMIT,
            "patrolli did not start within {STARTUP_LIMIT:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut client = setup::eventually(async || {
        Ok(LogsServiceClient::connect(format!("http://127.0.0.1:{port}")).await?)
    })
    .await;

    // The first batch is through before anything was asked: it comes out
    // unenriched, and leaves the questions behind.
    export(
        &mut client,
        vec![
            vec![s("probe.id", "first/found"), s("src", "8.8.8.8")],
            vec![s("probe.id", "first/negative"), s("src", "192.0.2.1")],
        ],
    )
    .await;
    let first = setup::eventually(async || {
        seen(&patrolli, "first/found").ok_or_else(|| anyhow::anyhow!("first batch not out yet"))
    })
    .await;
    assert_eq!(first.get("src.domain"), None);

    // A later batch finds the answers.
    let mut round = 0;
    let enriched = setup::eventually(async || {
        round += 1;
        let id = format!("later/{round}");
        export(
            &mut client,
            vec![
                vec![s("probe.id", &id), s("src", "8.8.8.8")],
                vec![
                    s("probe.id", &format!("{id}/negative")),
                    s("src", "192.0.2.1"),
                ],
            ],
        )
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let attrs = seen(&patrolli, &id).ok_or_else(|| anyhow::anyhow!("{id} not out yet"))?;
        anyhow::ensure!(attrs.contains_key("src.domain"), "{id} not enriched yet");
        let negative = seen(&patrolli, &format!("{id}/negative"))
            .ok_or_else(|| anyhow::anyhow!("{id}/negative not out yet"))?;
        Ok((attrs, negative))
    })
    .await;
    assert_eq!(
        enriched.0.get("src.domain").map(String::as_str),
        Some("dns.google")
    );
    // "no such name" is cached as nothing: no attribute, and asked once
    assert_eq!(enriched.1.get("src.domain"), None);
    assert_eq!(responder.asked("8.8.8.8"), 1);
    assert_eq!(responder.asked("192.0.2.1"), 1);

    // and the internal metrics saw it all
    let metrics = setup::eventually(async || {
        let body = reqwest::get(format!("http://127.0.0.1:{metrics_port}/metrics"))
            .await?
            .text()
            .await?;
        for line in [
            "patrolli_dns_resolved_positive_total{record=\"ptr\"} 1\n",
            "patrolli_dns_resolved_negative_total{record=\"ptr\"} 1\n",
            "patrolli_dns_queue_enqueued_total 2\n",
            "patrolli_dns_forwarder_requests_total{forwarder=",
        ] {
            anyhow::ensure!(body.contains(line), "no {line:?} yet");
        }
        Ok(body)
    })
    .await;
    assert!(metrics.contains("patrolli_dns_lookups_hit_total{record=\"ptr\"}"));

    patrolli.stop().await;
    assert!(patrolli.stderr_contains("dns resolver stopped"));
}
