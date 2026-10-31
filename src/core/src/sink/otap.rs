// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! OpenTelemetry (OTAP or OTLP) to one or more destinations.
//!
//! [`ExportSink`] batches; [`Otel::export`] hands each payload of a batch to
//! the destination's [`OtelClient`] and waits for the destination's answer
//! (a `BatchStatus`, or the OTLP response), so a batch is acked only once the
//! destinations have it, and retried by the driver otherwise. The clients keep
//! their own connections and reconnect with their own backoff; whether one is
//! connected is the endpoint's health.

use super::export::{Encoded, Export, ExportSink, Exporter};
use crate::common::internal_metrics::SinkEndpoint;
use crate::common::otap_client::{Answer, Job, NodeMetrics, OtelClient, OtelClientConfig};
use crate::config::sink::{OtelDistribution, OtelShardKey};
use crate::{chan, config, payload};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use flume::Sender;
use futures::future::join_all;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tracing::{Instrument, error};

/// Longest a payload waits for its destination's answer, including the wait
/// for a connection to take it. Past it the payload is retried, and may land
/// twice. A dead peer is normally caught sooner, by the HTTP/2 keepalive.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);

struct Destination {
    client: Arc<OtelClient>,
    jobs: Sender<Job>,
}

/// Which destinations a part goes to.
#[derive(Clone)]
enum To {
    /// Any one: the attempt picks it (`round_robin`, `failover`).
    Any,
    /// Each of these, by index; what is left of a `broadcast` or a shard.
    Each(Vec<usize>),
}

#[derive(Clone)]
struct Part {
    payload: payload::Payload,
    to: To,
    /// Items some destination refused already, and counted as errored. Under
    /// `broadcast` the worst answer wins, so a part is only counted once
    /// every destination has answered for it.
    rejected: u64,
}

impl Part {
    fn new(payload: payload::Payload, to: To) -> Self {
        Self {
            payload,
            to,
            rejected: 0,
        }
    }

    /// Items not settled yet.
    fn items(&self) -> u64 {
        (self.payload.num_items() as u64).saturating_sub(self.rejected)
    }
}

pub struct Parts(Vec<Part>);

pub struct Otel {
    conf: config::sink::OpenTelemetry,
    destinations: Vec<Destination>,
    rr_cursor: AtomicUsize,
}

pub type PatrolliSink = ExportSink<Otel>;

impl PatrolliSink {
    // `Worker::new` builds from a `Prepared` exporter; this is for tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub async fn new(
        conf: config::sink::Sink,
        channel: chan::Consumer<payload::Payload>,
    ) -> Result<Self> {
        let otel_conf = match &conf.config {
            config::sink::Config::OpenTelemetry(o) => o.clone(),
            _ => unreachable!("PatrolliSink::new called with a non-otel config"),
        };
        Ok(Self::with_exporter(conf, channel, Otel::new(otel_conf)))
    }
}

impl Otel {
    pub(super) fn new(conf: config::sink::OpenTelemetry) -> Self {
        let client_config = Arc::new(OtelClientConfig {
            compression: conf.compression,
            tls: conf.tls.clone(),
            write_timeout: conf.write_timeout,
            sink_mode: conf.mode.clone(),
        });
        let destinations = conf
            .destinations
            .iter()
            .map(|dst| {
                let (client, jobs) = OtelClient::new(
                    client_config.clone(),
                    dst.clone(),
                    Arc::new(NodeMetrics::default()),
                );
                client.run(conf.connection_count);
                Destination { client, jobs }
            })
            .collect();
        Self {
            conf,
            destinations,
            rr_cursor: AtomicUsize::new(0),
        }
    }

    /// The destination for this attempt's `Any` parts: the next connected one
    /// in turn, or the first connected one.
    fn pick(&self) -> Option<usize> {
        let n = self.destinations.len();
        let start = match self.conf.distribution {
            OtelDistribution::RoundRobin => self.rr_cursor.fetch_add(1, Ordering::Relaxed) % n,
            _ => 0,
        };
        (0..n)
            .map(|offset| (start + offset) % n)
            .find(|&idx| self.destinations[idx].client.is_connected())
    }

