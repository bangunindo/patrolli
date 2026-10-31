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
use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use sea_query::{PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use smol_str::ToSmolStr;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateListenerRequest {
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

impl CreateListenerRequest {
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
pub struct CreateListenerResponse {
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

/// Create a listener
#[utoipa::path(
    post,
    path = "/listeners",
    request_body(
        content = CreateListenerRequest,
        description = "Create listener request body",
        content_type = "application/json",
    ),
    responses(
        (status = 201, description = "Listener created", body = ResponseData<CreateListenerResponse>),
        (status = 400, description = "Validation error", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "listeners",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn create_listener(
    State(state): State<Arc<AppState>>,
    Json(mut payload): Json<CreateListenerRequest>,
) -> Result<ResponseData<CreateListenerResponse>, ResponseError> {
    // check secrets
    payload.config.check_new().bad_request("Validation error")?;

    payload.validate().bad_request("Validation error")?;

    // encrypt secrets
    payload
        .config
        .encrypt_all(&state.secret_keys)
        .ctx("encrypt listener config")?;

    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    let listener_id = Uuid::now_v7();
    let now = Utc::now();
    let (sql, values) = DbQuery::insert()
        .into_table(Listeners::Table)
        .columns([
            Listeners::ListenerId,
            Listeners::Name,
            Listeners::Type,
            Listeners::AdmStatus,
            Listeners::Labels,
            Listeners::LastUpdated,
            Listeners::WorkerCount,
            Listeners::BatchSize,
            Listeners::BatchTimeoutMs,
            Listeners::Config,
            Listeners::DisableClusterForwarding,
        ])
        .values([
            listener_id.into(),
            payload.name.clone().into(),
            ListenerKind::from(&payload.config).to_string().into(),
            (payload.adm_status as i16).into(),
            payload.labels.clone().into(),
            now.into(),
            payload.worker_count.into(),
            (payload.batch.batch_size as i32).into(),
            (payload.batch.batch_timeout.as_millis() as i32).into(),
            payload
                .config
                .to_json_value()
                .ctx("serialize config")?
                .into(),
            payload.disable_cluster_forwarding.into(),
        ])
        .ctx("build insert query")?
        .build_postgres(PostgresQueryBuilder);

    _ = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("execute insert query")?;

    tx.commit().await.ctx("commit transaction")?;

    Ok(
        ResponseData::from_status(StatusCode::CREATED).with_data(CreateListenerResponse {
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
