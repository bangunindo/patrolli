// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Kafka through librdkafka, used as a transport rather than as a second sink.
//!
//! [`ExportSink`] decides when a batch closes; [`Kafka::export`] produces the
//! whole batch and waits for the delivery report of every message before it
//! answers, so a batch is acked only once Kafka has it (as far as
//! `required_acks` asks), and retried by the driver otherwise. librdkafka keeps
//! the wire framing (per-partition requests) and its own fast retries inside one
//! attempt, which `message.timeout.ms` (`write_timeout_seconds`) bounds.

use super::export::{Encoded, Export, ExportSink, Exporter};
use crate::{chan, config, payload};
use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use rdkafka::config::{ClientConfig, RDKafkaLogLevel};
use rdkafka::error::{KafkaError, RDKafkaErrorCode};
use rdkafka::message::DeliveryResult;
use rdkafka::producer::{
    BaseRecord, NoCustomPartitioner, Producer, ProducerContext, ThreadedProducer,
};
use rdkafka::{ClientContext, Statistics};
use smol_str::SmolStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::Notify;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

type KafkaProducer = ThreadedProducer<KafkaSinkContext>;

/// librdkafka's linger (`queue.buffering.max.ms`). Batching is the sink's job,
/// and a worker waits for its whole batch to be acked, so any linger is added
/// to every batch's round trip. An attempt enqueues its batch at once, and
/// what queues while a request is in flight goes in the next one; packing
/// requests fuller isn't worth the wait.
const LINGER: Duration = Duration::ZERO;
/// How long `export` waits before offering a message again when librdkafka's
/// queue (`global_queue_size`) is full.
const QUEUE_FULL_WAIT: Duration = Duration::from_millis(10);
/// librdkafka's default `topic.metadata.propagation.max.ms`; see
/// [`Kafka::new`] for why it is capped below the attempt timeout.
const TOPIC_PROPAGATION: Duration = Duration::from_secs(30);
const FLUSH_TIMEOUT: Duration = Duration::from_secs(10);

pub struct KafkaSinkContext {
    sink_id: Uuid,
    sink_name: String,
}

// librdkafka calls these from its own threads, outside any sink span, so each
// event repeats the sink identity. The defaults log with interpolated messages,
// which defeats the log throttle.
impl ClientContext for KafkaSinkContext {
    fn log(&self, level: RDKafkaLogLevel, fac: &str, log_message: &str) {
        match level {
            RDKafkaLogLevel::Emerg
            | RDKafkaLogLevel::Alert
            | RDKafkaLogLevel::Critical
            | RDKafkaLogLevel::Error
            | RDKafkaLogLevel::Warning => {
                warn!(sink_id = %self.sink_id, sink = %self.sink_name, facility = fac, log = log_message, "librdkafka log")
            }
            RDKafkaLogLevel::Notice | RDKafkaLogLevel::Info => {
                info!(sink_id = %self.sink_id, sink = %self.sink_name, facility = fac, log = log_message, "librdkafka log")
            }
            RDKafkaLogLevel::Debug => {
                debug!(sink_id = %self.sink_id, sink = %self.sink_name, facility = fac, log = log_message, "librdkafka log")
            }
        }
    }

    fn stats(&self, statistics: Statistics) {
        debug!(sink_id = %self.sink_id, sink = %self.sink_name, stats = ?statistics, "kafka client stats");
    }

    fn error(&self, error: KafkaError, reason: &str) {
        warn!(sink_id = %self.sink_id, sink = %self.sink_name, error = %error, reason, "kafka client error");
    }
}

impl ProducerContext<NoCustomPartitioner> for KafkaSinkContext {
    /// The attempt the message belongs to, and its index in the batch.
    type DeliveryOpaque = Box<(Arc<Attempt>, usize)>;

    /// Runs on the producer's polling thread: records the result and nothing
    /// else. The driver counts and logs once the whole attempt is in.
    fn delivery(&self, result: &DeliveryResult<'_>, opaque: Self::DeliveryOpaque) {
        let (attempt, idx) = *opaque;
        attempt.report(idx, result.as_ref().err().map(|(e, _)| e.clone()));
    }
}

/// The delivery reports of one attempt at a batch.
pub struct Attempt {
    remaining: AtomicUsize,
    failed: Mutex<Vec<(usize, KafkaError)>>,
    done: Notify,
}

