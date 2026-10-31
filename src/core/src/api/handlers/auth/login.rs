// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::handlers::auth::TokenResponse;
use crate::api::jwt::issue_pair;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::{AdmStatus, Users};
use axum::{Json, extract::State, http::StatusCode};
use sea_query::{
    Asterisk, Expr, ExprTrait, Func, LockType, PostgresQueryBuilder, Query as DbQuery,
};
use sea_query_postgres::PostgresBinder;
use serde::Deserialize;
use std::sync::Arc;
use tracing::{info, warn};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    #[schema(example = "admin")]
    pub username: String,
    #[schema(example = "password")]
    pub password: String,
}

/// Login endpoint
///
/// Authenticates a user with username and password and returns a token pair.
#[utoipa::path(
    post,
    path = "/login",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful", body = ResponseData<TokenResponse>),
        (status = 401, description = "Invalid credentials", body = ResponseError),
        (status = 403, description = "Account is disabled", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "auth"
)]
pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<LoginRequest>,
) -> Result<ResponseData<TokenResponse>, ResponseError> {
    let mut conn = state.db_pool.get().await.ctx("get database connection")?;
    let tx = conn.transaction().await.ctx("start transaction")?;

    // Check if at least one user exists, if not tell user to set ENV vars to create initial user
    let (sql, values) = DbQuery::select()
        .expr(Func::count(Expr::col(Asterisk)))
        .from(Users::Table)
        .build_postgres(PostgresQueryBuilder);
    let user_count: i64 = tx
        .query_one(sql.as_str(), &values.as_params())
        .await
        .ctx("query user count")?
        .get(0);

    if user_count == 0 {
        warn!("login attempted but no users exist");
        return Err(ResponseError::from_status(StatusCode::UNAUTHORIZED)
            .with_title("No users found")
            .with_detail("Please configure initial user credentials using environment variables"));
    }

    // Find the user whose password matches. The row lock makes an admin disable,
    // password reset or switch to SSO that runs at the same time wait for this
    // login, so the refresh token issued here is revoked with the others.
    let (sql, values) = DbQuery::select()
        .columns([Users::UserId, Users::Username, Users::AdmStatus])
        .from(Users::Table)
        .and_where(Expr::col(Users::Username).eq(&payload.username))
        .and_where(Expr::col(Users::PasswordHash).eq(Expr::cust_with_values(
            "crypt($1, password_hash)",
            [payload.password.clone()],
        )))
        .lock(LockType::Share)
        .build_postgres(PostgresQueryBuilder);
    let row = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("query users")?;

    let Some(row) = row else {
        warn!(user = %payload.username, "login failed");
        return Err(ResponseError::from_status(StatusCode::UNAUTHORIZED)
            .with_title("Invalid credentials")
            .with_detail("Username or password is incorrect"));
    };

    let user_id: Uuid = row.get(0);
    let username: String = row.get(1);
    let adm_status: i16 = row.get(2);

    // Only reveal that the account is switched off after the password has matched.
    // Saying it any earlier would tell anyone which usernames exist.
    if adm_status != AdmStatus::Enabled as i16 {
        warn!(user_id = %user_id, user = %username, "login rejected for inactive account");
        return Err(ResponseError::from_status(StatusCode::FORBIDDEN)
            .with_title("Account is not active")
            .with_detail("Contact an administrator to re-enable this account"));
    }

    let (access_token, refresh_token) = issue_pair(&tx, &state.jwt, user_id, &username)
        .await
        .ctx("issue tokens")?;

    tx.commit().await.ctx("commit transaction")?;
    info!(user_id = %user_id, user = %username, "user logged in");

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(TokenResponse {
            access_token,
            refresh_token,
            expires_in: state.jwt.access_ttl_secs(),
        }),
    )
}
