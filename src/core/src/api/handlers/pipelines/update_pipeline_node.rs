// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::handlers::pipelines::get_pipeline_node::GetPipelineNodeResponse;
use crate::api::handlers::pipelines::graph::{
    GraphNodeContentInput, GraphNodeKind, GraphSourceContentInput, GroupProcessor,
};
use crate::api::handlers::pipelines::runtime::{
    check_stored_processors, delete_processors, load_node_content, route_items, sync_runtime,
};
use crate::api::handlers::pipelines::{
    find_canvas, find_pipeline, fmt_ids, missing_ids, node_not_found, now,
};
use crate::api::jwt::AuthUser;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::config::processor::ProcessorKind;
use crate::db::schema::{
    AdmStatus, HostTemplates, Items, PipelineHosts, PipelineNodeProcessors, PipelineNodeSinks,
    PipelineNodeSourceItems, PipelineNodeSources, PipelineNodes, PipelineSinks, Processors,
};
use anyhow::Context;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use chrono::{DateTime, Utc};
use deadpool_postgres::GenericClient;
use sea_query::{Expr, ExprTrait, OnConflict, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// Postgres takes at most 65535 parameters per statement; an item row takes two.
const ITEM_CHUNK: usize = 10_000;

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePipelineNodeRequest {
    /// The `last_updated` of the node when it was read.
    pub last_updated: DateTime<Utc>,
    #[serde(flatten)]
    pub content: GraphNodeContentInput,
}

enum ValidContent {
    Source(GraphSourceContentInput),
    ProcessorGroup(Vec<GroupProcessor>),
    Sink(Option<Uuid>),
}

fn validation_error(detail: impl Into<String>) -> ResponseError {
    ResponseError::from_status(StatusCode::BAD_REQUEST)
        .with_title("Validation error")
        .with_detail(detail)
}

/// Save the content of a node
///
/// A source takes items to add and to remove. A processor group takes its
/// whole processor list in execution order: new ids are created, missing ids
/// are deleted. A sink node takes a sink of the pipeline, or null to empty it.
#[utoipa::path(
    put,
    path = "/pipelines/{pipeline_id}/canvases/{canvas_id}/nodes/{node_id}",
    params(
        ("pipeline_id" = Uuid, Path, description = "Pipeline UUID"),
        ("canvas_id" = Uuid, Path, description = "Canvas UUID"),
        ("node_id" = Uuid, Path, description = "Node UUID"),
    ),
    request_body(
        content = UpdatePipelineNodeRequest,
        description = "Save node request body",
        content_type = "application/json",
    ),
    responses(
        (status = 200, description = "Saved node, a source with its first page of items", body = ResponseData<GetPipelineNodeResponse>),
        (status = 400, description = "Validation error", body = ResponseError),
        (status = 404, description = "Pipeline, canvas or node not found, or pipeline not visible to the caller", body = ResponseError),
        (status = 409, description = "The node was saved after the caller read it", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "pipelines",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn update_pipeline_node(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path((pipeline_id, canvas_id, node_id)): Path<(Uuid, Uuid, Uuid)>,
    Json(payload): Json<UpdatePipelineNodeRequest>,
) -> Result<ResponseData<GetPipelineNodeResponse>, ResponseError> {
    // validate content
    let content = match payload.content {
        GraphNodeContentInput::Source(input) => {
            input.validate().bad_request("Validation error")?;
            ValidContent::Source(input)
        }
        GraphNodeContentInput::ProcessorGroup(input) => {
            ValidContent::ProcessorGroup(input.validate().bad_request("Validation error")?)
        }
        GraphNodeContentInput::Sink(input) => ValidContent::Sink(input.map(|sink| sink.sink_id)),
    };

    let mut client = state.db_pool.get().await.ctx("get database connection")?;
    let tx = client.transaction().await.ctx("start transaction")?;

    // lock pipeline, check it owns the canvas and the node
    let pipeline = find_pipeline(&tx, pipeline_id, auth_user.user_id, true).await?;
    find_canvas(&tx, pipeline_id, canvas_id).await?;
    let (sql, values) = DbQuery::select()
        .columns([
            (PipelineNodes::Table, PipelineNodes::Kind),
            (PipelineNodes::Table, PipelineNodes::LastUpdated),
        ])
        .column((
            PipelineNodeSources::Table,
            PipelineNodeSources::SourceProcessorId,
        ))
        .from(PipelineNodes::Table)
        .left_join(
            PipelineNodeSources::Table,
            Expr::col((PipelineNodeSources::Table, PipelineNodeSources::NodeId))
                .equals((PipelineNodes::Table, PipelineNodes::NodeId)),
        )
        .and_where(Expr::col((PipelineNodes::Table, PipelineNodes::NodeId)).eq(node_id))
        .and_where(Expr::col((PipelineNodes::Table, PipelineNodes::CanvasId)).eq(canvas_id))
        .build_postgres(PostgresQueryBuilder);
    let row = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("query node")?
        .ok_or_else(|| node_not_found(node_id))?;
    let kind: String = row.get(0);
    let kind: GraphNodeKind = kind.parse().ctx("parse node kind")?;
    let last_updated: DateTime<Utc> = row.get(1);

    // check the content fits the node and nobody saved it since it was read
    let sent = match content {
        ValidContent::Source(_) => GraphNodeKind::Source,
        ValidContent::ProcessorGroup(_) => GraphNodeKind::ProcessorGroup,
        ValidContent::Sink(_) => GraphNodeKind::Sink,
    };
    if sent != kind {
        let (kind, sent): (&str, &str) = (kind.into(), sent.into());
        return Err(validation_error(format!(
            "Node {node_id} is a {kind} node, not a {sent} node"
        )));
    }
    if last_updated != payload.last_updated {
        return Err(ResponseError::from_status(StatusCode::CONFLICT)
            .with_title("Node changed")
            .with_detail("The node was saved after it was read. Read it again and retry."));
    }

    let now = now();
    match content {
        ValidContent::Source(input) => {
            let source_processor_id: Option<Uuid> = row.get(2);
            let source_processor_id = source_processor_id
                .context("source node without source processor")
                .ctx("query node")?;
            save_source(&tx, pipeline_id, node_id, source_processor_id, input).await?
        }
        ValidContent::ProcessorGroup(processors) => {
            save_processor_group(&tx, node_id, processors, now).await?
        }
        ValidContent::Sink(sink_id) => save_sink(&tx, pipeline_id, node_id, sink_id).await?,
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

    // bump node last_updated
    let (sql, values) = DbQuery::update()
        .table(PipelineNodes::Table)
        .value(PipelineNodes::LastUpdated, now)
        .and_where(Expr::col(PipelineNodes::NodeId).eq(node_id))
        .build_postgres(PostgresQueryBuilder);
    tx.execute(sql.as_str(), &values.as_params())
        .await
        .ctx("update node")?;

    let content = load_node_content(&tx, node_id, kind, None, 20, 0)
        .await
        .ctx("load node content")?;
    tx.commit().await.ctx("commit transaction")?;

    Ok(
        ResponseData::from_status(StatusCode::OK).with_data(GetPipelineNodeResponse {
            pipeline_id,
            canvas_id,
            node_id,
            last_updated: now,
            content,
        }),
    )
}

async fn save_source(
    client: &impl GenericClient,
    pipeline_id: Uuid,
    node_id: Uuid,
    source_processor_id: Uuid,
    input: GraphSourceContentInput,
) -> Result<(), ResponseError> {
    // check added items exist and belong to a host of the pipeline
    let add: Vec<Uuid> = input.add.into_iter().collect();
    let mut found = HashSet::new();
    let mut allowed = HashSet::new();
    for chunk in add.chunks(ITEM_CHUNK) {
        let (sql, values) = DbQuery::select()
            .column((Items::Table, Items::ItemId))
            .column((PipelineHosts::Table, PipelineHosts::HostId))
            .from(Items::Table)
            .left_join(
                HostTemplates::Table,
                Expr::col((HostTemplates::Table, HostTemplates::HostTemplateId))
                    .equals((Items::Table, Items::HostTemplateId)),
            )
            .left_join(
                PipelineHosts::Table,
                Expr::col((PipelineHosts::Table, PipelineHosts::HostId))
                    .equals((HostTemplates::Table, HostTemplates::HostId))
                    .and(
                        Expr::col((PipelineHosts::Table, PipelineHosts::PipelineId))
                            .eq(pipeline_id),
                    ),
            )
            .and_where(Expr::col((Items::Table, Items::ItemId)).is_in(chunk.iter().copied()))
            .build_postgres(PostgresQueryBuilder);
        for row in client
            .query(sql.as_str(), &values.as_params())
            .await
            .ctx("check item ids")?
        {
            let item_id: Uuid = row.get(0);
            found.insert(item_id);
            if row.get::<_, Option<Uuid>>(1).is_some() {
                allowed.insert(item_id);
            }
        }
    }
    let missing: Vec<Uuid> = add
        .iter()
        .filter(|id| !found.contains(id))
        .copied()
        .collect();
    if !missing.is_empty() {
        return Err(validation_error(format!(
            "Item IDs not found: {}",
            fmt_ids(&missing)
        )));
    }
    let outside: Vec<Uuid> = add
        .iter()
        .filter(|id| !allowed.contains(id))
        .copied()
        .collect();
    if !outside.is_empty() {
        return Err(validation_error(format!(
            "Items not on a host of this pipeline: {}",
            fmt_ids(&outside)
        )));
    }

    // insert added items, skipping the ones already there
    for chunk in add.chunks(ITEM_CHUNK) {
        let mut query = DbQuery::insert();
        query
            .into_table(PipelineNodeSourceItems::Table)
            .columns([
                PipelineNodeSourceItems::NodeId,
                PipelineNodeSourceItems::ItemId,
            ])
            .on_conflict(
                OnConflict::columns([
                    PipelineNodeSourceItems::NodeId,
                    PipelineNodeSourceItems::ItemId,
                ])
                .do_nothing()
                .to_owned(),
            );
        for item_id in chunk {
            query
                .values([node_id.into(), (*item_id).into()])
                .ctx("build insert items query")?;
        }
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        client
            .execute(sql.as_str(), &values.as_params())
            .await
            .ctx("insert node items")?;
        route_items(client, source_processor_id, chunk, true)
            .await
            .ctx("route items")?;
    }

    // delete removed items
    let remove: Vec<Uuid> = input.remove.into_iter().collect();
    for chunk in remove.chunks(ITEM_CHUNK) {
        let (sql, values) = DbQuery::delete()
            .from_table(PipelineNodeSourceItems::Table)
            .and_where(Expr::col(PipelineNodeSourceItems::NodeId).eq(node_id))
            .and_where(Expr::col(PipelineNodeSourceItems::ItemId).is_in(chunk.iter().copied()))
            .build_postgres(PostgresQueryBuilder);
        client
            .execute(sql.as_str(), &values.as_params())
            .await
            .ctx("delete node items")?;
        route_items(client, source_processor_id, chunk, false)
            .await
            .ctx("unroute items")?;
    }

    Ok(())
}

/// A processor row as stored, to tell which ones changed.
#[derive(PartialEq)]
struct StoredProcessor {
    name: String,
    r#type: String,
    adm_status: i16,
    config: serde_json::Value,
}

async fn save_processor_group(
    client: &impl GenericClient,
    node_id: Uuid,
    processors: Vec<GroupProcessor>,
    now: DateTime<Utc>,
) -> Result<(), ResponseError> {
    // get current processors of the group
    let (sql, values) = DbQuery::select()
        .columns([
            (Processors::Table, Processors::ProcessorId),
            (Processors::Table, Processors::Name),
            (Processors::Table, Processors::Type),
            (Processors::Table, Processors::AdmStatus),
            (Processors::Table, Processors::Config),
        ])
        .from(PipelineNodeProcessors::Table)
        .inner_join(
            Processors::Table,
            Expr::col((Processors::Table, Processors::ProcessorId)).equals((
                PipelineNodeProcessors::Table,
                PipelineNodeProcessors::ProcessorId,
            )),
        )
        .and_where(
            Expr::col((
                PipelineNodeProcessors::Table,
                PipelineNodeProcessors::NodeId,
            ))
            .eq(node_id),
        )
        .build_postgres(PostgresQueryBuilder);
    let mut current = HashMap::new();
    for row in client
        .query(sql.as_str(), &values.as_params())
        .await
        .ctx("query group processors")?
    {
        current.insert(
            row.get::<_, Uuid>(0),
            StoredProcessor {
                name: row.get(1),
                r#type: row.get(2),
                adm_status: row.get(3),
                config: row.get(4),
            },
        );
    }

    // check new processor ids are not used by any processor
    let new_ids: BTreeSet<Uuid> = processors
        .iter()
        .map(|p| p.processor_id)
        .filter(|id| !current.contains_key(id))
        .collect();
    let free: BTreeSet<Uuid> =
        missing_ids(client, Processors::Table, Processors::ProcessorId, &new_ids)
            .await
            .ctx("check processor ids")?
            .into_iter()
            .collect();
    let used: Vec<Uuid> = new_ids.difference(&free).copied().collect();
    if !used.is_empty() {
        return Err(validation_error(format!(
            "Processor IDs already used: {}",
            fmt_ids(&used)
        )));
    }

    // detach every processor; the order is written again below
    let (sql, values) = DbQuery::delete()
        .from_table(PipelineNodeProcessors::Table)
        .and_where(Expr::col(PipelineNodeProcessors::NodeId).eq(node_id))
        .build_postgres(PostgresQueryBuilder);
    client
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("detach group processors")?;

    // delete removed processors
    let kept: HashSet<Uuid> = processors.iter().map(|p| p.processor_id).collect();
    let removed: Vec<Uuid> = current
        .keys()
        .filter(|id| !kept.contains(id))
        .copied()
        .collect();
    delete_processors(client, &removed)
        .await
        .ctx("delete processors")?;

    // insert new processors, update changed ones
    for processor in &processors {
        let stored = StoredProcessor {
            name: processor.name.clone(),
            r#type: ProcessorKind::from(&processor.config).to_string(),
            adm_status: processor.adm_status as i16,
            config: processor
                .config
                .to_json_value()
                .ctx("serialize processor")?,
        };
        let id = processor.processor_id;
        let (sql, values) = match current.get(&id) {
            None => DbQuery::insert()
                .into_table(Processors::Table)
                .columns([
                    Processors::ProcessorId,
                    Processors::Name,
                    Processors::Type,
                    Processors::AdmStatus,
                    Processors::LastUpdated,
                    Processors::Config,
                ])
                .values([
                    id.into(),
                    stored.name.into(),
                    stored.r#type.into(),
                    stored.adm_status.into(),
                    now.into(),
                    stored.config.into(),
                ])
                .ctx("build insert processor query")?
                .build_postgres(PostgresQueryBuilder),
            // every update of a processor reloads it on every node
            Some(old) if *old == stored => continue,
            Some(_) => DbQuery::update()
                .table(Processors::Table)
                .values([
                    (Processors::Name, stored.name.into()),
                    (Processors::Type, stored.r#type.into()),
                    (Processors::AdmStatus, stored.adm_status.into()),
                    (Processors::LastUpdated, now.into()),
                    (Processors::Config, stored.config.into()),
                ])
                .and_where(Expr::col(Processors::ProcessorId).eq(id))
                .build_postgres(PostgresQueryBuilder),
        };
        client
            .execute(sql.as_str(), &values.as_params())
            .await
            .ctx("write processor")?;
    }

    // attach processors in order
    if !processors.is_empty() {
        let mut query = DbQuery::insert();
        query.into_table(PipelineNodeProcessors::Table).columns([
            PipelineNodeProcessors::ProcessorId,
            PipelineNodeProcessors::NodeId,
            PipelineNodeProcessors::Position,
        ]);
        for (position, processor) in processors.iter().enumerate() {
            query
                .values([
                    processor.processor_id.into(),
                    node_id.into(),
                    (position as i32).into(),
                ])
                .ctx("build attach processors query")?;
        }
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        client
            .execute(sql.as_str(), &values.as_params())
            .await
            .ctx("attach group processors")?;
    }

    // check the engine can load what was stored
    check_stored_processors(client, node_id)
        .await
        .ctx("read back processors")?
        .bad_request("Validation error")?;

    Ok(())
}

async fn save_sink(
    client: &impl GenericClient,
    pipeline_id: Uuid,
    node_id: Uuid,
    sink_id: Option<Uuid>,
) -> Result<(), ResponseError> {
    // empty the node
    let Some(sink_id) = sink_id else {
        let (sql, values) = DbQuery::delete()
            .from_table(PipelineNodeSinks::Table)
            .and_where(Expr::col(PipelineNodeSinks::NodeId).eq(node_id))
            .build_postgres(PostgresQueryBuilder);
        client
            .execute(sql.as_str(), &values.as_params())
            .await
            .ctx("delete node sink")?;
        return Ok(());
    };

    // check the sink is in the pipeline's sink subset
    let (sql, values) = DbQuery::select()
        .column(PipelineSinks::SinkId)
        .from(PipelineSinks::Table)
        .and_where(Expr::col(PipelineSinks::PipelineId).eq(pipeline_id))
        .and_where(Expr::col(PipelineSinks::SinkId).eq(sink_id))
        .build_postgres(PostgresQueryBuilder);
    if client
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("check sink id")?
        .is_none()
    {
        return Err(validation_error(format!(
            "Sink {sink_id} is not in this pipeline"
        )));
    }

    // set the sink of the node
    let (sql, values) = DbQuery::insert()
        .into_table(PipelineNodeSinks::Table)
        .columns([PipelineNodeSinks::NodeId, PipelineNodeSinks::SinkId])
        .values([node_id.into(), sink_id.into()])
        .ctx("build insert node sink query")?
        .on_conflict(
            OnConflict::column(PipelineNodeSinks::NodeId)
                .update_column(PipelineNodeSinks::SinkId)
                .to_owned(),
        )
        .build_postgres(PostgresQueryBuilder);
    client
        .execute(sql.as_str(), &values.as_params())
        .await
        .ctx("set node sink")?;

    Ok(())
}
