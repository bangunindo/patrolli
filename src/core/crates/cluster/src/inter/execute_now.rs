// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Running a monitor item at once, on whichever node monitors it.
//!
//! An item sits only in the cache of the node that owns its host, so the node
//! that takes the request usually cannot serve it. Every node is asked, the
//! same way item stats are, and all but the owner answer "not found".

use super::comm::Bearer;
use parking_lot::RwLock;
use patrolli::cache;
use patrolli_proto::v1::inter_cluster_service_client::InterClusterServiceClient;
use patrolli_proto::v1::{ExecuteNowOutcome, ExecuteNowRequest};
use std::time::Duration;
use tonic::Request;
use tonic::transport::Channel;
use tracing::debug;
use uuid::Uuid;

/// Why no node ran the item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ExecuteNowError {
    /// Every node answered, and none monitors the item.
    #[error("item not found")]
    NotFound,
    /// A listener item: it has no check to run.
    #[error("item not schedulable")]
    NotSchedulable,
    /// A check of the item is in flight on its node.
    #[error("item is already running")]
    AlreadyRunning,
    /// No node claimed the item, and `failed_nodes` of them did not answer, so
    /// one of those may be its owner.
    #[error("item not found, and {failed_nodes} node(s) did not answer")]
    Unreachable { failed_nodes: usize },
}

/// Serve the request from this node's own cache.
///
/// What both the gRPC handler and the caller's local step run, so a node
/// answers itself exactly as it answers a peer.
pub(super) fn local(cache: &RwLock<cache::item::Item>, item_id: &Uuid) -> ExecuteNowOutcome {
    use cache::item::ExecuteNowError as E;
    // Bound to a statement so the write lock is gone before anything else runs.
    let result = cache.write().execute_now(item_id);
    match result {
        Ok(()) => {
            debug!(item_id = %item_id, "item scheduled to run now");
            ExecuteNowOutcome::Scheduled
        }
        Err(E::NotFound) => ExecuteNowOutcome::NotFound,
        Err(E::NotSchedulable) => ExecuteNowOutcome::NotSchedulable,
        Err(E::AlreadyRunning) => ExecuteNowOutcome::AlreadyRunning,
    }
}

/// Turn one node's answer into the call's result, or `None` when the node does
/// not hold the item and the next one has to be heard.
pub(super) fn settle(
    outcome: ExecuteNowOutcome,
    node_id: Uuid,
) -> Option<Result<Uuid, ExecuteNowError>> {
    match outcome {
        ExecuteNowOutcome::Scheduled => Some(Ok(node_id)),
        ExecuteNowOutcome::NotSchedulable => Some(Err(ExecuteNowError::NotSchedulable)),
        ExecuteNowOutcome::AlreadyRunning => Some(Err(ExecuteNowError::AlreadyRunning)),
        // `peer` turns an unspecified outcome into a failure before it gets here.
        ExecuteNowOutcome::NotFound | ExecuteNowOutcome::Unspecified => None,
    }
}

