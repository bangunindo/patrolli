// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use crate::api::error::ApiResultExt;
use crate::api::handlers::items::{validate_endpoint, validate_item_kind_association};
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::config::item::{Config, Item, ItemKind, Schedule};
use crate::db::schema::{AdmStatus, HostTemplates, Items, default_adm_status};
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
pub struct UpdateHostItemRequest {
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

impl UpdateHostItemRequest {
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

        validate_item_kind_association(
            ItemKind::from(&self.config),
            self.listener_id,
            self.endpoint_id,
        )
    }
}

/// Update a host item by ID
///
/// Only accepts items that belong directly to a host, i.e. items that are not
/// template items and not derived from a template item.
#[utoipa::path(
	put,
	path = "/items/{item_id}/host",
	params(
		("item_id" = Uuid, Path, description = "Item UUID")
	),
	request_body = UpdateHostItemRequest,
    responses(
        (status = 200, description = "Item updated", body = ResponseData<UpdateHostItemRequest>),
        (status = 400, description = "Item is not a host item", body = ResponseError),
        (status = 404, description = "Item not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
	tag = "items",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn update_host_item(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    Json(payload): Json<UpdateHostItemRequest>,
) -> Result<ResponseData<UpdateHostItemRequest>, ResponseError> {
    payload.validate().bad_request("Validation error")?;

    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // A host item has a host_id and is not derived from a template item
    let mut query = DbQuery::select();
    query
        .column((Items::Table, Items::TemplateItemId))
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

    let Some(host_id) = row.get::<_, Option<Uuid>>("host_id") else {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Item is not a host item")
            .with_detail(format!(
                "Item with id {} belongs to a template, use PUT /items/{{item_id}}/template instead",
                item_id
            )));
    };

    if row.get::<_, Option<Uuid>>("template_item_id").is_some() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Item is not a host item")
            .with_detail(format!(
                "Item with id {} is derived from a template item, use PUT /items/{{item_id}}/applied-template instead",
                item_id
            )));
    }

    if let Some(endpoint_id) = payload.endpoint_id {
        validate_endpoint(&tx, endpoint_id, host_id, ItemKind::from(&payload.config)).await?;
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
