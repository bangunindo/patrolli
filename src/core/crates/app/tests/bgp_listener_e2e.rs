// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! End to end through the `patrolli` binary: a BMP feed fills the route table
//! of a `bgp` listener, and `attribute_lookup` enriches records from it.
//!
//! ```text
//!   GoBGP's Loc-RIB capture (akvorado's bmp-locrib.pcap) over TCP
//!   → bgp listener → `bgp` lookup table
//!
//!   OTLP logs (tonic client), each with a router and a destination
//!   → open_telemetry listener (otlp)
//!   → enrich (attribute_lookup on the route table, vrf bound to `global`)
//!   → file sink on stdout
//! ```
//!
//! Then the Prometheus endpoint shows the listener's `bgp` families. Needs no
//! Docker.

mod setup;

use otel_arrow_dfe_pdata::proto::opentelemetry::collector::logs::v1::{
    ExportLogsServiceRequest, logs_service_client::LogsServiceClient,
};
use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::any_value::Value as AnyVal;
use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::{AnyValue, KeyValue};
use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use setup::patrolli::{Patrolli, free_tcp_port};
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use uuid::Uuid;

/// A debug build starting on a busy machine.
const STARTUP_LIMIT: Duration = Duration::from_secs(60);
const LIMIT: Duration = Duration::from_secs(30);

const LOC_RIB_PCAP: &[u8] =
    include_bytes!("../../../src/listener/bgp/testdata/akvorado/bmp-locrib.pcap");

/// The concatenated TCP payloads of a classic Ethernet pcap. A copy of the
/// unit tests' reader, which integration tests cannot reach.
fn tcp_payload(pcap: &[u8]) -> Vec<u8> {
    let le = matches!(pcap.get(..4), Some([0xd4, 0xc3, 0xb2, 0xa1]));
    let u32_at = |o: usize| {
        let v: [u8; 4] = pcap[o..o + 4].try_into().unwrap();
        if le {
            u32::from_le_bytes(v)
        } else {
            u32::from_be_bytes(v)
        }
    };
    let be16 = |b: &[u8], o: usize| u16::from_be_bytes([b[o], b[o + 1]]) as usize;
    let mut out = Vec::new();
    let mut off = 24;
    while off + 16 <= pcap.len() {
        let incl = u32_at(off + 8) as usize;
        let pkt = &pcap[off + 16..off + 16 + incl];
        off += 16 + incl;
        let (tcp, end) = match be16(pkt, 12) {
            0x0800 => {
                let ihl = (pkt[14] & 0x0f) as usize * 4;
                (14 + ihl, 14 + be16(pkt, 16))
            }
            0x86dd => (14 + 40, 14 + 40 + be16(pkt, 18)),
            other => panic!("ethertype {other:#x}"),
        };
        let data = tcp + (pkt[tcp + 12] >> 4) as usize * 4;
        out.extend_from_slice(&pkt[data..end]);
    }
    out
}

fn config(otlp: u16, bmp: u16, prometheus: u16) -> String {
    let [bgp, listener, host, item, enrich, sink, routes] = [(); 7].map(|_| Uuid::now_v7());
    format!(
        r#"global:
  logger:
    level: info
    disable_color: true
  mib:
    disabled: true
  disk_channel:
    data_dir: ./data
  internal_metrics_interval: 200
  prometheus:
    address: "127.0.0.1:{prometheus}"
lookup_tables:
  - id: {routes}
    name: routes
    source:
      type: bgp
      listener: {bgp}
listeners:
  - id: {bgp}
    name: routes
    type: bgp
    worker_count: 1
    config:
      bmp:
        listener:
          address: 127.0.0.1:{bmp}
  - id: {listener}
    name: otlp
    type: open_telemetry
    worker_count: 1
    batch_size: 1000
    batch_timeout: 100
    config:
      mode: otlp
      listener:
        address: 127.0.0.1:{otlp}
hosts:
  - id: {host}
    name: h1
processors:
  - id: {enrich}
    name: enrich
    type: attribute_lookup
    config:
      lookups:
        - lookup_table: {routes}
          table: record
          match:
            keys:
              router: {{key: router, cast: ip}}
            literals:
              vrf: global
            point: {{key: dst.ip, cast: ip}}
          output:
            dst.as: origin_as
            dst.as_path: as_path
            dst.network: prefix
            dst.next_hop: next_hop
items:
  - type: listener
    id: {item}
    name: otlp-item
    host_id: {host}
    listener_id: {listener}
    processors: [{enrich}]
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
    inputs: [{enrich}]
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

/// The string attributes of each enriched record on stdout, by `probe.id`.
fn probes_seen(patrolli: &Patrolli) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut seen = BTreeMap::new();
    for line in patrolli.stdout_lines() {
        if !line.starts_with(r#"{"kind":"logs""#) {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        let attrs: BTreeMap<String, String> = value["record"]["attributes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|kv| {
                let v = &kv["value"];
                let text = v["stringValue"]
                    .as_str()
                    .or_else(|| v["intValue"].as_str())?;
                Some((kv["key"].as_str()?.to_string(), text.to_string()))
            })
            .collect();
        if let Some(id) = attrs.get("probe.id").cloned() {
            seen.insert(id, attrs);
        }
    }
    seen
}

