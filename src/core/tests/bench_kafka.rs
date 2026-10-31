// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Kafka sink versus raw librdkafka sustained delivery throughput.
//!
//! Run from src/core (Docker required unless targeting an existing plaintext broker):
//! ```text
//! cargo test --release --test bench_kafka -- --ignored --nocapture
//! KAFKA_BENCH_CASES=1:10 KAFKA_BENCH_WARMUP_SECS=2 KAFKA_BENCH_DURATION_SECS=5 \
//!   cargo test --release --test bench_kafka -- --ignored --nocapture
//! ```
//! Environment overrides (all prefixed KAFKA_BENCH_):
//! - BOOTSTRAP_SERVERS: existing broker; otherwise disposable apache/kafka:latest.
//! - CASES: worker:producer:linger_ms specs; default 1:1:3000,1:1:10,4:1:10,4:4:10,8:8:10.
//!   worker:linger_ms is still accepted and means a single producer.
//! - WARMUP_SECS / DURATION_SECS: default 10 / 30, both positive.
//! - PARTITIONS / MESSAGE_BYTES: default 6 / 1024 (serialized value bytes).
//! - COMPRESSION: none (default), gzip, snappy, lz4, zstd. ACKS: 0, 1 (default), all.
//! - TMPFS: 1 keeps the disposable broker's log dir on tmpfs.
//!
//! The sink closes a batch at 50,000 items or the case's linger (its
//! `batch_timeout`), and each worker waits for its batch's delivery reports
//! before the next; its workers share the case's producers round-robin. Raw is
//! always the one-producer reference, with queue capacity 50,000 and the
//! case's librdkafka linger. Both use Murmur2 keys and a 10ms QueueFull retry.
//! The sink includes bounded channel delivery, sharding and serialization; raw
//! uses the exact precomputed bytes.
//! Each path has one load-generator task per configured worker.
//! Fixtures have 100 logs/payload and 64 rotating host keys. Generation is untimed.
//! Completed delivery deltas during continuous load determine the reported rate.
//! MiB/s measures serialized values, not compressed Kafka/network traffic.
//! Final accounting includes warmup and drain; no throughput threshold is imposed.
//! Results depend on local CPU, Docker resources, storage, and these settings.
//! Each case can write tens of GiB; its topic is deleted after offset verification.

#[path = "setup/kafka.rs"]
mod kafka;

use anyhow::{Context, Result, bail, ensure};
use arc_swap::ArcSwap;
use bytes::Bytes;
use otel_arrow_dfe_pdata::proto::opentelemetry::{
    common::v1::{AnyValue, KeyValue, any_value::Value},
    logs::v1::{LogRecord, LogsData, ResourceLogs, ScopeLogs},
    resource::v1::Resource,
};
use patrolli::common::internal_metrics;
use patrolli::config::sink::{KafkaCompression, KafkaRequiredAcks};
use patrolli::{chan, config, payload::Payload, sink::WorkerGroup};
use rdkafka::error::{KafkaError, RDKafkaErrorCode};
use rdkafka::message::DeliveryResult;
use rdkafka::producer::{BaseRecord, Producer, ProducerContext, ThreadedProducer};
use rdkafka::{ClientConfig, ClientContext, Statistics};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use uuid::Uuid;

const RECORDS: usize = 100;
const KEYS: usize = 64;
const FORMAT: config::Format = config::Format::JsonNestedDenormalized;
const DRAIN_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Case {
    workers: usize,
    producers: usize,
    linger_ms: u64,
}

