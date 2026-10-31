// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::item_stats;
use super::v1::inter_cluster_service_client::InterClusterServiceClient;
use super::v1::inter_cluster_service_server::{InterClusterService, InterClusterServiceServer};
use super::v1::{
    CaptureMessage, CaptureRequest, ForwardAck, ForwardStatus, ItemStatsRequest, ItemStatsResponse,
    NodeIdentity, PayloadBatch, SignalType,
};
use crate::cache::item::StatsSnapshot;
use crate::chan::SenderChannel;
use crate::common::internal_metrics::{
    Forwarder as ForwarderMetrics, ForwarderServer as ForwarderServerMetrics, INTERNAL_METRICS,
    NodeMetrics,
};
use crate::listener::item_listener_annotate;
use crate::{cache, common, config, payload};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, FixedOffset};
use flume::{Receiver, Sender};
use futures::{Stream, StreamExt};
use otel_arrow_dfe_pdata::otap::from_record_messages;
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::BatchArrowRecords;
use otel_arrow_dfe_pdata::{Consumer, OtapArrowRecords};
use prost::Message;
use rustc_hash::{FxBuildHasher, FxHashMap};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{RwLock, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::{CancellationToken, DropGuard};
use tonic::metadata::{Ascii, MetadataValue};
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::{Channel, Endpoint};
use tonic::{Request, Response, Status, Streaming};
use tracing::{Instrument, debug, info, info_span, warn};
use uuid::Uuid;

#[expect(dead_code, reason = "hello handshake omitted for now (61d7c5f4)")]
const HELLO_TIMEOUT: Duration = Duration::from_secs(3);

/// Topic name of the inter-node forward channel (listeners broadcast
/// `ForwarderPayload`s here; the [`InterComm`] receiver bridge drains it).
pub static FORWARDER_CHANNEL_NAME: &str = "forwarder";

/// Upper bound for a single decoded gRPC message. The forwarder merges up to
/// `batch_size` (≤ 65535) items into one `BatchArrowRecords`, which can far
/// exceed tonic's 4 MiB default. 2 GiB is protobuf's practical per-message
/// ceiling (implementations use signed 32-bit lengths internally).
pub const MAX_GRPC_MESSAGE_SIZE: usize = 2 * 1024 * 1024 * 1024;

/// Shared-secret auth for the inter-node service, from `grpc.bearer` in the
/// cluster config (`None` disables auth). Holds the precomputed
/// `authorization` header value: clients stamp it on outgoing requests with
/// [`Bearer::apply`], and the server validates it as a tonic [`Interceptor`].
/// All nodes in a cluster must be configured with the same token.
#[derive(Clone)]
pub struct Bearer(Option<MetadataValue<Ascii>>);

impl Bearer {
    pub fn new(token: Option<&str>) -> Result<Self> {
        let value = token
            .map(|t| format!("Bearer {t}").parse())
            .transpose()
            .context("grpc bearer token is not a valid header value")?;
        Ok(Self(value))
    }

    pub(super) fn apply<T>(&self, request: &mut Request<T>) {
        if let Some(value) = &self.0 {
            request
                .metadata_mut()
                .insert("authorization", value.clone());
        }
    }
}

impl Interceptor for Bearer {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
        let Some(expected) = &self.0 else {
            return Ok(request);
        };
        match request.metadata().get("authorization") {
            Some(got) if got == expected => Ok(request),
            Some(_) => {
                warn!(reason = "invalid_token", "rejected inter-node request");
                Err(Status::unauthenticated("invalid bearer token"))
            }
            None => {
                warn!(reason = "missing_token", "rejected inter-node request");
                Err(Status::unauthenticated("missing bearer token"))
            }
        }
    }
}

async fn build_channel(dst: &str, tls: Option<&config::Tls>) -> Result<Channel> {
    let scheme = if tls.is_some() { "https" } else { "http" };
    let mut endpoint = Endpoint::from_shared(format!("{scheme}://{dst}"))?
        .tcp_nodelay(true)
        .http2_keep_alive_interval(Duration::from_secs(6))
        .keep_alive_while_idle(true)
        .keep_alive_timeout(Duration::from_secs(3))
        .connect_timeout(Duration::from_secs(3));
    if let Some(tls_conf) = tls {
        endpoint = match crate::common::tls::build_tonic_client_tls(tls_conf)? {
            crate::common::tls::TonicClientTls::Verified(config) => endpoint.tls_config(*config)?,
            crate::common::tls::TonicClientTls::Insecure(config, verifier) => {
                endpoint.tls_config_with_verifier(*config, verifier)?
            }
        };
    }
    let channel = endpoint.connect_lazy();
    Ok(channel)
}

