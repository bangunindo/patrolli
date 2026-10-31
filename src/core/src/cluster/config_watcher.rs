// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::{NodeState, NodeStateType};
use crate::config::cluster::NodeMetadata;
use crate::config::endpoint::{Config as EndpointConfig, Endpoint};
use crate::config::host::Host;
use crate::config::item::{Config as ItemConfig, Item, Schedule};
use crate::config::listener::{Config as ListenerConfig, Listener};
use crate::config::lookup_table::LookupTable as LookupTableConfig;
use crate::config::processor::{Config, Processor};
use crate::config::sensitive::{SecretError, SecretKeys, SensitiveFields};
use crate::config::sink::{Config as SinkConfig, Sink};
use crate::db::{query, schema};
use crate::{cache, listener, monitor, sink};

use crate::config::{Batch, Channel, MonitorConfig, Retry};
use anyhow::{Context, Result};
use deadpool_postgres::{GenericClient, Pool, Transaction};
use sea_query::{
    BinOper, Expr, ExprTrait, IntoColumnRef, Order, PostgresQueryBuilder, Query as DbQuery,
    QueryStatementBuilder,
};
use sea_query_postgres::PostgresBinder;
use smol_str::ToSmolStr;
use std::net::IpAddr;
use std::sync::Arc;
use thiserror::Error;
use tokio::time::{Duration, sleep};
use tokio_postgres::{IsolationLevel, Row};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

#[derive(Debug, Error)]
enum ConfigWatcherError {
    #[error("node lock lost")]
    NodeLockLost,
    #[error("other error: {0}")]
    Other(anyhow::Error),
}

impl From<anyhow::Error> for ConfigWatcherError {
    fn from(e: anyhow::Error) -> Self {
        ConfigWatcherError::Other(e)
    }
}

pub struct ConfigWatcher {
    pub node_id: Uuid,
    pub node_name: String,
    pub db_pool: Pool,
    pub secret_keys: Arc<SecretKeys>,
    pub cache: cache::Cache,
    pub token: CancellationToken,
    pub poll_interval: Duration,
    pub node_lock: Arc<NodeState>,
    pub initialized: bool,

    pub listeners: Arc<dyn listener::WorkerManager>,
    pub monitors: Arc<dyn monitor::WorkerManager>,
    pub sinks: Arc<dyn sink::WorkerManager>,
    pub inter_comm: Arc<dyn super::inter::InterCommManager>,
}

impl ConfigWatcher {
    async fn sink_query(&self, tx: &Transaction<'_>, sink_id: Option<Expr>) -> Result<Vec<Sink>> {
        let mut query_params = query::SinkParams::new();
        query_params
            .with_columns(vec![
                query::SinkColumn::SinkId,
                query::SinkColumn::Name,
                query::SinkColumn::BatchSize,
                query::SinkColumn::BatchTimeoutMs,
                query::SinkColumn::Channel,
                query::SinkColumn::Retry,
                query::SinkColumn::WorkerCount,
                query::SinkColumn::ConfigEmbed,
                query::SinkColumn::Inputs,
            ])
            .with_only_enabled(true)
            .with_id_subquery_filter(sink_id);
        let query = query::sinks(query_params);
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        trace!(sql = %sql, values = ?values, "sink query");
        let rows = tx
            .query(&sql, &values.as_params())
            .await
            .context("failed to query sinks")?;
        let mut sinks: Vec<Sink> = Vec::with_capacity(rows.len());
        for row in rows {
            let (id, name) = row_identity(
                &row,
                query::SinkColumn::SinkId.as_str(),
                query::SinkColumn::Name.as_str(),
            )?;
            let sink = sink_from_row(&row, id, name.clone(), &self.secret_keys)
                .with_context(|| format!("invalid sink {name:?} ({id})"))?;
            sinks.push(sink);
        }
        Ok(sinks)
    }

    async fn processor_query(
        &self,
        tx: &Transaction<'_>,
        processor_id: Option<Expr>,
    ) -> Result<Vec<Processor>> {
        let mut query_params = query::ProcessorParams::new();
        query_params
            .with_columns(vec![
                query::ProcessorColumn::ProcessorId,
                query::ProcessorColumn::Name,
                query::ProcessorColumn::ConfigEmbed,
                query::ProcessorColumn::Inputs,
            ])
            .with_disabled_to_null(true)
            .with_id_subquery_filter(processor_id);
        let query = query::processors(query_params);
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        trace!(sql = %sql, values = ?values, "processor query");
        let rows = tx
            .query(&sql, &values.as_params())
            .await
            .context("failed to query processors")?;
        let mut processors: Vec<Processor> = Vec::with_capacity(rows.len());
        for row in rows {
            let (id, name) = row_identity(
                &row,
                query::ProcessorColumn::ProcessorId.as_str(),
                query::ProcessorColumn::Name.as_str(),
            )?;
            let processor = processor_from_row(&row, id, name.clone())
                .with_context(|| format!("invalid processor {name:?} ({id})"))?;
            processors.push(processor);
        }
        Ok(processors)
    }

    async fn listener_query(
        &self,
        tx: &Transaction<'_>,
        listener_id: Option<Expr>,
    ) -> Result<Vec<Listener>> {
        let mut query_params = query::ListenerParams::new();
        query_params
            .with_columns(vec![
                query::ListenerColumn::ListenerId,
                query::ListenerColumn::Name,
                query::ListenerColumn::WorkerCount,
                query::ListenerColumn::BatchSize,
                query::ListenerColumn::BatchTimeoutMs,
                query::ListenerColumn::ConfigEmbed,
                query::ListenerColumn::DisableClusterForwarding,
            ])
            .with_only_enabled(true)
            .with_id_subquery_filter(listener_id);
        let query = query::listeners(query_params);
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        trace!(sql = %sql, values = ?values, "listener query");
        let rows = tx
            .query(&sql, &values.as_params())
            .await
            .context("failed to query listeners")?;
        let mut listeners: Vec<Listener> = Vec::with_capacity(rows.len());
        for row in rows {
            let (id, name) = row_identity(
                &row,
                query::ListenerColumn::ListenerId.as_str(),
                query::ListenerColumn::Name.as_str(),
            )?;
            let listener = listener_from_row(&row, id, &name, &self.secret_keys)
                .with_context(|| format!("invalid listener {name:?} ({id})"))?;
            listeners.push(listener);
        }
        Ok(listeners)
    }

