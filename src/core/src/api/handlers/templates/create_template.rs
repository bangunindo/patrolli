// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::{HostTemplates, Templates};
use axum::{Json, extract::State, http::StatusCode};
use sea_query::{PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateTemplateRequest {
    pub name: String,
    pub labels: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateTemplateResponse {
    pub template_id: Uuid,
    pub name: String,
    pub labels: serde_json::Value,
}

/// Create a template
#[utoipa::path(
    post,
    path = "/templates",
    request_body = CreateTemplateRequest,
    responses(
        (status = 201, description = "Template created", body = ResponseData<CreateTemplateResponse>),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "templates",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn create_template(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateTemplateRequest>,
) -> Result<ResponseData<CreateTemplateResponse>, ResponseError> {
    if payload.name.trim().is_empty() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Validation error")
            .with_detail("Template name cannot be empty"));
    }

    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // create template
    let id = Uuid::now_v7();
    let payload_clone = payload.clone();
    let (sql, values) = DbQuery::insert()
        .into_table(Templates::Table)
        .columns([Templates::TemplateId, Templates::Name, Templates::Labels])
        .values([id.into(), payload.name.into(), payload.labels.into()])
        .ctx("build insert query")?
        .build_postgres(PostgresQueryBuilder);

    _ = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    // create host template entry for the template
    let host_template_id = Uuid::now_v7();
    let (insert_sql, insert_values) = DbQuery::insert()
        .into_table(HostTemplates::Table)
        .columns([HostTemplates::HostTemplateId, HostTemplates::TemplateId])
        .values([host_template_id.into(), id.into()])
        .ctx("build insert query for host template")?
        .build_postgres(PostgresQueryBuilder);

    _ = tx
        .execute(insert_sql.as_str(), &insert_values.as_params())
        .await
        .ctx("execute insert query for host template")?;

    tx.commit().await.ctx("commit transaction")?;

    let response = CreateTemplateResponse {
        template_id: id,
        name: payload_clone.name,
        labels: payload_clone.labels,
    };

    Ok(ResponseData::from_status(StatusCode::CREATED).with_data(response))
}
