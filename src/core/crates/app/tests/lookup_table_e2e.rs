// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! End to end through the `patrolli` binary: lookup tables, `attribute_collect`
//! and `attribute_lookup`.
//!
//! ```text
//!   OTLP logs (tonic client)
//!   → open_telemetry listener (otlp)
//!   → collect   (attribute_collect: upsert / delete / delete_partition)
//!   → enrich    (attribute_lookup over all four tables)
//!   → file sink on stdout (json_otlp_denormalized, one record per line)
//! ```
//!
//! Four tables, one per key kind and source:
//!   - `owners`: hash key, file-backed
//!   - `geo`: prefix key, file-backed, nested v4 prefixes plus a v6 one
//!   - `iface`: hash key, processor-fed
//!   - `routes`: prefix key partitioned by peer, processor-fed, `ttl: 0`
//!
//! Write records carry `iface.op` or `route.op`, which the collect rules guard
//! on. Probe records carry `probe.id` and the attributes the lookup rules read.
//! Each phase sends its writes once, then sends rounds of probes until the
//! enriched probes on stdout show what the writes should have left behind.
//! A probe goes through the same collect → enrich path as the writes, so it can
//! overtake them on another processor worker; that is why a phase keeps
//! probing instead of checking once.
//!
//! Needs no Docker.

mod setup;

use anyhow::{Context, Result, bail};
use otel_arrow_dfe_pdata::proto::opentelemetry::collector::logs::v1::{
    ExportLogsServiceRequest, logs_service_client::LogsServiceClient,
};
use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::any_value::Value as AnyVal;
use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::{AnyValue, KeyValue};
use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use setup::patrolli::{Patrolli, free_tcp_port};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;
use tonic::transport::Channel;
use uuid::Uuid;

/// A debug build starting on a busy machine.
const STARTUP_LIMIT: Duration = Duration::from_secs(60);
/// How long one phase may take to show its writes in the probes.
const PHASE_LIMIT: Duration = Duration::from_secs(30);
/// How long to wait for one round of probes to reach stdout before sending
/// the next round.
const ROUND_LIMIT: Duration = Duration::from_secs(5);

const OWNERS_CSV: &str = "\
hostname,team
web-1,payments
db-1,storage
";

const GEO_CSV: &str = "\
network,country
10.0.0.0/8,private-8
10.1.0.0/16,private-16
10.1.2.0/24,private-24
2001:db8::/32,doc-v6
";

