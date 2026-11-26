// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

mod setup;

use anyhow::{Context, Result, ensure};
use patrolli::config;
use patrolli::db::Pool;
use patrolli_cluster as cluster;
use setup::eventually;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Three heartbeat ticks: long enough for a move that must not happen to have
/// happened. Only for those negative checks; anything that should happen is
/// waited for with [`eventually`].
const QUIET: Duration = Duration::from_secs(3);

/// Every live standby currently registered, ordered by name so it can be
/// compared against an expectation built in node order.
async fn standby_entries(pool: &Pool) -> Result<Vec<(Uuid, String)>> {
    let conn = pool.get().await.context("failed to get connection")?;
    conn.query(
        "SELECT node_id, name FROM cluster_standby_nodes ORDER BY name",
        &[],
    )
    .await
    .context("failed to query cluster_standby_nodes")?
    .iter()
    .map(|row| {
        Ok((
            row.try_get(0).context("failed to get node_id column")?,
            row.try_get(1).context("failed to get name column")?,
        ))
    })
    .collect()
}

/// Wait until `cluster_standby_nodes` holds exactly `expected`.
async fn wait_for_standby_entries(pool: &Pool, expected: Vec<(Uuid, String)>) {
    eventually(async || {
        let entries = standby_entries(pool).await?;
        ensure!(
            entries == expected,
            "standby entries are {entries:?}, waiting for {expected:?}"
        );
        Ok(())
    })
    .await
}

/// Wait until `state` holds the node lock, as leader or main.
async fn wait_until_main(state: &cluster::NodeState) {
    eventually(async || {
        ensure!(
            state.is_main(),
            "node is {:?}, waiting for it to take the node lock",
            state.get_state()
        );
        Ok(())
    })
    .await
}

fn node_lock(
    db_pool: &Pool,
    token: &CancellationToken,
    app_shutdown: &CancellationToken,
    node_id: Uuid,
    node_name: &str,
    state: &Arc<cluster::NodeState>,
) -> cluster::node_lock::NodeLock {
    node_lock_with_timeout(
        db_pool,
        token,
        app_shutdown,
        node_id,
        node_name,
        state,
        setup::CLUSTER_HEARTBEAT_TIMEOUT,
    )
}

fn node_lock_with_timeout(
    db_pool: &Pool,
    token: &CancellationToken,
    app_shutdown: &CancellationToken,
    node_id: Uuid,
    node_name: &str,
    state: &Arc<cluster::NodeState>,
    heartbeat_timeout: Duration,
) -> cluster::node_lock::NodeLock {
    node_lock_with(
        db_pool,
        token,
        app_shutdown,
        node_id,
        node_name,
        state,
        heartbeat_timeout,
        setup::CLUSTER_HEARTBEAT_STALE_AFTER,
    )
}

#[allow(clippy::too_many_arguments)]
fn node_lock_with(
    db_pool: &Pool,
    token: &CancellationToken,
    app_shutdown: &CancellationToken,
    node_id: Uuid,
    node_name: &str,
    state: &Arc<cluster::NodeState>,
    heartbeat_timeout: Duration,
    heartbeat_stale_after: Duration,
) -> cluster::node_lock::NodeLock {
    cluster::node_lock::NodeLock::new(
        db_pool.clone(),
        token.clone(),
        app_shutdown.clone(),
        config::cluster::Cluster {
            node_id,
            node_name: node_name.to_string(),
            heartbeat_timeout,
            heartbeat_interval: setup::CLUSTER_HEARTBEAT_INTERVAL,
            heartbeat_stale_after,
            inter_node: config::cluster::InterNode::default(),
            metadata: config::cluster::NodeMetadata {
                advertise_address: String::new(),
            },
        },
        state.clone(),
    )
}

/// Hosts per owner; `None` counts the unassigned ones.
async fn host_counts(pool: &Pool) -> Result<HashMap<Option<Uuid>, i64>> {
    let conn = pool.get().await.context("failed to get connection")?;
    conn.query("SELECT monitored_by, count(*) FROM hosts GROUP BY 1", &[])
        .await
        .context("failed to query hosts")?
        .iter()
        .map(|row| {
            Ok((
                row.try_get(0)
                    .context("failed to get monitored_by column")?,
                row.try_get(1).context("failed to get count column")?,
            ))
        })
        .collect()
}

/// Wait until the hosts are spread exactly as `expected`.
async fn wait_for_host_counts(pool: &Pool, expected: HashMap<Option<Uuid>, i64>) {
    eventually(async || {
        let counts = host_counts(pool).await?;
        ensure!(
            counts == expected,
            "hosts per owner are {counts:?}, waiting for {expected:?}"
        );
        Ok(())
    })
    .await
}

/// Wait until every host is assigned, spread over `nodes` owners with `per_node`
/// hosts each.
async fn wait_for_even_spread(pool: &Pool, nodes: usize, per_node: i64) {
    eventually(async || {
        let counts = host_counts(pool).await?;
        ensure!(
            counts.len() == nodes
                && counts
                    .iter()
                    .all(|(owner, &hosts)| owner.is_some() && hosts == per_node),
            "hosts per owner are {counts:?}, waiting for {nodes} nodes with {per_node} each"
        );
        Ok(())
    })
    .await
}

async fn set_cluster_policy(pool: &Pool, policy: serde_json::Value) -> Result<()> {
    let conn = pool.get().await.context("failed to get connection")?;
    conn.execute(
        "INSERT INTO configs (key, val, last_updated) VALUES ('cluster', $1, now()) \
        ON CONFLICT (key) DO UPDATE SET val = EXCLUDED.val, last_updated = now()",
        &[&policy],
    )
    .await
    .context("failed to set cluster policy")?;
    Ok(())
}

async fn create_hosts(pool: &Pool, count: usize) -> Result<Vec<Uuid>> {
    let mut conn = pool.get().await.context("failed to get connection")?;
    let tx = conn
        .transaction()
        .await
        .context("failed to start transaction")?;
    let mut ids = Vec::with_capacity(count);
    for i in 0..count {
        ids.push(setup::create_host(&tx, &format!("host-{i}")).await?);
    }
    tx.commit().await.context("failed to commit transaction")?;
    Ok(ids)
}

struct RunningNode {
    node_id: Uuid,
    name: String,
    state: Arc<cluster::NodeState>,
    token: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl RunningNode {
    fn start(pool: &Pool, node_id: Uuid, name: &str, heartbeat_timeout: Duration) -> Self {
        Self::start_with(
            pool,
            node_id,
            name,
            heartbeat_timeout,
            setup::CLUSTER_HEARTBEAT_STALE_AFTER,
        )
    }

    fn start_with(
        pool: &Pool,
        node_id: Uuid,
        name: &str,
        heartbeat_timeout: Duration,
        heartbeat_stale_after: Duration,
    ) -> Self {
        let token = CancellationToken::new();
        let state = Arc::new(cluster::NodeState::default());
        let lock = node_lock_with(
            pool,
            &token,
            &CancellationToken::new(),
            node_id,
            name,
            &state,
            heartbeat_timeout,
            heartbeat_stale_after,
        );
        Self {
            node_id,
            name: name.to_string(),
            state,
            token,
            task: tokio::spawn(lock.run()),
        }
    }

    async fn stop(self) -> Result<()> {
        self.token.cancel();
        self.task.await?;
        Ok(())
    }
}

#[tokio::test]
async fn test_spare_failover() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    // well past the 3s stale threshold, so the hosts fail over as a block before
    // the dead node is deleted
    let timeout = Duration::from_secs(30);
    set_cluster_policy(&db_pool, serde_json::json!({"max_active_nodes": 1})).await?;
    let mut nodes = vec![
        RunningNode::start(&db_pool, Uuid::now_v7(), "node-1", timeout),
        RunningNode::start(&db_pool, Uuid::now_v7(), "node-2", timeout),
    ];
    for node in &nodes {
        wait_until_main(&node.state).await;
    }
    create_hosts(&db_pool, 4).await?;

