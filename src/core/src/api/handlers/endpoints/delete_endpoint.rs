// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::{HostEndpoints, Items};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use std::sync::Arc;
use uuid::Uuid;

type DeleteEndpointResponse = ();

/// Delete an endpoint
#[utoipa::path(
    delete,
    path = "/endpoints/{endpoint_id}",
    params(
        ("endpoint_id" = Uuid, Path, description = "Endpoint UUID")
    ),
    responses(
        (status = 204, description = "Endpoint deleted", body = ResponseData<DeleteEndpointResponse>),
        (status = 404, description = "Endpoint not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "endpoints",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn delete_endpoint(
    State(state): State<Arc<AppState>>,
    Path(endpoint_id): Path<Uuid>,
) -> Result<ResponseData<DeleteEndpointResponse>, ResponseError> {
    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // get endpoint
    let mut query = DbQuery::select();
    query
        .column(HostEndpoints::EndpointId)
        .from(HostEndpoints::Table)
        .and_where(Expr::col(HostEndpoints::EndpointId).eq(endpoint_id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let row_opt = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("query endpoint")?;

    if row_opt.is_none() {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Endpoint not found")
            .with_detail(format!("Endpoint with id {} not found", endpoint_id)));
    }

    // set items with endpoint_id to null
    let mut query = DbQuery::update();
    query
        .table(Items::Table)
        .value(Items::EndpointId, None as Option<Uuid>)
        .and_where(Expr::col((Items::Table, Items::EndpointId)).eq(endpoint_id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    tx.query(sql.as_str(), &values.as_params())
        .await
        .ctx("update items")?;

    // delete endpoint
    let mut query = DbQuery::delete();
    query
        .from_table(HostEndpoints::Table)
        .and_where(Expr::col(HostEndpoints::EndpointId).eq(endpoint_id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let _ = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete endpoint")?;

    tx.commit().await.ctx("commit transaction")?;

    Ok(ResponseData::from_status(StatusCode::NO_CONTENT))
}
