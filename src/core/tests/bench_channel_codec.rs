// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Disk channel codec breakdown, no Docker needed.
//!
//! `BENCH_SHAPE=syslog` (default) has a syslog listener parse generated RFC
//! 5424 lines into full batches; `metrics` builds SNMP-style interface
//! counters and `traces` HTTP spans with events. Each payload is then encoded
//! by the disk channel codec and by the OTAP transport encoding (postcard
//! aside, what disk channels stored before `chan::codec`), timed stage by
//! stage. Not an assertion. Run it in release:
//!
//! ```bash
//! cargo test --release --test bench_channel_codec -- --ignored --nocapture
//! BENCH_BATCH=1000 BENCH_PAYLOADS=40 cargo test --release --test bench_channel_codec -- --ignored --nocapture
//! BENCH_SHAPE=metrics cargo test --release --test bench_channel_codec -- --ignored --nocapture
//! ```
//!
//! `BENCH_BATCH` counts items: log records, data points or spans.

use arc_swap::ArcSwap;
use arrow::array::RecordBatch;
use bytes::Bytes;
use otel_arrow_dfe_pdata::encode::producer::ProducerOptions;
use otel_arrow_dfe_pdata::otap::from_record_messages;
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::BatchArrowRecords;
use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::{AnyValue, KeyValue, any_value};
use otel_arrow_dfe_pdata::proto::opentelemetry::resource::v1::Resource;
use otel_arrow_dfe_pdata::{Consumer, OtapArrowRecords, Producer};
use patrolli::common::internal_metrics;
use patrolli::listener::WorkerGroup;
use patrolli::payload::{Payload, ProcessorPayload};
use patrolli::processor::PROCESSOR_CHANNEL_NAME;
use patrolli::{cache, chan, config};
use prost::Message;
use std::hint::black_box;
use std::io::Write;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

fn env<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// xorshift32, so runs are reproducible.
struct Rng(u32);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
    fn hex(&mut self, bytes: usize) -> String {
        (0..bytes)
            .map(|_| format!("{:02x}", self.below(256)))
            .collect()
    }
}

/// Mixed sshd / firewall / nginx / app lines, ~195 bytes on average, with
/// random ids so zstd sees roughly real-world ratios rather than templates.
fn lines(n: usize) -> Vec<u8> {
    let mut r = Rng(0x1234_5678);
    let words = [
        "connection",
        "reset",
        "timeout",
        "accepted",
        "denied",
        "session",
        "opened",
        "closed",
        "policy",
        "rule",
        "matched",
        "upstream",
        "backend",
        "latency",
    ];
    let mut out = Vec::new();
    for _ in 0..n {
        let host = format!(
            "{}-{:02}",
            ["fw", "sw", "rtr", "web", "db"][r.below(5) as usize],
            r.below(20)
        );
        let ip = format!("10.{}.{}.{}", r.below(256), r.below(256), r.below(256));
        let ts = format!(
            "2026-10-08T09:{:02}:{:02}.{:06}Z",
            r.below(60),
            r.below(60),
            r.below(1_000_000)
        );
        let msg = match r.below(4) {
            0 => format!(
                "sshd {} - - Accepted publickey for user{} from {ip} port {} ssh2: RSA SHA256:{}",
                1 + r.below(65534),
                r.below(500),
                1024 + r.below(64000),
                r.hex(16)
            ),
            1 => format!(
                "filterlog {} - - {},,,{},igb{},match,{},in,4,0x0,,64,{},0,DF,6,tcp,60,{ip},192.168.{}.{},{},{},0,S,{},,64240,,mss;sackOK;TS;nop;wscale",
                1 + r.below(65534),
                1 + r.below(199),
                r.hex(4),
                r.below(4),
                ["pass", "block"][r.below(2) as usize],
                r.below(65535),
                r.below(256),
                r.below(256),
                1024 + r.below(64000),
                [22, 80, 443, 3306, 5432][r.below(5) as usize],
                r.next()
            ),
            2 => format!(
                "nginx {} - - {ip} - - \"GET /api/v1/items/{}?q={} HTTP/1.1\" {} {} \"-\" \"Mozilla/5.0 (X11; Linux x86_64)\" rt=0.{:03}",
                1 + r.below(65534),
                r.below(1_000_000),
                r.hex(6),
                [200, 200, 200, 304, 404, 500][r.below(6) as usize],
                r.below(100_000),
                r.below(1000)
            ),
            _ => format!(
                "app {} - [meta trace_id=\"{}\" span_id=\"{}\"] {} duration_ms={} user=user{}",
                1 + r.below(65534),
                r.hex(16),
                r.hex(8),
                (0..8)
                    .map(|_| words[r.below(words.len() as u32) as usize])
                    .collect::<Vec<_>>()
                    .join(" "),
                r.below(5000),
                r.below(500)
            ),
        };
        out.extend_from_slice(format!("<{}>1 {ts} {host} {msg}\n", 8 + r.below(184)).as_bytes());
    }
    out
}

