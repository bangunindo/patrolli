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
pub struct LinkTemplateRequest {
    pub link_to_template_id: Uuid,
}

type EmptyResponse = ();

/// Link a template to another template
#[utoipa::path(
    put,
    path = "/templates/link/{id}",
    params(
        ("id" = Uuid, Path, description = "Template UUID")
    ),
    request_body = LinkTemplateRequest,
    responses(
        (status = 200, description = "Template updated", body = ResponseData<EmptyResponse>),
        (status = 404, description = "Template not found/Template destination not found", body = ResponseError),
        (status = 400, description = "Cannot link template to itself", body = ResponseError),
        (status = 409, description = "Link already exists", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "templates",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn link_template(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<LinkTemplateRequest>,
) -> Result<ResponseData<EmptyResponse>, ResponseError> {
    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    if id == payload.link_to_template_id {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Invalid template link")
            .with_detail("Cannot link template to itself"));
    }

    // Acquire 2 row-level locsk to prevent race conditions when linking templates concurrently.
    let lock_sql = "SELECT 1 FROM templates WHERE template_id = $1::uuid FOR UPDATE";
    tx.query_opt(lock_sql, &[&id])
        .await
        .ctx("acquire lock on source template")?;
    tx.query_opt(lock_sql, &[&payload.link_to_template_id])
        .await
        .ctx("acquire lock on target template")?;

    // get link src
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

    // get link target
    let mut query = DbQuery::select();
    query
        .column(Templates::Name)
        .from(Templates::Table)
        .and_where(Expr::col(Templates::TemplateId).eq(payload.link_to_template_id));

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
                payload.link_to_template_id
            )));
    };

    let target_template_name: String = target_template.try_get(0).ctx("get template name")?;

    // check if link already exists
    let mut query = DbQuery::select();
    query.from(TemplateLinks::Table).and_where(
        Expr::col(TemplateLinks::TemplateId)
            .eq(id)
            .and(Expr::col(TemplateLinks::LinkToTemplateId).eq(payload.link_to_template_id)),
    );

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);

    let existing_links = tx
        .query(sql.as_str(), &values.as_params())
        .await
        .ctx("execute existing link check query")?;

    if !existing_links.is_empty() {
        return Err(ResponseError::from_status(StatusCode::CONFLICT)
            .with_title("Template link already exists"));
    }

    // check if templates are already connected (undirected), returning the nearest meeting node
    let statement = "
        WITH RECURSIVE edges AS (
            SELECT TRUE AS src_is_template, template_id AS src_uuid,
                TRUE AS dst_is_template, link_to_template_id AS dst_uuid
            FROM template_links
            UNION ALL
            SELECT TRUE AS src_is_template, link_to_template_id AS src_uuid,
                TRUE AS dst_is_template, template_id AS dst_uuid
            FROM template_links
            UNION ALL
            SELECT TRUE AS src_is_template, template_id AS src_uuid,
                FALSE AS dst_is_template, host_id AS dst_uuid
            FROM host_templates
            WHERE host_id IS NOT NULL
            UNION ALL
            SELECT FALSE AS src_is_template, host_id AS src_uuid,
                TRUE AS dst_is_template, template_id AS dst_uuid
            FROM host_templates
            WHERE host_id IS NOT NULL
        ),
        nodes AS (
            SELECT TRUE AS node_is_template, template_id AS node_uuid, name FROM templates
            UNION ALL
            SELECT FALSE AS node_is_template, host_id AS node_uuid, name FROM hosts
        ),
        a_reach AS (
            SELECT ROW(TRUE, $1::uuid) AS node,
                TRUE AS node_is_template, $1::uuid AS node_uuid, 0 AS depth
            UNION ALL
            SELECT ROW(e.dst_is_template, e.dst_uuid) AS node,
                e.dst_is_template, e.dst_uuid, ar.depth + 1
            FROM edges e
            JOIN a_reach ar ON ar.node_is_template = e.src_is_template AND ar.node_uuid = e.src_uuid
            WHERE NOT ar.is_cycle
        ) CYCLE node SET is_cycle USING path,
        b_reach AS (
            SELECT ROW(TRUE, $2::uuid) AS node,
                TRUE AS node_is_template, $2::uuid AS node_uuid, 0 AS depth
            UNION ALL
            SELECT ROW(e.dst_is_template, e.dst_uuid) AS node,
                e.dst_is_template, e.dst_uuid, br.depth + 1
            FROM edges e
            JOIN b_reach br ON br.node_is_template = e.src_is_template AND br.node_uuid = e.src_uuid
            WHERE NOT br.is_cycle
        ) CYCLE node SET is_cycle USING path,
        a_min AS (
            SELECT node, node_is_template, node_uuid, MIN(depth) AS depth
            FROM a_reach
            WHERE NOT is_cycle
            GROUP BY node, node_is_template, node_uuid
        ),
        b_min AS (
            SELECT node, node_is_template, node_uuid, MIN(depth) AS depth
            FROM b_reach
            WHERE NOT is_cycle
            GROUP BY node, node_is_template, node_uuid
        )
        SELECT a.node_is_template, a.node_uuid, n.name
        FROM a_min a
        JOIN b_min b ON b.node = a.node
        JOIN nodes n ON n.node_is_template = a.node_is_template AND n.node_uuid = a.node_uuid
        WHERE n.node_is_template = TRUE -- hosts cannot be shared ancestor
        ORDER BY (a.depth + b.depth), b.depth, a.depth
        LIMIT 1;
    ";

    let existing_chain_opt = tx
        .query_opt(statement, &[&id, &payload.link_to_template_id])
        .await
        .ctx("execute inheritance chain check query")?;

    if let Some(row) = existing_chain_opt {
        let conflict_name: String = row.get(2);

        return Err(ResponseError::from_status(StatusCode::CONFLICT)
            .with_title("Conflicting template link")
            .with_detail(format!(
                "Cannot link template \"{}\" to template \"{}\", because template \"{}\" would be linked twice.",
                template_name,
                target_template_name,
                conflict_name
            )));
    }

    // insert new
    let template_link_id = Uuid::now_v7();
    let (insert_sql, values) = DbQuery::insert()
        .into_table(TemplateLinks::Table)
        .columns(vec![
            TemplateLinks::TemplateLinkId,
            TemplateLinks::TemplateId,
            TemplateLinks::LinkToTemplateId,
        ])
        .values(vec![
            template_link_id.into(),
            id.into(),
            payload.link_to_template_id.into(),
        ])
        .ctx("build query")?
        .build_postgres(PostgresQueryBuilder);

    _ = tx
        .execute(insert_sql.as_str(), &values.as_params())
        .await
        .ctx("execute insertion query")?;

    // check circular link
    let statement = "
        WITH RECURSIVE tree AS (
            SELECT template_id, link_to_template_id FROM template_links
            WHERE template_id = $1 and link_to_template_id = $2
            UNION
            SELECT template_links.template_id, template_links.link_to_template_id FROM template_links
            JOIN tree ON tree.template_id = template_links.link_to_template_id
        ) CYCLE template_id SET is_cycle USING path
        SELECT * FROM tree
        WHERE is_cycle IS TRUE;
    ";

    let rows = tx
        .query(statement, &[&id, &payload.link_to_template_id])
        .await
        .ctx("execute circular detection query")?;

    if !rows.is_empty() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Invalid template link")
            .with_detail("Cannot create circular link"));
    }

    // create child recursively
    let statement = "
        WITH RECURSIVE curr_template AS (
            SELECT host_template_id, template_id
            FROM host_templates
            WHERE template_id = $1::uuid AND host_id IS NULL
        ),
        parent_template AS (
            SELECT host_template_id
            FROM host_templates
            WHERE template_id = $2::uuid AND host_id IS NULL
        ),
        template_ids AS (
            SELECT template_id FROM curr_template
            UNION
            SELECT tl.template_id
            FROM template_links tl
            JOIN template_ids p 
            ON p.template_id = tl.link_to_template_id
        ),
        template_nodes AS (
            -- exclude host_templates that has host_id
            SELECT ht.host_template_id, ht.template_id
            FROM host_templates ht
            JOIN template_ids t ON t.template_id = ht.template_id
            WHERE ht.host_id IS NULL
        ),
        edges AS (
            -- template -> template
            SELECT
                p.host_template_id AS parent_id,
                c.host_template_id AS child_id
            FROM template_links tl
            JOIN template_nodes p ON p.template_id = tl.link_to_template_id
            JOIN template_nodes c ON c.template_id = tl.template_id
            UNION ALL
            -- template -> host
            SELECT
                p.host_template_id AS parent_id,
                h.host_template_id AS child_id
            FROM template_nodes p
            JOIN host_templates h
            ON h.template_id = p.template_id
            AND h.host_id IS NOT NULL
        ),
        parent_items AS (
            SELECT
                i.item_id,
                i.listener_id,
                i.endpoint_id,
                i.type,
                i.flags,
                i.adm_status,
                i.name,
                i.schedule,
                i.timeout_ms,
                i.config,
                i.labels
            FROM items i
            WHERE i.host_template_id = (SELECT host_template_id FROM parent_template)
            -- exclude items that are somehow already linked to the destination template to avoid duplication
            -- the point is to make this query idempotent for safety, 
            -- even though in practice this should not be an issue since we check for existing link beforehand
            AND i.item_id NOT IN (
                SELECT template_item_id FROM items i2 
                JOIN host_templates ht2 ON i2.host_template_id = ht2.host_template_id
                WHERE ht2.template_id = $1::uuid AND ht2.host_id IS NULL AND i2.template_item_id IS NOT NULL
            )
        ),
        item_tree AS (
            SELECT
                uuidv7() AS new_item_id,
                (SELECT host_template_id FROM curr_template) AS host_template_id,
                i.item_id AS template_item_id,
                i.listener_id,
                i.endpoint_id,
                i.type,
                i.flags,
                i.adm_status,
                NULL::text AS name,
                NULL::text AS schedule,
                NULL::int4 AS timeout_ms,
                NULL::jsonb AS config,
                NULL::jsonb AS labels,
                0 AS depth
            FROM parent_items i
            UNION ALL
            SELECT
                uuidv7() AS new_item_id,
                e.child_id AS host_template_id,
                it.new_item_id AS template_item_id,
                NULL::uuid AS listener_id,
                NULL::uuid AS endpoint_id,
                it.type,
                it.flags,
                it.adm_status,
                NULL::text AS name,
                NULL::text AS schedule,
                NULL::int4 AS timeout_ms,
                NULL::jsonb AS config,
                NULL::jsonb AS labels,
                it.depth + 1
            FROM item_tree it
            JOIN edges e ON e.parent_id = it.host_template_id
        )
        INSERT INTO items (
            item_id,
            host_template_id,
            template_item_id,
            listener_id,
            endpoint_id,
            type,
            flags,
            adm_status,
            name,
            schedule,
            timeout_ms,
            last_updated,
            config,
            labels
        )
        SELECT
            new_item_id,
            host_template_id,
            template_item_id,
            listener_id,
            endpoint_id,
            type,
            flags,
            adm_status,
            name,
            schedule,
            timeout_ms,
            now(),
            config,
            labels
        FROM item_tree;";

    let _ = tx
        .execute(statement, &[&id, &payload.link_to_template_id])
        .await
        .ctx("execute child creation query")?;

    tx.commit().await.ctx("commit transaction")?;

    Ok(ResponseData::from_status(StatusCode::OK))
}
