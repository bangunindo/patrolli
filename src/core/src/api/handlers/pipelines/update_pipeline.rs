// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::handlers::pipelines::runtime::{rename_hidden_processors, sync_runtime};
use crate::api::handlers::pipelines::{
    PipelineDetail, find_pipeline, fmt_ids, load_detail, missing_ids, now,
};
use crate::api::jwt::AuthUser;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::{
    AdmStatus, HostTemplates, Hosts, ItemProcessors, Items, PipelineCanvases, PipelineHosts,
    PipelineNodeSinks, PipelineNodeSourceItems, PipelineNodeSources, PipelineNodes, PipelineSinks,
    Pipelines, Sinks, Visibility,
};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use chrono::{DateTime, Utc};
use deadpool_postgres::GenericClient;
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// How many item ids a 409 lists at most.
const LISTED_ITEMS: usize = 100;

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePipelineRequest {
    /// The `last_updated` of the pipeline when it was read.
    pub last_updated: DateTime<Utc>,
    pub name: String,
    /// Only the owner can change it.
    #[schema(value_type = i16)]
    pub visibility: Visibility,
    #[schema(value_type = i16)]
    pub adm_status: AdmStatus,
    pub host_ids: BTreeSet<Uuid>,
    pub sink_ids: BTreeSet<Uuid>,
    /// Take items of a dropped host off their sources and empty the sink
    /// nodes of a dropped sink, instead of answering 409.
    #[serde(default)]
    pub force: bool,
}

/// What still uses the hosts and sinks a request drops.
#[derive(Debug, Serialize, ToSchema)]
pub struct PipelineSubsetInUse {
    /// Source nodes holding items of dropped hosts, and sink nodes of dropped sinks.
    pub node_ids: Vec<Uuid>,
    /// The first items of dropped hosts, at most 100.
    pub item_ids: Vec<Uuid>,
    pub item_count: usize,
}

