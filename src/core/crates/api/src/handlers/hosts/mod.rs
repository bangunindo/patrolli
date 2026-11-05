// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod create_host;
pub mod delete_host;
pub mod list_hosts;
pub mod update_host;

pub use create_host::create_host;
pub use delete_host::delete_host;
pub use list_hosts::{HostOrderBy, list_hosts};
pub use update_host::update_host;

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        list_hosts::list_hosts,
        update_host::update_host,
        create_host::create_host,
        delete_host::delete_host,
    ),
    components(schemas(HostOrderBy,))
)]
pub struct HostsHandler;
