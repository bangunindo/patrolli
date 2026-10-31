// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::{HostTemplates, Items, TemplateLinks, Templates};
use axum::{
    extract::{Path, State},
    http::StatusCode,
};
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
pub struct GetTemplateResponse {
    pub template_id: Uuid,
    pub name: String,
    pub labels: serde_json::Value,
    pub links_to_templates: Vec<InnerTemplate>,
    pub item_count: i64,
}

/// Get a template by ID
#[utoipa::path(
    get,
    path = "/templates/{id}",
    params(
        ("id" = Uuid, Path, description = "Template UUID")
    ),
    responses(
        (status = 200, description = "Template found", body = ResponseData<GetTemplateResponse>),
        (status = 404, description = "Template not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "templates",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn get_template(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<ResponseData<GetTemplateResponse>, ResponseError> {
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
        .and_where(Expr::col((Templates::Table, Templates::TemplateId)).eq(id))
        .group_by_col((Templates::Table, Templates::TemplateId));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);

    let row_opt = client
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    let Some(row) = row_opt else {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Template not found")
            .with_detail(format!("No template found for id: {}", id)));
    };
    let template = GetTemplateResponse {
        template_id: row.try_get(0).ctx("parse template_id")?,
        name: row.try_get(1).ctx("parse name")?,
        labels: row.try_get(2).ctx("parse labels")?,
        links_to_templates: {
            let links_json: serde_json::Value = row.try_get(3).ctx("parse links_to_templates")?;
            serde_json::from_value(links_json).ctx("deserialize links_to_templates")?
        },
        item_count: row.try_get(4).ctx("parse item_count")?,
    };

    Ok(ResponseData::from_status(StatusCode::OK).with_data(template))
}
