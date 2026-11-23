// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::error::ApiResultExt;
use crate::response::{ResponseData, ResponseError};
use crate::state::AppState;
use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use tokio_postgres::error::SqlState;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApplyTemplatesRequest {
    pub host_ids: Vec<Uuid>,
    pub template_ids: Vec<Uuid>,
}

type EmptyResponse = ();

fn apply_db_result<T>(
    result: Result<T, tokio_postgres::Error>,
    op: &'static str,
    action: &str,
    table: Option<&str>,
) -> Result<T, ResponseError> {
    match result {
        Err(error) if error.code() == Some(&SqlState::LOCK_NOT_AVAILABLE) => {
            let response =
                ResponseError::from_status(StatusCode::LOCKED).with_title("Database lock timeout");
            Err(match table {
                Some(table) => response
                    .with_detail(format!(
                        "Timed out waiting for a lock on the {table} table while {action}."
                    ))
                    .with_data(serde_json::json!({ "table": table })),
                None => response.with_detail(format!(
                    "Timed out waiting for a database lock while {action}."
                )),
            })
        }
        other => other.ctx(op),
    }
}

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
        (status = 423, description = "Database lock timeout", body = ResponseError),
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
    tx.batch_execute("SET LOCAL lock_timeout = '5s'")
        .await
        .ctx("set transaction lock timeout")?;

    // Lock all host rows and verify existence
    let host_rows = apply_db_result(
        tx.query(
            "SELECT host_id, name FROM hosts WHERE host_id = ANY($1) FOR UPDATE",
            &[&payload.host_ids],
        )
        .await,
        "acquire lock on hosts",
        "checking the selected hosts",
        Some("hosts"),
    )?;

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
    let template_rows = apply_db_result(
        tx.query(
            "SELECT template_id, name FROM templates WHERE template_id = ANY($1) FOR UPDATE",
            &[&payload.template_ids],
        )
        .await,
        "acquire lock on templates",
        "checking the selected templates",
        Some("templates"),
    )?;

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

    // Recursive UNION visits each node once per root instead of tracking individual paths.
    if template_name_map.len() > 1 {
        let template_conflict_statement = r#"
            WITH RECURSIVE template_reach(root_template_id, template_id) AS (
                SELECT t.template_id, t.template_id
                FROM unnest($1::uuid[]) AS t(template_id)
                UNION
                SELECT r.root_template_id, neighbors.template_id
                FROM template_reach r
                CROSS JOIN LATERAL (
                    SELECT t.link_to_template_id FROM template_links t
                    WHERE t.template_id = r.template_id
                    UNION ALL
                    SELECT t.template_id FROM template_links t
                    WHERE t.link_to_template_id = r.template_id
                ) AS neighbors(template_id)
            )
            SELECT root_template_id, template_id AS connected_template_id
            FROM template_reach
            WHERE template_id != root_template_id
            AND template_id = ANY($1::uuid[])
        "#;

        let template_conflict_rows = apply_db_result(
            tx.query(template_conflict_statement, &[&payload.template_ids])
                .await,
            "execute template conflict check query",
            "checking template inheritance",
            Some("template_links"),
        )?;

        if !template_conflict_rows.is_empty() {
            let mut conflicting_template_ids: BTreeSet<Uuid> = BTreeSet::new();
            for row in &template_conflict_rows {
                if let Ok(id) = row.try_get::<_, Uuid>(0) {
                    conflicting_template_ids.insert(id);
                }
                if let Ok(id) = row.try_get::<_, Uuid>(1) {
                    conflicting_template_ids.insert(id);
                }
            }

            let conflicting_template_ids: Vec<Uuid> =
                conflicting_template_ids.into_iter().collect();

            return Err(ResponseError::from_status(StatusCode::CONFLICT)
                .with_title("Conflicting template inheritance")
                .with_detail("The following templates are connected via inheritance and cannot be applied together: {}. \
                     Apply only one template from each connected group.")
                .with_data(serde_json::json!({
                    "conflicting_templates": conflicting_template_ids
                })));
        }
    }

    // Host and template UUIDs can overlap, so node identity includes the kind.
    let check_statement = r#"
        WITH RECURSIVE host_reach(root_host_id, is_template, node_id) AS (
            SELECT h.host_id, FALSE, h.host_id
            FROM unnest($1::uuid[]) AS h(host_id)
            UNION
            SELECT r.root_host_id, neighbors.is_template, neighbors.node_id
            FROM host_reach r
            CROSS JOIN LATERAL (
                SELECT TRUE, t.link_to_template_id FROM template_links t
                WHERE r.is_template AND t.template_id = r.node_id
                UNION ALL
                SELECT TRUE, t.template_id FROM template_links t
                WHERE r.is_template AND t.link_to_template_id = r.node_id
                UNION ALL
                SELECT FALSE, ht.host_id FROM host_templates ht
                WHERE r.is_template AND ht.template_id = r.node_id AND ht.host_id IS NOT NULL
                UNION ALL
                SELECT TRUE, ht.template_id FROM host_templates ht
                WHERE NOT r.is_template AND ht.host_id = r.node_id AND ht.template_id IS NOT NULL
            ) AS neighbors(is_template, node_id)
        )
        SELECT DISTINCT root_host_id
        FROM host_reach
        WHERE is_template AND node_id = ANY($2::uuid[])
    "#;

    let conflicting_rows = apply_db_result(
        tx.query(check_statement, &[&payload.host_ids, &payload.template_ids])
            .await,
        "execute connection check query",
        "checking for existing host and template connections",
        None,
    )?;

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

    apply_db_result(
        tx.execute(bulk_sql, &[&payload.host_ids, &payload.template_ids])
            .await,
        "execute bulk apply",
        "applying the templates",
        None,
    )?;

    apply_db_result(
        tx.commit().await,
        "commit transaction",
        "committing the template application",
        None,
    )?;

    Ok(ResponseData::from_status(StatusCode::OK))
}
