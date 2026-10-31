// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::config::endpoint::EndpointKind;
use crate::config::item::{Config, Item, ItemKind, Schedule};
use crate::db::schema::{AdmStatus, HostEndpoints, HostTemplates, Items, default_adm_status};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use smol_str::ToSmolStr;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateItemRequest {
    pub listener_id: Option<Uuid>,
    pub endpoint_id: Option<Uuid>,
    #[schema(default = 0)]
    pub flags: i32,
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    // Option<T> to accomodate applied template items update where name, schedule, timeout_ms can be null (inherits from template)
    // TODO: create separate update endpoints for host items, template items, and applied template items
    pub name: Option<String>,
    pub schedule: Option<String>,
    pub timeout_ms: Option<i32>,
    pub config: Config,
    pub labels: serde_json::Value,
}

impl UpdateItemRequest {
    pub fn validate(&self) -> anyhow::Result<()> {
        let mut item = Item::new(
            Uuid::now_v7(), // placeholder
            self.name.clone().unwrap_or("dummy".into()).to_smolstr(),
            self.endpoint_id,
            Uuid::now_v7(), // placeholder
            Schedule::from_str(&self.schedule.clone().unwrap_or("0".into()))?,
            self.listener_id,
            Duration::from_millis(self.timeout_ms.unwrap_or(0) as u64),
            vec![],
            self.config.clone(),
        );
        item.validate()?;

        if let Some(name) = &self.name
            && name.trim().is_empty()
        {
            anyhow::bail!("Item name cannot be empty");
        }

        match ItemKind::from(&self.config) {
            ItemKind::Listener => {
                if self.listener_id.is_none() {
                    anyhow::bail!("listener_id is required for listener item type");
                }
                if self.endpoint_id.is_some() {
                    anyhow::bail!("endpoint_id is not allowed for listener item type");
                }
            }
            ItemKind::Snmp | ItemKind::SnmpWalk | ItemKind::PrometheusScrape => {
                if self.endpoint_id.is_none() {
                    anyhow::bail!(
                        "endpoint_id is required for snmp/snmp_walk/prometheus item type"
                    );
                }
                if self.listener_id.is_some() {
                    anyhow::bail!(
                        "listener_id is not allowed for snmp/snmp_walk/prometheus item type"
                    );
                }
            }
            ItemKind::Dummy => {
                if self.listener_id.is_some() {
                    anyhow::bail!("listener_id is not allowed for dummy item type");
                }
                if self.endpoint_id.is_some() {
                    anyhow::bail!("endpoint_id is not allowed for dummy item type");
                }
            }
        }

        Ok(())
    }
}

/// Update an item by ID
///
/// Deprecated: use PUT /items/{item_id}/host, PUT /items/{item_id}/template or
/// PUT /items/{item_id}/applied-template instead.
#[utoipa::path(
	put,
	path = "/items/{item_id}",
	params(
		("item_id" = Uuid, Path, description = "Item UUID")
	),
	request_body = UpdateItemRequest,
    responses(
        (status = 200, description = "Item updated", body = ResponseData<UpdateItemRequest>),
        (status = 404, description = "Item not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
	tag = "items",
    security(
        ("bearerAuth" = [])
    )
)]
#[deprecated = "use update_host_item, update_template_item or update_applied_template_item instead"]
pub async fn update_item(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    Json(payload): Json<UpdateItemRequest>,
) -> Result<ResponseData<UpdateItemRequest>, ResponseError> {
    payload.validate().bad_request("Validation error")?;

    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // Get existing host_id (if non null, its a host item, if null its a template item)
    let mut query = DbQuery::select();
    query
        .column((Items::Table, Items::ItemId))
        .column((HostTemplates::Table, HostTemplates::HostId))
        .from(Items::Table)
        .inner_join(
            HostTemplates::Table,
            Expr::col((Items::Table, Items::HostTemplateId))
                .equals((HostTemplates::Table, HostTemplates::HostTemplateId)),
        )
        .and_where(Expr::col((Items::Table, Items::ItemId)).eq(item_id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let row_opt = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    let row = row_opt.ok_or_else(|| {
        ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Item not found")
            .with_detail(format!("No item found for item_id: {}", item_id))
    })?;

    let host_id = row.get::<_, Option<Uuid>>("host_id");

    if host_id.is_none() && (payload.endpoint_id.is_some() || payload.listener_id.is_some()) {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Endpoint ID provided for template item")
            .with_detail("Cannot associate an endpoint with a template item"));
    }

    // Validate endpoint type and endpoint's host_id
    if let (Some(endpoint_id), Some(host_id)) = (payload.endpoint_id, host_id) {
        let mut query = DbQuery::select();
        query
            .column((HostEndpoints::Table, HostEndpoints::Type))
            .column((HostEndpoints::Table, HostEndpoints::HostId))
            .from(HostEndpoints::Table)
            .and_where(
                Expr::col((HostEndpoints::Table, HostEndpoints::EndpointId)).eq(endpoint_id),
            );

        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        let row_opt = tx
            .query_opt(sql.as_str(), &values.as_params())
            .await
            .ctx("execute query")?;

        let Some(row) = row_opt else {
            return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
                .with_title("Endpoint not found")
                .with_detail(format!(
                    "Endpoint with id {} not found",
                    payload.endpoint_id.unwrap_or(Uuid::nil())
                )));
        };

        let endpoint_type_str: String = row.get("type");
        let endpoint_type =
            EndpointKind::from_str(&endpoint_type_str).ctx("parse endpoint type")?;

        if row.get::<_, Uuid>("host_id") != host_id {
            return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
                .with_title("Endpoint does not belong to the specified host")
                .with_detail(format!(
                    "Endpoint with id {} does not belong to host with id {}",
                    endpoint_id, host_id
                )));
        }

        match ItemKind::from(&payload.config) {
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
    }

    let now = chrono::Utc::now();
    let (sql, values) = DbQuery::update()
        .table(Items::Table)
        .and_where(Expr::col(Items::ItemId).eq(item_id))
        .value(Items::ListenerId, payload.listener_id)
        .value(Items::EndpointId, payload.endpoint_id)
        .value(Items::Type, ItemKind::from(&payload.config).to_string())
        .value(Items::Flags, payload.flags)
        .value(Items::AdmStatus, payload.adm_status as i16)
        .value(Items::Name, payload.name.clone())
        .value(Items::Schedule, payload.schedule.clone())
        .value(Items::TimeoutMs, payload.timeout_ms)
        .value(Items::LastUpdated, now)
        .value(
            Items::Config,
            payload.config.to_json_value().ctx("serialize config")?,
        )
        .value(Items::Labels, payload.labels.clone())
        .build_postgres(PostgresQueryBuilder);

    let affected = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("execute item update query")?;

    if affected == 0 {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Item not found")
            .with_detail(format!("No item found for item_id: {}", item_id)));
    }

    tx.commit().await.ctx("commit transaction")?;

    Ok(ResponseData::from_status(StatusCode::OK).with_data(payload))
}
