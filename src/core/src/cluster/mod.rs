// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod balancer;
pub mod config_watcher;
pub mod inter;
pub mod node_lock;

use crate::chan::{AdminChannel, new as new_channel};
use crate::cluster::inter::InterCommManager as _;
use crate::common::internal_metrics::INTERNAL_METRICS;
use crate::common::shutdown::{Shutdown, join_step};
use crate::config::MonitorConfig;
use crate::config::cluster::{Cluster as ClusterConfig, ClusterPolicy};
use crate::config::global::Global;
use crate::config::sensitive::SecretKeys;
use crate::db::schema::{ConfigKeys, Configs};
use crate::payload;
use crate::processor;
use crate::{cache, common, listener, monitor, sink};
use anyhow::{Context, Result};
use chrono::Utc;
use deadpool_postgres::{Pool, Transaction};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query};
use sea_query_postgres::PostgresBinder;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time;
use tokio_postgres::error::SqlState;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;

/// How long to wait for the inter-node gRPC server to fail before declaring
/// the bind successful (same idiom as the OTAP listener transport).
const GRPC_STARTUP_GRACE: time::Duration = time::Duration::from_millis(300);
/// How long shutdown waits for the inter-node gRPC server to close its
/// connections. Its streaming calls end as soon as it is told to stop, so this
/// only bounds a call stuck in a handler.
const GRPC_DRAIN_TIMEOUT: time::Duration = time::Duration::from_secs(5);

struct Lifecycle {
    sink_reporter: JoinHandle<()>,
    sink_supervisor: JoinHandle<()>,
    monitor_reporter: JoinHandle<()>,
    listener_reporter: JoinHandle<()>,
    listener_supervisor: JoinHandle<()>,
    monitor_run: JoinHandle<()>,
    inter_comm_reporter: JoinHandle<()>,
    grpc_server: (JoinHandle<()>, CancellationToken),
    /// The lookup-table reloader and metrics reporter, sharing one token
    /// because they are torn down together. Minted in `new` because `run`'s
    /// does not exist yet, the reason `grpc_server` carries its own.
    lookup_tasks: (JoinHandle<()>, JoinHandle<()>, CancellationToken),
}

#[repr(u8)]
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum NodeStateType {
    Leader = 1,
    Main = 2,
    // we can reach the database, but other node acquired the main lock
    Standby = 3,
    // we can't reach the database
    LostConnection = 4,
}

impl TryFrom<u8> for NodeStateType {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        match value {
            1 => Ok(NodeStateType::Leader),
            2 => Ok(NodeStateType::Main),
            3 => Ok(NodeStateType::Standby),
            4 => Ok(NodeStateType::LostConnection),
            _ => Err(anyhow::anyhow!("Invalid node state type: {}", value)),
        }
    }
}

pub struct NodeState(AtomicU8);

impl Default for NodeState {
    fn default() -> Self {
        Self(AtomicU8::new(NodeStateType::LostConnection as u8))
    }
}

impl NodeState {
    pub fn get_state(&self) -> NodeStateType {
        NodeStateType::try_from(self.0.load(Ordering::Relaxed)).unwrap()
    }

    pub fn set_state(&self, state: NodeStateType) {
        let prev = self.0.swap(state as u8, Ordering::Relaxed);
        if let Ok(prev) = NodeStateType::try_from(prev)
            && prev != state
        {
            info!(from = ?prev, to = ?state, "node state changed");
        }
    }

    pub fn is_main(&self) -> bool {
        let state = self.get_state();
        state == NodeStateType::Leader || state == NodeStateType::Main
    }
    pub fn is(&self, state: NodeStateType) -> bool {
        self.get_state() == state
    }
}

pub struct Cluster {
    cluster: ClusterConfig,
    db_pool: Pool,
    secret_keys: Arc<SecretKeys>,
    cache: cache::Cache,
    listeners: Arc<listener::WorkerGroup>,
    monitors: Arc<monitor::WorkerGroup>,
    sinks: Arc<sink::WorkerGroup>,
    inter_comm: Arc<inter::InterComm>,
    node_lock: Arc<NodeState>,
    processors: processor::WorkerGroup,
    sink_channel: AdminChannel<Uuid, payload::Payload>,
    processor_channel: AdminChannel<&'static str, payload::ProcessorPayload>,
    forwarder_channel: AdminChannel<&'static str, payload::ForwarderPayload>,
    lifecycle: Lifecycle,
    watcher_poll_interval: time::Duration,
    /// Process-wide shutdown: its token goes to the node lock so an
    /// unrecoverable standby identity clash can bring the whole process down,
    /// and its deadline bounds [`shutdown`](Self::shutdown).
    app_shutdown: Shutdown,
}

