// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

// Cost of building one OTAP logs payload through `payload::LogsBuilder`: push
// `ROWS` rows, then `finish`. This is the listener-side half of every pipeline,
// so it is measured on its own, away from transports and sinks.
//
// Two row shapes:
//   - `syslog`:  what the syslog listener pushes for an RFC 5424 line: a body,
//     a severity, a source address/port, and seven mostly-string attributes.
//   - `netflow`: a flow record: ~24 attributes, mostly integers, a few address
//     strings.
//
// And two metrics shapes through `payload::MetricsBuilder`, on the
// `push_row_owned` path the monitors and `internal_metrics` use:
//   - `prometheus`: one scrape, 110 families with labelled data points
//     (counters, gauges and 15-bucket histograms), 1000 data points in all.
//   - `one_dp_per_metric`: 1000 gauge rows of one data point each, the
//     row-heavy end.
//
// Usage:
//   cargo bench --bench bench_builder
//   cargo bench --bench bench_builder -- syslog

use std::borrow::Cow;
use std::net::{IpAddr, Ipv4Addr};

use chrono::{DateTime, Utc};
use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use opentelemetry_proto::tonic::logs::v1::SeverityNumber;
use otel_arrow_dfe_pdata::otlp::metrics::MetricType;
use otel_arrow_dfe_pdata::proto::opentelemetry::metrics::v1::AggregationTemporality;
use smol_str::SmolStr;
use uuid::Uuid;

use patrolli::payload::{
    DataPointsOwned, GaugeOwned, HistogramDataPointOwned, HistogramOwned, LogRow, LogsBuilder,
    MetricRowOwned, MetricsBuilder, NumberDataPointOwned, NumberValue, Resource, Scope, SumOwned,
    Value, ValueOwned, ZString, ZStringOwned,
};

const ROWS: usize = 1000;

const HOSTS: [&str; 8] = [
    "futurerobust.io",
    "dynamicinfomediaries.io",
    "futurearchitect.name",
    "chiefmarkets.net",
    "investoruser-centric.info",
    "seamlessmindshare.biz",
    "globalportals.org",
    "leadingschemas.com",
];
const APPS: [&str; 6] = ["ut", "aspernatur", "dignissimos", "est", "ipsam", "quia"];
const FACILITIES: [&str; 4] = ["local4", "daemon", "auth", "user"];
const SEVERITIES: [&str; 3] = ["info", "warning", "err"];

fn scope() -> Scope {
    Scope {
        name: SmolStr::new_static("syslog-1"),
        listener_id: Some(Uuid::now_v7()),
        listener_type: Some(SmolStr::new_static("syslog")),
        listener_name: Some(SmolStr::new_static("syslog-1")),
        monitor_type: None,
        node_id: None,
    }
}

struct SyslogInput {
    body: String,
    msgid: String,
    procid: i64,
}

fn syslog_inputs() -> Vec<SyslogInput> {
    (0..ROWS)
        .map(|i| SyslogInput {
            body: format!(
                "You can't calculate the port {i} without programming the redundant JSON program!"
            ),
            msgid: format!("ID{}", i % 997),
            procid: (i * 7919 % 10000) as i64,
        })
        .collect()
}

fn push_syslog(b: &mut LogsBuilder, inputs: &[SyslogInput]) {
    let src = IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3));
    let mut attrs: Vec<(ZString, Value)> = Vec::with_capacity(16);
    for (i, input) in inputs.iter().enumerate() {
        attrs.clear();
        attrs.push((
            ZString::from_static("protocol"),
            Value::String(ZString::from_ref("RFC5424")),
        ));
        attrs.push((
            ZString::from_static("facility_level"),
            Value::String(ZString::from_ref(FACILITIES[i % FACILITIES.len()])),
        ));
        attrs.push((
            ZString::from_static("severity_level"),
            Value::String(ZString::from_ref(SEVERITIES[i % SEVERITIES.len()])),
        ));
        attrs.push((
            ZString::from_static("hostname"),
            Value::String(ZString::from_ref(HOSTS[i % HOSTS.len()])),
        ));
        attrs.push((
            ZString::from_static("appname"),
            Value::String(ZString::from_ref(APPS[i % APPS.len()])),
        ));
        attrs.push((ZString::from_static("procid"), Value::Int64(input.procid)));
        attrs.push((
            ZString::from_static("msgid"),
            Value::String(ZString::from_ref(&input.msgid)),
        ));
        attrs.push((ZString::from_static("data"), Value::Array(Vec::new())));
        b.push_row(LogRow {
            observed_time: Some(chrono::Utc::now()),
            severity: Some(SeverityNumber::Info),
            body: Some(Cow::Borrowed(&input.body)),
            source_ip: Some(src),
            source_port: Some(40000 + (i % 8) as u16),
            attrs: &attrs,
        });
    }
}