impl Attempt {
    fn new(messages: usize) -> Self {
        Self {
            remaining: AtomicUsize::new(messages),
            failed: Mutex::new(Vec::new()),
            done: Notify::new(),
        }
    }

    fn report(&self, idx: usize, error: Option<KafkaError>) {
        if let Some(error) = error {
            self.failed.lock().push((idx, error));
        }
        // `notify_one` keeps a permit when nobody waits yet, so a batch whose
        // reports all arrive before `export` starts waiting still wakes it.
        if self.remaining.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.done.notify_one();
        }
    }
}

/// Only what is sure to be transient: the cluster or a partition leader is
/// unreachable or busy, or the attempt ran out of time.
fn retriable(error: &KafkaError) -> bool {
    use RDKafkaErrorCode::*;
    matches!(
        error.rdkafka_error_code(),
        Some(
            MessageTimedOut
                | AllBrokersDown
                | BrokerTransportFailure
                | LeaderNotAvailable
                | NotLeaderForPartition
                | RequestTimedOut
                | NotEnoughReplicas
                | NotEnoughReplicasAfterAppend
                | QueueFull
                | NetworkException
                | PurgeQueue
                | PurgeInflight
        )
    )
}

/// One Kafka message: a serialized blob under its partitioning key.
#[derive(Clone)]
pub struct Message {
    key: SmolStr,
    blob: Bytes,
    /// Items the blob carries, see [`super::blob_items`].
    items: u64,
}

pub struct Messages(Vec<Message>);

pub struct Kafka {
    conf: config::sink::Kafka,
    producers: Vec<Arc<KafkaProducer>>,
    /// Round-robin over `producers`, one pick per attempt.
    next: AtomicUsize,
}

pub type KafkaSink = ExportSink<Kafka>;

impl KafkaSink {
    // `Worker::new` builds from a `Prepared` exporter; this is for tests/kafka_sink_test.rs.
    #[allow(dead_code)]
    pub async fn new(
        sink_config: config::sink::Sink,
        channel: chan::Consumer<payload::Payload>,
    ) -> Result<Self> {
        let exporter = Kafka::new(&sink_config)?;
        Ok(Self::with_exporter(sink_config, channel, exporter))
    }
}

impl Kafka {
    pub(super) fn new(sink_config: &config::sink::Sink) -> Result<Self> {
        let kafka_conf = match &sink_config.config {
            config::sink::Config::Kafka(k) => k,
            _ => unreachable!(),
        };

        let hosts = kafka_conf.hosts.join(",");
        let timeout = Duration::from_secs(kafka_conf.write_timeout_seconds);

        let mut producer = ClientConfig::new();

        let mut producer = producer
            .set("bootstrap.servers", hosts)
            .set("compression.type", kafka_conf.compression.as_str())
            .set("acks", kafka_conf.required_acks.as_str())
            .set("partitioner", kafka_conf.balancer_strategy.as_str())
            // One attempt: librdkafka reports every message within it, so an
            // export always returns, and a longer outage is retried by us.
            .set("message.timeout.ms", timeout.as_millis().to_string())
            // A missing topic should fail as `UnknownTopic` (rejected), not
            // wait out the attempt and time out (retried forever).
            .set(
                "topic.metadata.propagation.max.ms",
                TOPIC_PROPAGATION.min(timeout / 2).as_millis().to_string(),
            );

        if kafka_conf.enable_producer_stats {
            producer = producer.set("statistics.interval.ms", "5000");
        }

        // set for every protocol: left unset, librdkafka connects in plaintext
        // even when the config asks for `ssl`
        producer = producer.set("security.protocol", kafka_conf.security.protocol.as_str());

        if let Some(sasl) = &kafka_conf.sasl {
            producer = producer
                .set("sasl.mechanism", sasl.mechanism.as_str())
                .set("sasl.username", &sasl.username)
                .set("sasl.password", sasl.password.expose()?);
        }

        if let Some(ssl) = &kafka_conf.ssl {
            if let Some(ca_cert_path) = &ssl.ca_cert_path {
                producer = producer.set("ssl.ca.location", ca_cert_path);
            }
            if let Some(cert_path) = &ssl.cert_path {
                producer = producer.set("ssl.certificate.location", cert_path);
            }
            if let Some(key_path) = &ssl.key_path {
                producer = producer.set("ssl.key.location", key_path);
            }
            if let Some(key_password) = ssl.key_password()? {
                producer = producer.set("ssl.key.password", key_password);
            }
            if ssl.insecure_skip_verify {
                producer = producer.set("enable.ssl.certificate.verification", "false");
            } else {
                producer = producer.set("enable.ssl.certificate.verification", "true");
            }
        }

        // What is in flight is bounded by the workers, one batch each, so the
        // message count goes unbounded; the byte cap still holds memory.
        producer = producer
            .set("batch.size", kafka_conf.partition_batch_size.to_string())
            // librdkafka's own cap, checked on the uncompressed message; the
            // broker still applies `max.message.bytes` to the compressed batch.
            .set("message.max.bytes", kafka_conf.max_message_size.to_string())
            .set(
                "batch.num.messages",
                kafka_conf.partition_batch_count.to_string(),
            )
            .set(
                "queue.buffering.max.kbytes",
                kafka_conf.global_queue_size.div_ceil(1024).to_string(),
            )
            .set("queue.buffering.max.messages", "0")
            .set("queue.buffering.max.ms", LINGER.as_millis().to_string());

        let producers = (0..kafka_conf.producer_count)
            .map(|_| {
                let ctx = KafkaSinkContext {
                    sink_id: sink_config.id,
                    sink_name: sink_config.name.clone(),
                };
                producer
                    .create_with_context(ctx)
                    .map(Arc::new)
                    .map_err(|e| anyhow::anyhow!("failed to create kafka producer: {}", e))
            })
            .collect::<Result<_>>()?;

        Ok(Self {
            conf: kafka_conf.clone(),
            producers,
            next: AtomicUsize::new(0),
        })
    }