fn parse_cases(value: &str) -> Result<Vec<Case>> {
    value
        .split(',')
        .map(|spec| {
            // worker:producer:linger_ms, or the older worker:linger_ms, which
            // means a single producer.
            let fields: Vec<&str> = spec.split(':').collect();
            let (workers, producers, linger) = match fields.as_slice() {
                [workers, linger] => (*workers, "1", *linger),
                [workers, producers, linger] => (*workers, *producers, *linger),
                _ => {
                    bail!("CASES must contain worker:producer:linger_ms or worker:linger_ms specs")
                }
            };
            let case = Case {
                workers: workers.parse()?,
                producers: producers.parse()?,
                linger_ms: linger.parse()?,
            };
            ensure!(
                case.workers > 0 && case.workers <= 256,
                "workers must be 1..=256"
            );
            ensure!(
                case.producers > 0 && case.producers <= 256,
                "producers must be 1..=256"
            );
            // The application's validator replaces zero linger with its default.
            ensure!(
                case.linger_ms > 0 && case.linger_ms < 30_000,
                "linger must be 1..29999 ms (delivery timeout is 30s)"
            );
            Ok(case)
        })
        .collect()
}

#[derive(Debug)]
struct Options {
    cases: Vec<Case>,
    warmup: Duration,
    duration: Duration,
    partitions: i32,
    message_bytes: usize,
    compression: KafkaCompression,
    acks: KafkaRequiredAcks,
}

fn env(name: &str, default: &str) -> String {
    std::env::var(format!("KAFKA_BENCH_{name}")).unwrap_or_else(|_| default.to_owned())
}

impl Options {
    fn read() -> Result<Self> {
        let warmup: u64 = env("WARMUP_SECS", "10").parse()?;
        let duration: u64 = env("DURATION_SECS", "30").parse()?;
        ensure!(
            warmup > 0 && duration > 0,
            "warmup and duration must be positive"
        );
        let partitions = env("PARTITIONS", "6").parse()?;
        ensure!(partitions > 0, "PARTITIONS must be positive");
        let message_bytes = env("MESSAGE_BYTES", "1024").parse()?;
        ensure!(
            message_bytes > 0 && message_bytes <= 900_000,
            "MESSAGE_BYTES must be 1..=900000"
        );
        let compression =
            serde_json::from_value(serde_json::Value::String(env("COMPRESSION", "none")))?;
        let acks = match env("ACKS", "1").as_str() {
            "0" => KafkaRequiredAcks::None,
            "1" => KafkaRequiredAcks::One,
            "all" => KafkaRequiredAcks::All,
            _ => bail!("ACKS must be 0, 1 or all"),
        };
        Ok(Self {
            cases: parse_cases(&env("CASES", "1:1:3000,1:1:10,4:1:10,4:4:10,8:8:10"))?,
            warmup: Duration::from_secs(warmup),
            duration: Duration::from_secs(duration),
            partitions,
            message_bytes,
            compression,
            acks,
        })
    }
}

struct Fixture {
    key: String,
    payload: Payload,
    blobs: Vec<Bytes>,
}

