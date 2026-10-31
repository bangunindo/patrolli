// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::handlers::pipelines::graph::PipelineGraph;
use crate::api::handlers::pipelines::runtime::load_graph;
use crate::api::handlers::pipelines::{find_canvas, find_pipeline};
use crate::api::jwt::AuthUser;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::models::serialize_utc_as_local;
use axum::Extension;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::sync::Arc;
use tokio_postgres::IsolationLevel;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GetPipelineGraphResponse {
    pub pipeline_id: Uuid,
    pub canvas_id: Uuid,
    /// Send it back when saving the graph.
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub last_updated: DateTime<Utc>,
    pub graph: PipelineGraph,
}

/// Get the graph of a canvas
///
/// Returns the structure of the canvas. Sources and processor groups carry a
/// summary of their content; the full content is read per node.
#[utoipa::path(
    get,
    path = "/pipelines/{pipeline_id}/canvases/{canvas_id}/graph",
    params(
        ("pipeline_id" = Uuid, Path, description = "Pipeline UUID"),
        ("canvas_id" = Uuid, Path, description = "Canvas UUID")
    ),
    responses(
        (status = 200, description = "Canvas graph", body = ResponseData<GetPipelineGraphResponse>),
        (status = 404, description = "Pipeline or canvas not found, or pipeline not visible to the caller", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "pipelines",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn get_pipeline_graph(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path((pipeline_id, canvas_id)): Path<(Uuid, Uuid)>,
) -> Result<ResponseData<GetPipelineGraphResponse>, ResponseError> {
    let mut client = state.db_pool.get().await.ctx("get database connection")?;
    // one snapshot for every query, so a concurrent save is seen whole or not at all
    let tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await
        .ctx("start transaction")?;

    // check pipeline is visible and owns the canvas
    find_pipeline(&tx, pipeline_id, auth_user.user_id, false).await?;
    let last_updated = find_canvas(&tx, pipeline_id, canvas_id).await?;

    let graph = load_graph(&tx, canvas_id).await.ctx("load graph")?;
    tx.commit().await.ctx("commit transaction")?;

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(GetPipelineGraphResponse {
            pipeline_id,
            canvas_id,
            last_updated,
            graph,
        }),
    )
}