/// Update a pipeline
///
/// Replaces the name, visibility, status and the host and sink subsets.
/// Disabling a pipeline stops what its sources receive from going further,
/// without sending it anywhere else.
#[utoipa::path(
    put,
    path = "/pipelines/{pipeline_id}",
    params(
        ("pipeline_id" = Uuid, Path, description = "Pipeline UUID")
    ),
    request_body(
        content = UpdatePipelineRequest,
        description = "Update pipeline request body",
        content_type = "application/json",
    ),
    responses(
        (status = 200, description = "Pipeline updated", body = ResponseData<PipelineDetail>),
        (status = 400, description = "Validation error", body = ResponseError),
        (status = 403, description = "Only the owner can change the visibility", body = ResponseError),
        (status = 404, description = "Pipeline not found or not visible to the caller", body = ResponseError),
        (status = 409, description = "The pipeline was saved after the caller read it, or a dropped host or sink is still used; `error.data` is a PipelineSubsetInUse", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "pipelines",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn update_pipeline(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(pipeline_id): Path<Uuid>,
    Json(payload): Json<UpdatePipelineRequest>,
) -> Result<ResponseData<PipelineDetail>, ResponseError> {
    // validate request
    if payload.name.trim().is_empty() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Validation error")
            .with_detail("Name cannot be empty"));
    }

    let mut client = state.db_pool.get().await.ctx("get database connection")?;
    let tx = client.transaction().await.ctx("start transaction")?;

    // lock pipeline, check nobody saved it since it was read
    let pipeline = find_pipeline(&tx, pipeline_id, auth_user.user_id, true).await?;
    if pipeline.last_updated != payload.last_updated {
        return Err(ResponseError::from_status(StatusCode::CONFLICT)
            .with_title("Pipeline changed")
            .with_detail("The pipeline was saved after it was read. Read it again and retry."));
    }

    // check only the owner changes visibility
    if payload.visibility != pipeline.visibility
        && pipeline.owner_user_id != Some(auth_user.user_id)
    {
        return Err(ResponseError::from_status(StatusCode::FORBIDDEN)
            .with_title("Not the owner")
            .with_detail("Only the owner can change the visibility of this pipeline"));
    }

    // check hosts and sinks exist
    let missing = missing_ids(&tx, Hosts::Table, Hosts::HostId, &payload.host_ids)
        .await
        .ctx("check host ids")?;
    if !missing.is_empty() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Validation error")
            .with_detail(format!("Host IDs not found: {}", fmt_ids(&missing))));
    }
    let missing = missing_ids(&tx, Sinks::Table, Sinks::SinkId, &payload.sink_ids)
        .await
        .ctx("check sink ids")?;
    if !missing.is_empty() {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Validation error")
            .with_detail(format!("Sink IDs not found: {}", fmt_ids(&missing))));
    }

    // find what still uses the dropped hosts and sinks
    let detail = load_detail(&tx, pipeline.clone()).await?;
    let dropped_hosts: Vec<Uuid> = detail
        .host_ids
        .iter()
        .filter(|id| !payload.host_ids.contains(id))
        .copied()
        .collect();
    let dropped_sinks: Vec<Uuid> = detail
        .sink_ids
        .iter()
        .filter(|id| !payload.sink_ids.contains(id))
        .copied()
        .collect();
    let items = items_of_hosts(&tx, pipeline_id, &dropped_hosts)
        .await
        .ctx("query items of dropped hosts")?;
    let sink_nodes = nodes_of_sinks(&tx, pipeline_id, &dropped_sinks)
        .await
        .ctx("query nodes of dropped sinks")?;
    if !(items.is_empty() && sink_nodes.is_empty()) && !payload.force {
        let node_ids: BTreeSet<Uuid> = items
            .iter()
            .map(|(node_id, _)| *node_id)
            .chain(sink_nodes.iter().copied())
            .collect();
        return Err(ResponseError::from_status(StatusCode::CONFLICT)
            .with_title("Subset in use")
            .with_detail(format!(
                "{} items and {} sink nodes use hosts or sinks that would be dropped. \
                 Send force to take them off the canvas.",
                items.len(),
                sink_nodes.len()
            ))
            .with_data(PipelineSubsetInUse {
                node_ids: node_ids.into_iter().collect(),
                item_ids: items
                    .iter()
                    .take(LISTED_ITEMS)
                    .map(|(_, item_id)| *item_id)
                    .collect(),
                item_count: items.len(),
            }));
    }

    let now = now();

    // take forced items off their sources
    let sources: BTreeSet<Uuid> = items.iter().map(|(node_id, _)| *node_id).collect();
    if !sources.is_empty() {
        let host_items = DbQuery::select()
            .column((Items::Table, Items::ItemId))
            .from(Items::Table)
            .inner_join(
                HostTemplates::Table,
                Expr::col((HostTemplates::Table, HostTemplates::HostTemplateId))
                    .equals((Items::Table, Items::HostTemplateId)),
            )
            .and_where(
                Expr::col((HostTemplates::Table, HostTemplates::HostId))
                    .is_in(dropped_hosts.iter().copied()),
            )
            .to_owned();
        let (sql, values) = DbQuery::delete()
            .from_table(PipelineNodeSourceItems::Table)
            .and_where(Expr::col(PipelineNodeSourceItems::NodeId).is_in(sources.iter().copied()))
            .and_where(Expr::col(PipelineNodeSourceItems::ItemId).in_subquery(host_items.clone()))
            .build_postgres(PostgresQueryBuilder);
        tx.execute(sql.as_str(), &values.as_params())
            .await
            .ctx("delete node items")?;
        let source_processors = DbQuery::select()
            .column(PipelineNodeSources::SourceProcessorId)
            .from(PipelineNodeSources::Table)
            .and_where(Expr::col(PipelineNodeSources::NodeId).is_in(sources.iter().copied()))
            .to_owned();
        let (sql, values) = DbQuery::delete()
            .from_table(ItemProcessors::Table)
            .and_where(Expr::col(ItemProcessors::ProcessorId).in_subquery(source_processors))
            .and_where(Expr::col(ItemProcessors::ItemId).in_subquery(host_items))
            .build_postgres(PostgresQueryBuilder);
        tx.execute(sql.as_str(), &values.as_params())
            .await
            .ctx("unroute items")?;
        let (sql, values) = DbQuery::update()
            .table(PipelineNodes::Table)
            .value(PipelineNodes::LastUpdated, now)
            .and_where(Expr::col(PipelineNodes::NodeId).is_in(sources))
            .build_postgres(PostgresQueryBuilder);
        tx.execute(sql.as_str(), &values.as_params())
            .await
            .ctx("update source nodes")?;
    }

    // empty forced sink nodes
    if !sink_nodes.is_empty() {
        let (sql, values) = DbQuery::delete()
            .from_table(PipelineNodeSinks::Table)
            .and_where(Expr::col(PipelineNodeSinks::NodeId).is_in(sink_nodes.iter().copied()))
            .build_postgres(PostgresQueryBuilder);
        tx.execute(sql.as_str(), &values.as_params())
            .await
            .ctx("empty sink nodes")?;
        let (sql, values) = DbQuery::update()
            .table(PipelineNodes::Table)
            .value(PipelineNodes::LastUpdated, now)
            .and_where(Expr::col(PipelineNodes::NodeId).is_in(sink_nodes.iter().copied()))
            .build_postgres(PostgresQueryBuilder);
        tx.execute(sql.as_str(), &values.as_params())
            .await
            .ctx("update sink nodes")?;
    }

    // replace host subset
    let (sql, values) = DbQuery::delete()
        .from_table(PipelineHosts::Table)
        .and_where(Expr::col(PipelineHosts::PipelineId).eq(pipeline_id))
        .build_postgres(PostgresQueryBuilder);
    tx.execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete pipeline hosts")?;
    if !payload.host_ids.is_empty() {
        let mut query = DbQuery::insert();
        query
            .into_table(PipelineHosts::Table)
            .columns([PipelineHosts::PipelineId, PipelineHosts::HostId]);
        for host_id in &payload.host_ids {
            query
                .values([pipeline_id.into(), (*host_id).into()])
                .ctx("build insert hosts query")?;
        }
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        tx.execute(sql.as_str(), &values.as_params())
            .await
            .ctx("insert pipeline hosts")?;
    }

    // replace sink subset
    let (sql, values) = DbQuery::delete()
        .from_table(PipelineSinks::Table)
        .and_where(Expr::col(PipelineSinks::PipelineId).eq(pipeline_id))
        .build_postgres(PostgresQueryBuilder);
    tx.execute(sql.as_str(), &values.as_params())
        .await
        .ctx("delete pipeline sinks")?;
    if !payload.sink_ids.is_empty() {
        let mut query = DbQuery::insert();
        query
            .into_table(PipelineSinks::Table)
            .columns([PipelineSinks::PipelineId, PipelineSinks::SinkId]);
        for sink_id in &payload.sink_ids {
            query
                .values([pipeline_id.into(), (*sink_id).into()])
                .ctx("build insert sinks query")?;
        }
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        tx.execute(sql.as_str(), &values.as_params())
            .await
            .ctx("insert pipeline sinks")?;
    }

    // update pipeline
    let (sql, values) = DbQuery::update()
        .table(Pipelines::Table)
        .values([
            (Pipelines::Name, payload.name.clone().into()),
            (Pipelines::Visibility, (payload.visibility as i16).into()),
            (Pipelines::AdmStatus, (payload.adm_status as i16).into()),
            (Pipelines::LastUpdated, now.into()),
        ])
        .and_where(Expr::col(Pipelines::PipelineId).eq(pipeline_id))
        .build_postgres(PostgresQueryBuilder);
    tx.execute(sql.as_str(), &values.as_params())
        .await
        .ctx("update pipeline")?;

    // rename hidden processors
    if payload.name != pipeline.name {
        rename_hidden_processors(&tx, pipeline_id, &payload.name, now)
            .await
            .ctx("rename hidden processors")?;
    }

    // rewire the runtime rows of every canvas
    let enabled = matches!(payload.adm_status, AdmStatus::Enabled);
    for canvas in &detail.canvases {
        sync_runtime(&tx, canvas.canvas_id, enabled)
            .await
            .ctx("sync runtime")?
            .bad_request("Validation error")?;
    }

    let pipeline = find_pipeline(&tx, pipeline_id, auth_user.user_id, false).await?;
    let detail = load_detail(&tx, pipeline).await?;
    tx.commit().await.ctx("commit transaction")?;

    Ok(ResponseData::from_status(StatusCode::OK).with_data(detail))
}

