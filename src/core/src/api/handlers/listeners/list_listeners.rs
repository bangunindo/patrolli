// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::OrderByDirection;
use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::config::Batch;
use crate::config::listener::{Config, ListenerKind};
use crate::config::sensitive::Masked;
use crate::db::models::serialize_utc_as_local;
use crate::db::query::{ListenerColumn, ListenerParams, generate_count_query, listeners};
use crate::db::schema::{AdmStatus, Listeners};
use axum::extract::State;
use axum::http::StatusCode;
use axum_extra::extract::Query as ApiQuery;
use chrono::{DateTime, Utc};
use sea_query::extension::postgres::PgExpr;
use sea_query::{ColumnRef, Expr, ExprTrait, IntoColumnRef, PostgresQueryBuilder};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListListenerRequest {
    pub listener_ids: Option<Vec<Uuid>>,
    pub name: Option<String>,
    #[serde(default = "default_list_listener_limit")]
    pub limit: u64,
    #[serde(default = "default_list_listener_offset")]
    pub offset: u64,
    #[serde(default)]
    pub order_by: ListenerOrderBy,
    #[serde(default)]
    pub order_dir: OrderByDirection,
    pub types: Option<Vec<ListenerKind>>,
}

fn default_list_listener_limit() -> u64 {
    20
}

fn default_list_listener_offset() -> u64 {
    0
}

#[derive(Default, Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ListenerOrderBy {
    #[default]
    Name,
    LastUpdated,
}

impl From<ListenerOrderBy> for ColumnRef {
    fn from(val: ListenerOrderBy) -> Self {
        match val {
            ListenerOrderBy::Name => Listeners::Name,
            ListenerOrderBy::LastUpdated => Listeners::LastUpdated,
        }
        .into_column_ref()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListListenerResponseInner {
    pub listener_id: Uuid,
    pub name: String,
    #[schema(value_type = i16)]
    pub adm_status: AdmStatus,
    pub labels: serde_json::Value,
    pub worker_count: u32,
    pub batch: Batch,
    #[schema(value_type = Config)]
    pub config: Masked<Config>,
    pub disable_cluster_forwarding: bool,
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListListenerResponse {
    pub listeners: Vec<ListListenerResponseInner>,
    pub total: u32,
}

/// List all listeners
#[utoipa::path(
    get,
    path = "/listeners",
    params(ListListenerRequest),
    responses(
        (status = 200, description = "List of listeners", body = ResponseData<ListListenerResponse>),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "listeners",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn list_listeners(
    State(state): State<Arc<AppState>>,
    ApiQuery(params): ApiQuery<ListListenerRequest>,
) -> Result<ResponseData<ListListenerResponse>, ResponseError> {
    let client = state.db_pool.get().await.ctx("get database connection")?;

    let mut query_params = ListenerParams::new();
    query_params.with_columns(vec![
        ListenerColumn::ListenerId,
        ListenerColumn::Name,
        ListenerColumn::AdmStatus,
        ListenerColumn::Labels,
        ListenerColumn::WorkerCount,
        ListenerColumn::BatchSize,
        ListenerColumn::BatchTimeoutMs,
        ListenerColumn::ConfigEmbed,
        ListenerColumn::DisableClusterForwarding,
        ListenerColumn::LastUpdated,
    ]);

    let mut query = listeners(query_params);

    if let Some(name) = &params.name {
        query.and_where(Expr::from(ListenerColumn::Name).ilike(format!("%{}%", name)));
    }

    if let Some(listener_ids) = &params.listener_ids {
        query.and_where(Expr::from(ListenerColumn::ListenerId).is_in(listener_ids.clone()));
    }

    if let Some(types) = &params.types {
        let type_strs: Vec<String> = types.iter().map(|t| t.to_string()).collect();
        query.and_where(Expr::from(ListenerColumn::Type).is_in(type_strs));
    }

    let total_query = generate_count_query(&query);

    query.order_by(params.order_by, params.order_dir.into());
    query.limit(params.limit).offset(params.offset);

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let rows = client
        .query(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    let listeners_result: Result<Vec<ListListenerResponseInner>, Box<dyn Error>> = rows
        .iter()
        .map(|row| {
            let config: serde_json::Value = row.try_get(ListenerColumn::ConfigEmbed.as_str())?;
            let config: Config = serde_json::from_value(config)?;

            let worker_count: i32 = row.try_get(ListenerColumn::WorkerCount.as_str())?;
            let batch = Batch {
                batch_size: row.try_get::<_, i32>(ListenerColumn::BatchSize.as_str())? as usize,
                batch_timeout: Duration::from_millis(
                    row.try_get::<_, i32>(ListenerColumn::BatchTimeoutMs.as_str())? as u64,
                ),
            };

            Ok(ListListenerResponseInner {
                listener_id: row.try_get(ListenerColumn::ListenerId.as_str())?,
                name: row.try_get(ListenerColumn::Name.as_str())?,
                adm_status: row
                    .try_get::<_, i16>(ListenerColumn::AdmStatus.as_str())?
                    .try_into()?,
                labels: row.try_get(ListenerColumn::Labels.as_str())?,
                worker_count: worker_count as u32,
                batch,
                config: Masked(config),
                disable_cluster_forwarding: row
                    .try_get(ListenerColumn::DisableClusterForwarding.as_str())?,
                last_updated: row.try_get(ListenerColumn::LastUpdated.as_str())?,
            })
        })
        .collect();

    let listeners = listeners_result.ctx("parse listeners")?;

    let (sql, values) = total_query.build_postgres(PostgresQueryBuilder);
    let total_row: i64 = client
        .query_one_scalar(sql.as_str(), &values.as_params())
        .await
        .ctx("execute total count query")?;

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(ListListenerResponse {
            listeners,
            total: total_row as u32,
        }),
    )
}
