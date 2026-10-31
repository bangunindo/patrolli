// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::balancer::{self, Move, NodeLoad};
use super::{NodeState, NodeStateType};
use crate::config::cluster::{Cluster as ClusterConfig, ClusterPolicy};
use crate::db::AdvisoryLock;
use crate::db::schema::ConfigKeys;
use anyhow::{Context, Result};
use deadpool_postgres::{ClientWrapper, GenericClient, Object, Pool, Transaction};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tokio_postgres::Error as PostgresError;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

pub struct NodeLock {
    db_pool: Pool,
    token: CancellationToken,
    /// Process-wide shutdown token. Cancelled when another live process already
    /// holds our `(node_id, name)` standby identity, which is unrecoverable.
    app_shutdown: CancellationToken,
    cluster: ClusterConfig,
    node_lock: Arc<NodeState>,
    /// The connection holding our standby advisory lock, kept alive across retry
    /// ticks. `Some` means this process owns the `(node_id, name)` identity;
    /// dropping it ends the session and releases the lock, so the two can't drift.
    standby_client: Option<ClientWrapper>,
    /// Last good `configs.cluster`, re-read by the leader every tick.
    policy: ClusterPolicy,
}

fn advisory_key(kind: AdvisoryLock, identity: &str) -> i64 {
    let mut key = xxhash_rust::xxh3::xxh3_64(identity.as_bytes()).to_be_bytes();
    key[7] = kind as u8;
    i64::from_be_bytes(key)
}

fn is_connection_error(err: &anyhow::Error) -> bool {
    for cause in err.chain() {
        // Check for tokio_postgres errors
        if let Some(pg_err) = cause.downcast_ref::<tokio_postgres::Error>()
            && pg_err.is_closed()
        {
            return true;
        }

        // Check for raw I/O errors (e.g. ConnectionRefused, ConnectionReset)
        // These are exposed as the `source()` of tokio_postgres::Error,
        // and anyhow's chain() walks through them automatically.
        if let Some(io_err) = cause.downcast_ref::<io::Error>() {
            match io_err.kind() {
                io::ErrorKind::ConnectionRefused
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::TimedOut
                | io::ErrorKind::BrokenPipe
                | io::ErrorKind::NotConnected => return true,
                _ => {}
            }
        }

        // Check for deadpool pool errors (timeout getting a connection from pool)
        if let Some(_pool_err) = cause.downcast_ref::<deadpool_postgres::PoolError>() {
            return true;
        }
    }
    false
}

impl NodeLock {
    pub fn new(
        db_pool: Pool,
        token: CancellationToken,
        app_shutdown: CancellationToken,
        cluster: ClusterConfig,
        node_lock: Arc<NodeState>,
    ) -> Self {
        Self {
            db_pool,
            token,
            app_shutdown,
            cluster,
            node_lock,
            standby_client: None,
            policy: ClusterPolicy::default(),
        }
    }

    fn node_key(&self) -> i64 {
        advisory_key(AdvisoryLock::NodeLock, &self.cluster.node_id.to_string())
    }

    /// A standby is identified by the pair, not the id alone: several processes share
    /// a `node_id` and are told apart by name.
    fn standby_key(&self) -> i64 {
        advisory_key(
            AdvisoryLock::StandbyLock,
            &format!("{}/{}", self.cluster.node_id, self.cluster.node_name),
        )
    }

    /// Re-read `configs.cluster`. A value that does not parse or validate keeps the
    /// last good policy: falling back to the default would put every spare to work.
    async fn refresh_policy(&mut self, tx: &Transaction<'_>) -> Result<()> {
        match super::read_config(tx, ConfigKeys::Cluster, ClusterPolicy::validate).await? {
            None => self.policy = ClusterPolicy::default(),
            Some(Ok(policy)) => self.policy = policy,
            Some(Err(e)) => {
                error!(error = %format!("{e:#}"), "invalid cluster policy, keeping the last good one")
            }
        }
        Ok(())
    }

