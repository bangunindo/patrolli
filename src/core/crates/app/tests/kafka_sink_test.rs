// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! End-to-end tests for the Kafka sink against real brokers.
//!
//! The sink is mostly glue between three things that each have their own idea of a "record":
//! patrolli payloads (items), the serializer (one blob per payload or per record), and librdkafka
//! (messages, acknowledged asynchronously through a delivery callback). Only a broker can show
//! that they agree, so every test here reads the topic back and checks it against the sink's own
//! counters: what `total_records_sent` claims must be what a consumer actually finds.
//!
//! Every scenario runs twice, against Apache Kafka (`apache::*`) and Redpanda (`redpanda::*`),
//! the Kafka-protocol broker deployments most often swap in. Filter by either prefix:
//! ```text
//! cargo test --test kafka_sink_test redpanda::
//! ```
//!
//! Like the other tests here, these need a running Docker daemon. Each test starts its own
//! broker; topics are created explicitly because both brokers run with auto-creation off.

mod setup;

use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::{
    AnyValue, InstrumentationScope, KeyValue, any_value::Value,
};
use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
    LogRecord, LogsData, ResourceLogs, ScopeLogs,
};
use otel_arrow_dfe_pdata::proto::opentelemetry::metrics::v1::MetricsData;
use otel_arrow_dfe_pdata::proto::opentelemetry::resource::v1::Resource;
use otel_arrow_dfe_pdata::proto::opentelemetry::trace::v1::TracesData;
use patrolli::common::internal_metrics;
use patrolli::config::Format;
use patrolli::payload::Payload;
use patrolli::sink::Sink as _;
use patrolli::sink::kafka::KafkaSink;
use prost::Message as _;
use rustc_hash::FxHashMap;
use setup::kafka::{Broker, Consumed, Flavor};
use std::sync::Arc;
use std::time::Duration;

const PARTITIONS: i32 = 3;

/// Run every listed scenario once per broker flavor, as `apache::<name>` and `redpanda::<name>`.
macro_rules! on_every_broker {
    ($($name:ident),* $(,)?) => {
        mod apache {
            $(
                #[tokio::test]
                async fn $name() {
                    super::$name(super::Flavor::Apache).await
                }
            )*
        }
        mod redpanda {
            $(
                #[tokio::test]
                async fn $name() {
                    super::$name(super::Flavor::Redpanda).await
                }
            )*
        }
    };
}

on_every_broker!(
    every_signal_round_trips_as_otlp_protobuf,
    each_key_strategy_shards_records_and_drops_the_keyless,
    every_compression_codec_round_trips,
    a_long_batch_timeout_is_cut_short_when_the_input_ends,
    concurrent_workers_and_producers_deliver_everything,
    a_missing_topic_counts_every_record_as_errored,
    a_broker_outage_is_retried_until_it_comes_back,
);

// ---------------------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------------------

async fn start(flavor: Flavor) -> Broker {
    setup::setup_logging();
    Broker::start_flavor(flavor)
        .await
        .unwrap_or_else(|e| panic!("start {flavor:?}: {e:#}"))
}

/// A fresh topic, so sub-cases sharing one broker never read each other's messages.
async fn topic(broker: &Broker, label: &str) -> String {
    let name = format!("patrolli-test-{label}-{}", uuid::Uuid::now_v7());
    broker
        .create_topic(&name, PARTITIONS)
        .await
        .unwrap_or_else(|e| panic!("create topic {name}: {e:#}"));
    name
}

/// A one-worker, one-producer sink with a 10ms linger and `acks: all`, so a successful delivery
/// report means the broker has the message. `config` is appended under `config:`, indented two
/// spaces, and must at least set `format`.
fn sink_config(broker: &Broker, topic: &str, config: &str) -> patrolli::config::sink::Sink {
    let yaml = format!(
        r#"
id: 00000000-0000-0000-0000-0000000000fe
name: kafka-e2e
worker_count: 1
batch_size: 10000
batch_timeout: 10
type: kafka
config:
  hosts:
    - {bootstrap}
  topic: {topic}
  required_acks: all
  security:
    protocol: plaintext
{config}
"#,
        bootstrap = broker.bootstrap,
    );
    let mut config: patrolli::config::sink::Sink =
        serde_yaml_ng::from_str(&yaml).unwrap_or_else(|e| panic!("sink config yaml: {e}\n{yaml}"));
    config.validate().expect("sink config validate");
    config
}

