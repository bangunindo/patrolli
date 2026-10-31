// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

// Cost of serializing one OTAP logs payload of `ROWS` records to JSON through
// `Payload::serialize_as`, the step the file and Kafka sinks run per batch. The
// payload is built once; only serialization is timed.
//
// Three record shapes, all through `payload::LogsBuilder`:
//   - `flat`:       syslog-like, scalar attributes only. No arrays, so the
//     nested modes only expand dotted object keys.
//   - `syslog_sd`:  the same plus RFC 5424 structured data laid out the way the
//     syslog listener pushes it (`data.N.id`, `data.N.params.M.{key,value}`),
//     and a 12-element array whose indices 10 and 11 sort before 2 as strings.
//   - `wide_array`: one attribute holding 200 scalars, like an SNMP walk's
//     `snmp.data`: the large-array end.
//   - `wide_object`: 48 distinct keys under one prefix, like an IPFIX flow's
//     `nf.data.*`: the large-object end.
//
// Each shape runs in both nested formats, and in `json_otlp_batch`, which keeps
// keys flat and serves as the control.
//
// Usage:
//   cargo bench --bench bench_json
//   cargo bench --bench bench_json -- syslog_sd

use std::borrow::Cow;
use std::net::{IpAddr, Ipv4Addr};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use opentelemetry_proto::tonic::logs::v1::SeverityNumber;
use smol_str::SmolStr;

use patrolli::config::Format;
use patrolli::payload::{LogRow, LogsBuilder, Payload, Scope, Value, ZString};

const ROWS: usize = 1000;
const SD_ELEMENTS: usize = 3;
const SD_PARAMS: usize = 4;
const FLAGS: usize = 12;
const WIDE: usize = 200;
const FIELDS: usize = 48;

const HOSTS: [&str; 4] = [
    "futurerobust.io",
    "chiefmarkets.net",
    "globalportals.org",
    "leadingschemas.com",
];
const SD_IDS: [&str; SD_ELEMENTS] = ["exampleSDID@32473", "origin", "meta"];
const SD_KEYS: [&str; SD_PARAMS] = ["iut", "eventSource", "eventID", "sequenceId"];

fn scope() -> Scope {
    Scope {
        name: SmolStr::new_static("bench"),
        listener_id: None,
        listener_type: Some(SmolStr::new_static("syslog")),
        listener_name: Some(SmolStr::new_static("syslog-1")),
        monitor_type: None,
        node_id: None,
    }
}

fn flat_attrs<'a>(i: usize, msgid: &'a str, attrs: &mut Vec<(ZString<'a>, Value<'a>)>) {
    attrs.push((
        ZString::from_static("protocol"),
        Value::String(ZString::from_ref("RFC5424")),
    ));
    attrs.push((
        ZString::from_static("host.name"),
        Value::String(ZString::from_ref(HOSTS[i % HOSTS.len()])),
    ));
    attrs.push((
        ZString::from_static("process.pid"),
        Value::Int64((i * 7919 % 10000) as i64),
    ));
    attrs.push((
        ZString::from_static("syslog.msgid"),
        Value::String(ZString::from_ref(msgid)),
    ));
    attrs.push((
        ZString::from_static("syslog.facility"),
        Value::Int64((i % 24) as i64),
    ));
}

fn structured_data<'a>(values: &'a [String]) -> Value<'a> {
    Value::Array(
        SD_IDS
            .iter()
            .map(|id| {
                let params = SD_KEYS
                    .iter()
                    .zip(values)
                    .map(|(k, v)| {
                        Value::KvList(vec![
                            (
                                ZString::from_ref("key"),
                                Value::String(ZString::from_ref(k)),
                            ),
                            (
                                ZString::from_ref("value"),
                                Value::String(ZString::from_ref(v)),
                            ),
                        ])
                    })
                    .collect();
                Value::KvList(vec![
                    (
                        ZString::from_ref("id"),
                        Value::String(ZString::from_ref(id)),
                    ),
                    (ZString::from_ref("params"), Value::Array(params)),
                ])
            })
            .collect(),
    )
}

fn build(shape: &str) -> Payload {
    let msgids: Vec<String> = (0..ROWS).map(|i| format!("ID{}", i % 997)).collect();
    let bodies: Vec<String> = (0..ROWS)
        .map(|i| format!("the redundant JSON program on port {i} is calculating"))
        .collect();
    let sd_values: Vec<String> = (0..SD_PARAMS).map(|i| format!("value-{i}")).collect();
    let fields: Vec<String> = (0..FIELDS).map(|f| format!("nf.data.field{f}")).collect();
    let src = IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3));

    let mut b = LogsBuilder::new();
    b.scope(scope());
    let mut attrs: Vec<(ZString, Value)> = Vec::with_capacity(16);
    for i in 0..ROWS {
        attrs.clear();
        flat_attrs(i, &msgids[i], &mut attrs);
        match shape {
            "flat" => {}
            "syslog_sd" => {
                attrs.push((ZString::from_static("data"), structured_data(&sd_values)));
                attrs.push((
                    ZString::from_static("flags"),
                    Value::Array(
                        (0..FLAGS)
                            .map(|f| Value::Int64(((i + f) % 2) as i64))
                            .collect(),
                    ),
                ));
            }
            "wide_array" => attrs.push((
                ZString::from_static("snmp.data"),
                Value::Array(
                    (0..WIDE)
                        .map(|w| Value::Int64((i * WIDE + w) as i64))
                        .collect(),
                ),
            )),
            "wide_object" => {
                for (f, key) in fields.iter().enumerate() {
                    attrs.push((
                        ZString::from_ref(key),
                        Value::Int64((i * FIELDS + f) as i64),
                    ));
                }
            }
            _ => unreachable!("unknown shape"),
        }
        b.push_row(LogRow {
            observed_time: Some(chrono::Utc::now()),
            severity: Some(SeverityNumber::Info),
            body: Some(Cow::Borrowed(&bodies[i])),
            source_ip: Some(src),
            source_port: Some(514),
            attrs: &attrs,
        });
    }
    b.finish()
}

fn bench(c: &mut Criterion) {
    for shape in ["flat", "syslog_sd", "wide_array", "wide_object"] {
        let payload = build(shape);
        let mut group = c.benchmark_group(format!("json/{shape}"));
        group.throughput(Throughput::Elements(ROWS as u64));
        for (name, format) in [
            ("nested_batch", Format::JsonNestedBatch),
            ("nested_denorm", Format::JsonNestedDenormalized),
            ("otlp_batch", Format::JsonOtlpBatch),
        ] {
            group.bench_function(name, |bench| {
                bench.iter(|| payload.serialize_as(format).unwrap())
            });
        }
        group.finish();
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