const NF_INT_KEYS: [&str; 18] = [
    "nf.data.sourceTransportPort",
    "nf.data.destinationTransportPort",
    "nf.data.protocolIdentifier",
    "nf.data.tcpControlBits",
    "nf.data.ipClassOfService",
    "nf.data.ingressInterface",
    "nf.data.egressInterface",
    "nf.data.octetDeltaCount",
    "nf.data.packetDeltaCount",
    "nf.data.flowStartSysUpTime",
    "nf.data.flowEndSysUpTime",
    "nf.data.bgpSourceAsNumber",
    "nf.data.bgpDestinationAsNumber",
    "nf.data.sourceIPv4PrefixLength",
    "nf.data.destinationIPv4PrefixLength",
    "nf.data.flowDirection",
    "nf.version",
    "nf.sequence",
];

fn push_netflow(b: &mut LogsBuilder, src_ips: &[String], dst_ips: &[String]) {
    let exporter = IpAddr::V4(Ipv4Addr::new(192, 168, 88, 1));
    let mut attrs: Vec<(ZString, Value)> = Vec::with_capacity(32);
    for i in 0..ROWS {
        attrs.clear();
        attrs.push((
            ZString::from_static("nf.type"),
            Value::String(ZString::from_ref("ipfix")),
        ));
        attrs.push((ZString::from_static("nf.uptime"), Value::Double(i as f64)));
        for (k, key) in NF_INT_KEYS.iter().enumerate() {
            // A mix of low-cardinality (protocol, interfaces) and high-cardinality
            // (counters, ports) values.
            let v = if k < 7 {
                (i % (k + 3)) as i64
            } else {
                (i * 2654435761 % 1_000_003 + k) as i64
            };
            attrs.push((ZString::from_static(key), Value::Int64(v)));
        }
        attrs.push((
            ZString::from_static("nf.data.sourceIPv4Address"),
            Value::String(ZString::from_ref(&src_ips[i])),
        ));
        attrs.push((
            ZString::from_static("nf.data.destinationIPv4Address"),
            Value::String(ZString::from_ref(&dst_ips[i])),
        ));
        attrs.push((
            ZString::from_static("nf.data.ipNextHopIPv4Address"),
            Value::String(ZString::from_ref("192.168.88.254")),
        ));
        b.push_row(LogRow {
            observed_time: Some(chrono::Utc::now()),
            severity: None,
            body: None,
            source_ip: Some(exporter),
            source_port: Some(2055),
            attrs: &attrs,
        });
    }
}

fn monitor_scope() -> Scope {
    Scope {
        name: SmolStr::new_static("monitor"),
        listener_id: None,
        listener_type: None,
        listener_name: None,
        monitor_type: Some(SmolStr::new_static("prometheus_scrape")),
        node_id: Some(Uuid::now_v7()),
    }
}

fn monitor_resource() -> Resource {
    Resource {
        host_id: Uuid::now_v7(),
        host_name: SmolStr::new_static("node-exporter-1"),
        host_timezone: None,
        item_id: Uuid::now_v7(),
        item_name: SmolStr::new_static("node"),
        endpoint_id: Some(Uuid::now_v7()),
        endpoint_name: Some(SmolStr::new_static("metrics")),
        endpoint_type: Some(SmolStr::new_static("prometheus")),
    }
}

fn labels(pairs: &[(&str, String)]) -> Vec<(ZStringOwned, ValueOwned)> {
    pairs
        .iter()
        .map(|(k, v)| {
            (
                ZStringOwned::String(k.to_string()),
                ValueOwned::String(ZStringOwned::String(v.clone())),
            )
        })
        .collect()
}

fn number_dp(
    time: DateTime<Utc>,
    value: NumberValue,
    attrs: Vec<(ZStringOwned, ValueOwned)>,
) -> NumberDataPointOwned {
    NumberDataPointOwned {
        start_time: None,
        time,
        value,
        flags: 0,
        attrs,
        exemplars: Vec::new(),
    }
}

fn metric(name: String, r#type: MetricType, data_points: DataPointsOwned) -> MetricRowOwned {
    MetricRowOwned {
        name: ZStringOwned::String(name),
        r#type,
        description: Some(ZStringOwned::String(
            "Total number of requests by method and status code.".into(),
        )),
        unit: None,
        source_ip: None,
        source_port: None,
        data_points,
        attrs: Vec::new(),
    }
}

