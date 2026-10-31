// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::items::{ItemsStatsResponse, collect_item_stats};
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::config::item::ItemKind;
use crate::db::models::serialize_utc_as_local;
use crate::db::query::{ItemColumn, ItemParams, items};
use crate::db::schema::{AdmStatus, Items, default_adm_status};
use axum::{
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct ItemSchema {
    pub item_id: Uuid,
    pub host_template_id: Uuid,
    pub template_item_id: Option<Uuid>,
    pub listener_id: Option<Uuid>,
    pub listener_name: Option<String>,
    pub endpoint_id: Option<Uuid>,
    pub endpoint_name: Option<String>,
    pub r#type: ItemKind,
    pub flags: i32,
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    pub name: Option<String>,
    pub schedule: Option<String>,
    pub timeout_ms: Option<i32>,
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub last_updated: DateTime<Utc>,
    pub config: Option<serde_json::Value>,
    pub labels: Option<serde_json::Value>,

    pub config_embed: serde_json::Value,
    pub outputs: Vec<Option<Uuid>>,
    pub host_id: Option<Uuid>,
    pub host_name: Option<String>,
    pub monitored_by: Option<Uuid>,
    pub template_id: Option<Uuid>,
    pub template_name: Option<String>,

    pub resolved_adm_status: Option<i16>,
    pub resolved_name: Option<String>,
    pub resolved_schedule: Option<String>,
    pub resolved_timeout_ms: Option<i32>,
    pub resolved_config: Option<serde_json::Value>,
    pub resolved_config_embed: Option<serde_json::Value>,
    pub resolved_outputs: Option<Vec<Option<Uuid>>>,
    pub root_item_id: Uuid,
    pub root_template_id: Option<Uuid>,
    pub root_template_name: Option<String>,

    pub stats: Option<ItemsStatsResponse>,
}

impl TryFrom<&tokio_postgres::Row> for ItemSchema {
    type Error = Box<dyn std::error::Error>;

    fn try_from(row: &tokio_postgres::Row) -> Result<Self, Self::Error> {
        Ok(Self {
            item_id: row.try_get(ItemColumn::ItemId.as_str())?,
            host_template_id: row.try_get(ItemColumn::HostTemplateId.as_str())?,
            template_item_id: row.try_get(ItemColumn::TemplateItemId.as_str())?,
            listener_id: row.try_get(ItemColumn::ListenerId.as_str())?,
            listener_name: row.try_get(ItemColumn::ListenerName.as_str())?,
            endpoint_id: row.try_get(ItemColumn::EndpointId.as_str())?,
            endpoint_name: row.try_get(ItemColumn::EndpointName.as_str())?,
            r#type: row
                .try_get::<&'static str, String>(ItemColumn::Type.as_str())?
                .parse()?,
            flags: row.try_get(ItemColumn::Flags.as_str())?,
            adm_status: row
                .try_get::<_, i16>(ItemColumn::AdmStatus.as_str())?
                .try_into()?,
            name: row.try_get(ItemColumn::Name.as_str())?,
            schedule: row.try_get(ItemColumn::Schedule.as_str())?,
            timeout_ms: row.try_get(ItemColumn::TimeoutMs.as_str())?,
            last_updated: row.try_get(ItemColumn::LastUpdated.as_str())?,
            config: row.try_get(ItemColumn::Config.as_str())?,
            labels: row.try_get(ItemColumn::Labels.as_str())?,

            config_embed: row.try_get(ItemColumn::ConfigEmbed.as_str())?,
            outputs: row.try_get(ItemColumn::Outputs.as_str())?,
            host_id: row.try_get(ItemColumn::HostId.as_str())?,
            host_name: row.try_get(ItemColumn::HostName.as_str())?,
            monitored_by: row.try_get(ItemColumn::MonitoredBy.as_str())?,
            template_id: row.try_get(ItemColumn::TemplateId.as_str())?,
            template_name: row.try_get(ItemColumn::TemplateName.as_str())?,

            resolved_adm_status: row.try_get(ItemColumn::ResolvedAdmStatus.as_str())?,
            resolved_name: row.try_get(ItemColumn::ResolvedName.as_str())?,
            resolved_schedule: row.try_get(ItemColumn::ResolvedSchedule.as_str())?,
            resolved_timeout_ms: row.try_get(ItemColumn::ResolvedTimeoutMs.as_str())?,
            resolved_config: row.try_get(ItemColumn::ResolvedConfig.as_str())?,
            resolved_config_embed: row.try_get(ItemColumn::ResolvedConfigEmbed.as_str())?,
            resolved_outputs: row.try_get(ItemColumn::ResolvedOutputs.as_str())?,
            root_item_id: row.try_get(ItemColumn::RootItemId.as_str())?,
            root_template_id: row.try_get(ItemColumn::RootTemplateId.as_str())?,
            root_template_name: row.try_get(ItemColumn::RootTemplateName.as_str())?,

            // Not stored in DB
            stats: None,
        })
    }
}

/// Get an item by ID
#[utoipa::path(
	get,
	path = "/items/{item_id}",
	params(
		("item_id" = Uuid, Path, description = "Item UUID")
	),
    responses(
        (status = 200, description = "Item found", body = ResponseData<ItemSchema>),
        (status = 404, description = "Item not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
	tag = "items",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn get_item(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
) -> Result<ResponseData<ItemSchema>, ResponseError> {
    let client = state.db_pool.get().await.ctx("get database connection")?;

    let mut params = ItemParams::new();
    params.with_columns(vec![
        ItemColumn::ItemId,
        ItemColumn::HostTemplateId,
        ItemColumn::TemplateItemId,
        ItemColumn::ListenerId,
        ItemColumn::ListenerName,
        ItemColumn::EndpointId,
        ItemColumn::EndpointName,
        ItemColumn::Type,
        ItemColumn::Flags,
        ItemColumn::AdmStatus,
        ItemColumn::Name,
        ItemColumn::Schedule,
        ItemColumn::TimeoutMs,
        ItemColumn::LastUpdated,
        ItemColumn::Config,
        ItemColumn::Labels,
        ItemColumn::ConfigEmbed,
        ItemColumn::Outputs,
        ItemColumn::HostId,
        ItemColumn::HostName,
        ItemColumn::MonitoredBy,
        ItemColumn::TemplateId,
        ItemColumn::TemplateName,
        ItemColumn::ResolvedAdmStatus,
        ItemColumn::ResolvedName,
        ItemColumn::ResolvedSchedule,
        ItemColumn::ResolvedTimeoutMs,
        ItemColumn::ResolvedConfig,
        ItemColumn::ResolvedConfigEmbed,
        ItemColumn::ResolvedOutputs,
        ItemColumn::RootItemId,
        ItemColumn::RootTemplateId,
        ItemColumn::RootTemplateName,
    ]);

    let subquery: Expr = DbQuery::select()
        .column(Items::ItemId)
        .from(Items::Table)
        .and_where(Expr::col(Items::ItemId).eq(item_id))
        .to_owned()
        .into();

    params.with_id_subquery_filter(Some(subquery));

    let item_query = items(params);

    let (sql, values) = item_query.build_postgres(PostgresQueryBuilder);

    let row_opt = client
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("execute item query")?;

    let Some(row) = &row_opt else {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Item not found")
            .with_detail(format!("No item found for item_id: {}", item_id)));
    };
    let mut resp: ItemSchema = row.try_into().ctx("parse item from database row")?;

    // Collect item stats from peers
    let collected_item_stats = collect_item_stats(state, vec![item_id])
        .await
        .remove(&item_id);

    resp.stats = collected_item_stats;

    Ok(ResponseData::from_status(StatusCode::OK).with_data(resp))
}
