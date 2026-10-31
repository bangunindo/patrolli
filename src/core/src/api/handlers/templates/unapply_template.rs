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
pub struct UnapplyTemplatesRequestInner {
    pub host_id: Uuid,
    pub template_id: Uuid,
    pub keep_items: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UnapplyTemplatesRequest {
    pub unapply_list: Vec<UnapplyTemplatesRequestInner>,
}

impl UnapplyTemplatesRequest {
    pub fn validate(&self) -> Result<(), ResponseError> {
        if self.unapply_list.is_empty() {
            return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
                .with_title("Invalid request payload")
                .with_detail("unapply_list cannot be empty"));
        }

        let mut counts = HashMap::new();
        let mut duplicates = Vec::new();
        for entry in &self.unapply_list {
            let pair = (entry.host_id, entry.template_id);

            if *counts.entry(pair).or_insert(0) == 1 {
                duplicates.push(pair);
            }

            *counts.get_mut(&pair).unwrap() += 1;
        }

        if !duplicates.is_empty() {
            let duplicate_pairs: Vec<serde_json::Value> = duplicates
                .iter()
                .map(|(host_id, template_id)| {
                    serde_json::json!({
                        "host_id": host_id,
                        "template_id": template_id,
                    })
                })
                .collect();

            return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
                .with_title("Duplicate host_id and template_id pairs found")
                .with_data(serde_json::json!({
                    "duplicate_pairs": duplicate_pairs
                })));
        }

        Ok(())
    }
}

type EmptyResponse = ();