#[async_trait]
pub trait InterCommManager: Send + Sync {
    async fn close_all(&self);
    async fn add(&self, node_id: Uuid, dst: String) -> Result<()>;
    async fn remove(&self, node_id: Uuid);
}

type PHashMap<K, V> = papaya::HashMap<K, V, FxBuildHasher>;

#[derive(Default)]
struct InternalMetrics {
    total_time_spent: AtomicU64,
    engine_time_spent: AtomicU64,
    send_wait_time: AtomicU64,
    receive_wait_time: AtomicU64,
    total_records: AtomicU64,
    total_records_sent: AtomicU64,
    total_records_dropped: AtomicU64,
    total_records_errored: AtomicU64,
    nodes: PHashMap<Uuid, Arc<NodeMetrics>>,
}

impl InternalMetrics {
    fn get(&self) -> ForwarderMetrics {
        ForwarderMetrics {
            total_time_spent: Duration::from_nanos(self.total_time_spent.load(Ordering::Relaxed)),
            engine_time_spent: Duration::from_nanos(self.engine_time_spent.load(Ordering::Relaxed)),
            send_wait_time: Duration::from_nanos(self.send_wait_time.load(Ordering::Relaxed)),
            receive_wait_time: Duration::from_nanos(self.receive_wait_time.load(Ordering::Relaxed)),
            total_records: self.total_records.load(Ordering::Relaxed),
            total_records_sent: self.total_records_sent.load(Ordering::Relaxed),
            total_records_dropped: self.total_records_dropped.load(Ordering::Relaxed),
            total_records_errored: self.total_records_errored.load(Ordering::Relaxed),
            nodes: self
                .nodes
                .pin()
                .iter()
                .map(|(id, metrics)| metrics.get(*id))
                .collect(),
        }
    }
}

pub struct InterComm {
    conf: config::cluster::InterNode,
    node_id: Uuid,
    #[expect(dead_code, reason = "hello handshake omitted for now (61d7c5f4)")]
    node_name: String,
    conf_cache: cache::Cache,
    bearer: Bearer,
    internal_metrics: Arc<InternalMetrics>,
    clients: RwLock<FxHashMap<Uuid, InterCommClient>>,
    forward_receiver: Receiver<payload::ForwarderPayload>,
}

pub struct CaptureStream {
    pub receiver: Receiver<CaptureMessage>,
    _guard: DropGuard,
}

/// One node's answer to an item-stats query.
///
/// Doesn't need serde/toschema, this will be used to integrate to existing
/// item get API.
///
/// `error` set means the answer is incomplete, and the other fields still carry
/// whatever did arrive: a stream that broke midway leaves `since` along with the
/// chunks that landed before the break, while a node that never answered at all
/// leaves `since` as `None`. `node_id` is always known — it comes from the cluster
/// config rather than from the response, so even an unreachable node is named.
pub struct ItemStats {
    /// Keyed by item id.
    pub stats: FxHashMap<Uuid, StatsSnapshot>,
    pub node_id: Uuid,
    /// When this node's counters started. `None` only when it never answered:
    /// without it every counter in the answer would be misdated, so it is not
    /// invented.
    pub since: Option<DateTime<FixedOffset>>,
    pub error: Option<String>,
}

impl ItemStats {
    /// A node that could not be reached, or gave up partway with nothing usable.
    pub(super) fn failed(node_id: Uuid, error: impl std::fmt::Display) -> Self {
        Self {
            stats: FxHashMap::default(),
            node_id,
            since: None,
            error: Some(error.to_string()),
        }
    }
}

impl InterComm {
    pub fn new(
        forward_receiver: Receiver<payload::ForwarderPayload>,
        conf: config::cluster::InterNode,
        node_id: Uuid,
        node_name: String,
        conf_cache: cache::Cache,
    ) -> Result<Arc<Self>> {
        let bearer = Bearer::new(conf.grpc.bearer.as_deref())?;
        let comm = Arc::new(Self {
            internal_metrics: Arc::new(InternalMetrics::default()),
            clients: RwLock::new(FxHashMap::default()),
            forward_receiver,
            conf,
            node_id,
            node_name,
            conf_cache,
            bearer,
        });
        tokio::spawn(comm.clone().receiver_bridge());
        Ok(comm)
    }

