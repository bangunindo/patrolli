// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::types::ListTemplateParams;
use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::db::query::generate_count_query;
use crate::db::schema::Templates;
use crate::{
    api::state::AppState,
    db::schema::{HostTemplates, Items, TemplateLinks},
};
use axum::{
    extract::{Query as ApiQuery, State},
    http::StatusCode,
};
use sea_query::extension::postgres::PgExpr;
use sea_query::{Expr, ExprTrait, JoinType, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InnerTemplate {
    pub template_id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Template {
    pub template_id: Uuid,
    pub name: String,
    pub labels: serde_json::Value,
    pub links_to_templates: Vec<InnerTemplate>,
    pub item_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListTemplateResponse {
    pub templates: Vec<Template>,
    pub total: u64,
}

/// List all templates
#[utoipa::path(
    get,
    path = "/templates",
    params(ListTemplateParams),
    responses(
        (status = 200, description = "List of templates", body = ResponseData<ListTemplateResponse>),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "templates",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn list_templates(
    State(state): State<Arc<AppState>>,
    ApiQuery(params): ApiQuery<ListTemplateParams>,
) -> Result<ResponseData<ListTemplateResponse>, ResponseError> {
    let client = state.db_pool.get().await.ctx("get database connection")?;

    let mut query = DbQuery::select();
    query
        .column((Templates::Table, Templates::TemplateId))
        .column((Templates::Table, Templates::Name))
        .column((Templates::Table, Templates::Labels))
        .expr_as(
            Expr::cust("coalesce(jsonb_agg(distinct jsonb_build_object('template_id', linked_templates.template_id, 'name', linked_templates.name)) filter (where linked_templates.template_id is not null), '[]'::jsonb)"),
            "links_to_templates",
        )
        .expr_as(
            Expr::cust("COUNT(items.item_id)"),
            "item_count",
        )
        .from(Templates::Table)
        .join(
            JoinType::LeftJoin,
            TemplateLinks::Table,
            Expr::col((Templates::Table, Templates::TemplateId))
                .equals((TemplateLinks::Table, TemplateLinks::TemplateId)),
        )
        .join_as(
            JoinType::LeftJoin,
            Templates::Table,
            "linked_templates",
            Expr::col((TemplateLinks::Table, TemplateLinks::LinkToTemplateId))
                .equals(("linked_templates", Templates::TemplateId)),
        )
        .join(
            JoinType::LeftJoin,
            HostTemplates::Table,
            Expr::col((Templates::Table, Templates::TemplateId))
                .equals((HostTemplates::Table, HostTemplates::TemplateId)),
        )
        .join(
            JoinType::LeftJoin,
            Items::Table,
            Expr::col((HostTemplates::Table, HostTemplates::HostTemplateId))
                .equals((Items::Table, Items::HostTemplateId)),
        )
        .group_by_col((Templates::Table, Templates::TemplateId));

    if let Some(name) = &params.name {
        query
            .and_where(Expr::col((Templates::Table, Templates::Name)).ilike(format!("%{}%", name)));
    }

    let total_query = generate_count_query(&query);

    query.order_by(params.order_by, params.order_dir.into());
    query.limit(params.limit).offset(params.offset);

    // get templates
    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let rows = client
        .query(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    let templates_result: Result<Vec<Template>, Box<dyn std::error::Error>> = rows
        .iter()
        .map(|row| {
            let links_to_templates: serde_json::Value = row.try_get(3)?;
            let links_to_templates: Vec<InnerTemplate> =
                serde_json::from_value(links_to_templates)?;

            Ok(Template {
                template_id: row.try_get(0)?,
                name: row.try_get(1)?,
                labels: row.try_get(2)?,
                links_to_templates,
                item_count: row.try_get(4)?,
            })
        })
        .collect();

    let templates = templates_result.ctx("parse templates")?;

    // get total count
    let (total_sql, total_values) = total_query.build_postgres(PostgresQueryBuilder);
    let total_row: i64 = client
        .query_one_scalar(total_sql.as_str(), &total_values.as_params())
        .await
        .ctx("execute total count query")?;

    let response = ListTemplateResponse {
        templates,
        total: total_row as u64,
    };

    Ok(ResponseData::from_status(StatusCode::OK).with_data(response))
}
