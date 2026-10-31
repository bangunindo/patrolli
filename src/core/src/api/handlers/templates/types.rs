// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::OrderByDirection;
use crate::db::schema::Templates;
use sea_query::{ColumnRef, IntoColumnRef};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

#[derive(Default, Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TemplateOrderBy {
    #[default]
    TemplateId,
    Name,
}

impl From<TemplateOrderBy> for ColumnRef {
    fn from(val: TemplateOrderBy) -> Self {
        match val {
            TemplateOrderBy::TemplateId => Templates::TemplateId,
            TemplateOrderBy::Name => Templates::Name,
        }
        .into_column_ref()
    }
}

#[derive(Clone, Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListTemplateParams {
    pub name: Option<String>,
    #[serde(default = "default_list_template_limit")]
    pub limit: u64,
    #[serde(default = "default_list_template_offset")]
    pub offset: u64,
    #[serde(default)]
    pub order_by: TemplateOrderBy,
    #[serde(default)]
    pub order_dir: OrderByDirection,
}

fn default_list_template_limit() -> u64 {
    20
}
fn default_list_template_offset() -> u64 {
    0
}
