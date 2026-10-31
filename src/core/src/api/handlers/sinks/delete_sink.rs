// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::{SinkInputs, Sinks};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use std::sync::Arc;
use uuid::Uuid;

type DeleteSinkResponse = ();

/// Delete a sink
#[utoipa::path(
    delete,
    path = "/sinks/{sink_id}",
    params(
        ("sink_id" = Uuid, Path, description = "Sink UUID")
    ),
    responses(
        (status = 204, description = "Sink deleted", body = ResponseData<DeleteSinkResponse>),
        (status = 404, description = "Sink not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "sinks",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn delete_sink(
    State(state): State<Arc<AppState>>,
    Path(sink_id): Path<Uuid>,
) -> Result<ResponseData<DeleteSinkResponse>, ResponseError> {
    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // check if sink exists
    let mut query = DbQuery::select();
    query
        .column((Sinks::Table, Sinks::SinkId))
        .from(Sinks::Table)
        .and_where(Expr::col((Sinks::Table, Sinks::SinkId)).eq(sink_id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let row_opt = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("query sink")?;

    if row_opt.is_none() {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Sink not found")
            .with_detail(format!("Sink with id {} not found", sink_id)));
    }

    // delete sink inputs
    let (sql, values) = DbQuery::delete()
        .from_table(SinkInputs::Table)
        .and_where(Expr::col((SinkInputs::Table, SinkInputs::SinkId)).eq(sink_id))
        .build_postgres(PostgresQueryBuilder);

    _ = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete sink inputs")?;

    // delete sink
    let (sql, values) = DbQuery::delete()
        .from_table(Sinks::Table)
        .and_where(Expr::col((Sinks::Table, Sinks::SinkId)).eq(sink_id))
        .build_postgres(PostgresQueryBuilder);

    _ = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete sink")?;

    tx.commit().await.ctx("commit transaction")?;

    Ok(ResponseData::from_status(StatusCode::NO_CONTENT))
}
