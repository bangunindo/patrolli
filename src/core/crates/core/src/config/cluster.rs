// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use anyhow::{Context, Result};
use axum::http::HeaderValue;
use deadpool_postgres::{Config as PostgresConfig, PoolConfig};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::Duration;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Forwarder {
    #[serde(default = "super::default_worker_count")]
    pub worker_count: usize,
    #[serde(flatten, default)]
    pub batch: super::Batch,
    #[serde(default)]
    pub channel: super::Channel,
    #[serde(default)]
    pub compression: super::OtapCompression,
}

impl Forwarder {
    pub fn validate(&mut self) -> Result<()> {
        if self.worker_count == 0 {
            self.worker_count = super::default_worker_count();
        }
        self.batch.validate()?;
        self.channel.validate()?;
        self.channel.validate_no_write_through()?;
        Ok(())
    }
}

impl Default for Forwarder {
    fn default() -> Self {
        default_forwarder()
    }
}

fn default_forwarder() -> Forwarder {
    Forwarder {
        worker_count: super::default_worker_count(),
        batch: super::Batch::default(),
        channel: super::Channel::default(),
        compression: super::OtapCompression::default(),
    }
}

fn default_grpc_address() -> SocketAddr {
    "0.0.0.0:4636".parse().unwrap()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grpc {
    #[serde(default = "default_grpc_address")]
    pub address: SocketAddr,
    pub tls: Option<super::Tls>,
    pub bearer: Option<String>,
}

impl Grpc {
    pub fn validate(&self) -> Result<()> {
        if let Some(tls) = &self.tls {
            tls.validate()?;
            tls.validate_client()?;
            tls.validate_server()?;
        }
        Ok(())
    }
}

impl Default for Grpc {
    fn default() -> Self {
        default_grpc()
    }
}

fn default_grpc() -> Grpc {
    Grpc {
        address: default_grpc_address(),
        tls: None,
        bearer: None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterNode {
    #[serde(default = "default_forwarder")]
    pub forwarder: Forwarder,
    #[serde(default = "default_grpc")]
    pub grpc: Grpc,
}

impl InterNode {
    pub fn validate(&mut self) -> Result<()> {
        self.forwarder.validate()?;
        self.grpc.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
// no deny_unknown_fields: written by each node and read by its peers, which may
// run another version during a rolling upgrade
pub struct NodeMetadata {
    pub advertise_address: String,
}

impl NodeMetadata {
    pub fn validate(&self) -> Result<()> {
        Ok(())
    }
}

impl Default for InterNode {
    fn default() -> Self {
        default_internode()
    }
}

fn default_internode() -> InterNode {
    InterNode {
        grpc: default_grpc(),
        forwarder: default_forwarder(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cluster {
    pub node_id: Uuid,
    pub node_name: String,
    #[serde(
        with = "crate::common::serde::duration_seconds",
        default = "default_heartbeat_timeout"
    )]
    pub heartbeat_timeout: Duration,
    #[serde(
        with = "crate::common::serde::duration_seconds",
        default = "default_heartbeat_interval"
    )]
    pub heartbeat_interval: Duration,
    pub metadata: NodeMetadata,
    #[serde(default = "default_internode")]
    pub inter_node: InterNode,
}

impl Cluster {
    pub fn validate(&mut self) -> Result<()> {
        self.metadata.validate()?;
        self.inter_node.validate()?;
        Ok(())
    }
}

/// Cluster-wide balancing policy, stored in the database under `configs.cluster`
/// and read by the leader every tick.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClusterPolicy {
    /// How many live nodes may carry hosts; the rest are kept empty as spares
    /// that a failed node's hosts move to as one block. `0` means every live node.
    #[serde(default)]
    pub max_active_nodes: usize,
}

impl ClusterPolicy {
    pub fn validate(&mut self) -> Result<()> {
        Ok(())
    }
}

fn default_heartbeat_timeout() -> Duration {
    Duration::from_secs(15)
}

fn default_heartbeat_interval() -> Duration {
    Duration::from_secs(5)
}

fn default_api_addr() -> SocketAddr {
    "0.0.0.0:8080".parse().unwrap()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorsConfig {
    #[serde(default, with = "crate::common::serde::header_value_vec")]
    pub allow_origins: Vec<HeaderValue>,
}

impl CorsConfig {
    pub fn validate(&self) -> Result<()> {
        for origin in &self.allow_origins {
            let _ = origin.to_str().context("invalid CORS allow_origin")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Api {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_api_addr")]
    pub addr: SocketAddr,
    pub tls: Option<super::Tls>,
    pub cors: Option<CorsConfig>,
    #[serde(default)]
    pub auth: BackendAuth,
}

impl Default for Api {
    /// Used when `backend` has no `api` block at all, which means this node does
    /// not serve the API. Within a block that is present, `enabled` defaults to
    /// true instead, since writing the block at all asks for the API.
    fn default() -> Self {
        Self {
            enabled: false,
            addr: default_api_addr(),
            tls: None,
            cors: None,
            auth: BackendAuth::default(),
        }
    }
}

impl Api {
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(tls) = &self.tls {
            tls.validate()?;
            tls.validate_server()?;
        }
        if let Some(cors) = &self.cors {
            cors.validate()?;
        }
        self.auth.validate()?;
        Ok(())
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalAuth {
    /// Used as `iss` claim
    #[serde(default = "default_jwt_issuer")]
    pub jwt_issuer: String,
    #[serde(
        with = "crate::common::serde::duration_seconds",
        default = "default_access_token_ttl"
    )]
    pub access_token_ttl: Duration,
    #[serde(
        with = "crate::common::serde::duration_seconds",
        default = "default_refresh_token_ttl"
    )]
    pub refresh_token_ttl: Duration,
}

impl Default for LocalAuth {
    fn default() -> Self {
        Self {
            jwt_issuer: default_jwt_issuer(),
            access_token_ttl: default_access_token_ttl(),
            refresh_token_ttl: default_refresh_token_ttl(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BackendAuth {
    #[serde(default)]
    pub local: LocalAuth,
    pub oidc: Option<OidcAuth>,
}

impl BackendAuth {
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(oidc) = &self.oidc {
            oidc.validate()?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backend {
    #[serde(default)]
    pub api: Api,
    pub db_url: String,
    pub db_tls: Option<super::Tls>,
    #[serde(default = "default_db_pool_size")]
    pub db_pool_size: usize,
    pub cluster: Cluster,
    #[serde(
        with = "crate::common::serde::duration_seconds",
        default = "default_watcher_interval"
    )]
    pub watcher_interval: Duration,
}

fn default_db_pool_size() -> usize {
    20
}

fn default_watcher_interval() -> Duration {
    Duration::from_secs(5)
}

fn default_jwt_issuer() -> String {
    "patrolli".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcAuth {
    pub issuer: String,
    pub jwks_url: String,
    /// Required `aud` on incoming tokens: the client id the ID token was issued for.
    pub audience: String,
    /// Dot path to the claim that is matched against `users.username`, e.g. `ext.login`.
    pub username_claim_path: String,
    pub tls: Option<super::Tls>,
}

impl OidcAuth {
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.issuer.trim().is_empty(),
            "oidc.issuer must not be empty"
        );
        anyhow::ensure!(
            !self.jwks_url.trim().is_empty(),
            "oidc.jwks_url must not be empty"
        );
        anyhow::ensure!(
            !self.audience.trim().is_empty(),
            "oidc.audience must not be empty"
        );
        anyhow::ensure!(
            !self.username_claim_path.trim().is_empty(),
            "oidc.username_claim_path must not be empty"
        );
        if let Some(tls) = &self.tls {
            tls.validate()?;
            tls.validate_client()?;
        }
        Ok(())
    }
}

fn default_access_token_ttl() -> Duration {
    Duration::from_secs(15 * 60)
}

fn default_refresh_token_ttl() -> Duration {
    Duration::from_secs(7 * 24 * 60 * 60)
}

impl Backend {
    pub fn validate(&mut self) -> anyhow::Result<()> {
        if self.db_pool_size == 0 {
            self.db_pool_size = default_db_pool_size();
        }
        let _ = self.get_db_config()?;
        self.api.validate()?;
        if let Some(tls) = &self.db_tls {
            tls.validate()?;
            tls.validate_client()?;
        }
        self.cluster.validate()?;
        Ok(())
    }

    pub fn get_db_config(&self) -> anyhow::Result<PostgresConfig> {
        let mut config = PostgresConfig::new();
        config.url = Some(self.db_url.clone());
        config.pool = Some(PoolConfig {
            max_size: self.db_pool_size,
            ..Default::default()
        });
        let _ = config.get_pg_config()?;
        Ok(config)
    }
}