    let active = eventually(async || {
        let counts = host_counts(&db_pool).await?;
        match counts.iter().collect::<Vec<_>>()[..] {
            [(&Some(owner), &4)] => Ok(owner),
            _ => anyhow::bail!("waiting for one node to carry all 4 hosts: {counts:?}"),
        }
    })
    .await;
    let idx = nodes
        .iter()
        .position(|n| n.node_id == active)
        .expect("hosts are owned by a running node");
    let failed = nodes.remove(idx);
    let spare = nodes[0].node_id;
    let (failed_id, failed_name) = (failed.node_id, failed.name.clone());
    failed.stop().await?;
    // the spare takes the failed node's hosts
    wait_for_host_counts(&db_pool, HashMap::from([(Some(spare), 4)])).await;

    // the failed node comes back empty and stays the spare: nothing moves back
    nodes.push(RunningNode::start(
        &db_pool,
        failed_id,
        &failed_name,
        timeout,
    ));
    wait_until_main(&nodes[1].state).await;
    sleep(QUIET).await;
    assert_eq!(
        host_counts(&db_pool).await?,
        HashMap::from([(Some(spare), 4)]),
        "a returning node does not pull hosts back"
    );

    for node in nodes {
        node.stop().await?;
    }
    Ok(())
}

#[tokio::test]
async fn test_max_active_shrink() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    let nodes: Vec<_> = (1..=3)
        .map(|i| {
            RunningNode::start(
                &db_pool,
                Uuid::now_v7(),
                &format!("node-{i}"),
                setup::CLUSTER_HEARTBEAT_TIMEOUT,
            )
        })
        .collect();
    for node in &nodes {
        wait_until_main(&node.state).await;
    }
    create_hosts(&db_pool, 6).await?;
    // unlimited spreads over every node
    wait_for_even_spread(&db_pool, 3, 2).await;

    set_cluster_policy(&db_pool, serde_json::json!({"max_active_nodes": 2})).await?;
    // one node is evacuated
    wait_for_even_spread(&db_pool, 2, 3).await;

    for node in nodes {
        node.stop().await?;
    }
    Ok(())
}

