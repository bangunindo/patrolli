// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::OrderByDirection;
use crate::api::error::ApiResultExt;
use crate::api::items::{ItemsStatsResponse, collect_item_stats};
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::config::item::ItemKind;
use crate::db::models::serialize_utc_as_local;
use crate::db::query::{ItemColumn, ItemParams, items};
use crate::db::schema::{AdmStatus, HostTemplates, Templates, default_adm_status};
use axum::{extract::State, http::StatusCode};
use axum_extra::extract::Query as ApiQuery;
use chrono::{DateTime, Utc};
use sea_query::extension::postgres::{PgBinOper, PgExpr};
use sea_query::{
    BinOper, Expr, ExprTrait, JoinType, PostgresQueryBuilder, Query as DbQuery,
    QueryStatementBuilder,
};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Default, Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ItemOrderBy {
    #[default]
    ItemId,
    Name,
    LastUpdated,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct ListItemResponseInner {
    pub item_id: Uuid,
    pub host_template_id: Uuid,
    pub template_item_id: Option<Uuid>,
    pub template_item_name: Option<String>,
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
    pub labels: Option<serde_json::Value>,

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
    pub resolved_outputs: Option<Vec<Option<Uuid>>>,
    pub root_item_id: Uuid,
    pub root_template_id: Option<Uuid>,
    pub root_template_name: Option<String>,

    pub stats: Option<ItemsStatsResponse>,
}

impl TryFrom<&tokio_postgres::Row> for ListItemResponseInner {
    type Error = Box<dyn std::error::Error>;

    fn try_from(row: &tokio_postgres::Row) -> Result<Self, Self::Error> {
        Ok(Self {
            item_id: row.try_get(ItemColumn::ItemId.as_str())?,
            host_template_id: row.try_get(ItemColumn::HostTemplateId.as_str())?,
            template_item_id: row.try_get(ItemColumn::TemplateItemId.as_str())?,
            template_item_name: row.try_get(ItemColumn::TemplateItemName.as_str())?,
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
            labels: row.try_get(ItemColumn::Labels.as_str())?,

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
            resolved_outputs: row.try_get(ItemColumn::ResolvedOutputs.as_str())?,
            root_item_id: row.try_get(ItemColumn::RootItemId.as_str())?,
            root_template_id: row.try_get(ItemColumn::RootTemplateId.as_str())?,
            root_template_name: row.try_get(ItemColumn::RootTemplateName.as_str())?,

            // Not stored in DB
            stats: None,
        })
    }
}

#[derive(Clone, Debug, Deserialize, ToSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ItemExclusivity {
    Host,
    Template,
    TemplateOnHost,
}