/// `(source node, item)` for the items of `host_ids` on the pipeline's canvases.
async fn items_of_hosts(
    client: &impl GenericClient,
    pipeline_id: Uuid,
    host_ids: &[Uuid],
) -> Result<Vec<(Uuid, Uuid)>, tokio_postgres::Error> {
    if host_ids.is_empty() {
        return Ok(Vec::new());
    }
    let (sql, values) = DbQuery::select()
        .columns([
            (
                PipelineNodeSourceItems::Table,
                PipelineNodeSourceItems::NodeId,
            ),
            (
                PipelineNodeSourceItems::Table,
                PipelineNodeSourceItems::ItemId,
            ),
        ])
        .from(PipelineNodeSourceItems::Table)
        .inner_join(
            PipelineNodes::Table,
            Expr::col((PipelineNodes::Table, PipelineNodes::NodeId)).equals((
                PipelineNodeSourceItems::Table,
                PipelineNodeSourceItems::NodeId,
            )),
        )
        .inner_join(
            PipelineCanvases::Table,
            Expr::col((PipelineCanvases::Table, PipelineCanvases::CanvasId))
                .equals((PipelineNodes::Table, PipelineNodes::CanvasId)),
        )
        .inner_join(
            Items::Table,
            Expr::col((Items::Table, Items::ItemId)).equals((
                PipelineNodeSourceItems::Table,
                PipelineNodeSourceItems::ItemId,
            )),
        )
        .inner_join(
            HostTemplates::Table,
            Expr::col((HostTemplates::Table, HostTemplates::HostTemplateId))
                .equals((Items::Table, Items::HostTemplateId)),
        )
        .and_where(
            Expr::col((PipelineCanvases::Table, PipelineCanvases::PipelineId)).eq(pipeline_id),
        )
        .and_where(
            Expr::col((HostTemplates::Table, HostTemplates::HostId))
                .is_in(host_ids.iter().copied()),
        )
        .order_by(
            (
                PipelineNodeSourceItems::Table,
                PipelineNodeSourceItems::ItemId,
            ),
            sea_query::Order::Asc,
        )
        .build_postgres(PostgresQueryBuilder);
    Ok(client
        .query(sql.as_str(), &values.as_params())
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect())
}