#[tokio::test]
async fn test_sticky_waits_for_node() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    let timeout = setup::CLUSTER_HEARTBEAT_TIMEOUT;
    let node_a = RunningNode::start(&db_pool, Uuid::now_v7(), "node-a", timeout);
    let node_b = RunningNode::start(&db_pool, Uuid::now_v7(), "node-b", timeout);
    wait_until_main(&node_a.state).await;
    wait_until_main(&node_b.state).await;
    let (a_id, a_name) = (node_a.node_id, node_a.name.clone());
    let hosts = create_hosts(&db_pool, 3).await?;
    let conn = db_pool.get().await.context("failed to get connection")?;
    conn.execute(
        "UPDATE hosts SET monitored_by = $1, sticky_monitored_by = true WHERE host_id = $2",
        &[&a_id, &hosts[0]],
    )
    .await
    .context("failed to pin host")?;
    eventually(async || {
        let counts = host_counts(&db_pool).await?;
        ensure!(
            !counts.contains_key(&None),
            "waiting for every host to be assigned: {counts:?}"
        );
        Ok(())
    })
    .await;

    node_a.stop().await?;
    // past heartbeat_timeout, when the node would otherwise be deleted
    eventually(async || {
        let expired: Option<bool> = conn
            .query_opt(
                "SELECT last_heartbeat + make_interval(secs := $2) < now() \
                FROM cluster_nodes WHERE node_id = $1",
                &[&a_id, &timeout.as_secs_f64()],
            )
            .await?
            .map(|row| row.try_get(0))
            .transpose()?;
        ensure!(
            expired.context("the node owning a sticky host lost its row")?,
            "waiting for node-a's heartbeat to expire"
        );
        Ok(())
    })
    .await;
    // what a config change logs for every node, the retained one included
    conn.execute(
        "INSERT INTO cluster_logs (node_id, resource, resource_id, op) VALUES ($1, 'host', $2, 1)",
        &[&a_id, &hosts[1]],
    )
    .await
    .context("failed to insert cluster log")?;
    // a leader tick after the expiry: the same cleanup purges the logs and would
    // have deleted the node
    eventually(async || {
        let logs: i64 = conn
            .query_one(
                "SELECT count(*) FROM cluster_logs WHERE node_id = $1",
                &[&a_id],
            )
            .await?
            .try_get(0)?;
        ensure!(logs == 0, "a retained node's logs are purged every tick");
        Ok(())
    })
    .await;
    // node-a's other host fails over to node-b
    eventually(async || {
        let counts = host_counts(&db_pool).await?;
        ensure!(
            counts.get(&Some(node_b.node_id)) == Some(&2),
            "waiting for node-b to carry 2 hosts: {counts:?}"
        );
        Ok(())
    })
    .await;

    let owner: Option<Uuid> = conn
        .query_one(
            "SELECT monitored_by FROM hosts WHERE host_id = $1",
            &[&hosts[0]],
        )
        .await?
        .try_get(0)?;
    assert_eq!(owner, Some(a_id), "the sticky host waits for its node");
    let op_status: i16 = conn
        .query_one(
            "SELECT op_status FROM cluster_nodes WHERE node_id = $1",
            &[&a_id],
        )
        .await
        .context("the node owning a sticky host keeps its row")?
        .try_get(0)?;
    assert_eq!(op_status, 0);
    assert!(
        node_b.state.is(cluster::NodeStateType::Leader),
        "the leader is not demoted by the retained row, got {:?}",
        node_b.state.get_state()
    );

    let node_a = RunningNode::start(&db_pool, a_id, &a_name, timeout);
    wait_until_main(&node_a.state).await;
    sleep(QUIET).await;
    let owner: Option<Uuid> = conn
        .query_one(
            "SELECT monitored_by FROM hosts WHERE host_id = $1",
            &[&hosts[0]],
        )
        .await?
        .try_get(0)?;
    assert_eq!(
        owner,
        Some(a_id),
        "the returning node still owns its sticky host"
    );

    node_a.stop().await?;
    node_b.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_main_standby_leader() {
    setup::setup_logging();
    let (_db, pool) = setup::setup_db().await;
    let db_pool = pool;
    let token_1 = CancellationToken::new();
    let token_2 = CancellationToken::new();
    let token_3 = CancellationToken::new();
    let token_4 = CancellationToken::new();
    let shutdown_1 = CancellationToken::new();
    let shutdown_2 = CancellationToken::new();
    let shutdown_3 = CancellationToken::new();
    let shutdown_4 = CancellationToken::new();
    let node_1_id = Uuid::now_v7();
    let node_2_id = Uuid::now_v7();
    let node_1_state = Arc::new(cluster::NodeState::default());
    let node_2_state = Arc::new(cluster::NodeState::default());
    let node_3_state = Arc::new(cluster::NodeState::default());
    let node_4_state = Arc::new(cluster::NodeState::default());
    let node_1_lock = cluster::node_lock::NodeLock::new(
        db_pool.clone(),
        token_1.clone(),
        shutdown_1.clone(),
        config::cluster::Cluster {
            node_id: node_1_id,
            node_name: "node-1".to_string(),
            heartbeat_timeout: setup::CLUSTER_HEARTBEAT_TIMEOUT,
            heartbeat_interval: setup::CLUSTER_HEARTBEAT_INTERVAL,
            heartbeat_stale_after: setup::CLUSTER_HEARTBEAT_STALE_AFTER,
            inter_node: config::cluster::InterNode::default(),
            metadata: config::cluster::NodeMetadata {
                advertise_address: String::new(),
            },
        },
        node_1_state.clone(),
    );
    let node_2_lock = cluster::node_lock::NodeLock::new(
        db_pool.clone(),
        token_2.clone(),
        shutdown_2.clone(),
        config::cluster::Cluster {
            node_id: node_1_id,
            node_name: "node-2".to_string(),
            heartbeat_timeout: setup::CLUSTER_HEARTBEAT_TIMEOUT,
            heartbeat_interval: setup::CLUSTER_HEARTBEAT_INTERVAL,
            heartbeat_stale_after: setup::CLUSTER_HEARTBEAT_STALE_AFTER,
            inter_node: config::cluster::InterNode::default(),
            metadata: config::cluster::NodeMetadata {
                advertise_address: String::new(),
            },
        },
        node_2_state.clone(),
    );
    let node_3_lock = cluster::node_lock::NodeLock::new(
        db_pool.clone(),
        token_3.clone(),
        shutdown_3.clone(),
        config::cluster::Cluster {
            node_id: node_2_id,
            node_name: "node-3".to_string(),
            heartbeat_timeout: setup::CLUSTER_HEARTBEAT_TIMEOUT,
            heartbeat_interval: setup::CLUSTER_HEARTBEAT_INTERVAL,
            heartbeat_stale_after: setup::CLUSTER_HEARTBEAT_STALE_AFTER,
            inter_node: config::cluster::InterNode::default(),
            metadata: config::cluster::NodeMetadata {
                advertise_address: String::new(),
            },
        },
        node_3_state.clone(),
    );
    let node_4_lock = cluster::node_lock::NodeLock::new(
        db_pool.clone(),
        token_4.clone(),
        shutdown_4.clone(),
        config::cluster::Cluster {
            node_id: node_2_id,
            node_name: "node-4".to_string(),
            heartbeat_timeout: setup::CLUSTER_HEARTBEAT_TIMEOUT,
            heartbeat_interval: setup::CLUSTER_HEARTBEAT_INTERVAL,
            heartbeat_stale_after: setup::CLUSTER_HEARTBEAT_STALE_AFTER,
            inter_node: config::cluster::InterNode::default(),
            metadata: config::cluster::NodeMetadata {
                advertise_address: String::new(),
            },
        },
        node_4_state.clone(),
    );
    assert!(node_1_state.is(cluster::NodeStateType::LostConnection));
    assert!(node_2_state.is(cluster::NodeStateType::LostConnection));
    assert!(node_3_state.is(cluster::NodeStateType::LostConnection));
    assert!(node_4_state.is(cluster::NodeStateType::LostConnection));
    let node_1_task = tokio::spawn(async move {
        node_1_lock.run().await;
    });
    let node_2_task = tokio::spawn(async move {
        node_2_lock.run().await;
    });
    let node_3_task = tokio::spawn(async move {
        node_3_lock.run().await;
    });
    let node_4_task = tokio::spawn(async move {
        node_4_lock.run().await;
    });

    // in pairs: node-1 and node-2 share node_1_id, node-3 and node-4 node_2_id
    let nodes = [
        ("node-1", &node_1_state, node_1_id),
        ("node-2", &node_2_state, node_1_id),
        ("node-3", &node_3_state, node_2_id),
        ("node-4", &node_4_state, node_2_id),
    ];
    let tokens = [&token_1, &token_2, &token_3, &token_4];
    let states = || {
        nodes
            .iter()
            .map(|(name, state, _)| (*name, state.get_state()))
            .collect::<Vec<_>>()
    };
    let leaders = || {
        nodes
            .iter()
            .filter(|(_, state, _)| state.is(cluster::NodeStateType::Leader))
            .map(|(name, _, _)| *name)
            .collect::<Vec<_>>()
    };

    eventually(async || {
        ensure!(
            leaders().len() == 1,
            "expected exactly one leader: {:?}",
            states()
        );
        for pair in nodes.chunks(2) {
            let standbys = pair
                .iter()
                .filter(|(_, state, _)| state.is(cluster::NodeStateType::Standby))
                .count();
            ensure!(
                standbys == 1,
                "expected one standby in each pair: {:?}",
                states()
            );
        }
        // only the standby of each pair registers itself, the main/leader never does
        let expected = nodes
            .iter()
            .filter(|(_, state, _)| state.is(cluster::NodeStateType::Standby))
            .map(|(name, _, node_id)| (*node_id, name.to_string()))
            .collect::<Vec<_>>();
        let entries = standby_entries(&db_pool).await?;
        ensure!(
            entries == expected,
            "expected cluster_standby_nodes to hold exactly the standby nodes \
             {expected:?}, got {entries:?}"
        );
        Ok(())
    })
    .await;

    // demote the current leader: its sibling, the standby sharing its node_id,
    // takes over the node lock and a new leader is elected
    let old = nodes
        .iter()
        .position(|(_, state, _)| state.is(cluster::NodeStateType::Leader))
        .expect("a leader was elected");
    tokens[old].cancel();
    let (leader, promoted) = (nodes[old].0, nodes[old ^ 1].0);
    eventually(async || {
        // the sibling took over the node lock, so it must have dropped its standby
        // entry, leaving only the standby of the untouched pair
        let entries = standby_entries(&db_pool).await?;
        ensure!(
            !entries.iter().any(|(_, name)| name == promoted) && entries.len() == 1,
            "expected only the surviving pair's standby to remain once {promoted} \
             is promoted, got {entries:?}"
        );
        let leaders = leaders();
        ensure!(
            leaders.len() == 1 && leaders[0] != leader,
            "expected a new leader in place of {leader}: {:?}",
            states()
        );
        Ok(())
    })
    .await;

    token_1.cancel();
    token_2.cancel();
    token_3.cancel();
    token_4.cancel();
    node_1_task.await.unwrap();
    node_2_task.await.unwrap();
    node_3_task.await.unwrap();
    node_4_task.await.unwrap();
}