    async fn receiver_bridge(self: Arc<Self>) {
        loop {
            let n_iter = Instant::now();
            let Ok(payload) = self.forward_receiver.recv_async().await else {
                break;
            };
            self.internal_metrics
                .receive_wait_time
                .fetch_add(n_iter.elapsed().as_nanos() as u64, Ordering::Relaxed);
            let pl_len = payload.payload.num_items() as u64;
            self.internal_metrics
                .total_records
                .fetch_add(pl_len, Ordering::Relaxed);
            let n = Instant::now();
            let target = {
                let client_guard = self.clients.read().await;
                match client_guard.get(&payload.node_id) {
                    None => {
                        debug!(peer_node_id = %payload.node_id, "no client for node, dropping payload");
                        None
                    }
                    Some(client) => Some((
                        client.forward_sender.clone(),
                        client.connected.clone(),
                        client.metrics.clone(),
                    )),
                }
            };
            self.internal_metrics
                .engine_time_spent
                .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
            let Some((sender, mut connected, node_metrics)) = target else {
                self.internal_metrics
                    .total_records_errored
                    .fetch_add(pl_len, Ordering::Relaxed);
                self.internal_metrics
                    .total_time_spent
                    .fetch_add(n_iter.elapsed().as_nanos() as u64, Ordering::Relaxed);
                continue;
            };
            let n = Instant::now();
            tokio::select! {
                res = sender.send_async(payload.payload) => {
                    match res {
                        Ok(()) => {
                            self.internal_metrics
                                .total_records_sent
                                .fetch_add(pl_len, Ordering::Relaxed);
                        }
                        Err(e) => {
                            self.internal_metrics
                                .total_records_errored
                                .fetch_add(pl_len, Ordering::Relaxed);
                            warn!(error = %e, "failed to forward payload")
                        }
                    }
                }
                // without a live stream nobody drains the rendezvous channel,
                // and blocking here would stall forwarding to every other
                // node: drop until the peer reconnects
                _ = connected.wait_for(|live| *live == 0) => {
                    self.internal_metrics
                        .total_records_dropped
                        .fetch_add(pl_len, Ordering::Relaxed);
                    node_metrics
                        .total_records_dropped
                        .fetch_add(pl_len, Ordering::Relaxed);
                    debug!(
                        peer_node_id = %payload.node_id,
                        "dropping payload, no live stream to peer"
                    );
                }
            }
            self.internal_metrics
                .send_wait_time
                .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
            self.internal_metrics
                .total_time_spent
                .fetch_add(n_iter.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
    }

    #[expect(dead_code, reason = "hello handshake omitted for now (61d7c5f4)")]
    async fn hello(&self, transport: &Channel, expect_node_id: Uuid, dst: &str) -> Result<()> {
        let mut client = InterClusterServiceClient::new(transport.clone());
        let identity = NodeIdentity {
            node_id: self.node_id.as_bytes().to_vec().into(),
            node_name: self.node_name.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        };
        let mut request = Request::new(identity);
        self.bearer.apply(&mut request);
        let peer = tokio::time::timeout(HELLO_TIMEOUT, client.hello(request))
            .await
            .map_err(|_| anyhow::anyhow!("hello to {dst} timed out"))?
            .map_err(|e| anyhow::anyhow!("hello to {dst} failed: {e}"))?
            .into_inner();

        let peer_id = Uuid::from_slice(peer.node_id.as_ref())
            .map_err(|e| anyhow::anyhow!("peer {dst} returned an invalid node_id: {e}"))?;
        if peer_id != expect_node_id {
            anyhow::bail!(
                "peer {dst} identified as node {peer_id} ({}), expected node {expect_node_id}",
                peer.node_name
            );
        }
        if peer.version != env!("CARGO_PKG_VERSION") {
            anyhow::bail!(
                "peer {dst} runs version {}, expected {}",
                peer.version,
                env!("CARGO_PKG_VERSION")
            );
        }
        info!(peer_node_id = %peer_id, peer_node = %peer.node_name, dst, "verified peer node");
        Ok(())
    }

    /// Item stats from every node in the cluster, one element per node.
    ///
    /// This node answers first and from its own cache — it holds stats like any other
    /// node, and the caller asking it shouldn't get an answer that omits it. An
    /// unreachable peer still appears, with `error` set: "no data" and "we couldn't
    /// ask" look identical in a UI otherwise, and the second one is a cluster problem
    /// rather than an item problem.
    ///
    /// Self comes first; peers follow in the order they finish. Every element names
    /// its node, so a caller wanting a stable order sorts on `node_id`.
    ///
    /// An empty `item_ids` asks for nothing, so it returns an empty `Vec` without
    /// contacting any node.
    pub async fn item_stats(
        &self,
        req: ItemStatsRequest,
        timeout: Duration,
    ) -> Result<Vec<ItemStats>> {
        if req.item_ids.is_empty() {
            return Ok(Vec::new());
        }
        // Locally first, so a malformed id fails the whole call here rather than
        // reaching every peer as a request none of them can serve.
        let mut out = vec![item_stats::local_stats(
            self.node_id,
            &self.conf_cache.item,
            &req,
        )?];

        let mut joins = JoinSet::new();
        {
            // Scoped to the spawn loop. It's a tokio `RwLock`, and holding it across
            // the peer streams would stall `add`/`remove` for the whole fan-out.
            let clients = self.clients.read().await;
            out.reserve(clients.len());
            for client in clients.values() {
                let (node_id, transport, bearer) = (
                    client.node_id,
                    client.transport.clone(),
                    client.bearer.clone(),
                );
                let req = req.clone();
                joins.spawn(async move {
                    peer_item_stats(node_id, transport, bearer, req, timeout).await
                });
            }
        }

        while let Some(joined) = joins.join_next().await {
            match joined {
                Ok(stats) => out.push(stats),
                // A panicked task loses which node it was for, so this can't be
                // attributed. Better surfaced than silently short by one.
                Err(e) => warn!(error = %e, "item stats task failed"),
            }
        }
        Ok(out)
    }

    pub async fn capture(&self, req: CaptureRequest) -> Result<CaptureStream> {
        let processor_id =
            Uuid::from_slice(req.processor_id.as_ref()).context("failed to parse processor_id")?;
        let (sender, receiver) = flume::bounded(0);
        let token = CancellationToken::new();
        // subscribe on this node first: it captures traffic too, and an
        // unknown processor fails fast before fanning out to peers (processor
        // config is cluster-wide)
        let subs = self
            .conf_cache
            .processor
            .subscribe_capture(processor_id, &req)
            .ok_or_else(|| anyhow::anyhow!("processor {processor_id} not found"))?;
        tokio::spawn(local_capture_bridge(subs, sender.clone(), token.clone()));
        for client in self.clients.read().await.values() {
            if let Err(e) = client
                .capture(req.clone(), sender.clone(), token.clone())
                .instrument(info_span!("capture", peer_node_id = %client.node_id))
                .await
            {
                warn!(peer_node_id = %client.node_id, error = %format!("{e:#}"), "failed to capture from peer");
            };
        }
        Ok(CaptureStream {
            receiver,
            _guard: token.drop_guard(),
        })
    }

    /// Drops the counters of peers that are no longer clients.
    ///
    /// `remove` clears its own entry, but a peer deleted from the cluster while
    /// this node was standby never reaches `remove`: `close_all` empties the
    /// client map without touching the counters, and coming back to main only
    /// re-adds what is configured now. An empty client map means standby, not
    /// "no peers", so the counters are kept then and carry on afterwards.
    async fn retain_live_peers(&self) {
        let clients = self.clients.read().await;
        if clients.is_empty() {
            return;
        }
        self.internal_metrics
            .nodes
            .pin()
            .retain(|id, _| clients.contains_key(id));
    }

    pub async fn spawn_metrics_reporter(
        self: &Arc<Self>,
        metrics_report_duration: Duration,
    ) -> JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            let mut n = Instant::now();
            loop {
                if this.forward_receiver.is_disconnected() {
                    break;
                }
                if n.elapsed() >= metrics_report_duration {
                    this.retain_live_peers().await;
                    let metrics = this.internal_metrics.get();
                    INTERNAL_METRICS
                        .forwarder
                        .store(Arc::new((Instant::now(), metrics)));
                    n = Instant::now();
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
    }
}

/// Ask one peer for its item stats and fold its stream into a single answer.
///
/// Takes the channel and bearer by value rather than borrowing an `InterCommClient`,
/// because the fan-out runs these on a `JoinSet` and so needs `'static` futures. Both
/// clones are cheap: a `Channel` is a handle, and `Bearer` holds one header value.
///
/// Never returns an error — every failure becomes `error` on the answer, so a bad peer
/// costs its own row and nothing else.
async fn peer_item_stats(
    node_id: Uuid,
    transport: Channel,
    bearer: Bearer,
    req: ItemStatsRequest,
    timeout: Duration,
) -> ItemStats {
    // Raised past tonic's 4 MiB default, unlike the crate's other clients. The
    // server sizes its chunks to fit that default, with one documented exception: an
    // item whose `detail_json` alone busts the budget ships alone and oversized.
    // Accepting it here is what makes that item arrive instead of failing the node.
    let mut client =
        InterClusterServiceClient::new(transport).max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE);
    let mut request = Request::new(req);
    bearer.apply(&mut request);

    let fetch = async {
        match client.get_item_stats(request).await {
            Ok(response) => item_stats::collect_node(node_id, response.into_inner()).await,
            Err(e) => ItemStats::failed(node_id, e),
        }
    };
    match tokio::time::timeout(timeout, fetch).await {
        Ok(stats) => stats,
        Err(_) => ItemStats::failed(node_id, format!("timed out after {}s", timeout.as_secs())),
    }
}

/// Pump the local node's capture broadcast into the merged capture stream,
/// mirroring what `CaptureClient` does for a peer's gRPC stream.
async fn local_capture_bridge(
    mut subs: cache::processor::CaptureSubscription,
    sender: Sender<CaptureMessage>,
    token: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = token.cancelled() => break,
            msg = subs.recv() => match msg {
                Ok(m) => if sender.send_async(m).await.is_err() { break },
                Err(RecvError::Lagged(skipped)) => {
                    debug!(skipped, "capture subscriber lagged");
                    continue;
                }
                Err(RecvError::Closed) => break,
            }
        }
    }
}

