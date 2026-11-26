// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::balancer::{self, Move, NodeLoad};
use super::{NodeState, NodeStateType};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use deadpool_postgres::{ClientWrapper, GenericClient, Transaction};
use patrolli::config::cluster::{Cluster as ClusterConfig, ClusterPolicy};
use patrolli::db::AdvisoryLock;
use patrolli::db::Pool;
use patrolli::db::schema::ConfigKeys;
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time;
use tokio_postgres::Error as PostgresError;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

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
    /// The heartbeat this process last committed, kept across a lost connection
    /// and dropped whenever the node goes standby. If the row still carries it
    /// when the lock is taken again, no other process held the lock meanwhile:
    /// a main writes its heartbeat before its watcher touches anything.
    last_heartbeat: Option<DateTime<Utc>>,
    /// Watches the process holding our node lock while this one stands by.
    main_watch: HolderWatch,
}

/// How often a lock holder that stays silent is reported again.
const SILENT_HOLDER_REMINDER: Duration = Duration::from_secs(300);

/// The `cluster_nodes` row of the node holding a lock this one could not take.
struct Holder {
    node_id: Uuid,
    name: String,
    /// How old its heartbeat is.
    age: Duration,
}

#[derive(Debug, PartialEq, Eq)]
enum HolderEvent {
    /// The holder has not written a heartbeat for this long.
    Silent(Duration),
    /// A holder reported silent writes heartbeats again, or was replaced.
    Recovered,
}

/// Tells a lock holder that stopped heartbeating from one that is between two
/// beats. An advisory lock lives as long as its session, so a process that
/// hangs with its connection open keeps the lock forever: nobody can take over
/// from it, and nothing else in the cluster would ever say so.
#[derive(Default)]
struct HolderWatch {
    /// When the holder was first seen without a row (the leader deleted it, or
    /// it never wrote one). It has no heartbeat to age then, so the silence is
    /// counted from here.
    missing_since: Option<Instant>,
    reported_at: Option<Instant>,
}

