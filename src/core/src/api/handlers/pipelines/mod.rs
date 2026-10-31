// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod create_pipeline;
pub mod delete_pipeline;
pub mod get_pipeline;
pub mod get_pipeline_graph;
pub mod get_pipeline_node;
mod graph;
pub mod list_pipelines;
mod runtime;
pub mod update_pipeline;
pub mod update_pipeline_graph;
pub mod update_pipeline_node;

pub use create_pipeline::create_pipeline;
pub use delete_pipeline::delete_pipeline;
pub use get_pipeline::get_pipeline;
pub use get_pipeline_graph::get_pipeline_graph;
pub use get_pipeline_node::get_pipeline_node;
pub use list_pipelines::{PipelineOrderBy, list_pipelines};
pub use update_pipeline::update_pipeline;
pub use update_pipeline_graph::update_pipeline_graph;
pub use update_pipeline_node::update_pipeline_node;

use crate::api::error::ApiResultExt;
use crate::api::response::ResponseError;
use crate::db::models::serialize_utc_as_local;
use crate::db::schema::{
    AdmStatus, PipelineCanvases, PipelineHosts, PipelineSinks, Pipelines, Visibility,
};
use axum::http::StatusCode;
use chrono::{DateTime, SubsecRound, Utc};
use deadpool_postgres::GenericClient;
use sea_query::{
    Cond, Expr, ExprTrait, IntoColumnRef, IntoTableRef, LockType, Order, PostgresQueryBuilder,
    Query as DbQuery,
};
use sea_query_postgres::PostgresBinder;
use serde::Serialize;
use std::collections::{BTreeSet, HashSet};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

const PIPELINE_COLUMNS: [Pipelines; 7] = [
    Pipelines::PipelineId,
    Pipelines::Name,
    Pipelines::Visibility,
    Pipelines::OwnerUserId,
    Pipelines::AdmStatus,
    Pipelines::CreatedAt,
    Pipelines::LastUpdated,
];

#[derive(OpenApi)]
#[openapi(
    paths(
        list_pipelines::list_pipelines,
        create_pipeline::create_pipeline,
        get_pipeline::get_pipeline,
        delete_pipeline::delete_pipeline,
        update_pipeline::update_pipeline,
        get_pipeline_graph::get_pipeline_graph,
        update_pipeline_graph::update_pipeline_graph,
        get_pipeline_node::get_pipeline_node,
        update_pipeline_node::update_pipeline_node,
    ),
    components(schemas(
        PipelineOrderBy,
        update_pipeline::PipelineSubsetInUse,
        graph::PipelineGraph,
        graph::GraphNode,
        graph::GraphNodeData,
        graph::GraphSourceNode,
        graph::GraphProcessorGroupNode,
        graph::GraphProcessorSummary,
        graph::GraphSinkNode,
        graph::GraphRelation,
        graph::PipelineGraphInput,
        graph::GraphNodeInput,
        graph::GraphNodeKind,
        graph::GraphNodeContent,
        graph::GraphSourceContent,
        graph::GraphSourceItem,
        graph::GraphProcessorGroupContent,
        graph::GraphProcessor,
        graph::GraphSinkContent,
        graph::GraphNodeContentInput,
        graph::GraphSourceContentInput,
        graph::GraphProcessorGroupContentInput,
        graph::GraphProcessorInput,
        graph::GraphSinkContentInput,
    ))
)]
pub struct PipelinesHandler;

#[derive(Debug, Clone)]
pub struct ManagedPipeline {
    pub pipeline_id: Uuid,
    pub name: String,
    pub visibility: Visibility,
    pub owner_user_id: Option<Uuid>,
    pub adm_status: AdmStatus,
    pub created_at: DateTime<Utc>,
    pub last_updated: DateTime<Utc>,
}