    /// Hands `payload` to destination `idx` and waits for its answer.
    async fn send(&self, idx: usize, payload: payload::Payload) -> Answer {
        let dst = &self.destinations[idx];
        // Skipping a destination that is down saves waiting out the timeout
        // for a connection that may take long to come back.
        if !dst.client.is_connected() {
            return Answer::Retry {
                error: anyhow!("destination {} is not connected", dst.client.dst()),
                after: None,
            };
        }
        let (answer, answered) = oneshot::channel();
        let sent = async {
            dst.jobs.send_async(Job { payload, answer }).await.ok()?;
            answered.await.ok()
        };
        match tokio::time::timeout(ANSWER_TIMEOUT, sent).await {
            Ok(Some(answer)) => answer,
            Ok(None) => Answer::Retry {
                error: anyhow!(
                    "connection to {} closed before it answered",
                    dst.client.dst()
                ),
                after: None,
            },
            Err(_) => Answer::Retry {
                error: anyhow!(
                    "no answer from {} within {ANSWER_TIMEOUT:?}",
                    dst.client.dst()
                ),
                after: None,
            },
        }
    }
}

fn shard_index(value: Option<&str>, n: usize) -> usize {
    match value {
        Some(v) => (xxhash_rust::xxh3::xxh3_128(v.as_bytes()) % n as u128) as usize,
        None => 0,
    }
}

/// What one part's destinations answered in one attempt.
#[derive(Default)]
struct Tally {
    /// Destinations to send the part to again.
    retry: Vec<usize>,
    /// Most items any destination refused.
    refused: u64,
}

#[async_trait]
impl Exporter for Otel {
    type Encoded = Parts;

    /// Merges the batch into as few payloads as the row-id cap allows (one
    /// OTAP batch, one answer each), or shards it, and routes each payload.
    fn encode(&self, mut batch: payload::PayloadBatch) -> Encoded<Parts> {
        let n = self.destinations.len();
        let mut parts = Vec::new();
        let key = match self.conf.distribution {
            OtelDistribution::Broadcast
            | OtelDistribution::RoundRobin
            | OtelDistribution::Failover => {
                let to = match self.conf.distribution {
                    OtelDistribution::Broadcast => To::Each((0..n).collect()),
                    _ => To::Any,
                };
                for pl in batch.batch(None) {
                    if pl.num_items() > 0 {
                        parts.push(Part::new(pl, to.clone()));
                    }
                }
                None
            }
            OtelDistribution::Sharded { key } => Some(key),
        };
        if let Some(key) = key {
            let sharded = match key {
                OtelShardKey::HostId => batch.shard(
                    payload::AttrLocation::Resource,
                    payload::ResourceAttrKey::HostId.as_str(),
                ),
                OtelShardKey::ItemId => batch.shard(
                    payload::AttrLocation::Resource,
                    payload::ResourceAttrKey::ItemId.as_str(),
                ),
                OtelShardKey::EndpointId => batch.shard(
                    payload::AttrLocation::Resource,
                    payload::ResourceAttrKey::EndpointId.as_str(),
                ),
                OtelShardKey::ListenerId => batch.shard(
                    payload::AttrLocation::Scope,
                    payload::ScopeAttrKey::ListenerId.as_str(),
                ),
                OtelShardKey::SourceIp => batch.shard(
                    payload::AttrLocation::Record,
                    payload::RecordAttrKey::SourceAddress.as_str(),
                ),
            };
            match sharded {
                Ok(groups) => {
                    for (key, group) in groups {
                        let idx = shard_index(key.as_ref().map(|k| k.as_ref()), n);
                        for pl in group {
                            if pl.num_items() > 0 {
                                parts.push(Part::new(pl, To::Each(vec![idx])));
                            }
                        }
                    }
                }
                Err(e) => {
                    error!(error = %format!("{e:#}"), "failed to shard payload");
                    return Encoded {
                        data: None,
                        filtered: 0,
                        errored: batch.num_items() as u64,
                    };
                }
            }
        }
        Encoded {
            data: (!parts.is_empty()).then_some(Parts(parts)),
            filtered: 0,
            errored: 0,
        }
    }

