// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::OrderByDirection;
use crate::api::error::ApiResultExt;
use crate::api::handlers::pipelines::{
    ManagedPipeline, PIPELINE_COLUMNS, row_to_pipeline, visible_to,
};
use crate::api::jwt::AuthUser;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::models::serialize_utc_as_local;
use crate::db::query::generate_count_query;
use crate::db::schema::{AdmStatus, Pipelines, Visibility};
use axum::Extension;
use axum::extract::State;
use axum::http::StatusCode;
use axum_extra::extract::Query as ApiQuery;
use chrono::{DateTime, Utc};
use sea_query::extension::postgres::PgExpr;
use sea_query::{ColumnRef, Expr, IntoColumnRef, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListPipelineRequest {
    pub name: Option<String>,
    #[serde(default = "default_list_limit")]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub order_by: PipelineOrderBy,
    #[serde(default)]
    pub order_dir: OrderByDirection,
}

fn default_list_limit() -> u64 {
    20
}

#[derive(Default, Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PipelineOrderBy {
    #[default]
    Name,
    CreatedAt,
    LastUpdated,
}

impl From<PipelineOrderBy> for ColumnRef {
    fn from(value: PipelineOrderBy) -> Self {
        match value {
            PipelineOrderBy::Name => Pipelines::Name,
            PipelineOrderBy::CreatedAt => Pipelines::CreatedAt,
            PipelineOrderBy::LastUpdated => Pipelines::LastUpdated,
        }
        .into_column_ref()
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ListPipelineResponseInner {
    pub pipeline_id: Uuid,
    pub name: String,
    #[schema(value_type = i16)]
    pub visibility: Visibility,
    pub owner_user_id: Option<Uuid>,
    #[schema(value_type = i16)]
    pub adm_status: AdmStatus,
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub created_at: DateTime<Utc>,
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub last_updated: DateTime<Utc>,
}

impl From<ManagedPipeline> for ListPipelineResponseInner {
    fn from(pipeline: ManagedPipeline) -> Self {
        Self {
            pipeline_id: pipeline.pipeline_id,
            name: pipeline.name,
            visibility: pipeline.visibility,
            owner_user_id: pipeline.owner_user_id,
            adm_status: pipeline.adm_status,
            created_at: pipeline.created_at,
            last_updated: pipeline.last_updated,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ListPipelineResponse {
    pub pipelines: Vec<ListPipelineResponseInner>,
    pub total: u32,
}

/// List pipelines
///
/// Returns public pipelines and the caller's private ones.
#[utoipa::path(
    get,
    path = "/pipelines",
    params(ListPipelineRequest),
    responses(
        (status = 200, description = "List of pipelines", body = ResponseData<ListPipelineResponse>),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "pipelines",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn list_pipelines(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    ApiQuery(params): ApiQuery<ListPipelineRequest>,
) -> Result<ResponseData<ListPipelineResponse>, ResponseError> {
    let client = state.db_pool.get().await.ctx("get database connection")?;

    // list visible pipelines, filtered
    let mut query = DbQuery::select();
    query
        .columns(PIPELINE_COLUMNS)
        .from(Pipelines::Table)
        .cond_where(visible_to(auth_user.user_id));

    if let Some(name) = &params.name {
        query.and_where(Expr::col(Pipelines::Name).ilike(format!("%{}%", name)));
    }

    let total_query = generate_count_query(&query);

    query.order_by(params.order_by, params.order_dir.into());
    query.limit(params.limit).offset(params.offset);

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let rows = client
        .query(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    let pipelines = rows
        .iter()
        .map(row_to_pipeline)
        .collect::<anyhow::Result<Vec<ManagedPipeline>>>()
        .ctx("parse pipeline")?
        .into_iter()
        .map(ListPipelineResponseInner::from)
        .collect();

    // count all matches, ignoring limit and offset
    let (sql, values) = total_query.build_postgres(PostgresQueryBuilder);
    let total: i64 = client
        .query_one_scalar(sql.as_str(), &values.as_params())
        .await
        .ctx("execute total count query")?;

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(ListPipelineResponse {
            pipelines,
            total: total as u32,
        }),
    )
}
