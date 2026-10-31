// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::OrderByDirection;
use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::config::processor::{Config, ProcessorKind};
use crate::db::models::serialize_utc_as_local;
use crate::db::query::{ProcessorColumn, ProcessorParams, generate_count_query, processors};
use crate::db::schema::{AdmStatus, Processors};
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
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListProcessorRequest {
    pub processor_ids: Option<Vec<Uuid>>,
    pub name: Option<String>,
    #[serde(default = "default_list_processor_limit")]
    pub limit: u64,
    #[serde(default = "default_list_processor_offset")]
    pub offset: u64,
    #[serde(default)]
    pub order_by: ProcessorOrderBy,
    #[serde(default)]
    pub order_dir: OrderByDirection,
    pub types: Option<Vec<ProcessorKind>>,
}

fn default_list_processor_limit() -> u64 {
    20
}

fn default_list_processor_offset() -> u64 {
    0
}

#[derive(Default, Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProcessorOrderBy {
    #[default]
    Name,
    LastUpdated,
}

impl From<ProcessorOrderBy> for ColumnRef {
    fn from(val: ProcessorOrderBy) -> Self {
        match val {
            ProcessorOrderBy::Name => Processors::Name,
            ProcessorOrderBy::LastUpdated => Processors::LastUpdated,
        }
        .into_column_ref()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListProcessorResponseInner {
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListProcessorResponse {
    pub processors: Vec<ListProcessorResponseInner>,
    pub total: u32,
}

/// List all processors
#[utoipa::path(
    get,
    path = "/processors",
    params(ListProcessorRequest),
    responses(
        (status = 200, description = "List of processors", body = ResponseData<ListProcessorResponse>),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "processors",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn list_processors(
    State(state): State<Arc<AppState>>,
    ApiQuery(params): ApiQuery<ListProcessorRequest>,
) -> Result<ResponseData<ListProcessorResponse>, ResponseError> {
    let client = state.db_pool.get().await.ctx("get database connection")?;

    let mut query_params = ProcessorParams::new();
    query_params.with_columns(vec![
        ProcessorColumn::ProcessorId,
        ProcessorColumn::Name,
        ProcessorColumn::AdmStatus,
        ProcessorColumn::Inputs,
        ProcessorColumn::InputItems,
        ProcessorColumn::ConfigEmbed,
        ProcessorColumn::LastUpdated,
    ]);

    let mut query = processors(query_params);

    if let Some(name) = &params.name {
        query.and_where(Expr::from(ProcessorColumn::Name).ilike(format!("%{}%", name)));
    }

    if let Some(processor_ids) = &params.processor_ids {
        query.and_where(Expr::from(ProcessorColumn::ProcessorId).is_in(processor_ids.clone()));
    }

    if let Some(types) = &params.types {
        let type_strs = types.iter().map(|t| t.to_string()).collect::<Vec<_>>();
        query.and_where(Expr::from(ProcessorColumn::Type).is_in(type_strs));
    }

    let total_query = generate_count_query(&query);

    query.order_by(params.order_by, params.order_dir.into());
    query.limit(params.limit).offset(params.offset);

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let rows = client
        .query(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    let processors_result: Result<Vec<ListProcessorResponseInner>, Box<dyn Error>> = rows
        .iter()
        .map(|row| {
            let config: serde_json::Value = row.try_get(ProcessorColumn::ConfigEmbed.as_str())?;
            let config: Config = serde_json::from_value(config)?;

            Ok(ListProcessorResponseInner {
                processor_id: row.try_get(ProcessorColumn::ProcessorId.as_str())?,
                name: row.try_get(ProcessorColumn::Name.as_str())?,
                adm_status: row
                    .try_get::<_, i16>(ProcessorColumn::AdmStatus.as_str())?
                    .try_into()?,
                input_processors: row.try_get(ProcessorColumn::Inputs.as_str())?,
                input_items: row.try_get(ProcessorColumn::InputItems.as_str())?,
                config,
                last_updated: row.try_get(ProcessorColumn::LastUpdated.as_str())?,
            })
        })
        .collect();

    let processors = processors_result.ctx("parse processors")?;

    let (sql, values) = total_query.build_postgres(PostgresQueryBuilder);
    let total_row: i64 = client
        .query_one_scalar(sql.as_str(), &values.as_params())
        .await
        .ctx("execute total count query")?;

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(ListProcessorResponse {
            processors,
            total: total_row as u32,
        }),
    )
}