/// Unapply multiple pair of host_id and template_id
///
/// Removing a template from a host will either delete the items that were created
/// from the template, or resolve their inherited fields and keep them as local items.
#[utoipa::path(
    post,
    path = "/templates/unapply",
    request_body = UnapplyTemplatesRequest,
    responses(
        (status = 200, description = "Template removed from host", body = ResponseData<EmptyResponse>),
        (status = 404, description = "Host not found / Template not found / Template not applied to host", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "templates",
    operation_id = "bulk_unapply_template",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn unapply_template(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<UnapplyTemplatesRequest>,
) -> Result<ResponseData<()>, ResponseError> {
    // Validate the request payload
    payload.validate()?;

    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // Check if all hosts and templates exist before proceeding
    let host_ids = payload
        .unapply_list
        .iter()
        .map(|u| u.host_id)
        .collect::<BTreeSet<Uuid>>()
        .into_iter()
        .collect::<Vec<Uuid>>();

    let template_ids = payload
        .unapply_list
        .iter()
        .map(|u| u.template_id)
        .collect::<BTreeSet<Uuid>>()
        .into_iter()
        .collect::<Vec<Uuid>>();

    let host_template_id_pairs = payload
        .unapply_list
        .iter()
        .map(|u| (u.host_id, u.template_id))
        .collect::<BTreeSet<(Uuid, Uuid)>>();

    let host_rows = tx
        .query(
            "SELECT host_id, name FROM hosts WHERE host_id = ANY($1)",
            &[&host_ids],
        )
        .await
        .ctx("query host")?;

    let mut host_map: HashMap<Uuid, String> = Default::default();
    for row in host_rows {
        let host_id: Uuid = row.try_get(0).ctx("get host_id")?;

        let host_name: String = row.try_get(1).ctx("get host name")?;

        host_map.insert(host_id, host_name);
    }

    let template_rows = tx
        .query(
            "SELECT template_id, name FROM templates WHERE template_id = ANY($1)",
            &[&template_ids],
        )
        .await
        .ctx("query template")?;

    let mut template_map: HashMap<Uuid, String> = Default::default();
    for row in template_rows {
        let template_id: Uuid = row.try_get(0).ctx("get template_id")?;

        let template_name: String = row.try_get(1).ctx("get template name")?;

        template_map.insert(template_id, template_name);
    }

    let missing_host_ids: BTreeSet<Uuid> = host_ids
        .iter()
        .cloned()
        .filter(|id| !host_map.contains_key(id))
        .collect();

    let missing_template_ids: BTreeSet<Uuid> = template_ids
        .iter()
        .cloned()
        .filter(|id| !template_map.contains_key(id))
        .collect();

    if !missing_host_ids.is_empty() || !missing_template_ids.is_empty() {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Host or template not found")
            .with_data(serde_json::json!({
                "missing_host_ids": missing_host_ids.into_iter().collect::<Vec<Uuid>>(),
                "missing_template_ids": missing_template_ids.into_iter().collect::<Vec<Uuid>>()
            })));
    }

    // Check if all host-template pairs are actually applied before proceeding
    let query = tx
        .query(
            r#"
        SELECT host_template_id, host_id, template_id
        FROM host_templates
        WHERE host_id = ANY($1) AND template_id = ANY($2)
    "#,
            &[&host_ids, &template_ids], // This query may return more rows than the number of pairs in the payload,
        )
        .await
        .ctx("query host_templates")?;

    let mut pairs_in_db: BTreeSet<(Uuid, Uuid)> = Default::default();
    let mut host_template_map: HashMap<(Uuid, Uuid), Uuid> = Default::default(); // (host_template_id, template_id) -> host_template_id
    for row in query {
        let host_template_id: Uuid = row
            .try_get(0)
            .ctx("get host_template_id from host_templates")?;

        let host_id: Uuid = row.try_get(1).ctx("get host_id from host_templates")?;

        let template_id: Uuid = row.try_get(2).ctx("get template_id from host_templates")?;

        pairs_in_db.insert((host_id, template_id));

        if host_template_id_pairs.contains(&(host_id, template_id)) {
            host_template_map.insert((host_id, template_id), host_template_id);
        }
    }

    let missing_pairs: Vec<(Uuid, Uuid)> = payload
        .unapply_list
        .iter()
        .filter(|u| !pairs_in_db.contains(&(u.host_id, u.template_id)))
        .map(|u| (u.host_id, u.template_id))
        .collect();

    if !missing_pairs.is_empty() {
        let missing_pairs_with_names: Vec<serde_json::Value> = missing_pairs
            .iter()
            .map(|(host_id, template_id)| {
                serde_json::json!({
                    "host_id": host_id,
                    "template_id": template_id,
                })
            })
            .collect();

        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Found templates that is not applied to the host")
            .with_data(serde_json::json!({
                "missing_pairs": missing_pairs_with_names
            })));
    }

    // Get base host_template_ids for all hosts involved in the unapply operation
    let query = tx
        .query(
            "SELECT host_id, host_template_id FROM host_templates WHERE host_id = ANY($1) AND template_id IS NULL",
            &[&host_ids],
        )
        .await
        .ctx("query base host_templates")?;

    let mut base_host_template_map: HashMap<Uuid, Uuid> = Default::default(); // host_id -> base_host_template_id
    for row in query {
        let host_id: Uuid = row.try_get(0).ctx("get host_id from base host_templates")?;

        let base_host_template_id: Uuid = row
            .try_get(1)
            .ctx("get host_template_id from base host_templates")?;

        base_host_template_map.insert(host_id, base_host_template_id);
    }

    // Execute unapply operations for each entry in the payload
    // Consider batching if needed
    for unapply_entry in &payload.unapply_list {
        let host_id = unapply_entry.host_id;
        let template_id = unapply_entry.template_id;
        let host_template_id = host_template_map
            .get(&(host_id, template_id))
            .ok_or_else(|| {
                ResponseError::from_status(StatusCode::INTERNAL_SERVER_ERROR)
                    .with_title("Host template not found")
                    .with_detail(format!(
                        "Host template entry not found for host_id: {} and template_id: {}",
                        host_id, template_id
                    ))
            })?;

        let base_host_template_id = base_host_template_map.get(&host_id).ok_or_else(|| {
            ResponseError::from_status(StatusCode::INTERNAL_SERVER_ERROR)
                .with_title("Base host_template not found")
                .with_detail(format!(
                    "Base host_template entry not found for host_id: {}",
                    host_id
                ))
        })?;

        if unapply_entry.keep_items {
            // Resolve items on the host that were inherited from the template
            // and reassign them to the host's own base host_template_id.
            let statement = r#"
                WITH RECURSIVE
                t_items AS (
                    SELECT i.item_id FROM items i
                    JOIN host_templates ht ON i.host_template_id = ht.host_template_id
                    WHERE ht.template_id = $1::uuid AND ht.host_id IS NULL
                ),
                inherited_items AS (
                    SELECT i.item_id, i.template_item_id, i.name, i.schedule, i.timeout_ms, i.config, i.labels
                    FROM items i
                    WHERE i.host_template_id = $2::uuid
                    AND i.template_item_id IN (SELECT item_id FROM t_items)
                ),
                resolution AS (
                    SELECT
                        ii.item_id,
                        i.template_item_id,
                        COALESCE(ii.name, i.name) AS resolved_name,
                        COALESCE(ii.schedule, i.schedule) AS resolved_schedule,
                        COALESCE(ii.timeout_ms, i.timeout_ms) AS resolved_timeout_ms,
                        COALESCE(ii.config, i.config) AS resolved_config,
                        COALESCE(ii.labels, i.labels) AS resolved_labels,
                        0 AS depth
                    FROM inherited_items ii
                    JOIN items i ON i.item_id = ii.template_item_id

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
                    host_template_id = $3::uuid,
                    template_item_id = NULL,
                    name = fr.resolved_name,
                    schedule = fr.resolved_schedule,
                    timeout_ms = fr.resolved_timeout_ms,
                    config = fr.resolved_config,
                    labels = fr.resolved_labels
                FROM final_resolved fr
                WHERE items.item_id = fr.item_id
            "#;
            tx.execute(
                statement,
                &[&template_id, &host_template_id, &base_host_template_id],
            )
            .await
            .ctx("resolve inherited items")?;
        } else {
            // Delete all items on the host that were inherited from the template.
            let statement = r#"
                DELETE FROM items i
                WHERE i.host_template_id = $1::uuid;
            "#;

            tx.execute(statement, &[&host_template_id])
                .await
                .ctx("delete inherited items")?;
        }
    }

    // Delete all host_template entries for the specified host-template pairs
    let host_template_ids: Vec<Uuid> = host_template_map.values().cloned().collect();
    tx.execute(
        "DELETE FROM host_templates WHERE host_template_id = ANY($1)",
        &[&host_template_ids],
    )
    .await
    .ctx("delete host_template entries")?;

    tx.commit().await.ctx("commit transaction")?;

    Ok(ResponseData::from_status(StatusCode::OK))
}