#[async_trait]
impl InterCommManager for InterComm {
    async fn close_all(&self) {
        let clients = {
            let mut clients = self.clients.write().await;
            std::mem::take(&mut *clients)
        };
        if !clients.is_empty() {
            debug!(count = clients.len(), "closing all peers");
        }
        let mut joins = JoinSet::new();
        for (_, client) in clients {
            joins.spawn(async move {
                client.forwarder.close().await;
            });
        }
        // Not `join_all`, which re-raises a panic: the hook reported it, and
        // the teardown this is part of must go on.
        while let Some(res) = joins.join_next().await {
            if let Err(e) = res {
                debug!(error = %e, "peer close task panicked");
            }
        }
    }

    async fn add(&self, node_id: Uuid, dst: String) -> Result<()> {
        let prev_client = {
            let mut clients = self.clients.write().await;
            if clients.get(&node_id).map(|c| c.dst == dst).unwrap_or(false) {
                return Ok(());
            }
            let transport = build_channel(&dst, self.conf.grpc.tls.as_ref()).await?;
            let token = CancellationToken::new();
            let (forward_sender, forward_receiver) = flume::bounded(0);
            let metrics = self
                .internal_metrics
                .nodes
                .pin()
                .get_or_insert_with(node_id, || Arc::new(NodeMetrics::default()))
                .clone();
            let forwarder = super::forwarder::ForwarderClient::new(
                forward_receiver,
                token.clone(),
                transport.clone(),
                self.conf.forwarder.clone(),
                self.bearer.clone(),
                metrics.clone(),
            );
            let forwarder_clone = forwarder.clone();
            let forwarder_span = info_span!("forwarder", peer_node_id = %node_id, dst = %dst);
            let client = InterCommClient {
                forward_sender,
                connected: forwarder.connection_watch(),
                metrics,
                node_id,
                dst,
                transport,
                bearer: self.bearer.clone(),
                forwarder,
            };
            for _ in 0..self.conf.forwarder.worker_count {
                let forwarder_clone = forwarder_clone.clone();
                tokio::spawn(
                    async move {
                        forwarder_clone.run().await;
                    }
                    .instrument(forwarder_span.clone()),
                );
            }
            clients.insert(node_id, client)
        };
        if let Some(prev_client) = prev_client {
            debug!(peer_node_id = %node_id, dst = %prev_client.dst, "peer replaced");
            prev_client.forwarder.close().await;
        } else {
            debug!(peer_node_id = %node_id, "peer added");
        }
        Ok(())
    }