impl From<ItemOrderBy> for Expr {
    fn from(value: ItemOrderBy) -> Self {
        match value {
            ItemOrderBy::ItemId => ItemColumn::ItemId.into(),
            ItemOrderBy::Name => ItemColumn::Name.into(),
            ItemOrderBy::LastUpdated => ItemColumn::LastUpdated.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct ListItemParams {
    pub item_ids: Option<Vec<uuid::Uuid>>,
    pub name: Option<String>,
    #[serde(default = "default_list_item_limit")]
    pub limit: u64,
    #[serde(default = "default_list_item_offset")]
    pub offset: u64,
    #[serde(default)]
    pub order_by: ItemOrderBy,
    #[serde(default)]
    pub order_dir: OrderByDirection,
    pub host_template_ids: Option<Vec<uuid::Uuid>>,
    /// Return items that from specified template IDs. This includes items that are defined in the templates, as well as items that are used by hosts that use the templates.
    pub template_ids: Option<Vec<uuid::Uuid>>,
    /// Return items that are used by the specified host IDs. This includes items from templates used by the hosts, as well as items directly defined on the hosts.
    pub host_ids: Option<Vec<uuid::Uuid>>,
    // /// If true, only return items defined in templates (i.e. exclude items used by hosts).
    // pub template_only: Option<bool>,
    // /// If true, only return items defined on hosts (i.e. exclude template items).
    // pub host_only: Option<bool>,
    /// If specified, include items based on their exclusivity:
    /// - `host`: include items that are directly defined on hosts
    /// - `template`: include items that are defined in templates but not used by any hosts
    /// - `template_on_host`: include items that are defined in templates and used by hosts
    ///
    /// If multiple exclusivity options are specified, items that match any of the specified exclusivity will be included. If no exclusivity is specified, items of all exclusivity will be included.
    pub exclusivity: Option<Vec<ItemExclusivity>>,
    /// If specified, only return items that are associated with the specified endpoint IDs
    pub endpoint_ids: Option<Vec<uuid::Uuid>>,
    /// If specified, only return items that are associated with the specified listener IDs
    pub listener_ids: Option<Vec<uuid::Uuid>>,
    /// If specified, only return items that outputs to the specified processor ID
    pub processor_id: Option<uuid::Uuid>,
    /// If specified, only return items that are of the specified types
    pub types: Option<Vec<ItemKind>>,
    /// If `true``, try to collect ItemStats from peers in cluster.
    pub collect_item_stats: Option<bool>,
}

fn default_list_item_limit() -> u64 {
    20
}

fn default_list_item_offset() -> u64 {
    0
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct ListItemResponse {
    pub items: Vec<ListItemResponseInner>,
    pub total: u32,
}

/// List all items
///
/// Returns all items from the database
#[utoipa::path(
	get,
	path = "/items",
	params(ListItemParams),
	responses(
		(status = 200, description = "List of items", body = ResponseData<ListItemResponse>),
		(status = 500, description = "Internal server error", body = ResponseError)
	),
	tag = "items",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn list_items(
    State(state): State<Arc<AppState>>,
    ApiQuery(params): ApiQuery<ListItemParams>,
) -> Result<ResponseData<ListItemResponse>, ResponseError> {
    let client = state.db_pool.get().await.ctx("get database connection")?;

    let mut query_params = ItemParams::new();
    query_params.with_columns(vec![
        ItemColumn::ItemId,
        ItemColumn::HostTemplateId,
        ItemColumn::TemplateItemId,
        ItemColumn::TemplateItemName,
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
        // ItemColumn::Config,
        ItemColumn::Labels,
        // ItemColumn::ConfigEmbed,
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
        // ItemColumn::ResolvedConfig,
        // ItemColumn::ResolvedConfigEmbed,
        ItemColumn::ResolvedOutputs,
        ItemColumn::RootItemId,
        ItemColumn::RootTemplateId,
        ItemColumn::RootTemplateName,
    ]);

    let mut filters = Vec::new();
    let mut filters_last = Vec::new();
    if let Some(name) = &params.name {
        filters_last.push(
            ItemColumn::Name
                .ilike(format!("%{}%", name))
                .or(ItemColumn::ResolvedName.ilike(format!("%{}%", name))),
        );
    }

    if let Some(item_ids) = &params.item_ids {
        filters.push(ItemColumn::ItemId.is_in(item_ids.clone()));
    }

    if let Some(processor_id) = &params.processor_id {
        filters_last.push(ItemColumn::ResolvedOutputs.binary(
            PgBinOper::Contains,
            sea_query::Value::Array(
                sea_query::ArrayType::Uuid,
                Some(Box::new(vec![sea_query::Value::Uuid(Some(*processor_id))])),
            ),
        ));
    }

    if let Some(endpoint_ids) = &params.endpoint_ids {
        filters.push(ItemColumn::EndpointId.is_in(endpoint_ids.clone()));
    }

    if let Some(listener_ids) = &params.listener_ids {
        filters.push(ItemColumn::ListenerId.is_in(listener_ids.clone()));
    }

    if let Some(types) = &params.types {
        let type_strs: Vec<String> = types.iter().map(|t| t.to_string()).collect();
        filters.push(ItemColumn::Type.is_in(type_strs));
    }

    if let Some(host_template_ids) = params.host_template_ids {
        filters.push(ItemColumn::HostTemplateId.is_in(host_template_ids));
    }

    if let Some(template_ids) = params.template_ids {
        let host_template_ids_subquery: Expr = DbQuery::select()
            .column((HostTemplates::Table, HostTemplates::HostTemplateId))
            .from(HostTemplates::Table)
            .join(
                JoinType::Join,
                Templates::Table,
                Expr::col((Templates::Table, Templates::TemplateId))
                    .eq(Expr::col((HostTemplates::Table, HostTemplates::TemplateId))),
            )
            .and_where(Expr::col((Templates::Table, Templates::TemplateId)).is_in(template_ids))
            .to_owned()
            .into_sub_query_statement()
            .into();

        filters.push(ItemColumn::HostTemplateId.binary(BinOper::In, host_template_ids_subquery));
    }

    if let Some(host_ids) = params.host_ids {
        let host_template_ids_subquery: Expr = DbQuery::select()
            .column((HostTemplates::Table, HostTemplates::HostTemplateId))
            .from(HostTemplates::Table)
            .and_where(Expr::col((HostTemplates::Table, HostTemplates::HostId)).is_in(host_ids))
            .to_owned()
            .into_sub_query_statement()
            .into();

        filters.push(ItemColumn::HostTemplateId.binary(BinOper::In, host_template_ids_subquery));
    }

    if let Some(exclusivity) = &params.exclusivity {
        let mut or_chains = Expr::value(false);

        if exclusivity.contains(&ItemExclusivity::Host) {
            or_chains = or_chains.or(ItemColumn::HostId
                .is_not_null()
                .and(ItemColumn::TemplateId.is_null()));
        }

        if exclusivity.contains(&ItemExclusivity::Template) {
            or_chains = or_chains.or(ItemColumn::TemplateId
                .is_not_null()
                .and(ItemColumn::HostId.is_null()));
        }

        if exclusivity.contains(&ItemExclusivity::TemplateOnHost) {
            or_chains = or_chains.or(ItemColumn::HostId
                .is_not_null()
                .and(ItemColumn::TemplateId.is_not_null()));
        }

        filters.push(or_chains);
    }

    if !filters.is_empty() {
        query_params.with_additional_filters(filters);
    }

    if !filters_last.is_empty() {
        query_params.with_additional_filters_last(filters_last);
    }

    let count_params = query_params.clone();

    query_params
        .with_order_bys(vec![(params.order_by.into(), params.order_dir.into())])
        .with_limit(params.limit)
        .with_offset(params.offset);

    let item_query = items(query_params);
    let (sql, values) = item_query.build_postgres(PostgresQueryBuilder);

    let rows = client
        .query(sql.as_str(), &values.as_params())
        .await
        .ctx("execute item list query")?;

    let items_result: Result<Vec<ListItemResponseInner>, Box<dyn std::error::Error>> =
        rows.iter().map(ListItemResponseInner::try_from).collect();

    let mut item_rows: Vec<ListItemResponseInner> = items_result.ctx("parse item rows")?;

    // Get total count
    let count_query_result = items(count_params);
    let (count_sql, count_values) = &count_query_result.build_postgres(PostgresQueryBuilder);

    let total_sql = format!("SELECT COUNT(*) as total FROM ({})", count_sql);
    let total_row = client
        .query_one(total_sql.as_str(), &count_values.as_params())
        .await
        .ctx("execute count query")?;

    let total: u32 = total_row
        .try_get::<_, i64>("total")
        .ctx("parse total count")?
        .try_into()
        .ctx("total count overflow")?;

    // Collect item stats from peers
    if params.collect_item_stats.unwrap_or(false) && !item_rows.is_empty() {
        let mut collected_item_stats =
            collect_item_stats(state, item_rows.iter().map(|r| r.item_id).collect()).await;

        for row in item_rows.iter_mut() {
            row.stats = collected_item_stats.remove(&row.item_id);
        }
    }

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(ListItemResponse {
            items: item_rows,
            total,
        }),
    )
}
