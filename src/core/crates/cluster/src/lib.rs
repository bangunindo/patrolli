// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod balancer;
pub mod config_watcher;
pub mod inter;
pub mod node_lock;

use crate::inter::InterCommManager as _;
use anyhow::{Context, Result};
use chrono::Utc;
use deadpool_postgres::{Pool, Transaction};
use patrolli::chan::{AdminChannel, PROCESSOR_CHANNEL_NAME, new as new_channel};
use patrolli::common::internal_metrics::INTERNAL_METRICS;
use patrolli::common::shutdown::{Shutdown, join_step};
use patrolli::config::MonitorConfig;
use patrolli::config::cluster::{Cluster as ClusterConfig, ClusterPolicy};
use patrolli::config::global::Global;
use patrolli::config::sensitive::SecretKeys;
use patrolli::db::schema::{ConfigKeys, Configs};
use patrolli::payload;
use patrolli::{cache, common};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query};
use sea_query_postgres::PostgresBinder;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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
/// How often the inter-node gRPC server pings each connection, and how long it
/// waits for the answer before closing it.
const GRPC_KEEPALIVE_INTERVAL: time::Duration = time::Duration::from_secs(6);
const GRPC_KEEPALIVE_TIMEOUT: time::Duration = time::Duration::from_secs(3);

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
    /// The DNS resolver behind `dns_lookup` (and its metrics reporter).
    /// Node-wide like the lookup tasks, so it survives a node-lock flap.
    dns: (JoinHandle<()>, CancellationToken),
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

impl NodeStateType {
    pub fn is_main(self) -> bool {
        matches!(self, NodeStateType::Leader | NodeStateType::Main)
    }
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

const NODE_STATE_EPOCH_MASK: u64 = (1u64 << 56) - 1;

fn pack_node_state(state: NodeStateType, epoch: u64) -> u64 {
    ((epoch & NODE_STATE_EPOCH_MASK) << 8) | (state as u8 as u64)
}

fn next_main_epoch(epoch: u64) -> u64 {
    epoch.wrapping_add(1) & NODE_STATE_EPOCH_MASK
}

fn unpack_node_state(raw: u64) -> (NodeStateType, u64) {
    let state = NodeStateType::try_from(raw as u8).unwrap();
    (state, raw >> 8)
}

/// Shared node role state, packing the current [`NodeStateType`] (low 8 bits)
/// with a monotonic main-lock epoch (high 56 bits) so polling tasks can detect
/// a lock drop and reacquisition that finished between polls, or a tenure the
/// node lock renewed because the leader expired this node meanwhile.
pub struct NodeState(AtomicU64);

impl Default for NodeState {
    fn default() -> Self {
        Self(AtomicU64::new(pack_node_state(
            NodeStateType::LostConnection,
            0,
        )))
    }
}

impl NodeState {
    pub fn get_state(&self) -> NodeStateType {
        unpack_node_state(self.0.load(Ordering::Acquire)).0
    }

    /// Returns the active main-lock epoch while the node holds the main lock
    /// (`Main` or `Leader`), or `None` otherwise.
    ///
    /// The epoch increments each time the node enters `Main`/`Leader` from a
    /// non-main state (`Standby`/`LostConnection`), unless that is a
    /// [`resume_main`](Self::resume_main), and on
    /// [`renew_main_epoch`](Self::renew_main_epoch), and stays unchanged across
    /// `Main <-> Leader` transitions.
    pub fn main_epoch(&self) -> Option<u64> {
        let (state, epoch) = unpack_node_state(self.0.load(Ordering::Acquire));
        state.is_main().then_some(epoch)
    }

    pub fn set_state(&self, state: NodeStateType) {
        let prev_raw = self
            .0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |raw| {
                let (prev, epoch) = unpack_node_state(raw);
                let next_epoch = if !prev.is_main() && state.is_main() {
                    next_main_epoch(epoch)
                } else {
                    epoch
                };
                Some(pack_node_state(state, next_epoch))
            })
            .unwrap();
        let (prev_state, _) = unpack_node_state(prev_raw);
        if prev_state != state {
            info!(from = ?prev_state, to = ?state, "node state changed");
        }
    }