/// Feed `payloads` through a real sink and return its metrics once it has fully closed.
///
/// The sender is dropped before the workers start, so they drain the channel, see the
/// disconnect and return; `close` then flushes the producers. That is the production shutdown
/// order, so whatever the sink still buffers when its input ends is on the broker afterwards.
async fn run_sink(
    config: patrolli::config::sink::Sink,
    payloads: Vec<Payload>,
) -> internal_metrics::Sink {
    let workers = config.worker_count;
    let (tx, rx) = flume::bounded(payloads.len().max(1));
    for pl in payloads {
        tx.send_async(pl).await.expect("send payload");
    }
    let rx = patrolli::chan::Consumer::from_flume(&tx, rx);
    drop(tx);

    let sink = Arc::new(KafkaSink::new(config, rx).await.expect("build sink"));
    let token = tokio_util::sync::CancellationToken::new();
    futures_util::future::join_all((0..workers).map(|_| sink.run(token.clone()))).await;
    tokio::time::timeout(Duration::from_secs(30), sink.close())
        .await
        .expect("sink close timed out");
    sink.metrics()
}

async fn consume(broker: &Broker, topic: &str) -> Vec<Consumed> {
    broker
        .consume_all(topic)
        .await
        .unwrap_or_else(|e| panic!("consume {topic}: {e:#}"))
}

/// `(total, sent, errored, filtered)`, for assertion messages.
fn counts(m: &internal_metrics::Sink) -> (u64, u64, u64, u64) {
    (
        m.total_records,
        m.total_records_sent,
        m.total_records_errored,
        m.total_records_filtered,
    )
}

/// Every ingested item lands in exactly one bucket, and nothing was filtered: the Kafka sink
/// never filters, so a filtered count would mean an item was booked in the wrong place.
fn assert_accounted(m: &internal_metrics::Sink) {
    assert_eq!(
        m.total_records_filtered,
        0,
        "kafka never filters: {:?}",
        counts(m)
    );
    assert_eq!(
        m.total_records,
        m.total_records_sent + m.total_records_errored,
        "total_records must equal sent + errored: {:?}",
        counts(m)
    );
}

fn json(msg: &Consumed) -> serde_json::Value {
    serde_json::from_slice(&msg.payload).unwrap_or_else(|e| {
        panic!(
            "message is not JSON: {e}: {}",
            String::from_utf8_lossy(&msg.payload)
        )
    })
}

fn key(msg: &Consumed) -> Option<&str> {
    msg.key
        .as_deref()
        .map(|k| std::str::from_utf8(k).expect("utf-8 key"))
}

fn kv(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(Value::StringValue(value.to_string())),
        }),
    }
}

/// Where a key strategy reads its key from, as far as building a fixture is concerned.
#[derive(Clone, Copy)]
enum On {
    Resource,
    Scope,
}

