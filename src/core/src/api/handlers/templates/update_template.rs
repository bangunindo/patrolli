// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::Templates;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateTemplateRequest {
    pub name: String,
    pub labels: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateTemplateResponse {
    pub template_id: Uuid,
    pub name: String,
    pub labels: serde_json::Value,
}

/// Update a template
#[utoipa::path(
    put,
    path = "/templates/{id}",
    params(
        ("id" = Uuid, Path, description = "Template UUID")
    ),
    request_body = UpdateTemplateRequest,
    responses(
        (status = 200, description = "Template updated", body = ResponseData<UpdateTemplateResponse>),
        (status = 404, description = "Template not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "templates",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn update_template(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateTemplateRequest>,
) -> Result<ResponseData<UpdateTemplateResponse>, ResponseError> {
    if payload.name.trim().is_empty() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Validation error")
            .with_detail("Template name cannot be empty"));
    }

    let payload_clone = payload.clone();
    let client = state.db_pool.get().await.ctx("get database connection")?;

    let mut query = DbQuery::update();
    query
        .table(Templates::Table)
        .and_where(Expr::col(Templates::TemplateId).eq(id))
        .value(Templates::Name, payload.name)
        .value(Templates::Labels, payload.labels);

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);

    let result = client
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    if result == 0 {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Template not found")
            .with_detail(format!("No template found for id: {}", id)));
    }

    let response = UpdateTemplateResponse {
        template_id: id,
        name: payload_clone.name,
        labels: payload_clone.labels,
    };

    Ok(ResponseData::from_status(StatusCode::OK).with_data(response))
}
