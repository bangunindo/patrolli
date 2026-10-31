// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApplyTemplatesRequest {
    pub host_ids: Vec<Uuid>,
    pub template_ids: Vec<Uuid>,
}

type EmptyResponse = ();

/// Apply multiple templates to multiple hosts
///
/// Apply a set of templates to a set of hosts. All specified hosts must not
/// have any existing connection (direct or through template inheritance) to
/// any of the specified templates. If any host already has a connection to
/// any template, the operation is rejected and no changes are made.
#[utoipa::path(
    post,
    path = "/templates/apply",
    request_body = ApplyTemplatesRequest,
    responses(
        (status = 200, description = "Templates applied to all hosts", body = ResponseData<EmptyResponse>),
        (status = 400, description = "No host IDs or template IDs provided", body = ResponseError),
        (status = 404, description = "One or more hosts/templates not found", body = ResponseError),
        (status = 409, description = "One or more hosts already have a connection to a template", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "templates",
    operation_id = "bulk_apply_template",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn apply_template(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ApplyTemplatesRequest>,
) -> Result<ResponseData<()>, ResponseError> {
    if payload.host_ids.is_empty() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("No host IDs provided")
            .with_detail("At least one host_id must be specified"));
    }
    if payload.template_ids.is_empty() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("No template IDs provided")
            .with_detail("At least one template_id must be specified"));
    }

    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // Lock all host rows and verify existence
    let host_rows = tx
        .query(
            "SELECT host_id, name FROM hosts WHERE host_id = ANY($1) FOR UPDATE",
            &[&payload.host_ids],
        )
        .await
        .ctx("acquire lock on hosts")?;

    let mut host_name_map: HashMap<Uuid, String> = HashMap::with_capacity(host_rows.len());
    for row in &host_rows {
        let id: Uuid = row.try_get(0).ctx("get host id")?;
        let name: String = row.try_get(1).ctx("get host name")?;
        host_name_map.insert(id, name);
    }

    // Check that all requested hosts were found
    for host_id in &payload.host_ids {
        if !host_name_map.contains_key(host_id) {
            return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
                .with_title("Host not found")
                .with_detail(format!("Host with ID {} not found", host_id)));
        }
    }

    // Lock all template rows and verify existence
    let template_rows = tx
        .query(
            "SELECT template_id, name FROM templates WHERE template_id = ANY($1) FOR UPDATE",
            &[&payload.template_ids],
        )
        .await
        .ctx("acquire lock on templates")?;

    let mut template_name_map: HashMap<Uuid, String> = HashMap::with_capacity(template_rows.len());
    for row in &template_rows {
        let id: Uuid = row.try_get(0).ctx("get template id")?;
        let name: String = row.try_get(1).ctx("get template name")?;
        template_name_map.insert(id, name);
    }

    // Check that all requested templates were found
    for template_id in &payload.template_ids {
        if !template_name_map.contains_key(template_id) {
            return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
                .with_title("Template not found")
                .with_detail(format!("Template with ID {} not found", template_id)));
        }
    }

    // Check for template inheritance conflicts among the specified templates
    let template_conflict_statement = r#"
        WITH RECURSIVE t_edges AS (
            SELECT TRUE AS src_is_template, template_id AS src_uuid,
                TRUE AS dst_is_template, link_to_template_id AS dst_uuid
            FROM template_links
            UNION ALL
            SELECT TRUE AS src_is_template, link_to_template_id AS src_uuid,
                TRUE AS dst_is_template, template_id AS dst_uuid
            FROM template_links
        ),
        t_reach AS (
            SELECT ROW(TRUE, t.template_id) AS node,
                t.template_id AS root_template_id,
                TRUE AS node_is_template, t.template_id AS node_uuid, 0 AS depth
            FROM unnest($1::uuid[]) AS t(template_id)
            UNION ALL
            SELECT ROW(e.dst_is_template, e.dst_uuid) AS node,
                tr.root_template_id,
                e.dst_is_template, e.dst_uuid, tr.depth + 1
            FROM t_edges e
            JOIN t_reach tr ON tr.node_is_template = e.src_is_template AND tr.node_uuid = e.src_uuid
            WHERE NOT tr.is_cycle
        ) CYCLE node SET is_cycle USING path
        SELECT DISTINCT tr.root_template_id, tr.node_uuid AS connected_template_id
        FROM t_reach tr
        WHERE tr.node_is_template = TRUE
        AND tr.node_uuid != tr.root_template_id
        AND tr.node_uuid = ANY($1::uuid[])
    "#;

    let template_conflict_rows = tx
        .query(template_conflict_statement, &[&payload.template_ids])
        .await
        .ctx("execute template conflict check query")?;

    if !template_conflict_rows.is_empty() {
        // Collect unique conflicting template names
        let mut conflicting_template_ids: BTreeSet<Uuid> = BTreeSet::new();
        for row in &template_conflict_rows {
            if let Ok(id) = row.try_get::<_, Uuid>(0) {
                conflicting_template_ids.insert(id);
            }
            if let Ok(id) = row.try_get::<_, Uuid>(1) {
                conflicting_template_ids.insert(id);
            }
        }

        let conflicting_template_ids: Vec<Uuid> = conflicting_template_ids.into_iter().collect();

        return Err(ResponseError::from_status(StatusCode::CONFLICT)
            .with_title("Conflicting template inheritance")
            .with_detail("The following templates are connected via inheritance and cannot be applied together: {}. \
                 Apply only one template from each connected group.")
            .with_data(serde_json::json!({
                "conflicting_templates": conflicting_template_ids
            })));
    }

    // Check that no host has a connection to any template
    //
    // Run a single recursive CTE starting from all hosts simultaneously,
    // traversing the undirected template-link / host-template graph, and
    // collecting which hosts can reach any of the specified templates.
    //
    // If any host can reach any template, return a conflict error with the list of
    // conflicting hosts.
    let check_statement = r#"
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
        all_host_reach AS (
            SELECT ROW(FALSE, h.host_id) AS node,
                h.host_id AS root_host_id,
                FALSE AS node_is_template, h.host_id AS node_uuid, 0 AS depth
            FROM unnest($1::uuid[]) AS h(host_id)
            UNION ALL
            SELECT ROW(e.dst_is_template, e.dst_uuid) AS node,
                hr.root_host_id,
                e.dst_is_template, e.dst_uuid, hr.depth + 1
            FROM edges e
            JOIN all_host_reach hr ON hr.node_is_template = e.src_is_template AND hr.node_uuid = e.src_uuid
            WHERE NOT hr.is_cycle
        ) CYCLE node SET is_cycle USING path
        SELECT DISTINCT hr.root_host_id
        FROM all_host_reach hr
        WHERE hr.node_is_template = TRUE
        AND hr.node_uuid = ANY($2::uuid[])
        AND NOT hr.is_cycle
    "#;

    let conflicting_rows = tx
        .query(check_statement, &[&payload.host_ids, &payload.template_ids])
        .await
        .ctx("execute connection check query")?;

    let conflicting_host_names: Vec<String> = conflicting_rows
        .iter()
        .filter_map(|row| {
            let host_id: Uuid = row.try_get(0).ok()?;
            host_name_map.get(&host_id).cloned()
        })
        .collect();

    if !conflicting_host_names.is_empty() {
        let detail = if conflicting_host_names.len() == 1 {
            format!(
                "Host \"{}\" is already connected to one or more of the specified templates. \
                 Each host must be disconnected from all specified templates before applying.",
                conflicting_host_names[0]
            )
        } else {
            format!(
                "The following hosts are already connected to one or more of the specified templates: {}. \
                 Each host must be disconnected from all specified templates before applying.",
                conflicting_host_names.join(", ")
            )
        };

        return Err(ResponseError::from_status(StatusCode::CONFLICT)
            .with_title("Conflicting template application")
            .with_detail(detail));
    }

    // Bulk insert all host_templates entries and copy items in one pass
    let bulk_sql = r#"
        WITH new_ht AS (
            INSERT INTO host_templates (host_template_id, host_id, template_id)
            SELECT uuidv7(), h.host_id, t.template_id
            FROM unnest($1::uuid[]) AS h(host_id)
            CROSS JOIN unnest($2::uuid[]) AS t(template_id)
            RETURNING host_template_id, template_id
        )
        INSERT INTO items (
            item_id, host_template_id, template_item_id,
            listener_id, endpoint_id, type, flags, adm_status,
            name, schedule, timeout_ms, config, labels, last_updated
        )
        SELECT
            uuidv7() AS item_id,
            nht.host_template_id,
            ti.item_id AS template_item_id,
            NULL::uuid AS listener_id,
            NULL::uuid AS endpoint_id,
            ti.type, ti.flags, ti.adm_status,
            NULL::text AS name,
            NULL::text AS schedule,
            NULL::int4 AS timeout_ms,
            NULL::jsonb AS config,
            NULL::jsonb AS labels,
            now() AS last_updated
        FROM new_ht nht
        JOIN host_templates template_ht
            ON template_ht.template_id = nht.template_id
            AND template_ht.host_id IS NULL
        JOIN items ti
            ON ti.host_template_id = template_ht.host_template_id
    "#;

    tx.execute(bulk_sql, &[&payload.host_ids, &payload.template_ids])
        .await
        .ctx("execute bulk apply")?;

    tx.commit().await.ctx("commit transaction")?;

    Ok(ResponseData::from_status(StatusCode::OK))
}