    /// Read the enabled lookup tables, optionally narrowed to a set of ids.
    ///
    /// Unlike [`Self::sink_query`] and [`Self::listener_query`], a row that
    /// will not deserialize or validate is logged and skipped rather than
    /// aborting the whole poll cycle: losing one table beats letting a
    /// hand-edited `key` stall every other resource on the node.
    ///
    /// Structural cases only. A source file missing on *this* node never
    /// reaches here — [`LookupTableConfig::validate_structure`] lets the table
    /// through so the loader can record the real error.
    async fn lookup_table_query(
        &self,
        tx: &Transaction<'_>,
        lookup_table_id: Option<Expr>,
    ) -> Result<Vec<LookupTableConfig>> {
        let mut query_params = query::LookupTableParams::new();
        query_params
            .with_columns(vec![
                query::LookupTableColumn::LookupTableId,
                query::LookupTableColumn::Name,
                query::LookupTableColumn::SourceEmbed,
                query::LookupTableColumn::KeyEmbed,
            ])
            .with_only_enabled(true)
            .with_id_subquery_filter(lookup_table_id);
        let query = query::lookup_tables(query_params);
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        trace!(sql = %sql, values = ?values, "lookup table query");
        let rows = tx
            .query(&sql, &values.as_params())
            .await
            .context("failed to query lookup tables")?;
        let mut tables: Vec<LookupTableConfig> = Vec::with_capacity(rows.len());
        for row in rows {
            let id: Uuid = row
                .try_get(query::LookupTableColumn::LookupTableId.as_str())
                .context("failed to get lookup table id")?;
            let name: String = row
                .try_get(query::LookupTableColumn::Name.as_str())
                .context("failed to get lookup table name")?;
            let source: serde_json::Value = row
                .try_get(query::LookupTableColumn::SourceEmbed.as_str())
                .context("failed to get lookup table source")?;
            let key: serde_json::Value = row
                .try_get(query::LookupTableColumn::KeyEmbed.as_str())
                .context("failed to get lookup table key")?;
            let source = match serde_json::from_value(source) {
                Ok(source) => source,
                Err(e) => {
                    error!(lookup_table_id = %id, lookup_table = %name, error = %e, "skipping lookup table: invalid source");
                    continue;
                }
            };
            let key = match serde_json::from_value(key) {
                Ok(key) => key,
                Err(e) => {
                    error!(lookup_table_id = %id, lookup_table = %name, error = %e, "skipping lookup table: invalid key");
                    continue;
                }
            };
            let mut table = LookupTableConfig {
                id,
                name,
                source,
                key,
            };
            if let Err(e) = table.validate_structure() {
                error!(lookup_table_id = %id, lookup_table = %table.name, error = %format!("{e:#}"), "skipping invalid lookup table");
                continue;
            }
            tables.push(table);
        }
        Ok(tables)
    }

    async fn host_query(&self, tx: &Transaction<'_>, host_id: Option<Expr>) -> Result<Vec<Host>> {
        let mut query_params = query::HostParams::new();
        query_params
            .with_columns(vec![
                query::HostColumn::HostId,
                query::HostColumn::Name,
                query::HostColumn::Timezone,
            ])
            .with_only_enabled(true)
            .with_monitored_by(Some(self.node_id))
            .with_id_subquery_filter(host_id);
        let query = query::hosts(query_params);
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        let rows = tx
            .query(&sql, &values.as_params())
            .await
            .context("failed to query hosts")?;
        let mut hosts: Vec<Host> = Vec::with_capacity(rows.len());
        for row in rows {
            let (id, name) = row_identity(
                &row,
                query::HostColumn::HostId.as_str(),
                query::HostColumn::Name.as_str(),
            )?;
            let host = host_from_row(&row, id, &name)
                .with_context(|| format!("invalid host {name:?} ({id})"))?;
            hosts.push(host);
        }
        Ok(hosts)
    }

    async fn endpoint_query(
        &self,
        tx: &Transaction<'_>,
        endpoint_id: Option<Expr>,
    ) -> Result<Vec<Endpoint>> {
        let mut query_params = query::EndpointParams::new();
        query_params
            .with_columns(vec![
                query::EndpointColumn::EndpointId,
                query::EndpointColumn::Name,
                query::EndpointColumn::HostId,
                query::EndpointColumn::ConfigEmbed,
            ])
            .with_only_enabled_resolved(true)
            .with_monitored_by(Some(self.node_id))
            .with_id_subquery_filter(endpoint_id);
        let query = query::endpoints(query_params);
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        let rows = tx
            .query(&sql, &values.as_params())
            .await
            .context("failed to query endpoints")?;
        let mut endpoints: Vec<Endpoint> = Vec::with_capacity(rows.len());
        for row in rows {
            let (id, name) = row_identity(
                &row,
                query::EndpointColumn::EndpointId.as_str(),
                query::EndpointColumn::Name.as_str(),
            )?;
            let endpoint = endpoint_from_row(&row, id, &name, &self.secret_keys)
                .with_context(|| format!("invalid endpoint {name:?} ({id})"))?;
            endpoints.push(endpoint);
        }
        Ok(endpoints)
    }

