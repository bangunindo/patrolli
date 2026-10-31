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
use axum::{
    Json,
    extract::{Path, State},
};
use chrono::{DateTime, Utc};
use sea_query::{Expr, ExprTrait, OnConflict, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateProcessorRequest {
    pub name: String,
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    #[serde(default)]
    pub input_processors: Vec<Uuid>,
    #[serde(default)]
    pub input_items: Vec<Uuid>,
    pub config: Config,
}

impl UpdateProcessorRequest {
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
pub struct UpdateProcessorResponse {
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

/// Update a processor
#[utoipa::path(
    put,
    path = "/processors/{processor_id}",
    params(
        ("processor_id" = Uuid, Path, description = "Processor UUID")
    ),
    request_body = UpdateProcessorRequest,
    responses(
        (status = 200, description = "Processor updated", body = ResponseData<UpdateProcessorResponse>),
        (status = 404, description = "Processor not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "processors",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn update_processor(
    State(state): State<Arc<AppState>>,
    Path(processor_id): Path<Uuid>,
    Json(payload): Json<UpdateProcessorRequest>,
) -> Result<ResponseData<UpdateProcessorResponse>, ResponseError> {
    payload.validate().bad_request("Validation error")?;

    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    let mut query = DbQuery::select();
    query
        .column((Processors::Table, Processors::ProcessorId))
        .from(Processors::Table)
        .and_where(Expr::col((Processors::Table, Processors::ProcessorId)).eq(processor_id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let row_opt = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("query processor")?;

    if row_opt.is_none() {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Processor not found")
            .with_detail(format!("Processor with id {} not found", processor_id)));
    }

    let now = Utc::now();
    let (sql, values) = DbQuery::update()
        .table(Processors::Table)
        .and_where(Expr::col((Processors::Table, Processors::ProcessorId)).eq(processor_id))
        .value(Processors::Name, payload.name.clone())
        .value(
            Processors::Type,
            ProcessorKind::from(&payload.config).to_string(),
        )
        .value(Processors::AdmStatus, payload.adm_status as i16)
        .value(
            Processors::Config,
            payload.config.to_json_value().ctx("serialize config")?,
        )
        .value(Processors::LastUpdated, now)
        .build_postgres(PostgresQueryBuilder);

    let affected = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("execute processor update query")?;

    if affected == 0 {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Processor not found")
            .with_detail(format!(
                "No processor found for processor_id: {}",
                processor_id
            )));
    }

    let (sql, values) = DbQuery::delete()
        .from_table(ProcessorInputs::Table)
        .and_where(
            Expr::col((ProcessorInputs::Table, ProcessorInputs::ProcessorId)).eq(processor_id),
        )
        .build_postgres(PostgresQueryBuilder);

    _ = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete processor inputs")?;

    if !payload.input_processors.is_empty() {
        // Check cycle
        // Start from current processor_id, get all downstream processor ids
        // Then compare with payload.input_processors, if any of them is in the result set, then it is a cycle.
        let cycle_statement = "
            WITH RECURSIVE processor_graph AS (
                SELECT processor_id
                FROM processor_inputs
                WHERE input_id = $1
                UNION ALL
                SELECT pi.processor_id
                FROM processor_inputs pi
                JOIN processor_graph pg
                ON pi.input_id = pg.processor_id
            ), distinct_id AS (
                SELECT DISTINCT processor_id
                FROM processor_graph
            ) SELECT d.processor_id, p.name FROM distinct_id d
             JOIN processors p ON d.processor_id = p.processor_id
        ";

        let rows = tx
            .query(cycle_statement, &[&processor_id])
            .await
            .ctx("execute cycle check query")?;

        let downstream_processor_ids: HashMap<Uuid, String> = rows
            .iter()
            .map(|row| {
                let id: Uuid = row.get(0);
                let name: String = row.get(1);
                (id, name)
            })
            .collect();

        let cycle_input_names: Vec<String> = payload
            .input_processors
            .iter()
            .filter_map(|id| downstream_processor_ids.get(id).cloned())
            .collect();

        if !cycle_input_names.is_empty() {
            return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
                .with_title("Cannot form a cycle within processor sequence")
                .with_detail(format!(
                    "Cycle detected in input processors: {}",
                    cycle_input_names.join(", ")
                )));
        }

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

    let (sql, values) = DbQuery::delete()
        .from_table(ItemProcessors::Table)
        .and_where(Expr::col((ItemProcessors::Table, ItemProcessors::ProcessorId)).eq(processor_id))
        .build_postgres(PostgresQueryBuilder);

    _ = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete item processors")?;

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
        ResponseData::from_status(StatusCode::OK).with_data(UpdateProcessorResponse {
            processor_id,
            name: payload.name,
            adm_status: payload.adm_status,
            input_processors: payload.input_processors,
            input_items: payload.input_items,
            config: payload.config,
            last_updated: now,
        }),
    )
}
