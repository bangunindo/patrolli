// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The link check: can this node reach each peer's inter-node gRPC port now.
//!
//! Every probe makes a connection of its own rather than calling over the
//! peer's shared channel. A connection made before a firewall rule changed can
//! outlive it, so the shared one would go on answering for a port no new
//! connection gets through; and a failed connect of its own tells the probe why,
//! where the shared channel only says it is not ready.

use super::comm::{Bearer, InterComm};
use patrolli::db::schema::LinkStatus;
use patrolli_proto::v1::NodeIdentity;
use patrolli_proto::v1::inter_cluster_service_client::InterClusterServiceClient;
use std::error::Error;
use std::io;
use std::time::Duration;
use tokio::task::JoinSet;
use tonic::transport::Endpoint;
use tonic::{Code, Request, Status};
use tracing::debug;
use uuid::Uuid;

/// What one probe of one peer found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkProbe {
    pub peer_id: Uuid,
    /// The advertise address that was dialed.
    pub dst: String,
    pub status: LinkStatus,
    /// The failure as text, `None` when the peer answered as expected.
    pub reason: Option<String>,
    /// `None` when the peer did not answer.
    pub peer_version: Option<String>,
}

impl InterComm {
    /// Probe every peer once, all at the same time, each within `timeout`
    /// (connect and answer together). One element per peer, in no order; a
    /// node without peers (a standby, a cluster of one) gets none.
    ///
    /// Also sets each peer's `reachable` in the forwarder metrics.
    pub async fn probe_links(&self, timeout: Duration) -> Vec<LinkProbe> {
        let identity = NodeIdentity {
            node_id: self.node_id.as_bytes().to_vec().into(),
            node_name: self.node_name.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        };
        let mut joins = JoinSet::new();
        {
            // Scoped to the spawn loop, as in `item_stats`.
            let clients = self.clients.read().await;
            for client in clients.values() {
                let (peer_id, dst, endpoint, bearer, metrics) = (
                    client.node_id,
                    client.dst.clone(),
                    client.endpoint.clone(),
                    client.bearer.clone(),
                    client.metrics.clone(),
                );
                let identity = identity.clone();
                joins.spawn(async move {
                    let (status, reason, peer_version) =
                        probe(endpoint, bearer, identity, peer_id, timeout).await;
                    metrics.set_reachable(status.is_reachable());
                    LinkProbe {
                        peer_id,
                        dst,
                        status,
                        reason,
                        peer_version,
                    }
                });
            }
        }
        let mut probes = Vec::with_capacity(joins.len());
        while let Some(joined) = joins.join_next().await {
            match joined {
                Ok(probe) => probes.push(probe),
                // a panic: the hook reported it and is shutting the node down
                Err(e) => debug!(error = %e, "link probe task failed"),
            }
        }
        probes
    }
}

type Outcome = (LinkStatus, Option<String>, Option<String>);

async fn probe(
    endpoint: Endpoint,
    bearer: Bearer,
    identity: NodeIdentity,
    expect_node_id: Uuid,
    timeout: Duration,
) -> Outcome {
    let version = identity.version.clone();
    let call = async {
        let channel = endpoint
            .connect_timeout(timeout)
            .connect()
            .await
            .map_err(|e| (classify_transport(&e), chain(&e)))?;
        let mut request = Request::new(identity);
        bearer.apply(&mut request);
        InterClusterServiceClient::new(channel)
            .hello(request)
            .await
            .map(|response| response.into_inner())
            .map_err(|status| {
                let reason = format!("{:?}: {}", status.code(), status.message());
                (classify_status(&status), reason)
            })
    };
    let peer = match tokio::time::timeout(timeout, call).await {
        Err(_) => {
            let reason = format!("no answer in {}s", timeout.as_secs());
            return (LinkStatus::Timeout, Some(reason), None);
        }
        Ok(Err((status, reason))) => return (status, Some(reason), None),
        Ok(Ok(peer)) => peer,
    };
    let peer_version = Some(peer.version.clone());
    match Uuid::from_slice(peer.node_id.as_ref()) {
        Err(e) => {
            let reason = format!("peer answered with an invalid node id: {e}");
            (LinkStatus::Error, Some(reason), peer_version)
        }
        Ok(peer_id) if peer_id != expect_node_id => {
            let reason = format!("answered by node {peer_id} ({})", peer.node_name);
            (LinkStatus::WrongIdentity, Some(reason), peer_version)
        }
        Ok(_) if peer.version != version => (LinkStatus::VersionSkew, None, peer_version),
        Ok(_) => (LinkStatus::Ok, None, peer_version),
    }
}

