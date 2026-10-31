// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::config::endpoint::{Config, Endpoint, EndpointKind};
use crate::config::sensitive::{Masked, SensitiveFields};
use crate::db::models::serialize_utc_as_local;
use crate::db::schema::{AdmStatus, HostEndpoints, Hosts, default_adm_status};
use axum::http::StatusCode;
use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use smol_str::ToSmolStr;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateEndpointRequest {
    pub host_id: Uuid,
    pub name: String,
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    pub config: Config,
}

impl CreateEndpointRequest {
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
pub struct CreateEndpointResponse {
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

/// Create an endpoint
#[utoipa::path(
    post,
    path = "/endpoints",
    request_body(
        content = CreateEndpointRequest,
        description = "Create endpoint request body",
        content_type = "application/json",
    ),
    responses(
        (status = 201, description = "Endpoint created", body = ResponseData<CreateEndpointResponse>),
        (status = 400, description = "Validation error", body = ResponseError),
        (status = 404, description = "Host not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "endpoints",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn create_endpoint(
    State(state): State<Arc<AppState>>,
    Json(mut payload): Json<CreateEndpointRequest>,
) -> Result<ResponseData<CreateEndpointResponse>, ResponseError> {
    // check secrets
    payload.config.check_new().bad_request("Validation error")?;

    payload.validate().bad_request("Validation error")?;

    // encrypt secrets
    payload
        .config
        .encrypt_all(&state.secret_keys)
        .ctx("encrypt endpoint config")?;

    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // get host
    let mut query = DbQuery::select();
    query
        .column((Hosts::Table, Hosts::HostId))
        .from(Hosts::Table)
        .and_where(Expr::col((Hosts::Table, Hosts::HostId)).eq(payload.host_id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let row_opt = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("query host")?;

    if row_opt.is_none() {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Host not found")
            .with_detail(format!("Host with id {} not found", payload.host_id)));
    }

    // create endpoint
    let endpoint_id = Uuid::now_v7();
    let now = Utc::now();
    let mut query = DbQuery::insert();
    query
        .into_table(HostEndpoints::Table)
        .columns([
            HostEndpoints::EndpointId,
            HostEndpoints::HostId,
            HostEndpoints::Name,
            HostEndpoints::AdmStatus,
            HostEndpoints::Type,
            HostEndpoints::Config,
            HostEndpoints::LastUpdated,
        ])
        .values([
            endpoint_id.into(),
            payload.host_id.into(),
            payload.name.clone().into(),
            (payload.adm_status as i16).into(),
            EndpointKind::from(&payload.config).to_string().into(),
            payload
                .config
                .to_json_value()
                .ctx("serialize config")?
                .into(),
            now.into(),
        ])
        .ctx("build insert query")?;

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);

    _ = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("execute insert query")?;

    tx.commit().await.ctx("commit transaction")?;

    Ok(
        ResponseData::from_status(StatusCode::CREATED).with_data(CreateEndpointResponse {
            endpoint_id,
            host_id: payload.host_id,
            name: payload.name,
            adm_status: payload.adm_status,
            config: Masked(payload.config),
            last_updated: now,
        }),
    )
}
