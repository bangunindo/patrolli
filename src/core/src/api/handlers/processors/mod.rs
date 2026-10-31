// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod create_processor;
pub mod delete_processor;
pub mod list_processors;
pub mod subscribe_capture;
pub mod update_processor;

pub use create_processor::create_processor;
pub use delete_processor::delete_processor;
pub use list_processors::{ProcessorOrderBy, list_processors};
pub use update_processor::update_processor;

use crate::config::processor::{Config, FilterValue, ProcessorKind};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        list_processors::list_processors,
        create_processor::create_processor,
        update_processor::update_processor,
        delete_processor::delete_processor,
    ),
    components(schemas(ProcessorOrderBy, Config, FilterValue, ProcessorKind))
)]
pub struct ProcessorsHandler;