#[tokio::test]
async fn test_host_distribution() -> Result<()> {
    setup::setup_logging();
    let (_db, pool) = setup::setup_db().await;
    let db_pool = pool;
    let token_1 = CancellationToken::new();
    let token_2 = CancellationToken::new();
    let shutdown_1 = CancellationToken::new();
    let shutdown_2 = CancellationToken::new();
    let node_1_id = Uuid::now_v7();
    let node_2_id = Uuid::now_v7();
    let node_1_state = Arc::new(cluster::NodeState::default());
    let node_2_state = Arc::new(cluster::NodeState::default());
    let node_1_lock = cluster::node_lock::NodeLock::new(
        db_pool.clone(),
        token_1.clone(),
        shutdown_1.clone(),
        config::cluster::Cluster {
            node_id: node_1_id,
            node_name: "node-1".to_string(),
            heartbeat_timeout: setup::CLUSTER_HEARTBEAT_TIMEOUT,
            heartbeat_interval: setup::CLUSTER_HEARTBEAT_INTERVAL,
            heartbeat_stale_after: setup::CLUSTER_HEARTBEAT_STALE_AFTER,
            inter_node: config::cluster::InterNode::default(),
            metadata: config::cluster::NodeMetadata {
                advertise_address: String::new(),
            },
        },
        node_1_state.clone(),
    );
    let node_2_lock = cluster::node_lock::NodeLock::new(
        db_pool.clone(),
        token_2.clone(),
        shutdown_2.clone(),
        config::cluster::Cluster {
            node_id: node_2_id,
            node_name: "node-2".to_string(),
            heartbeat_timeout: setup::CLUSTER_HEARTBEAT_TIMEOUT,
            heartbeat_interval: setup::CLUSTER_HEARTBEAT_INTERVAL,
            heartbeat_stale_after: setup::CLUSTER_HEARTBEAT_STALE_AFTER,
            inter_node: config::cluster::InterNode::default(),
            metadata: config::cluster::NodeMetadata {
                advertise_address: String::new(),
            },
        },
        node_2_state.clone(),
    );
    let node_1_task = tokio::spawn(async move {
        node_1_lock.run().await;
    });
    let node_2_task = tokio::spawn(async move {
        node_2_lock.run().await;
    });
    create_hosts(&db_pool, 4).await?;
    wait_for_host_counts(
        &db_pool,
        HashMap::from([(Some(node_1_id), 2), (Some(node_2_id), 2)]),
    )
    .await;
    token_1.cancel();
    node_1_task.await?;
    wait_for_host_counts(&db_pool, HashMap::from([(Some(node_2_id), 4)])).await;
    token_2.cancel();
    node_2_task.await?;
    Ok(())
}

#[tokio::test]
async fn test_duplicate_standby_name() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    let node_id = Uuid::now_v7();
    let token_main = CancellationToken::new();
    let token_standby = CancellationToken::new();
    let token_dup = CancellationToken::new();
    let shutdown_main = CancellationToken::new();
    let shutdown_standby = CancellationToken::new();
    let shutdown_dup = CancellationToken::new();
    let main_state = Arc::new(cluster::NodeState::default());
    let standby_state = Arc::new(cluster::NodeState::default());
    let dup_state = Arc::new(cluster::NodeState::default());

    // main takes the node lock first
    let main_lock = node_lock(
        &db_pool,
        &token_main,
        &shutdown_main,
        node_id,
        "main-node",
        &main_state,
    );
    let main_task = tokio::spawn(async move { main_lock.run().await });
    wait_until_main(&main_state).await;

    // the standby registers itself under (node_id, "standby-node")
    let standby_lock = node_lock(
        &db_pool,
        &token_standby,
        &shutdown_standby,
        node_id,
        "standby-node",
        &standby_state,
    );
    let standby_task = tokio::spawn(async move { standby_lock.run().await });
    wait_for_standby_entries(&db_pool, vec![(node_id, "standby-node".to_string())]).await;

    // a second process with the very same identity must shut the program down
    let dup_lock = node_lock(
        &db_pool,
        &token_dup,
        &shutdown_dup,
        node_id,
        "standby-node",
        &dup_state,
    );
    let dup_task = tokio::spawn(async move { dup_lock.run().await });
    eventually(async || {
        ensure!(
            shutdown_dup.is_cancelled(),
            "expected the duplicate standby to cancel its app shutdown token"
        );
        Ok(())
    })
    .await;
    assert!(
        !shutdown_standby.is_cancelled() && !shutdown_main.is_cancelled(),
        "Expected the incumbent nodes to keep running"
    );
    assert_eq!(
        standby_entries(&db_pool).await?,
        vec![(node_id, "standby-node".to_string())],
        "Expected the incumbent standby to keep its entry"
    );

    token_main.cancel();
    token_standby.cancel();
    token_dup.cancel();
    main_task.await?;
    standby_task.await?;
    dup_task.await?;
    Ok(())
}

#[tokio::test]
async fn test_standby_restart_is_immediate() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    let node_id = Uuid::now_v7();
    let token_main = CancellationToken::new();
    let shutdown_main = CancellationToken::new();
    let main_state = Arc::new(cluster::NodeState::default());
    let main_lock = node_lock(
        &db_pool,
        &token_main,
        &shutdown_main,
        node_id,
        "main-node",
        &main_state,
    );
    let main_task = tokio::spawn(async move { main_lock.run().await });
    wait_until_main(&main_state).await;

    // first incarnation of the standby
    let token_first = CancellationToken::new();
    let shutdown_first = CancellationToken::new();
    let first_state = Arc::new(cluster::NodeState::default());
    let first_lock = node_lock(
        &db_pool,
        &token_first,
        &shutdown_first,
        node_id,
        "standby-node",
        &first_state,
    );
    let first_task = tokio::spawn(async move { first_lock.run().await });
    wait_for_standby_entries(&db_pool, vec![(node_id, "standby-node".to_string())]).await;

    // it stops and comes straight back under the same identity, well inside
    // CLUSTER_HEARTBEAT_TIMEOUT — so its own row is still fresh in the table and
    // only the released advisory lock can tell the restart apart from a live twin
    token_first.cancel();
    first_task.await?;

    let token_second = CancellationToken::new();
    let shutdown_second = CancellationToken::new();
    let second_state = Arc::new(cluster::NodeState::default());
    let second_lock = node_lock(
        &db_pool,
        &token_second,
        &shutdown_second,
        node_id,
        "standby-node",
        &second_state,
    );
    let second_task = tokio::spawn(async move { second_lock.run().await });
    // a node leaves its initial LostConnection only once its first tick returns,
    // which is after the identity check and the standby heartbeat
    eventually(async || {
        ensure!(
            !second_state.is(cluster::NodeStateType::LostConnection),
            "waiting for the restarted standby's first tick"
        );
        Ok(())
    })
    .await;
    assert!(
        !shutdown_second.is_cancelled(),
        "Expected the restarted standby to reclaim its own identity, not shut down"
    );
    assert!(
        second_state.is(cluster::NodeStateType::Standby),
        "Expected the restarted standby to keep standing by, got {:?}",
        second_state.get_state()
    );
    assert_eq!(
        standby_entries(&db_pool).await?,
        vec![(node_id, "standby-node".to_string())],
        "Expected the restarted standby to be registered again"
    );

    token_main.cancel();
    token_second.cancel();
    main_task.await?;
    second_task.await?;
    Ok(())
}

#[tokio::test]
async fn test_stale_standby_reaped() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    let token = CancellationToken::new();
    let shutdown = CancellationToken::new();
    let state = Arc::new(cluster::NodeState::default());
    let leader_lock = node_lock(
        &db_pool,
        &token,
        &shutdown,
        Uuid::now_v7(),
        "leader-node",
        &state,
    );
    let leader_task = tokio::spawn(async move { leader_lock.run().await });
    eventually(async || {
        ensure!(
            state.is(cluster::NodeStateType::Leader),
            "expected the only node to become leader, got {:?}",
            state.get_state()
        );
        Ok(())
    })
    .await;

    // a standby that died without cleaning up after itself
    let stale_node_id = Uuid::now_v7();
    let conn = db_pool.get().await.context("failed to get connection")?;
    conn.execute(
        "INSERT INTO cluster_standby_nodes(node_id, name, last_heartbeat, metadata) \
        VALUES ($1, 'dead-node', now() - interval '1 hour', '{}'::jsonb)",
        &[&stale_node_id],
    )
    .await
    .context("failed to insert stale standby node")?;

    // the leader reaps the stale standby entry
    wait_for_standby_entries(&db_pool, vec![]).await;

    token.cancel();
    leader_task.await?;
    Ok(())
}

