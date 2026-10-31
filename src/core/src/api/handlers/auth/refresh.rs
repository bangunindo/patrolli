// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::handlers::auth::TokenResponse;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::{AdmStatus, RefreshTokens, Users};
use axum::{Json, extract::State, http::StatusCode};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::Deserialize;
use std::sync::Arc;
use tracing::debug;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Deserialize, ToSchema)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// Refresh endpoint
///
/// Exchanges a refresh token for a new access token. The refresh token itself is
/// returned unchanged and stays valid until it expires; rotation is not implemented.
#[utoipa::path(
    post,
    path = "/refresh",
    request_body = RefreshRequest,
    responses(
        (status = 200, description = "Access token refreshed", body = ResponseData<TokenResponse>),
        (status = 401, description = "Refresh token expired or revoked", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "auth"
)]
pub async fn refresh(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RefreshRequest>,
) -> Result<ResponseData<TokenResponse>, ResponseError> {
    let conn = state.db_pool.get().await.ctx("get database connection")?;

    // find the unexpired refresh token of an enabled user
    let (sql, values) = DbQuery::select()
        .column((RefreshTokens::Table, RefreshTokens::UserId))
        .column((Users::Table, Users::Username))
        .from(RefreshTokens::Table)
        .inner_join(
            Users::Table,
            Expr::col((Users::Table, Users::UserId))
                .equals((RefreshTokens::Table, RefreshTokens::UserId)),
        )
        .and_where(
            Expr::col((RefreshTokens::Table, RefreshTokens::TokenHash)).eq(Expr::cust_with_values(
                "digest($1, 'sha256')",
                [payload.refresh_token.clone()],
            )),
        )
        .and_where(
            Expr::col((RefreshTokens::Table, RefreshTokens::ExpiresAt))
                .gt(Expr::current_timestamp()),
        )
        .and_where(Expr::col((Users::Table, Users::AdmStatus)).eq(AdmStatus::Enabled as i16))
        .build_postgres(PostgresQueryBuilder);
    let row = conn
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("look up refresh token")?;

    let Some(row) = row else {
        debug!("refresh token rejected");
        return Err(ResponseError::from_status(StatusCode::UNAUTHORIZED)
            .with_title("Unauthorized")
            .with_detail("Your session has expired. Please log in again."));
    };

    let user_id: Uuid = row.get(0);
    let username: String = row.get(1);

    let access_token = state
        .jwt
        .sign_access(user_id, &username)
        .ctx("issue access token")?;

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(TokenResponse {
            access_token,
            refresh_token: payload.refresh_token,
            expires_in: state.jwt.access_ttl_secs(),
        }),
    )
}