    async fn item_query(
        &self,
        tx: &Transaction<'_>,
        item_id: Option<Expr>,
        limit: u64,
        order_bys: Vec<(Expr, Order)>,
        filters: Vec<Expr>,
    ) -> Result<Vec<Item>> {
        let mut query_params = query::ItemParams::new();
        query_params
            .with_columns(vec![
                query::ItemColumn::ItemId,
                query::ItemColumn::ResolvedName,
                query::ItemColumn::EndpointId,
                query::ItemColumn::HostId,
                query::ItemColumn::ResolvedSchedule,
                query::ItemColumn::ListenerId,
                query::ItemColumn::ResolvedTimeoutMs,
                query::ItemColumn::ResolvedOutputs,
                query::ItemColumn::ResolvedConfigEmbed,
            ])
            .with_monitored_by(Some(self.node_id))
            .with_only_enabled_resolved(true)
            .with_order_bys(order_bys)
            .with_limit(limit)
            .with_additional_filters(filters);
        if let Some(item_id) = item_id {
            query_params.with_id_subquery_filter(Some(item_id));
        }
        let query = query::items(query_params);
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        trace!(sql = %sql, values = ?values, "item query");
        let rows = tx
            .query(&sql, &values.as_params())
            .await
            .context("failed to query items")?;
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let (id, name) = row_identity(
                &row,
                query::ItemColumn::ItemId.as_str(),
                query::ItemColumn::ResolvedName.as_str(),
            )?;
            let item = item_from_row(&row, id, &name)
                .with_context(|| format!("invalid item {name:?} ({id})"))?;
            items.push(item);
        }
        Ok(items)
    }

    // Listener items live on every node (any node can receive listener traffic
    // and forward to the host owner), so we skip the monitored_by filter but
    // still need the host's monitored_by per-row to attach to ItemData.
    async fn listener_item_query(
        &self,
        tx: &Transaction<'_>,
        item_id: Option<Expr>,
        limit: u64,
        order_bys: Vec<(Expr, Order)>,
        filters: Vec<Expr>,
    ) -> Result<Vec<(Item, Option<Uuid>)>> {
        let mut query_params = query::ItemParams::new();
        query_params
            .with_columns(vec![
                query::ItemColumn::ItemId,
                query::ItemColumn::ResolvedName,
                query::ItemColumn::EndpointId,
                query::ItemColumn::HostId,
                query::ItemColumn::MonitoredBy,
                query::ItemColumn::ResolvedSchedule,
                query::ItemColumn::ListenerId,
                query::ItemColumn::ResolvedTimeoutMs,
                query::ItemColumn::ResolvedOutputs,
                query::ItemColumn::ResolvedConfigEmbed,
            ])
            .with_only_enabled_resolved(true)
            .with_only_monitored(true)
            .with_order_bys(order_bys)
            .with_limit(limit)
            .with_additional_filters(filters);
        if let Some(item_id) = item_id {
            query_params.with_id_subquery_filter(Some(item_id));
        }
        let query = query::items(query_params);
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        trace!(sql = %sql, values = ?values, "listener item query");
        let rows = tx
            .query(&sql, &values.as_params())
            .await
            .context("failed to query listener items")?;
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let (id, name) = row_identity(
                &row,
                query::ItemColumn::ItemId.as_str(),
                query::ItemColumn::ResolvedName.as_str(),
            )?;
            let item = item_from_row(&row, id, &name)
                .and_then(|item| {
                    let monitored_by: Option<Uuid> = row
                        .try_get(query::ItemColumn::MonitoredBy.as_str())
                        .context("failed to get monitored_by")?;
                    Ok((item, monitored_by))
                })
                .with_context(|| format!("invalid listener item {name:?} ({id})"))?;
            items.push(item);
        }
        Ok(items)
    }

    async fn listener_endpoint_query(
        &self,
        tx: &Transaction<'_>,
        endpoint_id: Option<Expr>,
    ) -> Result<Vec<(Uuid, IpAddr)>> {
        let mut query_params = query::EndpointParams::new();
        query_params
            .with_columns(vec![
                query::EndpointColumn::EndpointId,
                query::EndpointColumn::ConfigEmbed,
            ])
            .with_id_subquery_filter(endpoint_id);
        let query = query::endpoints(query_params);
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        trace!(sql = %sql, values = ?values, "listener endpoint query");
        let rows = tx
            .query(&sql, &values.as_params())
            .await
            .context("failed to query listener endpoints")?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: Uuid = row
                .try_get(query::EndpointColumn::EndpointId.as_str())
                .context("failed to get endpoint id")?;
            let config = endpoint_config_from_row(&row)
                .with_context(|| format!("invalid listener endpoint {id}"))?;
            match config.ip() {
                Some(ip) => out.push((id, ip)),
                None => warn!(
                    endpoint_id = %id,
                    "endpoint address is not an ip, source-ip matching disabled"
                ),
            }
        }
        Ok(out)
    }

    async fn cluster_node_query(
        &self,
        tx: &Transaction<'_>,
        node_id_filter: Option<Expr>,
    ) -> Result<Vec<(Uuid, String)>> {
        let mut select = DbQuery::select();
        select
            .from(schema::ClusterNodes::Table)
            .column(schema::ClusterNodes::NodeId)
            .column(schema::ClusterNodes::Metadata)
            .and_where(Expr::col(schema::ClusterNodes::NodeId).ne(self.node_id));
        if let Some(filter) = node_id_filter {
            select.and_where(Expr::col(schema::ClusterNodes::NodeId).binary(BinOper::In, filter));
        }
        let (sql, values) = select.build_postgres(PostgresQueryBuilder);
        trace!(sql = %sql, values = ?values, "cluster node query");
        let rows = tx
            .query(&sql, &values.as_params())
            .await
            .context("failed to query cluster nodes")?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: Uuid = row.try_get(0).context("failed to get node_id")?;
            let meta = node_metadata_from_row(&row)
                .with_context(|| format!("invalid cluster node {id}"))?;
            out.push((id, meta.advertise_address));
        }
        Ok(out)
    }

    async fn initialize_hosts(&self, tx: &Transaction<'_>) -> Result<()> {
        debug!("initializing hosts");
        let hosts = self.host_query(tx, None).await?;
        {
            let mut guard = self.cache.host.write();
            guard.clear();
            for host in hosts {
                guard.insert(host);
            }
        }
        Ok(())
    }

    async fn initialize_endpoints(&self, tx: &Transaction<'_>) -> Result<()> {
        debug!("initializing endpoints");
        let endpoints = self.endpoint_query(tx, None).await?;
        {
            let mut guard = self.cache.endpoint.write();
            guard.clear();
            for endpoint in endpoints {
                guard.insert(endpoint);
            }
        }
        Ok(())
    }

    /// Register every enabled lookup table and load the file-backed ones,
    /// reusing `loader::init` as local mode does.
    ///
    /// The parse deliberately happens inside the caller's `RepeatableRead`
    /// transaction, as `initialize_processors` already starts sinks and
    /// `initialize_listeners` binds sockets there. Deferring it to the
    /// reloader's next tick would let the node go active with empty tables, and
    /// every lookup missed in that window is already at a sink and
    /// unrecoverable. A long transaction costs vacuum headroom; an empty table
    /// costs data.
    async fn initialize_lookup_tables(&self, tx: &Transaction<'_>) -> Result<()> {
        debug!("initializing lookup tables");
        let tables = self.lookup_table_query(tx, None).await?;
        cache::lookup_table::loader::init(&tables, &self.cache).await?;
        Ok(())
    }

    // TODO: one error shouldn't fail everything, we should just log it and continue.
    async fn initialize_processors(&self, tx: &Transaction<'_>) -> Result<()> {
        debug!("initializing processors");
        let sinks = self.sink_query(tx, None).await?;
        let processors = self.processor_query(tx, None).await?;
        {
            let mut guard = self.cache.processor.write();
            let cache = guard.as_mut().unwrap();
            cache.clear();
            for sink in sinks.clone() {
                cache.insert_sink(sink);
            }
            for processor in processors {
                cache.insert_processor(processor);
            }
            self.cache
                .processor
                .compile(guard)
                .context("failed to compile the processor graph")?;
        }
        debug!("starting sinks");
        self.sinks.close_all_sink().await;
        for sink in sinks {
            let (id, name) = (sink.id, sink.name.clone());
            self.sinks
                .add(sink)
                .await
                .with_context(|| format!("failed to start sink {name:?} ({id})"))?;
        }
        // Sinks deleted while this node was down or not main left their disk
        // data: it never saw them go.
        self.sinks.remove_orphaned_data().await;
        Ok(())
    }

    async fn item_batch_insert(&self, tx: &Transaction<'_>, filter: Option<Expr>) -> Result<()> {
        let limit = 1000;
        let mut last_id = Uuid::nil();
        let mut loaded = 0u64;
        loop {
            let items = self
                .item_query(
                    tx,
                    filter.clone(),
                    limit,
                    vec![(
                        Expr::col((schema::Items::Table, schema::Items::ItemId)),
                        Order::Asc,
                    )],
                    vec![
                        Expr::col((schema::Items::Table, schema::Items::ItemId))
                            .gt(Expr::val(last_id).cast_as("uuid")),
                    ],
                )
                .await?;
            let item_len = items.len();
            loaded += item_len as u64;
            debug!(count = loaded, "items loaded");
            if !items.is_empty() {
                last_id = items.last().unwrap().id;
            }
            {
                let mut guard = self.cache.item.write();
                for item in items {
                    guard.insert(item);
                }
            }
            if item_len < limit as usize {
                break;
            }
        }
        Ok(())
    }

    /// Reload every item this node owns.
    ///
    /// Mark-and-sweep rather than clear-then-fill: this runs on every node-lock
    /// flap, not only at startup, and clearing would throw away the per-item
    /// [`crate::cache::item::Stats`] of every item on the node each time the
    /// database blinked. `begin_reload`/`end_reload` bracket the paginated insert
    /// so unchanged items keep their history and only items that have genuinely
    /// gone away are dropped.
    ///
    /// The sweep runs only on success — `?` on the insert skips it, because
    /// sweeping a partial load would delete items that simply hadn't been read
    /// yet.
    async fn initialize_items(&self, tx: &Transaction<'_>) -> Result<()> {
        debug!("initializing items");
        self.cache.item.write().begin_reload();
        self.item_batch_insert(tx, None).await?;
        self.cache.item.write().end_reload();
        Ok(())
    }

    async fn listener_item_batch_insert(
        &self,
        tx: &Transaction<'_>,
        filter: Option<Expr>,
    ) -> Result<()> {
        let limit = 1000;
        let mut last_id = Uuid::nil();
        let mut loaded = 0u64;
        loop {
            let items = self
                .listener_item_query(
                    tx,
                    filter.clone(),
                    limit,
                    vec![(
                        Expr::col((schema::Items::Table, schema::Items::ItemId)),
                        Order::Asc,
                    )],
                    vec![
                        Expr::col((schema::Items::Table, schema::Items::ItemId))
                            .gt(Expr::val(last_id).cast_as("uuid")),
                    ],
                )
                .await?;
            let item_len = items.len();
            loaded += item_len as u64;
            debug!(count = loaded, "listener items loaded");
            if let Some((last, _)) = items.last() {
                last_id = last.id;
            }
            {
                let mut guard = self.cache.listener.write();
                for (item, monitored_by) in items {
                    guard.insert_item(item, monitored_by);
                }
            }
            if item_len < limit as usize {
                break;
            }
        }
        Ok(())
    }

    async fn initialize_listener_items_and_endpoints(&self, tx: &Transaction<'_>) -> Result<()> {
        debug!("initializing listener items and endpoints");
        let endpoints = self.listener_endpoint_query(tx, None).await?;
        {
            let mut guard = self.cache.listener.write();
            for (endpoint_id, ip) in endpoints {
                guard.insert_ip(endpoint_id, ip);
            }
        }
        self.listener_item_batch_insert(tx, None).await?;
        Ok(())
    }

    async fn initialize_listeners(&self, tx: &Transaction<'_>) -> Result<()> {
        debug!("initializing listeners");
        let listeners = self.listener_query(tx, None).await?;
        {
            let mut guard = self.cache.listener.write();
            guard.clear();
            for listener in listeners.clone() {
                guard.insert(listener);
            }
        }
        self.listeners.close_all_listener().await;
        for listener in listeners {
            let (id, name) = (listener.id, listener.name.clone());
            self.listeners
                .add(listener)
                .await
                .with_context(|| format!("failed to start listener {name:?} ({id})"))?;
        }
        Ok(())
    }

    async fn initialize_cluster_nodes(&self, tx: &Transaction<'_>) -> Result<()> {
        debug!("initializing cluster nodes");
        self.inter_comm.close_all().await;
        let peers = self.cluster_node_query(tx, None).await?;
        for (node_id, dst) in peers {
            if let Err(e) = self.inter_comm.add(node_id, dst).await {
                error!(peer_node_id = %node_id, error = %format!("{e:#}"), "failed to add cluster node");
            }
        }
        Ok(())
    }

    async fn clean_all_logs(&self, tx: &Transaction<'_>) -> Result<()> {
        let mut query = DbQuery::delete();
        query.from_table(schema::ClusterLogs::Table).and_where(
            schema::ClusterLogs::NodeId
                .into_column_ref()
                .eq(self.node_id),
        );
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        let deleted_rows = tx
            .execute(&sql, &values.as_params())
            .await
            .context("failed to clean cluster logs")?;
        debug!(deleted = deleted_rows, "cluster logs deleted");
        Ok(())
    }

    async fn initialize(&mut self) -> Result<(), ConfigWatcherError> {
        debug!("initializing configuration cache");
        let mut client = self
            .db_pool
            .get()
            .await
            .context("failed to get db connection")?;
        let tx = client
            .transaction()
            .await
            .context("failed to start db transaction")?;
        tx.execute(
            "UPDATE cluster_nodes SET op_status = 2, name = $2 WHERE node_id = $1",
            &[&self.node_id, &self.node_name],
        )
        .await
        .context("failed to update cluster node status to initializing")?;
        tx.commit()
            .await
            .context("failed to commit db transaction")?;
        if !self.node_lock.is_main() {
            return Err(ConfigWatcherError::NodeLockLost);
        }
        let tx = client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .start()
            .await
            .context("failed to start db transaction")?;
        self.clean_all_logs(&tx)
            .await
            .context("failed to clean all logs")?;
        // Before the processors: a lookup processor resolves against the registry,
        // so the tables have to be there by the time a processor compiles.
        self.initialize_lookup_tables(&tx)
            .await
            .context("failed to load lookup tables")?;
        if !self.node_lock.is_main() {
            return Err(ConfigWatcherError::NodeLockLost);
        }
        self.initialize_processors(&tx)
            .await
            .context("failed to load processors and sinks")?;
        if !self.node_lock.is_main() {
            return Err(ConfigWatcherError::NodeLockLost);
        }
        self.initialize_listeners(&tx)
            .await
            .context("failed to load listeners")?;
        if !self.node_lock.is_main() {
            return Err(ConfigWatcherError::NodeLockLost);
        }
        self.initialize_hosts(&tx)
            .await
            .context("failed to load hosts")?;
        if !self.node_lock.is_main() {
            return Err(ConfigWatcherError::NodeLockLost);
        }
        self.initialize_endpoints(&tx)
            .await
            .context("failed to load endpoints")?;
        if !self.node_lock.is_main() {
            return Err(ConfigWatcherError::NodeLockLost);
        }
        self.initialize_items(&tx)
            .await
            .context("failed to load items")?;
        if !self.node_lock.is_main() {
            return Err(ConfigWatcherError::NodeLockLost);
        }
        self.initialize_listener_items_and_endpoints(&tx)
            .await
            .context("failed to load listener items and endpoints")?;
        if !self.node_lock.is_main() {
            return Err(ConfigWatcherError::NodeLockLost);
        }
        self.initialize_cluster_nodes(&tx)
            .await
            .context("failed to load cluster nodes")?;
        if !self.node_lock.is_main() {
            return Err(ConfigWatcherError::NodeLockLost);
        }
        self.read_monitor_config(&tx)
            .await
            .context("failed to load the monitor config")?;
        if !self.node_lock.is_main() {
            return Err(ConfigWatcherError::NodeLockLost);
        }
        tx.commit()
            .await
            .context("failed to commit db transaction")?;
        let tx = client
            .transaction()
            .await
            .context("failed to start db transaction")?;
        tx.execute(
            "UPDATE cluster_nodes SET op_status = 1, name = $2 WHERE node_id = $1",
            &[&self.node_id, &self.node_name],
        )
        .await
        .context("failed to update cluster node status to active")?;
        tx.commit()
            .await
            .context("failed to commit db transaction")?;
        // What an operator actually wants after a (re)start: how much config this node picked up.
        // The per-resource steps above are debug; this is the one INFO line for the whole load.
        info!(
            hosts = self.cache.host.read().len(),
            endpoints = self.cache.endpoint.read().len(),
            items = self.cache.item.read().len(),
            listeners = self.cache.listener.read().len(),
            "configuration cache loaded"
        );
        Ok(())
    }

    async fn get_delete_ids(
        &self,
        tx: &Transaction<'_>,
        resource: schema::ClusterLogResource,
    ) -> Result<Vec<Uuid>> {
        let mut params = query::ClusterLogParams::new(self.node_id);
        params
            .with_columns(vec![schema::ClusterLogs::ResourceId])
            .with_resources(vec![resource])
            .with_op(schema::ClusterLogsOp::Delete);
        let query = query::cluster_logs(params);
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        let rows = tx
            .query(&sql, &values.as_params())
            .await
            .context("failed to retrieve deleted IDs query")?;
        let delete_ids: Vec<Uuid> = rows
            .into_iter()
            .map(|row| row.try_get(0))
            .collect::<Result<_, _>>()
            .context("failed to parse deleted IDs")?;
        Ok(delete_ids)
    }

    fn get_upsert_ids_subquery(&self, resource: schema::ClusterLogResource) -> Expr {
        let mut params = query::ClusterLogParams::new(self.node_id);
        params
            .with_columns(vec![schema::ClusterLogs::ResourceId])
            .with_resources(vec![resource])
            .with_op(schema::ClusterLogsOp::Upsert);
        query::cluster_logs(params)
            .into_sub_query_statement()
            .into()
    }

    /// Apply the lookup-table deltas. No eager load, unlike
    /// [`Self::initialize_lookup_tables`]: a table that has only just appeared
    /// had nothing resolving against it a moment ago, and this runs every poll.
    async fn read_lookup_table_updates(&self, tx: &Transaction<'_>) -> Result<()> {
        let tables = self
            .lookup_table_query(
                tx,
                Some(self.get_upsert_ids_subquery(schema::ClusterLogResource::LookupTable)),
            )
            .await?;
        let delete_tables = self
            .get_delete_ids(tx, schema::ClusterLogResource::LookupTable)
            .await?;
        for id in &delete_tables {
            self.cache.lookup_table.remove(id);
        }
        for table in &tables {
            // `upsert`, not `insert`: an edit to a column the runtime does not
            // read still logs an upsert, and rebuilding would throw away a
            // loaded generation for nothing.
            if let Err(e) = self.cache.lookup_table.upsert(table) {
                error!(lookup_table_id = %table.id, lookup_table = %table.name, error = %format!("{e:#}"), "failed to build lookup table");
            }
        }
        if !tables.is_empty() || !delete_tables.is_empty() {
            info!(
                added = tables.len(),
                removed = delete_tables.len(),
                "lookup table config applied"
            );
        }
        Ok(())
    }

    async fn read_processor_updates(&self, tx: &Transaction<'_>) -> Result<()> {
        let processors = self
            .processor_query(
                tx,
                Some(self.get_upsert_ids_subquery(schema::ClusterLogResource::Processor)),
            )
            .await?;
        let sinks = self
            .sink_query(
                tx,
                Some(self.get_upsert_ids_subquery(schema::ClusterLogResource::Sink)),
            )
            .await?;
        let delete_processors = self
            .get_delete_ids(tx, schema::ClusterLogResource::Processor)
            .await?;
        let delete_sinks = self
            .get_delete_ids(tx, schema::ClusterLogResource::Sink)
            .await?;
        let (n_processors, n_sinks) = (processors.len(), sinks.len());
        let (n_del_processors, n_del_sinks) = (delete_processors.len(), delete_sinks.len());
        {
            let mut guard = self.cache.processor.write();
            let cache = guard.as_mut().unwrap();
            for processor in processors {
                cache.insert_processor(processor);
            }
            for sink in sinks.clone() {
                cache.insert_sink(sink);
            }
            for id in delete_processors {
                cache.remove_processor(id);
            }
            for id in delete_sinks.clone() {
                cache.remove_sink(id);
            }
            self.cache
                .processor
                .compile(guard)
                .context("failed to compile the processor graph")?;
        }
        for id in delete_sinks {
            self.sinks
                .close_sink(id)
                .await
                .with_context(|| format!("failed to remove sink {id}"))?;
        }
        for sink in sinks {
            let (id, name) = (sink.id, sink.name.clone());
            self.sinks
                .add(sink)
                .await
                .with_context(|| format!("failed to start sink {name:?} ({id})"))?;
        }
        if n_processors + n_del_processors > 0 {
            info!(
                added = n_processors,
                removed = n_del_processors,
                "processor config applied"
            );
        }
        if n_sinks + n_del_sinks > 0 {
            info!(
                added = n_sinks,
                removed = n_del_sinks,
                "sink config applied"
            );
        }
        Ok(())
    }

    async fn read_listener_updates(&self, tx: &Transaction<'_>) -> Result<()> {
        let listeners = self
            .listener_query(
                tx,
                Some(self.get_upsert_ids_subquery(schema::ClusterLogResource::Listener)),
            )
            .await?;
        let delete_listeners = self
            .get_delete_ids(tx, schema::ClusterLogResource::Listener)
            .await?;
        let delete_listener_endpoints = self
            .get_delete_ids(tx, schema::ClusterLogResource::ListenerEndpoint)
            .await?;
        let listener_endpoints = self
            .listener_endpoint_query(
                tx,
                Some(self.get_upsert_ids_subquery(schema::ClusterLogResource::ListenerEndpoint)),
            )
            .await?;
        let delete_listener_items = self
            .get_delete_ids(tx, schema::ClusterLogResource::ListenerItem)
            .await?;
        {
            let mut guard = self.cache.listener.write();
            for listener in listeners.clone() {
                guard.insert(listener);
            }
            for id in &delete_listeners {
                guard.remove(id);
            }
            for id in &delete_listener_endpoints {
                guard.remove_ip(id);
            }
            for (endpoint_id, ip) in listener_endpoints {
                guard.insert_ip(endpoint_id, ip);
            }
            for id in &delete_listener_items {
                guard.remove_item(id);
            }
        }
        // listener_item upserts are paginated; do them outside the write lock so
        // each page acquires the lock briefly.
        self.listener_item_batch_insert(
            tx,
            Some(self.get_upsert_ids_subquery(schema::ClusterLogResource::ListenerItem)),
        )
        .await?;
        let (n_listeners, n_del_listeners) = (listeners.len(), delete_listeners.len());
        for id in delete_listeners {
            self.listeners
                .close_listener(id)
                .await
                .with_context(|| format!("failed to remove listener {id}"))?;
        }
        for listener in listeners {
            let (id, name) = (listener.id, listener.name.clone());
            self.listeners
                .add(listener)
                .await
                .with_context(|| format!("failed to start listener {name:?} ({id})"))?;
        }
        if n_listeners + n_del_listeners > 0 {
            info!(
                added = n_listeners,
                removed = n_del_listeners,
                "listener config applied"
            );
        }
        Ok(())
    }

    async fn read_item_updates(&self, tx: &Transaction<'_>) -> Result<()> {
        let delete_items = self
            .get_delete_ids(tx, schema::ClusterLogResource::Item)
            .await?;
        let n_del_items = delete_items.len();
        {
            let mut guard = self.cache.item.write();
            for item in delete_items {
                guard.remove(item);
            }
        }
        let delete_endpoints = self
            .get_delete_ids(tx, schema::ClusterLogResource::HostEndpoint)
            .await?;
        let n_del_endpoints = delete_endpoints.len();
        {
            let mut guard = self.cache.endpoint.write();
            for endpoint in delete_endpoints {
                guard.remove(&endpoint);
            }
        }
        let delete_hosts = self
            .get_delete_ids(tx, schema::ClusterLogResource::Host)
            .await?;
        let hosts = self
            .host_query(
                tx,
                Some(self.get_upsert_ids_subquery(schema::ClusterLogResource::Host)),
            )
            .await?;
        let (n_hosts, n_del_hosts) = (hosts.len(), delete_hosts.len());
        {
            let mut guard = self.cache.host.write();
            for host in hosts {
                guard.insert(host);
            }
            for host in delete_hosts {
                guard.remove(&host);
            }
        }
        let endpoints = self
            .endpoint_query(
                tx,
                Some(self.get_upsert_ids_subquery(schema::ClusterLogResource::HostEndpoint)),
            )
            .await?;
        let n_endpoints = endpoints.len();
        {
            let mut guard = self.cache.endpoint.write();
            for endpoint in endpoints {
                guard.insert(endpoint);
            }
        }
        self.item_batch_insert(
            tx,
            Some(self.get_upsert_ids_subquery(schema::ClusterLogResource::Item)),
        )
        .await?;
        // Item upserts are paginated inside `item_batch_insert`, so only the removals are counted
        // here; `added` covers hosts and endpoints, which are applied above.
        if n_hosts + n_del_hosts + n_endpoints + n_del_endpoints + n_del_items > 0 {
            info!(
                hosts_added = n_hosts,
                hosts_removed = n_del_hosts,
                endpoints_added = n_endpoints,
                endpoints_removed = n_del_endpoints,
                items_removed = n_del_items,
                "host config applied"
            );
        }
        Ok(())
    }

    async fn read_cluster_node_updates(&self, tx: &Transaction<'_>) -> Result<()> {
        let upserts = self
            .cluster_node_query(
                tx,
                Some(self.get_upsert_ids_subquery(schema::ClusterLogResource::ClusterNode)),
            )
            .await?;
        let deletes = self
            .get_delete_ids(tx, schema::ClusterLogResource::ClusterNode)
            .await?;
        let (n_upserts, n_deletes) = (upserts.len(), deletes.len());
        for id in deletes {
            self.inter_comm.remove(id).await;
        }
        for (id, dst) in upserts {
            if let Err(e) = self.inter_comm.add(id, dst).await {
                error!(peer_node_id = %id, error = %format!("{e:#}"), "failed to add cluster node");
            }
        }
        if n_upserts + n_deletes > 0 {
            info!(
                added = n_upserts,
                removed = n_deletes,
                "cluster node config applied"
            );
        }
        Ok(())
    }

    /// Re-read `configs.monitor` and apply it to the monitor pool. It has no
    /// cluster log: one row, compared by value, is cheap enough to read every
    /// poll, and an edit made by hand applies even if `last_updated` is not
    /// touched. A missing row is left alone, `Cluster::new` seeds it.
    async fn read_monitor_config(&self, tx: &Transaction<'_>) -> Result<()> {
        match super::read_config(tx, schema::ConfigKeys::Monitor, MonitorConfig::validate).await? {
            None => {}
            Some(Ok(config)) => {
                let (worker_count, batch) = (config.worker_count, config.batch);
                if self.monitors.apply_config(config).await {
                    info!(
                        worker_count,
                        batch_size = batch.batch_size,
                        batch_timeout_ms = batch.batch_timeout.as_millis() as u64,
                        "monitor config applied"
                    );
                }
            }
            Some(Err(e)) => {
                error!(error = %format!("{e:#}"), "invalid monitor config, keeping the last good one")
            }
        }
        Ok(())
    }

    async fn delta_read(&mut self) -> Result<(), ConfigWatcherError> {
        // Fires every `watcher_interval` whether or not anything changed, so it is trace; the
        // `* config applied` events below are the ones that mean something happened.
        trace!("reading configuration changes");
        let mut client = self
            .db_pool
            .get()
            .await
            .context("failed to get db connection")?;
        let tx = client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .start()
            .await
            .context("failed to start db transaction")?;
        // we don't terminate with node_lock until everything finished.
        // aborting in the middle will corrupt the cache.
        // lookup tables first, for the same reason as in `initialize`: a
        // processor may resolve against them.
        self.read_lookup_table_updates(&tx)
            .await
            .context("failed to apply lookup table changes")?;
        self.read_processor_updates(&tx)
            .await
            .context("failed to apply processor and sink changes")?;
        self.read_listener_updates(&tx)
            .await
            .context("failed to apply listener changes")?;
        self.read_item_updates(&tx)
            .await
            .context("failed to apply host, endpoint and item changes")?;
        self.read_cluster_node_updates(&tx)
            .await
            .context("failed to apply cluster node changes")?;
        self.read_monitor_config(&tx)
            .await
            .context("failed to apply the monitor config")?;
        self.clean_all_logs(&tx)
            .await
            .context("failed to clean all logs")?;
        tx.commit()
            .await
            .context("failed to commit db transaction")?;
        Ok(())
    }

    async fn poll(&mut self) -> Result<(), ConfigWatcherError> {
        if self.initialized {
            self.delta_read().await?;
        } else {
            self.initialize().await?;
            self.initialized = true;
        };
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(node_id = %self.node_id, node = %self.node_name))]
    pub async fn run(mut self) {
        'root_loop: loop {
            let mut cache_cleared = false;
            loop {
                if !self.node_lock.is_main() {
                    if self.node_lock.is(NodeStateType::Standby) && !cache_cleared {
                        info!(
                            "node entered standby mode, clearing configuration, listeners, and sinks"
                        );
                        {
                            let mut guard = self.cache.listener.write();
                            guard.clear();
                        }
                        self.listeners.close_all_listener().await;
                        {
                            let mut guard = self.cache.item.write();
                            guard.clear();
                        }
                        {
                            let mut guard = self.cache.endpoint.write();
                            guard.clear();
                        }
                        {
                            let mut guard = self.cache.host.write();
                            guard.clear();
                        }
                        {
                            let mut guard = self.cache.processor.write();
                            guard.as_mut().unwrap().clear();
                            if let Err(e) = self.cache.processor.compile(guard) {
                                error!(error = %format!("{e:#}"), "failed to compile processor cache");
                            }
                        }
                        // Otherwise this node keeps answering lookups from data
                        // it is no longer responsible for, and its reloader
                        // keeps re-reading the source files to maintain it.
                        self.cache.lookup_table.clear();
                        self.sinks.close_all_sink().await;
                        self.inter_comm.close_all().await;
                        cache_cleared = true;
                    }
                    tokio::select! {
                        _ = self.token.cancelled() => return,
                        _ = sleep(Duration::from_secs(1)) => {},
                    }
                    continue;
                }
                break;
            }
            info!("starting config watcher");
            loop {
                if !self.node_lock.is_main() {
                    warn!("node lock is lost, config will not update");
                    self.initialized = false;
                    continue 'root_loop;
                }
                let initializing = !self.initialized;
                if let Err(ConfigWatcherError::Other(e)) = self.poll().await {
                    // `{:#}` renders the whole anyhow chain: the step, the resource, then
                    // the cause. Both roll back and the next poll retries the same work.
                    let error = format!("{e:#}");
                    let key_id = e
                        .chain()
                        .find_map(|c| c.downcast_ref::<SecretError>())
                        .and_then(SecretError::key_id);
                    if initializing {
                        error!(%error, key_id, "failed to load the configuration, retrying");
                    } else {
                        error!(%error, key_id, "failed to apply configuration changes, retrying");
                    }
                }
                tokio::select! {
                    _ = self.token.cancelled() => return,
                    // TODO: can be improved with a "nudge" scheme. if nudge is enabled,
                    //  we can poll less frequently.
                    _ = sleep(self.poll_interval) => {},
                }
            }
        }
    }
}

