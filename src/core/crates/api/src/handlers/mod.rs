// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use sea_query::Order;
use serde::Deserialize;
use utoipa::ToSchema;

pub(super) mod auth;
pub(super) mod endpoints;
pub(super) mod health;
pub(super) mod hosts;
pub(super) mod items;
pub(super) mod listeners;
pub(super) mod nodes;
pub(super) mod pipelines;
pub(super) mod processor_builder;
pub(super) mod processors;
pub(super) mod sinks;
pub(super) mod templates;
pub(super) mod users;

#[derive(Default, Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum OrderByDirection {
    #[default]
    Asc,
    Desc,
}

impl From<OrderByDirection> for Order {
    fn from(direction: OrderByDirection) -> Self {
        match direction {
            OrderByDirection::Asc => Order::Asc,
            OrderByDirection::Desc => Order::Desc,
        }
    }
}
