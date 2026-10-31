// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::handlers::auth::TokenResponse;
use crate::api::jwt::{AuthUser, issue_pair};
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::{AdmStatus, RefreshTokens, Users};
use axum::{Extension, Json, extract::State, http::StatusCode};
use chrono::Utc;
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::Deserialize;
use std::sync::Arc;
use tracing::{info, warn};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    #[schema(example = "password")]
    pub current_password: String,
    #[schema(example = "new-password")]
    pub new_password: String,
}

/// Change password endpoint
///
/// Updates a user's password after verifying their current credentials.
/// Only for the caller's own password; changing another user's password is
/// `PATCH /api/v1/users/{user_id}`.
#[utoipa::path(
	post,
	path = "/change-password",
	request_body = ChangePasswordRequest,
	responses(
		(status = 200, description = "Password updated", body = ResponseData<TokenResponse>),
		(status = 400, description = "Invalid input", body = ResponseError),
		(status = 401, description = "Invalid credentials", body = ResponseError),
		(status = 403, description = "Account has no password or is disabled", body = ResponseError),
		(status = 500, description = "Internal server error", body = ResponseError)
	),
	tag = "auth",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn change_password(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Json(payload): Json<ChangePasswordRequest>,
) -> Result<ResponseData<TokenResponse>, ResponseError> {
    let AuthUser { user_id, username } = auth_user;

    if payload.new_password.trim().is_empty() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Invalid password")
            .with_detail("New password must not be empty"));
    }

    if payload.current_password == payload.new_password {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Invalid password")
            .with_detail("New password must be different from current password"));
    }

    let mut conn = state.db_pool.get().await.ctx("get database connection")?;

    let tx = conn.transaction().await.ctx("start transaction")?;

    // reject users without a password and disabled users, whose access token can
    // still be valid for a while and must not get them new tokens
    let (sql, values) = DbQuery::select()
        .expr(Expr::col(Users::PasswordHash).is_null())
        .column(Users::AdmStatus)
        .from(Users::Table)
        .and_where(Expr::col(Users::UserId).eq(user_id))
        .build_postgres(PostgresQueryBuilder);
    let row = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("query password")?;
    if row.as_ref().is_some_and(|row| row.get::<_, bool>(0)) {
        return Err(ResponseError::from_status(StatusCode::FORBIDDEN)
            .with_title("Not permitted")
            .with_detail("This account has no password to change."));
    }
    if row.is_some_and(|row| row.get::<_, i16>(1) != AdmStatus::Enabled as i16) {
        return Err(ResponseError::from_status(StatusCode::FORBIDDEN)
            .with_title("Account is not active")
            .with_detail("Contact an administrator to re-enable this account"));
    }

    // set the new password only if the current one matches
    let (sql, values) = DbQuery::update()
        .table(Users::Table)
        .value(
            Users::PasswordHash,
            Expr::cust_with_values(
                "crypt($1, gen_salt('bf', 12))",
                [payload.new_password.clone()],
            ),
        )
        .value(Users::LastUpdated, Utc::now())
        .and_where(Expr::col(Users::UserId).eq(user_id))
        .and_where(Expr::col(Users::PasswordHash).eq(Expr::cust_with_values(
            "crypt($1, password_hash)",
            [payload.current_password.clone()],
        )))
        .build_postgres(PostgresQueryBuilder);
    let updated = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("update password")?;

    if updated == 0 {
        warn!("password change rejected, current password incorrect");
        return Err(ResponseError::from_status(StatusCode::UNAUTHORIZED)
            .with_title("Invalid credentials")
            .with_detail("Current password is incorrect"));
    }

    // Log every session out, including this one. Access tokens already handed
    // out stay valid until they expire; that window is the access token TTL.
    let (sql, values) = DbQuery::delete()
        .from_table(RefreshTokens::Table)
        .and_where(Expr::col(RefreshTokens::UserId).eq(user_id))
        .build_postgres(PostgresQueryBuilder);
    tx.execute(sql.as_str(), &values.as_params())
        .await
        .ctx("revoke refresh tokens")?;

    // Re-issue for the caller, so changing your own password is not a logout.
    let (access_token, refresh_token) = issue_pair(&tx, &state.jwt, user_id, &username)
        .await
        .ctx("issue tokens")?;

    tx.commit().await.ctx("commit transaction")?;
    info!("password changed, sessions revoked");

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(TokenResponse {
            access_token,
            refresh_token,
            expires_in: state.jwt.access_ttl_secs(),
        }),
    )
}
