// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::config::processor::{Config, Processor, ProcessorKind};
use crate::db::models::serialize_utc_as_local;
use crate::db::schema::{
    AdmStatus, ItemProcessors, Items, ProcessorInputs, Processors, default_adm_status,
};
use axum::http::StatusCode;
use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use sea_query::{Expr, ExprTrait, OnConflict, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateProcessorRequest {
    pub name: String,
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    #[serde(default)]
    pub input_processors: Vec<Uuid>,
    #[serde(default)]
    pub input_items: Vec<Uuid>,
    pub config: Config,
}

impl CreateProcessorRequest {
    pub fn validate(&self) -> anyhow::Result<()> {
        let mut processor = Processor {
            id: Uuid::now_v7(),
            name: self.name.clone(),
            config: self.config.clone(),
            inputs: self.input_processors.clone(),
        };
        processor.validate()?;

        if self.name.trim().is_empty() {
            anyhow::bail!("Processor name cannot be empty");
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateProcessorResponse {
    pub processor_id: Uuid,
    pub name: String,
    #[schema(value_type = i16)]
    pub adm_status: AdmStatus,
    pub input_processors: Vec<Uuid>,
    pub input_items: Vec<Uuid>,
    pub config: Config,
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub last_updated: DateTime<Utc>,
}

/// Create a processor
#[utoipa::path(
    post,
    path = "/processors",
    request_body(
        content = CreateProcessorRequest,
        description = "Create processor request body",
        content_type = "application/json",
    ),
    responses(
        (status = 201, description = "Processor created", body = ResponseData<CreateProcessorResponse>),
        (status = 400, description = "Validation error", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "processors",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn create_processor(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateProcessorRequest>,
) -> Result<ResponseData<CreateProcessorResponse>, ResponseError> {
    payload.validate().bad_request("Validation error")?;

    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    let processor_id = Uuid::now_v7();
    let now = Utc::now();
    let (sql, values) = DbQuery::insert()
        .into_table(Processors::Table)
        .columns([
            Processors::ProcessorId,
            Processors::Name,
            Processors::Type,
            Processors::AdmStatus,
            Processors::LastUpdated,
            Processors::Config,
        ])
        .values([
            processor_id.into(),
            payload.name.clone().into(),
            ProcessorKind::from(&payload.config).to_string().into(),
            (payload.adm_status as i16).into(),
            now.into(),
            payload
                .config
                .to_json_value()
                .ctx("serialize config")?
                .into(),
        ])
        .ctx("build insert query")?
        .build_postgres(PostgresQueryBuilder);

    _ = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("execute insert query")?;

    if !payload.input_processors.is_empty() {
        // check missing input ids
        let mut query = DbQuery::select();
        query
            .column((Processors::Table, Processors::ProcessorId))
            .from(Processors::Table)
            .and_where(Expr::col(Processors::ProcessorId).is_in(payload.input_processors.clone()));

        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        let rows = tx
            .query(sql.as_str(), &values.as_params())
            .await
            .ctx("execute inputs validation check query")?;

        let existing_input_ids: HashSet<Uuid> =
            rows.iter().map(|row| row.get::<_, Uuid>(0)).collect();
        let missing_input_ids: Vec<Uuid> = payload
            .input_processors
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
                .with_detail(format!("Processor IDs not found: {}", missing_ids_display)));
        }

        // insert processor inputs
        let mut insert_inputs = DbQuery::insert();
        insert_inputs
            .into_table(ProcessorInputs::Table)
            .columns([ProcessorInputs::ProcessorId, ProcessorInputs::InputId])
            .on_conflict(
                OnConflict::columns([ProcessorInputs::ProcessorId, ProcessorInputs::InputId])
                    .do_nothing()
                    .to_owned(),
            );

        for input_id in payload.input_processors.iter() {
            insert_inputs
                .values([processor_id.into(), (*input_id).into()])
                .ctx("build insert inputs query")?;
        }

        let (sql, values) = insert_inputs.build_postgres(PostgresQueryBuilder);
        _ = tx
            .execute(sql.as_str(), &values.as_params())
            .await
            .ctx("execute processor_inputs insert query")?;
    }

    if !payload.input_items.is_empty() {
        let mut query = DbQuery::select();
        query
            .column((Items::Table, Items::ItemId))
            .from(Items::Table)
            .and_where(Expr::col(Items::ItemId).is_in(payload.input_items.clone()));

        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        let rows = tx
            .query(sql.as_str(), &values.as_params())
            .await
            .ctx("execute items validation check query")?;

        let existing_item_ids: HashSet<Uuid> =
            rows.iter().map(|row| row.get::<_, Uuid>(0)).collect();
        let missing_item_ids: Vec<Uuid> = payload
            .input_items
            .iter()
            .filter(|id| !existing_item_ids.contains(id))
            .cloned()
            .collect();

        if !missing_item_ids.is_empty() {
            let n_ids = 3;
            let missing_ids_display = if missing_item_ids.len() > n_ids {
                let displayed_ids: Vec<String> = missing_item_ids
                    .iter()
                    .take(n_ids)
                    .map(|id| id.to_string())
                    .collect();
                format!(
                    "{} and {} more",
                    displayed_ids.join(", "),
                    missing_item_ids.len() - n_ids
                )
            } else {
                missing_item_ids
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<String>>()
                    .join(", ")
            };

            return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
                .with_title("Validation error")
                .with_detail(format!("Item IDs not found: {}", missing_ids_display)));
        }

        let mut insert_items = DbQuery::insert();
        insert_items
            .into_table(ItemProcessors::Table)
            .columns([ItemProcessors::ItemId, ItemProcessors::ProcessorId])
            .on_conflict(
                OnConflict::columns([ItemProcessors::ItemId, ItemProcessors::ProcessorId])
                    .do_nothing()
                    .to_owned(),
            );

        for item_id in payload.input_items.iter() {
            insert_items
                .values([(*item_id).into(), processor_id.into()])
                .ctx("build insert items query")?;
        }

        let (sql, values) = insert_items.build_postgres(PostgresQueryBuilder);
        _ = tx
            .execute(sql.as_str(), &values.as_params())
            .await
            .ctx("execute item_processors insert query")?;
    }

    tx.commit().await.ctx("commit transaction")?;

    Ok(
        ResponseData::from_status(StatusCode::CREATED).with_data(CreateProcessorResponse {
            processor_id,
            name: payload.name,
            adm_status: payload.adm_status,
            input_processors: payload.input_processors,
            config: payload.config,
            last_updated: now,
            input_items: payload.input_items,
        }),
    )
}
