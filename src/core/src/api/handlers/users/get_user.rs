// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::handlers::users::{USER_COLUMNS, User, not_found, row_to_user};
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::models::serialize_utc_as_local;
use crate::db::schema::{AdmStatus, Users};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GetUserResponse {
    pub user_id: Uuid,
    pub username: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    #[schema(value_type = i16)]
    pub adm_status: AdmStatus,
    pub allow_sso: bool,
    pub sso_linked: bool,
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub last_updated: DateTime<Utc>,
}

impl From<User> for GetUserResponse {
    fn from(user: User) -> Self {
        Self {
            user_id: user.user_id,
            username: user.username,
            display_name: user.display_name,
            email: user.email,
            adm_status: user.adm_status,
            allow_sso: user.allow_sso,
            sso_linked: user.sso_linked,
            last_updated: user.last_updated,
        }
    }
}

/// Get a user by ID
#[utoipa::path(
    get,
    path = "/users/{user_id}",
    params(
        ("user_id" = Uuid, Path, description = "User UUID")
    ),
    responses(
        (status = 200, description = "User found", body = ResponseData<GetUserResponse>),
        (status = 404, description = "User not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "users",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn get_user(
    State(state): State<Arc<AppState>>,
    Path(user_id): Path<Uuid>,
) -> Result<ResponseData<GetUserResponse>, ResponseError> {
    let (sql, values) = DbQuery::select()
        .columns(USER_COLUMNS)
        .from(Users::Table)
        .and_where(Expr::col(Users::UserId).eq(user_id))
        .build_postgres(PostgresQueryBuilder);

    let client = state.db_pool.get().await.ctx("get database connection")?;
    let row = client
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("query user")?
        .ok_or_else(|| not_found(user_id))?;
    let user = row_to_user(&row).ctx("parse user")?;

    Ok(ResponseData::from_status(StatusCode::OK).with_data(user.into()))
}
