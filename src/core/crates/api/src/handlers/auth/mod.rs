// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod change_password;
pub mod login;
pub mod logout;
pub mod refresh;

pub use change_password::change_password;
pub use login::login;
pub use logout::logout;
pub use refresh::refresh;

use serde::Serialize;
use utoipa::OpenApi;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct TokenResponse {
    pub access_token: String,
    /// Unchanged by a refresh; reuse it until it expires.
    pub refresh_token: String,
    /// Access token lifetime in seconds.
    pub expires_in: i64,
}

#[derive(OpenApi)]
#[openapi(
    paths(
        login::login,
        logout::logout,
        refresh::refresh,
        change_password::change_password
    ),
    components(schemas(
        TokenResponse,
        login::LoginRequest,
        logout::LogoutRequest,
        logout::LogoutResponse,
        refresh::RefreshRequest,
        change_password::ChangePasswordRequest
    ))
)]
pub struct AuthHandler;