/// One resource per group, each carrying `attr = value` (on the resource or on its scope, per
/// `on`) — or no such attribute when `value` is `None` — and `count` log records whose bodies
/// are `"<value or none>-<i>"`.
fn keyed_logs(on: On, attr: &str, groups: &[(Option<&str>, usize)]) -> Payload {
    let resource_logs = groups
        .iter()
        .map(|&(value, count)| {
            let tagged: Vec<KeyValue> = value.map(|v| kv(attr, v)).into_iter().collect();
            let (resource_attrs, scope_attrs) = match on {
                On::Resource => (tagged, vec![]),
                On::Scope => (vec![], tagged),
            };
            let label = value.unwrap_or("none");
            ResourceLogs {
                resource: Some(Resource {
                    attributes: [vec![kv("service.name", "patrolli")], resource_attrs].concat(),
                    ..Default::default()
                }),
                scope_logs: vec![ScopeLogs {
                    scope: Some(InstrumentationScope {
                        name: "patrolli.listener".to_string(),
                        attributes: scope_attrs,
                        ..Default::default()
                    }),
                    log_records: (0..count)
                        .map(|i| LogRecord {
                            time_unix_nano: setup::otlp::START_NANOS + i as u64,
                            severity_number: 9,
                            severity_text: "INFO".to_string(),
                            body: Some(AnyValue {
                                value: Some(Value::StringValue(format!("{label}-{i}"))),
                            }),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }],
                ..Default::default()
            }
        })
        .collect();
    let data = LogsData { resource_logs };
    Payload::new(otel_arrow_dfe_pdata::encode::encode_logs_otap_batch(&data).unwrap())
}

/// The denormalized JSON shape's log body.
fn body(msg: &serde_json::Value) -> &str {
    msg["record"]["body"]
        .as_str()
        .unwrap_or_else(|| panic!("no record.body string in {msg}"))
}

// ---------------------------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------------------------

/// Each signal, serialized as OTLP protobuf, comes back off the topic as the very OTLP request
/// the payload converts to — so sharding and merging inside the sink lost and reordered nothing.
///
/// One topic per signal: an OTLP protobuf message does not say which signal it carries, and
/// decoding one as the wrong type does not reliably fail.
async fn every_signal_round_trips_as_otlp_protobuf(flavor: Flavor) {
    let broker = start(flavor).await;

    for (label, payload) in [
        ("logs", setup::otlp::logs_payload()),
        ("traces", setup::otlp::traces_payload()),
        ("metrics", setup::otlp::metrics_payload()),
    ] {
        let topic = topic(&broker, label).await;
        let expected = payload.serialize_as(Format::ProtoOtlp).unwrap();
        assert_eq!(expected.len(), 1, "proto_otlp is one blob per payload");
        let items = payload.num_items() as u64;

        // The fixtures carry no item id, so the default key strategy needs `allow_empty_key`.
        let config = sink_config(
            &broker,
            &topic,
            "  format: proto_otlp\n  allow_empty_key: true",
        );
        let metrics = run_sink(config, vec![payload]).await;
        let messages = consume(&broker, &topic).await;

        assert_eq!(messages.len(), 1, "{label}: one payload, one message");
        let msg = &messages[0];
        assert!(
            key(msg).unwrap_or_default().is_empty(),
            "{label}: keyless records are sent with an empty key, got {:?}",
            key(msg)
        );
        match label {
            "logs" => assert_eq!(
                LogsData::decode(msg.payload.as_slice()).unwrap(),
                LogsData::decode(expected[0].as_ref()).unwrap(),
            ),
            "traces" => assert_eq!(
                TracesData::decode(msg.payload.as_slice()).unwrap(),
                TracesData::decode(expected[0].as_ref()).unwrap(),
            ),
            _ => assert_eq!(
                MetricsData::decode(msg.payload.as_slice()).unwrap(),
                MetricsData::decode(expected[0].as_ref()).unwrap(),
            ),
        }

        assert_eq!(
            metrics.total_records,
            items,
            "{label}: {:?}",
            counts(&metrics)
        );
        assert_eq!(
            metrics.total_records_sent,
            items,
            "{label}: {:?}",
            counts(&metrics)
        );
        assert_eq!(
            metrics.total_records_errored,
            0,
            "{label}: {:?}",
            counts(&metrics)
        );
        assert_accounted(&metrics);
    }
}

/// Every key strategy splits a payload into one shard per key value, keys each message with that
/// value, and keeps one key on one partition. Records without the attribute are dropped and
/// booked as errored, because `allow_empty_key` is off.
///
/// `json_nested_denormalized` makes every record its own message, so the per-key message counts
/// are exactly the per-key record counts, and every body can be traced back to its group.
async fn each_key_strategy_shards_records_and_drops_the_keyless(flavor: Flavor) {
    let broker = start(flavor).await;
    let groups: &[(Option<&str>, usize)] = &[
        (Some("alpha"), 3),
        (Some("bravo"), 2),
        (Some("charlie"), 4),
        (None, 5),
    ];
    let keyed: usize = groups.iter().filter(|g| g.0.is_some()).map(|g| g.1).sum();
    let keyless: usize = groups.iter().filter(|g| g.0.is_none()).map(|g| g.1).sum();

    for (strategy, on, attr) in [
        ("host_id", On::Resource, "host.id"),
        ("item_id", On::Resource, "patrolli.item.id"),
        ("listener_id", On::Scope, "patrolli.listener.id"),
    ] {
        let topic = topic(&broker, strategy).await;
        let config = sink_config(
            &broker,
            &topic,
            &format!(
                "  format: json_nested_denormalized\n  key_strategy: {strategy}\n  allow_empty_key: false"
            ),
        );
        let metrics = run_sink(config, vec![keyed_logs(on, attr, groups)]).await;
        let messages = consume(&broker, &topic).await;

        assert_eq!(
            messages.len(),
            keyed,
            "{strategy}: one message per keyed record"
        );
        let mut by_key: FxHashMap<&str, Vec<&Consumed>> = FxHashMap::default();
        for msg in &messages {
            by_key
                .entry(key(msg).expect("keyed message"))
                .or_default()
                .push(msg);
        }
        for &(value, count) in groups {
            let Some(value) = value else { continue };
            let msgs = by_key
                .get(value)
                .unwrap_or_else(|| panic!("{strategy}: nothing keyed {value}"));
            assert_eq!(msgs.len(), count, "{strategy}: messages keyed {value}");
            // The key really is the record's own attribute, not some other group's.
            for msg in msgs {
                let b = body(&json(msg)).to_owned();
                assert!(
                    b.starts_with(&format!("{value}-")),
                    "{strategy}: record {b:?} sent under key {value}"
                );
            }
            let partitions: std::collections::BTreeSet<i32> =
                msgs.iter().map(|m| m.partition).collect();
            assert_eq!(
                partitions.len(),
                1,
                "{strategy}: key {value} spread over partitions {partitions:?}"
            );
        }
        assert_eq!(
            by_key.len(),
            3,
            "{strategy}: unexpected keys {:?}",
            by_key.keys()
        );

        assert_eq!(
            metrics.total_records,
            (keyed + keyless) as u64,
            "{strategy}"
        );
        assert_eq!(metrics.total_records_sent, keyed as u64, "{strategy}");
        assert_eq!(
            metrics.total_records_errored, keyless as u64,
            "{strategy}: the keyless records are dropped as errored"
        );
        assert_accounted(&metrics);
    }
}

/// Each `compression` value is one librdkafka actually has compiled in and the broker accepts:
/// a codec missing from the build fails producer creation, and one the broker rejects fails
/// delivery. Reading the batch back proves the consumer side can decode it too.
async fn every_compression_codec_round_trips(flavor: Flavor) {
    let broker = start(flavor).await;
    for codec in ["none", "gzip", "snappy", "lz4", "zstd"] {
        let topic = topic(&broker, codec).await;
        let config = sink_config(
            &broker,
            &topic,
            &format!("  format: json_otlp_batch\n  allow_empty_key: true\n  compression: {codec}"),
        );
        let metrics = run_sink(config, vec![setup::otlp::logs_payload()]).await;
        let messages = consume(&broker, &topic).await;

        assert_eq!(messages.len(), 1, "{codec}: json_otlp_batch is one message");
        let records = &json(&messages[0])["resourceLogs"][0]["scopeLogs"][0]["logRecords"];
        assert_eq!(
            records.as_array().map(Vec::len),
            Some(2),
            "{codec}: both log records survive: {records}"
        );
        assert_eq!(
            metrics.total_records_sent,
            2,
            "{codec}: {:?}",
            counts(&metrics)
        );
        assert_eq!(
            metrics.total_records_errored,
            0,
            "{codec}: {:?}",
            counts(&metrics)
        );
        assert_accounted(&metrics);
    }
}

/// With a batch timeout far longer than the test, nothing closes the batch on a timer. The input
/// ending does: the worker sends what it holds and waits for it to land, so a shutdown or a
/// config reload does not wait out the timeout, nor drop the batch. `batch_timeout` no longer
/// reaches librdkafka, so it may exceed `write_timeout_seconds`.
async fn a_long_batch_timeout_is_cut_short_when_the_input_ends(flavor: Flavor) {
    let broker = start(flavor).await;
    let topic = topic(&broker, "linger").await;
    let mut config = sink_config(
        &broker,
        &topic,
        "  format: json_otlp_denormalized\n  allow_empty_key: true",
    );
    config.batch.batch_timeout = Duration::from_secs(60);

    let started = std::time::Instant::now();
    let metrics = run_sink(config, vec![setup::otlp::logs_payload()]).await;
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "the input ending should send at once, not wait out the batch timeout: {:?}",
        started.elapsed()
    );

    let messages = consume(&broker, &topic).await;
    assert_eq!(
        messages.len(),
        2,
        "denormalized: one message per log record"
    );
    assert_eq!(metrics.total_records_sent, 2, "{:?}", counts(&metrics));
    assert_accounted(&metrics);
}

/// Several workers sharding, serializing and producing in parallel, sharing a pool of producers
/// round-robin, still deliver each record exactly once.
async fn concurrent_workers_and_producers_deliver_everything(flavor: Flavor) {
    const PAYLOADS: usize = 40;
    const HOSTS: [&str; 4] = ["h0", "h1", "h2", "h3"];
    const PER_HOST: usize = 5;

    let broker = start(flavor).await;
    let topic = topic(&broker, "concurrent").await;
    let mut config = sink_config(
        &broker,
        &topic,
        "  format: json_nested_denormalized\n  key_strategy: host_id\n  producer_count: 3",
    );
    config.worker_count = 4;
    let groups: Vec<(Option<&str>, usize)> = HOSTS.iter().map(|h| (Some(*h), PER_HOST)).collect();
    let payloads = (0..PAYLOADS)
        .map(|_| keyed_logs(On::Resource, "host.id", &groups))
        .collect();

    let metrics = run_sink(config, payloads).await;
    let messages = consume(&broker, &topic).await;

    let expected = PAYLOADS * HOSTS.len() * PER_HOST;
    assert_eq!(messages.len(), expected, "every record exactly once");
    for host in HOSTS {
        let n = messages.iter().filter(|m| key(m) == Some(host)).count();
        assert_eq!(n, PAYLOADS * PER_HOST, "messages keyed {host}");
    }
    assert_eq!(
        metrics.total_records_sent,
        expected as u64,
        "{:?}",
        counts(&metrics)
    );
    assert_eq!(metrics.total_records_errored, 0, "{:?}", counts(&metrics));
    assert_accounted(&metrics);
}

/// A topic that does not exist (and that the broker will not auto-create) is a delivery
/// failure that retrying will not fix: librdkafka reports `UnknownTopic` once
/// `topic.metadata.propagation.max.ms` (half of `write_timeout_seconds` here) passes, before
/// the attempt would time out and be retried. Each record must be booked as errored exactly
/// once — not sent, not lost — and the worker must move on rather than retry forever.
async fn a_missing_topic_counts_every_record_as_errored(flavor: Flavor) {
    let broker = start(flavor).await;
    let config = sink_config(
        &broker,
        &format!("patrolli-test-absent-{}", uuid::Uuid::now_v7()),
        "  format: json_otlp_denormalized\n  allow_empty_key: true\n  write_timeout_seconds: 2",
    );

    let metrics = run_sink(
        config,
        vec![setup::otlp::logs_payload(), setup::otlp::traces_payload()],
    )
    .await;

    assert_eq!(metrics.total_records, 4, "{:?}", counts(&metrics));
    assert_eq!(metrics.total_records_sent, 0, "{:?}", counts(&metrics));
    assert_eq!(metrics.total_records_errored, 4, "{:?}", counts(&metrics));
    assert_accounted(&metrics);
}

/// The whole cluster goes away with a batch in hand. Each attempt times out after
/// `write_timeout_seconds`, the worker keeps the batch and retries it, and once the broker is
/// back every record lands. Exactly once: the broker was down for the attempts that timed out,
/// so none of them reached it.
async fn a_broker_outage_is_retried_until_it_comes_back(flavor: Flavor) {
    let broker = start(flavor).await;
    let topic = topic(&broker, "outage").await;
    let mut config = sink_config(
        &broker,
        &topic,
        "  format: json_otlp_denormalized\n  allow_empty_key: true\n  write_timeout_seconds: 2",
    );
    config.retry.initial_interval = Duration::from_millis(200);
    config.retry.max_interval = Duration::from_secs(1);
    broker.stop().await.expect("stop broker");

    let (tx, rx) = flume::bounded(1);
    tx.send_async(setup::otlp::logs_payload()).await.unwrap();
    let rx = patrolli::chan::Consumer::from_flume(&tx, rx);
    drop(tx);
    let sink = Arc::new(KafkaSink::new(config, rx).await.expect("build sink"));
    let worker = tokio::spawn({
        let sink = sink.clone();
        async move { sink.run(tokio_util::sync::CancellationToken::new()).await }
    });

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while sink.metrics().total_records_retried == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "never retried: {:?}",
            counts(&sink.metrics())
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let m = sink.metrics();
    assert_eq!(m.total_records_sent, 0, "{:?}", counts(&m));
    assert_eq!(
        m.total_records_errored, 0,
        "a timeout is retried, not dropped"
    );
    assert_eq!(m.records_pending_retry, 2);

    broker
        .resume(&topic, PARTITIONS)
        .await
        .expect("resume broker");
    tokio::time::timeout(Duration::from_secs(60), worker)
        .await
        .expect("the worker delivers once the broker is back")
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), sink.close())
        .await
        .expect("sink close timed out");

    let m = sink.metrics();
    let messages = consume(&broker, &topic).await;
    assert_eq!(messages.len(), 2, "every record exactly once");
    assert_eq!(m.total_records_sent, 2, "{:?}", counts(&m));
    assert_eq!(m.records_pending_retry, 0);
    assert_accounted(&m);
}
