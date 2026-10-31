// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod create_listener;
pub mod delete_listener;
pub mod list_listeners;
pub mod update_listener;

pub use create_listener::create_listener;
pub use delete_listener::delete_listener;
pub use list_listeners::{ListenerOrderBy, list_listeners};
pub use update_listener::update_listener;

use crate::config::listener::{Config, ListenerKind};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        list_listeners::list_listeners,
        create_listener::create_listener,
        update_listener::update_listener,
        delete_listener::delete_listener,
    ),
    components(schemas(ListenerOrderBy, Config, ListenerKind))
)]
pub struct ListenersHandler;
