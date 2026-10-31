// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::handlers::processor_builder::{
    EdgeData, Graph, ItemsNode, ItemsToProcessorEdge, ItemsToProcessorEdgeKey, Node, NodeData,
    ProcessorNode, SinkNode,
};
use crate::api::processor_builder::GraphDto;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use axum::{extract::State, http::StatusCode};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};
use uuid::Uuid;

/// Get the full graph of all resources in the system.
///
/// Returns all hosts, items, listeners, processors, and sinks as nodes,
/// with edges representing their relationships.
#[utoipa::path(
    get,
    path = "/processor_builder/graph",
    responses(
        (status = 200, description = "Full graph of all resources", body = ResponseData<Graph>),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
    tag = "processor_builder",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn get_graph(
    State(state): State<Arc<AppState>>,
) -> Result<ResponseData<GraphDto>, ResponseError> {
    let client = state.db_pool.get().await.ctx("get database connection")?;

    let mut graph = Graph::new();
    let mut next_id: u32 = 0;

    // build nodes

    // Processors
    let rows = client
        .query("SELECT processor_id, name, adm_status FROM processors", &[])
        .await
        .ctx("query processors")?;

    // processor_uuid -> node id
    let mut processor_map: HashMap<Uuid, u32> = HashMap::new();

    for row in &rows {
        let id = next_id;
        next_id += 1;

        let processor_id: Uuid = row.get(0);
        let name: String = row.get(1);
        let enabled: bool = row.get::<_, i16>(2) == 1;

        let node = Node {
            id,
            data: NodeData::Processor(ProcessorNode {
                processor_id,
                name,
                enabled,
            }),
        };
        if let Err(e) = graph.add_node(node) {
            warn!(error = %e, "processor graph node dropped");
        }
        processor_map.insert(processor_id, id);
    }

    // Sinks
    let rows = client
        .query("SELECT sink_id, name, adm_status FROM sinks", &[])
        .await
        .ctx("query sinks")?;

    // sink_uuid -> node id
    let mut sink_map: HashMap<Uuid, u32> = HashMap::new();

    for row in &rows {
        let id = next_id;
        next_id += 1;

        let sink_id: Uuid = row.get(0);
        let name: String = row.get(1);
        let enabled: bool = row.get::<_, i16>(2) == 1;

        let node = Node {
            id,
            data: NodeData::Sink(SinkNode {
                sink_id,
                name,
                enabled,
            }),
        };
        if let Err(e) = graph.add_node(node) {
            warn!(error = %e, "processor graph node dropped");
        }
        sink_map.insert(sink_id, id);
    }

    // Items
    // Aggregate items into ItemsNodes.
    let row_opt = client
        .query_opt(
            "SELECT COUNT(*) AS item_count,
                    COUNT(*) FILTER (WHERE i.adm_status = 1) AS enabled_count
             FROM items i
             JOIN host_templates ht ON ht.host_template_id = i.host_template_id
             WHERE ht.host_id IS NOT NULL",
            &[],
        )
        .await
        .ctx("query items")?;

    let mut items_node_id: Option<u32> = None;
    if let Some(row) = row_opt {
        next_id += 1;
        let id = next_id;

        let item_count: i64 = row.get(0);
        let enabled_count: i64 = row.get(1);

        items_node_id = Some(id);
        let node = Node {
            id,
            data: NodeData::Items(ItemsNode {
                item_count: item_count as usize,
                name: "Items".into(),
                enabled_count: enabled_count as usize,
            }),
        };
        if let Err(e) = graph.add_node(node) {
            warn!(error = %e, "processor graph node dropped");
        }
    }

    // Build edges

    // Items -> Processor
    // Aggregate item->processor edges by (host_id, endpoint_id, listener_id, type, processor_id).
    let rows = client
        .query(
            "SELECT ip.processor_id,
                    COUNT(*) AS item_count,
                    COUNT(*) FILTER (WHERE i.adm_status = 1) AS enabled_count
             FROM item_processors ip
             JOIN items i ON i.item_id = ip.item_id
             JOIN host_templates ht ON ht.host_template_id = i.host_template_id
             WHERE ht.host_id IS NOT NULL
             GROUP BY ip.processor_id",
            &[],
        )
        .await
        .ctx("query item-processor edges")?;

    for row in &rows {
        let processor_id: Uuid = row.get(0);
        let item_count: i64 = row.get(1);
        let enabled_count: i64 = row.get(2);

        let Some(items_nid) = items_node_id else {
            continue;
        };
        let Some(&processor_nid) = processor_map.get(&processor_id) else {
            debug!(processor_id = %processor_id, "item edge references unknown processor");
            continue;
        };

        let edge = ItemsToProcessorEdge {
            key: ItemsToProcessorEdgeKey { processor_id },
            item_count: item_count as usize,
            enabled_count: enabled_count as usize,
        };
        if let Err(e) = graph.connect(items_nid, processor_nid, EdgeData::ItemsToProcessor(edge)) {
            warn!(error = %e, "processor graph edge dropped");
        }
    }

    // Processor -> Processor
    let rows = client
        .query("SELECT processor_id, input_id FROM processor_inputs", &[])
        .await
        .ctx("query processor_inputs")?;

    for row in &rows {
        let processor_id: Uuid = row.get(0);
        let input_id: Uuid = row.get(1);

        let Some(&processor_nid) = processor_map.get(&processor_id) else {
            debug!(processor_id = %processor_id, "processor input references unknown processor");
            continue;
        };
        let Some(&input_nid) = processor_map.get(&input_id) else {
            debug!(processor_id = %processor_id, input_id = %input_id, "processor input references unknown input");
            continue;
        };
        if let Err(e) = graph.connect(input_nid, processor_nid, EdgeData::None) {
            warn!(error = %e, "processor graph edge dropped");
        }
    }

    // Processor -> Sink
    let rows = client
        .query("SELECT sink_id, input_id FROM sink_inputs", &[])
        .await
        .ctx("query sink_inputs")?;

    for row in &rows {
        let sink_id: Uuid = row.get(0);
        let processor_id: Uuid = row.get(1);

        let Some(&processor_nid) = processor_map.get(&processor_id) else {
            debug!(sink_id = %sink_id, input_id = %processor_id, "sink input references unknown processor");
            continue;
        };
        let Some(&sink_nid) = sink_map.get(&sink_id) else {
            debug!(sink_id = %sink_id, "sink input references unknown sink");
            continue;
        };
        if let Err(e) = graph.connect(processor_nid, sink_nid, EdgeData::None) {
            warn!(error = %e, "processor graph edge dropped");
        }
    }

    graph.shrink_empty();

    Ok(ResponseData::from_status(StatusCode::OK).with_data(graph.into_graph_response()))
}
