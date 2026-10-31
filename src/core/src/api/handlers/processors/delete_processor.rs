// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::{ItemProcessors, ProcessorInputs, Processors, SinkInputs};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use std::sync::Arc;
use uuid::Uuid;

type DeleteProcessorResponse = ();

/// Delete a processor
#[utoipa::path(
    delete,
    path = "/processors/{processor_id}",
    params(
        ("processor_id" = Uuid, Path, description = "Processor UUID")
    ),
    responses(
        (status = 204, description = "Processor deleted", body = ResponseData<DeleteProcessorResponse>),
        (status = 404, description = "Processor not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "processors",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn delete_processor(
    State(state): State<Arc<AppState>>,
    Path(processor_id): Path<Uuid>,
) -> Result<ResponseData<DeleteProcessorResponse>, ResponseError> {
    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    let mut query = DbQuery::select();
    query
        .column((Processors::Table, Processors::ProcessorId))
        .from(Processors::Table)
        .and_where(Expr::col((Processors::Table, Processors::ProcessorId)).eq(processor_id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let row_opt = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("query processor")?;

    if row_opt.is_none() {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Processor not found")
            .with_detail(format!("Processor with id {} not found", processor_id)));
    }

    let (sql, values) = DbQuery::delete()
        .from_table(ItemProcessors::Table)
        .and_where(Expr::col((ItemProcessors::Table, ItemProcessors::ProcessorId)).eq(processor_id))
        .build_postgres(PostgresQueryBuilder);

    _ = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete item processors")?;

    let (sql, values) = DbQuery::delete()
        .from_table(SinkInputs::Table)
        .and_where(Expr::col((SinkInputs::Table, SinkInputs::InputId)).eq(processor_id))
        .build_postgres(PostgresQueryBuilder);

    _ = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete sink inputs")?;

    let (sql, values) = DbQuery::delete()
        .from_table(ProcessorInputs::Table)
        .and_where(
            Expr::col((ProcessorInputs::Table, ProcessorInputs::ProcessorId))
                .eq(processor_id)
                .or(Expr::col((ProcessorInputs::Table, ProcessorInputs::InputId)).eq(processor_id)),
        )
        .build_postgres(PostgresQueryBuilder);

    _ = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete processor inputs")?;

    let (sql, values) = DbQuery::delete()
        .from_table(Processors::Table)
        .and_where(Expr::col((Processors::Table, Processors::ProcessorId)).eq(processor_id))
        .build_postgres(PostgresQueryBuilder);

    _ = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete processor")?;

    tx.commit().await.ctx("commit transaction")?;

    Ok(ResponseData::from_status(StatusCode::NO_CONTENT))
}
