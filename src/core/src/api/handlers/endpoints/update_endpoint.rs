// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::config::endpoint::{Config, Endpoint, EndpointKind};
use crate::config::sensitive::{Masked, SensitiveFields};
use crate::db::models::serialize_utc_as_local;
use crate::db::schema::{AdmStatus, HostEndpoints, default_adm_status};
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
pub struct UpdateEndpointRequest {
    pub host_id: Uuid,
    pub name: String,
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    pub config: Config,
}

impl UpdateEndpointRequest {
    pub fn validate(&self) -> anyhow::Result<()> {
        let mut endpoint = Endpoint::new(
            Uuid::now_v7(), // placeholder
            self.name.to_smolstr(),
            self.host_id,
            self.config.clone(),
        );
        endpoint.validate()?;

        if self.name.trim().is_empty() {
            anyhow::bail!("Endpoint name cannot be empty");
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateEndpointResponse {
    pub endpoint_id: Uuid,
    pub host_id: Uuid,
    pub name: String,
    #[schema(value_type = i16)]
    pub adm_status: AdmStatus,
    #[schema(value_type = Config)]
    pub config: Masked<Config>,
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub last_updated: DateTime<Utc>,
}

/// Update an endpoint
#[utoipa::path(
    put,
    path = "/endpoints/{endpoint_id}",
    params(
        ("endpoint_id" = Uuid, Path, description = "Endpoint UUID")
    ),
    request_body = UpdateEndpointRequest,
    responses(
        (status = 200, description = "Endpoint updated", body = ResponseData<UpdateEndpointResponse>),
        (status = 404, description = "Endpoint not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "endpoints",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn update_endpoint(
    State(state): State<Arc<AppState>>,
    Path(endpoint_id): Path<Uuid>,
    Json(mut payload): Json<UpdateEndpointRequest>,
) -> Result<ResponseData<UpdateEndpointResponse>, ResponseError> {
    payload.validate().bad_request("Validation error")?;

    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // get endpoint
    let mut query = DbQuery::select();
    query
        .columns([HostEndpoints::Type, HostEndpoints::Config])
        .from(HostEndpoints::Table)
        .and_where(Expr::col(HostEndpoints::EndpointId).eq(endpoint_id))
        .lock(LockType::Update);

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let row_opt = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("query endpoint")?;

    let Some(row) = row_opt else {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Endpoint not found")
            .with_detail(format!("Endpoint with id {} not found", endpoint_id)));
    };

    // keep stored secrets
    let stored_type: String = row.try_get(0).ctx("get endpoint type")?;
    let stored_config: serde_json::Value = row.try_get(1).ctx("get endpoint config")?;
    payload
        .config
        .keep_stored(&stored_type, stored_config)
        .bad_request("Validation error")?;

    // encrypt secrets
    payload
        .config
        .encrypt_all(&state.secret_keys)
        .ctx("encrypt endpoint config")?;

    // update endpoint
    let now = Utc::now();
    let (sql, values) = DbQuery::update()
        .table(HostEndpoints::Table)
        .and_where(Expr::col(HostEndpoints::EndpointId).eq(endpoint_id))
        .value(HostEndpoints::HostId, payload.host_id)
        .value(HostEndpoints::Name, payload.name.clone())
        .value(HostEndpoints::AdmStatus, payload.adm_status as i16)
        .value(
            HostEndpoints::Type,
            EndpointKind::from(&payload.config).to_string(),
        )
        .value(
            HostEndpoints::Config,
            payload
                .config
                .to_json_value()
                .ctx("serialize endpoint config")?,
        )
        .value(HostEndpoints::LastUpdated, now)
        .build_postgres(PostgresQueryBuilder);

    let affected = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("execute endpoint update query")?;

    if affected == 0 {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Endpoint not found")
            .with_detail(format!(
                "No endpoint found for endpoint_id: {}",
                endpoint_id
            )));
    }

    tx.commit().await.ctx("commit transaction")?;

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(UpdateEndpointResponse {
            endpoint_id,
            host_id: payload.host_id,
            name: payload.name,
            adm_status: payload.adm_status,
            config: Masked(payload.config),
            last_updated: now,
        }),
    )
}
