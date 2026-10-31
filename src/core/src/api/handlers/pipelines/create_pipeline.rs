// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::handlers::pipelines::{
    PIPELINE_COLUMNS, PipelineDetail, fmt_ids, load_detail, missing_ids, now, row_to_pipeline,
};
use crate::api::jwt::AuthUser;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::{
    AdmStatus, Hosts, PipelineCanvases, PipelineHosts, PipelineSinks, Pipelines, Sinks, Visibility,
    default_adm_status,
};
use axum::{Extension, Json, extract::State, http::StatusCode};
use sea_query::{PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePipelineRequest {
    pub name: String,
    #[serde(default = "default_visibility")]
    #[schema(default = default_visibility, value_type = i16)]
    pub visibility: Visibility,
    #[serde(default = "default_adm_status")]
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    /// Hosts whose items the canvas may use.
    #[serde(default)]
    pub host_ids: BTreeSet<Uuid>,
    /// Sinks the canvas may use.
    #[serde(default)]
    pub sink_ids: BTreeSet<Uuid>,
}

fn default_visibility() -> Visibility {
    Visibility::Private
}

/// Create a pipeline
///
/// The caller becomes the owner. The pipeline starts with one empty canvas.
#[utoipa::path(
    post,
    path = "/pipelines",
    request_body(
        content = CreatePipelineRequest,
        description = "Create pipeline request body",
        content_type = "application/json",
    ),
    responses(
        (status = 201, description = "Pipeline created", body = ResponseData<PipelineDetail>),
        (status = 400, description = "Validation error", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "pipelines",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn create_pipeline(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Json(payload): Json<CreatePipelineRequest>,
) -> Result<ResponseData<PipelineDetail>, ResponseError> {
    // validate request
    if payload.name.trim().is_empty() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Validation error")
            .with_detail("Name cannot be empty"));
    }

    let mut client = state.db_pool.get().await.ctx("get database connection")?;
    let tx = client.transaction().await.ctx("start transaction")?;

    // check hosts and sinks exist
    let missing = missing_ids(&tx, Hosts::Table, Hosts::HostId, &payload.host_ids)
        .await
        .ctx("check host ids")?;
    if !missing.is_empty() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Validation error")
            .with_detail(format!("Host IDs not found: {}", fmt_ids(&missing))));
    }
    let missing = missing_ids(&tx, Sinks::Table, Sinks::SinkId, &payload.sink_ids)
        .await
        .ctx("check sink ids")?;
    if !missing.is_empty() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Validation error")
            .with_detail(format!("Sink IDs not found: {}", fmt_ids(&missing))));
    }

    // insert pipeline
    let pipeline_id = Uuid::now_v7();
    let now = now();
    let (sql, values) = DbQuery::insert()
        .into_table(Pipelines::Table)
        .columns([
            Pipelines::PipelineId,
            Pipelines::Name,
            Pipelines::Visibility,
            Pipelines::OwnerUserId,
            Pipelines::AdmStatus,
            Pipelines::CreatedAt,
            Pipelines::LastUpdated,
        ])
        .values([
            pipeline_id.into(),
            payload.name.into(),
            (payload.visibility as i16).into(),
            auth_user.user_id.into(),
            (payload.adm_status as i16).into(),
            now.into(),
            now.into(),
        ])
        .ctx("build insert query")?
        .returning(DbQuery::returning().columns(PIPELINE_COLUMNS))
        .build_postgres(PostgresQueryBuilder);
    let row = tx
        .query_one(sql.as_str(), &values.as_params())
        .await
        .ctx("insert pipeline")?;
    let pipeline = row_to_pipeline(&row).ctx("parse pipeline")?;

    // insert host subset
    if !payload.host_ids.is_empty() {
        let mut query = DbQuery::insert();
        query
            .into_table(PipelineHosts::Table)
            .columns([PipelineHosts::PipelineId, PipelineHosts::HostId]);
        for host_id in &payload.host_ids {
            query
                .values([pipeline_id.into(), (*host_id).into()])
                .ctx("build insert hosts query")?;
        }
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        tx.execute(sql.as_str(), &values.as_params())
            .await
            .ctx("insert pipeline hosts")?;
    }

    // insert sink subset
    if !payload.sink_ids.is_empty() {
        let mut query = DbQuery::insert();
        query
            .into_table(PipelineSinks::Table)
            .columns([PipelineSinks::PipelineId, PipelineSinks::SinkId]);
        for sink_id in &payload.sink_ids {
            query
                .values([pipeline_id.into(), (*sink_id).into()])
                .ctx("build insert sinks query")?;
        }
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        tx.execute(sql.as_str(), &values.as_params())
            .await
            .ctx("insert pipeline sinks")?;
    }

    // insert the empty canvas
    let (sql, values) = DbQuery::insert()
        .into_table(PipelineCanvases::Table)
        .columns([
            PipelineCanvases::CanvasId,
            PipelineCanvases::PipelineId,
            PipelineCanvases::LastUpdated,
        ])
        .values([Uuid::now_v7().into(), pipeline_id.into(), now.into()])
        .ctx("build insert canvas query")?
        .build_postgres(PostgresQueryBuilder);
    tx.execute(sql.as_str(), &values.as_params())
        .await
        .ctx("insert pipeline canvas")?;

    let detail = load_detail(&tx, pipeline).await?;
    tx.commit().await.ctx("commit transaction")?;

    Ok(ResponseData::from_status(StatusCode::CREATED).with_data(detail))
}