    fn producer(&self) -> &KafkaProducer {
        let next = self.next.fetch_add(1, Ordering::Relaxed);
        &self.producers[next % self.producers.len()]
    }

    /// Splits one payload by its partitioning key and serializes each shard
    /// into `messages`. Returns the items that could not be.
    fn encode_payload(
        &self,
        pl: payload::Payload,
        messages: &mut Vec<Message>,
        keyless: &mut u64,
    ) -> u64 {
        let items = pl.num_items() as u64;
        let mut one = payload::PayloadBatch::new();
        one.push(pl);
        let shards = match self.conf.key_strategy {
            config::sink::KafkaKeyStrategy::HostId => one.shard(
                payload::AttrLocation::Resource,
                payload::ResourceAttrKey::HostId.as_str(),
            ),
            config::sink::KafkaKeyStrategy::ItemId => one.shard(
                payload::AttrLocation::Resource,
                payload::ResourceAttrKey::ItemId.as_str(),
            ),
            config::sink::KafkaKeyStrategy::ListenerId => one.shard(
                payload::AttrLocation::Scope,
                payload::ScopeAttrKey::ListenerId.as_str(),
            ),
        };
        let shards = match shards {
            Ok(shards) => shards,
            Err(e) => {
                error!(error = %format!("{e:#}"), records = items, "failed to shard record");
                return items;
            }
        };
        let mut errored = 0;
        for (key, shard) in shards {
            let key = key.map_or_else(SmolStr::default, |k| SmolStr::new(k.as_ref()));
            for pl in shard {
                // This shard's share, not the whole payload's count.
                let shard_items = pl.num_items() as u64;
                if !self.conf.allow_empty_key && key.is_empty() {
                    *keyless += shard_items;
                    continue;
                }
                match pl.serialize_as(self.conf.format) {
                    Ok(blobs) => {
                        let count = blobs.len();
                        messages.extend(blobs.into_iter().enumerate().map(|(idx, blob)| Message {
                            key: key.clone(),
                            blob,
                            items: super::blob_items(shard_items, count, idx),
                        }));
                    }
                    Err(e) => {
                        errored += shard_items;
                        error!(error = %format!("{e:#}"), records = shard_items, "failed to serialize record");
                    }
                }
            }
        }
        errored
    }
}

#[async_trait]
impl Exporter for Kafka {
    type Encoded = Messages;

    fn encode(&self, batch: payload::PayloadBatch) -> Encoded<Messages> {
        let mut messages = Vec::new();
        let mut errored = 0;
        let mut keyless = 0;
        for pl in batch {
            if pl.num_items() == 0 {
                continue;
            }
            errored += self.encode_payload(pl, &mut messages, &mut keyless);
        }
        if keyless > 0 {
            warn!(
                records = keyless,
                "empty partitioning key is not allowed, dropping records"
            );
        }
        Encoded {
            data: (!messages.is_empty()).then_some(Messages(messages)),
            filtered: 0,
            errored: errored + keyless,
        }
    }

