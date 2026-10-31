// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use crate::api::error::ApiResultExt;
use crate::api::handlers::items::item_kind_unchanged;
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

/// A root template item owns its values, so name, schedule, timeout_ms and
/// config are mandatory. A template item linked from another template inherits
/// them, so they are optional there and only need to be set to override.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateTemplateItemRequest {
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

impl UpdateTemplateItemRequest {
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
                None,
                Uuid::now_v7(), // placeholder
                schedule,
                None,
                Duration::from_millis(self.timeout_ms.unwrap_or(0) as u64),
                vec![],
                config.clone(),
            );
            item.validate()?;
        }

        Ok(())
    }

    /// A root template item has nothing to inherit from, so it has to carry all
    /// of its values.
    pub fn validate_root(&self) -> anyhow::Result<()> {
        if self.name.is_none() {
            anyhow::bail!(
                "field `name` is required for a template item that is not linked to a template"
            );
        }
        if self.schedule.is_none() {
            anyhow::bail!(
                "field `schedule` is required for a template item that is not linked to a template"
            );
        }
        if self.timeout_ms.is_none() {
            anyhow::bail!(
                "field `timeout_ms` is required for a template item that is not linked to a template"
            );
        }
        if self.config.is_none() {
            anyhow::bail!(
                "field `config` is required for a template item that is not linked to a template"
            );
        }

        Ok(())
    }
}

/// Update a template item by ID
///
/// Only accepts items that belong to a template, they have no listener or
/// endpoint association. An item linked from another template inherits name,
/// schedule, timeout_ms and config: leave them null to keep inheriting, or set
/// them to override. On a template item that is not linked to a template they
/// are mandatory. A config cannot change the item type, not even on a template
/// item that is not linked to a template, since the items derived from it keep
/// their own type.
#[utoipa::path(
	put,
	path = "/items/{item_id}/template",
	params(
		("item_id" = Uuid, Path, description = "Item UUID")
	),
	request_body = UpdateTemplateItemRequest,
    responses(
        (status = 200, description = "Item updated", body = ResponseData<UpdateTemplateItemRequest>),
        (status = 400, description = "Item is not a template item", body = ResponseError),
        (status = 404, description = "Item not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
	tag = "items",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn update_template_item(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    Json(payload): Json<UpdateTemplateItemRequest>,
) -> Result<ResponseData<UpdateTemplateItemRequest>, ResponseError> {
    payload.validate().bad_request("Validation error")?;

    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // A template item has no host_id. It is either a root item of this template
    // or an item linked from another template, the latter has a template_item_id
    // to inherit from.
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

    let template_item_id = row.get::<_, Option<Uuid>>("template_item_id");

    if row.get::<_, Option<Uuid>>("host_id").is_some() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Item is not a template item")
            .with_detail(format!(
                "Item with id {} belongs to a host, use PUT /items/{{item_id}}/{} instead",
                item_id,
                if template_item_id.is_some() {
                    "applied-template"
                } else {
                    "host"
                }
            )));
    }

    // a root item owns every value, an item linked from another template only
    // owns the ones it overrides
    if template_item_id.is_none() {
        payload.validate_root().bad_request("Validation error")?;
    }

    // the kind is fixed at creation, items.type is never rewritten here
    item_kind_unchanged(item_id, row.get("type"), payload.config.as_ref())?;

    let now = chrono::Utc::now();
    let (sql, values) = DbQuery::update()
        .table(Items::Table)
        .and_where(Expr::col(Items::ItemId).eq(item_id))
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