async fn wait_for(what: &str, mut check: impl AsyncFnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + LIMIT;
    while !check().await {
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn a_bmp_feed_enriches_records() {
    let (otlp, bmp, prometheus) = (free_tcp_port(), free_tcp_port(), free_tcp_port());
    let mut patrolli = Patrolli::start("bgp", &config(otlp, bmp, prometheus), &[]);
    let started = tokio::time::Instant::now();
    while !patrolli.stderr_contains("startup complete") {
        assert!(
            started.elapsed() < STARTUP_LIMIT,
            "patrolli did not start within {STARTUP_LIMIT:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // the router: kept connected, so its routes are live
    let mut router =
        setup::eventually(async || Ok(TcpStream::connect(("127.0.0.1", bmp)).await?)).await;
    router.write_all(&tcp_payload(LOC_RIB_PCAP)).await.unwrap();

    let mut client = setup::eventually(async || {
        Ok(LogsServiceClient::connect(format!("http://127.0.0.1:{otlp}")).await?)
    })
    .await;

    // probes until the routes are in: a probe can arrive before the dump
    let want = [
        (
            "v4",
            "198.51.100.77",
            "64476",
            Some("65011 174 64476"),
            "198.51.100.0/25",
        ),
        ("v6", "2001:db8:2::9", "12322", None, "2001:db8:2::/64"),
    ];
    let mut round = 0;
    wait_for("probes enriched from the route table", async || {
        round += 1;
        let records = want
            .iter()
            .map(|(name, dst, ..)| LogRecord {
                time_unix_nano: 1_700_000_000_000_000_000,
                attributes: vec![
                    s("probe.id", &format!("{round}/{name}")),
                    s("router", "127.0.0.1"),
                    s("dst.ip", dst),
                ],
                ..Default::default()
            })
            .collect();
        client
            .export(ExportLogsServiceRequest {
                resource_logs: vec![ResourceLogs {
                    scope_logs: vec![ScopeLogs {
                        log_records: records,
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
            })
            .await
            .expect("otlp export failed");
        tokio::time::sleep(Duration::from_millis(300)).await;
        let seen = probes_seen(&patrolli);
        want.iter().all(|(name, _, origin, path, prefix)| {
            seen.get(&format!("{round}/{name}")).is_some_and(|a| {
                a.get("dst.as").map(String::as_str) == Some(origin)
                    && path.is_none_or(|p| a.get("dst.as_path").map(String::as_str) == Some(p))
                    && a.get("dst.network").map(String::as_str) == Some(prefix)
            })
        })
    })
    .await;

    // the listener's own families
    let http = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{prometheus}/metrics");
    wait_for("bgp families on the prometheus endpoint", async || {
        let Ok(response) = http.get(&url).send().await else {
            return false;
        };
        let text = response.text().await.unwrap_or_default();
        let has = |family: &str, value: &str| {
            text.lines().any(|l| {
                l.starts_with(family)
                    && l.contains("listener_name=\"routes\"")
                    && l.ends_with(value)
            })
        };
        has("patrolli_listener_bgp_routers_live{", " 1")
            && has("patrolli_listener_bgp_route_table{", " 1")
            && has("patrolli_listener_bgp_routes_announced_total{", " 5")
    })
    .await;

    drop(router);
    patrolli.stop().await;
}
