// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod create_sink;
pub mod delete_sink;
pub mod get_sink;
pub mod list_sinks;
pub mod update_sink;

pub use create_sink::create_sink;
pub use delete_sink::delete_sink;
pub use get_sink::get_sink;
pub use list_sinks::{SinkOrderBy, list_sinks};
pub use update_sink::update_sink;

use crate::config::sink::{Config, SinkKind};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        list_sinks::list_sinks,
        create_sink::create_sink,
        get_sink::get_sink,
        update_sink::update_sink,
        delete_sink::delete_sink,
    ),
    components(schemas(SinkOrderBy, Config, SinkKind))
)]
pub struct SinksHandler;
