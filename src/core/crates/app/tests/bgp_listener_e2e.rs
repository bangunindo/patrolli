// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! End to end through the `patrolli` binary: a BMP feed, or a passive BGP
//! session, fills the route table of a `bgp` listener, and `attribute_lookup`
//! enriches records from it.
//!
//! ```text
//!   GoBGP's Loc-RIB capture (akvorado's bmp-locrib.pcap) over TCP,
//!   or a router played by the test over BGP
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
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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

/// `section` is the bgp listener's protocol section, indented under `config:`.
fn config(otlp: u16, section: &str, prometheus: u16) -> String {
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
{section}
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
              # no mapping processor in front, so every probe falls back to
              # its `router`
              router: {{key: [bgp.router, router], cast: ip}}
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

async fn start(section: &str) -> (Patrolli, u16, u16) {
    let (otlp, prometheus) = (free_tcp_port(), free_tcp_port());
    let patrolli = Patrolli::start("bgp", &config(otlp, section, prometheus), &[]);
    let started = tokio::time::Instant::now();
    while !patrolli.stderr_contains("startup complete") {
        assert!(
            started.elapsed() < STARTUP_LIMIT,
            "patrolli did not start within {STARTUP_LIMIT:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    (patrolli, otlp, prometheus)
}

/// One probe: a name, a destination, and the origin AS, AS path (if
/// checked) and prefix it must come back with.
type Probe<'a> = (&'a str, &'a str, &'a str, Option<&'a str>, &'a str);

/// Sends `want`'s probes for `router` until they all come back enriched: a
/// probe can arrive before the routes do.
async fn probe(patrolli: &Patrolli, otlp: u16, router: &str, want: &[Probe<'_>]) {
    let mut client = setup::eventually(async || {
        Ok(LogsServiceClient::connect(format!("http://127.0.0.1:{otlp}")).await?)
    })
    .await;
    let mut round = 0;
    wait_for("probes enriched from the route table", async || {
        round += 1;
        let records = want
            .iter()
            .map(|(name, dst, ..)| LogRecord {
                time_unix_nano: 1_700_000_000_000_000_000,
                attributes: vec![
                    s("probe.id", &format!("{round}/{name}")),
                    s("router", router),
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
        let seen = probes_seen(patrolli);
        want.iter().all(|(name, _, origin, path, prefix)| {
            seen.get(&format!("{round}/{name}")).is_some_and(|a| {
                a.get("dst.as").map(String::as_str) == Some(origin)
                    && path.is_none_or(|p| a.get("dst.as_path").map(String::as_str) == Some(p))
                    && a.get("dst.network").map(String::as_str) == Some(prefix)
            })
        })
    })
    .await;
}

/// Waits for every `(series prefix, value)` of the listener on the
/// Prometheus endpoint.
async fn scrape(prometheus: u16, series: &[(&str, &str)]) {
    let http = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{prometheus}/metrics");
    wait_for("bgp families on the prometheus endpoint", async || {
        let Ok(response) = http.get(&url).send().await else {
            return false;
        };
        let text = response.text().await.unwrap_or_default();
        series.iter().all(|(family, value)| {
            text.lines().any(|l| {
                l.starts_with(family)
                    && l.contains("listener_name=\"routes\"")
                    && l.ends_with(value)
            })
        })
    })
    .await;
}

#[tokio::test]
async fn a_bmp_feed_enriches_records() {
    let bmp = free_tcp_port();
    let section = format!("      bmp:\n        listener:\n          address: 127.0.0.1:{bmp}");
    let (mut patrolli, otlp, prometheus) = start(&section).await;

    // the router: kept connected, so its routes are live
    let mut router =
        setup::eventually(async || Ok(TcpStream::connect(("127.0.0.1", bmp)).await?)).await;
    router.write_all(&tcp_payload(LOC_RIB_PCAP)).await.unwrap();

    probe(
        &patrolli,
        otlp,
        "127.0.0.1",
        &[
            (
                "v4",
                "198.51.100.77",
                "64476",
                Some("65011 174 64476"),
                "198.51.100.0/25",
            ),
            ("v6", "2001:db8:2::9", "12322", None, "2001:db8:2::/64"),
        ],
    )
    .await;
    scrape(
        prometheus,
        &[
            ("patrolli_listener_bgp_routers_live{", " 1"),
            ("patrolli_listener_bgp_route_table{", " 1"),
            ("patrolli_listener_bgp_routes_announced_total{", " 5"),
        ],
    )
    .await;

    drop(router);
    patrolli.stop().await;
}

/// A BGP message: marker, length, type, body.
fn bgp_message(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![0xff; 16];
    out.extend_from_slice(&((19 + body.len()) as u16).to_be_bytes());
    out.push(kind);
    out.extend_from_slice(body);
    out
}

/// An UPDATE body announcing `nlri` (already encoded) with an AS_SEQUENCE of
/// 4-octet ASes and `attrs` besides.
fn update_body(path: &[u32], mut attrs: Vec<u8>, nlri: &[u8]) -> Vec<u8> {
    let mut seg = vec![2, path.len() as u8];
    for asn in path {
        seg.extend_from_slice(&asn.to_be_bytes());
    }
    let mut all = vec![0x40, 1, 1, 0]; // ORIGIN IGP
    all.extend_from_slice(&[0x40, 2, seg.len() as u8]);
    all.extend_from_slice(&seg);
    all.append(&mut attrs);
    let mut out = vec![0, 0]; // no withdrawn routes
    out.extend_from_slice(&(all.len() as u16).to_be_bytes());
    out.extend_from_slice(&all);
    out.extend_from_slice(nlri);
    out
}

#[tokio::test]
async fn a_bgp_peer_enriches_records() {
    let bgp = free_tcp_port();
    let section = format!(
        "      bgp:\n        listener:\n          address: 127.0.0.1:{bgp}\n        hold_time: 30"
    );
    let (mut patrolli, otlp, prometheus) = start(&section).await;

    // the router, from its own loopback address
    let mut router = setup::eventually(async || {
        let socket = tokio::net::TcpSocket::new_v4()?;
        socket.bind("127.0.0.2:0".parse()?)?;
        Ok(socket.connect(([127, 0, 0, 1], bgp).into()).await?)
    })
    .await;
    // OPEN: AS 64500, hold 30, id 198.51.100.1; IPv4 and IPv6 unicast,
    // 4-octet AS
    let caps = [
        1, 4, 0, 1, 0, 1, // multiprotocol ipv4 unicast
        1, 4, 0, 2, 0, 1, // multiprotocol ipv6 unicast
        65, 4, 0, 0, 0xfb, 0xf4, // 4-octet AS 64500
    ];
    let mut open = vec![
        4,
        0xfb,
        0xf4,
        0,
        30,
        198,
        51,
        100,
        1,
        caps.len() as u8 + 2,
        2,
    ];
    open.push(caps.len() as u8);
    open.extend_from_slice(&caps);
    router.write_all(&bgp_message(1, &open)).await.unwrap();
    // patrolli's OPEN and KEEPALIVE, then ours
    let mut header = [0u8; 19];
    for want in [1u8, 4] {
        router.read_exact(&mut header).await.unwrap();
        assert_eq!(header[18], want);
        let len = u16::from_be_bytes([header[16], header[17]]) as usize;
        let mut body = vec![0; len - 19];
        router.read_exact(&mut body).await.unwrap();
    }
    router.write_all(&bgp_message(4, &[])).await.unwrap();

    // 203.0.113.0/24 via 192.0.2.1, and 2001:db8:5::/48 over MP_REACH
    let v4 = update_body(
        &[64501, 64510],
        vec![0x40, 3, 4, 192, 0, 2, 1],
        &[24, 203, 0, 113],
    );
    let mut mp = vec![0, 2, 1, 16];
    mp.extend_from_slice(
        &"2001:db8::1"
            .parse::<std::net::Ipv6Addr>()
            .unwrap()
            .octets(),
    );
    mp.push(0);
    mp.extend_from_slice(&[48, 0x20, 0x01, 0x0d, 0xb8, 0, 5]);
    let mut mp_attr = vec![0x80, 14, mp.len() as u8];
    mp_attr.extend_from_slice(&mp);
    let v6 = update_body(&[64502], mp_attr, &[]);
    router.write_all(&bgp_message(2, &v4)).await.unwrap();
    router.write_all(&bgp_message(2, &v6)).await.unwrap();

    probe(
        &patrolli,
        otlp,
        "127.0.0.2",
        &[
            (
                "v4",
                "203.0.113.9",
                "64510",
                Some("64501 64510"),
                "203.0.113.0/24",
            ),
            ("v6", "2001:db8:5::1", "64502", None, "2001:db8:5::/48"),
        ],
    )
    .await;
    scrape(
        prometheus,
        &[
            ("patrolli_listener_bgp_routers_live{", " 1"),
            ("patrolli_listener_bgp_routes_announced_total{", " 2"),
            ("patrolli_listener_bgp_sessions_opened_total{", " 1"),
        ],
    )
    .await;

    drop(router);
    patrolli.stop().await;
}