    async fn remove(&self, node_id: Uuid) {
        if let Some(client) = self.clients.write().await.remove(&node_id) {
            debug!(peer_node_id = %node_id, dst = %client.dst, "peer removed");
            client.forwarder.close().await;
        };
        self.internal_metrics.nodes.pin().remove(&node_id);
    }
}

struct InterCommClient {
    forward_sender: Sender<payload::Payload>,
    /// Mirrors the forwarder's live-stream count; the receiver bridge selects
    /// on this to drop instead of block while the peer is disconnected.
    connected: watch::Receiver<usize>,
    /// Per-node counters, shared with the `ForwarderClient` and registered in
    /// the aggregate's `nodes` map for snapshots.
    metrics: Arc<NodeMetrics>,
    node_id: Uuid,
    dst: String,
    transport: Channel,
    bearer: Bearer,

    forwarder: Arc<super::forwarder::ForwarderClient>,
}

impl InterCommClient {
    async fn capture(
        &self,
        req: CaptureRequest,
        sender: Sender<CaptureMessage>,
        token: CancellationToken,
    ) -> Result<()> {
        let client =
            super::capture::CaptureClient::new(self.transport.clone(), self.bearer.clone());
        client.capture(req, sender, token).await?;
        Ok(())
    }
}

/// Maps a failure to decode or annotate a forwarded batch to the status sent
/// back to the peer, logging it here since otherwise only the peer sees it.
fn reject_forwarded(e: impl std::fmt::Display) -> Status {
    warn!(error = %e, "forwarded batch rejected");
    Status::internal(e.to_string())
}