/// One scrape as `monitor::prometheus` converts it: 60 counters x 8, 40 gauges
/// x 10 and 10 histograms x 12 data points.
fn prometheus_rows() -> Vec<MetricRowOwned> {
    let t = Utc::now();
    let mut rows = Vec::new();
    for f in 0..60 {
        let dps = (0..8)
            .map(|i| {
                number_dp(
                    t,
                    NumberValue::Float((f * 1000 + i) as f64),
                    labels(&[
                        ("method", ["GET", "POST"][i % 2].into()),
                        ("code", ["200", "404", "500", "503"][i / 2].into()),
                        ("handler", format!("/api/v{}", f % 5)),
                    ]),
                )
            })
            .collect();
        rows.push(metric(
            format!("http_requests_{f}_total"),
            MetricType::Sum,
            DataPointsOwned::Sum(SumOwned {
                data_points: dps,
                aggregation_temporality: AggregationTemporality::Cumulative,
                is_monotonic: true,
            }),
        ));
    }
    for f in 0..40 {
        let dps = (0..10)
            .map(|i| {
                number_dp(
                    t,
                    NumberValue::Float(i as f64 * 0.5),
                    labels(&[("cpu", i.to_string()), ("mode", format!("mode{}", f % 8))]),
                )
            })
            .collect();
        rows.push(metric(
            format!("node_cpu_{f}_seconds"),
            MetricType::Gauge,
            DataPointsOwned::Gauge(GaugeOwned { data_points: dps }),
        ));
    }
    let bounds: Vec<f64> = (0..14).map(|b| 0.005 * 2f64.powi(b)).collect();
    for f in 0..10 {
        let dps = (0..12)
            .map(|i| HistogramDataPointOwned {
                start_time: None,
                time: t,
                count: 1000 + i,
                sum: Some(12.5 * i as f64),
                bucket_counts: (0..15).map(|b| b * i).collect(),
                explicit_bounds: bounds.clone(),
                flags: 0,
                min: None,
                max: None,
                attrs: labels(&[("le_group", format!("g{i}")), ("route", format!("/r{f}"))]),
                exemplars: Vec::new(),
            })
            .collect();
        rows.push(metric(
            format!("http_request_duration_{f}_seconds"),
            MetricType::Histogram,
            DataPointsOwned::Histogram(HistogramOwned {
                data_points: dps,
                aggregation_temporality: AggregationTemporality::Cumulative,
            }),
        ));
    }
    rows
}

fn one_dp_rows() -> Vec<MetricRowOwned> {
    let t = Utc::now();
    (0..ROWS)
        .map(|i| {
            metric(
                format!("ifHCInOctets_{}", i % 50),
                MetricType::Gauge,
                DataPointsOwned::Gauge(GaugeOwned {
                    data_points: vec![number_dp(
                        t,
                        NumberValue::Int(i as i64 * 7919),
                        labels(&[("ifIndex", (i % 48).to_string())]),
                    )],
                }),
            )
        })
        .collect()
}

fn bench_metrics(c: &mut Criterion) {
    let mut group = c.benchmark_group("metrics_builder");
    group.throughput(Throughput::Elements(ROWS as u64));
    for (name, rows) in [
        ("prometheus", prometheus_rows()),
        ("one_dp_per_metric", one_dp_rows()),
    ] {
        let (scope, resource) = (monitor_scope(), monitor_resource());
        let mut b = MetricsBuilder::new();
        group.bench_function(name, |bench| {
            bench.iter_batched(
                || rows.clone(),
                |rows| {
                    b.scope(scope.clone());
                    b.resource(resource.clone());
                    for row in rows {
                        b.push_row_owned(row);
                    }
                    b.finish()
                },
                BatchSize::LargeInput,
            )
        });
    }
    group.finish();
}

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("logs_builder");
    group.throughput(Throughput::Elements(ROWS as u64));

    let inputs = syslog_inputs();
    let mut b = LogsBuilder::new();
    b.scope(scope());
    group.bench_function("syslog", |bench| {
        bench.iter(|| {
            push_syslog(&mut b, &inputs);
            let pl = b.finish();
            b.scope(scope());
            pl
        })
    });

    let src_ips: Vec<String> = (0..ROWS)
        .map(|i| format!("10.{}.{}.{}", i % 7, i % 251, i % 13))
        .collect();
    let dst_ips: Vec<String> = (0..ROWS)
        .map(|i| format!("172.16.{}.{}", i % 31, i % 199))
        .collect();
    let mut b = LogsBuilder::new();
    b.scope(scope());
    group.bench_function("netflow", |bench| {
        bench.iter(|| {
            push_netflow(&mut b, &src_ips, &dst_ips);
            let pl = b.finish();
            b.scope(scope());
            pl
        })
    });

    group.finish();
}

criterion_group!(benches, bench, bench_metrics);
criterion_main!(benches);