/// A row's id and name, read before the rest so that whatever else in it
/// fails can say which resource it was.
fn row_identity(row: &Row, id_column: &str, name_column: &str) -> Result<(Uuid, String)> {
    let id = row
        .try_get(id_column)
        .with_context(|| format!("failed to get {id_column}"))?;
    let name = row
        .try_get(name_column)
        .with_context(|| format!("failed to get {name_column} of {id}"))?;
    Ok((id, name))
}

fn sink_from_row(row: &Row, id: Uuid, name: String, keys: &SecretKeys) -> Result<Sink> {
    let config: serde_json::Value = row
        .try_get(query::SinkColumn::ConfigEmbed.as_str())
        .context("failed to get config")?;
    let mut config: SinkConfig =
        serde_json::from_value(config).context("failed to deserialize sink config")?;
    config
        .decrypt_all(keys)
        .context("failed to decrypt sink config")?;
    let worker_count = row
        .try_get::<_, i32>(query::SinkColumn::WorkerCount.as_str())
        .context("failed to get worker_count")? as usize;
    let batch = Batch {
        batch_size: row
            .try_get::<_, i32>(query::SinkColumn::BatchSize.as_str())
            .context("failed to get batch_size")? as usize,
        batch_timeout: Duration::from_millis(
            row.try_get::<_, i32>(query::SinkColumn::BatchTimeoutMs.as_str())
                .context("failed to get batch_timeout_ms")? as u64,
        ),
    };
    let channel: serde_json::Value = row
        .try_get(query::SinkColumn::Channel.as_str())
        .context("failed to get channel")?;
    let channel: Channel =
        serde_json::from_value(channel).context("failed to deserialize channel config")?;
    let retry: serde_json::Value = row
        .try_get(query::SinkColumn::Retry.as_str())
        .context("failed to get retry")?;
    let retry: Retry =
        serde_json::from_value(retry).context("failed to deserialize retry config")?;
    let mut sink = Sink {
        id,
        name,
        batch,
        channel,
        retry,
        worker_count,
        inputs: row
            .try_get(query::SinkColumn::Inputs.as_str())
            .context("failed to get inputs")?,
        config,
    };
    sink.validate()?;
    Ok(sink)
}