#[tokio::test]
async fn test_heartbeat_restores_inactive_status_without_clobbering_initializing() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    let node_id = Uuid::now_v7();
    let token = CancellationToken::new();
    let shutdown = CancellationToken::new();
    let state = Arc::new(cluster::NodeState::default());
    let lock = node_lock(&db_pool, &token, &shutdown, node_id, "leader-node", &state);
    let task = tokio::spawn(async move { lock.run().await });

    eventually(async || {
        ensure!(
            state.is(cluster::NodeStateType::Leader),
            "expected node to become leader, got {:?}",
            state.get_state()
        );
        Ok(())
    })
    .await;

    let conn = db_pool.get().await.context("failed to get connection")?;

    // Simulate the row having been marked Inactive (0) during a transient stall or
    // before this node took leadership. The next heartbeat must restore it to Active (1).
    conn.execute(
        "UPDATE cluster_nodes SET op_status = 0 WHERE node_id = $1",
        &[&node_id],
    )
    .await?;
    eventually(async || {
        let row = conn
            .query_one(
                "SELECT op_status, is_leader FROM cluster_nodes WHERE node_id = $1",
                &[&node_id],
            )
            .await?;
        let op_status: i16 = row.try_get(0)?;
        let is_leader: bool = row.try_get(1)?;
        ensure!(
            op_status == 1 && is_leader,
            "expected heartbeat to restore active leader, got op_status={op_status}, is_leader={is_leader}"
        );
        Ok(())
    })
    .await;

    // Heartbeats must NOT overwrite Initializing (2) while ConfigWatcher::initialize is running.
    let before: chrono::DateTime<chrono::Utc> = conn
        .query_one(
            "UPDATE cluster_nodes SET op_status = 2 WHERE node_id = $1 RETURNING last_heartbeat",
            &[&node_id],
        )
        .await?
        .try_get(0)?;
    eventually(async || {
        let row = conn
            .query_one(
                "SELECT op_status, last_heartbeat FROM cluster_nodes WHERE node_id = $1",
                &[&node_id],
            )
            .await?;
        let op_status: i16 = row.try_get(0)?;
        let after: chrono::DateTime<chrono::Utc> = row.try_get(1)?;
        ensure!(after > before, "waiting for another heartbeat tick");
        ensure!(
            op_status == 2,
            "heartbeat must preserve Initializing (2), got {op_status}"
        );
        Ok(())
    })
    .await;

    token.cancel();
    task.await?;
    Ok(())
}

/// When the leader dies while owning a sticky host, its row stays behind in
/// `cluster_nodes` (because of the FK). The surviving node must take over
/// leadership, clear `is_leader` on the dead row, mark it `op_status = 0`, and
/// fail over its non-sticky hosts. When the old leader comes back, its
/// `op_status` must return to `1` while staying a non-leader `Main`.
#[tokio::test]
async fn test_leader_failover_updates_db_flags_with_sticky_host() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    let timeout = setup::CLUSTER_HEARTBEAT_TIMEOUT;
    let node_a = RunningNode::start(&db_pool, Uuid::now_v7(), "node-a", timeout);
    let node_b = RunningNode::start(&db_pool, Uuid::now_v7(), "node-b", timeout);
    wait_until_main(&node_a.state).await;
    wait_until_main(&node_b.state).await;

    // Wait until one node is elected leader in both memory and DB.
    let mut nodes = vec![node_a, node_b];
    let leader_idx = eventually(async || {
        let pos = nodes
            .iter()
            .position(|n| n.state.is(cluster::NodeStateType::Leader))
            .context("waiting for a leader to be elected")?;
        Ok(pos)
    })
    .await;
    let old_leader = nodes.remove(leader_idx);
    let survivor = nodes.remove(0);
    let (old_id, old_name) = (old_leader.node_id, old_leader.name.clone());
    let survivor_id = survivor.node_id;

    let hosts = create_hosts(&db_pool, 3).await?;
    let conn = db_pool.get().await.context("failed to get connection")?;
    conn.execute(
        "UPDATE hosts SET monitored_by = $1, sticky_monitored_by = true WHERE host_id = $2",
        &[&old_id, &hosts[0]],
    )
    .await?;
    eventually(async || {
        let counts = host_counts(&db_pool).await?;
        ensure!(
            !counts.contains_key(&None),
            "waiting for all hosts to be assigned: {counts:?}"
        );
        Ok(())
    })
    .await;

    // Kill the leader; the survivor must take leadership and mark the old leader inactive.
    old_leader.stop().await?;
    eventually(async || {
        ensure!(
            survivor.state.is(cluster::NodeStateType::Leader),
            "waiting for survivor to become leader, got {:?}",
            survivor.state.get_state()
        );
        let rows = conn
            .query(
                "SELECT node_id, op_status, is_leader FROM cluster_nodes ORDER BY node_id",
                &[],
            )
            .await?;
        let mut by_id = HashMap::new();
        for row in rows {
            by_id.insert(
                row.try_get::<_, Uuid>(0)?,
                (row.try_get::<_, i16>(1)?, row.try_get::<_, bool>(2)?),
            );
        }
        ensure!(
            by_id.get(&survivor_id) == Some(&(1, true)),
            "expected survivor active leader, got {by_id:?}"
        );
        ensure!(
            by_id.get(&old_id) == Some(&(0, false)),
            "expected dead leader row retained as inactive non-leader, got {by_id:?}"
        );
        Ok(())
    })
    .await;

    // Non-sticky hosts moved to survivor; sticky host stayed with old_id.
    wait_for_host_counts(
        &db_pool,
        HashMap::from([(Some(old_id), 1), (Some(survivor_id), 2)]),
    )
    .await;

    // Old leader comes back: must restore op_status = 1 while staying non-leader.
    let returned = RunningNode::start(&db_pool, old_id, &old_name, timeout);
    wait_until_main(&returned.state).await;
    eventually(async || {
        let row = conn
            .query_one(
                "SELECT op_status, is_leader FROM cluster_nodes WHERE node_id = $1",
                &[&old_id],
            )
            .await?;
        let op_status: i16 = row.try_get(0)?;
        let is_leader: bool = row.try_get(1)?;
        ensure!(
            op_status == 1 && !is_leader,
            "expected returning node active non-leader, got op_status={op_status}, is_leader={is_leader}"
        );
        Ok(())
    })
    .await;
    let leader_count: i64 = conn
        .query_one(
            "SELECT count(*) FROM cluster_nodes WHERE is_leader = true",
            &[],
        )
        .await?
        .try_get(0)?;
    assert_eq!(leader_count, 1, "cluster must have exactly one leader");

    returned.stop().await?;
    survivor.stop().await?;
    Ok(())
}