/// The add_service-ready tower service for the inter-node gRPC server: the
/// generated tonic service wrapped with bearer-token validation.
pub type InterCommService = InterceptedService<InterClusterServiceServer<InterCommServer>, Bearer>;

#[derive(Default)]
struct ServerMetrics {
    engine_time_spent: AtomicU64,
    send_wait_time: AtomicU64,
    total_records: AtomicU64,
    total_records_sent: AtomicU64,
    total_records_errored: AtomicU64,
    total_bytes: AtomicU64,
}

impl ServerMetrics {
    fn get(&self) -> ForwarderServerMetrics {
        ForwarderServerMetrics {
            engine_time_spent: Duration::from_nanos(self.engine_time_spent.load(Ordering::Relaxed)),
            send_wait_time: Duration::from_nanos(self.send_wait_time.load(Ordering::Relaxed)),
            total_records: self.total_records.load(Ordering::Relaxed),
            total_records_sent: self.total_records_sent.load(Ordering::Relaxed),
            total_records_errored: self.total_records_errored.load(Ordering::Relaxed),
            total_bytes: self.total_bytes.load(Ordering::Relaxed),
        }
    }
}

pub struct InterCommServer {
    node_id: Uuid,
    node_name: String,
    conf_cache: cache::Cache,
    processor_sender: SenderChannel<&'static str, payload::ProcessorPayload>,
    metrics: Arc<ServerMetrics>,
    /// The token that stops the serve task. The server's graceful shutdown
    /// waits for every open stream, and a peer's forwarder holds its
    /// `ForwardPayloads` streams open until its own node closes them, so the
    /// long-lived streams end themselves on this instead.
    shutdown: CancellationToken,
}

impl InterCommServer {
    /// Build the inter-node gRPC service (`Hello`, `ForwardPayloads`,
    /// `ProcessorCapture`), enforcing bearer auth when `conf.bearer` is set and
    /// accepting messages up to [`MAX_GRPC_MESSAGE_SIZE`]. The caller serves it
    /// via `tonic::transport::Server::builder().add_service(...)`. Also spawns
    /// the metrics reporter, which runs until `shutdown` (the same token that
    /// stops the serve task) is cancelled; the streaming calls end on it too.
    pub fn new_service(
        node_id: Uuid,
        node_name: String,
        conf_cache: cache::Cache,
        processor_sender: SenderChannel<&'static str, payload::ProcessorPayload>,
        conf: &config::cluster::Grpc,
        metrics_report_duration: Duration,
        shutdown: CancellationToken,
    ) -> Result<InterCommService> {
        let bearer = Bearer::new(conf.bearer.as_deref())?;
        let metrics = Arc::new(ServerMetrics::default());
        {
            let metrics = metrics.clone();
            let shutdown = shutdown.clone();
            tokio::spawn(async move {
                let mut n = Instant::now();
                loop {
                    if shutdown.is_cancelled() {
                        break;
                    }
                    if n.elapsed() >= metrics_report_duration {
                        INTERNAL_METRICS
                            .forwarder_server
                            .store(Arc::new((Instant::now(), metrics.get())));
                        n = Instant::now();
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            });
        }
        let service = InterClusterServiceServer::new(Self {
            node_id,
            node_name,
            conf_cache,
            processor_sender,
            metrics,
            shutdown,
        })
        .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE);
        Ok(InterceptedService::new(service, bearer))
    }
}

#[async_trait]
impl InterClusterService for InterCommServer {
    async fn hello(
        &self,
        request: Request<NodeIdentity>,
    ) -> Result<Response<NodeIdentity>, Status> {
        let version = env!("CARGO_PKG_VERSION");
        // TODO: rethink of how to deal with cross-version compatibility
        if version != request.get_ref().version {
            warn!(
                peer_version = %request.get_ref().version,
                version,
                "peer version mismatch"
            );
            Err(Status::failed_precondition("version mismatch"))
        } else {
            Ok(Response::new(NodeIdentity {
                version: version.to_string(),
                node_id: self.node_id.as_bytes().to_vec().into(),
                node_name: self.node_name.clone(),
            }))
        }
    }