fn processor_from_row(row: &Row, id: Uuid, name: String) -> Result<Processor> {
    let config: serde_json::Value = row
        .try_get(query::ProcessorColumn::ConfigEmbed.as_str())
        .context("failed to get config")?;
    let config: Config =
        serde_json::from_value(config).context("failed to deserialize processor config")?;
    let mut processor = Processor {
        id,
        name,
        config,
        inputs: row
            .try_get(query::ProcessorColumn::Inputs.as_str())
            .context("failed to get inputs")?,
    };
    processor.validate()?;
    Ok(processor)
}

fn listener_from_row(row: &Row, id: Uuid, name: &str, keys: &SecretKeys) -> Result<Listener> {
    let config: serde_json::Value = row
        .try_get(query::ListenerColumn::ConfigEmbed.as_str())
        .context("failed to get config")?;
    let mut config: ListenerConfig =
        serde_json::from_value(config).context("failed to deserialize listener config")?;
    config
        .decrypt_all(keys)
        .context("failed to decrypt listener config")?;
    let worker_count = row
        .try_get::<_, i32>(query::ListenerColumn::WorkerCount.as_str())
        .context("failed to get worker_count")? as usize;
    let batch = Batch {
        batch_size: row
            .try_get::<_, i32>(query::ListenerColumn::BatchSize.as_str())
            .context("failed to get batch_size")? as usize,
        batch_timeout: Duration::from_millis(
            row.try_get::<_, i32>(query::ListenerColumn::BatchTimeoutMs.as_str())
                .context("failed to get batch_timeout_ms")? as u64,
        ),
    };
    let mut listener = Listener::new(
        id,
        name.to_smolstr(),
        worker_count,
        batch,
        config,
        row.try_get(query::ListenerColumn::DisableClusterForwarding.as_str())
            .context("failed to get disable_cluster_forwarding")?,
    );
    listener.validate()?;
    Ok(listener)
}