/// A call the peer or the connection under it failed.
fn classify_status(status: &Status) -> LinkStatus {
    match status.code() {
        Code::Unauthenticated => LinkStatus::Unauthenticated,
        // how a peer older than the link check answers another version
        Code::FailedPrecondition => LinkStatus::VersionSkew,
        Code::DeadlineExceeded => LinkStatus::Timeout,
        _ => status
            .source()
            .and_then(classify_io)
            .unwrap_or(LinkStatus::Error),
    }
}

/// A connection that could not be made.
fn classify_transport(err: &(dyn Error + 'static)) -> LinkStatus {
    classify_io(err).unwrap_or(LinkStatus::Unreachable)
}

/// The first I/O error in the chain says what the network did.
fn classify_io(err: &(dyn Error + 'static)) -> Option<LinkStatus> {
    let mut cause = Some(err);
    while let Some(err) = cause {
        if let Some(io_err) = err.downcast_ref::<io::Error>() {
            return Some(match io_err.kind() {
                io::ErrorKind::ConnectionRefused => LinkStatus::Refused,
                io::ErrorKind::TimedOut => LinkStatus::Timeout,
                // how tokio-rustls hands over a failed handshake (a bad
                // certificate, a peer that does not speak TLS)
                io::ErrorKind::InvalidData => LinkStatus::Tls,
                _ => LinkStatus::Unreachable,
            });
        }
        cause = err.source();
    }
    None
}

/// The error with its causes: a transport error's own text is "transport error".
fn chain(err: &(dyn Error + 'static)) -> String {
    let mut text = err.to_string();
    let mut cause = err.source();
    while let Some(err) = cause {
        // some errors already print their cause, or wrap one that reads the same
        let cause_text = err.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        cause = err.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inter::InterCommManager;
    use crate::inter::test_util::{closed_addr, local_node, serve_peer};

    const TIMEOUT: Duration = Duration::from_secs(5);

    /// The one probe of a node with one peer.
    async fn probe_one(comm: &InterComm, timeout: Duration) -> LinkProbe {
        let mut probes = comm.probe_links(timeout).await;
        assert_eq!(probes.len(), 1);
        probes.remove(0)
    }

    #[tokio::test]
    async fn a_node_without_peers_probes_nothing() {
        let (comm, _, _) = local_node(None);
        assert!(comm.probe_links(TIMEOUT).await.is_empty());
    }

    #[tokio::test]
    async fn a_peer_that_answers_is_ok() {
        let peer = serve_peer(Some("secret")).await;
        let (comm, _, _) = local_node(Some("secret"));
        comm.add(peer.node_id, peer.addr.clone()).await.unwrap();

        assert_eq!(
            probe_one(&comm, TIMEOUT).await,
            LinkProbe {
                peer_id: peer.node_id,
                dst: peer.addr.clone(),
                status: LinkStatus::Ok,
                reason: None,
                peer_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }
        );
    }

    #[tokio::test]
    async fn a_closed_port_is_refused() {
        let (comm, _, _) = local_node(None);
        let peer_id = Uuid::now_v7();
        comm.add(peer_id, closed_addr().await).await.unwrap();

        let probe = probe_one(&comm, TIMEOUT).await;
        assert_eq!(probe.status, LinkStatus::Refused, "{:?}", probe.reason);
        assert_eq!(probe.peer_id, peer_id);
        assert_eq!(probe.peer_version, None);
        // the cause, not just tonic's "transport error"
        let reason = probe.reason.unwrap();
        assert!(reason.to_lowercase().contains("refused"), "{reason}");
        assert_eq!(reason.matches("tcp connect error").count(), 1, "{reason}");
    }

    #[tokio::test]
    async fn a_peer_that_never_answers_times_out() {
        // accepts the connection and says nothing, as far as a probe can tell
        // from a firewall that drops
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });
        let (comm, _, _) = local_node(None);
        comm.add(Uuid::now_v7(), addr).await.unwrap();

        let probe = probe_one(&comm, Duration::from_millis(300)).await;
        assert_eq!(probe.status, LinkStatus::Timeout, "{:?}", probe.reason);
        assert_eq!(probe.peer_version, None);
    }

    #[tokio::test]
    async fn a_rejected_bearer_is_unauthenticated() {
        let peer = serve_peer(Some("secret")).await;
        let (comm, _, _) = local_node(Some("other"));
        comm.add(peer.node_id, peer.addr.clone()).await.unwrap();

        let probe = probe_one(&comm, TIMEOUT).await;
        assert_eq!(probe.status, LinkStatus::Unauthenticated);
        assert_eq!(
            probe.reason.as_deref(),
            Some("Unauthenticated: invalid bearer token")
        );
    }

    #[tokio::test]
    async fn another_node_at_the_address_is_a_wrong_identity() {
        let peer = serve_peer(None).await;
        let (comm, _, _) = local_node(None);
        // the address of `peer`, registered as some other node's
        comm.add(Uuid::now_v7(), peer.addr.clone()).await.unwrap();

        let probe = probe_one(&comm, TIMEOUT).await;
        assert_eq!(probe.status, LinkStatus::WrongIdentity);
        assert_eq!(
            probe.reason,
            Some(format!("answered by node {} (peer)", peer.node_id))
        );
        assert_eq!(
            probe.peer_version.as_deref(),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }

    #[tokio::test]
    async fn every_peer_is_probed_on_its_own() {
        let peer = serve_peer(None).await;
        let (comm, _, _) = local_node(None);
        let down = Uuid::now_v7();
        comm.add(peer.node_id, peer.addr.clone()).await.unwrap();
        comm.add(down, closed_addr().await).await.unwrap();

        let mut probes = comm.probe_links(TIMEOUT).await;
        probes.sort_by_key(|probe| probe.peer_id != peer.node_id);
        let statuses: Vec<_> = probes.iter().map(|p| (p.peer_id, p.status)).collect();
        assert_eq!(
            statuses,
            [(peer.node_id, LinkStatus::Ok), (down, LinkStatus::Refused)]
        );
    }

    #[tokio::test]
    async fn a_probe_sets_the_peers_reachable_metric_until_it_is_closed() {
        let peer = serve_peer(None).await;
        let (comm, _, _) = local_node(None);
        let down = Uuid::now_v7();
        comm.add(peer.node_id, peer.addr.clone()).await.unwrap();
        comm.add(down, closed_addr().await).await.unwrap();
        // the peers' own counters, which outlive their clients
        let metrics = |id: Uuid| {
            let comm = comm.clone();
            async move { comm.clients.read().await[&id].metrics.clone() }
        };
        let (up_metrics, down_metrics) = (metrics(peer.node_id).await, metrics(down).await);
        assert_eq!(up_metrics.get(peer.node_id).reachable, None);

        comm.probe_links(TIMEOUT).await;
        assert_eq!(up_metrics.get(peer.node_id).reachable, Some(true));
        assert_eq!(down_metrics.get(down).reachable, Some(false));

        // a standby closes its peers: what it last saw is no longer an answer
        comm.close_all().await;
        assert_eq!(up_metrics.get(peer.node_id).reachable, None);
        assert_eq!(down_metrics.get(down).reachable, None);
    }

    #[test]
    fn a_status_is_classified_by_its_code() {
        for (status, expected) in [
            (Status::unauthenticated("x"), LinkStatus::Unauthenticated),
            // a peer older than the link check, on another version
            (
                Status::failed_precondition("version mismatch"),
                LinkStatus::VersionSkew,
            ),
            (Status::deadline_exceeded("x"), LinkStatus::Timeout),
            (Status::internal("x"), LinkStatus::Error),
            (Status::unimplemented("x"), LinkStatus::Error),
        ] {
            assert_eq!(classify_status(&status), expected, "{status}");
        }
    }

    #[test]
    fn an_io_error_is_classified_by_its_kind_through_the_chain() {
        #[derive(Debug, thiserror::Error)]
        #[error("outer")]
        struct Outer(#[source] io::Error);

        for (kind, expected) in [
            (io::ErrorKind::ConnectionRefused, LinkStatus::Refused),
            (io::ErrorKind::TimedOut, LinkStatus::Timeout),
            (io::ErrorKind::InvalidData, LinkStatus::Tls),
            (io::ErrorKind::HostUnreachable, LinkStatus::Unreachable),
            (io::ErrorKind::ConnectionReset, LinkStatus::Unreachable),
        ] {
            let err = Outer(io::Error::new(kind, "inner"));
            assert_eq!(classify_transport(&err), expected);
            assert_eq!(chain(&err), "outer: inner");
        }
        // a connection that failed for a reason no I/O error names
        let err = io::Error::other("x");
        assert_eq!(classify_transport(&err), LinkStatus::Unreachable);
        #[derive(Debug, thiserror::Error)]
        #[error("bare")]
        struct Bare;
        assert_eq!(classify_transport(&Bare), LinkStatus::Unreachable);
        assert_eq!(classify_io(&Bare), None);
    }
}
