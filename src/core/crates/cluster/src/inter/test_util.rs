// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Nodes for the inter-node tests: a peer behind the real gRPC service, and
//! the node a call is made on.

use crate::inter::{InterComm, InterCommServer};
use patrolli::common::internal_metrics::INTERNAL_METRICS;
use patrolli::{cache, config};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::{CancellationToken, DropGuard};
use tonic::transport::server::TcpIncoming;
use uuid::Uuid;

/// A peer node: its own cache behind the real gRPC service.
pub(super) struct Peer {
    pub(super) node_id: Uuid,
    pub(super) cache: cache::Cache,
    pub(super) addr: String,
    _stop: DropGuard,
}

pub(super) async fn serve_peer(bearer: Option<&str>) -> Peer {
    let node_id = Uuid::now_v7();
    let cache = cache::Cache::new();
    let token = CancellationToken::new();
    // forwarding is not under test; the service just needs a sender
    let (_admin, processor_sender) = patrolli::chan::new(
        Duration::from_secs(60),
        INTERNAL_METRICS.processor_channel.clone(),
    );
    let grpc = config::cluster::Grpc {
        address: "127.0.0.1:0".parse().unwrap(),
        tls: None,
        bearer: bearer.map(str::to_owned),
    };
    let service = InterCommServer::new_service(
        node_id,
        "peer".to_string(),
        cache.clone(),
        processor_sender,
        &grpc,
        Duration::from_secs(60),
        token.clone(),
    )
    .unwrap();
    // Bound before the server task starts, so a call can go out right away.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let stop = token.clone();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming_shutdown(TcpIncoming::from(listener), stop.cancelled_owned())
            .await
            .unwrap();
    });
    Peer {
        node_id,
        cache,
        addr,
        _stop: token.drop_guard(),
    }
}

/// The node the call is made on, with no peers yet.
pub(super) fn local_node(bearer: Option<&str>) -> (Arc<InterComm>, Uuid, cache::Cache) {
    // the sender is dropped, so the forwarder bridge ends right away
    let (_, forward_receiver) = flume::unbounded();
    let mut conf = config::cluster::InterNode::default();
    conf.grpc.bearer = bearer.map(str::to_owned);
    let node_id = Uuid::now_v7();
    let cache = cache::Cache::new();
    let (_admin, processor_sender) = patrolli::chan::new(
        Duration::from_secs(60),
        INTERNAL_METRICS.processor_channel.clone(),
    );
    let comm = InterComm::new(
        forward_receiver,
        conf,
        node_id,
        "local".to_string(),
        cache.clone(),
        processor_sender,
    )
    .unwrap();
    (comm, node_id, cache)
}

/// An address nothing listens on.
pub(super) async fn closed_addr() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().to_string()
}