    type ForwardPayloadsStream = Pin<Box<dyn Stream<Item = Result<ForwardAck, Status>> + Send>>;

    // TODO: payload batch
    async fn forward_payloads(
        &self,
        request: Request<Streaming<PayloadBatch>>,
    ) -> Result<Response<Self::ForwardPayloadsStream>, Status> {
        let mut inbound = request.into_inner();
        let mut log_consumer = Consumer::default();
        let mut metric_consumer = Consumer::default();
        let mut trace_consumer = Consumer::default();
        let conf_cache = self.conf_cache.clone();
        let processor_sender = self.processor_sender.clone();
        let metrics = self.metrics.clone();
        let shutdown = self.shutdown.clone();
        let out = async_stream::try_stream! {
            // Ends the call between batches on shutdown: the peer sees its ack
            // stream end and reconnects with backoff, as it does to a node
            // that went away.
            while let Some(pl_batch) = tokio::select! {
                biased;
                _ = shutdown.cancelled() => Ok(None),
                msg = inbound.message() => msg,
            }
            .inspect_err(|e| debug!(error = %e, "forward stream broken"))?
            {
                metrics
                    .total_bytes
                    .fetch_add(pl_batch.otap_payload.len() as u64, Ordering::Relaxed);
                let n = Instant::now();
                let mut bar = BatchArrowRecords::decode(pl_batch.otap_payload.as_ref())
                    .map_err(reject_forwarded)?;
                let mut pl = match SignalType::try_from(pl_batch.signal) {
                    Ok(SignalType::Logs) => {
                        let msgs = log_consumer
                            .consume_bar(&mut bar)
                            .map_err(reject_forwarded)?;
                        OtapArrowRecords::Logs(
                            from_record_messages(msgs).map_err(reject_forwarded)?,
                        )
                    }
                    Ok(SignalType::Metrics) => {
                        let msgs = metric_consumer
                            .consume_bar(&mut bar)
                            .map_err(reject_forwarded)?;
                        OtapArrowRecords::Metrics(
                            from_record_messages(msgs).map_err(reject_forwarded)?,
                        )
                    }
                    Ok(SignalType::Traces) => {
                        let msgs = trace_consumer
                            .consume_bar(&mut bar)
                            .map_err(reject_forwarded)?;
                        OtapArrowRecords::Traces(
                            from_record_messages(msgs).map_err(reject_forwarded)?,
                        )
                    }
                    _ => {
                        warn!(signal = pl_batch.signal, "forwarded batch rejected, invalid signal type");
                        yield ForwardAck{status: ForwardStatus::Error.into(), message: "invalid signal type".into()};
                        continue;
                    },
                };
                pl.decode_transport_optimized_ids()
                    .map_err(reject_forwarded)?;
                let pl = payload::Payload::new(pl);
                let pl_items = pl.num_items() as u64;
                metrics.total_records.fetch_add(pl_items, Ordering::Relaxed);
                let annotated = item_listener_annotate(&pl, None, &conf_cache, None, true);
                metrics
                    .engine_time_spent
                    .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                let pls = annotated
                    .inspect_err(|_| {
                        metrics
                            .total_records_errored
                            .fetch_add(pl_items, Ordering::Relaxed);
                    })
                    .map_err(reject_forwarded)?
                    .for_process;
                let n = Instant::now();
                for pl in pls {
                    let items = pl.payload.num_items() as u64;
                    match processor_sender.broadcast(pl).await {
                        Ok(()) => metrics.total_records_sent.fetch_add(items, Ordering::Relaxed),
                        Err(_) => metrics.total_records_errored.fetch_add(items, Ordering::Relaxed),
                    };
                };
                metrics
                    .send_wait_time
                    .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                yield ForwardAck{status: ForwardStatus::Ok.into(), message: "success".into()};
            };
            debug!("forward stream closed");
        };
        Ok(Response::new(Box::pin(out)))
    }

    type ProcessorCaptureStream =
        Pin<Box<dyn Stream<Item = Result<CaptureMessage, Status>> + Send>>;

