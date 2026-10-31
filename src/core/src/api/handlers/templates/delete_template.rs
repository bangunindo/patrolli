// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::{HostTemplates, TemplateLinks, Templates};
use axum::{extract::Path, extract::State, http::StatusCode};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use std::sync::Arc;
use uuid::Uuid;

type DeleteTemplateResponse = ();

/// Delete a template
#[utoipa::path(
    delete,
    path = "/templates/{id}",
    params(
        ("id" = Uuid, Path, description = "Template UUID")
    ),
    responses(
        (status = 204, description = "Template deleted", body = ResponseData<DeleteTemplateResponse>),
        (status = 404, description = "Template not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "templates",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn delete_template(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<ResponseData<()>, ResponseError> {
    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // Check if the template exists
    let mut query = DbQuery::select();
    query
        .column(Templates::TemplateId)
        .from(Templates::Table)
        .and_where(Expr::col(Templates::TemplateId).eq(id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let result = tx
        .query(sql.as_str(), &values.as_params())
        .await
        .ctx("get template")?;

    if result.is_empty() {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Template not found")
            .with_detail(format!("No template found for id: {}", id)));
    }

    // Delete items from the template and all others that inherit from it
    let statement = "
        WITH RECURSIVE children AS (
            SELECT item_id, template_item_id FROM host_templates ht
            JOIN items i ON i.host_template_id = ht.host_template_id 
            WHERE template_id = $1
        ), tree AS (
            SELECT children.item_id, children.template_item_id FROM children
            UNION ALL
            SELECT items.item_id, items.template_item_id FROM items
            JOIN tree t ON t.item_id = items.template_item_id 
        )  DELETE FROM items WHERE item_id IN (SELECT item_id FROM tree);
    ";

    tx.execute(statement, &[&id])
        .await
        .ctx("delete template items")?;

    // Delete host_templates owned by the template or uses this template
    let mut query = DbQuery::delete();
    query
        .from_table(HostTemplates::Table)
        .and_where(Expr::col(HostTemplates::TemplateId).eq(id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    tx.execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete host_templates")?;

    // Delete template links that links to or from this template
    let mut query = DbQuery::delete();
    query.from_table(TemplateLinks::Table).and_where(
        Expr::col(TemplateLinks::LinkToTemplateId)
            .eq(id)
            .or(Expr::col(TemplateLinks::TemplateId).eq(id)),
    );

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    tx.execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete template links")?;

    // Delete the template itself
    let mut query = DbQuery::delete();
    query
        .from_table(Templates::Table)
        .and_where(Expr::col(Templates::TemplateId).eq(id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);

    let result = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    if result == 0 {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Template not found")
            .with_detail(format!("No template found for id: {}", id)));
    }

    tx.commit().await.ctx("commit transaction")?;

    Ok(ResponseData::from_status(StatusCode::NO_CONTENT))
}
