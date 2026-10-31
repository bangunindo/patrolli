// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

mod setup;

use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use parking_lot::Mutex;
use patrolli::cluster::inter;
use patrolli::config::MonitorConfig;
use patrolli::config::listener::Listener;
use patrolli::config::sink::Sink;
use patrolli::{cache, cluster, listener, monitor, sink};
use setup::eventually;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
enum ForwarderCall {
    Add(Uuid, String),
    Remove(Uuid),
    CloseAll,
}

#[derive(Default)]
struct DummyWorkerManager {
    forwarder_calls: Mutex<Vec<ForwarderCall>>,
    /// Every monitor config that changed what was applied, in order.
    monitor_configs: Mutex<Vec<MonitorConfig>>,
}

#[async_trait]
impl sink::WorkerManager for DummyWorkerManager {
    async fn close_all_sink(&self) {}

    async fn add(&self, _conf: Sink) -> Result<()> {
        Ok(())
    }

    async fn close_sink(&self, _id: Uuid) -> Result<()> {
        Ok(())
    }

    async fn remove_orphaned_data(&self) {}
}

#[async_trait]
impl listener::WorkerManager for DummyWorkerManager {
    async fn close_all_listener(&self) {}

    async fn add(&self, _conf: Listener) -> Result<()> {
        Ok(())
    }

    async fn close_listener(&self, _id: Uuid) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl monitor::WorkerManager for DummyWorkerManager {
    async fn apply_config(&self, config: MonitorConfig) -> bool {
        let mut applied = self.monitor_configs.lock();
        if applied.last() == Some(&config) {
            return false;
        }
        applied.push(config);
        true
    }
}

#[async_trait]
impl inter::InterCommManager for DummyWorkerManager {
    async fn close_all(&self) {
        self.forwarder_calls.lock().push(ForwarderCall::CloseAll);
    }

    async fn add(&self, node_id: Uuid, dst: String) -> Result<()> {
        self.forwarder_calls
            .lock()
            .push(ForwarderCall::Add(node_id, dst));
        Ok(())
    }

