// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod create_host_item;
pub mod create_template_item;
pub mod delete_item;
pub mod get_item;
pub mod list_items;
pub mod update_applied_template_item;
pub mod update_host_item;
pub mod update_item;
pub mod update_template_item;

use crate::db::models::serialize_utc_as_local;
use chrono::{DateTime, Utc};
pub use create_host_item::create_host_item;
pub use create_template_item::create_template_item;
pub use delete_item::delete_item;
pub use get_item::get_item;
pub use list_items::ItemOrderBy;
pub use list_items::{ItemExclusivity, list_items};
use serde::{Deserialize, Serialize};
pub use update_applied_template_item::update_applied_template_item;
pub use update_host_item::update_host_item;
#[allow(deprecated)]
pub use update_item::update_item;
pub use update_template_item::update_template_item;

use axum::http::StatusCode;
use deadpool_postgres::Transaction;
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tracing::warn;
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

use crate::api::error::ApiResultExt;
use crate::api::response::ResponseError;
use crate::api::state::AppState;
use crate::cache::item::StatsSnapshot;
use crate::cluster::inter::v1::ItemStatsRequest;
use crate::config::endpoint::EndpointKind;
use crate::config::item::{Config, ItemKind};
use crate::db::schema::HostEndpoints;
use crate::payload::{KvPair, ValueOwned};

#[derive(OpenApi)]
#[openapi(
    paths(
        create_template_item::create_template_item,
        create_host_item::create_host_item,
        list_items::list_items,
        get_item::get_item,
        update_host_item::update_host_item,
        update_template_item::update_template_item,
        update_applied_template_item::update_applied_template_item,
        update_item::update_item,
        delete_item::delete_item,
    ),
    components(schemas(
        ItemOrderBy,
        Config,
        ItemKind,
        ItemExclusivity,
        ValueOwned,
        KvPair,
        StatsSnapshot
    ))
)]
pub struct ItemsHandler;

/// Check that the endpoint exists, belongs to the given host, and that its type
/// is compatible with the item kind.
pub(crate) async fn validate_endpoint(
    tx: &Transaction<'_>,
    endpoint_id: Uuid,
    host_id: Uuid,
    item_kind: ItemKind,
) -> Result<(), ResponseError> {
    let mut query = DbQuery::select();
    query
        .column((HostEndpoints::Table, HostEndpoints::Type))
        .column((HostEndpoints::Table, HostEndpoints::HostId))
        .from(HostEndpoints::Table)
        .and_where(Expr::col((HostEndpoints::Table, HostEndpoints::EndpointId)).eq(endpoint_id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let row_opt = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    let Some(row) = row_opt else {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Endpoint not found")
            .with_detail(format!("Endpoint with id {} not found", endpoint_id)));
    };

    let endpoint_type_str: String = row.get("type");
    let endpoint_type = EndpointKind::from_str(&endpoint_type_str).ctx("parse endpoint type")?;

    if row.get::<_, Uuid>("host_id") != host_id {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Endpoint does not belong to the specified host")
            .with_detail(format!(
                "Endpoint with id {} does not belong to host with id {}",
                endpoint_id, host_id
            )));
    }

    match item_kind {
        ItemKind::PrometheusScrape => {
            if endpoint_type != EndpointKind::Http {
                return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
                    .with_title("Invalid endpoint type for PrometheusScrape item")
                    .with_detail(format!(
                        "Endpoint with id {} must be of type Http for PrometheusScrape item",
                        endpoint_id
                    )));
            }
        }
        ItemKind::Snmp | ItemKind::SnmpWalk
            if !matches!(
                endpoint_type,
                EndpointKind::SnmpV1 | EndpointKind::SnmpV2C | EndpointKind::SnmpV3
            ) =>
        {
            return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
                    .with_title("Invalid endpoint type for Snmp/SnmpWalk item")
                    .with_detail(format!(
                        "Endpoint with id {} must be of type SnmpV1, SnmpV2C, or SnmpV3 for Snmp/SnmpWalk item",
                        endpoint_id
                    )));
        }
        _ => {}
    }

    Ok(())
}

