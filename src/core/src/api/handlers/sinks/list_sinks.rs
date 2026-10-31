// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::OrderByDirection;
use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::config::sensitive::Masked;
use crate::config::sink::{Config, SinkKind};
use crate::config::{Batch, Channel, Retry};
use crate::db::models::serialize_utc_as_local;
use crate::db::query::{SinkColumn, SinkParams, generate_count_query, sinks};
use crate::db::schema::{AdmStatus, Sinks};
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
pub struct ListSinkRequest {
    pub sink_ids: Option<Vec<Uuid>>,
    pub name: Option<String>,
    #[serde(default = "default_list_sink_limit")]
    pub limit: u64,
    #[serde(default = "default_list_sink_offset")]
    pub offset: u64,
    #[serde(default)]
    pub order_by: SinkOrderBy,
    #[serde(default)]
    pub order_dir: OrderByDirection,
    pub types: Option<Vec<SinkKind>>,
}

fn default_list_sink_limit() -> u64 {
    20
}

fn default_list_sink_offset() -> u64 {
    0
}

#[derive(Default, Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SinkOrderBy {
    #[default]
    Name,
    LastUpdated,
}

impl From<SinkOrderBy> for ColumnRef {
    fn from(val: SinkOrderBy) -> Self {
        match val {
            SinkOrderBy::Name => Sinks::Name,
            SinkOrderBy::LastUpdated => Sinks::LastUpdated,
        }
        .into_column_ref()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListSinkResponseInner {
    pub sink_id: Uuid,
    pub name: String,
    #[schema(value_type = i16)]
    pub adm_status: AdmStatus,
    pub labels: serde_json::Value,
    pub worker_count: u32,
    pub input_count: u64,
    #[schema(value_type = Config)]
    pub config: Masked<Config>,
    pub channel: Channel,
    pub batch: Batch,
    pub retry: Retry,
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListSinkResponse {
    pub sinks: Vec<ListSinkResponseInner>,
    pub total: u32,
}

/// List all sinks
#[utoipa::path(
    get,
    path = "/sinks",
    params(ListSinkRequest),
    responses(
        (status = 200, description = "List of sinks", body = ResponseData<ListSinkResponse>),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "sinks",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn list_sinks(
    State(state): State<Arc<AppState>>,
    ApiQuery(params): ApiQuery<ListSinkRequest>,
) -> Result<ResponseData<ListSinkResponse>, ResponseError> {
    let client = state.db_pool.get().await.ctx("get database connection")?;

    let mut query_params = SinkParams::new();
    query_params.with_columns(vec![
        SinkColumn::SinkId,
        SinkColumn::Name,
        SinkColumn::AdmStatus,
        SinkColumn::Labels,
        SinkColumn::WorkerCount,
        SinkColumn::BatchSize,
        SinkColumn::BatchTimeoutMs,
        SinkColumn::Channel,
        SinkColumn::Retry,
        SinkColumn::InputCount,
        SinkColumn::Type,
        SinkColumn::ConfigEmbed,
        SinkColumn::LastUpdated,
    ]);

    let mut query = sinks(query_params);

    if let Some(name) = &params.name {
        query.and_where(Expr::from(SinkColumn::Name).ilike(format!("%{}%", name)));
    }

    if let Some(sink_ids) = &params.sink_ids {
        query.and_where(Expr::from(SinkColumn::SinkId).is_in(sink_ids.clone()));
    }

    if let Some(types) = &params.types {
        let type_strs = types.iter().map(|t| t.to_string()).collect::<Vec<_>>();
        query.and_where(Expr::from(SinkColumn::Type).is_in(type_strs));
    }

    let total_query = generate_count_query(&query);

    query.order_by(params.order_by, params.order_dir.into());
    query.limit(params.limit).offset(params.offset);

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let rows = client
        .query(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    let sinks_result: Result<Vec<ListSinkResponseInner>, Box<dyn Error>> = rows
        .iter()
        .map(|row| {
            let worker_count: i32 = row.try_get(SinkColumn::WorkerCount.as_str())?;
            let input_count: i32 = row.try_get(SinkColumn::InputCount.as_str())?;

            let config: serde_json::Value = row.try_get(SinkColumn::ConfigEmbed.as_str())?;
            let config: Config = serde_json::from_value(config)?;

            let batch = Batch {
                batch_size: row.try_get::<_, i32>(SinkColumn::BatchSize.as_str())? as usize,
                batch_timeout: Duration::from_millis(
                    row.try_get::<_, i32>(SinkColumn::BatchTimeoutMs.as_str())? as u64,
                ),
            };
            let channel: serde_json::Value = row.try_get(SinkColumn::Channel.as_str())?;
            let channel: Channel = serde_json::from_value(channel)?;
            let retry: serde_json::Value = row.try_get(SinkColumn::Retry.as_str())?;
            let retry: Retry = serde_json::from_value(retry)?;

            Ok(ListSinkResponseInner {
                sink_id: row.try_get(SinkColumn::SinkId.as_str())?,
                name: row.try_get(SinkColumn::Name.as_str())?,
                adm_status: row
                    .try_get::<_, i16>(SinkColumn::AdmStatus.as_str())?
                    .try_into()?,
                labels: row.try_get(SinkColumn::Labels.as_str())?,
                worker_count: worker_count as u32,
                input_count: input_count as u64,
                config: Masked(config),
                channel,
                batch,
                retry,
                last_updated: row.try_get(SinkColumn::LastUpdated.as_str())?,
            })
        })
        .collect();

    let sinks = sinks_result.ctx("parse sinks")?;

    let (sql, values) = total_query.build_postgres(PostgresQueryBuilder);
    let total_row: i64 = client
        .query_one_scalar(sql.as_str(), &values.as_params())
        .await
        .ctx("execute total count query")?;

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(ListSinkResponse {
            sinks,
            total: total_row as u32,
        }),
    )
}