    /// Enters `Main`/`Leader` under the epoch the node last held, for a lock
    /// taken again after a lost connection with nothing missed in between (the
    /// node lock decides that). Polling tasks that loaded under that epoch carry
    /// on with what they hold. With no earlier epoch to continue, or from
    /// `Standby`, which clears what was loaded, this is [`set_state`](Self::set_state).
    pub fn resume_main(&self, state: NodeStateType) {
        debug_assert!(state.is_main());
        let prev_raw = self
            .0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |raw| {
                let (prev, epoch) = unpack_node_state(raw);
                let next_epoch =
                    if prev.is_main() || (prev == NodeStateType::LostConnection && epoch != 0) {
                        epoch
                    } else {
                        next_main_epoch(epoch)
                    };
                Some(pack_node_state(state, next_epoch))
            })
            .unwrap();
        let (prev_state, _) = unpack_node_state(prev_raw);
        if prev_state != state {
            info!(from = ?prev_state, to = ?state, "node state changed");
        }
    }

    /// The state and the epoch last started, read together. Unlike
    /// [`main_epoch`](Self::main_epoch) the epoch is there in any state: it is
    /// the one a lost connection may yet continue.
    pub fn snapshot(&self) -> (NodeStateType, u64) {
        unpack_node_state(self.0.load(Ordering::Acquire))
    }

    /// Starts a new main-lock epoch without leaving `Main`/`Leader`, for a
    /// tenure that kept the lock but can no longer trust what it loaded (the
    /// leader expired this node while its heartbeat was held up). Polling
    /// tasks then reload as they do after a lock flap. Does nothing while the
    /// node is not main: entering main starts an epoch by itself.
    pub fn renew_main_epoch(&self) {
        _ = self
            .0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |raw| {
                let (state, epoch) = unpack_node_state(raw);
                state
                    .is_main()
                    .then(|| pack_node_state(state, next_main_epoch(epoch)))
            });
    }

    pub fn is_main(&self) -> bool {
        self.get_state().is_main()
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
    listeners: Arc<patrolli_listener::WorkerGroup>,
    monitors: Arc<patrolli_monitor::WorkerGroup>,
    sinks: Arc<patrolli_sink::WorkerGroup>,
    inter_comm: Arc<inter::InterComm>,
    node_lock: Arc<NodeState>,
    processors: patrolli_processor::WorkerGroup,
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
        // before any processor can read it
        config_cache.dns.configure(&global_conf.dns)?;
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
        let mut processors = patrolli_processor::WorkerGroup::new(
            sink_send,
            processor_admin
                .get_receiver(&PROCESSOR_CHANNEL_NAME, global_conf.processor.channel)
                .await?,
            config_cache.clone(),
        );
        let listeners = patrolli_listener::WorkerGroup::new(
            processor_send.clone(),
            Some(forwarder_send),
            config_cache.clone(),
            Some(cluster_config.node_id),
        );
        let monitors = patrolli_monitor::WorkerGroup::new(
            processor_send,
            monitor_config.batch,
            config_cache.clone(),
        );
        let sinks = patrolli_sink::WorkerGroup::new(sink_admin.clone());
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
        let dns_token = CancellationToken::new();
        let dns = patrolli_dns::spawn(
            config_cache.dns.clone(),
            &global_conf.dns,
            global_conf.internal_metrics_interval,
            dns_token.clone(),
        )?;
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
                dns: (dns, dns_token),
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
        // Same timing as the client side (`inter::comm::build_channel`). A peer
        // that vanishes without closing (power loss, partition) otherwise
        // leaves its streams open here forever: the handler only waits on the
        // inbound stream, and nothing else would ever touch the connection.
        let mut builder = tonic::transport::Server::builder()
            .http2_keepalive_interval(Some(GRPC_KEEPALIVE_INTERVAL))
            .http2_keepalive_timeout(Some(GRPC_KEEPALIVE_TIMEOUT));
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
        // The resolver only fills the DNS cache, which outlives it; nothing
        // downstream waits for an answer, so it goes before anything else.
        self.lifecycle.dns.1.cancel();
        join_step("dns", self.lifecycle.dns.0).await;
        // Then: the reloader swaps the data behind a lookup table that a
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_epoch_tracks_lock_flaps_but_not_leader_promotion() {
        let state = NodeState::default();
        assert_eq!(state.get_state(), NodeStateType::LostConnection);
        assert_eq!(state.main_epoch(), None);

        // Acquiring main bumps the epoch.
        state.set_state(NodeStateType::Main);
        let first = state.main_epoch().expect("main must have an epoch");

        // Promoting Main -> Leader (or repeating the same state) keeps the same epoch.
        state.set_state(NodeStateType::Main);
        assert_eq!(state.main_epoch(), Some(first));
        state.set_state(NodeStateType::Leader);
        assert_eq!(state.main_epoch(), Some(first));
        state.set_state(NodeStateType::Leader);
        assert_eq!(state.main_epoch(), Some(first));

        // Dropping to LostConnection and reacquiring bumps the epoch.
        state.set_state(NodeStateType::LostConnection);
        assert_eq!(state.main_epoch(), None);
        state.set_state(NodeStateType::Leader);
        let second = state.main_epoch().expect("reacquired leader has an epoch");
        assert_ne!(second, first);

        // Dropping to Standby and returning to Main bumps the epoch again.
        state.set_state(NodeStateType::Standby);
        assert_eq!(state.main_epoch(), None);
        state.set_state(NodeStateType::Main);
        let third = state.main_epoch().expect("promoted main has an epoch");
        assert_ne!(third, second);
    }

    #[test]
    fn main_epoch_ignores_non_main_transitions_and_leader_demotion() {
        let state = NodeState::default();
        // Flipping between non-main states never yields an epoch or bumps it.
        state.set_state(NodeStateType::Standby);
        assert_eq!(state.main_epoch(), None);
        state.set_state(NodeStateType::LostConnection);
        assert_eq!(state.main_epoch(), None);
        state.set_state(NodeStateType::Standby);
        assert_eq!(state.main_epoch(), None);

        // First entry into main sets epoch to 1.
        state.set_state(NodeStateType::Leader);
        assert_eq!(state.main_epoch(), Some(1));

        // Demoting Leader -> Main without leaving main preserves the epoch.
        state.set_state(NodeStateType::Main);
        assert_eq!(state.main_epoch(), Some(1));
        assert!(state.is_main());
    }

    #[test]
    fn renewing_the_main_epoch_keeps_the_state_and_needs_the_lock() {
        let state = NodeState::default();
        // Not main: nothing to renew, and no epoch appears.
        state.renew_main_epoch();
        assert_eq!(state.get_state(), NodeStateType::LostConnection);
        assert_eq!(state.main_epoch(), None);
        state.set_state(NodeStateType::Standby);
        state.renew_main_epoch();
        assert_eq!(state.get_state(), NodeStateType::Standby);

        // The renewals above were no-ops, so entering main is still epoch 1.
        state.set_state(NodeStateType::Main);
        assert_eq!(state.main_epoch(), Some(1));

        // Main: a new epoch, same state.
        state.renew_main_epoch();
        assert_eq!(state.get_state(), NodeStateType::Main);
        assert_eq!(state.main_epoch(), Some(2));

        // Leader too.
        state.set_state(NodeStateType::Leader);
        state.renew_main_epoch();
        assert_eq!(state.get_state(), NodeStateType::Leader);
        assert_eq!(state.main_epoch(), Some(3));
    }

    #[test]
    fn resuming_continues_the_epoch_a_lost_connection_interrupted() {
        let state = NodeState::default();
        // No tenure yet: nothing to continue, so this is a first epoch.
        state.resume_main(NodeStateType::Main);
        assert_eq!(state.main_epoch(), Some(1));

        // Lost and resumed: same epoch, as Main or as Leader.
        state.set_state(NodeStateType::LostConnection);
        assert_eq!(state.main_epoch(), None);
        assert_eq!(state.snapshot(), (NodeStateType::LostConnection, 1));
        state.resume_main(NodeStateType::Leader);
        assert_eq!(state.snapshot(), (NodeStateType::Leader, 1));

        // Lost and taken without resuming: a new one.
        state.set_state(NodeStateType::LostConnection);
        state.set_state(NodeStateType::Main);
        assert_eq!(state.main_epoch(), Some(2));

        // Standby cleared what was loaded: resuming from it starts over.
        state.set_state(NodeStateType::Standby);
        state.resume_main(NodeStateType::Main);
        assert_eq!(state.main_epoch(), Some(3));
    }

    #[test]
    fn node_state_epoch_wraps_without_corrupting_state() {
        // Start right at the 56-bit epoch ceiling in a non-main state.
        let state = NodeState(AtomicU64::new(pack_node_state(
            NodeStateType::Standby,
            NODE_STATE_EPOCH_MASK,
        )));
        assert_eq!(state.get_state(), NodeStateType::Standby);
        assert_eq!(state.main_epoch(), None);

        // Entering main wraps the epoch to 0 while keeping the low 8-bit state intact.
        state.set_state(NodeStateType::Leader);
        assert_eq!(state.get_state(), NodeStateType::Leader);
        assert_eq!(state.main_epoch(), Some(0));

        // Next flap advances normally from 0 -> 1.
        state.set_state(NodeStateType::LostConnection);
        assert_eq!(state.get_state(), NodeStateType::LostConnection);
        assert_eq!(state.main_epoch(), None);

        state.set_state(NodeStateType::Main);
        assert_eq!(state.get_state(), NodeStateType::Main);
        assert_eq!(state.main_epoch(), Some(1));
    }
}