/// Prevent changing the item kind of template item to prevent a case
/// where a root item has different `type` than its descendant that has their `items.config` set to NULL
/// in which case will cause mismatch between `items.type` and `items.config` at read-time when `items.config` is resolved
pub(crate) fn item_kind_unchanged(
    item_id: Uuid,
    current_type: &str,
    config: Option<&Config>,
) -> Result<ItemKind, ResponseError> {
    let current_kind = ItemKind::from_str(current_type).ctx("parse item type")?;

    if let Some(config) = config {
        let config_kind = ItemKind::from(config);
        if config_kind != current_kind {
            return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
                .with_title("Item type cannot be changed")
                .with_detail(format!(
                    "Item with id {} is of type {}, but the given config is of type {}",
                    item_id, current_kind, config_kind
                )));
        }
    }

    Ok(current_kind)
}

/// Check that the listener/endpoint association matches what the item kind allows.
pub(crate) fn validate_item_kind_association(
    item_kind: ItemKind,
    listener_id: Option<Uuid>,
    endpoint_id: Option<Uuid>,
) -> anyhow::Result<()> {
    match item_kind {
        ItemKind::Listener => {
            if listener_id.is_none() {
                anyhow::bail!("listener_id is required for listener item type");
            }
            if endpoint_id.is_some() {
                anyhow::bail!("endpoint_id is not allowed for listener item type");
            }
        }
        ItemKind::Snmp | ItemKind::SnmpWalk | ItemKind::PrometheusScrape => {
            if endpoint_id.is_none() {
                anyhow::bail!("endpoint_id is required for snmp/snmp_walk/prometheus item type");
            }
            if listener_id.is_some() {
                anyhow::bail!("listener_id is not allowed for snmp/snmp_walk/prometheus item type");
            }
        }
        ItemKind::Dummy => {
            if listener_id.is_some() {
                anyhow::bail!("listener_id is not allowed for dummy item type");
            }
            if endpoint_id.is_some() {
                anyhow::bail!("endpoint_id is not allowed for dummy item type");
            }
        }
    }

    Ok(())
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct ItemsStatsResponse {
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub since: DateTime<Utc>,
    pub snapshot: StatsSnapshot,
}

pub(crate) async fn collect_item_stats(
    state: Arc<AppState>,
    item_ids: Vec<Uuid>,
) -> HashMap<Uuid, ItemsStatsResponse> {
    // An empty page asks for nothing. `InterComm::item_stats` and every node's
    // `gather` already treat empty ids as "no items", so this only skips building
    // a request that would come back empty.
    if item_ids.is_empty() {
        return HashMap::new();
    }

    let req_timeout = std::time::Duration::from_secs(5);

    let req = ItemStatsRequest {
        item_ids: item_ids
            .into_iter()
            .map(|x| x.into_bytes().to_vec().into())
            .collect(),
        ..Default::default()
    };
    let list_item_stats = match state.inter_comm.item_stats(req, req_timeout).await {
        Ok(v) => v,
        Err(e) => {
            // A peer that cannot answer costs us stats for this response, not correctness.
            warn!(error = %e, "item stats unavailable, returning empty stats");
            return HashMap::new();
        }
    };

    let mut out = HashMap::new();
    for s in list_item_stats {
        // only trust stats if `since` is set and `error` is None
        let trusted_since = match (s.since, &s.error) {
            (Some(_), Some(error)) => {
                warn!(error = ?error, "peer reported an error collecting item stats");
                None
            }
            (Some(since), None) => Some(since.with_timezone(&Utc)),
            (None, error) => {
                warn!(error = ?error, "item stats missing the `since` field");
                None
            }
        };
        for (item_id, snapshot) in s.stats {
            // First node holding an item wins,
            // 1 item seen in multiple node, practically would only
            // happen briefly during rebalancing
            if out.contains_key(&item_id) {
                continue;
            }
            if let Some(since) = trusted_since {
                out.insert(item_id, ItemsStatsResponse { since, snapshot });
            }
        }
    }
    out
}
