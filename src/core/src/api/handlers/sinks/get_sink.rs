// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::config::sensitive::Masked;
use crate::config::sink::Config;
use crate::config::{Batch, Channel, Retry};
use crate::db::models::serialize_utc_as_local;
use crate::db::query::{SinkColumn, SinkParams, sinks};
use crate::db::schema::{AdmStatus, Sinks, default_adm_status};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GetSinkResponse {
    pub sink_id: Uuid,
    pub name: String,
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    pub labels: serde_json::Value,
    pub worker_count: u32,
    pub inputs: Vec<Uuid>,
    #[schema(value_type = Config)]
    pub config: Masked<Config>,
    pub channel: Channel,
    pub batch: Batch,
    pub retry: Retry,
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub last_updated: DateTime<Utc>,
}

/// Get a sink by ID
#[utoipa::path(
    get,
    path = "/sinks/{sink_id}",
    params(
        ("sink_id" = Uuid, Path, description = "Sink UUID")
    ),
    responses(
        (status = 200, description = "Sink found", body = ResponseData<GetSinkResponse>),
        (status = 404, description = "Sink not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "sinks",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn get_sink(
    State(state): State<Arc<AppState>>,
    Path(sink_id): Path<Uuid>,
) -> Result<ResponseData<GetSinkResponse>, ResponseError> {
    let client = state.db_pool.get().await.ctx("get database connection")?;

    let mut params = SinkParams::new();
    params.with_columns(vec![
        SinkColumn::SinkId,
        SinkColumn::Name,
        SinkColumn::AdmStatus,
        SinkColumn::Labels,
        SinkColumn::WorkerCount,
        SinkColumn::BatchSize,
        SinkColumn::BatchTimeoutMs,
        SinkColumn::Channel,
        SinkColumn::Retry,
        SinkColumn::Inputs,
        SinkColumn::Type,
        SinkColumn::ConfigEmbed,
        SinkColumn::LastUpdated,
    ]);

    let subquery: Expr = DbQuery::select()
        .column(Sinks::SinkId)
        .from(Sinks::Table)
        .and_where(Expr::col(Sinks::SinkId).eq(sink_id))
        .to_owned()
        .into();

    params.with_id_subquery_filter(Some(subquery));

    let sink_query = sinks(params);
    let (sql, values) = sink_query.build_postgres(PostgresQueryBuilder);

    let row_opt = client
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("execute sink query")?;

    let Some(row) = row_opt else {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Sink not found")
            .with_detail(format!("No sink found for sink_id: {}", sink_id)));
    };

    let response = row_to_sink_response(&row).ctx("parse sink row")?;

    Ok(ResponseData::from_status(StatusCode::OK).with_data(response))
}

fn row_to_sink_response(
    row: &tokio_postgres::Row,
) -> Result<GetSinkResponse, Box<dyn std::error::Error>> {
    let worker_count: i32 = row.try_get(SinkColumn::WorkerCount.as_str())?;

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

    Ok(GetSinkResponse {
        sink_id: row.try_get(SinkColumn::SinkId.as_str())?,
        name: row.try_get(SinkColumn::Name.as_str())?,
        adm_status: row
            .try_get::<_, i16>(SinkColumn::AdmStatus.as_str())?
            .try_into()?,
        labels: row.try_get(SinkColumn::Labels.as_str())?,
        worker_count: worker_count as u32,
        inputs: row.try_get(SinkColumn::Inputs.as_str())?,
        config: Masked(config),
        channel,
        batch,
        retry,
        last_updated: row.try_get(SinkColumn::LastUpdated.as_str())?,
    })
}
