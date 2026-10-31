// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod create_user;
pub mod delete_user;
pub mod get_user;
pub mod get_user_summary;
pub mod list_users;
pub mod unlink_sso;
pub mod update_user;

pub use create_user::create_user;
pub use delete_user::delete_user;
pub use get_user::get_user;
pub use get_user_summary::get_user_summary;
pub use list_users::{UserOrderBy, list_users};
pub use unlink_sso::unlink_sso;
pub use update_user::update_user;

use crate::api::response::ResponseError;
use crate::db::schema::{AdmStatus, Users};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use sea_query::Expr;
use utoipa::OpenApi;

/// `OidcSubject` is read only to fill `sso_linked`; the value itself is never returned.
const USER_COLUMNS: [Users; 8] = [
    Users::UserId,
    Users::Username,
    Users::DisplayName,
    Users::Email,
    Users::AdmStatus,
    Users::AllowSso,
    Users::OidcSubject,
    Users::LastUpdated,
];

#[derive(OpenApi)]
#[openapi(
    paths(
        list_users::list_users,
        create_user::create_user,
        get_user::get_user,
        get_user_summary::get_user_summary,
        update_user::update_user,
        delete_user::delete_user,
        unlink_sso::unlink_sso,
    ),
    components(schemas(UserOrderBy))
)]
pub struct UsersHandler;

#[derive(Debug, Clone)]
pub struct User {
    pub user_id: uuid::Uuid,
    pub username: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub adm_status: AdmStatus,
    pub allow_sso: bool,
    pub sso_linked: bool,
    pub last_updated: DateTime<Utc>,
}

fn row_to_user(row: &tokio_postgres::Row) -> anyhow::Result<User> {
    let adm_status: i16 = row.try_get("adm_status")?;

    Ok(User {
        user_id: row.try_get("user_id")?,
        username: row.try_get("username")?,
        display_name: row.try_get("display_name")?,
        email: row.try_get("email")?,
        adm_status: adm_status
            .try_into()
            .map_err(|_| anyhow::anyhow!("unknown adm_status: {adm_status}"))?,
        allow_sso: row.try_get("allow_sso")?,
        sso_linked: row.try_get::<_, Option<String>>("oidc_subject")?.is_some(),
        last_updated: row.try_get("last_updated")?,
    })
}

fn hash_password(password: String) -> Expr {
    Expr::cust_with_values("crypt($1, gen_salt('bf', 12))", [password])
}

fn not_found(user_id: uuid::Uuid) -> ResponseError {
    ResponseError::from_status(StatusCode::NOT_FOUND)
        .with_title("User not found")
        .with_detail(format!("No user found for user_id: {}", user_id))
}
