// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::handlers::pipelines::find_pipeline;
use crate::api::handlers::pipelines::runtime::{delete_processors, owned_processor_ids};
use crate::api::jwt::AuthUser;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::Pipelines;
use axum::Extension;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use std::sync::Arc;
use uuid::Uuid;

type DeletePipelineResponse = ();

/// Delete a pipeline
///
/// Only the owner can delete a pipeline. A pipeline whose owner was deleted
/// can be deleted by anyone.
#[utoipa::path(
    delete,
    path = "/pipelines/{pipeline_id}",
    params(
        ("pipeline_id" = Uuid, Path, description = "Pipeline UUID")
    ),
    responses(
        (status = 204, description = "Pipeline deleted", body = ResponseData<DeletePipelineResponse>),
        (status = 403, description = "Caller is not the owner", body = ResponseError),
        (status = 404, description = "Pipeline not found or not visible to the caller", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "pipelines",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn delete_pipeline(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(pipeline_id): Path<Uuid>,
) -> Result<ResponseData<DeletePipelineResponse>, ResponseError> {
    let mut client = state.db_pool.get().await.ctx("get database connection")?;
    let tx = client.transaction().await.ctx("start transaction")?;

    // lock pipeline, only its owner can delete it
    let pipeline = find_pipeline(&tx, pipeline_id, auth_user.user_id, true).await?;
    if pipeline
        .owner_user_id
        .is_some_and(|owner| owner != auth_user.user_id)
    {
        return Err(ResponseError::from_status(StatusCode::FORBIDDEN)
            .with_title("Not the owner")
            .with_detail("Only the owner can delete this pipeline"));
    }

    // collect processors before the nodes that reference them are gone
    let processor_ids = owned_processor_ids(&tx, pipeline_id)
        .await
        .ctx("query owned processors")?;

    // delete pipeline, canvases and nodes go with it
    let (sql, values) = DbQuery::delete()
        .from_table(Pipelines::Table)
        .and_where(Expr::col(Pipelines::PipelineId).eq(pipeline_id))
        .build_postgres(PostgresQueryBuilder);
    tx.execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete pipeline")?;

    // delete its processors and their runtime edges
    delete_processors(&tx, &processor_ids)
        .await
        .ctx("delete processors")?;

    tx.commit().await.ctx("commit transaction")?;

    Ok(ResponseData::from_status(StatusCode::NO_CONTENT))
}