fn kv(key: &str, value: any_value::Value) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: Some(AnyValue { value: Some(value) }),
    }
}

fn str_kv(key: &str, value: String) -> KeyValue {
    kv(key, any_value::Value::StringValue(value))
}

fn int_kv(key: &str, value: i64) -> KeyValue {
    kv(key, any_value::Value::IntValue(value))
}

fn device(r: &mut Rng, seed: usize) -> Option<Resource> {
    Some(Resource {
        attributes: vec![
            str_kv("host.name", format!("rtr-{seed:03}")),
            str_kv("host.id", Uuid::now_v7().to_string()),
            str_kv(
                "net.host.ip",
                format!("10.0.{}.{}", r.below(256), r.below(256)),
            ),
        ],
        ..Default::default()
    })
}

/// SNMP-style interface metrics: counters and gauges per interface, with
/// ifIndex / ifName / ifAlias attributes, `batch` data points per payload.
fn metrics_payload(batch: usize, seed: usize) -> ProcessorPayload {
    use otel_arrow_dfe_pdata::proto::opentelemetry::metrics::v1::{
        Gauge, Metric, MetricsData, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum, metric,
        number_data_point,
    };
    let mut r = Rng(0x9e37_79b9 ^ seed as u32);
    let names = [
        ("ifHCInOctets", true),
        ("ifHCOutOctets", true),
        ("ifInErrors", true),
        ("ifOutDiscards", true),
        ("ifOperStatus", false),
        ("ifHighSpeed", false),
        ("cpu.utilization", false),
        ("mem.used", false),
    ];
    let per_metric = batch / names.len();
    let metrics = names
        .iter()
        .map(|(name, counter)| {
            let points = (0..per_metric)
                .map(|i| NumberDataPoint {
                    attributes: vec![
                        int_kv("ifIndex", i as i64),
                        str_kv("ifName", format!("GigabitEthernet0/{}/{}", i / 48, i % 48)),
                        str_kv("ifAlias", format!("uplink-{}-{}", r.hex(3), i % 7)),
                    ],
                    start_time_unix_nano: 1_791_000_000_000_000_000,
                    time_unix_nano: 1_791_000_000_000_000_000
                        + u64::from(r.below(60_000)) * 1_000_000,
                    value: Some(if *counter {
                        number_data_point::Value::AsInt(i64::from(r.next()) << 8)
                    } else {
                        number_data_point::Value::AsDouble(f64::from(r.below(100_000)) / 100.0)
                    }),
                    ..Default::default()
                })
                .collect();
            Metric {
                name: (*name).into(),
                unit: if *counter { "By".into() } else { "1".into() },
                data: Some(if *counter {
                    metric::Data::Sum(Sum {
                        data_points: points,
                        aggregation_temporality: 2,
                        is_monotonic: true,
                    })
                } else {
                    metric::Data::Gauge(Gauge {
                        data_points: points,
                    })
                }),
                ..Default::default()
            }
        })
        .collect();
    let data = MetricsData {
        resource_metrics: vec![ResourceMetrics {
            resource: device(&mut r, seed),
            scope_metrics: vec![ScopeMetrics {
                metrics,
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    ProcessorPayload::new(
        None,
        Payload::new(otel_arrow_dfe_pdata::encode::encode_metrics_otap_batch(&data).unwrap()),
    )
}

/// HTTP server spans with route / status / peer attributes, an event on
/// every fourth, `batch` spans per payload.
fn traces_payload(batch: usize, seed: usize) -> ProcessorPayload {
    use otel_arrow_dfe_pdata::proto::opentelemetry::trace::v1::{
        ResourceSpans, ScopeSpans, Span, TracesData, span,
    };
    let mut r = Rng(0x85eb_ca6b ^ seed as u32);
    let routes = [
        "/api/v1/items/{id}",
        "/api/v1/users/{id}",
        "/healthz",
        "/api/v1/search",
    ];
    let spans = (0..batch)
        .map(|i| {
            let start = 1_791_000_000_000_000_000 + u64::from(r.next());
            let route = routes[r.below(routes.len() as u32) as usize];
            Span {
                trace_id: (0..16).map(|_| r.below(256) as u8).collect(),
                span_id: (0..8).map(|_| r.below(256) as u8).collect(),
                name: format!("GET {route}"),
                kind: 2,
                start_time_unix_nano: start,
                end_time_unix_nano: start + u64::from(r.below(50_000_000)),
                attributes: vec![
                    str_kv("http.request.method", "GET".into()),
                    str_kv("http.route", route.into()),
                    int_kv(
                        "http.response.status_code",
                        [200, 200, 200, 404, 500][r.below(5) as usize],
                    ),
                    str_kv(
                        "client.address",
                        format!("10.{}.{}.{}", r.below(256), r.below(256), r.below(256)),
                    ),
                    str_kv("user.id", format!("user{}", r.below(500))),
                ],
                events: if i % 4 == 0 {
                    vec![span::Event {
                        time_unix_nano: start + 1_000,
                        name: "cache.miss".into(),
                        attributes: vec![str_kv("cache.key", r.hex(8))],
                        ..Default::default()
                    }]
                } else {
                    vec![]
                },
                ..Default::default()
            }
        })
        .collect();
    let data = TracesData {
        resource_spans: vec![ResourceSpans {
            resource: device(&mut r, seed),
            scope_spans: vec![ScopeSpans {
                spans,
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    ProcessorPayload::new(
        None,
        Payload::new(otel_arrow_dfe_pdata::encode::encode_spans_otap_batch(&data).unwrap()),
    )
}

/// Every record of `listener_id` belongs to one item.
fn item_for(listener_id: Uuid) -> cache::Cache {
    let cache = cache::Cache::new();
    let host_id = Uuid::now_v7();
    cache.host.write().insert(config::host::Host {
        id: host_id,
        name: "host".into(),
        timezone: None,
    });
    let item = config::item::Item::new(
        Uuid::now_v7(),
        "syslog-item".into(),
        None,
        host_id,
        config::item::Schedule::Never,
        Some(listener_id),
        Duration::from_secs(1),
        vec![Uuid::now_v7()],
        config::item::Config::Listener(config::item::Listener {
            match_strategy: config::item::ListenerItemMatchStrategy::None,
        }),
    );
    cache.item.write().insert(item.clone());
    cache.listener.write().insert_item(item, None);
    cache
}

/// `count` full payloads of `batch` records, straight off a syslog listener.
fn collect_payloads(batch: usize, count: usize) -> Vec<ProcessorPayload> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async move {
        let addr = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let (admin, sender) = chan::new::<&'static str, ProcessorPayload>(
            Duration::from_secs(3600),
            Arc::new(ArcSwap::from_pointee((
                Instant::now(),
                internal_metrics::Channel::default(),
            ))),
        );
        let rx = admin
            .get_receiver(
                &PROCESSOR_CHANNEL_NAME,
                config::Channel::Memory(config::MemoryChannel {
                    non_blocking: false,
                    channel_buffer_size: count + 1,
                }),
            )
            .await
            .unwrap();
        let listener_id = Uuid::now_v7();
        let conf = config::listener::Listener::new(
            listener_id,
            "syslog-codec".into(),
            1,
            config::Batch {
                batch_size: batch,
                batch_timeout: Duration::from_secs(30),
            },
            config::listener::Config::Syslog(Box::new(config::listener::Syslog {
                listener: config::listener::SyslogListener {
                    udp: None,
                    tcp: Some(config::listener::TcpListener {
                        address: addr,
                        ip_whitelist: Vec::new(),
                        ip_blacklist: Vec::new(),
                    }),
                    tls: None,
                    idle_timeout: Duration::from_secs(60),
                },
                cise: Default::default(),
                cef: Default::default(),
                cisco: Default::default(),
            })),
            false,
        );
        let group = WorkerGroup::new(sender, None, item_for(listener_id), None);
        group.add(conf).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let data = lines(batch * count);
        std::thread::spawn(move || {
            let mut s = TcpStream::connect(addr).unwrap();
            s.write_all(&data).unwrap();
            s.flush().unwrap();
            std::thread::sleep(Duration::from_secs(60));
        });
        let mut out = Vec::new();
        while out.len() < count {
            let pl = tokio::time::timeout(Duration::from_secs(60), rx.recv_async())
                .await
                .expect("payloads timed out")
                .unwrap();
            assert_eq!(pl.payload.num_rows(), batch, "expected full batches");
            out.push(pl);
        }
        out
    })
}

/// Mean time of `f` over `payloads`, `rounds` times each, in µs per payload.
fn time<T>(payloads: usize, rounds: usize, mut f: impl FnMut(usize) -> T) -> f64 {
    for i in 0..payloads {
        black_box(f(i));
    }
    let start = Instant::now();
    for _ in 0..rounds {
        for i in 0..payloads {
            black_box(f(i));
        }
    }
    start.elapsed().as_secs_f64() * 1e6 / (rounds * payloads) as f64
}

fn zstd(data: &[u8]) -> Vec<u8> {
    zstd::encode_all(data, 0).unwrap()
}

/// The OTAP transport encoding of a payload: one `BatchArrowRecords`.
fn otap_encode(p: &ProcessorPayload) -> Vec<u8> {
    let mut data = p.payload.data.clone();
    Producer::new_with_options(ProducerOptions {
        ipc_compression: None,
    })
    .produce_bar(&mut data)
    .unwrap()
    .encode_to_vec()
}

/// Back from [`otap_encode`]; `like` says which signal it carries.
fn otap_decode(buf: &[u8], like: &ProcessorPayload) -> OtapArrowRecords {
    let mut bar = BatchArrowRecords::decode(buf).unwrap();
    let msgs = Consumer::default().consume_bar(&mut bar).unwrap();
    let mut data = match &like.payload.data {
        OtapArrowRecords::Logs(_) => OtapArrowRecords::Logs(from_record_messages(msgs).unwrap()),
        OtapArrowRecords::Metrics(_) => {
            OtapArrowRecords::Metrics(from_record_messages(msgs).unwrap())
        }
        OtapArrowRecords::Traces(_) => {
            OtapArrowRecords::Traces(from_record_messages(msgs).unwrap())
        }
    };
    data.decode_transport_optimized_ids().unwrap();
    data
}

/// `value` with every array sorted: the transport encoding reorders data
/// points, events and attributes, never what is inside a value.
fn sorted_deep(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Array(items) => {
            let mut items: Vec<Value> = items.into_iter().map(sorted_deep).collect();
            items.sort_by_cached_key(Value::to_string);
            Value::Array(items)
        }
        Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, sorted_deep(v))).collect())
        }
        other => other,
    }
}

/// A payload's records as JSON with object keys sorted (serde_json's map is
/// ordered), in a stable order: equal for the same records whatever the
/// order of their attributes.
fn normalized(p: &Payload) -> Vec<String> {
    let mut records: Vec<String> = p
        .serialize_as(config::Format::JsonNestedDenormalized)
        .unwrap()
        .iter()
        .map(|b| sorted_deep(serde_json::from_slice(b).unwrap()).to_string())
        .collect();
    records.sort();
    records
}

fn pinned(rb: &RecordBatch) -> usize {
    rb.get_array_memory_size()
}

#[test]
#[ignore = "throughput benchmark: run in release with --ignored --nocapture"]
fn bench_channel_codec() {
    let batch: usize = env("BENCH_BATCH", 10_000);
    let count: usize = env("BENCH_PAYLOADS", 8);
    let rounds: usize = env("BENCH_ROUNDS", 5);
    let shape = std::env::var("BENCH_SHAPE").unwrap_or_else(|_| "syslog".into());
    let payloads: Vec<ProcessorPayload> = match shape.as_str() {
        "syslog" => collect_payloads(batch, count),
        "metrics" => (0..count).map(|i| metrics_payload(batch, i)).collect(),
        "traces" => (0..count).map(|i| traces_payload(batch, i)).collect(),
        s => panic!("unknown BENCH_SHAPE {s}"),
    };
    let batch = payloads[0].payload.num_items();
    let records = (batch * count) as f64;
    let n = payloads.len();
    let per_rec = |us_per_payload: f64| us_per_payload / batch as f64;

    let first = &payloads[0].payload.data;
    println!("\n== {count} payloads x {batch} {shape} items ==");
    for &pt in first.allowed_payload_types() {
        if let Some(rb) = first.get(pt) {
            println!(
                "  {:?}: {} rows x {} cols, {} KiB in memory",
                pt,
                rb.num_rows(),
                rb.num_columns(),
                pinned(rb) / 1024
            );
        }
    }
    let arrow_mem: usize = payloads
        .iter()
        .map(|p| p.payload.data.retained_memory_bytes())
        .sum();
    println!(
        "  retained Arrow memory: {:.0} B/record",
        arrow_mem as f64 / records
    );

    let mut rows: Vec<(String, f64, f64, f64)> = Vec::new(); // name, enc µs/rec, dec µs/rec, bytes/rec

    // ---- the OTAP transport encoding (what the cluster forwarder sends), stage by stage ----
    let opt_us = time(n, rounds, |i| {
        let mut d = payloads[i].payload.data.clone();
        d.encode_transport_optimized().unwrap();
        d
    });
    let optimized: Vec<OtapArrowRecords> = payloads
        .iter()
        .map(|p| {
            let mut d = p.payload.data.clone();
            d.encode_transport_optimized().unwrap();
            d
        })
        .collect();
    let bar_us = time(n, rounds, |i| {
        let mut d = optimized[i].clone();
        Producer::new_with_options(ProducerOptions {
            ipc_compression: None,
        })
        .produce_bar(&mut d)
        .unwrap()
    });
    let bars: Vec<BatchArrowRecords> = optimized
        .iter()
        .map(|d| {
            Producer::new_with_options(ProducerOptions {
                ipc_compression: None,
            })
            .produce_bar(&mut d.clone())
            .unwrap()
        })
        .collect();
    let proto_us = time(n, rounds, |i| bars[i].encode_to_vec());
    let otap_bytes: Vec<Vec<u8>> = payloads.iter().map(otap_encode).collect();
    let enc_total_us = time(n, rounds, |i| otap_encode(&payloads[i]));
    let zstd_us = time(n, rounds, |i| zstd(&otap_bytes[i]));
    let lz4_us = time(n, rounds, |i| {
        lz4_flex::compress_prepend_size(&otap_bytes[i])
    });
    let zstd_bytes: Vec<Vec<u8>> = otap_bytes.iter().map(|b| zstd(b)).collect();
    let lz4_bytes: Vec<Vec<u8>> = otap_bytes
        .iter()
        .map(|b| lz4_flex::compress_prepend_size(b))
        .collect();
    let unzstd_us = time(n, rounds, |i| zstd::decode_all(&zstd_bytes[i][..]).unwrap());
    let unlz4_us = time(n, rounds, |i| {
        lz4_flex::decompress_size_prepended(&lz4_bytes[i]).unwrap()
    });
    let dec_total_us = time(n, rounds, |i| otap_decode(&otap_bytes[i], &payloads[i]));
    let unproto_us = time(n, rounds, |i| {
        BatchArrowRecords::decode(&otap_bytes[i][..]).unwrap()
    });
    let consume_us = time(n, rounds, |i| {
        let mut bar = bars[i].clone();
        Consumer::default().consume_bar(&mut bar).unwrap()
    });
    let unopt_us = time(n, rounds, |i| {
        let mut d = optimized[i].clone();
        d.decode_transport_optimized_ids().unwrap();
        d
    });
    let raw_len: f64 = otap_bytes.iter().map(Vec::len).sum::<usize>() as f64 / records;
    let zstd_len: f64 = zstd_bytes.iter().map(Vec::len).sum::<usize>() as f64 / records;
    let lz4_len: f64 = lz4_bytes.iter().map(Vec::len).sum::<usize>() as f64 / records;

    println!("\n-- OTAP transport encoding --  (µs/record)");
    println!("  encode_transport_optimized   {:.3}", per_rec(opt_us));
    println!("  produce_bar (IPC, after opt) {:.3}", per_rec(bar_us));
    println!("  prost encode                 {:.3}", per_rec(proto_us));
    println!(
        "  -> encode total              {:.3}",
        per_rec(enc_total_us)
    );
    println!("  + zstd                       {:.3}", per_rec(zstd_us));
    println!("  + lz4                        {:.3}", per_rec(lz4_us));
    println!(
        "  decode: unzstd {:.3}  unlz4 {:.3}",
        per_rec(unzstd_us),
        per_rec(unlz4_us)
    );
    println!(
        "  decode: prost {:.3}  consume_bar {:.3}  decode_ids {:.3}",
        per_rec(unproto_us),
        per_rec(consume_us),
        per_rec(unopt_us)
    );
    println!(
        "  -> decode total              {:.3}",
        per_rec(dec_total_us)
    );
    rows.push((
        "otap none".into(),
        per_rec(enc_total_us),
        per_rec(dec_total_us),
        raw_len,
    ));
    rows.push((
        "otap zstd".into(),
        per_rec(enc_total_us + zstd_us),
        per_rec(dec_total_us + unzstd_us),
        zstd_len,
    ));
    rows.push((
        "otap lz4".into(),
        per_rec(enc_total_us + lz4_us),
        per_rec(dec_total_us + unlz4_us),
        lz4_len,
    ));

    // ---- the disk channel codec, plain and `transport_optimized` ----
    use patrolli::chan::codec;
    for transport in [false, true] {
        for comp in [
            config::OtapCompression::None,
            config::OtapCompression::Lz4,
            config::OtapCompression::Zstd,
        ] {
            let enc_us = time(n, rounds, |i| {
                codec::encode(&payloads[i], comp, transport).unwrap()
            });
            let encoded: Vec<Bytes> = payloads
                .iter()
                .map(|p| codec::encode(p, comp, transport).unwrap())
                .collect();
            let dec_us = time(n, rounds, |i| {
                codec::decode::<ProcessorPayload>(encoded[i].clone()).unwrap()
            });
            for (p, e) in payloads.iter().zip(&encoded) {
                let back: ProcessorPayload = codec::decode(e.clone()).unwrap();
                if transport {
                    // Attributes come back reordered: the same records.
                    assert_eq!(
                        normalized(&back.payload),
                        normalized(&p.payload),
                        "{comp:?} transport round trip"
                    );
                } else {
                    assert_eq!(back.payload.data, p.payload.data, "{comp:?} round trip");
                }
            }
            let len = encoded.iter().map(Bytes::len).sum::<usize>() as f64 / records;
            let name = if transport { "codec+T" } else { "codec" };
            rows.push((
                format!("{name} {comp:?}"),
                per_rec(enc_us),
                per_rec(dec_us),
                len,
            ));
        }
    }

    // What the sink does next: does the round trip change its cost?
    let fmt = config::Format::JsonNestedDenormalized;
    let trip_otap: Vec<OtapArrowRecords> = otap_bytes
        .iter()
        .zip(&payloads)
        .map(|(b, p)| otap_decode(b, p))
        .collect();
    let trip_new: Vec<ProcessorPayload> = payloads
        .iter()
        .map(|p| {
            let comp = config::OtapCompression::Zstd;
            codec::decode(codec::encode(p, comp, false).unwrap()).unwrap()
        })
        .collect();
    let json_orig = time(n, rounds, |i| {
        payloads[i].payload.serialize_as(fmt).unwrap()
    });
    let trip_otap: Vec<Payload> = trip_otap.into_iter().map(Payload::new).collect();
    let json_otap = time(n, rounds, |i| trip_otap[i].serialize_as(fmt).unwrap());
    let json_new = time(n, rounds, |i| {
        trip_new[i].payload.serialize_as(fmt).unwrap()
    });
    let same = |trip: &[&Payload]| {
        payloads
            .iter()
            .zip(trip)
            .all(|(a, b)| a.payload.serialize_as(fmt).unwrap() == b.serialize_as(fmt).unwrap())
    };
    println!(
        "\n-- sink json_nested_denormalized after the trip (µs/record) --\n  original {:.3}  after otap {:.3}  after codec {:.3}",
        per_rec(json_orig),
        per_rec(json_otap),
        per_rec(json_new)
    );
    println!(
        "  identical json: otap {}  codec {}",
        same(&trip_otap.iter().collect::<Vec<_>>()),
        same(&trip_new.iter().map(|p| &p.payload).collect::<Vec<_>>())
    );

    println!("\n-- summary (µs/record; bytes/record as stored) --");
    println!(
        "  {:<20} {:>8} {:>8} {:>8} {:>9}",
        "codec", "encode", "decode", "total", "bytes"
    );
    for (name, e, d, len) in rows {
        println!("  {name:<20} {e:>8.3} {d:>8.3} {:>8.3} {len:>9.1}", e + d);
    }
}
