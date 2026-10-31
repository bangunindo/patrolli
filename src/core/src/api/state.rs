// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::jwt::Jwt;
use crate::cache::Cache;
use crate::cluster::inter::InterComm;
use crate::config::sensitive::SecretKeys;
use deadpool_postgres::Pool;
use std::sync::Arc;

/// AppState holds all shared state for the API server
/// This includes the database connection pool, cache, and any custom application state
#[derive(Clone)]
pub struct AppState {
    /// Database connection pool for accessing PostgreSQL
    pub db_pool: Pool,
    /// Application cache containing hosts, endpoints, items, etc.
    pub cache: Cache,
    /// Inter-node comm layer, used for cluster-wide operations like live
    /// processor capture
    pub inter_comm: Arc<InterComm>,
    /// Token signing/verification and the auth cookie settings
    pub jwt: Arc<Jwt>,
    pub secret_keys: Arc<SecretKeys>,
}

impl AppState {
    pub fn new(
        db_pool: Pool,
        cache: Cache,
        inter_comm: Arc<InterComm>,
        jwt: Arc<Jwt>,
        secret_keys: Arc<SecretKeys>,
    ) -> Self {
        Self {
            db_pool,
            cache,
            inter_comm,
            jwt,
            secret_keys,
        }
    }
}