/// Read the config stored under `key`, `None` when there is no row. The outer
/// error is the database's; the inner one is a value that does not parse or
/// validate, which a caller re-reading it at runtime answers by keeping the last
/// good value instead of failing.
pub(crate) async fn read_config<T: DeserializeOwned>(
    tx: &Transaction<'_>,
    key: ConfigKeys,
    validate: fn(&mut T) -> Result<()>,
) -> Result<Option<Result<T>>> {
    let (sql, values) = Query::select()
        .columns([Configs::Val])
        .from(Configs::Table)
        .and_where(Expr::col(Configs::Key).eq(key))
        .build_postgres(PostgresQueryBuilder);
    let row = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .with_context(|| format!("failed to query {key} config"))?;
    Ok(row.map(|row| {
        let raw: serde_json::Value = row
            .try_get(0)
            .with_context(|| format!("failed to get {key} config"))?;
        let mut config: T = serde_json::from_value(raw)
            .with_context(|| format!("failed to deserialize {key} config"))?;
        validate(&mut config).with_context(|| format!("invalid {key} config"))?;
        Ok(config)
    }))
}

impl Cluster {
    /// Read the config stored under `key`, inserting `T::default()` when there is
    /// none yet so the row exists for an operator to edit. Another node seeding
    /// the same key concurrently is not an error.
    async fn db_config<T>(
        pool: &Pool,
        key: ConfigKeys,
        validate: fn(&mut T) -> Result<()>,
    ) -> Result<T>
    where
        T: Default + Serialize + DeserializeOwned,
    {
        let mut client = pool.get().await.context("failed to get db connection")?;
        let tx = client
            .transaction()
            .await
            .context("failed to start db transaction")?;
        match read_config(&tx, key, validate).await? {
            None => {
                let config = T::default();
                let (sql, values) = Query::insert()
                    .into_table(Configs::Table)
                    .columns([Configs::Key, Configs::Val, Configs::LastUpdated])
                    .values([
                        key.into(),
                        serde_json::to_value(&config)
                            .context("failed to serialize config")?
                            .into(),
                        Utc::now().into(),
                    ])
                    .context("failed to build config values")?
                    .build_postgres(PostgresQueryBuilder);
                let insert_result = tx.execute(sql.as_str(), &values.as_params()).await;
                match insert_result {
                    Ok(_) => {
                        tx.commit()
                            .await
                            .context("failed to commit db transaction")?;
                        Ok(config)
                    }
                    Err(e) => {
                        if let Some(db_err) = e.as_db_error()
                            && db_err.code() == &SqlState::UNIQUE_VIOLATION
                        {
                            return Ok(config);
                        }
                        Err(e).context(format!("failed to insert {key} config"))
                    }
                }
            }
            Some(config) => config,
        }
    }

