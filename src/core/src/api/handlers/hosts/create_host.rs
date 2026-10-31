// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::models::serialize_utc_as_local;
use crate::db::schema::{AdmStatus, HostTemplates, Hosts, default_adm_status};
use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use sea_query::{PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateHostRequest {
    pub name: String,
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    pub labels: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateHostResponse {
    pub host_id: Uuid,
    pub name: String,
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    pub labels: serde_json::Value,
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub last_updated: DateTime<Utc>,
    pub monitored_by: Option<Uuid>,
}

/// Create a new host
///
/// Creates a new host in the database
#[utoipa::path(
    post,
    path = "/hosts",
    request_body = CreateHostRequest,
    responses(
        (status = 201, description = "Host created", body = ResponseData<CreateHostResponse>),
        (status = 400, description = "Invalid input", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "hosts",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn create_host(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateHostRequest>,
) -> Result<ResponseData<CreateHostResponse>, ResponseError> {
    if payload.name.trim().is_empty() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Validation error")
            .with_detail("Host name cannot be empty"));
    }

    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // create host
    let id = Uuid::now_v7();
    let (sql, values) = DbQuery::insert()
        .into_table(Hosts::Table)
        .columns([
            Hosts::HostId,
            Hosts::Name,
            Hosts::AdmStatus,
            Hosts::Labels,
            Hosts::LastUpdated,
        ])
        .returning(DbQuery::returning().columns([
            Hosts::HostId,
            Hosts::Name,
            Hosts::AdmStatus,
            Hosts::Labels,
            Hosts::LastUpdated,
            Hosts::MonitoredBy,
        ]))
        .values([
            id.into(),
            payload.name.clone().into(),
            (payload.adm_status as i16).into(),
            payload.labels.clone().into(),
            chrono::Utc::now().into(),
        ])
        .ctx("build insert query")?
        .build_postgres(PostgresQueryBuilder);

    let returned_row = tx
        .query_one(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    // create host template
    let host_template_id = Uuid::now_v7();
    let (insert_sql, insert_values) = DbQuery::insert()
        .into_table(HostTemplates::Table)
        .columns([HostTemplates::HostTemplateId, HostTemplates::HostId])
        .values([host_template_id.into(), id.into()])
        .ctx("build insert query for host template")?
        .build_postgres(PostgresQueryBuilder);

    _ = tx
        .execute(insert_sql.as_str(), &insert_values.as_params())
        .await
        .ctx("execute insert query for host template")?;

    let response_payload = row_to_db_host(&returned_row).ctx("parse host row")?;

    tx.commit().await.ctx("commit transaction")?;

    Ok(ResponseData::from_status(StatusCode::CREATED).with_data(response_payload))
}

fn row_to_db_host(row: &tokio_postgres::Row) -> Result<CreateHostResponse, Box<dyn Error>> {
    Ok(CreateHostResponse {
        host_id: row.try_get(0)?,
        name: row.try_get(1)?,
        adm_status: row.try_get::<_, i16>(2)?.try_into()?,
        labels: row.try_get(3)?,
        last_updated: row.try_get(4)?,
        monitored_by: row.try_get(5)?,
    })
}
