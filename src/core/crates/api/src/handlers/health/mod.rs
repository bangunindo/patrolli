// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod health_check;

pub use health_check::health_check;

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(paths(health_check::health_check,))]
pub struct HealthHandler;
