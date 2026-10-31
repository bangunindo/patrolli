// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use crate::api::error::ApiResultExt;
use crate::api::handlers::items::{
    item_kind_unchanged, validate_endpoint, validate_item_kind_association,
};
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::config::item::{Config, Item, Schedule};
use crate::db::schema::{AdmStatus, Items, default_adm_status};
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

/// An applied template item inherits name, schedule, timeout_ms and config from
/// the template item it is derived from. Any of those fields that is provided
/// overrides the inherited value, and any field left null keeps inheriting.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateAppliedTemplateItemRequest {
    pub listener_id: Option<Uuid>,
    pub endpoint_id: Option<Uuid>,
    #[schema(default = 0)]
    pub flags: i32,
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    pub name: Option<String>,
    pub schedule: Option<String>,
    pub timeout_ms: Option<i32>,
    pub config: Option<Config>,
    pub labels: serde_json::Value,
}

impl UpdateAppliedTemplateItemRequest {
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(name) = &self.name
            && name.trim().is_empty()
        {
            anyhow::bail!("Item name cannot be empty");
        }

        let schedule = Schedule::from_str(self.schedule.as_deref().unwrap_or("0"))?;

        if let Some(config) = &self.config {
            let mut item = Item::new(
                Uuid::now_v7(), // placeholder
                self.name.clone().unwrap_or_default().to_smolstr(),
                self.endpoint_id,
                Uuid::now_v7(), // placeholder
                schedule,
                self.listener_id,
                Duration::from_millis(self.timeout_ms.unwrap_or(0) as u64),
                vec![],
                config.clone(),
            );
            item.validate()?;
        }

        Ok(())
    }
}

/// Update an applied template item by ID
///
/// Only accepts items on a host that are derived from a template item. name,
/// schedule, timeout_ms and config are optional: leave them null to keep
/// inheriting the template item's value, or set them to override it. A config
/// override cannot change the item type.
#[utoipa::path(
	put,
	path = "/items/{item_id}/applied-template",
	params(
		("item_id" = Uuid, Path, description = "Item UUID")
	),
	request_body = UpdateAppliedTemplateItemRequest,
    responses(
        (status = 200, description = "Item updated", body = ResponseData<UpdateAppliedTemplateItemRequest>),
        (status = 400, description = "Item is not an applied template item", body = ResponseError),
        (status = 404, description = "Item not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
	tag = "items",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn update_applied_template_item(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    Json(payload): Json<UpdateAppliedTemplateItemRequest>,
) -> Result<ResponseData<UpdateAppliedTemplateItemRequest>, ResponseError> {
    payload.validate().bad_request("Validation error")?;

    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // An applied template item lives on a host and is derived from a template
    // item. Items derived onto a linked template have no host_id and are
    // handled by update_template_item.
    let row_opt = tx
        .query_opt(
            "SELECT ht.host_id, i.template_item_id, i.type
             FROM items i
             JOIN host_templates ht ON ht.host_template_id = i.host_template_id
             WHERE i.item_id = $1",
            &[&item_id],
        )
        .await
        .ctx("execute query")?;

    let row = row_opt.ok_or_else(|| {
        ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Item not found")
            .with_detail(format!("No item found for item_id: {}", item_id))
    })?;

    let Some(host_id) = row.get::<_, Option<Uuid>>("host_id") else {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Item is not an applied template item")
            .with_detail(format!(
                "Item with id {} belongs to a template, use PUT /items/{{item_id}}/template instead",
                item_id
            )));
    };

    if row.get::<_, Option<Uuid>>("template_item_id").is_none() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Item is not an applied template item")
            .with_detail(format!(
                "Item with id {} is not derived from a template item, use PUT /items/{{item_id}}/host instead",
                item_id
            )));
    }

    // the kind is fixed at creation, items.type is never rewritten here
    let item_kind = item_kind_unchanged(item_id, row.get("type"), payload.config.as_ref())?;

    validate_item_kind_association(item_kind, payload.listener_id, payload.endpoint_id)
        .bad_request("Validation error")?;

    if let Some(endpoint_id) = payload.endpoint_id {
        validate_endpoint(&tx, endpoint_id, host_id, item_kind).await?;
    }

    let now = chrono::Utc::now();
    let (sql, values) = DbQuery::update()
        .table(Items::Table)
        .and_where(Expr::col(Items::ItemId).eq(item_id))
        .value(Items::ListenerId, payload.listener_id)
        .value(Items::EndpointId, payload.endpoint_id)
        .value(Items::Flags, payload.flags)
        .value(Items::AdmStatus, payload.adm_status as i16)
        .value(Items::Name, payload.name.clone())
        .value(Items::Schedule, payload.schedule.clone())
        .value(Items::TimeoutMs, payload.timeout_ms)
        .value(Items::LastUpdated, now)
        .value(
            Items::Config,
            payload
                .config
                .as_ref()
                .map(|config| config.to_json_value())
                .transpose()
                .ctx("serialize config")?,
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
