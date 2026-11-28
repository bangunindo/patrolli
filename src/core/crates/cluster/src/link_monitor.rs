// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Records what this node sees when it dials each peer (`cluster_node_links`).
//!
//! A node whose inter-node port is closed still reaches the database, so its
//! heartbeat says nothing of it: the peers that cannot dial it have to. The
//! database is where they say it, being the one path that needs no peer.
//!
//! A task of its own, never a step of the heartbeat: against a firewall that
//! drops, every probe waits its whole timeout, and a heartbeat held up that
//! long is a node its leader takes for stale.

use crate::NodeState;
use crate::inter::{InterComm, LinkProbe};
use anyhow::{Context, Result};
use patrolli::db::Pool;
use patrolli::db::schema::{ClusterNodeLinks, LinkStatus};
use rustc_hash::FxHashMap;
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query};
use sea_query_postgres::PostgresBinder;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{self, MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use uuid::Uuid;

pub struct LinkMonitor {
    pub node_id: Uuid,
    pub db_pool: Pool,
    pub node_lock: Arc<NodeState>,
    pub inter_comm: Arc<InterComm>,
    /// Between two rounds, `cluster.inter_node.link_check.interval`.
    pub interval: Duration,
    /// For one probe, `cluster.inter_node.link_check.timeout`.
    pub timeout: Duration,
    pub token: CancellationToken,
}

/// The columns of a stored row a probe is compared with. Not the reason: the
/// text of an error may change from one probe to the next, the link has not.
#[derive(Debug, PartialEq, Eq)]
struct Stored {
    status: String,
    dst_address: String,
    peer_version: Option<String>,
}

impl LinkMonitor {
    /// Runs until the token is cancelled. Probes only while this node is main:
    /// a standby has no peers, and without the database there is nowhere to
    /// write.
    pub async fn run(self) {
        let mut ticker = time::interval(self.interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        // what each peer's last probe found, for the log
        let mut seen = FxHashMap::default();
        loop {
            tokio::select! {
                _ = self.token.cancelled() => break,
                _ = ticker.tick() => {}
            }
            if self.node_lock.main_epoch().is_none() {
                seen.clear();
                continue;
            }
            tokio::select! {
                _ = self.token.cancelled() => break,
                _ = self.round(&mut seen) => {}
            }
        }
    }

    async fn round(&self, seen: &mut FxHashMap<Uuid, LinkStatus>) {
        let probes = self.inter_comm.probe_links(self.timeout).await;
        seen.retain(|peer_id, _| probes.iter().any(|probe| probe.peer_id == *peer_id));
        for probe in &probes {
            log_change(seen.insert(probe.peer_id, probe.status), probe);
        }
        if probes.is_empty() {
            return;
        }
        if let Err(e) = self.store(&probes).await {
            // the rows stay as they were, and the next round compares again
            warn!(error = %format!("{e:#}"), "failed to store peer links");
        }
    }

    /// Write the probes that differ from their row. Compared with the rows
    /// themselves rather than with what this process last wrote, so a row
    /// lost with its node (the leader expired one of the two, and it came
    /// back) or left by another process of this node is put right, at the
    /// cost of one small read a round. An unchanged link is never written.
    async fn store(&self, probes: &[LinkProbe]) -> Result<()> {
        let client = self
            .db_pool
            .get()
            .await
            .context("failed to get a database connection")?;
        let (sql, values) = Query::select()
            .from(ClusterNodeLinks::Table)
            .columns([
                ClusterNodeLinks::DstNodeId,
                ClusterNodeLinks::Status,
                ClusterNodeLinks::DstAddress,
                ClusterNodeLinks::PeerVersion,
            ])
            .and_where(Expr::col(ClusterNodeLinks::SrcNodeId).eq(self.node_id))
            .build_postgres(PostgresQueryBuilder);
        let rows = client
            .query(&sql, &values.as_params())
            .await
            .context("failed to read peer links")?;
        let mut stored = FxHashMap::default();
        for row in rows {
            let dst: Uuid = row.try_get(0)?;
            let row = Stored {
                status: row.try_get(1)?,
                dst_address: row.try_get(2)?,
                peer_version: row.try_get(3)?,
            };
            stored.insert(dst, row);
        }
        let changed = changed(&stored, probes);
        if changed.is_empty() {
            return Ok(());
        }

        let dsts: Vec<Uuid> = changed.iter().map(|p| p.peer_id).collect();
        let addresses: Vec<&str> = changed.iter().map(|p| p.dst.as_str()).collect();
        let statuses: Vec<&str> = changed.iter().map(|p| p.status.as_str()).collect();
        let reasons: Vec<Option<&str>> = changed.iter().map(|p| p.reason.as_deref()).collect();
        let versions: Vec<Option<&str>> =
            changed.iter().map(|p| p.peer_version.as_deref()).collect();
        // The join leaves out a peer the leader has just deleted, whose row
        // the foreign key would refuse. `state_since` moves only with the
        // status: every SET expression reads the row as it was.
        let written = client
            .execute(
                "INSERT INTO cluster_node_links \
                     (src_node_id, dst_node_id, dst_address, status, reason, peer_version, state_since) \
                 SELECT $1, v.dst, v.address, v.status, v.reason, v.version, now() \
                 FROM unnest($2::uuid[], $3::text[], $4::text[], $5::text[], $6::text[]) \
                      AS v(dst, address, status, reason, version) \
                 JOIN cluster_nodes d ON d.node_id = v.dst \
                 ON CONFLICT (src_node_id, dst_node_id) DO UPDATE SET \
                     dst_address = EXCLUDED.dst_address, \
                     reason = EXCLUDED.reason, \
                     peer_version = EXCLUDED.peer_version, \
                     state_since = CASE \
                         WHEN cluster_node_links.status IS DISTINCT FROM EXCLUDED.status \
                         THEN EXCLUDED.state_since \
                         ELSE cluster_node_links.state_since END, \
                     status = EXCLUDED.status",
                &[
                    &self.node_id,
                    &dsts,
                    &addresses,
                    &statuses,
                    &reasons,
                    &versions,
                ],
            )
            .await
            .context("failed to write peer links")?;
        debug!(changed = changed.len(), written, "peer links stored");
        Ok(())
    }
}

/// The probes whose row is missing or says something else.
fn changed<'a>(stored: &FxHashMap<Uuid, Stored>, probes: &'a [LinkProbe]) -> Vec<&'a LinkProbe> {
    probes
        .iter()
        .filter(|probe| {
            stored.get(&probe.peer_id).is_none_or(|row| {
                row.status != probe.status.as_str()
                    || row.dst_address != probe.dst
                    || row.peer_version != probe.peer_version
            })
        })
        .collect()
}