    async fn processor_capture(
        &self,
        request: Request<CaptureRequest>,
    ) -> Result<Response<Self::ProcessorCaptureStream>, Status> {
        let req = request.into_inner();
        let processor_id = Uuid::from_slice(req.processor_id.as_ref())
            .map_err(|e| Status::internal(format!("failed to parse processor_id: {}", e)))?;
        let mut subs = self
            .conf_cache
            .processor
            .subscribe_capture(processor_id, &req)
            .ok_or_else(|| Status::not_found("processor not found"))?;
        let shutdown = self.shutdown.clone();
        let out = async_stream::try_stream! {
            loop {
                let msg = tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => break,
                    msg = subs.recv() => msg,
                };
                match msg {
                    Ok(msg) => yield msg,
                    Err(RecvError::Lagged(skipped)) => {
                        debug!(skipped, "capture subscriber lagged");
                        continue;
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        };
        Ok(Response::new(Box::pin(out)))
    }

    type GetItemStatsStream = Pin<Box<dyn Stream<Item = Result<ItemStatsResponse, Status>> + Send>>;

    async fn get_item_stats(
        &self,
        request: Request<ItemStatsRequest>,
    ) -> Result<Response<Self::GetItemStatsStream>, Status> {
        let req = request.into_inner();
        // Validated before the stream is built, so a bad request is a plain error
        // rather than a stream that dies on its first message.
        let ids = item_stats::parse_ids(&req.item_ids)?;
        let now = common::now().fixed_offset();
        let (items, unknown_item_ids) = item_stats::gather(&self.conf_cache.item, &ids);

        let sweep = item_stats::Sweep {
            node_id: self.node_id,
            now,
            // Per response, not per item, so an item inserted by a mid-life config
            // reload reports a window wider than its own counters cover. The schema
            // can't express per-item origins and nothing persists stats today, so
            // process start is the honest answer available.
            stats_since_unix_ms: common::get_start_datetime().timestamp_millis(),
            include_recent_failures: !req.exclude_recent_failures,
            states: item_stats::StateFilter::new(&req.states),
            items,
            unknown_item_ids,
        };
        Ok(Response::new(Box::pin(sweep.into_stream().map(Ok))))
    }
}

#[cfg(test)]
mod tests {
    use super::{Bearer, InternalMetrics, ServerMetrics};
    use crate::common::internal_metrics::NodeMetrics;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use tonic::Request;
    use tonic::service::Interceptor;
    use uuid::Uuid;

    #[test]
    fn bearer_disabled_allows_all() {
        let mut auth = Bearer::new(None).unwrap();
        assert!(auth.call(Request::new(())).is_ok());
    }

    #[test]
    fn bearer_rejects_control_chars_in_token() {
        assert!(Bearer::new(Some("bad\ntoken")).is_err());
    }

    #[test]
    fn bearer_apply_and_validate_roundtrip() {
        let bearer = Bearer::new(Some("s3cret")).unwrap();
        let mut auth = bearer.clone();

        // missing header
        assert!(auth.call(Request::new(())).is_err());

        // wrong token
        let mut request = Request::new(());
        Bearer::new(Some("wrong")).unwrap().apply(&mut request);
        assert!(auth.call(request).is_err());

        // client-applied token passes
        let mut request = Request::new(());
        bearer.apply(&mut request);
        assert!(auth.call(request).is_ok());
    }

    #[test]
    fn forwarder_metrics_snapshot_maps_fields() {
        let m = InternalMetrics::default();
        m.total_time_spent.store(1_500, Ordering::Relaxed);
        m.total_records.store(7, Ordering::Relaxed);
        m.total_records_dropped.store(2, Ordering::Relaxed);
        let node_id = Uuid::new_v4();
        let node = m
            .nodes
            .pin()
            .get_or_insert_with(node_id, || Arc::new(NodeMetrics::default()))
            .clone();
        node.total_connections.store(2, Ordering::Relaxed);
        node.total_records_dropped.store(3, Ordering::Relaxed);
        node.receive_wait_time.store(4_000, Ordering::Relaxed);

        let snap = m.get();
        assert_eq!(snap.total_time_spent, Duration::from_nanos(1_500));
        assert_eq!(snap.total_records, 7);
        assert_eq!(snap.total_records_dropped, 2);
        assert_eq!(snap.nodes.len(), 1);
        assert_eq!(snap.nodes[0].id, node_id);
        assert_eq!(snap.nodes[0].total_connections, 2);
        assert_eq!(snap.nodes[0].total_records_dropped, 3);
        assert_eq!(snap.nodes[0].receive_wait_time, Duration::from_nanos(4_000));
    }

    #[test]
    fn server_metrics_snapshot_maps_fields() {
        let m = ServerMetrics::default();
        m.engine_time_spent.store(2_000, Ordering::Relaxed);
        m.total_bytes.store(42, Ordering::Relaxed);
        m.total_records_sent.store(5, Ordering::Relaxed);

        let snap = m.get();
        assert_eq!(snap.engine_time_spent, Duration::from_nanos(2_000));
        assert_eq!(snap.total_bytes, 42);
        assert_eq!(snap.total_records_sent, 5);
    }
}