fn config(port: u16, owners_csv: &Path, geo_csv: &Path) -> String {
    let [listener, host, item, collect, enrich, sink] = [(); 6].map(|_| Uuid::now_v7());
    let [owners, geo, iface, routes] = [(); 4].map(|_| Uuid::now_v7());
    let (owners_csv, geo_csv) = (owners_csv.display(), geo_csv.display());
    format!(
        r#"global:
  logger:
    level: info
    disable_color: true
  mib:
    disabled: true
  disk_channel:
    data_dir: ./data
lookup_tables:
  - id: {owners}
    name: owners
    source:
      type: file
      location:
        type: file_system
      paths: [{owners_csv}]
      format:
        type: csv
        has_headers: true
    key:
      type: hash
      key_columns: [hostname]
  - id: {geo}
    name: geo
    source:
      type: file
      location:
        type: file_system
      paths: [{geo_csv}]
      format:
        type: csv
        has_headers: true
    key:
      type: prefix
      prefix_column: network
  - id: {iface}
    name: iface
    source:
      type: processor
      ttl: 3600
      max_rows: 100
      columns:
        - name: ifIndex
          value_type: integer
        - name: ifDescr
    key:
      type: hash
      key_columns: [ifIndex]
  - id: {routes}
    name: routes
    source:
      type: processor
      ttl: 0
      max_rows: 100
      columns:
        - name: peer
        - name: prefix
        - name: origin_as
          value_type: integer
    key:
      type: prefix
      prefix_column: prefix
      partition_columns: [peer]
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
  - id: {collect}
    name: collect
    type: attribute_collect
    config:
      collections:
        - lookup_table: {iface}
          table: record
          when: {{match: {{table: record, key: iface.op, key_is_regex: false, op: "=", value: {{string: upsert}}}}}}
          columns:
            ifIndex: {{key: if.index, cast: integer}}
            ifDescr: {{key: if.descr.new}}
        - lookup_table: {iface}
          table: record
          action: delete
          when: {{match: {{table: record, key: iface.op, key_is_regex: false, op: "=", value: {{string: delete}}}}}}
          columns:
            ifIndex: {{key: if.index, cast: integer}}
        - lookup_table: {routes}
          table: record
          when: {{match: {{table: record, key: route.op, key_is_regex: false, op: "=", value: {{string: upsert}}}}}}
          columns:
            peer: {{key: peer}}
            prefix: {{key: route.new}}
            origin_as: {{key: route.origin_as, cast: integer}}
        - lookup_table: {routes}
          table: record
          action: delete
          when: {{match: {{table: record, key: route.op, key_is_regex: false, op: "=", value: {{string: delete}}}}}}
          columns:
            peer: {{key: peer}}
            prefix: {{key: route.new}}
        - lookup_table: {routes}
          table: record
          action: delete_partition
          when: {{match: {{table: record, key: route.op, key_is_regex: false, op: "=", value: {{string: peer_down}}}}}}
          columns:
            peer: {{key: peer}}
  - id: {enrich}
    name: enrich
    type: attribute_lookup
    inputs: [{collect}]
    config:
      lookups:
        - lookup_table: {owners}
          table: record
          match:
            keys:
              hostname: {{key: host.name}}
          output:
            owner.team: team
        - lookup_table: {geo}
          table: record
          match:
            point: {{key: src.ip}}
          output:
            geo.country: country
            geo.network: network
        - lookup_table: {iface}
          table: record
          match:
            keys:
              ifIndex: {{key: if.index, cast: integer}}
          output:
            if.descr: ifDescr
        - lookup_table: {routes}
          table: record
          match:
            keys:
              peer: {{key: peer}}
            point: {{key: dst.ip, cast: ip}}
          output:
            route.as: origin_as
            route.prefix: prefix
items:
  - type: listener
    id: {item}
    name: otlp-item
    host_id: {host}
    listener_id: {listener}
    processors: [{collect}]
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

fn i(key: &str, value: i64) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(AnyVal::IntValue(value)),
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

/// A probe record and what `enrich` must have made of it. `None` means the
/// attribute must be absent.
struct Probe {
    name: &'static str,
    attrs: Vec<KeyValue>,
    expect: Vec<(&'static str, Option<&'static str>)>,
}

fn probe(
    name: &'static str,
    attrs: Vec<KeyValue>,
    expect: &[(&'static str, Option<&'static str>)],
) -> Probe {
    Probe {
        name,
        attrs,
        expect: expect.to_vec(),
    }
}

/// The record attributes of every enriched log on stdout, as text (an int
/// attribute's JSON `intValue` already is), keyed by `probe.id`.
fn probes_seen(patrolli: &Patrolli) -> Result<BTreeMap<String, BTreeMap<String, String>>> {
    let mut seen = BTreeMap::new();
    for line in patrolli.stdout_lines() {
        if !line.starts_with(r#"{"kind":"logs""#) {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_str(&line).with_context(|| format!("not json: {line}"))?;
        let mut attrs = BTreeMap::new();
        for kv in value["record"]["attributes"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let key = kv["key"].as_str().context("attribute without a key")?;
            let v = &kv["value"];
            let text = v["stringValue"]
                .as_str()
                .or_else(|| v["intValue"].as_str())
                .map(str::to_string)
                .unwrap_or_else(|| v.to_string());
            attrs.insert(key.to_string(), text);
        }
        if let Some(id) = attrs.get("probe.id").cloned() {
            seen.insert(id, attrs);
        }
    }
    Ok(seen)
}

/// Sends rounds of `probes` until one round comes back enriched as expected.
async fn converge(
    patrolli: &Patrolli,
    client: &mut LogsServiceClient<Channel>,
    phase: &str,
    probes: &[Probe],
) {
    let deadline = tokio::time::Instant::now() + PHASE_LIMIT;
    let mut last_error = String::from("no round sent");
    for round in 0.. {
        if tokio::time::Instant::now() >= deadline {
            panic!("phase {phase:?} did not converge within {PHASE_LIMIT:?}: {last_error}");
        }
        let id = |p: &Probe| format!("{phase}/{round}/{}", p.name);
        let records = probes
            .iter()
            .map(|p| {
                let mut attrs = p.attrs.clone();
                attrs.push(s("probe.id", &id(p)));
                attrs
            })
            .collect();
        export(client, records).await;

        let round_deadline = tokio::time::Instant::now() + ROUND_LIMIT;
        let check = || -> Result<()> {
            let seen = probes_seen(patrolli)?;
            for p in probes {
                let Some(attrs) = seen.get(&id(p)) else {
                    bail!("probe {} not on stdout yet", id(p));
                };
                for (key, want) in &p.expect {
                    let got = attrs.get(*key).map(String::as_str);
                    if got != *want {
                        bail!("probe {}: {key} is {got:?}, want {want:?}", id(p));
                    }
                }
            }
            Ok(())
        };
        loop {
            match check() {
                Ok(()) => return,
                Err(e) => last_error = format!("{e:#}"),
            }
            // a round that arrived whole but wrong is stale: send the next one
            if !last_error.contains("not on stdout yet")
                || tokio::time::Instant::now() >= round_deadline
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

#[tokio::test]
async fn collect_and_lookup_through_hash_and_prefix_tables() {
    let csv_dir = tempfile::tempdir().unwrap();
    let owners_csv = csv_dir.path().join("owners.csv");
    let geo_csv = csv_dir.path().join("geo.csv");
    std::fs::write(&owners_csv, OWNERS_CSV).unwrap();
    std::fs::write(&geo_csv, GEO_CSV).unwrap();

    let port = free_tcp_port();
    let mut patrolli = Patrolli::start("lookup", &config(port, &owners_csv, &geo_csv), &[]);
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

    // ── phase 1: upserts into both processor tables
    let iface_up = |index: i64, descr: &str| {
        vec![
            s("iface.op", "upsert"),
            i("if.index", index),
            s("if.descr.new", descr),
        ]
    };
    let route = |op: &str, peer: &str, prefix: &str| {
        vec![s("route.op", op), s("peer", peer), s("route.new", prefix)]
    };
    let route_up = |peer: &str, prefix: &str, origin_as: i64| {
        let mut r = route("upsert", peer, prefix);
        r.push(i("route.origin_as", origin_as));
        r
    };
    export(
        &mut client,
        vec![
            iface_up(1, "eth0"),
            iface_up(2, "eth1"),
            route_up("a", "10.0.0.0/8", 100),
            route_up("a", "10.1.0.0/16", 200),
            route_up("a", "2001:db8::/32", 600),
            route_up("b", "10.0.0.0/8", 300),
        ],
    )
    .await;

    let ifp =
        |name, index: i64, descr| probe(name, vec![i("if.index", index)], &[("if.descr", descr)]);
    let rtp = |name, peer: &str, dst: &str, origin_as, prefix| {
        probe(
            name,
            vec![s("peer", peer), s("dst.ip", dst)],
            &[("route.as", origin_as), ("route.prefix", prefix)],
        )
    };
    // the file-backed tables, unaffected by any phase
    let file_probes = || {
        vec![
            probe(
                "web",
                vec![s("host.name", "web-1"), s("src.ip", "10.1.2.3")],
                &[
                    ("owner.team", Some("payments")),
                    ("geo.country", Some("private-24")),
                    ("geo.network", Some("10.1.2.0/24")),
                ],
            ),
            probe(
                "db",
                vec![s("host.name", "db-1"), s("src.ip", "10.9.9.9")],
                &[
                    ("owner.team", Some("storage")),
                    ("geo.country", Some("private-8")),
                    ("geo.network", Some("10.0.0.0/8")),
                ],
            ),
            probe(
                "mid",
                vec![s("src.ip", "10.1.200.1")],
                &[
                    ("geo.country", Some("private-16")),
                    ("geo.network", Some("10.1.0.0/16")),
                ],
            ),
            probe(
                "v6",
                vec![s("host.name", "nobody"), s("src.ip", "2001:db8::1")],
                &[
                    ("owner.team", None),
                    ("geo.country", Some("doc-v6")),
                    ("geo.network", Some("2001:db8::/32")),
                ],
            ),
            probe(
                "miss",
                vec![s("src.ip", "192.0.2.1")],
                &[("geo.country", None), ("geo.network", None)],
            ),
            probe(
                "not-an-ip",
                vec![s("src.ip", "nope")],
                &[("geo.country", None)],
            ),
        ]
    };

    let mut phase1 = file_probes();
    phase1.extend([
        ifp("if1", 1, Some("eth0")),
        ifp("if2", 2, Some("eth1")),
        ifp("if3", 3, None),
        rtp("a-16", "a", "10.1.2.3", Some("200"), Some("10.1.0.0/16")),
        rtp("a-8", "a", "10.9.0.1", Some("100"), Some("10.0.0.0/8")),
        rtp(
            "a-v6",
            "a",
            "2001:db8::5",
            Some("600"),
            Some("2001:db8::/32"),
        ),
        rtp("b", "b", "10.1.2.3", Some("300"), Some("10.0.0.0/8")),
        rtp("c", "c", "10.1.2.3", None, None),
        rtp("a-miss", "a", "11.0.0.1", None, None),
    ]);
    converge(&patrolli, &mut client, "upsert", &phase1).await;

    // ── phase 2: delete, replace and delete_partition
    export(
        &mut client,
        vec![
            vec![s("iface.op", "delete"), i("if.index", 1)],
            iface_up(2, "eth1-renamed"),
            route("delete", "a", "10.1.0.0/16"),
            // exact match only: no row has this key, and the /8 that
            // contains it must survive
            route("delete", "a", "10.0.0.0/9"),
            vec![s("route.op", "peer_down"), s("peer", "b")],
        ],
    )
    .await;

    let mut phase2 = file_probes();
    phase2.extend([
        ifp("if1", 1, None),
        ifp("if2", 2, Some("eth1-renamed")),
        // the /16 is gone, so the /8 under it answers
        rtp("a-16", "a", "10.1.2.3", Some("100"), Some("10.0.0.0/8")),
        rtp("a-8", "a", "10.9.0.1", Some("100"), Some("10.0.0.0/8")),
        rtp(
            "a-v6",
            "a",
            "2001:db8::5",
            Some("600"),
            Some("2001:db8::/32"),
        ),
        rtp("b", "b", "10.1.2.3", None, None),
    ]);
    converge(&patrolli, &mut client, "delete", &phase2).await;

    // ── phase 3: a dropped partition takes new routes again
    export(&mut client, vec![route_up("b", "10.1.0.0/16", 301)]).await;
    converge(
        &patrolli,
        &mut client,
        "repartition",
        &[
            rtp("b-16", "b", "10.1.2.3", Some("301"), Some("10.1.0.0/16")),
            rtp("b-8", "b", "10.9.0.1", None, None),
            rtp("a-16", "a", "10.1.2.3", Some("100"), Some("10.0.0.0/8")),
        ],
    )
    .await;

    patrolli.stop().await;
}