fn host_from_row(row: &Row, id: Uuid, name: &str) -> Result<Host> {
    let mut host = Host {
        id,
        name: name.to_smolstr(),
        timezone: row
            .try_get::<_, Option<String>>(query::HostColumn::Timezone.as_str())
            .context("failed to get timezone")?
            .map(|tz| tz.to_smolstr()),
    };
    host.validate()?;
    Ok(host)
}

fn endpoint_from_row(row: &Row, id: Uuid, name: &str, keys: &SecretKeys) -> Result<Endpoint> {
    let config: serde_json::Value = row
        .try_get(query::EndpointColumn::ConfigEmbed.as_str())
        .context("failed to get config")?;
    let mut config: EndpointConfig =
        serde_json::from_value(config).context("failed to deserialize endpoint config")?;
    config
        .decrypt_all(keys)
        .context("failed to decrypt endpoint config")?;
    let mut endpoint = Endpoint::new(
        id,
        name.to_smolstr(),
        row.try_get(query::EndpointColumn::HostId.as_str())
            .context("failed to get host_id")?,
        config,
    );
    endpoint.validate()?;
    Ok(endpoint)
}

fn endpoint_config_from_row(row: &Row) -> Result<EndpointConfig> {
    let config: serde_json::Value = row
        .try_get(query::EndpointColumn::ConfigEmbed.as_str())
        .context("failed to get config")?;
    let mut config: EndpointConfig =
        serde_json::from_value(config).context("failed to deserialize endpoint config")?;
    config.validate()?;
    Ok(config)
}