fn make_payload(key: &str, body: &str) -> Result<Payload> {
    let record = LogRecord {
        time_unix_nano: 1_700_000_000_000_000_000,
        severity_number: 9,
        severity_text: "INFO".into(),
        body: Some(AnyValue {
            value: Some(Value::StringValue(body.into())),
        }),
        ..Default::default()
    };
    let data = LogsData {
        resource_logs: vec![ResourceLogs {
            resource: Some(Resource {
                attributes: vec![KeyValue {
                    key: "host.id".into(),
                    value: Some(AnyValue {
                        value: Some(Value::StringValue(key.into())),
                    }),
                }],
                ..Default::default()
            }),
            scope_logs: vec![ScopeLogs {
                log_records: vec![record; RECORDS],
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    Ok(Payload::new(
        otel_arrow_dfe_pdata::encode::encode_logs_otap_batch(&data)?,
    ))
}

fn fixtures(message_bytes: usize) -> Result<Vec<Fixture>> {
    (0..KEYS)
        .map(|index| {
            let key = format!("bench-host-{index:04}");
            // Empty strings are normalized by the OTAP encoder. Calibrate with a
            // nonempty body so the JSON envelope has the same shape as the load.
            let overhead = make_payload(&key, "a")?.serialize_as(FORMAT)?[0].len() - 1;
            ensure!(
                message_bytes > overhead,
                "MESSAGE_BYTES must exceed {overhead} for the JSON envelope"
            );
            // Reproducible printable bytes rather than zero padding. Compression is
            // disabled by default; repeated templates are disclosed in the docs.
            let mut state = 0x1234_5678u32 + index as u32;
            let body: String = (0..message_bytes - overhead)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    char::from(b'a' + (state % 26) as u8)
                })
                .collect();
            let payload = make_payload(&key, &body)?;
            // Precompute through the same sharding operation the real sink uses.
            let mut batch = patrolli::payload::PayloadBatch::new();
            batch.push(payload.clone());
            let shards = batch.shard(patrolli::payload::AttrLocation::Resource, "host.id")?;
            let mut blobs = Vec::new();
            for (shard_key, shard) in shards {
                ensure!(
                    shard_key
                        .as_ref()
                        .map(|k| k.as_ref().to_string())
                        .as_deref()
                        == Some(key.as_str()),
                    "fixture key mismatch"
                );
                for pl in shard.into_iter() {
                    blobs.extend(pl.serialize_as(FORMAT)?);
                }
            }
            ensure!(
                payload.num_items() == RECORDS && blobs.len() == RECORDS,
                "expected one Kafka message per log"
            );
            ensure!(
                blobs.iter().all(|b| b.len() == message_bytes),
                "serialized message sizes differ: target={message_bytes}, first={:?}",
                blobs.first().map(Bytes::len)
            );
            Ok(Fixture {
                key,
                payload,
                blobs,
            })
        })
        .collect()
}

#[derive(Default)]
struct Counters {
    accepted: AtomicU64,
    delivered: AtomicU64,
    failed: AtomicU64,
    queue_full: AtomicU64,
    stop: AtomicBool,
    first_error: Mutex<Option<String>>,
    stats: Mutex<Option<Statistics>>,
}

struct RawContext(Arc<Counters>);
impl ClientContext for RawContext {
    fn stats(&self, stats: Statistics) {
        *self.0.stats.lock().unwrap() = Some(stats);
    }
}
impl ProducerContext for RawContext {
    type DeliveryOpaque = ();
    fn delivery(&self, result: &DeliveryResult<'_>, _: ()) {
        match result {
            Ok(_) => {
                self.0.delivered.fetch_add(1, Relaxed);
            }
            Err((error, _)) => {
                if self.0.failed.fetch_add(1, Relaxed) == 0 {
                    *self.0.first_error.lock().unwrap() = Some(error.to_string());
                }
            }
        }
    }
}

enum Backend {
    Sink(Arc<WorkerGroup>),
    Raw(Arc<ThreadedProducer<RawContext>>),
}

struct Running {
    backend: Backend,
    counters: Arc<Counters>,
    feeders: Vec<JoinHandle<Result<()>>>,
}

#[derive(Default, Debug, Clone, Copy)]
struct Snapshot {
    accepted: u64,
    delivered: u64,
    failed: u64,
    queue_full: u64,
    total: f64,
    send: f64,
    engine: f64,
    receive: f64,
}

impl Running {
    async fn snapshot(&self) -> Snapshot {
        let c = &self.counters;
        let mut snap = Snapshot {
            accepted: c.accepted.load(Relaxed),
            queue_full: c.queue_full.load(Relaxed),
            ..Default::default()
        };
        match &self.backend {
            Backend::Raw(_) => {
                snap.delivered = c.delivered.load(Relaxed);
                snap.failed = c.failed.load(Relaxed);
            }
            Backend::Sink(group) => {
                let metrics = group.metrics().await;
                let m = &metrics[0];
                snap.delivered = m.total_records_sent;
                snap.failed = m.total_records_errored;
                snap.total = m.total_time_spent.as_secs_f64();
                snap.send = m.send_wait_time.as_secs_f64();
                snap.engine = m.engine_time_spent.as_secs_f64();
                snap.receive = m.receive_wait_time.as_secs_f64();
            }
        }
        snap
    }

    async fn drain(&mut self) -> Result<Snapshot> {
        self.counters.stop.store(true, Relaxed);
        let drained = tokio::time::timeout(DRAIN_TIMEOUT, async {
            for feeder in &mut self.feeders {
                feeder.await??;
            }
            loop {
                let snap = self.snapshot().await;
                ensure!(
                    snap.failed == 0,
                    "delivery failures: {snap:?}; first raw error: {:?}",
                    self.counters.first_error.lock().unwrap()
                );
                ensure!(
                    snap.delivered <= snap.accepted,
                    "delivery accounting mismatch: {snap:?}"
                );
                if snap.delivered == snap.accepted {
                    return Ok(snap);
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        match drained {
            Ok(result) => result,
            Err(_) => bail!("60s drain deadline exceeded: {:?}", self.snapshot().await),
        }
    }

    async fn close(&mut self) -> Result<()> {
        self.counters.stop.store(true, Relaxed);
        for feeder in &self.feeders {
            feeder.abort();
        }
        match &self.backend {
            Backend::Sink(group) => {
                tokio::time::timeout(Duration::from_secs(15), group.close_all_sink())
                    .await
                    .context("sink shutdown timed out")?;
            }
            Backend::Raw(producer) => {
                let producer = producer.clone();
                tokio::task::spawn_blocking(move || producer.flush(Duration::from_secs(10)))
                    .await??;
            }
        }
        Ok(())
    }
}

async fn start_sink(
    bootstrap: &str,
    topic: &str,
    case: Case,
    opts: &Options,
    data: Arc<Vec<Fixture>>,
) -> Result<Running> {
    let metrics = Arc::new(ArcSwap::from_pointee((
        Instant::now(),
        internal_metrics::Channel::default(),
    )));
    let (admin, sender) = chan::new::<Uuid, Payload>(Duration::from_secs(1), metrics);
    let group = WorkerGroup::new(admin);
    let mut conf = config::sink::Sink {
        id: Uuid::now_v7(),
        name: "kafka-benchmark".into(),
        worker_count: case.workers,
        inputs: vec![],
        batch: config::Batch {
            batch_size: 50_000,
            batch_timeout: Duration::from_millis(case.linger_ms),
        },
        channel: config::Channel::Memory(config::MemoryChannel {
            non_blocking: false,
            channel_buffer_size: 64,
        }),
        retry: config::Retry::default(),
        config: config::sink::Config::Kafka(config::sink::Kafka {
            hosts: vec![bootstrap.into()],
            topic: topic.into(),
            key_strategy: config::sink::KafkaKeyStrategy::HostId,
            allow_empty_key: false,
            format: FORMAT,
            write_timeout_seconds: 30,
            balancer_strategy: config::sink::KafkaBalancerStrategy::Murmur2,
            required_acks: opts.acks,
            compression: opts.compression,
            security: config::sink::KafkaSecurity {
                protocol: config::sink::KafkaSecurityProtocol::PlainText,
            },
            ssl: None,
            sasl: None,
            producer_count: case.producers,
            partition_batch_size: 1_000_000,
            partition_batch_count: 10_000,
            global_queue_size: 1_048_576_000,
            max_message_size: 1_000_000,
            enable_producer_stats: false,
        }),
    };
    conf.validate()?;
    group.add(conf).await?;
    let counters = Arc::new(Counters::default());
    let mut feeders = Vec::new();
    for worker in 0..case.workers {
        let (c, sender, data) = (counters.clone(), sender.clone(), data.clone());
        feeders.push(tokio::spawn(async move {
            let mut index = worker;
            while !c.stop.load(Relaxed) {
                sender
                    .broadcast(data[index % data.len()].payload.clone())
                    .await
                    .map_err(|e| anyhow::anyhow!("sink channel send: {e}"))?;
                c.accepted.fetch_add(RECORDS as u64, Relaxed);
                index += 1;
                tokio::task::yield_now().await;
            }
            Ok(())
        }));
    }
    Ok(Running {
        backend: Backend::Sink(group),
        counters,
        feeders,
    })
}

fn start_raw(
    bootstrap: &str,
    topic: &str,
    case: Case,
    opts: &Options,
    data: Arc<Vec<Fixture>>,
) -> Result<Running> {
    let counters = Arc::new(Counters::default());
    let producer: ThreadedProducer<RawContext> = ClientConfig::new()
        .set("bootstrap.servers", bootstrap)
        .set("queue.buffering.max.messages", "50000")
        .set("linger.ms", case.linger_ms.to_string())
        .set("compression.type", opts.compression.as_str())
        .set("acks", opts.acks.as_str())
        .set("partitioner", "murmur2")
        .set("message.timeout.ms", "30000")
        .set("statistics.interval.ms", "1000")
        .create_with_context(RawContext(counters.clone()))?;
    let producer = Arc::new(producer);
    let mut feeders = Vec::new();
    for worker in 0..case.workers {
        let (producer, c, data, topic) = (
            producer.clone(),
            counters.clone(),
            data.clone(),
            topic.to_owned(),
        );
        feeders.push(tokio::spawn(async move {
            let mut index = worker;
            while !c.stop.load(Relaxed) {
                let fixture = &data[index % data.len()];
                for blob in &fixture.blobs {
                    let mut record = BaseRecord::to(&topic)
                        .key(&fixture.key)
                        .payload(blob.as_ref());
                    loop {
                        match producer.send(record) {
                            Ok(()) => break,
                            Err((
                                KafkaError::MessageProduction(RDKafkaErrorCode::QueueFull),
                                returned,
                            )) => {
                                c.queue_full.fetch_add(1, Relaxed);
                                record = returned;
                                tokio::time::sleep(Duration::from_millis(10)).await;
                            }
                            Err((e, _)) => return Err(e.into()),
                        }
                    }
                }
                c.accepted.fetch_add(RECORDS as u64, Relaxed);
                index += 1;
                tokio::task::yield_now().await;
            }
            Ok(())
        }));
    }
    Ok(Running {
        backend: Backend::Raw(producer),
        counters,
        feeders,
    })
}

fn delivery_rate(start: u64, end: u64, elapsed: Duration) -> f64 {
    (end - start) as f64 / elapsed.as_secs_f64()
}

#[derive(Debug)]
struct Report {
    path: &'static str,
    case: Case,
    rate: f64,
    mib: f64,
    send_pct: Option<f64>,
}

async fn measure(
    run: &mut Running,
    path: &'static str,
    case: Case,
    opts: &Options,
) -> Result<Report> {
    tokio::time::sleep(opts.warmup).await;
    let before = run.snapshot().await;
    let start = Instant::now();
    let deadline = tokio::time::Instant::now() + opts.duration;
    let mut previous = before;
    let mut previous_at = start;
    let after;
    let elapsed;
    loop {
        tokio::time::sleep_until(
            (tokio::time::Instant::now() + Duration::from_secs(5)).min(deadline),
        )
        .await;
        let snap = run.snapshot().await;
        let now = Instant::now();
        println!(
            "  {path}: {:.0} delivered/s, accepted={}, delivered={}, failed={}, pending~{}",
            delivery_rate(
                previous.delivered,
                snap.delivered,
                now.duration_since(previous_at)
            ),
            snap.accepted,
            snap.delivered,
            snap.failed,
            snap.accepted.saturating_sub(snap.delivered)
        );
        ensure!(
            snap.failed == 0,
            "delivery errors during measurement: {snap:?}"
        );
        ensure!(
            !run.feeders.iter().any(JoinHandle::is_finished),
            "load generator exited during measurement"
        );
        if tokio::time::Instant::now() >= deadline {
            after = snap;
            elapsed = now.duration_since(start);
            break;
        }
        previous = snap;
        previous_at = now;
    }
    let rate = delivery_rate(before.delivered, after.delivered, elapsed);
    ensure!(
        after.delivered > before.delivered,
        "no completed deliveries in measurement window"
    );
    let stats = run.counters.stats.lock().unwrap().clone();
    let final_counts = run.drain().await?;
    println!(
        "  measured={:.3}s deliveries={} total accepted/delivered={}/{} errors={} drain complete",
        elapsed.as_secs_f64(),
        after.delivered - before.delivered,
        final_counts.accepted,
        final_counts.delivered,
        final_counts.failed
    );
    let send_pct = if path == "sink" {
        let total = after.total - before.total;
        ensure!(total > 0.0, "sink reported no worker time");
        let pct = |end: f64, begin: f64| 100.0 * (end - begin) / total;
        println!(
            "  sink worker time: send_wait={:.2}% engine={:.2}% receive_wait={:.2}%",
            pct(after.send, before.send),
            pct(after.engine, before.engine),
            pct(after.receive, before.receive)
        );
        Some(pct(after.send, before.send))
    } else {
        println!(
            "  raw QueueFull retries during measurement={}",
            after.queue_full - before.queue_full
        );
        if let Some(stats) = stats {
            println!(
                "  raw last producer sample: queued={}/{} bytes={}/{} replyq={}",
                stats.msg_cnt, stats.msg_max, stats.msg_size, stats.msg_size_max, stats.replyq
            );
            for broker in stats.brokers.values().filter(|b| b.nodeid >= 0) {
                println!(
                    "  broker {} state={} rtt_avg_us={} throttle_avg_ms={} retries={} timeouts={} waiting_messages={}",
                    broker.name,
                    broker.state,
                    broker.rtt.as_ref().map_or(0, |w| w.avg),
                    broker.throttle.as_ref().map_or(0, |w| w.avg),
                    broker.txretries,
                    broker.req_timeouts,
                    broker.waitresp_msg_cnt
                );
            }
        }
        None
    };
    Ok(Report {
        path,
        case,
        rate,
        mib: rate * opts.message_bytes as f64 / 1_048_576.0,
        send_pct,
    })
}

async fn benchmark() -> Result<()> {
    let opts = Options::read()?;
    let data = Arc::new(fixtures(opts.message_bytes)?);
    println!(
        "Kafka benchmark: {opts:?}\nfixtures={} keys, {} records/payload, {} serialized bytes/message; queue=50000/producer; format=json_nested_denormalized",
        KEYS, RECORDS, opts.message_bytes
    );
    let broker = kafka::Broker::start().await?;
    println!(
        "broker={} at {}; replication=1; payload MiB/s excludes Kafka framing/compression",
        broker.description, broker.bootstrap
    );
    let mut reports = Vec::new();
    for &case in &opts.cases {
        for path in ["sink", "raw"] {
            let topic = format!("patrolli-bench-{}", Uuid::now_v7());
            // raw is always the single-producer reference.
            let producers = if path == "sink" { case.producers } else { 1 };
            println!(
                "\n{path}: workers={} producers={producers} linger={}ms topic={topic}",
                case.workers, case.linger_ms
            );
            // Topic creation can succeed before readiness fails: clean up on either failure.
            let result = async {
                broker.create_topic(&topic, opts.partitions).await?;
                let mut run = if path == "sink" {
                    start_sink(&broker.bootstrap, &topic, case, &opts, data.clone()).await?
                } else {
                    start_raw(&broker.bootstrap, &topic, case, &opts, data.clone())?
                };
                let measured = measure(&mut run, path, case, &opts).await;
                let final_counts = run.snapshot().await;
                let closed = run.close().await;
                let report = measured?;
                closed?;
                if opts.acks == KafkaRequiredAcks::None {
                    // acks=0 reports a message delivered once it is on the socket,
                    // so what the broker dropped is only visible in its offsets.
                    let landed = broker
                        .settled_count(&topic, opts.partitions, final_counts.delivered)
                        .await?;
                    println!(
                        "  broker offsets: {landed} of {} delivered messages landed ({} lost)",
                        final_counts.delivered,
                        final_counts.delivered.saturating_sub(landed)
                    );
                } else {
                    broker
                        .verify_count(&topic, opts.partitions, final_counts.delivered)
                        .await?;
                    println!(
                        "  verified broker offsets: {} messages",
                        final_counts.delivered
                    );
                }
                Ok::<_, anyhow::Error>(report)
            }
            .await;
            let cleanup = broker.delete_topic(&topic).await;
            if let Err(error) = &cleanup {
                eprintln!("topic cleanup failed for {topic}: {error:#}");
            }
            reports
                .push(result.with_context(|| format!("{path} case {case:?} failed; not ranked"))?);
            cleanup?;
        }
    }
    println!("\npath  workers producers linger_ms    delivered/s  payload_MiB/s send_wait_%");
    for r in &reports {
        println!(
            "{:<5} {:>7} {:>9} {:>9} {:>14.0} {:>14.2} {:>11}",
            r.path,
            r.case.workers,
            if r.path == "sink" {
                r.case.producers
            } else {
                1
            },
            r.case.linger_ms,
            r.rate,
            r.mib,
            r.send_pct.map_or_else(|| "-".into(), |p| format!("{p:.2}"))
        );
    }
    for path in ["sink", "raw"] {
        let best = reports
            .iter()
            .filter(|r| r.path == path)
            .max_by(|a, b| a.rate.total_cmp(&b.rate))
            .unwrap();
        println!(
            "Best observed {path}: {:.0} delivered msgs/s ({:.2} payload MiB/s), {} workers, {} producers, {}ms linger",
            best.rate,
            best.mib,
            best.case.workers,
            if path == "sink" {
                best.case.producers
            } else {
                1
            },
            best.case.linger_ms
        );
    }
    Ok(())
}

#[test]
#[ignore = "throughput benchmark: requires Docker or KAFKA_BENCH_BOOTSTRAP_SERVERS; run in release"]
fn bench_kafka() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .try_init();
    let threads = std::thread::available_parallelism()
        .map_or(2, usize::from)
        .max(2);
    println!(
        "runtime threads={threads}; release={}",
        !cfg!(debug_assertions)
    );
    ensure!(
        !cfg!(debug_assertions),
        "use cargo test --release for throughput measurements"
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(threads)
        .enable_all()
        .build()?;
    let result = runtime.block_on(benchmark());
    // Failure cleanup remains bounded even if an existing sink worker is stuck
    // retrying QueueFull or a library call is still finishing on a blocking thread.
    runtime.shutdown_timeout(Duration::from_secs(15));
    result
}

#[test]
fn case_parser_rejects_invalid_settings() {
    assert_eq!(
        parse_cases("1:3000,4:8:10").unwrap(),
        vec![
            // Two fields keep their old meaning: one producer.
            Case {
                workers: 1,
                producers: 1,
                linger_ms: 3000
            },
            Case {
                workers: 4,
                producers: 8,
                linger_ms: 10
            }
        ]
    );
    for bad in [
        "", "1", "0:10", "1:0", "1:30000", "257:10", "1:10,", "1:0:10", "1:257:10", "1:1:1:10",
    ] {
        assert!(parse_cases(bad).is_err(), "{bad}");
    }
}

#[test]
fn fixture_preserves_keys_counts_and_serialized_sizes() -> Result<()> {
    let data = fixtures(1024)?;
    assert_eq!(data.len(), KEYS);
    for f in data {
        assert_eq!(f.payload.num_items(), RECORDS);
        assert_eq!(f.blobs.len(), RECORDS);
        for blob in f.blobs {
            assert_eq!(blob.len(), 1024);
            let json: serde_json::Value = serde_json::from_slice(&blob)?;
            assert!(json.to_string().contains(&f.key));
        }
    }
    assert!(fixtures(1).is_err());
    Ok(())
}

#[test]
fn throughput_uses_delivery_delta_and_actual_elapsed() {
    assert_eq!(
        delivery_rate(50_000, 70_000, Duration::from_secs(4)),
        5_000.0
    );
    assert_eq!(delivery_rate(50_000, 50_000, Duration::from_secs(4)), 0.0);
}
