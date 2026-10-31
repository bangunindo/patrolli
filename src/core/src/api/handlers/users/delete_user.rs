// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::handlers::users::not_found;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::Users;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

type DeleteUserResponse = ();

/// Delete a user
#[utoipa::path(
    delete,
    path = "/users/{user_id}",
    params(
        ("user_id" = Uuid, Path, description = "User UUID")
    ),
    responses(
        (status = 204, description = "User deleted", body = ResponseData<DeleteUserResponse>),
        (status = 404, description = "User not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "users",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn delete_user(
    State(state): State<Arc<AppState>>,
    Path(user_id): Path<Uuid>,
) -> Result<ResponseData<DeleteUserResponse>, ResponseError> {
    // delete user, its refresh tokens go with it
    let (sql, values) = DbQuery::delete()
        .from_table(Users::Table)
        .and_where(Expr::col(Users::UserId).eq(user_id))
        .build_postgres(PostgresQueryBuilder);

    let client = state.db_pool.get().await.ctx("get database connection")?;
    let deleted = client
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete user")?;
    if deleted == 0 {
        return Err(not_found(user_id));
    }
    info!(target_user_id = %user_id, "user deleted");

    Ok(ResponseData::from_status(StatusCode::NO_CONTENT))
}