/// An invalid edit to `configs.cluster` must keep the last good policy rather
/// than falling back to the default (`max_active_nodes = 0`, which would put
/// every spare to work).
#[tokio::test]
async fn test_invalid_cluster_policy_keeps_last_good_policy() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    let timeout = setup::CLUSTER_HEARTBEAT_TIMEOUT;
    set_cluster_policy(&db_pool, serde_json::json!({"max_active_nodes": 1})).await?;
    let node_1 = RunningNode::start(&db_pool, Uuid::now_v7(), "node-1", timeout);
    let node_2 = RunningNode::start(&db_pool, Uuid::now_v7(), "node-2", timeout);
    wait_until_main(&node_1.state).await;
    wait_until_main(&node_2.state).await;
    create_hosts(&db_pool, 4).await?;

    let active_owner = eventually(async || {
        let counts = host_counts(&db_pool).await?;
        match counts.iter().collect::<Vec<_>>()[..] {
            [(&Some(owner), &4)] => Ok(owner),
            _ => anyhow::bail!("waiting for 1 active node with 4 hosts: {counts:?}"),
        }
    })
    .await;

    // Write an unparseable policy; leader must keep `max_active_nodes = 1` and
    // continue assigning new hosts to the single active node.
    set_cluster_policy(&db_pool, serde_json::json!({"max_active_nodes": "bad"})).await?;
    create_hosts(&db_pool, 2).await?;
    wait_for_host_counts(&db_pool, HashMap::from([(Some(active_owner), 6)])).await;
    sleep(QUIET).await;
    assert_eq!(
        host_counts(&db_pool).await?,
        HashMap::from([(Some(active_owner), 6)]),
        "invalid policy must not reset max_active_nodes to 0 and activate the spare"
    );

    // Once repaired, the new policy takes effect.
    set_cluster_policy(&db_pool, serde_json::json!({"max_active_nodes": 2})).await?;
    wait_for_even_spread(&db_pool, 2, 3).await;

    node_1.stop().await?;
    node_2.stop().await?;
    Ok(())
}

/// Terminate the Postgres backend holding `AdvisoryLock::Leader` in this test
/// database: the leader's node-lock session, and only that.
async fn terminate_leader_backend(conn: &deadpool_postgres::Object) -> Result<()> {
    let terminated: i64 = conn
        .query_one(
            "SELECT count(*) FROM ( \
                SELECT pg_terminate_backend(pid) \
                FROM pg_locks \
                WHERE locktype = 'advisory' \
                  AND database = (SELECT oid FROM pg_database WHERE datname = current_database()) \
                  AND objsubid = 1 \
                  AND classid = 0 \
                  AND objid = $1 \
            ) t",
            &[&(patrolli::db::AdvisoryLock::Leader as u32)],
        )
        .await?
        .try_get(0)?;
    ensure!(terminated == 1, "expected to terminate one leader backend");
    Ok(())
}

/// Wait for `state` to lose the node lock and then hold it again.
async fn wait_for_lock_flap(state: &cluster::NodeState) {
    // the retry sleeps a second before it reconnects, so this is not missed
    eventually(async || {
        ensure!(!state.is_main(), "waiting for the node to lose its lock");
        Ok(())
    })
    .await;
    wait_until_main(state).await;
}

/// Killing the leader's Postgres backend connection mid-run drops its lock.
/// It reconnects well within `heartbeat_timeout` and nobody held its lock in
/// between, so it missed nothing: the tenure continues under the same
/// `main_epoch` (the watcher keeps what it loaded), without split-brain, with
/// both nodes active (`op_status = 1`) and all hosts balanced.
#[tokio::test]
async fn test_leader_recovers_after_postgres_backend_termination() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    // far beyond the second or two the reconnect takes
    let timeout = Duration::from_secs(30);
    let node_1 = RunningNode::start(&db_pool, Uuid::now_v7(), "node-1", timeout);
    let node_2 = RunningNode::start(&db_pool, Uuid::now_v7(), "node-2", timeout);
    wait_until_main(&node_1.state).await;
    wait_until_main(&node_2.state).await;
    create_hosts(&db_pool, 4).await?;
    wait_for_even_spread(&db_pool, 2, 2).await;

    let nodes = [&node_1, &node_2];
    let (killed_idx, old_epoch) = eventually(async || {
        let pos = nodes
            .iter()
            .position(|n| n.state.is(cluster::NodeStateType::Leader))
            .context("waiting for a leader")?;
        let epoch = nodes[pos]
            .state
            .main_epoch()
            .context("leader must have an active main epoch")?;
        Ok((pos, epoch))
    })
    .await;

    let conn = db_pool.get().await.context("failed to get connection")?;
    terminate_leader_backend(&conn).await?;
    wait_for_lock_flap(&nodes[killed_idx].state).await;
    assert_eq!(
        nodes[killed_idx].state.main_epoch(),
        Some(old_epoch),
        "a reconnect that missed nothing must continue its main epoch"
    );

    eventually(async || {
        let leader_count = nodes
            .iter()
            .filter(|n| n.state.is(cluster::NodeStateType::Leader))
            .count();
        ensure!(
            leader_count == 1,
            "expected exactly one in-memory leader after recovery, got {leader_count}"
        );
        let rows = conn
            .query(
                "SELECT node_id, op_status, is_leader FROM cluster_nodes",
                &[],
            )
            .await?;
        ensure!(rows.len() == 2, "expected 2 cluster_nodes rows");
        let mut db_leaders = 0;
        for row in rows {
            let op_status: i16 = row.try_get(1)?;
            let is_leader: bool = row.try_get(2)?;
            ensure!(op_status == 1, "both recovered nodes must be active (1)");
            if is_leader {
                db_leaders += 1;
            }
        }
        ensure!(
            db_leaders == 1,
            "expected exactly one DB leader, got {db_leaders}"
        );
        Ok(())
    })
    .await;

    // All 4 hosts remain evenly balanced across the two live nodes.
    wait_for_even_spread(&db_pool, 2, 2).await;

    node_1.stop().await?;
    node_2.stop().await?;
    Ok(())
}

/// A reconnect continues its main epoch only when nothing can have been
/// missed. A heartbeat on the row that this process did not write means another
/// process was main on the id in between, and read its cluster logs away.
#[tokio::test]
async fn test_reconnect_after_another_holder_starts_a_new_epoch() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    let node = RunningNode::start(&db_pool, Uuid::now_v7(), "node-1", Duration::from_secs(30));
    eventually(async || {
        ensure!(
            node.state.is(cluster::NodeStateType::Leader),
            "waiting for the node to lead"
        );
        Ok(())
    })
    .await;
    let old_epoch = node.state.main_epoch().context("leader has an epoch")?;

    let conn = db_pool.get().await.context("failed to get connection")?;
    terminate_leader_backend(&conn).await?;
    // The dead session writes no more heartbeats, and the retry is a second
    // away: this is what a standby's first heartbeat would have left.
    conn.execute(
        "UPDATE cluster_nodes SET last_heartbeat = last_heartbeat + interval '1 millisecond' \
         WHERE node_id = $1",
        &[&node.node_id],
    )
    .await?;
    wait_for_lock_flap(&node.state).await;
    assert_ne!(
        node.state.main_epoch(),
        Some(old_epoch),
        "a heartbeat from another holder must start a new main epoch"
    );

    node.stop().await?;
    Ok(())
}

/// The other half: a reconnect that comes after the node's heartbeat expired
/// starts a new epoch, since the leader may have purged its cluster logs. A
/// `heartbeat_timeout` of zero makes every reconnect that late.
#[tokio::test]
async fn test_reconnect_after_expiry_starts_a_new_epoch() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    let node = RunningNode::start(&db_pool, Uuid::now_v7(), "node-1", Duration::ZERO);
    eventually(async || {
        ensure!(
            node.state.is(cluster::NodeStateType::Leader),
            "waiting for the node to lead"
        );
        Ok(())
    })
    .await;
    let old_epoch = node.state.main_epoch().context("leader has an epoch")?;

    let conn = db_pool.get().await.context("failed to get connection")?;
    terminate_leader_backend(&conn).await?;
    wait_for_lock_flap(&node.state).await;
    assert_ne!(
        node.state.main_epoch(),
        Some(old_epoch),
        "a reconnect past the heartbeat timeout must start a new main epoch"
    );

    node.stop().await?;
    Ok(())
}

