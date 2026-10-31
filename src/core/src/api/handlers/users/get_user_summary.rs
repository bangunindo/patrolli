// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::{AdmStatus, Users};
use axum::extract::State;
use axum::http::StatusCode;
use sea_query::{Asterisk, Expr, ExprTrait, Func, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UserSummaryResponse {
    pub total: u32,
    pub allow_sso: u32,
    pub disabled: u32,
}

/// Get user counts
#[utoipa::path(
    get,
    path = "/users/summary",
    responses(
        (status = 200, description = "User counts", body = ResponseData<UserSummaryResponse>),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "users",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn get_user_summary(
    State(state): State<Arc<AppState>>,
) -> Result<ResponseData<UserSummaryResponse>, ResponseError> {
    // count users, sso users and disabled users
    let (sql, values) = DbQuery::select()
        .expr(Func::count(Expr::col(Asterisk)))
        .expr(Func::count(Expr::case(
            Expr::col(Users::AllowSso).eq(true),
            Expr::col(Users::UserId),
        )))
        .expr(Func::count(Expr::case(
            Expr::col(Users::AdmStatus).eq(AdmStatus::Disabled as i16),
            Expr::col(Users::UserId),
        )))
        .from(Users::Table)
        .build_postgres(PostgresQueryBuilder);

    let client = state.db_pool.get().await.ctx("get database connection")?;
    let row = client
        .query_one(sql.as_str(), &values.as_params())
        .await
        .ctx("query user summary")?;

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(UserSummaryResponse {
            total: row.get::<_, i64>(0) as u32,
            allow_sso: row.get::<_, i64>(1) as u32,
            disabled: row.get::<_, i64>(2) as u32,
        }),
    )
}