    async fn apply_move(tx: &Transaction<'_>, mv: Move) -> Result<u64> {
        let updated = match mv {
            Move::Block { from, to } => tx
                .execute(
                    "UPDATE hosts SET monitored_by = $1 \
                    WHERE monitored_by = $2 AND NOT sticky_monitored_by",
                    &[&to, &from],
                )
                .await
                .context("failed to fail over hosts")?,
            // markers: their hosts leave through the transfers that follow
            Move::Release { .. } | Move::Evacuate { .. } => 0,
            Move::Transfer {
                from: None,
                to,
                count,
            } => tx
                .execute(
                    "UPDATE hosts \
                    SET monitored_by = $1 \
                    WHERE host_id IN (\
                    SELECT host_id \
                    FROM hosts \
                    WHERE monitored_by IS NULL \
                    LIMIT $2)",
                    &[&to, &count],
                )
                .await
                .context("failed to distribute unmonitored hosts")?,
            Move::Transfer {
                from: Some(from),
                to,
                count,
            } => tx
                .execute(
                    "UPDATE hosts \
                    SET monitored_by = $1 \
                    WHERE host_id IN (\
                    SELECT host_id \
                    FROM hosts \
                    WHERE monitored_by = $2 \
                      AND NOT sticky_monitored_by \
                    LIMIT $3)",
                    &[&to, &from, &count],
                )
                .await
                .context("failed to rebalance hosts")?,
        };
        Ok(updated)
    }

    async fn leader_tasks(&mut self, tx: &Transaction<'_>) -> Result<()> {
        let expire_after = self.cluster.heartbeat_timeout.as_secs_f64();
        // a node is live while its heartbeat is this fresh; past it, its hosts fail
        // over. An expired node is never live, so its hosts are moved off before
        // the expiry below deletes it.
        let stale_after = self
            .cluster
            .heartbeat_interval
            .saturating_add(Duration::from_secs(2))
            .as_secs_f64()
            .min(expire_after);
        tx.execute(
            "UPDATE cluster_nodes SET op_status = 0 WHERE last_heartbeat + make_interval(secs := $1) < now()",
            &[&stale_after],
        )
        .await
        .context("failed updating inactive cluster nodes")?;
        self.refresh_policy(tx).await?;

        let rows = tx
            .query(
                "SELECT cn.node_id, \
                    cn.last_heartbeat + make_interval(secs := $1) >= now(), \
                    COUNT(h.host_id) FILTER (WHERE NOT h.sticky_monitored_by) \
                FROM cluster_nodes cn \
                LEFT JOIN hosts h ON h.monitored_by = cn.node_id \
                GROUP BY cn.node_id, cn.last_heartbeat",
                &[&stale_after],
            )
            .await
            .context("failed to get host distribution")?;
        let nodes = rows
            .iter()
            .map(|row| {
                Ok(NodeLoad {
                    node_id: row.try_get(0)?,
                    live: row.try_get(1)?,
                    hosts: row.try_get(2)?,
                })
            })
            .collect::<Result<Vec<NodeLoad>, PostgresError>>()
            .context("failed to deserialize host distribution")?;
        let unassigned: i64 = tx
            .query_one("SELECT COUNT(*) FROM hosts WHERE monitored_by IS NULL", &[])
            .await
            .context("failed to count unmonitored hosts")?
            .try_get(0)
            .context("failed to deserialize unmonitored host count")?;

        let mut hosts_assigned: u64 = 0;
        let mut hosts_moved: u64 = 0;
        for mv in balancer::plan(&nodes, unassigned, self.policy.max_active_nodes) {
            let updated = Self::apply_move(tx, mv).await?;
            match mv {
                Move::Block { from, to } => info!(
                    from_node_id = %from,
                    to_node_id = %to,
                    hosts = updated,
                    "hosts failed over to spare node"
                ),
                Move::Release { from, hosts } => info!(
                    from_node_id = %from,
                    hosts,
                    "hosts of failed node redistributed"
                ),
                Move::Evacuate { from, hosts } => info!(
                    from_node_id = %from,
                    hosts,
                    "node evacuated"
                ),
                Move::Transfer { from: None, .. } => hosts_assigned += updated,
                Move::Transfer {
                    from: Some(from),
                    to,
                    ..
                } => {
                    if updated > 0 {
                        debug!(
                            from_node_id = %from,
                            to_node_id = %to,
                            hosts = updated,
                            "hosts moved between nodes"
                        );
                    }
                    hosts_moved += updated;
                }
            }
        }
        if hosts_assigned > 0 || hosts_moved > 0 {
            info!(
                assigned = hosts_assigned,
                moved = hosts_moved,
                "hosts rebalanced"
            );
        }

        // sticky hosts wait for their node (or its standby) to come back, so a node
        // still owning one keeps its row; its logs are dropped every tick regardless,
        // since a returning node reloads everything anyway
        tx.execute(
            "DELETE FROM cluster_logs \
            WHERE node_id IN (\
            SELECT node_id FROM cluster_nodes \
            WHERE last_heartbeat + make_interval(secs := $1) < now())",
            &[&expire_after],
        )
        .await
        .context("failed to delete cluster logs")?;
        let removed = tx
            .execute(
                "DELETE FROM cluster_nodes \
                WHERE last_heartbeat + make_interval(secs := $1) < now() \
                  AND NOT EXISTS (SELECT 1 FROM hosts h WHERE h.monitored_by = cluster_nodes.node_id)",
                &[&expire_after],
            )
            .await
            .context("failed to delete inactive cluster nodes")?;
        if removed > 0 {
            info!(nodes = removed, "inactive cluster nodes removed");
        }
        tx.execute(
            "DELETE FROM cluster_standby_nodes WHERE last_heartbeat + make_interval(secs := $1) < now()",
            &[&expire_after],
        )
        .await
        .context("failed to delete inactive standby nodes")?;

        Ok(())
    }

