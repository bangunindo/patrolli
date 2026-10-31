// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::config::Batch;
use crate::config::listener::{Config, Listener, ListenerKind};
use crate::config::sensitive::{Masked, SensitiveFields};
use crate::db::models::serialize_utc_as_local;
use crate::db::schema::{AdmStatus, Listeners, default_adm_status};
use axum::http::StatusCode;
use axum::{
    Json,
    extract::{Path, State},
};
use chrono::{DateTime, Utc};
use sea_query::{Expr, ExprTrait, LockType, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use smol_str::ToSmolStr;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateListenerRequest {
    pub name: String,
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    pub labels: serde_json::Value,
    #[serde(default)]
    pub worker_count: u32,
    #[serde(default)]
    pub batch: Batch,
    pub config: Config,
    #[serde(default)]
    pub disable_cluster_forwarding: bool,
}

impl UpdateListenerRequest {
    /// Validates the request and normalizes its batch to the values stored.
    pub fn validate(&mut self) -> anyhow::Result<()> {
        let mut listener = Listener::new(
            Uuid::now_v7(),
            self.name.to_smolstr(),
            self.worker_count as usize,
            self.batch,
            self.config.clone(),
            self.disable_cluster_forwarding,
        );
        listener.validate()?;
        self.batch = listener.batch;

        if self.worker_count == 0 {
            anyhow::bail!("Worker count must be greater than 0");
        }

        if self.name.trim().is_empty() {
            anyhow::bail!("Item name cannot be empty");
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateListenerResponse {
    pub listener_id: Uuid,
    pub name: String,
    #[schema(value_type = i16)]
    pub adm_status: AdmStatus,
    pub labels: serde_json::Value,
    pub worker_count: u32,
    pub batch: Batch,
    #[schema(value_type = Config)]
    pub config: Masked<Config>,
    pub disable_cluster_forwarding: bool,
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub last_updated: DateTime<Utc>,
}

/// Update a listener
#[utoipa::path(
    put,
    path = "/listeners/{listener_id}",
    params(
        ("listener_id" = Uuid, Path, description = "Listener UUID")
    ),
    request_body = UpdateListenerRequest,
    responses(
        (status = 200, description = "Listener updated", body = ResponseData<UpdateListenerResponse>),
        (status = 404, description = "Listener not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "listeners",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn update_listener(
    State(state): State<Arc<AppState>>,
    Path(listener_id): Path<Uuid>,
    Json(mut payload): Json<UpdateListenerRequest>,
) -> Result<ResponseData<UpdateListenerResponse>, ResponseError> {
    payload.validate().bad_request("Validation error")?;

    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    let mut query = DbQuery::select();
    query
        .columns([
            (Listeners::Table, Listeners::Type),
            (Listeners::Table, Listeners::Config),
        ])
        .from(Listeners::Table)
        .and_where(Expr::col((Listeners::Table, Listeners::ListenerId)).eq(listener_id))
        .lock(LockType::Update);

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let row_opt = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("query listener")?;

    let Some(row) = row_opt else {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Listener not found")
            .with_detail(format!("Listener with id {} not found", listener_id)));
    };

    // keep stored secrets
    let stored_type: String = row.try_get(0).ctx("get listener type")?;
    let stored_config: serde_json::Value = row.try_get(1).ctx("get listener config")?;
    payload
        .config
        .keep_stored(&stored_type, stored_config)
        .bad_request("Validation error")?;

    // encrypt secrets
    payload
        .config
        .encrypt_all(&state.secret_keys)
        .ctx("encrypt listener config")?;

    let now = Utc::now();
    let (sql, values) = DbQuery::update()
        .table(Listeners::Table)
        .and_where(Expr::col((Listeners::Table, Listeners::ListenerId)).eq(listener_id))
        .value(Listeners::Name, payload.name.clone())
        .value(
            Listeners::Type,
            ListenerKind::from(&payload.config).to_string(),
        )
        .value(Listeners::AdmStatus, payload.adm_status as i16)
        .value(Listeners::Labels, payload.labels.clone())
        .value(Listeners::WorkerCount, payload.worker_count)
        .value(Listeners::BatchSize, payload.batch.batch_size as i32)
        .value(
            Listeners::BatchTimeoutMs,
            payload.batch.batch_timeout.as_millis() as i32,
        )
        .value(
            Listeners::Config,
            payload.config.to_json_value().ctx("serialize config")?,
        )
        .value(
            Listeners::DisableClusterForwarding,
            payload.disable_cluster_forwarding,
        )
        .value(Listeners::LastUpdated, now)
        .build_postgres(PostgresQueryBuilder);

    let affected = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("execute listener update query")?;

    if affected == 0 {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Listener not found")
            .with_detail(format!(
                "No listener found for listener_id: {}",
                listener_id
            )));
    }

    tx.commit().await.ctx("commit transaction")?;

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(UpdateListenerResponse {
            listener_id,
            name: payload.name,
            adm_status: payload.adm_status,
            labels: payload.labels,
            worker_count: payload.worker_count,
            batch: payload.batch,
            config: Masked(payload.config),
            disable_cluster_forwarding: payload.disable_cluster_forwarding,
            last_updated: now,
        }),
    )
}
