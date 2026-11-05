// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod apply_template;
pub mod create_template;
pub mod delete_template;
pub mod get_template;
pub mod link_template;
pub mod list_templates;
pub mod types;
pub mod unapply_template;
pub mod unlink_template;
pub mod update_template;

pub use apply_template::{ApplyTemplatesRequest, apply_template};
pub use create_template::create_template;
pub use delete_template::delete_template;
pub use get_template::get_template;
pub use link_template::link_template;
pub use list_templates::list_templates;
pub use types::TemplateOrderBy;
pub use unapply_template::unapply_template;
pub use unlink_template::unlink_template;
pub use update_template::update_template;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        apply_template::apply_template,
        unapply_template::unapply_template,
        create_template::create_template,
        delete_template::delete_template,
        get_template::get_template,
        list_templates::list_templates,
        update_template::update_template,
        link_template::link_template,
        unlink_template::unlink_template,
    ),
    components(schemas(TemplateOrderBy, ApplyTemplatesRequest))
)]
pub struct TemplatesHandler;