/// The sink nodes on the pipeline's canvases that hold one of `sink_ids`.
async fn nodes_of_sinks(
    client: &impl GenericClient,
    pipeline_id: Uuid,
    sink_ids: &[Uuid],
) -> Result<Vec<Uuid>, tokio_postgres::Error> {
    if sink_ids.is_empty() {
        return Ok(Vec::new());
    }
    let (sql, values) = DbQuery::select()
        .column((PipelineNodeSinks::Table, PipelineNodeSinks::NodeId))
        .from(PipelineNodeSinks::Table)
        .inner_join(
            PipelineNodes::Table,
            Expr::col((PipelineNodes::Table, PipelineNodes::NodeId))
                .equals((PipelineNodeSinks::Table, PipelineNodeSinks::NodeId)),
        )
        .inner_join(
            PipelineCanvases::Table,
            Expr::col((PipelineCanvases::Table, PipelineCanvases::CanvasId))
                .equals((PipelineNodes::Table, PipelineNodes::CanvasId)),
        )
        .and_where(
            Expr::col((PipelineCanvases::Table, PipelineCanvases::PipelineId)).eq(pipeline_id),
        )
        .and_where(
            Expr::col((PipelineNodeSinks::Table, PipelineNodeSinks::SinkId))
                .is_in(sink_ids.iter().copied()),
        )
        .build_postgres(PostgresQueryBuilder);
    Ok(client
        .query(sql.as_str(), &values.as_params())
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect())
}