impl HolderWatch {
    /// `age` is the holder's heartbeat age, `None` when it has no row. Answers
    /// once when the silence passes `limit`, then every
    /// [`SILENT_HOLDER_REMINDER`], and once when it ends.
    fn observe(
        &mut self,
        age: Option<Duration>,
        limit: Duration,
        now: Instant,
    ) -> Option<HolderEvent> {
        let silent = match age {
            Some(age) => {
                self.missing_since = None;
                age
            }
            None => now.duration_since(*self.missing_since.get_or_insert(now)),
        };
        if silent <= limit {
            return self.reported_at.take().map(|_| HolderEvent::Recovered);
        }
        match self.reported_at {
            Some(at) if now.duration_since(at) < SILENT_HOLDER_REMINDER => None,
            _ => {
                self.reported_at = Some(now);
                Some(HolderEvent::Silent(silent))
            }
        }
    }
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
            last_heartbeat: None,
            main_watch: HolderWatch::default(),
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
            None => self.set_policy(ClusterPolicy::default()),
            Some(Ok(policy)) => self.set_policy(policy),
            Some(Err(e)) => {
                error!(error = %format!("{e:#}"), "invalid cluster policy, keeping the last good one")
            }
        }
        Ok(())
    }

    fn set_policy(&mut self, policy: ClusterPolicy) {
        if policy.max_active_nodes != self.policy.max_active_nodes {
            info!(
                max_active_nodes = policy.max_active_nodes,
                "cluster policy applied"
            );
        }
        self.policy = policy;
    }

    /// The row of the lock holder `filter` selects, `None` when it has none.
    async fn holder(
        client: &tokio_postgres::Client,
        filter: &str,
        node_id: Uuid,
    ) -> Result<Option<Holder>> {
        let row = client
            .query_opt(
                &format!(
                    "SELECT node_id, name, \
                        EXTRACT(EPOCH FROM clock_timestamp() - last_heartbeat)::float8 \
                    FROM cluster_nodes WHERE {filter} LIMIT 1"
                ),
                &[&node_id],
            )
            .await
            .context("failed to read the lock holder")?;
        row.map(|row| {
            let age: f64 = row.try_get(2)?;
            Ok(Holder {
                node_id: row.try_get(0)?,
                name: row.try_get(1)?,
                age: Duration::from_secs_f64(age.max(0.0)),
            })
        })
        .transpose()
        .map_err(|e: PostgresError| {
            anyhow::Error::new(e).context("failed to parse the lock holder")
        })
    }

    /// While standing by: say who holds the node lock, and notice a holder
    /// that keeps it without heartbeating, which this standby cannot replace.
    async fn watch_main(&mut self, client: &tokio_postgres::Client, fresh: bool) -> Result<()> {
        let holder = Self::holder(client, "node_id = $1", self.cluster.node_id).await?;
        let main_node = holder.as_ref().map(|h| h.name.as_str());
        let age = holder.as_ref().map(|h| h.age);
        if fresh {
            info!(
                main_node,
                heartbeat_age_ms = age.map(|a| a.as_millis() as u64),
                "node lock is held by another process, standing by"
            );
        }
        match self
            .main_watch
            .observe(age, self.cluster.heartbeat_timeout, Instant::now())
        {
            Some(HolderEvent::Silent(silent)) => error!(
                main_node,
                silent_ms = silent.as_millis() as u64,
                "node lock holder is not heartbeating, this standby cannot take over"
            ),
            Some(HolderEvent::Recovered) => {
                info!(main_node, "node lock holder is heartbeating again")
            }
            None => {}
        }
        Ok(())
    }

    /// While main but not leader: notice a leader that keeps the leader lock
    /// without heartbeating. Nobody marks nodes inactive or moves hosts then.
    async fn watch_leader(
        &self,
        client: &tokio_postgres::Client,
        watch: &mut HolderWatch,
    ) -> Result<()> {
        let holder =
            Self::holder(client, "is_leader AND node_id <> $1", self.cluster.node_id).await?;
        let leader_node = holder.as_ref().map(|h| h.name.as_str());
        let leader_node_id = holder.as_ref().map(|h| tracing::field::display(h.node_id));
        match watch.observe(
            holder.as_ref().map(|h| h.age),
            self.cluster.heartbeat_timeout,
            Instant::now(),
        ) {
            Some(HolderEvent::Silent(silent)) => error!(
                leader_node_id,
                leader_node,
                silent_ms = silent.as_millis() as u64,
                "leader lock holder is not heartbeating, hosts are not failed over"
            ),
            Some(HolderEvent::Recovered) => {
                info!(leader_node_id, leader_node, "leader is heartbeating again")
            }
            None => {}
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

    /// `judged_from` is the heartbeat this leader committed on its previous tick.
    /// A process that just started has none, and judges against the heartbeat it
    /// found on its row, written by whoever held this node id before it: its own
    /// first heartbeat is younger than those a stall holds up by however long
    /// the stall already lasts. Only with no row either is it this tick's.
    /// Every peer is judged against it instead
    /// of the clock: a database stall holds every node's heartbeat up alike, and
    /// the heartbeats it releases still carry the time they were started at.
    /// Against the clock they would all look stale at once. Against a stamp as
    /// old as theirs, only a node that fell behind the leader does. A node that
    /// died is found one tick later for it.
    async fn leader_tasks(
        &mut self,
        tx: &Transaction<'_>,
        judged_from: DateTime<Utc>,
    ) -> Result<()> {
        let expire_after = self.cluster.heartbeat_timeout.as_secs_f64();
        // a node is live while its heartbeat is this fresh; past it, its hosts fail
        // over. An expired node is never live, so its hosts are moved off before
        // the expiry below deletes it.
        let stale_after = self
            .cluster
            .heartbeat_stale_after
            .as_secs_f64()
            .min(expire_after);
        let stale = tx
            .query(
                "UPDATE cluster_nodes SET op_status = 0 \
                WHERE node_id <> $2 \
                  AND op_status <> 0 \
                  AND last_heartbeat + make_interval(secs := $1) < $3::timestamptz \
                RETURNING node_id, name, \
                    EXTRACT(EPOCH FROM clock_timestamp() - last_heartbeat)::float8",
                &[&stale_after, &self.cluster.node_id, &judged_from],
            )
            .await
            .context("failed updating inactive cluster nodes")?;
        for row in stale {
            let peer_node_id: Uuid = row.try_get(0).context("failed to read a stale node")?;
            let peer_node: String = row.try_get(1).context("failed to read a stale node")?;
            let age: f64 = row.try_get(2).context("failed to read a stale node")?;
            warn!(
                %peer_node_id,
                %peer_node,
                heartbeat_age_ms = (age.max(0.0) * 1000.0) as u64,
                "node marked inactive, its heartbeat is stale"
            );
        }
        self.refresh_policy(tx).await?;

        let rows = tx
            .query(
                "SELECT cn.node_id, \
                    cn.node_id = $2 OR cn.last_heartbeat + make_interval(secs := $1) >= $3::timestamptz, \
                    COUNT(h.host_id) FILTER (WHERE NOT h.sticky_monitored_by) \
                FROM cluster_nodes cn \
                LEFT JOIN hosts h ON h.monitored_by = cn.node_id \
                GROUP BY cn.node_id, cn.last_heartbeat",
                &[&stale_after, &self.cluster.node_id, &judged_from],
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

        debug!(
            nodes = nodes.len(),
            live = nodes.iter().filter(|n| n.live).count(),
            unassigned,
            max_active_nodes = self.policy.max_active_nodes,
            "planning host placement"
        );
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
            WHERE node_id <> $2 \
              AND last_heartbeat + make_interval(secs := $1) < $3::timestamptz)",
            &[&expire_after, &self.cluster.node_id, &judged_from],
        )
        .await
        .context("failed to delete cluster logs")?;
        let removed = tx
            .query(
                "DELETE FROM cluster_nodes \
                WHERE node_id <> $2 \
                  AND last_heartbeat + make_interval(secs := $1) < $3::timestamptz \
                  AND NOT EXISTS (SELECT 1 FROM hosts h WHERE h.monitored_by = cluster_nodes.node_id) \
                RETURNING node_id, name",
                &[&expire_after, &self.cluster.node_id, &judged_from],
            )
            .await
            .context("failed to delete inactive cluster nodes")?;
        for row in removed {
            let peer_node_id: Uuid = row.try_get(0).context("failed to read a removed node")?;
            let peer_node: String = row.try_get(1).context("failed to read a removed node")?;
            info!(%peer_node_id, %peer_node, "expired cluster node removed");
        }
        tx.execute(
            "DELETE FROM cluster_standby_nodes \
            WHERE last_heartbeat + make_interval(secs := $1) < $2::timestamptz",
            &[&expire_after, &judged_from],
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
                self.db_pool
                    .get()
                    .await
                    .context("failed to get db connection")?
                    .detach(),
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
            self.watch_main(&client, fresh).await?;
            // keep the session, and with it the lock, alive until the next tick
            self.standby_client = Some(client);
            return Ok(());
        }
        // we're main now, so we're no longer a standby
        self.release_standby(&client, !fresh).await?;
        self.main_watch = HolderWatch::default();
        let mut leader_watch = HolderWatch::default();
        // when this tenure last committed a heartbeat, on this process's clock
        let mut last_beat: Option<Instant> = None;
        let expire_after = self.cluster.heartbeat_timeout.as_secs_f64();
        let mut ticker = time::interval(self.cluster.heartbeat_interval);
        ticker.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
        // the first heartbeat of a tenure decides how it starts (see below); later
        // ones only watch for the leader having expired this node
        let mut first_tick = true;
        loop {
            let tick_started = Instant::now();
            let was_leader = self.node_lock.is(NodeStateType::Leader);
            let mut leader_lock = was_leader;
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
            // `prev` reads the statement's snapshot, so it is the heartbeat this one
            // replaces (none: the row was deleted). It is judged against the clock
            // at the time the upsert is done, not the transaction's `now()`, which
            // a statement that was held up on its way would leave in the past.
            let row = tx
                .query_one(
                    "WITH prev AS (\
                        SELECT last_heartbeat, op_status FROM cluster_nodes WHERE node_id = $1\
                    ), beat AS (\
                        INSERT INTO cluster_nodes(node_id, name, op_status, last_heartbeat, metadata) \
                        VALUES ($1, $2, 1, now(), $3) \
                        ON CONFLICT (node_id) DO UPDATE SET \
                        name = EXCLUDED.name, \
                        op_status = CASE WHEN cluster_nodes.op_status = 0 THEN 1 ELSE cluster_nodes.op_status END, \
                        last_heartbeat = EXCLUDED.last_heartbeat, \
                        metadata = EXCLUDED.metadata \
                        RETURNING last_heartbeat\
                    ) \
                    SELECT COALESCE(\
                        (SELECT last_heartbeat FROM prev) + make_interval(secs := $4) < clock_timestamp(), \
                        true), \
                        COALESCE((SELECT last_heartbeat FROM prev) = $5::timestamptz, false), \
                        beat.last_heartbeat, \
                        (SELECT op_status FROM prev), \
                        (SELECT last_heartbeat FROM prev) \
                    FROM beat",
                    &[
                        &self.cluster.node_id,
                        &self.cluster.node_name,
                        &node_metadata,
                        &expire_after,
                        &self.last_heartbeat,
                    ],
                )
                .await
                .context("failed to update node info")?;
            let expired: bool = row
                .try_get(0)
                .context("failed to read the previous heartbeat")?;
            // the row still carried the heartbeat this process wrote last
            let untouched: bool = row
                .try_get(1)
                .context("failed to compare the previous heartbeat")?;
            let heartbeat: DateTime<Utc> =
                row.try_get(2).context("failed to read the heartbeat")?;
            // what the leader made of this node since its last heartbeat
            let prev_status: Option<i16> = row
                .try_get(3)
                .context("failed to read the previous status")?;
            // the heartbeat this one replaces, whoever wrote it
            let found: Option<DateTime<Utc>> = row
                .try_get(4)
                .context("failed to read the replaced heartbeat")?;
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
            // what the leader tasks below judge the other nodes against
            let judged_from = self.last_heartbeat.or(found).unwrap_or(heartbeat);
            // only once committed: after a commit that failed, the row may or may
            // not carry it, and not matching is the answer that reloads
            self.last_heartbeat = Some(heartbeat);
            let since_last_heartbeat_ms = last_beat.map(|at| at.elapsed().as_millis() as u64);
            last_beat = Some(Instant::now());
            // A lock taken again after a lost connection continues the tenure it
            // interrupted when nothing can have been missed: the leader has not
            // expired this node (its cluster logs are all still there), and no
            // other process was main on this id in between (none read those logs
            // away). The watcher then carries on from the logs instead of closing
            // every listener and sink to load what it already holds.
            let resume = first_tick && untouched && !expired;
            // A heartbeat this late kept the session, and with it the node lock,
            // but a leader that saw it expired has moved our hosts, purged our
            // cluster logs and deleted our row: what the watcher holds is no longer
            // what the logs can bring up to date. Only after the commit, so the
            // reload finds the row. A node that led throughout was judged by nobody.
            if expired && !first_tick && !was_leader {
                warn!(
                    since_last_heartbeat_ms,
                    "heartbeat expired while holding the node lock, configuration will be reloaded"
                );
                self.node_lock.renew_main_epoch();
            } else if prev_status == Some(0) && !first_tick {
                // late, but not past the timeout: the row and the logs are intact,
                // only the hosts may have been failed over, which the logs carry
                warn!(
                    since_last_heartbeat_ms,
                    "heartbeat was late, the leader had marked this node inactive"
                );
            }
            first_tick = false;
            let state = if leader_lock {
                NodeStateType::Leader
            } else {
                NodeStateType::Main
            };
            if resume {
                info!("node lock taken again with nothing missed, configuration kept");
                self.node_lock.resume_main(state);
            } else {
                self.node_lock.set_state(state);
            }
            if leader_lock {
                let tx = client
                    .transaction()
                    .await
                    .context("failed to start transaction")?;
                self.leader_tasks(&tx, judged_from).await?;
                tx.commit().await.context("failed to commit transaction")?;
                leader_watch = HolderWatch::default();
            } else {
                self.watch_leader(&client, &mut leader_watch).await?;
            }
            let elapsed = tick_started.elapsed();
            if elapsed > self.cluster.heartbeat_interval {
                warn!(
                    elapsed_ms = elapsed.as_millis() as u64,
                    leader = leader_lock,
                    "node lock tick took longer than the heartbeat interval"
                );
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
                Ok(_) => {
                    self.last_heartbeat = None;
                    self.node_lock.set_state(NodeStateType::Standby)
                }
                Err(e) => {
                    if is_connection_error(&e) {
                        warn!(error = %format!("{e:#}"), "database connection lost");
                        self.node_lock.set_state(NodeStateType::LostConnection);
                    } else {
                        error!(error = %format!("{e:#}"), "node lock loop failed");
                        // standby clears what the watcher holds: nothing to resume
                        self.last_heartbeat = None;
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

    #[test]
    fn holder_watch_reports_silence_once_then_reminds_and_recovers() {
        let limit = Duration::from_secs(15);
        let t0 = Instant::now();
        let at = |secs: u64| t0 + Duration::from_secs(secs);
        let age = |secs: u64| Some(Duration::from_secs(secs));
        let mut watch = HolderWatch::default();
        // between two beats: nothing to say, and nothing to recover from
        assert_eq!(watch.observe(age(3), limit, at(0)), None);
        assert_eq!(
            watch.observe(age(16), limit, at(1)),
            Some(HolderEvent::Silent(Duration::from_secs(16)))
        );
        assert_eq!(watch.observe(age(17), limit, at(2)), None);
        let again = 1 + SILENT_HOLDER_REMINDER.as_secs();
        assert_eq!(
            watch.observe(age(400), limit, at(again)),
            Some(HolderEvent::Silent(Duration::from_secs(400)))
        );
        assert_eq!(
            watch.observe(age(1), limit, at(again + 1)),
            Some(HolderEvent::Recovered)
        );
        assert_eq!(watch.observe(age(2), limit, at(again + 2)), None);
    }

    #[test]
    fn holder_watch_counts_a_missing_row_from_when_it_was_first_seen() {
        let limit = Duration::from_secs(15);
        let t0 = Instant::now();
        let at = |secs: u64| t0 + Duration::from_secs(secs);
        let mut watch = HolderWatch::default();
        // a main that just took the lock has not written its row yet
        assert_eq!(watch.observe(None, limit, at(0)), None);
        assert_eq!(watch.observe(None, limit, at(15)), None);
        assert_eq!(
            watch.observe(None, limit, at(16)),
            Some(HolderEvent::Silent(Duration::from_secs(16)))
        );
        // a row in between restarts the count
        assert_eq!(
            watch.observe(Some(Duration::ZERO), limit, at(17)),
            Some(HolderEvent::Recovered)
        );
        assert_eq!(watch.observe(None, limit, at(18)), None);
        assert_eq!(watch.observe(None, limit, at(30)), None);
    }

    #[test]
    fn is_connection_error_classifies_transient_io_and_pool_errors() {
        for kind in [
            io::ErrorKind::ConnectionRefused,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::TimedOut,
            io::ErrorKind::BrokenPipe,
            io::ErrorKind::NotConnected,
        ] {
            let err =
                anyhow::Error::new(io::Error::new(kind, "transient")).context("outer wrapper");
            assert!(
                is_connection_error(&err),
                "expected {kind:?} to be treated as a connection error"
            );
        }

        let pool_err = anyhow::Error::new(deadpool_postgres::PoolError::Timeout(
            deadpool_postgres::TimeoutType::Wait,
        ))
        .context("failed to get db connection");
        assert!(
            is_connection_error(&pool_err),
            "pool wait timeout must be treated as a connection error"
        );

        let non_conn_io =
            anyhow::Error::new(io::Error::new(io::ErrorKind::InvalidInput, "bad input"));
        assert!(
            !is_connection_error(&non_conn_io),
            "non-connection io error must not trigger LostConnection"
        );

        let plain = anyhow::anyhow!("serialization or query bug");
        assert!(!is_connection_error(&plain));
    }
}
