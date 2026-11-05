// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

mod setup;

use anyhow::{Context, Result, ensure};
use deadpool_postgres::Pool;
use patrolli::{cluster, config};
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
    cluster::node_lock::NodeLock::new(
        db_pool.clone(),
        token.clone(),
        app_shutdown.clone(),
        config::cluster::Cluster {
            node_id,
            node_name: node_name.to_string(),
            heartbeat_timeout,
            heartbeat_interval: setup::CLUSTER_HEARTBEAT_INTERVAL,
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
        let token = CancellationToken::new();
        let state = Arc::new(cluster::NodeState::default());
        let lock = node_lock_with_timeout(
            pool,
            &token,
            &CancellationToken::new(),
            node_id,
            name,
            &state,
            heartbeat_timeout,
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
