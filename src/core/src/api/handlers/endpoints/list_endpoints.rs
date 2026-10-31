// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::OrderByDirection;
use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::config::endpoint::{Config, EndpointKind};
use crate::config::sensitive::Masked;
use crate::db::models::serialize_utc_as_local;
use crate::db::query::generate_count_query;
use crate::db::schema::{AdmStatus, HostEndpoints, Hosts};
use axum::extract::State;
use axum::http::StatusCode;
use axum_extra::extract::Query as ApiQuery;
use chrono::{DateTime, Utc};
use sea_query::extension::postgres::PgExpr;
use sea_query::{
    ColumnRef, Expr, ExprTrait, IntoColumnRef, PgFunc, PostgresQueryBuilder, Query as DbQuery,
};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct ListEndpointRequest {
    pub host_ids: Option<Vec<Uuid>>,
    pub endpoint_ids: Option<Vec<Uuid>>,
    pub name: Option<String>,
    #[serde(default = "default_list_endpoint_limit")]
    pub limit: u64,
    #[serde(default = "default_list_endpoint_offset")]
    pub offset: u64,
    #[serde(default)]
    pub order_by: EndpointOrderBy,
    #[serde(default)]
    pub order_dir: OrderByDirection,
    pub types: Option<Vec<EndpointKind>>,
}

fn default_list_endpoint_limit() -> u64 {
    20
}

fn default_list_endpoint_offset() -> u64 {
    0
}

#[derive(Default, Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EndpointOrderBy {
    #[default]
    Name,
    LastUpdated,
}

impl From<EndpointOrderBy> for ColumnRef {
    fn from(val: EndpointOrderBy) -> Self {
        match val {
            EndpointOrderBy::Name => HostEndpoints::Name,
            EndpointOrderBy::LastUpdated => HostEndpoints::LastUpdated,
        }
        .into_column_ref()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListEndpointResponseInner {
    pub endpoint_id: Uuid,
    pub host_id: Uuid,
    pub host_name: String,
    pub name: String,
    #[schema(value_type = i16)]
    pub adm_status: AdmStatus,
    #[schema(value_type = Config)]
    pub config: Masked<Config>,
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListEndpointResponse {
    pub endpoints: Vec<ListEndpointResponseInner>,
    pub total: u32,
}

/// List all endpoints
#[utoipa::path(
    get,
    path = "/endpoints",
    params(ListEndpointRequest),
    responses(
        (status = 200, description = "List of endpoints", body = ResponseData<ListEndpointResponse>),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "endpoints",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn list_endpoints(
    State(state): State<Arc<AppState>>,
    ApiQuery(params): ApiQuery<ListEndpointRequest>,
) -> Result<ResponseData<ListEndpointResponse>, ResponseError> {
    let client = state.db_pool.get().await.ctx("get database connection")?;

    let mut query = DbQuery::select();
    query
        .column((HostEndpoints::Table, HostEndpoints::EndpointId))
        .column((HostEndpoints::Table, HostEndpoints::HostId))
        .column((HostEndpoints::Table, HostEndpoints::Name))
        .column((HostEndpoints::Table, HostEndpoints::AdmStatus))
        .column((HostEndpoints::Table, HostEndpoints::LastUpdated))
        .expr(PgFunc::json_build_object(vec![
            (
                Expr::val("type").cast_as("text"),
                Expr::col((HostEndpoints::Table, HostEndpoints::Type)),
            ),
            (
                Expr::val("config").cast_as("text"),
                Expr::col((HostEndpoints::Table, HostEndpoints::Config)),
            ),
        ]))
        .expr_as(Expr::col((Hosts::Table, Hosts::Name)), "host_name")
        .from(HostEndpoints::Table)
        .inner_join(
            Hosts::Table,
            Expr::col((HostEndpoints::Table, HostEndpoints::HostId))
                .equals((Hosts::Table, Hosts::HostId)),
        );

    if let Some(name) = &params.name {
        query.and_where(
            Expr::col((HostEndpoints::Table, HostEndpoints::Name)).ilike(format!("%{}%", name)),
        );
    }

    if let Some(endpoint_ids) = &params.endpoint_ids {
        query.and_where(
            Expr::col((HostEndpoints::Table, HostEndpoints::EndpointId))
                .is_in(endpoint_ids.clone()),
        );
    }

    if let Some(host_ids) = &params.host_ids {
        query.and_where(
            Expr::col((HostEndpoints::Table, HostEndpoints::HostId)).is_in(host_ids.clone()),
        );
    }

    if let Some(types) = &params.types {
        let type_strs: Vec<String> = types.iter().map(|t| t.to_string()).collect();
        query.and_where(Expr::col((HostEndpoints::Table, HostEndpoints::Type)).is_in(type_strs));
    }

    // generate total count query
    let total_query = generate_count_query(&query);

    query.order_by(params.order_by, params.order_dir.into());
    query.limit(params.limit).offset(params.offset);

    // get endpoints
    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let rows = client
        .query(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    let endpoints_result: Result<Vec<ListEndpointResponseInner>, Box<dyn Error>> = rows
        .iter()
        .map(|row| {
            let config: serde_json::Value = row.try_get(5)?;
            let config: Config = serde_json::from_value(config)?;

            Ok(ListEndpointResponseInner {
                endpoint_id: row.try_get(0)?,
                host_id: row.try_get(1)?,
                name: row.try_get(2)?,
                adm_status: row.try_get::<_, i16>(3)?.try_into()?,
                last_updated: row.try_get(4)?,
                host_name: row.try_get(6)?,
                config: Masked(config),
            })
        })
        .collect();

    let endpoints = endpoints_result.ctx("parse endpoints")?;

    // get total count
    let (sql, values) = total_query.build_postgres(PostgresQueryBuilder);
    let total_row: i64 = client
        .query_one_scalar(sql.as_str(), &values.as_params())
        .await
        .ctx("execute total count query")?;

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(ListEndpointResponse {
            endpoints,
            total: total_row as u32,
        }),
    )
}