/// The same for both item queries: [`ConfigWatcher::item_query`] and
/// [`ConfigWatcher::listener_item_query`] select these columns alike.
fn item_from_row(row: &Row, id: Uuid, name: &str) -> Result<Item> {
    let config: serde_json::Value = row
        .try_get(query::ItemColumn::ResolvedConfigEmbed.as_str())
        .context("failed to get config")?;
    let config: ItemConfig =
        serde_json::from_value(config).context("failed to deserialize item config")?;
    let schedule: String = row
        .try_get(query::ItemColumn::ResolvedSchedule.as_str())
        .context("failed to get schedule")?;
    let schedule: Schedule = schedule
        .as_str()
        .parse()
        .with_context(|| format!("failed to parse schedule {schedule:?}"))?;
    let timeout: i32 = row
        .try_get(query::ItemColumn::ResolvedTimeoutMs.as_str())
        .context("failed to get timeout")?;
    let mut item = Item::new(
        id,
        name.to_smolstr(),
        row.try_get(query::ItemColumn::EndpointId.as_str())
            .context("failed to get endpoint_id")?,
        row.try_get(query::ItemColumn::HostId.as_str())
            .context("failed to get host_id")?,
        schedule,
        row.try_get(query::ItemColumn::ListenerId.as_str())
            .context("failed to get listener_id")?,
        Duration::from_millis(timeout as u64),
        row.try_get(query::ItemColumn::ResolvedOutputs.as_str())
            .context("failed to get outputs")?,
        config,
    );
    item.validate()?;
    Ok(item)
}

fn node_metadata_from_row(row: &Row) -> Result<NodeMetadata> {
    let raw: serde_json::Value = row.try_get(1).context("failed to get node metadata")?;
    let meta: NodeMetadata =
        serde_json::from_value(raw).context("failed to deserialize node metadata")?;
    meta.validate()?;
    Ok(meta)
}
