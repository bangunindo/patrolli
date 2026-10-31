// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::handlers::users::update_user::UpdateUserResponse;
use crate::api::handlers::users::{USER_COLUMNS, not_found, row_to_user};
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::Users;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

/// Unlink a user's SSO account
///
/// The next SSO login that matches the username links again, so this also fixes a
/// user whose identity provider account was recreated. `allow_sso` is not changed.
#[utoipa::path(
    post,
    path = "/users/{user_id}/unlink-sso",
    params(
        ("user_id" = Uuid, Path, description = "User UUID")
    ),
    responses(
        (status = 200, description = "User unlinked", body = ResponseData<UpdateUserResponse>),
        (status = 404, description = "User not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "users",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn unlink_sso(
    State(state): State<Arc<AppState>>,
    Path(user_id): Path<Uuid>,
) -> Result<ResponseData<UpdateUserResponse>, ResponseError> {
    let (sql, values) = DbQuery::update()
        .table(Users::Table)
        .value(Users::OidcSubject, Option::<String>::None)
        .value(Users::LastUpdated, Utc::now())
        .and_where(Expr::col(Users::UserId).eq(user_id))
        .returning(DbQuery::returning().columns(USER_COLUMNS))
        .build_postgres(PostgresQueryBuilder);

    let client = state.db_pool.get().await.ctx("get database connection")?;
    let row = client
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("unlink sso")?
        .ok_or_else(|| not_found(user_id))?;
    let unlinked = row_to_user(&row).ctx("parse user")?;
    info!(
        target_user_id = %unlinked.user_id,
        target_user = %unlinked.username,
        "user sso unlinked"
    );

    Ok(ResponseData::from_status(StatusCode::OK).with_data(unlinked.into()))
}
