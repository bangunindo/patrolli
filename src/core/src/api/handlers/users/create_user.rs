// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::handlers::users::{USER_COLUMNS, User, hash_password, row_to_user};
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::models::serialize_utc_as_local;
use crate::db::schema::{AdmStatus, Users, default_adm_status};
use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateUserRequest {
    #[schema(example = "operator")]
    pub username: String,
    #[schema(example = "password")]
    pub password: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    #[serde(default)]
    pub allow_sso: bool,
    #[serde(default = "default_adm_status")]
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreateUserResponse {
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

impl From<User> for CreateUserResponse {
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

/// Create a user
#[utoipa::path(
    post,
    path = "/users",
    request_body(
        content = CreateUserRequest,
        description = "Create user request body",
        content_type = "application/json",
    ),
    responses(
        (status = 201, description = "User created", body = ResponseData<CreateUserResponse>),
        (status = 400, description = "Validation error", body = ResponseError),
        (status = 409, description = "Username already exists", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "users",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn create_user(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateUserRequest>,
) -> Result<ResponseData<CreateUserResponse>, ResponseError> {
    // validate request
    if payload.username.trim().is_empty() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Validation error")
            .with_detail("Username cannot be empty"));
    }
    if payload.username.trim() != payload.username {
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

    // check username is not taken
    let (sql, values) = DbQuery::select()
        .column(Users::UserId)
        .from(Users::Table)
        .and_where(Expr::col(Users::Username).eq(&payload.username))
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

    // insert user
    let mut query = DbQuery::insert();
    query
        .into_table(Users::Table)
        .columns([
            Users::UserId,
            Users::Username,
            Users::PasswordHash,
            Users::DisplayName,
            Users::Email,
            Users::AdmStatus,
            Users::AllowSso,
            Users::LastUpdated,
        ])
        .values([
            Uuid::now_v7().into(),
            payload.username.into(),
            match payload.password {
                Some(password) => hash_password(password),
                None => Option::<String>::None.into(),
            },
            payload.display_name.into(),
            payload.email.into(),
            (payload.adm_status as i16).into(),
            payload.allow_sso.into(),
            Utc::now().into(),
        ])
        .ctx("build insert query")?
        .returning(DbQuery::returning().columns(USER_COLUMNS));
    let (sql, values) = query.build_postgres(PostgresQueryBuilder);

    let row = tx
        .query_one(sql.as_str(), &values.as_params())
        .await
        .ctx("create user")?;
    let created = row_to_user(&row).ctx("parse user")?;
    tx.commit().await.ctx("commit transaction")?;
    info!(
        target_user_id = %created.user_id,
        target_user = %created.username,
        allow_sso = created.allow_sso,
        "user created"
    );

    Ok(ResponseData::from_status(StatusCode::CREATED).with_data(created.into()))
}
