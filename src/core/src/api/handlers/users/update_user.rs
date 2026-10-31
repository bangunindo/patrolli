// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::handlers::users::{USER_COLUMNS, User, hash_password, not_found, row_to_user};
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::models::serialize_utc_as_local;
use crate::db::schema::{AdmStatus, RefreshTokens, Users};
use axum::extract::{Path, State};
use axum::{Json, http::StatusCode};
use chrono::{DateTime, Utc};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;
use utoipa::ToSchema;
use uuid::Uuid;

/// Fields left out are kept as they are.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateUserRequest {
    #[schema(example = "operator")]
    pub username: Option<String>,
    #[schema(example = "new-password")]
    pub password: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    #[schema(value_type = i16)]
    pub adm_status: Option<AdmStatus>,
    pub allow_sso: Option<bool>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UpdateUserResponse {
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

impl From<User> for UpdateUserResponse {
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

/// Update a user
///
/// Only the fields present in the body are changed. This is also where another
/// user's password is changed, without their current password. It is meant for
/// admins and will be protected by RBAC in the future.
#[utoipa::path(
    patch,
    path = "/users/{user_id}",
    params(
        ("user_id" = Uuid, Path, description = "User UUID")
    ),
    request_body(
        content = UpdateUserRequest,
        description = "Update user request body",
        content_type = "application/json",
    ),
    responses(
        (status = 200, description = "User updated", body = ResponseData<UpdateUserResponse>),
        (status = 400, description = "Validation error", body = ResponseError),
        (status = 404, description = "User not found", body = ResponseError),
        (status = 409, description = "Username already exists", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "users",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn update_user(
    State(state): State<Arc<AppState>>,
    Path(user_id): Path<Uuid>,
    Json(payload): Json<UpdateUserRequest>,
) -> Result<ResponseData<UpdateUserResponse>, ResponseError> {
    // validate request
    if payload
        .username
        .as_ref()
        .is_some_and(|name| name.trim().is_empty())
    {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Validation error")
            .with_detail("Username cannot be empty"));
    }
    if payload
        .username
        .as_ref()
        .is_some_and(|name| name.trim() != name)
    {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Validation error")
            .with_detail("Username cannot start or end with whitespace"));
    }
    if payload
        .password
        .as_ref()
        .is_some_and(|password| password.trim().is_empty())
    {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Validation error")
            .with_detail("Password cannot be empty"));
    }

    let mut client = state.db_pool.get().await.ctx("get database connection")?;
    let tx = client.transaction().await.ctx("start transaction")?;

    // check new username is not taken
    if let Some(username) = &payload.username {
        let (sql, values) = DbQuery::select()
            .column(Users::UserId)
            .from(Users::Table)
            .and_where(Expr::col(Users::Username).eq(username))
            .and_where(Expr::col(Users::UserId).ne(user_id))
            .build_postgres(PostgresQueryBuilder);
        let taken = tx
            .query_opt(sql.as_str(), &values.as_params())
            .await
            .ctx("check username")?;
        if taken.is_some() {
            return Err(ResponseError::from_status(StatusCode::CONFLICT)
                .with_title("Username already exists")
                .with_detail("Choose a different username."));
        }
    }

    let password_changed = payload.password.is_some();
    let adm_status_changed = payload.adm_status.is_some();
    let disabled = matches!(payload.adm_status, Some(AdmStatus::Disabled));

    // update only the fields that were sent
    let mut query = DbQuery::update();
    query
        .table(Users::Table)
        .value(Users::LastUpdated, Utc::now())
        .and_where(Expr::col(Users::UserId).eq(user_id))
        .returning(DbQuery::returning().columns(USER_COLUMNS));
    if let Some(username) = payload.username {
        query.value(Users::Username, username);
    }
    if let Some(password) = payload.password {
        query.value(Users::PasswordHash, hash_password(password));
    }
    if let Some(display_name) = payload.display_name {
        query.value(Users::DisplayName, display_name);
    }
    if let Some(email) = payload.email {
        query.value(Users::Email, email);
    }
    if let Some(adm_status) = payload.adm_status {
        query.value(Users::AdmStatus, adm_status as i16);
    }
    if let Some(allow_sso) = payload.allow_sso {
        query.value(Users::AllowSso, allow_sso);
    }
    let (sql, values) = query.build_postgres(PostgresQueryBuilder);

    let row = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("update user")?
        .ok_or_else(|| not_found(user_id))?;
    let updated = row_to_user(&row).ctx("parse user")?;

    // end the user's sessions when disabled or given a new password
    if disabled || password_changed {
        let (sql, values) = DbQuery::delete()
            .from_table(RefreshTokens::Table)
            .and_where(Expr::col(RefreshTokens::UserId).eq(user_id))
            .build_postgres(PostgresQueryBuilder);
        tx.execute(sql.as_str(), &values.as_params())
            .await
            .ctx("revoke refresh tokens")?;
    }
    tx.commit().await.ctx("commit transaction")?;
    info!(
        target_user_id = %updated.user_id,
        target_user = %updated.username,
        password_changed,
        adm_status_changed,
        adm_status = ?updated.adm_status,
        allow_sso = updated.allow_sso,
        "user updated"
    );

    Ok(ResponseData::from_status(StatusCode::OK).with_data(updated.into()))
}