fn row_to_pipeline(row: &tokio_postgres::Row) -> anyhow::Result<ManagedPipeline> {
    let visibility: i16 = row.try_get("visibility")?;
    let adm_status: i16 = row.try_get("adm_status")?;

    Ok(ManagedPipeline {
        pipeline_id: row.try_get("pipeline_id")?,
        name: row.try_get("name")?,
        visibility: visibility.try_into()?,
        owner_user_id: row.try_get("owner_user_id")?,
        adm_status: adm_status.try_into()?,
        created_at: row.try_get("created_at")?,
        last_updated: row.try_get("last_updated")?,
    })
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PipelineCanvas {
    pub canvas_id: Uuid,
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PipelineDetail {
    pub pipeline_id: Uuid,
    pub name: String,
    #[schema(value_type = i16)]
    pub visibility: Visibility,
    /// Null when the owner was deleted. Such a pipeline is always public.
    pub owner_user_id: Option<Uuid>,
    #[schema(value_type = i16)]
    pub adm_status: AdmStatus,
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub created_at: DateTime<Utc>,
    #[serde(serialize_with = "serialize_utc_as_local")]
    pub last_updated: DateTime<Utc>,
    pub host_ids: Vec<Uuid>,
    pub sink_ids: Vec<Uuid>,
    pub canvases: Vec<PipelineCanvas>,
}

/// Public pipelines, plus the private ones `user_id` owns.
fn visible_to(user_id: Uuid) -> Cond {
    Cond::any()
        .add(Expr::col(Pipelines::Visibility).eq(Visibility::Public as i16))
        .add(Expr::col(Pipelines::OwnerUserId).eq(user_id))
}

/// Loads a pipeline `user_id` can see, optionally locking its row until the
/// transaction ends.
async fn find_pipeline(
    client: &impl GenericClient,
    pipeline_id: Uuid,
    user_id: Uuid,
    lock: bool,
) -> Result<ManagedPipeline, ResponseError> {
    let mut query = DbQuery::select();
    query
        .columns(PIPELINE_COLUMNS)
        .from(Pipelines::Table)
        .and_where(Expr::col(Pipelines::PipelineId).eq(pipeline_id))
        .cond_where(visible_to(user_id));
    if lock {
        query.lock(LockType::Update);
    }
    let (sql, values) = query.build_postgres(PostgresQueryBuilder);

    let row = client
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("query pipeline")?
        .ok_or_else(|| not_found(pipeline_id))?;
    row_to_pipeline(&row).ctx("parse pipeline")
}

/// Adds the host and sink subsets and the canvases to a pipeline.
async fn load_detail(
    client: &impl GenericClient,
    pipeline: ManagedPipeline,
) -> Result<PipelineDetail, ResponseError> {
    let id = pipeline.pipeline_id;

    // load host subset
    let (sql, values) = DbQuery::select()
        .column(PipelineHosts::HostId)
        .from(PipelineHosts::Table)
        .and_where(Expr::col(PipelineHosts::PipelineId).eq(id))
        .order_by(PipelineHosts::HostId, Order::Asc)
        .build_postgres(PostgresQueryBuilder);
    let host_ids = client
        .query(sql.as_str(), &values.as_params())
        .await
        .ctx("query pipeline hosts")?
        .iter()
        .map(|row| row.get(0))
        .collect();

    // load sink subset
    let (sql, values) = DbQuery::select()
        .column(PipelineSinks::SinkId)
        .from(PipelineSinks::Table)
        .and_where(Expr::col(PipelineSinks::PipelineId).eq(id))
        .order_by(PipelineSinks::SinkId, Order::Asc)
        .build_postgres(PostgresQueryBuilder);
    let sink_ids = client
        .query(sql.as_str(), &values.as_params())
        .await
        .ctx("query pipeline sinks")?
        .iter()
        .map(|row| row.get(0))
        .collect();

    // load canvases
    let (sql, values) = DbQuery::select()
        .columns([PipelineCanvases::CanvasId, PipelineCanvases::LastUpdated])
        .from(PipelineCanvases::Table)
        .and_where(Expr::col(PipelineCanvases::PipelineId).eq(id))
        .order_by(PipelineCanvases::CanvasId, Order::Asc)
        .build_postgres(PostgresQueryBuilder);
    let canvases = client
        .query(sql.as_str(), &values.as_params())
        .await
        .ctx("query pipeline canvases")?
        .iter()
        .map(|row| PipelineCanvas {
            canvas_id: row.get(0),
            last_updated: row.get(1),
        })
        .collect();

    Ok(PipelineDetail {
        pipeline_id: id,
        name: pipeline.name,
        visibility: pipeline.visibility,
        owner_user_id: pipeline.owner_user_id,
        adm_status: pipeline.adm_status,
        created_at: pipeline.created_at,
        last_updated: pipeline.last_updated,
        host_ids,
        sink_ids,
        canvases,
    })
}

/// Returns the `last_updated` of a canvas, if the pipeline owns it.
async fn find_canvas(
    client: &impl GenericClient,
    pipeline_id: Uuid,
    canvas_id: Uuid,
) -> Result<DateTime<Utc>, ResponseError> {
    let (sql, values) = DbQuery::select()
        .column(PipelineCanvases::LastUpdated)
        .from(PipelineCanvases::Table)
        .and_where(Expr::col(PipelineCanvases::CanvasId).eq(canvas_id))
        .and_where(Expr::col(PipelineCanvases::PipelineId).eq(pipeline_id))
        .build_postgres(PostgresQueryBuilder);
    Ok(client
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("query canvas")?
        .ok_or_else(|| canvas_not_found(canvas_id))?
        .get(0))
}

/// Returns the ids in `ids` that have no row in `table`.
async fn missing_ids(
    client: &impl GenericClient,
    table: impl IntoTableRef,
    column: impl IntoColumnRef,
    ids: &BTreeSet<Uuid>,
) -> Result<Vec<Uuid>, tokio_postgres::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let column = Expr::col(column);
    let (sql, values) = DbQuery::select()
        .expr(column.clone())
        .from(table)
        .and_where(column.is_in(ids.iter().copied()))
        .build_postgres(PostgresQueryBuilder);
    let found: HashSet<Uuid> = client
        .query(sql.as_str(), &values.as_params())
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();

    Ok(ids
        .iter()
        .filter(|id| !found.contains(id))
        .copied()
        .collect())
}

/// Formats up to 3 ids, then "and N more".
fn fmt_ids(ids: &[Uuid]) -> String {
    let n_ids = 3;
    let shown = ids
        .iter()
        .take(n_ids)
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    if ids.len() > n_ids {
        format!("{} and {} more", shown, ids.len() - n_ids)
    } else {
        shown
    }
}

/// Postgres keeps microseconds. Dropping the rest up front means the
/// `last_updated` a client gets back compares equal to the stored one.
fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

fn not_found(pipeline_id: Uuid) -> ResponseError {
    ResponseError::from_status(StatusCode::NOT_FOUND)
        .with_title("Pipeline not found")
        .with_detail(format!(
            "No pipeline found for pipeline_id: {}",
            pipeline_id
        ))
}

fn canvas_not_found(canvas_id: Uuid) -> ResponseError {
    ResponseError::from_status(StatusCode::NOT_FOUND)
        .with_title("Canvas not found")
        .with_detail(format!("No canvas found for canvas_id: {}", canvas_id))
}

fn node_not_found(node_id: Uuid) -> ResponseError {
    ResponseError::from_status(StatusCode::NOT_FOUND)
        .with_title("Node not found")
        .with_detail(format!("No node found for node_id: {}", node_id))
}
