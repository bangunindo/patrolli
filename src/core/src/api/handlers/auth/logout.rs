// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::RefreshTokens;
use axum::{Json, extract::State, http::StatusCode};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct LogoutRequest {
    pub refresh_token: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LogoutResponse {
    pub message: String,
}

/// Logout endpoint
///
/// Revokes the refresh token. Any access token already issued stays valid until
/// it expires, which is why its lifetime is short.
#[utoipa::path(
    post,
    path = "/logout",
    request_body = LogoutRequest,
    responses(
        (status = 200, description = "Logout successful", body = ResponseData<LogoutResponse>),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "auth"
)]
pub async fn logout(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<LogoutRequest>,
) -> Result<ResponseData<LogoutResponse>, ResponseError> {
    let conn = state.db_pool.get().await.ctx("get database connection")?;

    // Unknown or already-revoked tokens are not an error: logout is idempotent.
    let (sql, values) = DbQuery::delete()
        .from_table(RefreshTokens::Table)
        .and_where(
            Expr::col(RefreshTokens::TokenHash).eq(Expr::cust_with_values(
                "digest($1, 'sha256')",
                [payload.refresh_token.clone()],
            )),
        )
        .build_postgres(PostgresQueryBuilder);
    conn.execute(sql.as_str(), &values.as_params())
        .await
        .ctx("revoke refresh token")?;

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(LogoutResponse {
            message: "Logout successful".to_string(),
        }),
    )
}