    fn items(&self, parts: &Parts) -> u64 {
        parts.0.iter().map(Part::items).sum()
    }

    /// Sends every part to its destinations at once and waits for all the
    /// answers. Dropped mid-way (a cancelled worker), what the clients already
    /// took still goes out, and may land after the batch is requeued.
    async fn export(&self, parts: &Parts) -> Export<Parts> {
        let any = self.pick();
        let mut sends = Vec::new();
        for (i, part) in parts.0.iter().enumerate() {
            match (&part.to, any) {
                (To::Any, Some(idx)) => sends.push((i, idx)),
                (To::Any, None) => {}
                (To::Each(dsts), _) => sends.extend(dsts.iter().map(|&idx| (i, idx))),
            }
        }
        let answers = join_all(
            sends
                .iter()
                .map(|&(i, idx)| self.send(idx, parts.0[i].payload.clone())),
        )
        .await;

        let mut tallies: Vec<Tally> = parts.0.iter().map(|_| Tally::default()).collect();
        let mut landed = false;
        let mut retry_error = None;
        // The longest wait any destination asked for.
        let mut retry_after: Option<Duration> = None;
        let mut reject_error = None;
        for (&(i, idx), answer) in sends.iter().zip(answers) {
            match answer {
                Answer::Delivered => landed = true,
                Answer::Retry { error, after } => {
                    tallies[i].retry.push(idx);
                    retry_error.get_or_insert(error);
                    retry_after = retry_after.max(after);
                }
                Answer::Rejected { items, error } => {
                    landed |= items < parts.0[i].payload.num_items() as u64;
                    tallies[i].refused = tallies[i].refused.max(items);
                    reject_error.get_or_insert(error);
                }
            }
        }

        let mut retry = Vec::new();
        let mut rejected = 0u64;
        for (part, tally) in parts.0.iter().zip(tallies) {
            let refused = tally
                .refused
                .min(part.payload.num_items() as u64)
                .saturating_sub(part.rejected);
            rejected += refused;
            let left = match (&part.to, any) {
                (To::Any, None) => Some(To::Any),
                (To::Any, Some(_)) => (!tally.retry.is_empty()).then_some(To::Any),
                (To::Each(_), _) => (!tally.retry.is_empty()).then_some(To::Each(tally.retry)),
            };
            if let Some(to) = left {
                retry.push(Part {
                    payload: part.payload.clone(),
                    to,
                    rejected: part.rejected + refused,
                });
            }
        }
        if any.is_none() && retry.iter().any(|p| matches!(p.to, To::Any)) {
            retry_error.get_or_insert_with(|| anyhow!("no destination is connected"));
        }

        match (retry.is_empty(), rejected, landed) {
            (true, 0, _) => Export::Delivered,
            // Nothing landed and nothing changed: send it all again.
            (false, 0, false) => Export::Retry {
                error: retry_error.expect("a part is left to retry"),
                after: retry_after,
            },
            (true, _, false) => Export::Reject {
                error: reject_error.expect("items were refused"),
            },
            _ => Export::Partial {
                retry: (!retry.is_empty())
                    .then(|| (Parts(retry), retry_error.expect("a part is left to retry"))),
                rejected: (rejected > 0)
                    .then(|| (rejected, reject_error.expect("items were refused"))),
            },
        }
    }

    fn endpoints(&self) -> Vec<SinkEndpoint> {
        self.destinations
            .iter()
            .map(|dst| SinkEndpoint {
                endpoint: dst.client.dst().to_string(),
                up: dst.client.is_connected(),
            })
            .collect()
    }

