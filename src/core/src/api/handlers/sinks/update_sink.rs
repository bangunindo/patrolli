// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::config::sensitive::{Masked, SensitiveFields};
use crate::config::sink::{Config, Sink, SinkKind};
use crate::config::{Batch, Channel, Retry};
use crate::db::models::serialize_utc_as_local;
use crate::db::schema::{AdmStatus, Processors, SinkInputs, Sinks, default_adm_status};
use axum::http::StatusCode;
use axum::{
    Json,
    extract::{Path, State},
};
use chrono::{DateTime, Utc};
use sea_query::{Expr, ExprTrait, LockType, OnConflict, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateSinkRequest {
    pub name: String,
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    pub labels: serde_json::Value,
    #[serde(default)]
    pub worker_count: u32,
    #[serde(default)]
    /// List of processor IDs that are inputs to this sink.
    /// If empty, the sink will have no inputs and will not receive any data.
    pub inputs: Vec<Uuid>,
    pub config: Config,
    pub batch: Batch,
    pub channel: Channel,
    /// Omitted takes every default.
    #[serde(default)]
    pub retry: Retry,
}

impl UpdateSinkRequest {
    /// Validates the request and normalizes its batch and retry to the values stored.
    pub fn validate(&mut self) -> anyhow::Result<()> {
        let mut sink = Sink {
            id: Uuid::now_v7(),
            name: self.name.clone(),
            worker_count: self.worker_count as usize,
            inputs: self.inputs.clone(),
            config: self.config.clone(),
            channel: self.channel,
            batch: self.batch,
            retry: self.retry,
        };
        sink.validate()?;
        self.batch = sink.batch;
        self.retry = sink.retry;

        if self.worker_count == 0 {
            anyhow::bail!("Worker count must be greater than 0");
        }

        if self.name.trim().is_empty() {
            anyhow::bail!("Sink name cannot be empty");
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateSinkResponse {
    pub sink_id: Uuid,
    pub name: String,
    #[schema(value_type = i16)]
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

/// Update a sink
#[utoipa::path(
    put,
    path = "/sinks/{sink_id}",
    params(
        ("sink_id" = Uuid, Path, description = "Sink UUID")
    ),
    request_body = UpdateSinkRequest,
    responses(
        (status = 200, description = "Sink updated", body = ResponseData<UpdateSinkResponse>),
        (status = 404, description = "Sink not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "sinks",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn update_sink(
    State(state): State<Arc<AppState>>,
    Path(sink_id): Path<Uuid>,
    Json(mut payload): Json<UpdateSinkRequest>,
) -> Result<ResponseData<UpdateSinkResponse>, ResponseError> {
    payload.validate().bad_request("Validation error")?;

    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    let mut query = DbQuery::select();
    query
        .columns([(Sinks::Table, Sinks::Type), (Sinks::Table, Sinks::Config)])
        .from(Sinks::Table)
        .and_where(Expr::col((Sinks::Table, Sinks::SinkId)).eq(sink_id))
        .lock(LockType::Update);

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let row_opt = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("query sink")?;

    let Some(row) = row_opt else {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Sink not found")
            .with_detail(format!("Sink with id {} not found", sink_id)));
    };

    // keep stored secrets
    let stored_type: String = row.try_get(0).ctx("get sink type")?;
    let stored_config: serde_json::Value = row.try_get(1).ctx("get sink config")?;
    payload
        .config
        .keep_stored(&stored_type, stored_config)
        .bad_request("Validation error")?;

    // encrypt secrets
    payload
        .config
        .encrypt_all(&state.secret_keys)
        .ctx("encrypt sink config")?;

    let now = Utc::now();
    let (sql, values) = DbQuery::update()
        .table(Sinks::Table)
        .and_where(Expr::col((Sinks::Table, Sinks::SinkId)).eq(sink_id))
        .value(Sinks::Name, payload.name.clone())
        .value(Sinks::Type, SinkKind::from(&payload.config).to_string())
        .value(Sinks::AdmStatus, payload.adm_status as i16)
        .value(Sinks::Labels, payload.labels.clone())
        .value(Sinks::WorkerCount, payload.worker_count)
        .value(
            Sinks::Config,
            payload
                .config
                .to_json_value()
                .ctx("serialize sink config")?,
        )
        .value(Sinks::BatchSize, payload.batch.batch_size as i32)
        .value(
            Sinks::BatchTimeoutMs,
            payload.batch.batch_timeout.as_millis() as i32,
        )
        .value(
            Sinks::Channel,
            serde_json::to_value(payload.channel).ctx("serialize channel config")?,
        )
        .value(
            Sinks::Retry,
            serde_json::to_value(payload.retry).ctx("serialize retry config")?,
        )
        .value(Sinks::LastUpdated, now)
        .build_postgres(PostgresQueryBuilder);

    let affected = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("execute sink update query")?;

    if affected == 0 {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Sink not found")
            .with_detail(format!("No sink found for sink_id: {}", sink_id)));
    }

    // delete existing sink inputs
    let (sql, values) = DbQuery::delete()
        .from_table(SinkInputs::Table)
        .and_where(Expr::col((SinkInputs::Table, SinkInputs::SinkId)).eq(sink_id))
        .build_postgres(PostgresQueryBuilder);

    _ = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete sink inputs")?;

    if !payload.inputs.is_empty() {
        // check missing input ids
        let mut query = DbQuery::select();
        query
            .column((Processors::Table, Processors::ProcessorId))
            .from(Processors::Table)
            .and_where(Expr::col(Processors::ProcessorId).is_in(payload.inputs.clone()));

        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        let rows = tx
            .query(sql.as_str(), &values.as_params())
            .await
            .ctx("execute inputs validation check query")?;

        let existing_input_ids: HashSet<Uuid> =
            rows.iter().map(|row| row.get::<_, Uuid>(0)).collect();
        let missing_input_ids: Vec<Uuid> = payload
            .inputs
            .iter()
            .filter(|id| !existing_input_ids.contains(id))
            .cloned()
            .collect();

        if !missing_input_ids.is_empty() {
            // truncate up to N ids, if more just say "and X more"
            let n_ids = 3;
            let missing_ids_display = if missing_input_ids.len() > n_ids {
                let displayed_ids: Vec<String> = missing_input_ids
                    .iter()
                    .take(n_ids)
                    .map(|id| id.to_string())
                    .collect();
                format!(
                    "{} and {} more",
                    displayed_ids.join(", "),
                    missing_input_ids.len() - n_ids
                )
            } else {
                missing_input_ids
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<String>>()
                    .join(", ")
            };

            return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
                .with_title("Validation error")
                .with_detail(format!(
                    "Processor IDs not found: {:?}",
                    missing_ids_display
                )));
        }

        let mut insert_inputs = DbQuery::insert();
        insert_inputs
            .into_table(SinkInputs::Table)
            .columns([SinkInputs::SinkId, SinkInputs::InputId])
            .on_conflict(
                OnConflict::columns([SinkInputs::SinkId, SinkInputs::InputId])
                    .do_nothing()
                    .to_owned(),
            );

        for input_id in &payload.inputs {
            insert_inputs
                .values([sink_id.into(), (*input_id).into()])
                .ctx("build insert inputs query")?;
        }

        let (sql, values) = insert_inputs.build_postgres(PostgresQueryBuilder);
        _ = tx
            .execute(sql.as_str(), &values.as_params())
            .await
            .ctx("execute sink_inputs insert query")?;
    }

    tx.commit().await.ctx("commit transaction")?;

    let unique_inputs: Vec<Uuid> = payload
        .inputs
        .into_iter()
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(UpdateSinkResponse {
            sink_id,
            name: payload.name,
            adm_status: payload.adm_status,
            labels: payload.labels,
            worker_count: payload.worker_count,
            inputs: unique_inputs,
            config: Masked(payload.config),
            channel: payload.channel,
            batch: payload.batch,
            retry: payload.retry,
            last_updated: now,
        }),
    )
}