    fn items(&self, messages: &Messages) -> u64 {
        messages.0.iter().map(|m| m.items).sum()
    }

    /// Produces every message, then waits for all their delivery reports.
    /// Dropped mid-way (a cancelled worker), what was already produced stays
    /// with librdkafka and may still land after the batch is requeued.
    async fn export(&self, messages: &Messages) -> Export<Messages> {
        let producer = self.producer();
        let attempt = Arc::new(Attempt::new(messages.0.len()));
        for (idx, msg) in messages.0.iter().enumerate() {
            let mut record =
                BaseRecord::with_opaque_to(&self.conf.topic, Box::new((attempt.clone(), idx)))
                    .key(msg.key.as_str())
                    .payload(msg.blob.as_ref());
            loop {
                match producer.send(record) {
                    Ok(()) => break,
                    Err((KafkaError::MessageProduction(RDKafkaErrorCode::QueueFull), back)) => {
                        record = back;
                        tokio::time::sleep(QUEUE_FULL_WAIT).await;
                    }
                    Err((e, _)) => {
                        attempt.report(idx, Some(e));
                        break;
                    }
                }
            }
        }
        attempt.done.notified().await;

        let mut failed = std::mem::take(&mut *attempt.failed.lock());
        if failed.is_empty() {
            return Export::Delivered;
        }
        // Reports come in completion order; a retry keeps the batch's order.
        failed.sort_unstable_by_key(|(idx, _)| *idx);
        let none_landed = failed.len() == messages.0.len();
        let mut retry = Vec::new();
        let mut retry_error = None;
        let mut rejected = 0u64;
        let mut reject_error = None;
        for (idx, error) in failed {
            let msg = &messages.0[idx];
            if retriable(&error) {
                retry.push(msg.clone());
                retry_error.get_or_insert(error);
            } else {
                rejected += msg.items;
                reject_error.get_or_insert(error);
            }
        }
        // `Retry` and `Reject` both mean none of it landed.
        match (none_landed, retry_error, reject_error) {
            (true, Some(error), None) => Export::Retry {
                error: error.into(),
                after: None,
            },
            (true, None, Some(error)) => Export::Reject {
                error: error.into(),
            },
            (_, retry_error, reject_error) => Export::Partial {
                retry: retry_error.map(|e| (Messages(retry), e.into())),
                rejected: reject_error.map(|e| (rejected, e.into())),
            },
        }
    }

    /// Every worker has stopped, so the producers only still hold what a
    /// cancelled attempt left behind. Past the timeout, dropping a producer
    /// purges it.
    async fn close(&self) {
        for producer in &self.producers {
            let producer = producer.clone();
            let flushed = tokio::task::spawn_blocking(move || {
                flush(&producer, FLUSH_TIMEOUT).map_err(|e| (e, producer.in_flight_count()))
            })
            .await;
            match flushed {
                Ok(Ok(())) => debug!("kafka producer flushed"),
                Ok(Err((e, in_flight))) => {
                    warn!(error = %e, in_flight, "failed to flush kafka producer")
                }
                Err(e) => error!(error = %e, "kafka producer flush task failed"),
            }
        }
    }
}

/// `Producer::flush` can't be used here: rust-rdkafka implements it as a loop of
/// `rd_kafka_flush(rk, 0)`, and librdkafka only treats `linger.ms` as elapsed *while* a flush
/// call is running. A zero-timeout call returns before the broker threads wake up, so the
/// linger is waited out instead; past the timeout, dropping the producer purges what it still
/// holds. One call with the whole timeout keeps the flag up until the queue is empty. The
/// producer runs librdkafka's event API, so that call only waits: the delivery reports are
/// still served by `ThreadedProducer`'s own polling thread.
fn flush(producer: &KafkaProducer, timeout: Duration) -> Result<(), KafkaError> {
    let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: the pointer is the live handle `producer` owns for the duration of the borrow,
    // and `rd_kafka_flush` is thread-safe.
    let err =
        unsafe { rdkafka::bindings::rd_kafka_flush(producer.client().native_ptr(), timeout_ms) };
    match RDKafkaErrorCode::from(err) {
        RDKafkaErrorCode::NoError => Ok(()),
        code => Err(KafkaError::Flush(code)),
    }
}
