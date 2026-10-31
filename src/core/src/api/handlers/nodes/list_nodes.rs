// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::config::cluster::NodeMetadata;
use crate::db::models::serialize_utc_as_local;
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Inactive,     // 0
    Active,       // 1
    Initializing, // 2
    // 3, synthesized for cluster_standby_nodes rows, never stored in cluster_nodes
    Standby,
    Unknown, // else, just in case
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ResponseClusterNode {
    pub node_id: Uuid,
    pub name: String,
    pub op_status: NodeStatus,
    pub is_leader: bool,
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub last_heartbeat: DateTime<Utc>,
    pub metadata: NodeMetadata,
    pub host_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListNodesResponse {
    pub nodes: Vec<ResponseClusterNode>,
}

/// List all cluster nodes
///
/// Returns information about every node registered in the cluster
#[utoipa::path(
    get,
    path = "/nodes",
    responses(
        (status = 200, description = "List of cluster nodes", body = ResponseData<ListNodesResponse>),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "nodes",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn list_nodes(
    State(state): State<Arc<AppState>>,
) -> Result<ResponseData<ListNodesResponse>, ResponseError> {
    let client = state.db_pool.get().await.ctx("get database connection")?;

    let rows = client
        .query(
            r#"
            SELECT n.node_id, n.name, n.op_status, n.is_leader, n.last_heartbeat, n.metadata,
                COUNT(h.host_id) AS host_count
            FROM cluster_nodes n
            LEFT JOIN hosts h ON h.monitored_by = n.node_id
            GROUP BY n.node_id, n.name, n.op_status, n.is_leader, n.last_heartbeat, n.metadata
            UNION ALL
            -- standby nodes share their main's node_id, so a row is keyed by
            -- (node_id, name); they never lead and never own hosts
            SELECT s.node_id, s.name, 3::SMALLINT, false, s.last_heartbeat, s.metadata, 0::BIGINT
            FROM cluster_standby_nodes s
            "#,
            &[],
        )
        .await
        .ctx("query cluster_nodes")?;

    let nodes: Result<Vec<ResponseClusterNode>, Box<dyn std::error::Error>> = rows
        .iter()
        .map(|row| {
            let status = match row.try_get::<_, i16>(2)? {
                0 => NodeStatus::Inactive,
                1 => NodeStatus::Active,
                2 => NodeStatus::Initializing,
                3 => NodeStatus::Standby,
                _ => NodeStatus::Unknown,
            };

            let metadata: serde_json::Value = row.try_get(5)?;
            let metadata: NodeMetadata = serde_json::from_value(metadata)?;

            Ok(ResponseClusterNode {
                node_id: row.try_get(0)?,
                name: row.try_get(1)?,
                op_status: status,
                is_leader: row.try_get(3)?,
                last_heartbeat: row.try_get(4)?,
                metadata,
                host_count: row.try_get(6)?,
            })
        })
        .collect();

    let nodes = nodes.ctx("parse cluster node rows")?;
    Ok(ResponseData::from_status(StatusCode::OK).with_data(ListNodesResponse { nodes }))
}
