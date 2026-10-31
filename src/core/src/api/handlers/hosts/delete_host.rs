// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::{HostEndpoints, HostTemplates, Hosts};
use axum::{
    extract::{Path, State},
    http::StatusCode,
};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use std::sync::Arc;
use uuid::Uuid;

// utoipa macro tweaks out when using `body = ResponseData<()>``
type DeleteHostResponse = ();

/// Delete a host
///
/// Deletes a host from the database and cache
#[utoipa::path(
    delete,
    path = "/hosts/{id}",
    params(
        ("id" = Uuid, Path, description = "Host UUID")
    ),
    responses(
        (status = 204, description = "Host deleted", body = ResponseData<DeleteHostResponse>),
        (status = 404, description = "Host not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "hosts",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn delete_host(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<ResponseData<DeleteHostResponse>, ResponseError> {
    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // Check if the host exists
    let mut query = DbQuery::select();
    query
        .column(Hosts::HostId)
        .from(Hosts::Table)
        .and_where(Expr::col(Hosts::HostId).eq(id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let result = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("get host")?;

    if result.is_none() {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Host not found")
            .with_detail(format!("No host found for id: {}", id)));
    }

    // Delete items
    let statement = "
        DELETE FROM items
        USING host_templates
        WHERE items.host_template_id = host_templates.host_template_id
        AND host_templates.host_id = $1;";

    tx.execute(statement, &[&id])
        .await
        .ctx("delete items for host")?;

    // Delete host templates
    let mut query = DbQuery::delete();
    query
        .from_table(HostTemplates::Table)
        .and_where(Expr::col(HostTemplates::HostId).eq(id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    tx.execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete host templates for host")?;

    // Detach any remaining items (other hosts') from this host's endpoints
    let statement = "
        UPDATE items
        SET endpoint_id = NULL
        FROM host_endpoints
        WHERE items.endpoint_id = host_endpoints.endpoint_id
        AND host_endpoints.host_id = $1;";

    tx.execute(statement, &[&id])
        .await
        .ctx("detach items from endpoints of host")?;

    // Delete endpoints owned by the host
    let mut query = DbQuery::delete();
    query
        .from_table(HostEndpoints::Table)
        .and_where(Expr::col(HostEndpoints::HostId).eq(id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    tx.execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete endpoints for host")?;

    // Delete host
    let mut query = DbQuery::delete();
    query
        .from_table(Hosts::Table)
        .and_where(Expr::col(Hosts::HostId).eq(id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    tx.execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete host")?;

    tx.commit().await.ctx("commit transaction")?;

    Ok(ResponseData::from_status(StatusCode::NO_CONTENT))
}