    pub async fn new(
        cluster_config: ClusterConfig,
        db_pool: Pool,
        global_conf: &Global,
        app_shutdown: Shutdown,
        secret_keys: Arc<SecretKeys>,
    ) -> Result<Self> {
        let backend = global_conf
            .backend
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("backend config is required"))?;
        let monitor_config =
            Self::db_config(&db_pool, ConfigKeys::Monitor, MonitorConfig::validate).await?;
        // only seeded here: the leader re-reads it every tick
        Self::db_config(&db_pool, ConfigKeys::Cluster, ClusterPolicy::validate).await?;
        let config_cache = cache::get_cache();
        let (processor_admin, processor_send) = new_channel(
            global_conf.internal_metrics_interval,
            INTERNAL_METRICS.processor_channel.clone(),
        );
        let (forwarder_admin, forwarder_send) = new_channel(
            global_conf.internal_metrics_interval,
            INTERNAL_METRICS.forwarder_channel.clone(),
        );
        let (sink_admin, sink_send) = new_channel(
            global_conf.internal_metrics_interval,
            INTERNAL_METRICS.sink_channel.clone(),
        );
        let inter_comm = inter::InterComm::new(
            forwarder_admin
                .get_receiver(
                    &inter::FORWARDER_CHANNEL_NAME,
                    cluster_config.inter_node.forwarder.channel,
                )
                .await?,
            cluster_config.inter_node.clone(),
            cluster_config.node_id,
            cluster_config.node_name.clone(),
            config_cache.clone(),
        )?;
        let grpc_token = CancellationToken::new();
        let grpc_service = inter::InterCommServer::new_service(
            cluster_config.node_id,
            cluster_config.node_name.clone(),
            config_cache.clone(),
            processor_send.clone(),
            &cluster_config.inter_node.grpc,
            global_conf.internal_metrics_interval,
            grpc_token.clone(),
        )?;
        let grpc_server =
            Self::spawn_grpc_server(grpc_service, &cluster_config, grpc_token.clone()).await?;
        let mut processors = processor::WorkerGroup::new(
            sink_send,
            processor_admin
                .get_receiver(
                    &processor::PROCESSOR_CHANNEL_NAME,
                    global_conf.processor.channel,
                )
                .await?,
            config_cache.clone(),
        );
        let listeners = listener::WorkerGroup::new(
            processor_send.clone(),
            Some(forwarder_send),
            config_cache.clone(),
            Some(cluster_config.node_id),
        );
        let monitors =
            monitor::WorkerGroup::new(processor_send, monitor_config.batch, config_cache.clone());
        let sinks = sink::WorkerGroup::new(sink_admin.clone());
        let sink_reporter = sinks
            .clone()
            .spawn_metrics_reporter(global_conf.internal_metrics_interval)
            .await;
        let monitor_reporter = monitors
            .clone()
            .spawn_metrics_reporter(global_conf.internal_metrics_interval)
            .await;
        let listener_reporter = listeners
            .clone()
            .spawn_metrics_reporter(global_conf.internal_metrics_interval)
            .await;
        let sink_supervisor = sinks.clone().spawn_supervisor();
        let listener_supervisor = listeners.clone().spawn_supervisor();
        processors
            .spawn_metrics_reporter(global_conf.internal_metrics_interval)
            .await;
        let inter_comm_reporter = inter_comm
            .spawn_metrics_reporter(global_conf.internal_metrics_interval)
            .await;
        processors.run().await;
        let monitor_run = monitors.clone().run().await;
        monitors.adjust(monitor_config.worker_count).await;
        // The registry is empty until `config_watcher` fills it, so both tasks
        // simply find nothing to do. Starting them here rather than in the
        // watcher keeps them alive across a node-lock flap.
        let lookup_token = CancellationToken::new();
        let lookup_reloader =
            cache::lookup_table::loader::spawn_reloader(config_cache.clone(), lookup_token.clone());
        let lookup_reporter = cache::lookup_table::metrics::spawn_metrics_reporter(
            config_cache.clone(),
            global_conf.internal_metrics_interval,
            lookup_token.clone(),
        );
        Ok(Self {
            cluster: cluster_config,
            db_pool,
            secret_keys,
            cache: config_cache,
            listeners,
            processors,
            monitors,
            sinks,
            inter_comm,
            sink_channel: sink_admin,
            processor_channel: processor_admin,
            forwarder_channel: forwarder_admin,
            watcher_poll_interval: backend.watcher_interval,
            app_shutdown,
            lifecycle: Lifecycle {
                sink_reporter,
                sink_supervisor,
                monitor_reporter,
                listener_reporter,
                listener_supervisor,
                monitor_run,
                inter_comm_reporter,
                grpc_server: (grpc_server, grpc_token),
                lookup_tasks: (lookup_reloader, lookup_reporter, lookup_token),
            },
            node_lock: Arc::new(NodeState::default()),
        })
    }

    pub fn inter_comm(&self) -> Arc<inter::InterComm> {
        self.inter_comm.clone()
    }

    /// Serve the inter-node gRPC service, bound on all interfaces at the port
    /// this node advertises to its peers (`metadata.advertise_address`).
    async fn spawn_grpc_server(
        service: inter::InterCommService,
        cluster_config: &ClusterConfig,
        token: CancellationToken,
    ) -> Result<JoinHandle<()>> {
        let addr = cluster_config.inter_node.grpc.address;
        let mut builder = tonic::transport::Server::builder();
        if let Some(tls_conf) = &cluster_config.inter_node.grpc.tls {
            builder = builder.tls_config(common::tls::build_tonic_server_tls(tls_conf)?)?;
        }
        let router = builder.add_service(service);
        // Forward the serve outcome over a oneshot. A bind failure (e.g. port
        // in use) makes `serve_with_shutdown` return almost immediately, so if
        // no error arrives within the grace window the bind succeeded.
        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<Result<()>>();
        let server_task = tokio::spawn(async move {
            let res = router.serve_with_shutdown(addr, token.cancelled()).await;
            // a failed send means the startup grace window already passed and
            // spawn_grpc_server returned, so nobody else will report this outcome
            if let Err(res) = started_tx.send(res.map_err(anyhow::Error::from)) {
                match res {
                    Ok(()) => info!(addr = %addr, "inter-node grpc server stopped"),
                    Err(e) => error!(
                        error = %format!("{e:#}"),
                        addr = %addr,
                        "inter-node grpc server failed"
                    ),
                }
            }
        });
        match tokio::time::timeout(GRPC_STARTUP_GRACE, started_rx).await {
            // Serve returned within the grace window → startup failed.
            Ok(Ok(Err(e))) => Err(e.context("inter-node grpc server failed to start")),
            Ok(Ok(Ok(()))) => Err(anyhow::anyhow!(
                "inter-node grpc server stopped immediately after start"
            )),
            Ok(Err(_)) => Err(anyhow::anyhow!(
                "inter-node grpc server task ended unexpectedly"
            )),
            // Still serving after the grace window → bind succeeded.
            Err(_) => {
                info!(addr = %addr, "inter-node grpc server listening");
                Ok(server_task)
            }
        }
    }

    async fn shutdown(self) {
        let deadline = self.app_shutdown.deadline();
        // First: the reloader swaps the data behind a lookup table that a
        // processor may still be reading, so it has to stop before they do.
        self.lifecycle.lookup_tasks.2.cancel();
        join_step("lookup_reload", self.lifecycle.lookup_tasks.0).await;
        join_step("lookup_sweep", self.lifecycle.lookup_tasks.1).await;
        self.monitors.close_scheduler();
        join_step("monitor_run", self.lifecycle.monitor_run).await;
        self.monitors.close_metrics_reporter();
        join_step("monitor_reporter", self.lifecycle.monitor_reporter).await;
        drop(self.monitors);
        self.listeners.close(deadline).await;
        join_step("listener_reporter", self.lifecycle.listener_reporter).await;
        join_step("listener_supervisor", self.lifecycle.listener_supervisor).await;
        drop(self.listeners);
        // the inter-node grpc server feeds the processor channel, so it must
        // stop before the processors close
        self.lifecycle.grpc_server.1.cancel();
        let mut grpc_server = self.lifecycle.grpc_server.0;
        let limit = deadline.budget(GRPC_DRAIN_TIMEOUT);
        match time::timeout(limit, &mut grpc_server).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => error!(error = %e, step = "grpc_server", "shutdown step failed"),
            Err(_) => {
                warn!(
                    timeout_ms = limit.as_millis() as u64,
                    "inter-node grpc drain timed out, aborting server"
                );
                grpc_server.abort();
            }
        }
        drop(self.processor_channel);
        self.processors.close(deadline).await;
        drop(self.forwarder_channel);
        join_step("inter_comm_reporter", self.lifecycle.inter_comm_reporter).await;
        self.inter_comm.close_all().await;
        drop(self.sink_channel);
        self.sinks.close(deadline).await;
        join_step("sink_reporter", self.lifecycle.sink_reporter).await;
        join_step("sink_supervisor", self.lifecycle.sink_supervisor).await;
        drop(self.sinks);
        info!("cluster shutdown complete");
    }

    #[tracing::instrument(skip_all, fields(node_id = %self.cluster.node_id, node = %self.cluster.node_name))]
    async fn main_loop(self, token: CancellationToken) {
        let mut watchers = JoinSet::<()>::new();
        let token_clone = token.clone();
        let db_pool = self.db_pool.clone();
        let secret_keys = self.secret_keys.clone();
        let cache = self.cache.clone();
        let node_lock = self.node_lock.clone();
        let node_id = self.cluster.node_id;
        let node_name = self.cluster.node_name.clone();
        let listeners = self.listeners.clone();
        let monitors = self.monitors.clone();
        let sinks = self.sinks.clone();
        let inter_comm = self.inter_comm.clone();
        watchers.spawn(async move {
            let watcher = config_watcher::ConfigWatcher {
                node_id,
                node_name,
                db_pool,
                secret_keys,
                cache,
                token: token_clone,
                poll_interval: self.watcher_poll_interval,
                node_lock,
                initialized: false,
                listeners,
                monitors,
                sinks,
                inter_comm,
            };
            watcher.run().await;
        });
        let token_clone = token.clone();
        let db_pool = self.db_pool.clone();
        let cluster_config = self.cluster.clone();
        let node_lock = self.node_lock.clone();
        let app_shutdown = self.app_shutdown.token();
        watchers.spawn(async move {
            let watcher = node_lock::NodeLock::new(
                db_pool,
                token_clone,
                app_shutdown,
                cluster_config,
                node_lock,
            );
            watcher.run().await;
        });
        token.cancelled().await;
        while let Some(watcher) = watchers.join_next().await {
            if let Err(e) = watcher {
                error!(error = %e, "watcher task failed");
            }
        }
        self.shutdown().await;
    }

    pub fn run(self) -> (JoinHandle<()>, CancellationToken) {
        let token = CancellationToken::new();
        let token_clone = token.clone();
        (
            tokio::spawn(async move {
                self.main_loop(token_clone).await;
            }),
            token,
        )
    }
}
