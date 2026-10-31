// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod create_endpoint;
pub mod delete_endpoint;
pub mod list_endpoints;
pub mod update_endpoint;

pub use create_endpoint::create_endpoint;
pub use delete_endpoint::delete_endpoint;
pub use list_endpoints::{EndpointOrderBy, list_endpoints};
pub use update_endpoint::update_endpoint;

use crate::config::endpoint::{Config, EndpointKind};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        list_endpoints::list_endpoints,
        create_endpoint::create_endpoint,
        update_endpoint::update_endpoint,
        delete_endpoint::delete_endpoint,
    ),
    components(schemas(EndpointOrderBy, Config, EndpointKind))
)]
pub struct EndpointsHandler;