    async fn remove(&self, node_id: Uuid) {
        self.forwarder_calls
            .lock()
            .push(ForwarderCall::Remove(node_id));
    }
}

/// Wait until the watcher reporting to `workers` has finished its initial read.
/// `read_monitor_config` is the last step of `initialize` and the first config
/// is always applied, so from then on the read's snapshot is taken: anything
/// committed later reaches the node through `delta_read`. Give each node its own
/// manager for this — a shared one records the same config only once.
async fn wait_initialized(workers: &DummyWorkerManager) {
    eventually(async || {
        ensure!(
            !workers.monitor_configs.lock().is_empty(),
            "waiting for the watcher to finish initializing"
        );
        Ok(())
    })
    .await
}

/// Hosts, endpoints, items and processors in one node's cache.
fn cache_sizes(cache: &cache::Cache) -> [usize; 4] {
    [
        cache.host.read().len(),
        cache.endpoint.read().len(),
        cache.item.read().len(),
        cache.processor.len(),
    ]
}

/// Wait until node_1 holds hosts 1-3 and node_2 host 4, each with its endpoint
/// and item, and both have the processor.
async fn wait_for_host_split(node_1: &cache::Cache, node_2: &cache::Cache) {
    eventually(async || {
        let (one, two) = (cache_sizes(node_1), cache_sizes(node_2));
        ensure!(
            one == [3, 3, 3, 1] && two == [1, 1, 1, 1],
            "[hosts, endpoints, items, processors]: node_1 has {one:?}, node_2 {two:?}"
        );
        Ok(())
    })
    .await
}

/// The destinations `workers` was told to add for `node_id`, in order.
fn forwarder_adds(workers: &DummyWorkerManager, node_id: Uuid) -> Vec<String> {
    workers
        .forwarder_calls
        .lock()
        .iter()
        .filter_map(|c| match c {
            ForwarderCall::Add(id, dst) if *id == node_id => Some(dst.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn test_config_initialization() -> Result<()> {
    setup::setup_logging();
    setup::setup_mib().await;
    let (_db, db_pool) = setup::setup_db().await;
    let worker_manager = Arc::new(DummyWorkerManager::default());
    let app_cache_1 = cache::Cache::new();
    let app_cache_2 = cache::Cache::new();
    let token_1 = CancellationToken::new();
    let token_2 = CancellationToken::new();
    let node_1_id = Uuid::now_v7();
    let node_2_id = Uuid::now_v7();
    let node_1_state = Arc::new(cluster::NodeState::default());
    node_1_state.set_state(cluster::NodeStateType::Leader);
    let node_1_watcher = cluster::config_watcher::ConfigWatcher {
        initialized: false,
        sinks: worker_manager.clone(),
        listeners: worker_manager.clone(),
        monitors: worker_manager.clone(),
        inter_comm: worker_manager.clone(),
        node_id: node_1_id,
        node_name: "node_1".to_string(),
        cache: app_cache_1.clone(),
        db_pool: db_pool.clone(),
        secret_keys: setup::secret_keys(),
        node_lock: node_1_state.clone(),
        token: token_1.clone(),
        poll_interval: Duration::from_secs(1),
    };
    let node_2_state = Arc::new(cluster::NodeState::default());
    node_2_state.set_state(cluster::NodeStateType::Main);
    let node_2_watcher = cluster::config_watcher::ConfigWatcher {
        initialized: false,
        sinks: worker_manager.clone(),
        listeners: worker_manager.clone(),
        monitors: worker_manager.clone(),
        inter_comm: worker_manager.clone(),
        node_id: node_2_id,
        node_name: "node_2".to_string(),
        cache: app_cache_2.clone(),
        db_pool: db_pool.clone(),
        secret_keys: setup::secret_keys(),
        node_lock: node_2_state.clone(),
        token: token_2.clone(),
        poll_interval: Duration::from_secs(1),
    };
    let mut conn = db_pool.get().await?;
    let tx = conn.transaction().await?;
    tx.execute(
        "INSERT INTO cluster_nodes(node_id, name, op_status, last_heartbeat, metadata) \
                VALUES ($1, $2, 1, now(), '{\"advertise_address\": \"127.0.0.1:0\"}') \
                ON CONFLICT (node_id) DO UPDATE SET \
                name = EXCLUDED.name, \
                last_heartbeat = EXCLUDED.last_heartbeat, \
                metadata = EXCLUDED.metadata",
        &[&node_1_id, &"node_1".to_string()],
    )
    .await
    .context("failed to insert node_1")?;
    tx.execute(
        "INSERT INTO cluster_nodes(node_id, name, op_status, last_heartbeat, metadata) \
                VALUES ($1, $2, 1, now(), '{\"advertise_address\": \"127.0.0.1:0\"}') \
                ON CONFLICT (node_id) DO UPDATE SET \
                name = EXCLUDED.name, \
                last_heartbeat = EXCLUDED.last_heartbeat, \
                metadata = EXCLUDED.metadata",
        &[&node_2_id, &"node_2".to_string()],
    )
    .await
    .context("failed to insert node_2")?;
    let processor_id = setup::create_processor(&tx, "processor_1").await?;
    setup::create_data(
        &tx,
        "host_1",
        "endpoint_1",
        "item_1",
        Some(processor_id),
        Some(node_1_id),
    )
    .await?;
    setup::create_data(
        &tx,
        "host_2",
        "endpoint_2",
        "item_2",
        Some(processor_id),
        Some(node_1_id),
    )
    .await?;
    setup::create_data(
        &tx,
        "host_3",
        "endpoint_3",
        "item_3",
        Some(processor_id),
        Some(node_1_id),
    )
    .await?;
    setup::create_data(
        &tx,
        "host_4",
        "endpoint_4",
        "item_4",
        Some(processor_id),
        Some(node_2_id),
    )
    .await?;
    tx.commit().await.context("failed to commit transaction")?;
    drop(conn);
    let node_1_task = tokio::spawn(async move {
        node_1_watcher.run().await;
    });
    let node_2_task = tokio::spawn(async move {
        node_2_watcher.run().await;
    });
    wait_for_host_split(&app_cache_1, &app_cache_2).await;
    token_1.cancel();
    token_2.cancel();
    node_1_task.await?;
    node_2_task.await?;
    Ok(())
}

#[tokio::test]
async fn test_config_incremental_update() -> Result<()> {
    setup::setup_logging();
    setup::setup_mib().await;
    let (_db, db_pool) = setup::setup_db().await;
    let worker_manager_1 = Arc::new(DummyWorkerManager::default());
    let worker_manager_2 = Arc::new(DummyWorkerManager::default());
    let app_cache_1 = cache::Cache::new();
    let app_cache_2 = cache::Cache::new();
    let token_1 = CancellationToken::new();
    let token_2 = CancellationToken::new();
    let node_1_id = Uuid::now_v7();
    let node_2_id = Uuid::now_v7();
    let node_1_state = Arc::new(cluster::NodeState::default());
    node_1_state.set_state(cluster::NodeStateType::Leader);
    let node_1_watcher = cluster::config_watcher::ConfigWatcher {
        initialized: false,
        sinks: worker_manager_1.clone(),
        listeners: worker_manager_1.clone(),
        monitors: worker_manager_1.clone(),
        inter_comm: worker_manager_1.clone(),
        node_id: node_1_id,
        node_name: "node_1".to_string(),
        cache: app_cache_1.clone(),
        db_pool: db_pool.clone(),
        secret_keys: setup::secret_keys(),
        node_lock: node_1_state.clone(),
        token: token_1.clone(),
        poll_interval: Duration::from_secs(1),
    };
    let node_2_state = Arc::new(cluster::NodeState::default());
    node_2_state.set_state(cluster::NodeStateType::Main);
    let node_2_watcher = cluster::config_watcher::ConfigWatcher {
        initialized: false,
        sinks: worker_manager_2.clone(),
        listeners: worker_manager_2.clone(),
        monitors: worker_manager_2.clone(),
        inter_comm: worker_manager_2.clone(),
        node_id: node_2_id,
        node_name: "node_2".to_string(),
        cache: app_cache_2.clone(),
        db_pool: db_pool.clone(),
        secret_keys: setup::secret_keys(),
        node_lock: node_2_state.clone(),
        token: token_2.clone(),
        poll_interval: Duration::from_secs(1),
    };
    let node_1_task = tokio::spawn(async move {
        node_1_watcher.run().await;
    });
    let node_2_task = tokio::spawn(async move {
        node_2_watcher.run().await;
    });
    // both start from an empty database, so everything below arrives as deltas
    wait_initialized(&worker_manager_1).await;
    wait_initialized(&worker_manager_2).await;
    let mut conn = db_pool.get().await?;
    let tx = conn.transaction().await?;
    tx.execute(
        "INSERT INTO cluster_nodes(node_id, name, op_status, last_heartbeat, metadata) \
                VALUES ($1, $2, 1, now(), '{\"advertise_address\": \"127.0.0.1:0\"}') \
                ON CONFLICT (node_id) DO UPDATE SET \
                name = EXCLUDED.name, \
                last_heartbeat = EXCLUDED.last_heartbeat, \
                metadata = EXCLUDED.metadata",
        &[&node_1_id, &"node_1".to_string()],
    )
    .await
    .context("failed to insert node_1")?;
    tx.execute(
        "INSERT INTO cluster_nodes(node_id, name, op_status, last_heartbeat, metadata) \
                VALUES ($1, $2, 1, now(), '{\"advertise_address\": \"127.0.0.1:0\"}') \
                ON CONFLICT (node_id) DO UPDATE SET \
                name = EXCLUDED.name, \
                last_heartbeat = EXCLUDED.last_heartbeat, \
                metadata = EXCLUDED.metadata",
        &[&node_2_id, &"node_2".to_string()],
    )
    .await
    .context("failed to insert node_2")?;
    let processor_id = setup::create_processor(&tx, "processor_1").await?;
    setup::create_data(
        &tx,
        "host_1",
        "endpoint_1",
        "item_1",
        Some(processor_id),
        Some(node_1_id),
    )
    .await?;
    setup::create_data(
        &tx,
        "host_2",
        "endpoint_2",
        "item_2",
        Some(processor_id),
        Some(node_1_id),
    )
    .await?;
    setup::create_data(
        &tx,
        "host_3",
        "endpoint_3",
        "item_3",
        Some(processor_id),
        Some(node_1_id),
    )
    .await?;
    setup::create_data(
        &tx,
        "host_4",
        "endpoint_4",
        "item_4",
        Some(processor_id),
        Some(node_2_id),
    )
    .await?;
    tx.commit().await.context("failed to commit transaction")?;
    drop(conn);
    wait_for_host_split(&app_cache_1, &app_cache_2).await;
    token_1.cancel();
    token_2.cancel();
    node_1_task.await?;
    node_2_task.await?;
    Ok(())
}

#[tokio::test]
async fn test_listener_item_cache_propagation() -> Result<()> {
    setup::setup_logging();
    setup::setup_mib().await;
    let (_db, db_pool) = setup::setup_db().await;
    let worker_manager = Arc::new(DummyWorkerManager::default());
    let app_cache_1 = cache::Cache::new();
    let app_cache_2 = cache::Cache::new();
    let token_1 = CancellationToken::new();
    let token_2 = CancellationToken::new();
    let node_1_id = Uuid::now_v7();
    let node_2_id = Uuid::now_v7();
    let node_1_state = Arc::new(cluster::NodeState::default());
    node_1_state.set_state(cluster::NodeStateType::Leader);
    let node_1_watcher = cluster::config_watcher::ConfigWatcher {
        initialized: false,
        sinks: worker_manager.clone(),
        listeners: worker_manager.clone(),
        monitors: worker_manager.clone(),
        inter_comm: worker_manager.clone(),
        node_id: node_1_id,
        node_name: "node_1".to_string(),
        cache: app_cache_1.clone(),
        db_pool: db_pool.clone(),
        secret_keys: setup::secret_keys(),
        node_lock: node_1_state.clone(),
        token: token_1.clone(),
        poll_interval: Duration::from_secs(1),
    };
    let node_2_state = Arc::new(cluster::NodeState::default());
    node_2_state.set_state(cluster::NodeStateType::Main);
    let node_2_watcher = cluster::config_watcher::ConfigWatcher {
        initialized: false,
        sinks: worker_manager.clone(),
        listeners: worker_manager.clone(),
        monitors: worker_manager.clone(),
        inter_comm: worker_manager.clone(),
        node_id: node_2_id,
        node_name: "node_2".to_string(),
        cache: app_cache_2.clone(),
        db_pool: db_pool.clone(),
        secret_keys: setup::secret_keys(),
        node_lock: node_2_state.clone(),
        token: token_2.clone(),
        poll_interval: Duration::from_secs(1),
    };
    let mut conn = db_pool.get().await?;
    let tx = conn.transaction().await?;
    tx.execute(
        "INSERT INTO cluster_nodes(node_id, name, op_status, last_heartbeat, metadata) \
         VALUES ($1, $2, 1, now(), '{\"advertise_address\": \"127.0.0.1:0\"}')",
        &[&node_1_id, &"node_1".to_string()],
    )
    .await?;
    tx.execute(
        "INSERT INTO cluster_nodes(node_id, name, op_status, last_heartbeat, metadata) \
         VALUES ($1, $2, 1, now(), '{\"advertise_address\": \"127.0.0.1:0\"}')",
        &[&node_2_id, &"node_2".to_string()],
    )
    .await?;

    let processor_id = setup::create_processor(&tx, "processor_listener").await?;
    let listener_id = Uuid::now_v7();
    // the default syslog listener, UDP 0.0.0.0:514
    let listener_cfg = serde_json::json!({});
    tx.execute(
        "INSERT INTO listeners (listener_id, name, type, adm_status, last_updated, config) \
         VALUES ($1, $2, $3, 1, now(), $4)",
        &[&listener_id, &"syslog_listener", &"syslog", &listener_cfg],
    )
    .await
    .context("failed to insert listener")?;

    let host_id = Uuid::now_v7();
    tx.execute(
        "INSERT INTO hosts (host_id, name, adm_status, last_updated, monitored_by) \
         VALUES ($1, $2, 1, now(), $3)",
        &[&host_id, &"listener_host", &node_1_id],
    )
    .await?;

    let endpoint_id = Uuid::now_v7();
    let endpoint_cfg = serde_json::json!({ "address": "10.99.0.7:161", "community": "public" });
    tx.execute(
        "INSERT INTO host_endpoints (endpoint_id, host_id, name, type, adm_status, last_updated, config) \
         VALUES ($1, $2, $3, $4, 1, now(), $5)",
        &[
            &endpoint_id,
            &host_id,
            &"listener_endpoint",
            &"snmp_v2c",
            &endpoint_cfg,
        ],
    )
    .await?;

    let host_template_id = Uuid::now_v7();
    tx.execute(
        "INSERT INTO host_templates (host_template_id, host_id) VALUES ($1, $2)",
        &[&host_template_id, &host_id],
    )
    .await?;

    let item_id = Uuid::now_v7();
    let item_cfg = serde_json::json!({ "match_strategy": { "method": "endpoint_source_ip" } });
    tx.execute(
        "INSERT INTO items (item_id, host_template_id, endpoint_id, listener_id, name, type, adm_status, last_updated, config, schedule, timeout_ms) \
         VALUES ($1, $2, $3, $4, $5, $6, 1, now(), $7, $8, $9)",
        &[
            &item_id,
            &host_template_id,
            &endpoint_id,
            &listener_id,
            &"listener_item",
            &"listener",
            &item_cfg,
            &"* * * * * * *",
            &10000i32,
        ],
    )
    .await?;
    tx.execute(
        "INSERT INTO item_processors (item_id, processor_id) VALUES ($1, $2)",
        &[&item_id, &processor_id],
    )
    .await?;
    tx.commit().await?;
    drop(conn);

    let node_1_task = tokio::spawn(async move { node_1_watcher.run().await });
    let node_2_task = tokio::spawn(async move { node_2_watcher.run().await });

    let caches = [("node_1", &app_cache_1), ("node_2", &app_cache_2)];
    eventually(async || {
        for (label, app_cache) in caches {
            let guard = app_cache.listener.read();
            let items: Vec<_> = guard
                .get_items(&listener_id)
                .map(|it| it.cloned().collect())
                .unwrap_or_default();
            ensure!(items.len() == 1, "{label} should have 1 listener item");
            ensure!(
                items[0].monitored_by == Some(node_1_id),
                "{label} monitored_by is {:?}",
                items[0].monitored_by
            );
            let ip = guard.get_endpoint_ip(&endpoint_id).copied();
            ensure!(
                ip == Some("10.99.0.7".parse()?),
                "{label} endpoint ip is {ip:?}"
            );
        }
        Ok(())
    })
    .await;

    let mut conn = db_pool.get().await?;
    let tx = conn.transaction().await?;
    tx.execute(
        "UPDATE items SET adm_status = 0, last_updated = now() WHERE item_id = $1",
        &[&item_id],
    )
    .await?;
    tx.commit().await?;
    drop(conn);
    eventually(async || {
        for (label, app_cache) in caches {
            let guard = app_cache.listener.read();
            let items_len = guard
                .get_items(&listener_id)
                .map(|it| it.len())
                .unwrap_or(0);
            ensure!(
                items_len == 0,
                "{label} should evict listener item after disable"
            );
            ensure!(
                guard.get_endpoint_ip(&endpoint_id).is_none(),
                "{label} endpoint ip should be evicted"
            );
        }
        Ok(())
    })
    .await;

    token_1.cancel();
    token_2.cancel();
    node_1_task.await?;
    node_2_task.await?;
    Ok(())
}

#[tokio::test]
async fn test_cluster_node_forwarder() -> Result<()> {
    setup::setup_logging();
    setup::setup_mib().await;
    let (_db, db_pool) = setup::setup_db().await;
    let worker_manager_1 = Arc::new(DummyWorkerManager::default());
    let worker_manager_2 = Arc::new(DummyWorkerManager::default());
    let app_cache_1 = cache::Cache::new();
    let app_cache_2 = cache::Cache::new();
    let token_1 = CancellationToken::new();
    let token_2 = CancellationToken::new();
    let node_1_id = Uuid::now_v7();
    let node_2_id = Uuid::now_v7();

    let mut conn = db_pool.get().await?;
    let tx = conn.transaction().await?;
    tx.execute(
        "INSERT INTO cluster_nodes(node_id, name, op_status, last_heartbeat, metadata) \
         VALUES ($1, $2, 1, now(), $3::jsonb)",
        &[
            &node_1_id,
            &"node_1".to_string(),
            &serde_json::json!({ "advertise_address": "127.0.0.1:9001" }),
        ],
    )
    .await
    .context("failed to insert node_1")?;
    tx.execute(
        "INSERT INTO cluster_nodes(node_id, name, op_status, last_heartbeat, metadata) \
         VALUES ($1, $2, 1, now(), $3::jsonb)",
        &[
            &node_2_id,
            &"node_2".to_string(),
            &serde_json::json!({ "advertise_address": "127.0.0.1:9002" }),
        ],
    )
    .await
    .context("failed to insert node_2")?;
    tx.commit().await?;
    drop(conn);

    let node_1_state = Arc::new(cluster::NodeState::default());
    node_1_state.set_state(cluster::NodeStateType::Leader);
    let node_1_watcher = cluster::config_watcher::ConfigWatcher {
        initialized: false,
        sinks: worker_manager_1.clone(),
        listeners: worker_manager_1.clone(),
        monitors: worker_manager_1.clone(),
        inter_comm: worker_manager_1.clone(),
        node_id: node_1_id,
        node_name: "node_1".to_string(),
        cache: app_cache_1.clone(),
        db_pool: db_pool.clone(),
        secret_keys: setup::secret_keys(),
        node_lock: node_1_state.clone(),
        token: token_1.clone(),
        poll_interval: Duration::from_secs(1),
    };
    let node_2_state = Arc::new(cluster::NodeState::default());
    node_2_state.set_state(cluster::NodeStateType::Main);
    let node_2_watcher = cluster::config_watcher::ConfigWatcher {
        initialized: false,
        sinks: worker_manager_2.clone(),
        listeners: worker_manager_2.clone(),
        monitors: worker_manager_2.clone(),
        inter_comm: worker_manager_2.clone(),
        node_id: node_2_id,
        node_name: "node_2".to_string(),
        cache: app_cache_2.clone(),
        db_pool: db_pool.clone(),
        secret_keys: setup::secret_keys(),
        node_lock: node_2_state.clone(),
        token: token_2.clone(),
        poll_interval: Duration::from_secs(1),
    };
    let node_1_task = tokio::spawn(async move { node_1_watcher.run().await });
    let node_2_task = tokio::spawn(async move { node_2_watcher.run().await });

    // Each node also flips op_status 1->2->1 during initialize(), which fires
    // the trigger and emits an upsert log to the other node. So node_1 may see
    // duplicate adds for node_2 (one from initialize_cluster_nodes, one from
    // the first delta_read). They're idempotent on the real Forwarder.
    wait_initialized(&worker_manager_1).await;
    wait_initialized(&worker_manager_2).await;
    let adds_for_node_2 = forwarder_adds(&worker_manager_1, node_2_id);
    assert!(
        !adds_for_node_2.is_empty(),
        "node_1 should record at least one add for node_2"
    );
    assert!(
        adds_for_node_2.iter().all(|d| d == "127.0.0.1:9002"),
        "init-time adds for node_2 should all use 127.0.0.1:9002, got {adds_for_node_2:?}"
    );
    assert!(
        forwarder_adds(&worker_manager_1, node_1_id).is_empty(),
        "node_1 should not add itself"
    );
    let adds_for_node_1 = forwarder_adds(&worker_manager_2, node_1_id);
    assert!(
        !adds_for_node_1.is_empty(),
        "node_2 should record at least one add for node_1"
    );
    assert!(
        adds_for_node_1.iter().all(|d| d == "127.0.0.1:9001"),
        "init-time adds for node_1 should all use 127.0.0.1:9001, got {adds_for_node_1:?}"
    );
    assert!(
        forwarder_adds(&worker_manager_2, node_2_id).is_empty(),
        "node_2 should not add itself"
    );

    let mut conn = db_pool.get().await?;
    let tx = conn.transaction().await?;
    tx.execute(
        "UPDATE cluster_nodes SET metadata = $2::jsonb WHERE node_id = $1",
        &[
            &node_2_id,
            &serde_json::json!({ "advertise_address": "127.0.0.1:9999" }),
        ],
    )
    .await?;
    tx.commit().await?;
    drop(conn);
    eventually(async || {
        let adds = forwarder_adds(&worker_manager_1, node_2_id);
        ensure!(
            adds.last().map(String::as_str) == Some("127.0.0.1:9999"),
            "node_1 should pick up node_2 metadata change, adds so far {adds:?}"
        );
        Ok(())
    })
    .await;
    assert!(
        forwarder_adds(&worker_manager_2, node_2_id).is_empty(),
        "node_2 should not see its own cluster_node update"
    );

    // Drop node_2's unread logs first, as NodeLock::leader_tasks does: node_1's
    // initialize may have logged to node_2 after node_2's last clean_all_logs,
    // and cluster_logs.node_id references cluster_nodes.
    let mut conn = db_pool.get().await?;
    let tx = conn.transaction().await?;
    tx.execute("DELETE FROM cluster_logs WHERE node_id = $1", &[&node_2_id])
        .await?;
    tx.execute(
        "DELETE FROM cluster_nodes WHERE node_id = $1",
        &[&node_2_id],
    )
    .await?;
    tx.commit().await?;
    drop(conn);
    eventually(async || {
        ensure!(
            worker_manager_1
                .forwarder_calls
                .lock()
                .iter()
                .any(|c| matches!(c, ForwarderCall::Remove(id) if *id == node_2_id)),
            "node_1 should record remove for node_2"
        );
        Ok(())
    })
    .await;

    token_1.cancel();
    token_2.cancel();
    node_1_task.await?;
    node_2_task.await?;
    Ok(())
}

/// `configs.monitor` has no cluster log: the watcher re-reads it every poll and
/// applies it when the value changed. A value that does not validate keeps the
/// last good one.
#[tokio::test]
async fn the_monitor_config_is_applied_at_runtime() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    let workers = Arc::new(DummyWorkerManager::default());
    let token = CancellationToken::new();
    let state = Arc::new(cluster::NodeState::default());
    state.set_state(cluster::NodeStateType::Leader);
    let watcher = cluster::config_watcher::ConfigWatcher {
        initialized: false,
        sinks: workers.clone(),
        listeners: workers.clone(),
        monitors: workers.clone(),
        inter_comm: workers.clone(),
        node_id: Uuid::now_v7(),
        node_name: "node_1".to_string(),
        cache: cache::Cache::new(),
        db_pool: db_pool.clone(),
        secret_keys: setup::secret_keys(),
        node_lock: state,
        token: token.clone(),
        poll_interval: Duration::from_millis(200),
    };
    let task = tokio::spawn(async move { watcher.run().await });
    let set_monitor_config = async |val: &str| -> Result<()> {
        let conn = db_pool.get().await?;
        conn.execute(
            "UPDATE configs SET val = $1::TEXT::JSONB WHERE key = 'monitor'",
            &[&val],
        )
        .await?;
        Ok(())
    };

    // initialize applies the seeded row
    wait_initialized(&workers).await;
    {
        let applied = workers.monitor_configs.lock();
        assert_eq!(applied.len(), 1, "initialize applies the seeded row once");
        assert_eq!(applied[0].worker_count, 100);
    }

    // `last_updated` deliberately untouched: the value is what is compared
    set_monitor_config(r#"{"worker_count": 7, "batch_size": 500, "batch_timeout": 200}"#).await?;
    eventually(async || {
        let applied = workers.monitor_configs.lock();
        ensure!(
            applied.len() >= 2,
            "waiting for the changed row to be applied"
        );
        Ok(())
    })
    .await;
    {
        let applied = workers.monitor_configs.lock();
        assert_eq!(applied.len(), 2, "a changed row is applied once");
        assert_eq!(applied[1].worker_count, 7);
        assert_eq!(applied[1].batch.batch_size, 500);
        assert_eq!(applied[1].batch.batch_timeout, Duration::from_millis(200));
    }

    set_monitor_config(r#"{"worker_count": 9, "batch_size": 70000}"#).await?;
    // five polls of 200ms: nothing should change, so there is nothing to wait for
    sleep(Duration::from_secs(1)).await;
    assert_eq!(
        workers.monitor_configs.lock().len(),
        2,
        "an invalid row keeps the last good config"
    );

    token.cancel();
    task.await?;
    Ok(())
}

/// One node's worth of running watcher, registered in `cluster_nodes` so the
/// triggers have somewhere to fan their log rows to.
struct WatchedNode {
    cache: cache::Cache,
    workers: Arc<DummyWorkerManager>,
    state: Arc<cluster::NodeState>,
    token: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl WatchedNode {
    /// Wait for the cold start to finish, lookup tables included.
    async fn initialized(&self) {
        wait_initialized(&self.workers).await;
    }

    /// Wait until `check` holds for this node's registry.
    async fn wait_for(&self, what: &str, check: impl Fn(&Self) -> bool) {
        eventually(async || {
            ensure!(check(self), "waiting until {what}");
            Ok(())
        })
        .await
    }

    fn tables(&self) -> Vec<patrolli::cache::lookup_table::Table> {
        self.cache.lookup_table.tables()
    }

    async fn stop(self) -> Result<()> {
        self.token.cancel();
        self.task.await?;
        Ok(())
    }
}

async fn start_node(
    db_pool: &deadpool_postgres::Pool,
    node_id: Uuid,
    name: &str,
) -> Result<WatchedNode> {
    let mut conn = db_pool.get().await?;
    let tx = conn.transaction().await?;
    tx.execute(
        "INSERT INTO cluster_nodes(node_id, name, op_status, last_heartbeat, metadata) \
         VALUES ($1, $2, 1, now(), '{\"advertise_address\": \"127.0.0.1:0\"}') \
         ON CONFLICT (node_id) DO NOTHING",
        &[&node_id, &name.to_string()],
    )
    .await
    .context("failed to register cluster node")?;
    tx.commit().await?;
    drop(conn);

    let workers = Arc::new(DummyWorkerManager::default());
    let cache = cache::Cache::new();
    let token = CancellationToken::new();
    let state = Arc::new(cluster::NodeState::default());
    state.set_state(cluster::NodeStateType::Leader);
    let watcher = cluster::config_watcher::ConfigWatcher {
        initialized: false,
        sinks: workers.clone(),
        listeners: workers.clone(),
        monitors: workers.clone(),
        inter_comm: workers.clone(),
        node_id,
        node_name: name.to_string(),
        cache: cache.clone(),
        db_pool: db_pool.clone(),
        secret_keys: setup::secret_keys(),
        node_lock: state.clone(),
        token: token.clone(),
        poll_interval: Duration::from_millis(200),
    };
    let task = tokio::spawn(async move { watcher.run().await });
    Ok(WatchedNode {
        cache,
        workers,
        state,
        token,
        task,
    })
}

// ── lookup tables ─────────────────────────────────────────────────────────────
//
// A lookup table has no worker group, so unlike a sink or a listener there is
// nothing to assert against `DummyWorkerManager`: the cache registry *is* the
// applied state. These tests read it directly, and read health through
// `status::local_status`, which is what the API will eventually serve.

fn processor_source(columns: &[&str], max_rows: usize) -> patrolli::config::lookup_table::Source {
    use patrolli::config::lookup_table::{ColumnSpec, Processor, Source, ValueType};
    Source::Processor(Processor {
        columns: columns
            .iter()
            .map(|name| ColumnSpec {
                name: name.to_string(),
                value_type: ValueType::String,
            })
            .collect(),
        ttl: Duration::from_secs(60),
        max_rows,
    })
}

fn file_source(path: &str) -> patrolli::config::lookup_table::Source {
    use patrolli::config::lookup_table::{Compression, Csv, File, Format, Location, Source};
    Source::File(File {
        format: Format::Csv(Csv {
            type_cast: Default::default(),
            delimiter: ',',
            quote: '"',
            escape: None,
            null_values: vec![String::new()],
            headers: vec![],
            has_headers: Some(true),
        }),
        paths: vec![path.to_string()],
        compression: Compression::None,
        location: Location::FileSystem,
        reload_interval: Duration::from_secs(60),
    })
}

fn hash_key(columns: &[&str]) -> patrolli::config::lookup_table::Key {
    use patrolli::config::lookup_table::{Hash, Key};
    Key::Hash(Hash {
        key_columns: columns.iter().map(|s| s.to_string()).collect(),
    })
}

fn health_of(
    node: &WatchedNode,
    node_id: Uuid,
    table_id: Uuid,
) -> patrolli::cache::lookup_table::status::TableHealth {
    use patrolli::cache::lookup_table::status::{StatusFilter, local_status};
    let answer = local_status(
        node_id,
        &node.cache.lookup_table,
        &StatusFilter::by_ids(&[table_id]),
    );
    answer
        .tables
        .first()
        .unwrap_or_else(|| panic!("no status for {table_id}"))
        .health
}

/// A cold start has to register every enabled table and, because it reuses
/// `loader::init`, finish loading the file-backed ones before the node reports
/// itself active. A disabled row must not be registered at all.
#[tokio::test]
async fn test_lookup_table_initialization() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;

    let dir = tempfile::tempdir()?;
    let csv_path = dir.path().join("sites.csv");
    std::fs::write(&csv_path, b"host,site\na,jkt\nb,sby\nc,jkt\n")?;
    let csv_path = csv_path.display().to_string();

    let mut conn = db_pool.get().await?;
    let tx = conn.transaction().await?;
    let file_id = setup::create_lookup_table(
        &tx,
        "sites",
        &file_source(&csv_path),
        &hash_key(&["host"]),
        1,
    )
    .await?;
    let pipe_id = setup::create_lookup_table(
        &tx,
        "seen",
        &processor_source(&["host", "last"], 100),
        &hash_key(&["host"]),
        1,
    )
    .await?;
    let disabled_id = setup::create_lookup_table(
        &tx,
        "off",
        &processor_source(&["host"], 10),
        &hash_key(&["host"]),
        0,
    )
    .await?;
    tx.commit().await?;
    drop(conn);

    let node_id = Uuid::now_v7();
    let node = start_node(&db_pool, node_id, "node_1").await?;
    node.initialized().await;

    assert_eq!(node.tables().len(), 2, "the disabled table must be skipped");
    assert!(
        node.cache.lookup_table.get(&disabled_id).is_none(),
        "a disabled table must not be registered"
    );
    assert!(node.cache.lookup_table.get(&pipe_id).is_some());

    let file = node
        .cache
        .lookup_table
        .get(&file_id)
        .expect("the file table is registered");
    let data = file.as_immutable().expect("a file source is immutable");
    assert_eq!(
        data.data().rows(),
        3,
        "initialize must finish the load, not leave it to the reloader"
    );
    assert_eq!(
        health_of(&node, node_id, file_id),
        patrolli::cache::lookup_table::status::TableHealth::Ready
    );

    node.stop().await
}

/// The delta path must apply the four things the trigger can tell it — a new
/// row, a changed row, a disabled row, a deleted row — and, crucially, must
/// *not* rebuild a table whose config did not actually change. The trigger
/// fires on any row update, so a label edit reaches this path too.
#[tokio::test]
async fn test_lookup_table_incremental_update() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;

    let mut conn = db_pool.get().await?;
    let tx = conn.transaction().await?;
    let first = setup::create_lookup_table(
        &tx,
        "seen",
        &processor_source(&["host", "last"], 100),
        &hash_key(&["host"]),
        1,
    )
    .await?;
    tx.commit().await?;
    drop(conn);

    let node_id = Uuid::now_v7();
    let node = start_node(&db_pool, node_id, "node_1").await?;
    node.initialized().await;
    assert_eq!(node.tables().len(), 1);
    let before = node.cache.lookup_table.get(&first).unwrap();

    // a new row appears
    let mut conn = db_pool.get().await?;
    let tx = conn.transaction().await?;
    let second = setup::create_lookup_table(
        &tx,
        "sites",
        &processor_source(&["host", "site"], 50),
        &hash_key(&["host"]),
        1,
    )
    .await?;
    // ...and the first is touched in a way the runtime does not read
    tx.execute(
        "UPDATE lookup_tables SET labels = '{\"env\":\"prod\"}', last_updated = now() \
         WHERE lookup_table_id = $1",
        &[&first],
    )
    .await?;
    tx.commit().await?;
    drop(conn);
    // one transaction, so the delta that brings the new table also carries the
    // label edit
    node.wait_for("the new table is registered", |n| {
        n.cache.lookup_table.get(&second).is_some()
    })
    .await;
    assert_eq!(node.tables().len(), 2);
    let after = node.cache.lookup_table.get(&first).unwrap();
    assert!(
        Arc::ptr_eq(before.meta(), after.meta()),
        "a label edit must not rebuild the table: that would drop its loaded \
         data and reset its counters"
    );

    // a real config change does rebuild it
    let mut conn = db_pool.get().await?;
    let tx = conn.transaction().await?;
    tx.execute(
        "UPDATE lookup_tables SET source = jsonb_set(source, '{max_rows}', '7'), \
         last_updated = now() WHERE lookup_table_id = $1",
        &[&first],
    )
    .await?;
    tx.commit().await?;
    drop(conn);
    node.wait_for("a changed config rebuilds the table", |n| {
        !Arc::ptr_eq(
            before.meta(),
            n.cache.lookup_table.get(&first).unwrap().meta(),
        )
    })
    .await;
    let rebuilt = node.cache.lookup_table.get(&first).unwrap();
    assert_eq!(rebuilt.as_mutable().unwrap().table().max_rows(), 7);

    // disabling one unloads it
    let mut conn = db_pool.get().await?;
    let tx = conn.transaction().await?;
    tx.execute(
        "UPDATE lookup_tables SET adm_status = 0, last_updated = now() \
         WHERE lookup_table_id = $1",
        &[&second],
    )
    .await?;
    tx.commit().await?;
    drop(conn);
    node.wait_for(
        "a disabled table is unloaded, not left serving stale rows",
        |n| n.cache.lookup_table.get(&second).is_none(),
    )
    .await;

    // and deleting the last one empties the registry
    let mut conn = db_pool.get().await?;
    let tx = conn.transaction().await?;
    tx.execute(
        "DELETE FROM lookup_tables WHERE lookup_table_id = $1",
        &[&first],
    )
    .await?;
    tx.commit().await?;
    drop(conn);
    node.wait_for("the registry is empty", |n| n.tables().is_empty())
        .await;

    node.stop().await
}

/// The row is cluster-wide but its paths are node-local, so a node that cannot
/// see the file must still register the table and report why it is unhealthy.
/// Dropping the table instead would leave the operator with nothing to look at.
#[tokio::test]
async fn a_lookup_table_whose_file_is_missing_is_still_registered_and_reports_its_error()
-> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;

    let mut conn = db_pool.get().await?;
    let tx = conn.transaction().await?;
    let id = setup::create_lookup_table(
        &tx,
        "sites",
        &file_source("/nonexistent/definitely/not/here.csv"),
        &hash_key(&["host"]),
        1,
    )
    .await?;
    tx.commit().await?;
    drop(conn);

    let node_id = Uuid::now_v7();
    let node = start_node(&db_pool, node_id, "node_1").await?;
    node.initialized().await;

    assert!(
        node.cache.lookup_table.get(&id).is_some(),
        "a missing source file must not erase the table from this node"
    );
    assert_eq!(
        health_of(&node, node_id, id),
        patrolli::cache::lookup_table::status::TableHealth::Failed
    );

    use patrolli::cache::lookup_table::status::{StatusFilter, local_status};
    let answer = local_status(
        node_id,
        &node.cache.lookup_table,
        &StatusFilter::by_ids(&[id]),
    );
    let load = answer.tables[0]
        .load
        .as_ref()
        .expect("a file-backed table has a load section");
    let error = load.last_error.as_deref().unwrap_or_default();
    assert!(
        !error.is_empty(),
        "the operator needs the loader's own message, not just a health state"
    );

    node.stop().await
}

/// A standby node is not responsible for this configuration, so it has to let
/// the tables go — otherwise it keeps answering lookups from data it no longer
/// owns, and keeps re-reading the source files to maintain them.
#[tokio::test]
async fn a_standby_node_clears_its_lookup_tables() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;

    let mut conn = db_pool.get().await?;
    let tx = conn.transaction().await?;
    setup::create_lookup_table(
        &tx,
        "seen",
        &processor_source(&["host"], 100),
        &hash_key(&["host"]),
        1,
    )
    .await?;
    tx.commit().await?;
    drop(conn);

    let node_id = Uuid::now_v7();
    let node = start_node(&db_pool, node_id, "node_1").await?;
    node.initialized().await;
    assert_eq!(node.tables().len(), 1);

    node.state.set_state(cluster::NodeStateType::Standby);
    node.wait_for("standby releases the tables it no longer owns", |n| {
        n.tables().is_empty()
    })
    .await;

    node.stop().await
}