/// Ask one peer to run the item.
///
/// Takes the channel and bearer by value for the same reason `peer_item_stats`
/// does: the fan-out runs these on a `JoinSet`. An `Err` is a peer that gave no
/// usable answer (down, timed out, rejected the call, or too old to know the
/// RPC), which is not the same as one that answered "not found".
pub(super) async fn peer(
    transport: Channel,
    bearer: Bearer,
    req: ExecuteNowRequest,
    timeout: Duration,
) -> Result<ExecuteNowOutcome, String> {
    let mut client = InterClusterServiceClient::new(transport);
    let mut request = Request::new(req);
    bearer.apply(&mut request);

    let response = match tokio::time::timeout(timeout, client.execute_now(request)).await {
        Ok(Ok(response)) => response.into_inner(),
        Ok(Err(status)) => return Err(status.to_string()),
        Err(_) => return Err(format!("timed out after {}s", timeout.as_secs())),
    };
    match ExecuteNowOutcome::try_from(response.outcome) {
        Ok(ExecuteNowOutcome::Unspecified) | Err(_) => {
            Err(format!("unknown outcome {}", response.outcome))
        }
        Ok(outcome) => Ok(outcome),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inter::{InterComm, InterCommManager, InterCommServer};
    use patrolli::common::internal_metrics::INTERNAL_METRICS;
    use patrolli::config;
    use patrolli::config::item::{
        Config, Dummy, DummyPayload, Item, Listener, ListenerItemMatchStrategy, Schedule,
    };
    use std::sync::Arc;
    use tokio_util::sync::{CancellationToken, DropGuard};
    use tonic::transport::server::TcpIncoming;

    const TIMEOUT: Duration = Duration::from_secs(5);

    fn monitor_item() -> Item {
        Item::new(
            Uuid::now_v7(),
            "test".into(),
            None,
            Uuid::now_v7(),
            Schedule::Never,
            None,
            Duration::from_secs(1),
            vec![],
            Config::Dummy(Dummy {
                payload: DummyPayload::default(),
                count: 1,
            }),
        )
    }

    fn listener_item() -> Item {
        let mut item = monitor_item();
        item.config = Config::Listener(Listener {
            match_strategy: ListenerItemMatchStrategy::None,
            expect_every: None,
        });
        item
    }

    /// A peer node: its own cache behind the real gRPC service.
    struct Peer {
        node_id: Uuid,
        cache: cache::Cache,
        addr: String,
        _stop: DropGuard,
    }

    async fn serve_peer(bearer: Option<&str>) -> Peer {
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
    fn local_node(bearer: Option<&str>) -> (Arc<InterComm>, Uuid, cache::Cache) {
        // the sender is dropped, so the forwarder bridge ends right away
        let (_, forward_receiver) = flume::unbounded();
        let mut conf = config::cluster::InterNode::default();
        conf.grpc.bearer = bearer.map(str::to_owned);
        let node_id = Uuid::now_v7();
        let cache = cache::Cache::new();
        let comm = InterComm::new(
            forward_receiver,
            conf,
            node_id,
            "local".to_string(),
            cache.clone(),
        )
        .unwrap();
        (comm, node_id, cache)
    }

    /// An address nothing listens on.
    async fn closed_addr() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap().to_string()
    }

    #[tokio::test]
    async fn the_local_node_runs_its_own_item() {
        let (comm, node_id, cache) = local_node(None);
        let item = monitor_item();
        let id = item.id;
        cache.item.write().insert(item);

        assert_eq!(comm.execute_now(id, TIMEOUT).await, Ok(node_id));
        assert_eq!(cache.item.write().pop_due().len(), 1);
    }

    #[tokio::test]
    async fn the_owning_peer_answers_over_grpc() {
        let peer = serve_peer(Some("secret")).await;
        let (comm, _, _) = local_node(Some("secret"));
        comm.add(peer.node_id, peer.addr.clone()).await.unwrap();

        let item = monitor_item();
        let id = item.id;
        let listener = listener_item();
        let listener_id = listener.id;
        peer.cache.item.write().insert(item);
        peer.cache.item.write().insert(listener);

        assert_eq!(comm.execute_now(id, TIMEOUT).await, Ok(peer.node_id));
        let due = peer.cache.item.write().pop_due();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].item.id, id);

        // Popped and not requeued: a worker on the peer has it.
        assert_eq!(
            comm.execute_now(id, TIMEOUT).await,
            Err(ExecuteNowError::AlreadyRunning)
        );
        assert_eq!(
            comm.execute_now(listener_id, TIMEOUT).await,
            Err(ExecuteNowError::NotSchedulable)
        );
        assert_eq!(
            comm.execute_now(Uuid::now_v7(), TIMEOUT).await,
            Err(ExecuteNowError::NotFound)
        );
    }

    /// A node that did not answer may be the owner, so its silence must not
    /// read as "no such item". A node that does hold the item still decides.
    #[tokio::test]
    async fn a_silent_peer_is_not_a_not_found() {
        let peer = serve_peer(None).await;
        let (comm, _, _) = local_node(None);
        comm.add(peer.node_id, peer.addr.clone()).await.unwrap();
        comm.add(Uuid::now_v7(), closed_addr().await).await.unwrap();

        let item = monitor_item();
        let id = item.id;
        assert_eq!(
            comm.execute_now(id, TIMEOUT).await,
            Err(ExecuteNowError::Unreachable { failed_nodes: 1 })
        );

        peer.cache.item.write().insert(item);
        assert_eq!(comm.execute_now(id, TIMEOUT).await, Ok(peer.node_id));
    }

    #[tokio::test]
    async fn a_peer_that_rejects_the_bearer_counts_as_silent() {
        let peer = serve_peer(Some("secret")).await;
        let (comm, _, _) = local_node(Some("other"));
        comm.add(peer.node_id, peer.addr.clone()).await.unwrap();

        let item = monitor_item();
        let id = item.id;
        peer.cache.item.write().insert(item);
        assert_eq!(
            comm.execute_now(id, TIMEOUT).await,
            Err(ExecuteNowError::Unreachable { failed_nodes: 1 })
        );
        assert!(peer.cache.item.write().pop_due().is_empty());
    }

    #[test]
    fn local_maps_every_cache_answer() {
        let cache = RwLock::new(cache::item::Item::new());
        let item = monitor_item();
        let id = item.id;
        cache.write().insert(item);
        let listener_item = listener_item();
        let listener_id = listener_item.id;
        cache.write().insert(listener_item);

        assert_eq!(local(&cache, &Uuid::now_v7()), ExecuteNowOutcome::NotFound);
        assert_eq!(
            local(&cache, &listener_id),
            ExecuteNowOutcome::NotSchedulable
        );

        // `Never` parks the item a year out, so only the request makes it due.
        assert!(cache.write().pop_due().is_empty());
        assert_eq!(local(&cache, &id), ExecuteNowOutcome::Scheduled);
        let due = cache.write().pop_due();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].item.id, id);

        // Popped and not requeued: a worker has it.
        assert_eq!(local(&cache, &id), ExecuteNowOutcome::AlreadyRunning);
    }

    #[test]
    fn settle_ends_the_call_on_every_answer_but_not_found() {
        let node = Uuid::now_v7();
        assert_eq!(settle(ExecuteNowOutcome::Scheduled, node), Some(Ok(node)));
        assert_eq!(
            settle(ExecuteNowOutcome::NotSchedulable, node),
            Some(Err(ExecuteNowError::NotSchedulable))
        );
        assert_eq!(
            settle(ExecuteNowOutcome::AlreadyRunning, node),
            Some(Err(ExecuteNowError::AlreadyRunning))
        );
        assert_eq!(settle(ExecuteNowOutcome::NotFound, node), None);
    }
}
