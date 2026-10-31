// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::config::item::{Config, Item, ItemKind, Schedule};
use crate::db::schema::{AdmStatus, HostTemplates, Items, default_adm_status};
use async_recursion::async_recursion;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use deadpool_postgres::Transaction;
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use smol_str::ToSmolStr;
use std::collections::HashSet;
use std::str::FromStr;
use std::time::Duration;
use std::{collections::HashMap, sync::Arc};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateTemplateItemRequest {
    #[schema(default = 0)]
    pub flags: i32,
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    pub name: String,
    pub schedule: String,
    #[schema(default = 0)]
    pub timeout_ms: i32,
    pub config: Config,
    pub labels: serde_json::Value,
}

impl CreateTemplateItemRequest {
    pub fn validate(&self) -> anyhow::Result<()> {
        let mut item = Item::new(
            Uuid::now_v7(), // placeholder
            self.name.to_smolstr(),
            None,
            Uuid::now_v7(), // placeholder
            Schedule::from_str(&self.schedule)?,
            None,
            Duration::from_millis(self.timeout_ms as u64),
            vec![],
            self.config.clone(),
        );
        item.validate()?;

        if self.name.trim().is_empty() {
            anyhow::bail!("Item name cannot be empty");
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateTemplateItemResponse {
    pub item_id: Uuid,
    #[schema(default = 0)]
    pub flags: i32,
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    pub name: String,
    pub schedule: String,
    #[schema(default = 0)]
    pub timeout_ms: i32,
    pub config: Config,
    pub labels: serde_json::Value,
}

/// Create an item for a template
#[utoipa::path(
    post,
    path = "/items/template/{template_id}",
     params(
        ("template_id" = Uuid, Path, description = "Template UUID")
    ),
    request_body = CreateTemplateItemRequest,
    responses(
        (status = 201, description = "Item created", body = ResponseData<CreateTemplateItemResponse>),
        (status = 404, description = "Host template not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "items",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn create_template_item(
    State(state): State<Arc<AppState>>,
    Path(template_id): Path<Uuid>,
    Json(payload): Json<CreateTemplateItemRequest>,
) -> Result<ResponseData<CreateTemplateItemResponse>, ResponseError> {
    payload.validate().bad_request("Validation error")?;

    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // get host_templates for the template
    let mut query = DbQuery::select();
    query
        .column((HostTemplates::Table, HostTemplates::HostTemplateId))
        .from(HostTemplates::Table)
        .and_where(
            Expr::col(HostTemplates::TemplateId)
                .eq(template_id)
                .and(Expr::col(HostTemplates::HostId).is_null()),
        );

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    let row_opt = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    let Some(row) = row_opt else {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Host template not found")
            .with_detail(format!(
                "No host template found for template_id: {}",
                template_id
            )));
    };
    let curr_template_host_template_id = row.get::<_, Uuid>("host_template_id");

    // build adjacency list of host_template_id starting from the current template's host_template_id
    let statement = "
        WITH RECURSIVE root_template AS (
            SELECT template_id
            FROM host_templates
            WHERE host_template_id = $1::uuid
        ),
        template_ids AS (
            SELECT template_id FROM root_template
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
        all_nodes AS (
            SELECT host_template_id FROM template_nodes
            UNION
            SELECT child_id FROM edges
        )
        SELECT
            n.host_template_id AS curr_host_template_id,
            COALESCE(
                array_agg(DISTINCT e.child_id ORDER BY e.child_id)
                    FILTER (WHERE e.child_id IS NOT NULL),
                ARRAY[]::uuid[]
            ) AS children_host_template_ids
        FROM all_nodes n
        LEFT JOIN edges e
        ON e.parent_id = n.host_template_id
        GROUP BY n.host_template_id
        ORDER BY n.host_template_id;
    ";

    let rows = tx
        .query(statement, &[&curr_template_host_template_id])
        .await
        .ctx("execute recursive template query")?;

    let adj: HashMap<Uuid, Vec<Uuid>> = rows
        .iter()
        .map(|row| {
            let host_template_id = row.get::<_, Uuid>("curr_host_template_id");
            let children_host_template_ids = row
                .get::<_, Vec<Option<Uuid>>>("children_host_template_ids")
                .into_iter()
                .flatten()
                .collect();
            (host_template_id, children_host_template_ids)
        })
        .collect();

    // craete item for current template
    let base_item_id = Uuid::now_v7();
    let now = chrono::Utc::now();
    let (sql, values) = DbQuery::insert()
        .into_table(Items::Table)
        .columns([
            Items::ItemId,
            Items::HostTemplateId,
            Items::Type,
            Items::Flags,
            Items::AdmStatus,
            Items::Name,
            Items::Schedule,
            Items::TimeoutMs,
            Items::LastUpdated,
            Items::Config,
            Items::Labels,
        ])
        .values([
            base_item_id.into(),
            curr_template_host_template_id.into(),
            ItemKind::from(&payload.config).to_string().into(),
            payload.flags.into(),
            (payload.adm_status as i16).into(),
            payload.name.clone().into(),
            payload.schedule.clone().into(),
            payload.timeout_ms.into(),
            now.into(),
            payload
                .config
                .to_json_value()
                .ctx("serialize config")?
                .into(),
            payload.labels.clone().into(),
        ])
        .ctx("build insert query")?
        .build_postgres(PostgresQueryBuilder);

    _ = tx
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("execute query")?;

    // recursively create items for linked templates and hosts
    create_link_item(
        &tx,
        &payload,
        base_item_id,
        curr_template_host_template_id,
        &adj,
        &mut HashSet::new(),
    )
    .await?;

    tx.commit().await.ctx("commit transaction")?;

    let response = CreateTemplateItemResponse {
        item_id: base_item_id,
        flags: payload.flags,
        adm_status: payload.adm_status,
        name: payload.name,
        schedule: payload.schedule,
        timeout_ms: payload.timeout_ms,
        config: payload.config.clone(),
        labels: payload.labels,
    };

    Ok(ResponseData::from_status(StatusCode::CREATED).with_data(response))
}

#[async_recursion]
async fn create_link_item(
    tx: &Transaction<'_>,
    payload: &CreateTemplateItemRequest,
    parent_item_id: Uuid,
    parent_host_template_id: Uuid,
    adj: &HashMap<Uuid, Vec<Uuid>>,
    visited: &mut HashSet<Uuid>,
) -> Result<(), ResponseError> {
    if let Some(children_host_template_ids) = adj.get(&parent_host_template_id) {
        for child_host_template_id in children_host_template_ids {
            // prevent infinite recursion in case of cycles in template links
            // which should never happen in the first place
            if visited.contains(child_host_template_id) {
                return Err(
                    ResponseError::from_status(StatusCode::INTERNAL_SERVER_ERROR)
                        .with_title("Detected cycle in template links"),
                );
            }
            visited.insert(*child_host_template_id);

            let new_item_id = Uuid::now_v7();
            let now = chrono::Utc::now();
            let (sql, values) = DbQuery::insert()
                .into_table(Items::Table)
                .columns([
                    Items::ItemId,
                    Items::HostTemplateId,
                    Items::TemplateItemId,
                    Items::Type,
                    Items::Flags,
                    Items::AdmStatus,
                    Items::LastUpdated,
                ])
                .values([
                    new_item_id.into(),
                    (*child_host_template_id).into(),
                    parent_item_id.into(),
                    ItemKind::from(&payload.config).to_string().into(),
                    payload.flags.into(),
                    (payload.adm_status as i16).into(),
                    now.into(),
                ])
                .ctx("build insert query for linked template")?
                .build_postgres(PostgresQueryBuilder);

            _ = tx
                .execute(sql.as_str(), &values.as_params())
                .await
                .ctx("execute query for linked template")?;

            create_link_item(
                tx,
                payload,
                new_item_id,
                *child_host_template_id,
                adj,
                visited,
            )
            .await?;
        }
    }

    return Ok(());
}