/// When a standby takes over a `node_id` whose previous main died and was
/// marked `op_status = 0` (retained because of a sticky host), the standby must
/// update `cluster_nodes.name` to its own name, restore `op_status = 1`, keep
/// the sticky host, and leave `cluster_standby_nodes` clean.
#[tokio::test]
async fn test_standby_takeover_updates_node_name_and_restores_active_status() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    let timeout = setup::CLUSTER_HEARTBEAT_TIMEOUT;
    let shared_id = Uuid::now_v7();
    let node_a = RunningNode::start(&db_pool, shared_id, "node-a-primary", timeout);
    let node_b = RunningNode::start(&db_pool, Uuid::now_v7(), "node-b", timeout);
    wait_until_main(&node_a.state).await;
    wait_until_main(&node_b.state).await;

    let hosts = create_hosts(&db_pool, 1).await?;
    let conn = db_pool.get().await.context("failed to get connection")?;
    conn.execute(
        "UPDATE hosts SET monitored_by = $1, sticky_monitored_by = true WHERE host_id = $2",
        &[&shared_id, &hosts[0]],
    )
    .await?;

    // Primary stops; peer leader marks its retained row Inactive (0).
    node_a.stop().await?;
    eventually(async || {
        let op_status: i16 = conn
            .query_one(
                "SELECT op_status FROM cluster_nodes WHERE node_id = $1",
                &[&shared_id],
            )
            .await?
            .try_get(0)?;
        ensure!(op_status == 0, "waiting for primary row to become inactive");
        Ok(())
    })
    .await;

    // Standby with the same node_id takes over.
    let standby = RunningNode::start(&db_pool, shared_id, "node-a-standby", timeout);
    wait_until_main(&standby.state).await;
    eventually(async || {
        let row = conn
            .query_one(
                "SELECT name, op_status FROM cluster_nodes WHERE node_id = $1",
                &[&shared_id],
            )
            .await?;
        let name: String = row.try_get(0)?;
        let op_status: i16 = row.try_get(1)?;
        ensure!(
            name == "node-a-standby" && op_status == 1,
            "expected takeover to update name and restore op_status=1, got name={name:?}, op_status={op_status}"
        );
        Ok(())
    })
    .await;
    assert!(
        standby_entries(&db_pool).await?.is_empty(),
        "promoted standby must not leave an entry in cluster_standby_nodes"
    );
    let owner: Option<Uuid> = conn
        .query_one(
            "SELECT monitored_by FROM hosts WHERE host_id = $1",
            &[&hosts[0]],
        )
        .await?
        .try_get(0)?;
    assert_eq!(owner, Some(shared_id));

    standby.stop().await?;
    node_b.stop().await?;
    Ok(())
}

/// The leader's stale check must never apply to the leader itself. Its own
/// heartbeat is written just before its leader tasks, in another transaction:
/// held up between the two, it would otherwise find its own row stale, take
/// itself for dead (no live node, so no host is placed), mark itself inactive,
/// purge its own cluster logs and delete its own row. A `heartbeat_timeout` of
/// zero stands in for that delay: every heartbeat is stale by the time anyone
/// looks at it.
#[tokio::test]
async fn test_leader_never_expires_itself() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    let node_id = Uuid::now_v7();
    let token = CancellationToken::new();
    let shutdown = CancellationToken::new();
    let state = Arc::new(cluster::NodeState::default());
    let lock = node_lock_with_timeout(
        &db_pool,
        &token,
        &shutdown,
        node_id,
        "leader-node",
        &state,
        Duration::ZERO,
    );
    let task = tokio::spawn(async move { lock.run().await });
    eventually(async || {
        ensure!(
            state.is(cluster::NodeStateType::Leader),
            "expected node to become leader, got {:?}",
            state.get_state()
        );
        Ok(())
    })
    .await;
    let epoch = state.main_epoch();

    // With no host to keep the row, an expired node is deleted, and a stale one
    // is inactive from the leader tasks until its next heartbeat: either would
    // show within a tick.
    let conn = db_pool.get().await.context("failed to get connection")?;
    let quiet_until = tokio::time::Instant::now() + QUIET;
    while tokio::time::Instant::now() < quiet_until {
        let op_status: Option<i16> = conn
            .query_opt(
                "SELECT op_status FROM cluster_nodes WHERE node_id = $1",
                &[&node_id],
            )
            .await?
            .map(|row| row.try_get(0))
            .transpose()?;
        assert_eq!(
            op_status,
            Some(1),
            "the leader must keep its row, and keep it active"
        );
        sleep(Duration::from_millis(50)).await;
    }

    // It counts itself live, so it places hosts on itself...
    create_hosts(&db_pool, 3).await?;
    wait_for_host_counts(&db_pool, HashMap::from([(Some(node_id), 3)])).await;
    // ...and the logs of that placement outlive the tick that wrote them.
    let logs: i64 = conn
        .query_one(
            "SELECT count(*) FROM cluster_logs WHERE node_id = $1",
            &[&node_id],
        )
        .await?
        .try_get(0)?;
    assert!(logs > 0, "the leader must not purge its own cluster logs");
    assert_eq!(
        state.main_epoch(),
        epoch,
        "a leader is expired by nobody, so it has nothing to reload"
    );

    token.cancel();
    task.await?;
    Ok(())
}

/// A leader with nothing on it and a second node owning two hosts, under
/// `max_active_nodes: 1`: the leader is the spare a failover would move them to,
/// and since nothing ever moves back, hosts still on the owner afterwards mean
/// the leader never took it for failed.
async fn start_spare_leader_and_owner(
    pool: &Pool,
    heartbeat_timeout: Duration,
    heartbeat_stale_after: Duration,
) -> Result<(RunningNode, RunningNode)> {
    set_cluster_policy(pool, serde_json::json!({"max_active_nodes": 1})).await?;
    let start = |name: &str| {
        RunningNode::start_with(
            pool,
            Uuid::now_v7(),
            name,
            heartbeat_timeout,
            heartbeat_stale_after,
        )
    };
    let leader = start("leader");
    eventually(async || {
        ensure!(
            leader.state.is(cluster::NodeStateType::Leader),
            "waiting for the first node to lead, got {:?}",
            leader.state.get_state()
        );
        Ok(())
    })
    .await;
    let owner = start("owner");
    wait_until_main(&owner.state).await;

    // assigned as they are created, so the leader never sees them unassigned
    let mut conn = pool.get().await.context("failed to get connection")?;
    let tx = conn.transaction().await?;
    for i in 0..2 {
        let host_id = setup::create_host(&tx, &format!("host-{i}")).await?;
        tx.execute(
            "UPDATE hosts SET monitored_by = $1 WHERE host_id = $2",
            &[&owner.node_id, &host_id],
        )
        .await?;
    }
    tx.commit().await?;
    Ok((leader, owner))
}

