// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::OrderByDirection;
use crate::api::error::ApiResultExt;
use crate::api::handlers::users::{USER_COLUMNS, User, row_to_user};
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::models::serialize_utc_as_local;
use crate::db::query::generate_count_query;
use crate::db::schema::{AdmStatus, Users};
use axum::extract::State;
use axum::http::StatusCode;
use axum_extra::extract::Query as ApiQuery;
use chrono::{DateTime, Utc};
use sea_query::extension::postgres::PgExpr;
use sea_query::{
    ColumnRef, Expr, ExprTrait, IntoColumnRef, PostgresQueryBuilder, Query as DbQuery,
};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListUserRequest {
    pub user_ids: Option<Vec<Uuid>>,
    pub username: Option<String>,
    #[serde(default = "default_list_user_limit")]
    pub limit: u64,
    #[serde(default = "default_list_user_offset")]
    pub offset: u64,
    #[serde(default)]
    pub order_by: UserOrderBy,
    #[serde(default)]
    pub order_dir: OrderByDirection,
}

fn default_list_user_limit() -> u64 {
    20
}

fn default_list_user_offset() -> u64 {
    0
}

#[derive(Default, Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UserOrderBy {
    #[default]
    Username,
    LastUpdated,
}

impl From<UserOrderBy> for ColumnRef {
    fn from(value: UserOrderBy) -> Self {
        match value {
            UserOrderBy::Username => Users::Username,
            UserOrderBy::LastUpdated => Users::LastUpdated,
        }
        .into_column_ref()
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ListUserResponseInner {
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

impl From<User> for ListUserResponseInner {
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

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ListUserResponse {
    pub users: Vec<ListUserResponseInner>,
    pub total: u32,
}

/// List users
#[utoipa::path(
    get,
    path = "/users",
    params(ListUserRequest),
    responses(
        (status = 200, description = "List of users", body = ResponseData<ListUserResponse>),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "users",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn list_users(
    State(state): State<Arc<AppState>>,
    ApiQuery(params): ApiQuery<ListUserRequest>,
) -> Result<ResponseData<ListUserResponse>, ResponseError> {
    let client = state.db_pool.get().await.ctx("get database connection")?;

    // list users, filtered
    let mut query = DbQuery::select();
    query.columns(USER_COLUMNS).from(Users::Table);

    if let Some(username) = &params.username {
        query.and_where(Expr::col(Users::Username).ilike(format!("%{}%", username)));
    }

    if let Some(user_ids) = &params.user_ids {
        query.and_where(Expr::col(Users::UserId).is_in(user_ids.clone()));
    }

    let total_query = generate_count_query(&query);

    query.order_by(params.order_by, params.order_dir.into());
    query.limit(params.limit).offset(params.offset);

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let rows = client
        .query(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    let users = rows
        .iter()
        .map(row_to_user)
        .collect::<Result<Vec<User>, _>>()
        .ctx("parse users")?
        .into_iter()
        .map(ListUserResponseInner::from)
        .collect();

    // count all matches, ignoring limit and offset
    let (sql, values) = total_query.build_postgres(PostgresQueryBuilder);
    let total: i64 = client
        .query_one_scalar(sql.as_str(), &values.as_params())
        .await
        .ctx("execute total count query")?;

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(ListUserResponse {
            users,
            total: total as u32,
        }),
    )
}