    /// Closes the destinations together, so shutdown waits out at most one
    /// drain timeout rather than one per destination.
    async fn close(&self) {
        let mut closes = JoinSet::new();
        for dst in &self.destinations {
            let client = dst.client.clone();
            closes.spawn(async move { client.close().await }.in_current_span());
        }
        while let Some(res) = closes.join_next().await {
            if let Err(e) = res {
                error!(error = %e, "destination close task panicked");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otel_arrow_dfe_pdata::encode::encode_logs_otap_batch;
    use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::{
        BatchArrowRecords, BatchStatus, StatusCode,
        arrow_logs_service_server::{ArrowLogsService, ArrowLogsServiceServer},
        arrow_metrics_service_server::{ArrowMetricsService, ArrowMetricsServiceServer},
        arrow_traces_service_server::{ArrowTracesService, ArrowTracesServiceServer},
    };
    use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
        LogRecord, LogsData, ResourceLogs, ScopeLogs,
    };
    use parking_lot::Mutex;
    use std::collections::VecDeque;
    use std::pin::Pin;
    use tokio_stream::Stream;
    use tonic::{Request, Response, Status, Streaming};

    type StatusStream = Pin<Box<dyn Stream<Item = Result<BatchStatus, Status>> + Send>>;

    /// An OTAP destination that answers each batch with the next scripted
    /// status, then with ok, and counts the batches it got.
    #[derive(Clone, Default)]
    struct Destination {
        script: Arc<Mutex<VecDeque<StatusCode>>>,
        batches: Arc<AtomicUsize>,
    }

    impl Destination {
        fn answer(&self, request: Request<Streaming<BatchArrowRecords>>) -> StatusStream {
            let this = self.clone();
            let mut inbound = request.into_inner();
            Box::pin(async_stream::try_stream! {
                while let Some(bar) = inbound.message().await? {
                    this.batches.fetch_add(1, Ordering::Relaxed);
                    let code = this.script.lock().pop_front().unwrap_or(StatusCode::Ok);
                    yield BatchStatus {
                        batch_id: bar.batch_id,
                        status_code: code as i32,
                        status_message: String::new(),
                    };
                }
            })
        }

        /// Serves on a free port, which it returns.
        async fn serve(&self) -> String {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap().to_string();
            let incoming = async_stream::stream! {
                loop {
                    yield listener.accept().await.map(|(stream, _)| stream);
                }
            };
            let router = tonic::transport::Server::builder()
                .add_service(ArrowLogsServiceServer::new(self.clone()))
                .add_service(ArrowTracesServiceServer::new(self.clone()))
                .add_service(ArrowMetricsServiceServer::new(self.clone()));
            tokio::spawn(router.serve_with_incoming(incoming));
            addr
        }
    }

    #[async_trait]
    impl ArrowLogsService for Destination {
        type ArrowLogsStream = StatusStream;
        async fn arrow_logs(
            &self,
            request: Request<Streaming<BatchArrowRecords>>,
        ) -> Result<Response<StatusStream>, Status> {
            Ok(Response::new(self.answer(request)))
        }
    }

    #[async_trait]
    impl ArrowTracesService for Destination {
        type ArrowTracesStream = StatusStream;
        async fn arrow_traces(
            &self,
            request: Request<Streaming<BatchArrowRecords>>,
        ) -> Result<Response<StatusStream>, Status> {
            Ok(Response::new(self.answer(request)))
        }
    }

    #[async_trait]
    impl ArrowMetricsService for Destination {
        type ArrowMetricsStream = StatusStream;
        async fn arrow_metrics(
            &self,
            request: Request<Streaming<BatchArrowRecords>>,
        ) -> Result<Response<StatusStream>, Status> {
            Ok(Response::new(self.answer(request)))
        }
    }

    /// Refuses connections straight away.
    const UNREACHABLE: &str = "127.0.0.1:1";

    fn otel(destinations: &[&str], distribution: &str) -> Otel {
        let mut conf: config::sink::OpenTelemetry = serde_yaml_ng::from_str(&format!(
            "destinations: {destinations:?}\ndistribution: {{strategy: {distribution}}}\ncompression: none\n"
        ))
        .unwrap();
        conf.validate().unwrap();
        Otel::new(conf)
    }

    /// Waits until the destinations that can connect have.
    async fn connected(otel: &Otel, up: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while otel.endpoints().iter().filter(|e| e.up).count() < up {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("destination never connected");
    }

    /// Two payloads of `records` log records each, encoded as one batch.
    fn encode(otel: &Otel, records: usize) -> Parts {
        let logs = LogsData {
            resource_logs: vec![ResourceLogs {
                resource: None,
                scope_logs: vec![ScopeLogs {
                    scope: None,
                    log_records: vec![LogRecord::default(); records],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        let mut batch = payload::PayloadBatch::new();
        for _ in 0..2 {
            batch.push(payload::Payload::new(
                encode_logs_otap_batch(&logs).unwrap(),
            ));
        }
        let encoded = otel.encode(batch);
        assert_eq!(encoded.errored + encoded.filtered, 0);
        encoded.data.unwrap()
    }

    #[tokio::test]
    async fn a_batch_is_delivered_once_the_destination_answers_ok() {
        let dst = Destination::default();
        let addr = dst.serve().await;
        let otel = otel(&[&addr], "round_robin");
        connected(&otel, 1).await;
        let parts = encode(&otel, 3);
        // the two payloads merge into one OTAP batch
        assert_eq!(parts.0.len(), 1);
        assert_eq!(otel.items(&parts), 6);
        assert!(matches!(otel.export(&parts).await, Export::Delivered));
        assert_eq!(dst.batches.load(Ordering::Relaxed), 1);
        otel.close().await;
    }

    #[tokio::test]
    async fn a_non_ok_status_rejects_the_batch() {
        let dst = Destination::default();
        dst.script.lock().push_back(StatusCode::InvalidArgument);
        let addr = dst.serve().await;
        let otel = otel(&[&addr], "failover");
        connected(&otel, 1).await;
        let parts = encode(&otel, 3);
        let Export::Reject { error } = otel.export(&parts).await else {
            panic!("a refused batch is not retried");
        };
        assert!(
            format!("{error:#}").contains("INVALID_ARGUMENT"),
            "{error:#}"
        );
        // the stream carries on: the next batch lands
        assert!(matches!(otel.export(&parts).await, Export::Delivered));
        otel.close().await;
    }

    #[tokio::test]
    async fn with_no_destination_connected_the_batch_is_retried() {
        let otel = otel(&[UNREACHABLE], "round_robin");
        let parts = encode(&otel, 3);
        assert!(matches!(otel.export(&parts).await, Export::Retry { .. }));
        otel.close().await;
    }

    #[tokio::test]
    async fn broadcast_retries_only_the_destinations_that_failed() {
        let dst = Destination::default();
        let addr = dst.serve().await;
        let otel = otel(&[&addr, UNREACHABLE], "broadcast");
        connected(&otel, 1).await;
        let parts = encode(&otel, 3);
        let Export::Partial {
            retry: Some((left, _)),
            rejected: None,
        } = otel.export(&parts).await
        else {
            panic!("one destination took it, the other is down");
        };
        // not counted as sent until every destination has it
        assert_eq!(otel.items(&left), 6);
        assert!(matches!(&left.0[0].to, To::Each(dsts) if dsts == &[1]));
        assert!(matches!(otel.export(&left).await, Export::Retry { .. }));
        assert_eq!(dst.batches.load(Ordering::Relaxed), 1);
        otel.close().await;
    }

    #[tokio::test]
    async fn broadcast_counts_a_refusal_once_every_destination_answered() {
        let (a, b) = (Destination::default(), Destination::default());
        a.script.lock().push_back(StatusCode::InvalidArgument);
        let addrs = [a.serve().await, b.serve().await];
        let otel = otel(&[&addrs[0], &addrs[1]], "broadcast");
        connected(&otel, 2).await;
        let parts = encode(&otel, 3);
        // b took it, a refused it: the worst answer wins
        let Export::Partial {
            retry: None,
            rejected: Some((6, _)),
        } = otel.export(&parts).await
        else {
            panic!("a refusal from one destination rejects the part");
        };
        otel.close().await;
    }
}