/// A database stall holds every node's heartbeat up alike, the leader's too.
/// The heartbeats it releases carry the time they were started at, so against
/// the clock every peer looks stale the moment the leader gets to look, and its
/// hosts would fail over although it never stopped. The leader judges them
/// against its own previous heartbeat instead, which the stall aged as much.
#[tokio::test]
async fn a_stall_that_hits_every_node_marks_nobody_stale() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    let stale_after = Duration::from_secs(3);
    let (leader, owner) =
        start_spare_leader_and_owner(&db_pool, Duration::from_secs(30), stale_after).await?;
    let conn = db_pool.get().await.context("failed to get connection")?;

    // The stall: every heartbeat waits for a lock this session holds, in a
    // BEFORE INSERT trigger (it runs ahead of the ON CONFLICT check, before any
    // row lock), and all of them are let go at the same instant.
    let gate = db_pool.get().await.context("failed to get connection")?;
    gate.execute("SELECT pg_advisory_lock(7001, 1)", &[])
        .await?;
    conn.batch_execute(
        "CREATE FUNCTION stall_heartbeat() RETURNS trigger AS $$ \
         BEGIN \
           PERFORM pg_advisory_xact_lock_shared(7001, 1); \
           RETURN NEW; \
         END $$ LANGUAGE plpgsql; \
         CREATE TRIGGER trg_stall_heartbeat BEFORE INSERT ON cluster_nodes \
         FOR EACH ROW EXECUTE FUNCTION stall_heartbeat();",
    )
    .await?;
    // held for twice the stale threshold
    eventually(async || {
        let stalled: i64 = conn
            .query_one(
                "SELECT count(*) FROM cluster_nodes \
                 WHERE last_heartbeat + make_interval(secs := $1) < clock_timestamp()",
                &[&(stale_after.as_secs_f64() * 2.0)],
            )
            .await?
            .try_get(0)?;
        ensure!(stalled == 2, "{stalled} of 2 heartbeats are stalled so far");
        Ok(())
    })
    .await;
    let released: chrono::DateTime<chrono::Utc> = gate
        .query_one(
            "SELECT clock_timestamp() FROM pg_advisory_unlock(7001, 1)",
            &[],
        )
        .await?
        .try_get(0)?;

    // The leader gets to judge on the held-up tick, on the one that follows it
    // at once, and on every tick after: wait until a few of those are behind it
    // and the owner heartbeats again.
    eventually(async || {
        let row = conn
            .query_one(
                "SELECT \
                    (SELECT last_heartbeat FROM cluster_nodes WHERE node_id = $1) \
                        > $3::timestamptz + interval '2 seconds', \
                    (SELECT last_heartbeat FROM cluster_nodes WHERE node_id = $2) \
                        > $3::timestamptz",
                &[&leader.node_id, &owner.node_id, &released],
            )
            .await?;
        let leader_ticked: Option<bool> = row.try_get(0)?;
        let owner_beat: Option<bool> = row.try_get(1)?;
        ensure!(
            leader_ticked == Some(true) && owner_beat == Some(true),
            "waiting for heartbeats after the stall: leader {leader_ticked:?}, owner {owner_beat:?}"
        );
        Ok(())
    })
    .await;

    assert_eq!(
        host_counts(&db_pool).await?,
        HashMap::from([(Some(owner.node_id), 2)]),
        "a node held up no longer than the leader keeps its hosts"
    );

    leader.stop().await?;
    owner.stop().await?;
    Ok(())
}

/// `heartbeat_stale_after` is what decides a failover, not a fixed margin over
/// the interval: a node silent for less keeps its hosts.
#[tokio::test]
async fn a_silent_node_keeps_its_hosts_until_stale_after() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    let long = Duration::from_secs(30);
    let (leader, owner) = start_spare_leader_and_owner(&db_pool, long, long).await?;
    let owner_id = owner.node_id;
    owner.stop().await?;

    // silent for five intervals, with the leader ticking all along
    let conn = db_pool.get().await.context("failed to get connection")?;
    eventually(async || {
        let silent: bool = conn
            .query_one(
                "SELECT last_heartbeat + make_interval(secs := $2) < clock_timestamp() \
                 FROM cluster_nodes WHERE node_id = $1",
                &[
                    &owner_id,
                    &(setup::CLUSTER_HEARTBEAT_INTERVAL.as_secs_f64() * 5.0),
                ],
            )
            .await
            .context("the silent node lost its row")?
            .try_get(0)?;
        ensure!(silent, "waiting for the stopped node to have been silent");
        Ok(())
    })
    .await;
    assert_eq!(
        host_counts(&db_pool).await?,
        HashMap::from([(Some(owner_id), 2)]),
        "a node silent for less than heartbeat_stale_after keeps its hosts"
    );

    leader.stop().await?;
    Ok(())
}

/// A leader whose process restarts while every heartbeat is slow has no
/// heartbeat of its own from before: its first one starts later than the
/// heartbeat a peer has under way by however long that one has been waiting.
/// Against that first heartbeat, a peer whose heartbeats take longer than
/// `heartbeat_stale_after` loses its hosts although it never stopped. It judges
/// against the heartbeat its predecessor left on the row instead, which is as
/// old as the peer's.
#[tokio::test]
async fn a_leader_that_restarts_while_heartbeats_are_slow_marks_nobody_stale() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    let stale_after = Duration::from_secs(2);
    let timeout = Duration::from_secs(60);
    // how long every heartbeat takes, as on a disk whose commits wait this long
    let slow = 5.0_f64;
    let (leader, owner) = start_spare_leader_and_owner(&db_pool, timeout, stale_after).await?;
    let conn = db_pool.get().await.context("failed to get connection")?;
    conn.batch_execute(&format!(
        "CREATE FUNCTION slow_heartbeat() RETURNS trigger AS $$ \
         BEGIN \
           PERFORM pg_sleep({slow}); \
           RETURN NEW; \
         END $$ LANGUAGE plpgsql; \
         CREATE TRIGGER trg_slow_heartbeat BEFORE INSERT ON cluster_nodes \
         FOR EACH ROW EXECUTE FUNCTION slow_heartbeat();"
    ))
    .await?;

    // A slow heartbeat becomes visible `slow` after it started, which is when the
    // owner starts its next one. Wait for that next one to have been under way
    // for longer than the stale threshold, with time left before it is through.
    let under_way = stale_after.as_secs_f64() + 1.0;
    eventually(async || {
        let row = conn
            .query_one(
                "SELECT clock_timestamp() - last_heartbeat > make_interval(secs := $2), \
                        clock_timestamp() - last_heartbeat < make_interval(secs := $3) \
                 FROM cluster_nodes WHERE node_id = $1",
                &[
                    &owner.node_id,
                    &(slow + under_way),
                    &(slow + under_way + 1.0),
                ],
            )
            .await?;
        let (waited, in_time): (bool, bool) = (row.try_get(0)?, row.try_get(1)?);
        ensure!(
            waited && in_time,
            "waiting for a slow owner heartbeat to be under way"
        );
        Ok(())
    })
    .await;

    // the leader's process goes away and a new one takes its place, before the
    // owner's next tick could take the lead
    leader.token.cancel();
    terminate_leader_backend(&conn).await?;
    leader.task.await?;
    let started: chrono::DateTime<chrono::Utc> = conn
        .query_one("SELECT clock_timestamp()", &[])
        .await?
        .try_get(0)?;
    let restarted =
        RunningNode::start_with(&db_pool, leader.node_id, "leader", timeout, stale_after);

    // three of its heartbeats are in, so it has judged on two whole ticks
    eventually(async || {
        let ticked: bool = conn
            .query_one(
                "SELECT last_heartbeat > $2::timestamptz + make_interval(secs := $3) \
                 FROM cluster_nodes WHERE node_id = $1",
                &[&restarted.node_id, &started, &(slow * 2.0 - 1.0)],
            )
            .await?
            .try_get(0)?;
        ensure!(
            restarted.state.is(cluster::NodeStateType::Leader) && ticked,
            "waiting for the restarted leader's ticks, got {:?}",
            restarted.state.get_state()
        );
        Ok(())
    })
    .await;

    assert_eq!(
        host_counts(&db_pool).await?,
        HashMap::from([(Some(owner.node_id), 2)]),
        "a node as slow as the leader that just restarted keeps its hosts"
    );

    restarted.stop().await?;
    owner.stop().await?;
    Ok(())
}
