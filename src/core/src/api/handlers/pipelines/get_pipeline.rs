// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::handlers::pipelines::{PipelineDetail, find_pipeline, load_detail};
use crate::api::jwt::AuthUser;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use axum::Extension;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use std::sync::Arc;
use uuid::Uuid;

/// Get a pipeline by ID
#[utoipa::path(
    get,
    path = "/pipelines/{pipeline_id}",
    params(
        ("pipeline_id" = Uuid, Path, description = "Pipeline UUID")
    ),
    responses(
        (status = 200, description = "Pipeline found", body = ResponseData<PipelineDetail>),
        (status = 404, description = "Pipeline not found or not visible to the caller", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "pipelines",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn get_pipeline(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(pipeline_id): Path<Uuid>,
) -> Result<ResponseData<PipelineDetail>, ResponseError> {
    let client = state.db_pool.get().await.ctx("get database connection")?;
    let pipeline = find_pipeline(&client, pipeline_id, auth_user.user_id, false).await?;
    let detail = load_detail(&client, pipeline).await?;

    Ok(ResponseData::from_status(StatusCode::OK).with_data(detail))
}
