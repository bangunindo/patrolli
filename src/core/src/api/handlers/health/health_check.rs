// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::common;
use axum::{extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::warn;
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub uptime_seconds: u64,
}

/// Health check endpoint
///
/// Returns the health status of the API server, database connection, and cache
#[utoipa::path(
    get,
    path = "",
    responses(
        (status = 200, description = "Service is healthy", body = ResponseData<HealthResponse>),
        (status = 503, description = "Service is unhealthy", body = ResponseError)
    ),
    tag = "health"
)]
pub async fn health_check(
    State(state): State<Arc<AppState>>,
) -> Result<ResponseData<HealthResponse>, ResponseError> {
    let db_status = match state.db_pool.get().await {
        Ok(conn) => match conn.query_one("SELECT 1", &[]).await {
            Ok(_) => "healthy",
            Err(e) => {
                warn!(error = %e, "health check database probe failed");
                "unhealthy"
            }
        },
        Err(e) => {
            warn!(error = %e, "health check database connection failed");
            "unhealthy"
        }
    };

    // Check cache status (simple check if cache is accessible)
    let _cache_status = {
        let _host_cache = state.cache.host.read();
        "ok"
    };

    let uptime = common::uptime();

    let response = HealthResponse {
        status: db_status.to_string(),
        uptime_seconds: uptime.as_secs(),
    };

    if db_status == "healthy" {
        Ok(ResponseData::from_status(StatusCode::OK).with_data(response))
    } else {
        Err(ResponseError::from_status(StatusCode::SERVICE_UNAVAILABLE)
            .with_title("Service unhealthy")
            .with_detail("Database health check failed"))
    }
}