    async fn acquire_standby_lock(&self, client: &tokio_postgres::Client) -> Result<bool> {
        let acquired: bool = client
            .query_one_scalar("SELECT pg_try_advisory_lock($1)", &[&self.standby_key()])
            .await
            .context("failed to acquire standby lock")?;
        if !acquired {
            error!("another standby is already running with this node_id and name, shutting down");
            self.app_shutdown.cancel();
        }
        Ok(acquired)
    }

    /// Register (or refresh) our entry in `cluster_standby_nodes` so the API can
    /// report this process as a live standby.
    async fn standby_heartbeat(
        &self,
        client: &tokio_postgres::Client,
        metadata: &serde_json::Value,
    ) -> Result<()> {
        client
            .execute(
                "INSERT INTO cluster_standby_nodes(node_id, name, last_heartbeat, metadata) \
                VALUES ($1, $2, now(), $3) \
                ON CONFLICT (node_id, name) DO UPDATE SET \
                last_heartbeat = now(), \
                metadata = EXCLUDED.metadata",
                &[&self.cluster.node_id, &self.cluster.node_name, metadata],
            )
            .await
            .context("failed to heartbeat standby node")?;
        Ok(())
    }

    /// Stop being a standby now that we hold the node lock.
    ///
    /// The delete is unconditional so it also clears a stale row left behind by a
    /// previous crashed run of this `(node_id, name)`; it is a no-op for a process
    /// that started as main. `holds_lock` says whether this connection is the one that
    /// claimed the standby lock — it carries on as the main connection, so the lock has
    /// to be handed back explicitly instead of by ending the session.
    async fn release_standby(
        &self,
        client: &tokio_postgres::Client,
        holds_lock: bool,
    ) -> Result<()> {
        client
            .execute(
                "DELETE FROM cluster_standby_nodes WHERE node_id = $1 AND name = $2",
                &[&self.cluster.node_id, &self.cluster.node_name],
            )
            .await
            .context("failed to delete standby node entry")?;
        if holds_lock {
            client
                .execute("SELECT pg_advisory_unlock($1)", &[&self.standby_key()])
                .await
                .context("failed to release standby lock")?;
        }
        Ok(())
    }

