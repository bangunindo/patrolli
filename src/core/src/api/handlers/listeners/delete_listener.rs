// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::{Items, Listeners};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use std::sync::Arc;
use uuid::Uuid;

type DeleteListenerResponse = ();

/// Delete a listener
#[utoipa::path(
    delete,
    path = "/listeners/{listener_id}",
    params(
        ("listener_id" = Uuid, Path, description = "Listener UUID")
    ),
    responses(
        (status = 204, description = "Listener deleted", body = ResponseData<DeleteListenerResponse>),
        (status = 404, description = "Listener not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "listeners",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn delete_listener(
    State(state): State<Arc<AppState>>,
    Path(listener_id): Path<Uuid>,
) -> Result<ResponseData<DeleteListenerResponse>, ResponseError> {
    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // get listener
    let mut query = DbQuery::select();
    query
        .column(Listeners::ListenerId)
        .from(Listeners::Table)
        .and_where(Expr::col(Listeners::ListenerId).eq(listener_id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let row_opt = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("query listener")?;

    if row_opt.is_none() {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Listener not found")
            .with_detail(format!("Listener with id {} not found", listener_id)));
    }

    // set items with listener_id to null
    let mut query = DbQuery::update();
    query
        .table(Items::Table)
        .value(Items::ListenerId, None as Option<Uuid>)
        .and_where(Expr::col((Items::Table, Items::ListenerId)).eq(listener_id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    tx.query(sql.as_str(), &values.as_params())
        .await
        .ctx("update items")?;

    // delete listener
    let mut query = DbQuery::delete();
    query
        .from_table(Listeners::Table)
        .and_where(Expr::col(Listeners::ListenerId).eq(listener_id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let _ = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete listener")?;

    tx.commit().await.ctx("commit transaction")?;

    Ok(ResponseData::from_status(StatusCode::NO_CONTENT))
}
