// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::OrderByDirection;
use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::models::serialize_utc_as_local;
use crate::db::schema::{
    AdmStatus, ClusterNodes, HostTemplates, Hosts, Items, Templates, default_adm_status,
};
use axum::{extract::State, http::StatusCode};
use axum_extra::extract::Query as ApiQuery;
use chrono::{DateTime, Utc};
use sea_query::extension::postgres::PgExpr;
use sea_query::{
    Asterisk, ColumnRef, CommonTableExpression, Expr, ExprTrait, Func, IntoColumnRef, JoinType,
    PgFunc, PostgresQueryBuilder, Query as DbQuery, WithClause,
};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio_postgres::types::Json;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ResponseHostTemplate {
    pub host_template_id: Uuid,
    pub template_id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ResponseHost {
    pub host_template_id: Uuid,
    pub host_id: Uuid,
    pub name: String,
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    pub labels: serde_json::Value,
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub last_updated: DateTime<Utc>,
    pub monitored_by: Option<Uuid>,
    pub monitored_by_node_name: Option<String>,
    #[schema(value_type = Vec<ResponseHostTemplate>)]
    pub templates: Json<Vec<ResponseHostTemplate>>,
    #[serde(default)]
    pub item_count: i64,
}

#[derive(Default, Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HostOrderBy {
    #[default]
    HostId,
    Name,
}

impl From<HostOrderBy> for ColumnRef {
    fn from(val: HostOrderBy) -> Self {
        match val {
            HostOrderBy::HostId => Hosts::HostId,
            HostOrderBy::Name => Hosts::Name,
        }
        .into_column_ref()
    }
}

#[derive(Clone, Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListHostParams {
    pub host_ids: Option<Vec<Uuid>>,
    pub name: Option<String>,
    pub monitored_by: Option<Uuid>,
    #[serde(default = "default_list_host_limit")]
    pub limit: u64,
    #[serde(default = "default_list_host_offset")]
    pub offset: u64,
    #[serde(default)]
    pub order_by: HostOrderBy,
    #[serde(default)]
    pub order_dir: OrderByDirection,
}

