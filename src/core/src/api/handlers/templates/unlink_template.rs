// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::{TemplateLinks, Templates};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UnlinkTemplateRequest {
    pub unlink_from_template_id: Uuid,
    pub keep_items: bool,
}

type EmptyResponse = ();

/// Unlink a template from another template
#[utoipa::path(
    put,
    path = "/templates/unlink/{id}",
    params(
        ("id" = Uuid, Path, description = "Template UUID")
    ),
    request_body = UnlinkTemplateRequest,
    responses(
        (status = 200, description = "Template unlinked", body = ResponseData<EmptyResponse>),
        (status = 404, description = "Template not found / Template destination not found / Link not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "templates",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn unlink_template(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UnlinkTemplateRequest>,
) -> Result<ResponseData<EmptyResponse>, ResponseError> {
    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // Validate source template exists
    let mut query = DbQuery::select();
    query
        .column(Templates::Name)
        .from(Templates::Table)
        .and_where(Expr::col(Templates::TemplateId).eq(id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);

    let src_template_opt = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("execute source template query")?;

    let Some(src_template) = src_template_opt else {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Template not found")
            .with_detail(format!("No template found for id: {}", id)));
    };

    let template_name: String = src_template.try_get(0).ctx("get template name")?;

    // Validate target template exists
    let mut query = DbQuery::select();
    query
        .column(Templates::Name)
        .from(Templates::Table)
        .and_where(Expr::col(Templates::TemplateId).eq(payload.unlink_from_template_id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);

    let target_template_opt = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("execute target template query")?;

    let Some(target_template) = target_template_opt else {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Template destination not found")
            .with_detail(format!(
                "No template found for id: {}",
                payload.unlink_from_template_id
            )));
    };

    let target_template_name: String = target_template.try_get(0).ctx("get template name")?;

    // Check if the link exists
    let mut query = DbQuery::select();
    query
        .column(TemplateLinks::TemplateLinkId)
        .from(TemplateLinks::Table)
        .and_where(
            Expr::col(TemplateLinks::TemplateId).eq(id).and(
                Expr::col(TemplateLinks::LinkToTemplateId).eq(payload.unlink_from_template_id),
            ),
        );

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);

    let link_opt = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("execute link check query")?;

    if link_opt.is_none() {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Template link not found")
            .with_detail(format!(
                "No link from template \"{}\" to template \"{}\" found",
                template_name, target_template_name
            )));
    }

    if payload.keep_items {
        // Resolve items that are direct children of items target template
        // So the inheritance chain would resolve to the same thing
        let statement = r#"
            WITH RECURSIVE
            a_ht AS (
                SELECT host_template_id FROM host_templates WHERE template_id = $1::uuid AND host_id IS NULL
            ),
            b_items AS (
                SELECT i.item_id FROM items i
                JOIN host_templates ht ON i.host_template_id = ht.host_template_id
                WHERE ht.template_id = $2::uuid AND ht.host_id IS NULL
            ),
            a_inherited_items AS (
                SELECT i.item_id, i.template_item_id, i.name, i.schedule, i.timeout_ms, i.config, i.labels
                FROM items i
                JOIN a_ht ON i.host_template_id = a_ht.host_template_id
                WHERE i.template_item_id IN (SELECT item_id FROM b_items)
            ),
            resolution AS (
                SELECT
                    aii.item_id,
                    i.template_item_id,
                    COALESCE(aii.name, i.name) AS resolved_name,
                    COALESCE(aii.schedule, i.schedule) AS resolved_schedule,
                    COALESCE(aii.timeout_ms, i.timeout_ms) AS resolved_timeout_ms,
                    COALESCE(aii.config, i.config) AS resolved_config,
                    COALESCE(aii.labels, i.labels) AS resolved_labels,
                    0 AS depth
                FROM a_inherited_items aii
                JOIN items i ON i.item_id = aii.template_item_id

                UNION ALL

                SELECT
                    r.item_id,
                    i.template_item_id,
                    COALESCE(r.resolved_name, i.name),
                    COALESCE(r.resolved_schedule, i.schedule),
                    COALESCE(r.resolved_timeout_ms, i.timeout_ms),
                    COALESCE(r.resolved_config, i.config),
                    COALESCE(r.resolved_labels, i.labels),
                    r.depth + 1
                FROM resolution r
                JOIN items i ON i.item_id = r.template_item_id
                WHERE r.template_item_id IS NOT NULL
            ),
            final_resolved AS (
                SELECT DISTINCT ON (item_id)
                    item_id, resolved_name, resolved_schedule, resolved_timeout_ms, resolved_config, resolved_labels
                FROM resolution
                ORDER BY item_id, depth DESC
            )
            UPDATE items SET
                template_item_id = NULL,
                name = fr.resolved_name,
                schedule = fr.resolved_schedule,
                timeout_ms = fr.resolved_timeout_ms,
                config = fr.resolved_config,
                labels = fr.resolved_labels
            FROM final_resolved fr
            WHERE items.item_id = fr.item_id
        "#;

        tx.execute(statement, &[&id, &payload.unlink_from_template_id])
            .await
            .ctx("resolve inherited items")?;
    } else {
        // Recursively delete all items inherited from the unlink target template.
        // Similar to delete_template
        let statement = r#"
            WITH RECURSIVE
            b_items AS (
                SELECT i.item_id FROM items i
                JOIN host_templates ht ON i.host_template_id = ht.host_template_id
                WHERE ht.template_id = $2::uuid AND ht.host_id IS NULL
            ),
            inherited AS (
                SELECT i.item_id FROM items i
                JOIN host_templates ht ON i.host_template_id = ht.host_template_id
                WHERE ht.template_id = $1::uuid AND ht.host_id IS NULL
                AND i.template_item_id IN (SELECT item_id FROM b_items)
                UNION ALL
                SELECT i.item_id FROM items i
                JOIN inherited ON i.template_item_id = inherited.item_id
            )
            DELETE FROM items WHERE item_id IN (SELECT item_id FROM inherited)
        "#;

        tx.execute(statement, &[&id, &payload.unlink_from_template_id])
            .await
            .ctx("delete inherited items")?;
    }

    // Delete the link
    let mut query = DbQuery::delete();
    query.from_table(TemplateLinks::Table).and_where(
        Expr::col(TemplateLinks::TemplateId)
            .eq(id)
            .and(Expr::col(TemplateLinks::LinkToTemplateId).eq(payload.unlink_from_template_id)),
    );

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    tx.execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete template link")?;

    tx.commit().await.ctx("commit transaction")?;

    Ok(ResponseData::from_status(StatusCode::OK))
}
