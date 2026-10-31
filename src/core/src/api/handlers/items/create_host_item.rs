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
pub struct CreateHostItemRequest {
    pub listener_id: Option<Uuid>,
    pub endpoint_id: Option<Uuid>,
    #[schema(default = 0)]
    pub flags: i32,
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    pub name: String,
    pub schedule: String,
    pub timeout_ms: i32,
    pub config: Config,
    pub labels: serde_json::Value,
}

impl CreateHostItemRequest {
    pub fn validate(&self) -> anyhow::Result<()> {
        let mut item = Item::new(
            Uuid::now_v7(), // placeholder
            self.name.to_smolstr(),
            self.endpoint_id,
            Uuid::now_v7(), // placeholder
            Schedule::from_str(&self.schedule)?,
            self.listener_id,
            Duration::from_millis(self.timeout_ms as u64),
            vec![],
            self.config.clone(),
        );
        item.validate()?;

        if self.name.trim().is_empty() {
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateHostItemResponse {
    pub item_id: Uuid,
    pub listener_id: Option<Uuid>,
    pub endpoint_id: Option<Uuid>,
    #[schema(default = 0)]
    pub flags: i32,
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    pub name: String,
    pub schedule: String,
    #[schema(default = 0)]
    pub timeout_ms: i32,
    pub config: Config,
    pub labels: serde_json::Value,
}

/// Create an item for a host
#[utoipa::path(
    post,
    path = "/items/host/{host_id}",
     params(
        ("host_id" = Uuid, Path, description = "Host UUID")
    ),
    request_body(
        content = CreateHostItemRequest,
        description = "Create host item request body",
        content_type = "application/json",
    ),
    responses(
        (status = 201, description = "Item created", body = ResponseData<CreateHostItemResponse>),
        (status = 404, description = "Host template not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "items",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn create_host_item(
    State(state): State<Arc<AppState>>,
    Path(host_id): Path<Uuid>,
    Json(payload): Json<CreateHostItemRequest>,
) -> Result<ResponseData<CreateHostItemResponse>, ResponseError> {
    payload.validate().bad_request("Validation error")?;

    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // get host_templates for the host
    let mut query = DbQuery::select();
    query
        .column((HostTemplates::Table, HostTemplates::HostTemplateId))
        .from(HostTemplates::Table)
        .and_where(
            Expr::col(HostTemplates::HostId)
                .eq(host_id)
                .and(Expr::col(HostTemplates::TemplateId).is_null()),
        );

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let row_opt = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    let Some(row) = row_opt else {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Host template not found")
            .with_detail(format!("No host template found for host_id: {}", host_id)));
    };

    let host_template_id = row.get::<_, Uuid>("host_template_id");

    // Validate endpoint type and endpoint's host_id
    if let Some(endpoint_id) = payload.endpoint_id {
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

    // create item for current host
    let base_item_id = Uuid::now_v7();
    let now = chrono::Utc::now();
    let (sql, values) = DbQuery::insert()
        .into_table(Items::Table)
        .columns([
            Items::ItemId,
            Items::HostTemplateId,
            Items::Type,
            Items::ListenerId,
            Items::EndpointId,
            Items::Flags,
            Items::AdmStatus,
            Items::Name,
            Items::Schedule,
            Items::TimeoutMs,
            Items::LastUpdated,
            Items::Config,
            Items::Labels,
        ])
        .values([
            base_item_id.into(),
            host_template_id.into(),
            ItemKind::from(&payload.config).to_string().into(),
            payload.listener_id.into(),
            payload.endpoint_id.into(),
            payload.flags.into(),
            (payload.adm_status as i16).into(),
            payload.name.clone().into(),
            payload.schedule.clone().into(),
            payload.timeout_ms.into(),
            now.into(),
            payload
                .config
                .to_json_value()
                .ctx("serialize config")?
                .into(),
            payload.labels.clone().into(),
        ])
        .ctx("build insert query")?
        .build_postgres(PostgresQueryBuilder);

    _ = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    tx.commit().await.ctx("commit transaction")?;

    let response = CreateHostItemResponse {
        item_id: base_item_id,
        listener_id: payload.listener_id,
        endpoint_id: payload.endpoint_id,
        flags: payload.flags,
        adm_status: payload.adm_status,
        name: payload.name,
        schedule: payload.schedule,
        timeout_ms: payload.timeout_ms,
        config: payload.config,
        labels: payload.labels,
    };

    Ok(ResponseData::from_status(StatusCode::CREATED).with_data(response))
}
