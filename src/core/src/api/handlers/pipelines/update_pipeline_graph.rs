// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::handlers::pipelines::get_pipeline_graph::GetPipelineGraphResponse;
use crate::api::handlers::pipelines::graph::{GraphNodeKind, PipelineGraphInput};
use crate::api::handlers::pipelines::runtime::{
    delete_processors, insert_hidden_processors, load_graph, sync_runtime,
};
use crate::api::handlers::pipelines::{find_canvas, find_pipeline, fmt_ids, missing_ids, now};
use crate::api::jwt::AuthUser;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::{
    AdmStatus, PipelineCanvases, PipelineNodeProcessorGroups, PipelineNodeProcessors,
    PipelineNodeRelations, PipelineNodeSources, PipelineNodes,
};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use chrono::{DateTime, Utc};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePipelineGraphRequest {
    /// The `last_updated` of the canvas when it was read.
    pub last_updated: DateTime<Utc>,
    pub graph: PipelineGraphInput,
}

/// A node as it is stored now.
struct CurrentNode {
    kind: GraphNodeKind,
    /// The hidden source and gate processors of a source node.
    hidden_processors: Option<(Uuid, Uuid)>,
}

/// Save the graph of a canvas
///
/// Saves the structure only: nodes with their names and positions, and
/// relations. A new node starts empty and its content is saved per node. A
/// node left out of the graph is deleted with its content.
#[utoipa::path(
    put,
    path = "/pipelines/{pipeline_id}/canvases/{canvas_id}/graph",
    params(
        ("pipeline_id" = Uuid, Path, description = "Pipeline UUID"),
        ("canvas_id" = Uuid, Path, description = "Canvas UUID")
    ),
    request_body(
        content = UpdatePipelineGraphRequest,
        description = "Save graph request body",
        content_type = "application/json",
    ),
    responses(
        (status = 200, description = "Saved graph", body = ResponseData<GetPipelineGraphResponse>),
        (status = 400, description = "Validation error", body = ResponseError),
        (status = 404, description = "Pipeline or canvas not found, or pipeline not visible to the caller", body = ResponseError),
        (status = 409, description = "The canvas was saved after the caller read it", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "pipelines",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn update_pipeline_graph(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path((pipeline_id, canvas_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<UpdatePipelineGraphRequest>,
) -> Result<ResponseData<GetPipelineGraphResponse>, ResponseError> {
    // validate graph
    let graph = payload.graph;
    graph.validate().bad_request("Validation error")?;

    let mut client = state.db_pool.get().await.ctx("get database connection")?;
    let tx = client.transaction().await.ctx("start transaction")?;

    // lock pipeline, check it owns the canvas and nobody saved it since it was read
    let pipeline = find_pipeline(&tx, pipeline_id, auth_user.user_id, true).await?;
    let last_updated = find_canvas(&tx, pipeline_id, canvas_id).await?;
    if last_updated != payload.last_updated {
        return Err(ResponseError::from_status(StatusCode::CONFLICT)
            .with_title("Canvas changed")
            .with_detail("The canvas was saved after it was read. Read it again and retry."));
    }

    // get current nodes, with the hidden processors of the source node
    let (sql, values) = DbQuery::select()
        .columns([
            (PipelineNodes::Table, PipelineNodes::NodeId),
            (PipelineNodes::Table, PipelineNodes::Kind),
        ])
        .columns([
            (
                PipelineNodeSources::Table,
                PipelineNodeSources::SourceProcessorId,
            ),
            (
                PipelineNodeSources::Table,
                PipelineNodeSources::GateProcessorId,
            ),
        ])
        .from(PipelineNodes::Table)
        .left_join(
            PipelineNodeSources::Table,
            Expr::col((PipelineNodeSources::Table, PipelineNodeSources::NodeId))
                .equals((PipelineNodes::Table, PipelineNodes::NodeId)),
        )
        .and_where(Expr::col((PipelineNodes::Table, PipelineNodes::CanvasId)).eq(canvas_id))
        .build_postgres(PostgresQueryBuilder);
    let mut current = HashMap::new();
    for row in tx
        .query(sql.as_str(), &values.as_params())
        .await
        .ctx("query nodes")?
    {
        let kind: String = row.get(1);
        let source: Option<Uuid> = row.get(2);
        let gate: Option<Uuid> = row.get(3);
        current.insert(
            row.get::<_, Uuid>(0),
            CurrentNode {
                kind: kind.parse().ctx("parse node kind")?,
                hidden_processors: source.zip(gate),
            },
        );
    }

    // check kept nodes keep their kind
    for node in &graph.nodes {
        if let Some(current) = current.get(&node.node_id)
            && current.kind != node.kind
        {
            let (from, to): (&str, &str) = (current.kind.into(), node.kind.into());
            return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
                .with_title("Validation error")
                .with_detail(format!(
                    "Node {} is a {from} node and cannot become a {to} node",
                    node.node_id
                )));
        }
    }

    // check new node ids are not used by another canvas
    let new_ids: BTreeSet<Uuid> = graph
        .nodes
        .iter()
        .map(|node| node.node_id)
        .filter(|id| !current.contains_key(id))
        .collect();
    let free: BTreeSet<Uuid> =
        missing_ids(&tx, PipelineNodes::Table, PipelineNodes::NodeId, &new_ids)
            .await
            .ctx("check node ids")?
            .into_iter()
            .collect();
    let used: Vec<Uuid> = new_ids.difference(&free).copied().collect();
    if !used.is_empty() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Validation error")
            .with_detail(format!("Node IDs already used: {}", fmt_ids(&used))));
    }

    // delete removed nodes, then the processors they owned
    let kept: HashSet<Uuid> = graph.nodes.iter().map(|node| node.node_id).collect();
    let removed: Vec<Uuid> = current
        .keys()
        .filter(|id| !kept.contains(id))
        .copied()
        .collect();
    if !removed.is_empty() {
        let mut processor_ids: Vec<Uuid> = removed
            .iter()
            .filter_map(|id| current[id].hidden_processors)
            .flat_map(|(source, gate)| [source, gate])
            .collect();
        let (sql, values) = DbQuery::select()
            .column(PipelineNodeProcessors::ProcessorId)
            .from(PipelineNodeProcessors::Table)
            .and_where(Expr::col(PipelineNodeProcessors::NodeId).is_in(removed.iter().copied()))
            .build_postgres(PostgresQueryBuilder);
        for row in tx
            .query(sql.as_str(), &values.as_params())
            .await
            .ctx("query processors of removed nodes")?
        {
            processor_ids.push(row.get(0));
        }

        let (sql, values) = DbQuery::delete()
            .from_table(PipelineNodes::Table)
            .and_where(Expr::col(PipelineNodes::NodeId).is_in(removed.iter().copied()))
            .build_postgres(PostgresQueryBuilder);
        tx.execute(sql.as_str(), &values.as_params())
            .await
            .ctx("delete nodes")?;

        delete_processors(&tx, &processor_ids)
            .await
            .ctx("delete processors")?;
    }

    // insert new nodes
    let now = now();
    let new_nodes: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| new_ids.contains(&node.node_id))
        .collect();
    if !new_nodes.is_empty() {
        let mut query = DbQuery::insert();
        query.into_table(PipelineNodes::Table).columns([
            PipelineNodes::NodeId,
            PipelineNodes::CanvasId,
            PipelineNodes::Kind,
            PipelineNodes::PositionX,
            PipelineNodes::PositionY,
            PipelineNodes::LastUpdated,
        ]);
        for node in &new_nodes {
            query
                .values([
                    node.node_id.into(),
                    canvas_id.into(),
                    <&str>::from(node.kind).into(),
                    node.position_x.into(),
                    node.position_y.into(),
                    now.into(),
                ])
                .ctx("build insert nodes query")?;
        }
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        tx.execute(sql.as_str(), &values.as_params())
            .await
            .ctx("insert nodes")?;
    }

    // insert the kind row of each new node, a source node with its hidden processors
    let mut group_ids = Vec::new();
    for node in &new_nodes {
        match node.kind {
            GraphNodeKind::Source => {
                let (source, gate) = (Uuid::now_v7(), Uuid::now_v7());
                insert_hidden_processors(&tx, &pipeline.name, source, gate, now)
                    .await
                    .ctx("insert hidden processors")?;
                let (sql, values) = DbQuery::insert()
                    .into_table(PipelineNodeSources::Table)
                    .columns([
                        PipelineNodeSources::NodeId,
                        PipelineNodeSources::SourceProcessorId,
                        PipelineNodeSources::GateProcessorId,
                    ])
                    .values([node.node_id.into(), source.into(), gate.into()])
                    .ctx("build insert source query")?
                    .build_postgres(PostgresQueryBuilder);
                tx.execute(sql.as_str(), &values.as_params())
                    .await
                    .ctx("insert source")?;
            }
            GraphNodeKind::ProcessorGroup => group_ids.push(node.node_id),
            // a sink node starts empty, which is no row
            GraphNodeKind::Sink => {}
        }
    }
    if !group_ids.is_empty() {
        let mut query = DbQuery::insert();
        query
            .into_table(PipelineNodeProcessorGroups::Table)
            .columns([PipelineNodeProcessorGroups::NodeId]);
        for id in group_ids {
            query
                .values([id.into()])
                .ctx("build insert processor groups query")?;
        }
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        tx.execute(sql.as_str(), &values.as_params())
            .await
            .ctx("insert processor groups")?;
    }

    // update kept nodes; their content and last_updated stay
    for node in graph
        .nodes
        .iter()
        .filter(|node| !new_ids.contains(&node.node_id))
    {
        let (sql, values) = DbQuery::update()
            .table(PipelineNodes::Table)
            .values([
                (PipelineNodes::PositionX, node.position_x.into()),
                (PipelineNodes::PositionY, node.position_y.into()),
            ])
            .and_where(Expr::col(PipelineNodes::NodeId).eq(node.node_id))
            .build_postgres(PostgresQueryBuilder);
        tx.execute(sql.as_str(), &values.as_params())
            .await
            .ctx("update node")?;
    }

    // replace relations
    let (sql, values) = DbQuery::delete()
        .from_table(PipelineNodeRelations::Table)
        .and_where(Expr::col(PipelineNodeRelations::CanvasId).eq(canvas_id))
        .build_postgres(PostgresQueryBuilder);
    tx.execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete relations")?;
    if !graph.relations.is_empty() {
        let mut query = DbQuery::insert();
        query.into_table(PipelineNodeRelations::Table).columns([
            PipelineNodeRelations::CanvasId,
            PipelineNodeRelations::FromNodeId,
            PipelineNodeRelations::ToNodeId,
        ]);
        for relation in &graph.relations {
            query
                .values([
                    canvas_id.into(),
                    relation.from_node_id.into(),
                    relation.to_node_id.into(),
                ])
                .ctx("build insert relations query")?;
        }
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        tx.execute(sql.as_str(), &values.as_params())
            .await
            .ctx("insert relations")?;
    }

    // rewire the runtime rows
    sync_runtime(
        &tx,
        canvas_id,
        matches!(pipeline.adm_status, AdmStatus::Enabled),
    )
    .await
    .ctx("sync runtime")?
    .bad_request("Validation error")?;

    // bump canvas last_updated
    let (sql, values) = DbQuery::update()
        .table(PipelineCanvases::Table)
        .value(PipelineCanvases::LastUpdated, now)
        .and_where(Expr::col(PipelineCanvases::CanvasId).eq(canvas_id))
        .build_postgres(PostgresQueryBuilder);
    tx.execute(sql.as_str(), &values.as_params())
        .await
        .ctx("update canvas")?;

    let graph = load_graph(&tx, canvas_id).await.ctx("load graph")?;
    tx.commit().await.ctx("commit transaction")?;

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(GetPipelineGraphResponse {
            pipeline_id,
            canvas_id,
            last_updated: now,
            graph,
        }),
    )
}
