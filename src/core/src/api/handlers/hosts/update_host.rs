// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::models::serialize_utc_as_local;
use crate::db::schema::{AdmStatus, Hosts, default_adm_status};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateHostRequest {
    pub name: String,
    #[schema(value_type = i16)]
    pub adm_status: AdmStatus,
    pub labels: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateHostResponse {
    pub host_id: Uuid,
    pub name: String,
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    pub labels: serde_json::Value,
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub last_updated: DateTime<Utc>,
    pub monitored_by: Option<Uuid>,
}

/// Update a host
///
/// Updates an existing host in the database and cache
#[utoipa::path(
    put,
    path = "/hosts/{id}",
    params(
        ("id" = Uuid, Path, description = "Host UUID")
    ),
    request_body = UpdateHostRequest,
    responses(
        (status = 200, description = "Host updated", body = ResponseData<UpdateHostResponse>),
        (status = 404, description = "Host not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "hosts",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn update_host(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateHostRequest>,
) -> Result<ResponseData<UpdateHostResponse>, ResponseError> {
    if payload.name.trim().is_empty() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Validation error")
            .with_detail("Host name cannot be empty"));
    }

    let client = state.db_pool.get().await.ctx("get database connection")?;

    let mut query = DbQuery::update();
    query
        .table(Hosts::Table)
        .and_where(Expr::col(Hosts::HostId).eq(id))
        .returning(DbQuery::returning().columns([
            Hosts::HostId,
            Hosts::Name,
            Hosts::AdmStatus,
            Hosts::Labels,
            Hosts::LastUpdated,
            Hosts::MonitoredBy,
        ]))
        .value(Hosts::Name, payload.name)
        .value(Hosts::AdmStatus, payload.adm_status as i16)
        .value(Hosts::Labels, payload.labels)
        .value(Hosts::LastUpdated, chrono::Utc::now());

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);

    let row_opt = client
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    let Some(row) = row_opt else {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Host not found")
            .with_detail(format!("No host found for id: {}", id)));
    };

    let host = row_to_db_host(&row).ctx("parse host row")?;

    Ok(ResponseData::from_status(StatusCode::OK).with_data(host))
}

fn row_to_db_host(row: &tokio_postgres::Row) -> Result<UpdateHostResponse, Box<dyn Error>> {
    Ok(UpdateHostResponse {
        host_id: row.try_get(0)?,
        name: row.try_get(1)?,
        adm_status: row.try_get::<_, i16>(2)?.try_into()?,
        labels: row.try_get(3)?,
        last_updated: row.try_get(4)?,
        monitored_by: row.try_get(5)?,
    })
}