/// What a probe says of its link next to the probe before it.
#[derive(Debug, PartialEq, Eq)]
enum Change {
    /// It does not work, or fails in another way than before.
    Failed,
    /// It works again.
    Restored,
    /// It works: the first probe of the peer, or another status that does.
    Checked,
}

/// `prev` is what the probe before found, `None` for the first of this peer
/// since the node became main. `None`: nothing changed.
fn change(prev: Option<LinkStatus>, status: LinkStatus) -> Option<Change> {
    if prev == Some(status) {
        return None;
    }
    Some(if !status.is_reachable() {
        Change::Failed
    } else if prev.is_some_and(|prev| !prev.is_reachable()) {
        Change::Restored
    } else {
        Change::Checked
    })
}

/// One event per change of a link's status.
fn log_change(prev: Option<LinkStatus>, probe: &LinkProbe) {
    let (peer_node_id, dst, status) = (probe.peer_id, probe.dst.as_str(), probe.status.as_str());
    match change(prev, probe.status) {
        None => {}
        Some(Change::Failed) => {
            let error = probe.reason.as_deref().unwrap_or_default();
            warn!(%peer_node_id, dst, status, error, "peer link failed");
        }
        Some(Change::Restored) => info!(%peer_node_id, dst, status, "peer link restored"),
        Some(Change::Checked) => debug!(%peer_node_id, dst, status, "peer link checked"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(peer_id: Uuid, status: LinkStatus) -> LinkProbe {
        LinkProbe {
            peer_id,
            dst: "10.0.0.2:4636".to_string(),
            status,
            reason: (!status.is_reachable()).then(|| "connection refused".to_string()),
            peer_version: status.is_reachable().then(|| "1.0.0".to_string()),
        }
    }

    fn stored(probe: &LinkProbe) -> Stored {
        Stored {
            status: probe.status.as_str().to_string(),
            dst_address: probe.dst.clone(),
            peer_version: probe.peer_version.clone(),
        }
    }

    #[test]
    fn only_a_link_that_differs_from_its_row_is_written() {
        let (a, b, c) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let probes = [
            probe(a, LinkStatus::Ok),
            probe(b, LinkStatus::Refused),
            probe(c, LinkStatus::Ok),
        ];
        let peers = |changed: Vec<&LinkProbe>| -> Vec<Uuid> {
            changed.into_iter().map(|probe| probe.peer_id).collect()
        };

        // no rows yet: all of them
        let mut rows = FxHashMap::default();
        assert_eq!(peers(changed(&rows, &probes)), [a, b, c]);

        // rows that say the same: none, whatever the reason reads now
        for probe in &probes {
            rows.insert(probe.peer_id, stored(probe));
        }
        let mut reworded = probes.clone();
        reworded[1].reason = Some("connection refused (os error 61)".to_string());
        assert!(changed(&rows, &reworded).is_empty());

        // one row each for a status, an address and a version that moved on
        let mut moved = probes.clone();
        moved[0].status = LinkStatus::Timeout;
        assert_eq!(peers(changed(&rows, &moved)), [a]);
        let mut moved = probes.clone();
        moved[1].dst = "10.0.0.9:4636".to_string();
        assert_eq!(peers(changed(&rows, &moved)), [b]);
        let mut moved = probes.clone();
        moved[2].peer_version = Some("1.0.1".to_string());
        assert_eq!(peers(changed(&rows, &moved)), [c]);

        // a row that went with its node is written again
        rows.remove(&b);
        assert_eq!(peers(changed(&rows, &probes)), [b]);
    }

    #[test]
    fn a_link_is_logged_when_what_it_says_changes() {
        use LinkStatus::*;
        for (prev, status, expected) in [
            // the first probe of a peer
            (None, Ok, Some(Change::Checked)),
            (None, Refused, Some(Change::Failed)),
            // nothing new
            (Some(Ok), Ok, None),
            (Some(Timeout), Timeout, None),
            (Some(Ok), Timeout, Some(Change::Failed)),
            // another failure than the one before is said again
            (Some(Timeout), Refused, Some(Change::Failed)),
            (Some(Refused), Ok, Some(Change::Restored)),
            (Some(Refused), VersionSkew, Some(Change::Restored)),
            // a peer mid upgrade was reachable all along
            (Some(Ok), VersionSkew, Some(Change::Checked)),
            (Some(VersionSkew), Ok, Some(Change::Checked)),
        ] {
            assert_eq!(change(prev, status), expected, "{prev:?} -> {status:?}");
        }
    }
}