fn default_list_host_limit() -> u64 {
    20
}
fn default_list_host_offset() -> u64 {
    0
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListHostResponse {
    pub hosts: Vec<ResponseHost>,
    pub total: u32,
}

/// List all hosts
///
/// Returns all hosts from the database
#[utoipa::path(
    get,
    path = "/hosts",
    params(ListHostParams),
    responses(
        (status = 200, description = "List of hosts", body = ListHostResponse),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "hosts",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn list_hosts(
    State(state): State<Arc<AppState>>,
    ApiQuery(params): ApiQuery<ListHostParams>,
) -> Result<ResponseData<ListHostResponse>, ResponseError> {
    let client = state.db_pool.get().await.ctx("get database connection")?;

    const ITEM_COUNT_CTE: &str = "item_count";
    const TEMPLATE_AGG_CTE: &str = "template_agg";

    // item_count cte
    let item_count_cte = {
        let mut q = DbQuery::select();
        q.from(Items::Table)
            .expr_as(
                Expr::col((HostTemplates::Table, HostTemplates::HostId)),
                "host_id",
            )
            .expr_as(Func::count(Expr::col(Asterisk)), "item_count")
            .join(
                JoinType::Join,
                HostTemplates::Table,
                Expr::col((HostTemplates::Table, HostTemplates::HostTemplateId))
                    .eq(Expr::col((Items::Table, Items::HostTemplateId))),
            )
            .add_group_by([Expr::col((HostTemplates::Table, HostTemplates::HostId))]);

        CommonTableExpression::new()
            .table_name(ITEM_COUNT_CTE)
            .query(q)
            .to_owned()
    };

    // template_agg cte
    let template_agg_cte: CommonTableExpression = {
        let json_obj: Expr = PgFunc::json_build_object(vec![
            (
                Expr::val("host_template_id").cast_as("text"),
                Expr::col((HostTemplates::Table, HostTemplates::HostTemplateId)),
            ),
            (
                Expr::val("template_id").cast_as("text"),
                Expr::col((Templates::Table, Templates::TemplateId)),
            ),
            (
                Expr::val("name").cast_as("text"),
                Expr::col((Templates::Table, Templates::Name)),
            ),
        ])
        .into();

        let mut q = DbQuery::select();
        q.from(HostTemplates::Table)
            .expr_as(
                Expr::col((HostTemplates::Table, HostTemplates::HostId)),
                "host_id",
            )
            .expr_as(
                Func::coalesce(vec![
                    PgFunc::json_agg(json_obj).into(),
                    Expr::cust("'{}'::json"),
                ]),
                "templates",
            )
            .join(
                JoinType::Join,
                Templates::Table,
                Expr::col((HostTemplates::Table, HostTemplates::TemplateId))
                    .eq(Expr::col((Templates::Table, Templates::TemplateId))),
            )
            .and_where(Expr::col((HostTemplates::Table, HostTemplates::TemplateId)).is_not_null())
            .add_group_by([Expr::col((HostTemplates::Table, HostTemplates::HostId))]);

        CommonTableExpression::new()
            .table_name(TEMPLATE_AGG_CTE)
            .query(q)
            .to_owned()
    };

    let mut with_clause = WithClause::new();
    with_clause.cte(item_count_cte).cte(template_agg_cte);

    let mut query = DbQuery::select();
    query
        .column((HostTemplates::Table, HostTemplates::HostTemplateId))
        .columns([
            (Hosts::Table, Hosts::HostId),
            (Hosts::Table, Hosts::Name),
            (Hosts::Table, Hosts::AdmStatus),
            (Hosts::Table, Hosts::Labels),
            (Hosts::Table, Hosts::LastUpdated),
            (Hosts::Table, Hosts::MonitoredBy),
        ])
        .expr_as(
            Func::coalesce(vec![
                Expr::col((ITEM_COUNT_CTE, "item_count")),
                Expr::cust("0"),
            ]),
            "item_count",
        )
        .expr_as(
            Func::coalesce(vec![
                Expr::col((TEMPLATE_AGG_CTE, "templates")),
                Expr::cust("'[]'::json"),
            ]),
            "templates",
        )
        .expr_as(
            Expr::col((ClusterNodes::Table, ClusterNodes::Name)),
            "monitored_by_node_name",
        )
        .from(Hosts::Table)
        .join(
            JoinType::LeftJoin,
            ClusterNodes::Table,
            Expr::col((ClusterNodes::Table, ClusterNodes::NodeId))
                .eq(Expr::col((Hosts::Table, Hosts::MonitoredBy))),
        )
        .join(
            JoinType::Join,
            HostTemplates::Table,
            Expr::col((HostTemplates::Table, HostTemplates::HostId))
                .eq(Expr::col((Hosts::Table, Hosts::HostId)))
                .and(Expr::col((HostTemplates::Table, HostTemplates::TemplateId)).is_null()),
        )
        .join(
            JoinType::LeftJoin,
            ITEM_COUNT_CTE,
            Expr::col((ITEM_COUNT_CTE, "host_id")).eq(Expr::col((Hosts::Table, Hosts::HostId))),
        )
        .join(
            JoinType::LeftJoin,
            TEMPLATE_AGG_CTE,
            Expr::col((TEMPLATE_AGG_CTE, "host_id")).eq(Expr::col((Hosts::Table, Hosts::HostId))),
        );

    // Apply name filter if provided
    if let Some(name) = &params.name {
        query.and_where(Expr::col((Hosts::Table, Hosts::Name)).ilike(format!("%{}%", name)));
    }

    // Apply host_ids filter if provided
    if let Some(host_ids) = &params.host_ids {
        query.and_where(Expr::col((Hosts::Table, Hosts::HostId)).is_in(host_ids.clone()));
    }

    // Apply monitored_by filter if provided
    if let Some(monitored_by) = &params.monitored_by {
        query.and_where(Expr::col((Hosts::Table, Hosts::MonitoredBy)).eq(*monitored_by));
    }

    const HOST_LIST_CTE: &str = "host_list";
    let host_list_cte = CommonTableExpression::new()
        .table_name(HOST_LIST_CTE)
        .query(query.clone())
        .to_owned();

    let mut total_query = DbQuery::select();
    total_query
        .expr(Func::count(Expr::col(Hosts::HostId)))
        .from(HOST_LIST_CTE);

    let total_with_clause = with_clause.clone().cte(host_list_cte).to_owned();

    let (sql, values) = total_query
        .with(total_with_clause)
        .build_postgres(PostgresQueryBuilder);

    let total_row: i64 = client
        .query_one_scalar(sql.as_str(), &values.as_params())
        .await
        .ctx("execute total count query")?;

    // Apply ordering
    query.order_by(params.order_by, params.order_dir.into());

    // Apply pagination
    query.limit(params.limit).offset(params.offset);

    let (sql, values) = query.with(with_clause).build_postgres(PostgresQueryBuilder);

    let rows = client
        .query(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    let hosts: Result<Vec<ResponseHost>, Box<dyn std::error::Error>> = rows
        .iter()
        .map(|row| {
            Ok(ResponseHost {
                host_template_id: row.try_get(0)?,
                host_id: row.try_get(1)?,
                name: row.try_get(2)?,
                adm_status: row.try_get::<_, i16>(3)?.try_into()?,
                labels: row.try_get(4)?,
                last_updated: row.try_get(5)?,
                monitored_by: row.try_get(6)?,
                item_count: row.try_get(7)?,
                templates: row.try_get(8)?,
                monitored_by_node_name: row.try_get(9)?,
            })
        })
        .collect();

    let hosts = hosts.ctx("parse host rows")?;
    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(ListHostResponse {
            hosts,
            total: total_row as u32,
        }),
    )
}
