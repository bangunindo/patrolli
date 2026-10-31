// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::handlers::pipelines::graph::{
    CanvasRuntime, GraphNode, GraphNodeContent, GraphNodeData, GraphNodeKind, GraphProcessor,
    GraphProcessorGroupContent, GraphProcessorGroupNode, GraphProcessorSummary, GraphRelation,
    GraphSinkContent, GraphSinkNode, GraphSourceContent, GraphSourceItem, GraphSourceNode,
    PipelineGraph, RuntimeEdges, RuntimeNode, compile, find_cycle,
};
use crate::config::processor::{Config, ProcessorKind};
use crate::db::query::{ProcessorColumn, ProcessorParams, processors};
use crate::db::schema::{
    AdmStatus, HostTemplates, Hosts, ItemProcessors, Items, PipelineCanvases,
    PipelineNodeProcessors, PipelineNodeRelations, PipelineNodeSinks, PipelineNodeSourceItems,
    PipelineNodeSources, PipelineNodes, ProcessorInputs, Processors, SinkInputs, Sinks,
};
use anyhow::Context;
use chrono::{DateTime, Utc};
use deadpool_postgres::GenericClient;
use sea_query::extension::postgres::PgExpr;
use sea_query::{
    Asterisk, CommonTableExpression, Cond, Cycle, Expr, ExprTrait, Func, OnConflict, Order,
    PostgresQueryBuilder, Query as DbQuery, QueryStatementBuilder, UnionType, WindowStatement,
    WithClause,
};
use sea_query_postgres::PostgresBinder;
use std::collections::{BTreeSet, HashMap, HashSet};
use uuid::Uuid;