    async fn main_loop(&mut self) -> Result<()> {
        // a standby carries its connection across retry ticks: the standby advisory
        // lock lives in that session, so reopening would drop the identity every second
        let (mut client, fresh) = match self.standby_client.take() {
            Some(client) => (client, false),
            None => (
                Object::take(
                    self.db_pool
                        .get()
                        .await
                        .context("failed to get db connection")?,
                ),
                true,
            ),
        };
        let node_key = self.node_key();
        let node_metadata = serde_json::to_value(&self.cluster.metadata)
            .context("failed to serialize node metadata")?;
        let params: [&(dyn tokio_postgres::types::ToSql + Sync); 1] = [&node_key];
        // the standby bookkeeping below needs `&mut self`, so the lock attempt is
        // kept to a value the select! yields rather than handled inside an arm
        let got_lock: bool = tokio::select! {
            _ = self.token.cancelled() => {
                return Ok(());
            }
            res = client.query_one_scalar("SELECT pg_try_advisory_lock($1)", &params) => {
                res.context("failed to acquire node lock")?
            }
        };
        if !got_lock {
            debug!("node lock is held by another node, waiting for it to become available");
            // only a freshly opened session has to claim the identity; a carried-over
            // one still holds the lock, which no other session can take from it
            if fresh && !self.acquire_standby_lock(&client).await? {
                return Ok(());
            }
            self.standby_heartbeat(&client, &node_metadata).await?;
            // keep the session, and with it the lock, alive until the next tick
            self.standby_client = Some(client);
            return Ok(());
        }
        // we're main now, so we're no longer a standby
        self.release_standby(&client, !fresh).await?;
        let mut ticker = time::interval(self.cluster.heartbeat_interval);
        ticker.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
        loop {
            let mut leader_lock = self.node_lock.is(NodeStateType::Leader);
            if !leader_lock {
                leader_lock = client
                    .query_one_scalar(
                        "SELECT pg_try_advisory_lock($1)",
                        &[&(AdvisoryLock::Leader as i64)],
                    )
                    .await
                    .context("failed trying to acquire leader lock")?;
            }
            let tx = client
                .transaction()
                .await
                .context("failed to start transaction")?;
            tx.execute(
                "INSERT INTO cluster_nodes(node_id, name, op_status, last_heartbeat, metadata) \
                VALUES ($1, $2, 1, now(), $3) \
                ON CONFLICT (node_id) DO UPDATE SET \
                name = EXCLUDED.name, \
                last_heartbeat = EXCLUDED.last_heartbeat, \
                metadata = EXCLUDED.metadata",
                &[
                    &self.cluster.node_id,
                    &self.cluster.node_name,
                    &node_metadata,
                ],
            )
            .await
            .context("failed to update node info")?;
            if leader_lock && !self.node_lock.is(NodeStateType::Leader) {
                tx.execute("UPDATE cluster_nodes SET is_leader = false", &[])
                    .await
                    .context("failed to demote leader status")?;
                tx.execute(
                    "UPDATE cluster_nodes SET is_leader = true WHERE node_id = $1",
                    &[&self.cluster.node_id],
                )
                .await
                .context("failed to update leader status")?;
            }
            tx.commit().await.context("failed to commit transaction")?;
            if leader_lock {
                self.node_lock.set_state(NodeStateType::Leader);
                let tx = client
                    .transaction()
                    .await
                    .context("failed to start transaction")?;
                self.leader_tasks(&tx).await?;
                tx.commit().await.context("failed to commit transaction")?;
            } else {
                self.node_lock.set_state(NodeStateType::Main);
            }
            tokio::select! {
                _ = self.token.cancelled() => break,
                _ = ticker.tick() => {}
            }
        }
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(node_id = %self.cluster.node_id, node = %self.cluster.node_name))]
    pub async fn run(mut self) {
        loop {
            match self.main_loop().await {
                Ok(_) => self.node_lock.set_state(NodeStateType::Standby),
                Err(e) => {
                    if is_connection_error(&e) {
                        warn!(error = %format!("{e:#}"), "database connection lost");
                        self.node_lock.set_state(NodeStateType::LostConnection);
                    } else {
                        error!(error = %format!("{e:#}"), "node lock loop failed");
                        self.node_lock.set_state(NodeStateType::Standby);
                    }
                }
            }
            // app_shutdown is cancelled when our standby identity is already taken,
            // which no amount of retrying fixes
            if self.token.is_cancelled() || self.app_shutdown.is_cancelled() {
                break;
            }
            time::sleep(Duration::from_secs(1)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advisory_key_hashes_the_whole_identity() {
        // these two ids share their trailing 7 bytes, which is all the derivation
        // this replaced ever looked at
        let a = advisory_key(
            AdvisoryLock::NodeLock,
            "11111111-1111-1111-1101-020304050607",
        );
        let b = advisory_key(
            AdvisoryLock::NodeLock,
            "22222222-2222-2222-2201-020304050607",
        );
        assert_ne!(a, b, "distinct node ids must not share a node lock");
        // the tag byte still namespaces derived keys away from the fixed ones
        for key in [a, b] {
            assert_eq!(key.to_be_bytes()[7], AdvisoryLock::NodeLock as u8);
            assert_ne!(key, AdvisoryLock::Migration as i64);
            assert_ne!(key, AdvisoryLock::Leader as i64);
        }
        assert_ne!(
            advisory_key(AdvisoryLock::NodeLock, "node/name"),
            advisory_key(AdvisoryLock::StandbyLock, "node/name"),
            "the same identity must yield a different key per lock kind"
        );
    }
}
