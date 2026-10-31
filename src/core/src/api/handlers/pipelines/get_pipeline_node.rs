// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::handlers::pipelines::graph::{GraphNodeContent, GraphNodeKind};
use crate::api::handlers::pipelines::runtime::load_node_content;
use crate::api::handlers::pipelines::{find_canvas, find_pipeline, node_not_found};
use crate::api::jwt::AuthUser;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::models::serialize_utc_as_local;
use crate::db::schema::PipelineNodes;
use axum::Extension;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum_extra::extract::Query as ApiQuery;
use chrono::{DateTime, Utc};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio_postgres::IsolationLevel;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct GetPipelineNodeRequest {
    /// Source nodes only: keep items whose name contains this, ignoring case.
    pub name: Option<String>,
    /// Source nodes only.
    #[serde(default = "default_limit")]
    pub limit: u64,
    /// Source nodes only.
    #[serde(default)]
    pub offset: u64,
}

fn default_limit() -> u64 {
    20
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GetPipelineNodeResponse {
    pub pipeline_id: Uuid,
    pub canvas_id: Uuid,
    pub node_id: Uuid,
    /// Send it back when saving the node's content.
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub last_updated: DateTime<Utc>,
    #[serde(flatten)]
    pub content: GraphNodeContent,
}

/// Get the content of a node
///
/// A source holds items, read one page at a time. A processor group holds
/// its processors with their config. A sink node shows its sink.
#[utoipa::path(
    get,
    path = "/pipelines/{pipeline_id}/canvases/{canvas_id}/nodes/{node_id}",
    params(
        ("pipeline_id" = Uuid, Path, description = "Pipeline UUID"),
        ("canvas_id" = Uuid, Path, description = "Canvas UUID"),
        ("node_id" = Uuid, Path, description = "Node UUID"),
        GetPipelineNodeRequest,
    ),
    responses(
        (status = 200, description = "Node content", body = ResponseData<GetPipelineNodeResponse>),
        (status = 404, description = "Pipeline, canvas or node not found, or pipeline not visible to the caller", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "pipelines",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn get_pipeline_node(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path((pipeline_id, canvas_id, node_id)): Path<(Uuid, Uuid, Uuid)>,
    ApiQuery(params): ApiQuery<GetPipelineNodeRequest>,
) -> Result<ResponseData<GetPipelineNodeResponse>, ResponseError> {
    let mut client = state.db_pool.get().await.ctx("get database connection")?;
    // one snapshot, so the page and the total agree
    let tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await
        .ctx("start transaction")?;

    // check pipeline is visible and owns the canvas and the node
    find_pipeline(&tx, pipeline_id, auth_user.user_id, false).await?;
    find_canvas(&tx, pipeline_id, canvas_id).await?;
    let (sql, values) = DbQuery::select()
        .columns([PipelineNodes::Kind, PipelineNodes::LastUpdated])
        .from(PipelineNodes::Table)
        .and_where(Expr::col(PipelineNodes::NodeId).eq(node_id))
        .and_where(Expr::col(PipelineNodes::CanvasId).eq(canvas_id))
        .build_postgres(PostgresQueryBuilder);
    let row = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("query node")?
        .ok_or_else(|| node_not_found(node_id))?;
    let kind: String = row.get(0);
    let kind: GraphNodeKind = kind.parse().ctx("parse node kind")?;

    let content = load_node_content(&tx, node_id, kind, params.name, params.limit, params.offset)
        .await
        .ctx("load node content")?;
    tx.commit().await.ctx("commit transaction")?;

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(GetPipelineNodeResponse {
            pipeline_id,
            canvas_id,
            node_id,
            last_updated: row.get(1),
            content,
        }),
    )
}