/// Reads the structure of a canvas, with a summary of what each node holds.
/// Nodes come ordered by id, processors in execution order, and relations by
/// their ends, so the same canvas always reads the same.
pub(super) async fn load_graph(
    client: &impl GenericClient,
    canvas_id: Uuid,
) -> anyhow::Result<PipelineGraph> {
    // count items per source node
    let (sql, values) = DbQuery::select()
        .column((
            PipelineNodeSourceItems::Table,
            PipelineNodeSourceItems::NodeId,
        ))
        .expr(Func::count(Expr::col(Asterisk)))
        .from(PipelineNodeSourceItems::Table)
        .inner_join(
            PipelineNodes::Table,
            Expr::col((PipelineNodes::Table, PipelineNodes::NodeId)).equals((
                PipelineNodeSourceItems::Table,
                PipelineNodeSourceItems::NodeId,
            )),
        )
        .and_where(Expr::col((PipelineNodes::Table, PipelineNodes::CanvasId)).eq(canvas_id))
        .group_by_col((
            PipelineNodeSourceItems::Table,
            PipelineNodeSourceItems::NodeId,
        ))
        .build_postgres(PostgresQueryBuilder);
    let mut item_counts = HashMap::new();
    for row in client.query(sql.as_str(), &values.as_params()).await? {
        let count: i64 = row.try_get(1)?;
        item_counts.insert(row.try_get::<_, Uuid>(0)?, u32::try_from(count)?);
    }

    // load processors per group, in execution order
    let (sql, values) = DbQuery::select()
        .column((
            PipelineNodeProcessors::Table,
            PipelineNodeProcessors::NodeId,
        ))
        .columns([
            (Processors::Table, Processors::ProcessorId),
            (Processors::Table, Processors::Name),
            (Processors::Table, Processors::Type),
            (Processors::Table, Processors::AdmStatus),
        ])
        .from(PipelineNodeProcessors::Table)
        .inner_join(
            PipelineNodes::Table,
            Expr::col((PipelineNodes::Table, PipelineNodes::NodeId)).equals((
                PipelineNodeProcessors::Table,
                PipelineNodeProcessors::NodeId,
            )),
        )
        .inner_join(
            Processors::Table,
            Expr::col((Processors::Table, Processors::ProcessorId)).equals((
                PipelineNodeProcessors::Table,
                PipelineNodeProcessors::ProcessorId,
            )),
        )
        .and_where(Expr::col((PipelineNodes::Table, PipelineNodes::CanvasId)).eq(canvas_id))
        .order_by(
            (
                PipelineNodeProcessors::Table,
                PipelineNodeProcessors::NodeId,
            ),
            Order::Asc,
        )
        .order_by(
            (
                PipelineNodeProcessors::Table,
                PipelineNodeProcessors::Position,
            ),
            Order::Asc,
        )
        .build_postgres(PostgresQueryBuilder);
    let mut processors: HashMap<Uuid, Vec<GraphProcessorSummary>> = HashMap::new();
    for row in client.query(sql.as_str(), &values.as_params()).await? {
        let r#type: String = row.try_get(3)?;
        let adm_status: i16 = row.try_get(4)?;
        processors
            .entry(row.try_get(0)?)
            .or_default()
            .push(GraphProcessorSummary {
                processor_id: row.try_get(1)?,
                name: row.try_get(2)?,
                r#type: r#type
                    .parse::<ProcessorKind>()
                    .with_context(|| format!("unknown processor type: {}", r#type))?,
                adm_status: adm_status.try_into()?,
            });
    }

    // load nodes, with the sink of each sink node
    let (sql, values) = DbQuery::select()
        .columns([
            (PipelineNodes::Table, PipelineNodes::NodeId),
            (PipelineNodes::Table, PipelineNodes::Kind),
            (PipelineNodes::Table, PipelineNodes::PositionX),
            (PipelineNodes::Table, PipelineNodes::PositionY),
        ])
        .column((PipelineNodeSinks::Table, PipelineNodeSinks::SinkId))
        .from(PipelineNodes::Table)
        .left_join(
            PipelineNodeSinks::Table,
            Expr::col((PipelineNodeSinks::Table, PipelineNodeSinks::NodeId))
                .equals((PipelineNodes::Table, PipelineNodes::NodeId)),
        )
        .and_where(Expr::col((PipelineNodes::Table, PipelineNodes::CanvasId)).eq(canvas_id))
        .order_by((PipelineNodes::Table, PipelineNodes::NodeId), Order::Asc)
        .build_postgres(PostgresQueryBuilder);
    let mut nodes = Vec::new();
    for row in client.query(sql.as_str(), &values.as_params()).await? {
        let node_id: Uuid = row.try_get(0)?;
        let kind: String = row.try_get(1)?;
        let data = match kind
            .parse::<GraphNodeKind>()
            .with_context(|| format!("unknown node kind: {kind}"))?
        {
            GraphNodeKind::Source => GraphNodeData::Source(GraphSourceNode {
                item_count: item_counts.remove(&node_id).unwrap_or(0),
            }),
            GraphNodeKind::ProcessorGroup => {
                GraphNodeData::ProcessorGroup(GraphProcessorGroupNode {
                    processors: processors.remove(&node_id).unwrap_or_default(),
                })
            }
            GraphNodeKind::Sink => GraphNodeData::Sink(
                row.try_get::<_, Option<Uuid>>(4)?
                    .map(|sink_id| GraphSinkNode { sink_id }),
            ),
        };
        nodes.push(GraphNode {
            node_id,
            position_x: row.try_get(2)?,
            position_y: row.try_get(3)?,
            data,
        });
    }

    // load relations
    let (sql, values) = DbQuery::select()
        .columns([
            PipelineNodeRelations::FromNodeId,
            PipelineNodeRelations::ToNodeId,
        ])
        .from(PipelineNodeRelations::Table)
        .and_where(Expr::col(PipelineNodeRelations::CanvasId).eq(canvas_id))
        .order_by(PipelineNodeRelations::FromNodeId, Order::Asc)
        .order_by(PipelineNodeRelations::ToNodeId, Order::Asc)
        .build_postgres(PostgresQueryBuilder);
    let relations = client
        .query(sql.as_str(), &values.as_params())
        .await?
        .iter()
        .map(|row| {
            Ok(GraphRelation {
                from_node_id: row.try_get(0)?,
                to_node_id: row.try_get(1)?,
            })
        })
        .collect::<anyhow::Result<_>>()?;

    Ok(PipelineGraph { nodes, relations })
}

/// Creates the two hidden null processors of a new source node.
pub(super) async fn insert_hidden_processors(
    client: &impl GenericClient,
    pipeline_name: &str,
    source_processor_id: Uuid,
    gate_processor_id: Uuid,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let mut query = DbQuery::insert();
    query.into_table(Processors::Table).columns([
        Processors::ProcessorId,
        Processors::Name,
        Processors::Type,
        Processors::AdmStatus,
        Processors::LastUpdated,
        Processors::Config,
    ]);
    for (id, suffix) in [(source_processor_id, "source"), (gate_processor_id, "gate")] {
        query.values([
            id.into(),
            hidden_processor_name(pipeline_name, suffix).into(),
            ProcessorKind::Null.to_string().into(),
            (AdmStatus::Enabled as i16).into(),
            now.into(),
            Config::Null.to_json_value()?.into(),
        ])?;
    }
    let (sql, values) = query.build_postgres(PostgresQueryBuilder);
    client.execute(sql.as_str(), &values.as_params()).await?;
    Ok(())
}

/// Recursive CTE that walks up template items until an item has a name.
const NAME_CHAIN_CTE: &str = "name_chain";

/// Reads what a node holds: one page of a source's items, a group's
/// processors in execution order, or the sink of a sink node.
pub(super) async fn load_node_content(
    client: &impl GenericClient,
    node_id: Uuid,
    kind: GraphNodeKind,
    name: Option<String>,
    limit: u64,
    offset: u64,
) -> anyhow::Result<GraphNodeContent> {
    match kind {
        GraphNodeKind::Source => {
            // An item without a name of its own takes the name of the template
            // item it was applied from, up the chain. This walks only names; the
            // shared items query resolves every column and was 10x slower on
            // 100k items.
            let mut start = DbQuery::select()
                .column((
                    PipelineNodeSourceItems::Table,
                    PipelineNodeSourceItems::ItemId,
                ))
                .columns([
                    (Items::Table, Items::TemplateItemId),
                    (Items::Table, Items::Name),
                ])
                .from(PipelineNodeSourceItems::Table)
                .inner_join(
                    Items::Table,
                    Expr::col((Items::Table, Items::ItemId)).equals((
                        PipelineNodeSourceItems::Table,
                        PipelineNodeSourceItems::ItemId,
                    )),
                )
                .and_where(
                    Expr::col((
                        PipelineNodeSourceItems::Table,
                        PipelineNodeSourceItems::NodeId,
                    ))
                    .eq(node_id),
                )
                .to_owned();
            let step = DbQuery::select()
                .column((NAME_CHAIN_CTE, "item_id"))
                .columns([
                    (Items::Table, Items::TemplateItemId),
                    (Items::Table, Items::Name),
                ])
                .from(NAME_CHAIN_CTE)
                .inner_join(
                    Items::Table,
                    Expr::col((Items::Table, Items::ItemId)).equals((NAME_CHAIN_CTE, "next_id")),
                )
                .and_where(Expr::col((NAME_CHAIN_CTE, "name")).is_null())
                .to_owned();
            let chain = WithClause::new()
                .recursive(true)
                .cte(
                    CommonTableExpression::new()
                        .query(start.union(UnionType::All, step).to_owned())
                        .columns(["item_id", "next_id", "name"])
                        .table_name(NAME_CHAIN_CTE)
                        .to_owned(),
                )
                .cycle(Cycle::new_from_expr_set_using(
                    Expr::col("next_id"),
                    "is_cycle",
                    "path",
                ))
                .to_owned();

            let mut page = DbQuery::select();
            page.columns([(NAME_CHAIN_CTE, "item_id"), (NAME_CHAIN_CTE, "name")])
                .columns([
                    (Items::Table, Items::Type),
                    (Items::Table, Items::AdmStatus),
                ])
                .column((HostTemplates::Table, HostTemplates::HostId))
                .column((Hosts::Table, Hosts::Name))
                .expr_window_as(
                    Func::count(Expr::col(Asterisk)),
                    WindowStatement::new(),
                    "total",
                )
                .from(NAME_CHAIN_CTE)
                .inner_join(
                    Items::Table,
                    Expr::col((Items::Table, Items::ItemId)).equals((NAME_CHAIN_CTE, "item_id")),
                )
                .inner_join(
                    HostTemplates::Table,
                    Expr::col((HostTemplates::Table, HostTemplates::HostTemplateId))
                        .equals((Items::Table, Items::HostTemplateId)),
                )
                .inner_join(
                    Hosts::Table,
                    Expr::col((Hosts::Table, Hosts::HostId))
                        .equals((HostTemplates::Table, HostTemplates::HostId)),
                )
                // the end of each walk: a name, no template item left, or a loop
                .cond_where(
                    Cond::any()
                        .add(Expr::col((NAME_CHAIN_CTE, "name")).is_not_null())
                        .add(Expr::col((NAME_CHAIN_CTE, "next_id")).is_null())
                        .add(Expr::col((NAME_CHAIN_CTE, "is_cycle")).eq(true)),
                )
                .order_by((NAME_CHAIN_CTE, "name"), Order::Asc)
                .order_by((NAME_CHAIN_CTE, "item_id"), Order::Asc)
                .limit(limit)
                .offset(offset);
            if let Some(name) = &name {
                page.and_where(Expr::col((NAME_CHAIN_CTE, "name")).ilike(format!("%{name}%")));
            }
            let (sql, values) = page
                .clone()
                .with(chain.clone())
                .build_postgres(PostgresQueryBuilder);
            let rows = client.query(sql.as_str(), &values.as_params()).await?;

            let mut items = Vec::with_capacity(rows.len());
            let mut total: i64 = 0;
            for row in &rows {
                let r#type: String = row.try_get(2)?;
                let adm_status: i16 = row.try_get(3)?;
                total = row.try_get(6)?;
                items.push(GraphSourceItem {
                    item_id: row.try_get(0)?,
                    name: row.try_get(1)?,
                    host_id: row.try_get(4)?,
                    host_name: row.try_get(5)?,
                    r#type: r#type.parse()?,
                    adm_status: adm_status.try_into()?,
                });
            }
            // a page past the end has no row to carry the total
            if rows.is_empty() && offset > 0 {
                let (sql, values) = page
                    .offset(0)
                    .limit(1)
                    .to_owned()
                    .with(chain)
                    .build_postgres(PostgresQueryBuilder);
                if let Some(row) = client.query_opt(sql.as_str(), &values.as_params()).await? {
                    total = row.try_get(6)?;
                }
            }

            Ok(GraphNodeContent::Source(GraphSourceContent {
                items,
                total: total.try_into()?,
            }))
        }
        GraphNodeKind::ProcessorGroup => {
            let (sql, values) = DbQuery::select()
                .columns([
                    (Processors::Table, Processors::ProcessorId),
                    (Processors::Table, Processors::Name),
                    (Processors::Table, Processors::AdmStatus),
                ])
                .expr(ProcessorColumn::ConfigEmbed)
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
                .order_by(
                    (
                        PipelineNodeProcessors::Table,
                        PipelineNodeProcessors::Position,
                    ),
                    Order::Asc,
                )
                .build_postgres(PostgresQueryBuilder);
            let mut processors = Vec::new();
            for row in client.query(sql.as_str(), &values.as_params()).await? {
                let adm_status: i16 = row.try_get(2)?;
                processors.push(GraphProcessor {
                    processor_id: row.try_get(0)?,
                    name: row.try_get(1)?,
                    adm_status: adm_status.try_into()?,
                    config: serde_json::from_value(row.try_get(3)?)?,
                });
            }
            Ok(GraphNodeContent::ProcessorGroup(
                GraphProcessorGroupContent { processors },
            ))
        }
        GraphNodeKind::Sink => {
            let (sql, values) = DbQuery::select()
                .columns([
                    (Sinks::Table, Sinks::SinkId),
                    (Sinks::Table, Sinks::Name),
                    (Sinks::Table, Sinks::Type),
                    (Sinks::Table, Sinks::AdmStatus),
                ])
                .from(PipelineNodeSinks::Table)
                .inner_join(
                    Sinks::Table,
                    Expr::col((Sinks::Table, Sinks::SinkId))
                        .equals((PipelineNodeSinks::Table, PipelineNodeSinks::SinkId)),
                )
                .and_where(
                    Expr::col((PipelineNodeSinks::Table, PipelineNodeSinks::NodeId)).eq(node_id),
                )
                .build_postgres(PostgresQueryBuilder);
            let Some(row) = client.query_opt(sql.as_str(), &values.as_params()).await? else {
                return Ok(GraphNodeContent::Sink(None));
            };
            let r#type: String = row.try_get(2)?;
            let adm_status: i16 = row.try_get(3)?;
            Ok(GraphNodeContent::Sink(Some(GraphSinkContent {
                sink_id: row.try_get(0)?,
                name: row.try_get(1)?,
                r#type: r#type.parse()?,
                adm_status: adm_status.try_into()?,
            })))
        }
    }
}

/// Reads the processors of a group back the way the engine does, so a row
/// the engine could not load fails the save instead of every node's config
/// reload. Disabled processors are read too, as `GET /pipelines` reads them.
// jsonb rewrites numbers, e.g. 1e20 comes back as 100000000000000000000,
// which no longer parses as a float
pub(super) async fn check_stored_processors(
    client: &impl GenericClient,
    node_id: Uuid,
) -> Result<anyhow::Result<()>, tokio_postgres::Error> {
    let group: Expr = DbQuery::select()
        .column(PipelineNodeProcessors::ProcessorId)
        .from(PipelineNodeProcessors::Table)
        .and_where(Expr::col(PipelineNodeProcessors::NodeId).eq(node_id))
        .to_owned()
        .into_sub_query_statement()
        .into();
    let mut params = ProcessorParams::new();
    params
        .with_columns(vec![
            ProcessorColumn::ProcessorId,
            ProcessorColumn::ConfigEmbed,
        ])
        .with_id_subquery_filter(Some(group));
    let (sql, values) = processors(params).build_postgres(PostgresQueryBuilder);
    let rows = client.query(sql.as_str(), &values.as_params()).await?;
    Ok(rows.iter().try_for_each(|row| {
        let id: Uuid = row.try_get(ProcessorColumn::ProcessorId.as_str())?;
        let mut processor: Config =
            serde_json::from_value(row.try_get(ProcessorColumn::ConfigEmbed.as_str())?)
                .with_context(|| format!("Processor {id} cannot be read back as stored"))?;
        processor
            .validate()
            .with_context(|| format!("Processor {id} cannot be read back as stored"))
    }))
}

/// Reads a canvas as `compile` needs it.
async fn load_canvas_runtime(
    client: &impl GenericClient,
    canvas_id: Uuid,
) -> anyhow::Result<CanvasRuntime> {
    // load processors per group, in execution order
    let (sql, values) = DbQuery::select()
        .columns([
            (
                PipelineNodeProcessors::Table,
                PipelineNodeProcessors::NodeId,
            ),
            (
                PipelineNodeProcessors::Table,
                PipelineNodeProcessors::ProcessorId,
            ),
        ])
        .from(PipelineNodeProcessors::Table)
        .inner_join(
            PipelineNodes::Table,
            Expr::col((PipelineNodes::Table, PipelineNodes::NodeId)).equals((
                PipelineNodeProcessors::Table,
                PipelineNodeProcessors::NodeId,
            )),
        )
        .and_where(Expr::col((PipelineNodes::Table, PipelineNodes::CanvasId)).eq(canvas_id))
        .order_by(
            (
                PipelineNodeProcessors::Table,
                PipelineNodeProcessors::Position,
            ),
            Order::Asc,
        )
        .build_postgres(PostgresQueryBuilder);
    let mut processors: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for row in client.query(sql.as_str(), &values.as_params()).await? {
        processors
            .entry(row.try_get(0)?)
            .or_default()
            .push(row.try_get(1)?);
    }

    // load nodes, with the hidden processors of a source and the sink of a sink node
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
        .column((PipelineNodeSinks::Table, PipelineNodeSinks::SinkId))
        .from(PipelineNodes::Table)
        .left_join(
            PipelineNodeSources::Table,
            Expr::col((PipelineNodeSources::Table, PipelineNodeSources::NodeId))
                .equals((PipelineNodes::Table, PipelineNodes::NodeId)),
        )
        .left_join(
            PipelineNodeSinks::Table,
            Expr::col((PipelineNodeSinks::Table, PipelineNodeSinks::NodeId))
                .equals((PipelineNodes::Table, PipelineNodes::NodeId)),
        )
        .and_where(Expr::col((PipelineNodes::Table, PipelineNodes::CanvasId)).eq(canvas_id))
        .build_postgres(PostgresQueryBuilder);
    let mut canvas = CanvasRuntime::default();
    for row in client.query(sql.as_str(), &values.as_params()).await? {
        let node_id: Uuid = row.try_get(0)?;
        let kind: String = row.try_get(1)?;
        let node = match kind.parse::<GraphNodeKind>()? {
            GraphNodeKind::Source => RuntimeNode::Source {
                source: row
                    .try_get::<_, Option<Uuid>>(2)?
                    .context("source node without source processor")?,
                gate: row
                    .try_get::<_, Option<Uuid>>(3)?
                    .context("source node without gate processor")?,
            },
            GraphNodeKind::ProcessorGroup => {
                RuntimeNode::ProcessorGroup(processors.remove(&node_id).unwrap_or_default())
            }
            GraphNodeKind::Sink => RuntimeNode::Sink(row.try_get(4)?),
        };
        canvas.nodes.insert(node_id, node);
    }

    // load relations
    let (sql, values) = DbQuery::select()
        .columns([
            PipelineNodeRelations::FromNodeId,
            PipelineNodeRelations::ToNodeId,
        ])
        .from(PipelineNodeRelations::Table)
        .and_where(Expr::col(PipelineNodeRelations::CanvasId).eq(canvas_id))
        .build_postgres(PostgresQueryBuilder);
    for row in client.query(sql.as_str(), &values.as_params()).await? {
        canvas.relations.insert((row.try_get(0)?, row.try_get(1)?));
    }
    Ok(canvas)
}

/// Makes the runtime rows of a canvas's processors match the canvas: the
/// rows that feed its processors and the rows from them into sinks. Rows
/// another client added there are dropped; rows that read a processor of
/// the canvas from outside it are left alone.
///
/// Nothing is written when the rows would give the engine a cycle; the inner
/// error says where.
pub(super) async fn sync_runtime(
    client: &impl GenericClient,
    canvas_id: Uuid,
    enabled: bool,
) -> anyhow::Result<anyhow::Result<()>> {
    let canvas = load_canvas_runtime(client, canvas_id).await?;
    let owned = canvas.processor_ids();
    if owned.is_empty() {
        return Ok(Ok(()));
    }
    let desired = compile(&canvas, enabled);

    // get current rows into the canvas's processors and from them into sinks
    let (sql, values) = DbQuery::select()
        .columns([ProcessorInputs::ProcessorId, ProcessorInputs::InputId])
        .from(ProcessorInputs::Table)
        .and_where(Expr::col(ProcessorInputs::ProcessorId).is_in(owned.iter().copied()))
        .build_postgres(PostgresQueryBuilder);
    let mut processor_inputs = BTreeSet::new();
    for row in client.query(sql.as_str(), &values.as_params()).await? {
        processor_inputs.insert((row.try_get(0)?, row.try_get(1)?));
    }
    let (sql, values) = DbQuery::select()
        .columns([SinkInputs::SinkId, SinkInputs::InputId])
        .from(SinkInputs::Table)
        .and_where(Expr::col(SinkInputs::InputId).is_in(owned.iter().copied()))
        .build_postgres(PostgresQueryBuilder);
    let mut sink_inputs = BTreeSet::new();
    for row in client.query(sql.as_str(), &values.as_params()).await? {
        sink_inputs.insert((row.try_get(0)?, row.try_get(1)?));
    }

    // check for a cycle before writing any row: a cycle makes every node fail
    // to load the pipeline graph. Only the canvas's own rows can form one
    // through its processors, as every input of a canvas processor is replaced
    // by `desired`; a cycle elsewhere is not this save's to fix.
    let current = RuntimeEdges {
        processor_inputs,
        sink_inputs,
    };
    if let Some(node_id) = find_cycle(&desired) {
        return Ok(Err(anyhow::anyhow!(
            "The runtime graph would have a cycle through processor {node_id}"
        )));
    }

    // delete rows the canvas no longer has
    for (processor_id, input_id) in current
        .processor_inputs
        .difference(&desired.processor_inputs)
    {
        let (sql, values) = DbQuery::delete()
            .from_table(ProcessorInputs::Table)
            .and_where(Expr::col(ProcessorInputs::ProcessorId).eq(*processor_id))
            .and_where(Expr::col(ProcessorInputs::InputId).eq(*input_id))
            .build_postgres(PostgresQueryBuilder);
        client.execute(sql.as_str(), &values.as_params()).await?;
    }
    for (sink_id, input_id) in current.sink_inputs.difference(&desired.sink_inputs) {
        let (sql, values) = DbQuery::delete()
            .from_table(SinkInputs::Table)
            .and_where(Expr::col(SinkInputs::SinkId).eq(*sink_id))
            .and_where(Expr::col(SinkInputs::InputId).eq(*input_id))
            .build_postgres(PostgresQueryBuilder);
        client.execute(sql.as_str(), &values.as_params()).await?;
    }

    // insert rows the canvas gained
    let new_processor_inputs: Vec<_> = desired
        .processor_inputs
        .difference(&current.processor_inputs)
        .collect();
    if !new_processor_inputs.is_empty() {
        let mut query = DbQuery::insert();
        query
            .into_table(ProcessorInputs::Table)
            .columns([ProcessorInputs::ProcessorId, ProcessorInputs::InputId]);
        for (processor_id, input_id) in new_processor_inputs {
            query.values([(*processor_id).into(), (*input_id).into()])?;
        }
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        client.execute(sql.as_str(), &values.as_params()).await?;
    }
    let new_sink_inputs: Vec<_> = desired
        .sink_inputs
        .difference(&current.sink_inputs)
        .collect();
    if !new_sink_inputs.is_empty() {
        let mut query = DbQuery::insert();
        query
            .into_table(SinkInputs::Table)
            .columns([SinkInputs::SinkId, SinkInputs::InputId]);
        for (sink_id, input_id) in new_sink_inputs {
            query.values([(*sink_id).into(), (*input_id).into()])?;
        }
        let (sql, values) = query.build_postgres(PostgresQueryBuilder);
        client.execute(sql.as_str(), &values.as_params()).await?;
    }

    // items reach the canvas through source processors only; the rows of a
    // source's own items are written with its items, see `route_items`
    let sources: HashSet<Uuid> = canvas
        .nodes
        .values()
        .filter_map(|node| match node {
            RuntimeNode::Source { source, .. } => Some(*source),
            _ => None,
        })
        .collect();
    let (sql, values) = DbQuery::delete()
        .from_table(ItemProcessors::Table)
        .and_where(
            Expr::col(ItemProcessors::ProcessorId)
                .is_in(owned.iter().copied().filter(|id| !sources.contains(id))),
        )
        .build_postgres(PostgresQueryBuilder);
    client.execute(sql.as_str(), &values.as_params()).await?;

    Ok(Ok(()))
}

/// Routes `item_ids` into a source processor, or stops routing them there.
pub(super) async fn route_items(
    client: &impl GenericClient,
    source_processor_id: Uuid,
    item_ids: &[Uuid],
    route: bool,
) -> anyhow::Result<()> {
    if item_ids.is_empty() {
        return Ok(());
    }
    let (sql, values) = if route {
        let mut query = DbQuery::insert();
        query
            .into_table(ItemProcessors::Table)
            .columns([ItemProcessors::ItemId, ItemProcessors::ProcessorId])
            .on_conflict(
                OnConflict::columns([ItemProcessors::ItemId, ItemProcessors::ProcessorId])
                    .do_nothing()
                    .to_owned(),
            );
        for item_id in item_ids {
            query.values([(*item_id).into(), source_processor_id.into()])?;
        }
        query.build_postgres(PostgresQueryBuilder)
    } else {
        DbQuery::delete()
            .from_table(ItemProcessors::Table)
            .and_where(Expr::col(ItemProcessors::ProcessorId).eq(source_processor_id))
            .and_where(Expr::col(ItemProcessors::ItemId).is_in(item_ids.iter().copied()))
            .build_postgres(PostgresQueryBuilder)
    };
    client.execute(sql.as_str(), &values.as_params()).await?;
    Ok(())
}

/// Hidden processors are named after their pipeline, so they read well where
/// every processor is listed, like `GET /pipelines`.
fn hidden_processor_name(pipeline_name: &str, role: &str) -> String {
    format!("{pipeline_name} ({role})")
}

/// Renames the hidden processors of every source node of a pipeline.
pub(super) async fn rename_hidden_processors(
    client: &impl GenericClient,
    pipeline_id: Uuid,
    pipeline_name: &str,
    now: DateTime<Utc>,
) -> Result<(), tokio_postgres::Error> {
    for (role, column) in [
        ("source", PipelineNodeSources::SourceProcessorId),
        ("gate", PipelineNodeSources::GateProcessorId),
    ] {
        let processors = DbQuery::select()
            .column((PipelineNodeSources::Table, column))
            .from(PipelineNodeSources::Table)
            .inner_join(
                PipelineNodes::Table,
                Expr::col((PipelineNodes::Table, PipelineNodes::NodeId))
                    .equals((PipelineNodeSources::Table, PipelineNodeSources::NodeId)),
            )
            .inner_join(
                PipelineCanvases::Table,
                Expr::col((PipelineCanvases::Table, PipelineCanvases::CanvasId))
                    .equals((PipelineNodes::Table, PipelineNodes::CanvasId)),
            )
            .and_where(
                Expr::col((PipelineCanvases::Table, PipelineCanvases::PipelineId)).eq(pipeline_id),
            )
            .to_owned();
        let (sql, values) = DbQuery::update()
            .table(Processors::Table)
            .values([
                (
                    Processors::Name,
                    hidden_processor_name(pipeline_name, role).into(),
                ),
                (Processors::LastUpdated, now.into()),
            ])
            .and_where(Expr::col(Processors::ProcessorId).in_subquery(processors))
            .build_postgres(PostgresQueryBuilder);
        client.execute(sql.as_str(), &values.as_params()).await?;
    }
    Ok(())
}

/// Every processor a pipeline created: the hidden source and gate of each
/// source node, and the members of each processor group.
pub(super) async fn owned_processor_ids(
    client: &impl GenericClient,
    pipeline_id: Uuid,
) -> Result<Vec<Uuid>, tokio_postgres::Error> {
    let (sql, values) = DbQuery::select()
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
        .from(PipelineNodeSources::Table)
        .inner_join(
            PipelineNodes::Table,
            Expr::col((PipelineNodes::Table, PipelineNodes::NodeId))
                .equals((PipelineNodeSources::Table, PipelineNodeSources::NodeId)),
        )
        .inner_join(
            PipelineCanvases::Table,
            Expr::col((PipelineCanvases::Table, PipelineCanvases::CanvasId))
                .equals((PipelineNodes::Table, PipelineNodes::CanvasId)),
        )
        .and_where(
            Expr::col((PipelineCanvases::Table, PipelineCanvases::PipelineId)).eq(pipeline_id),
        )
        .build_postgres(PostgresQueryBuilder);
    let mut ids = Vec::new();
    for row in client.query(sql.as_str(), &values.as_params()).await? {
        ids.push(row.get(0));
        ids.push(row.get(1));
    }

    let (sql, values) = DbQuery::select()
        .column((
            PipelineNodeProcessors::Table,
            PipelineNodeProcessors::ProcessorId,
        ))
        .from(PipelineNodeProcessors::Table)
        .inner_join(
            PipelineNodes::Table,
            Expr::col((PipelineNodes::Table, PipelineNodes::NodeId)).equals((
                PipelineNodeProcessors::Table,
                PipelineNodeProcessors::NodeId,
            )),
        )
        .inner_join(
            PipelineCanvases::Table,
            Expr::col((PipelineCanvases::Table, PipelineCanvases::CanvasId))
                .equals((PipelineNodes::Table, PipelineNodes::CanvasId)),
        )
        .and_where(
            Expr::col((PipelineCanvases::Table, PipelineCanvases::PipelineId)).eq(pipeline_id),
        )
        .build_postgres(PostgresQueryBuilder);
    for row in client.query(sql.as_str(), &values.as_params()).await? {
        ids.push(row.get(0));
    }

    Ok(ids)
}

/// Deletes processors and every runtime row that points at them, the same way
/// `delete_processor` does for one. Nodes must not reference them anymore.
pub(super) async fn delete_processors(
    client: &impl GenericClient,
    processor_ids: &[Uuid],
) -> Result<(), tokio_postgres::Error> {
    if processor_ids.is_empty() {
        return Ok(());
    }
    let ids = || processor_ids.iter().copied();

    let (sql, values) = DbQuery::delete()
        .from_table(ItemProcessors::Table)
        .and_where(Expr::col(ItemProcessors::ProcessorId).is_in(ids()))
        .build_postgres(PostgresQueryBuilder);
    client.execute(sql.as_str(), &values.as_params()).await?;

    let (sql, values) = DbQuery::delete()
        .from_table(SinkInputs::Table)
        .and_where(Expr::col(SinkInputs::InputId).is_in(ids()))
        .build_postgres(PostgresQueryBuilder);
    client.execute(sql.as_str(), &values.as_params()).await?;

    let (sql, values) = DbQuery::delete()
        .from_table(ProcessorInputs::Table)
        .and_where(
            Expr::col(ProcessorInputs::ProcessorId)
                .is_in(ids())
                .or(Expr::col(ProcessorInputs::InputId).is_in(ids())),
        )
        .build_postgres(PostgresQueryBuilder);
    client.execute(sql.as_str(), &values.as_params()).await?;

    let (sql, values) = DbQuery::delete()
        .from_table(Processors::Table)
        .and_where(Expr::col(Processors::ProcessorId).is_in(ids()))
        .build_postgres(PostgresQueryBuilder);
    client.execute(sql.as_str(), &values.as_params()).await?;

    Ok(())
}
