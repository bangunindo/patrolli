// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod get_graph;

pub use get_graph::get_graph;

use std::collections::HashMap;

use utoipa::OpenApi;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(OpenApi)]
#[openapi(
    paths(get_graph::get_graph,),
    components(schemas(
        HostNode,
        ItemsNode,
        ListenerNode,
        EndpointNode,
        ProcessorNode,
        SinkNode,
        NodeKind,
        NodeData,
        Node,
        // ItemsNodeKey,
        ItemsToProcessorEdgeKey,
        ItemsToProcessorEdge,
        Graph,
        GraphDto,
        GraphEdgeDto,
    ))
)]
pub struct ProcessorBuilderHandler;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HostNode {
    pub host_id: Uuid, // key
    pub name: String,
    pub enabled: bool,
}

// #[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq, Hash)]
// pub struct ItemsNodeKey {
//     pub host_id: Uuid,
//     pub endpoint_id: Option<Uuid>,
//     pub listener_id: Option<Uuid>,
//     pub r#type: ItemKind,
// }

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ItemsNode {
    // pub key: ItemsNodeKey,
    pub item_count: usize,
    pub name: String,
    pub enabled_count: usize, // Disabled count = item_count - enabled_count
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListenerNode {
    pub listener_id: Uuid, // key
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EndpointNode {
    pub endpoint_id: Uuid, // key
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProcessorNode {
    pub processor_id: Uuid, // key
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SinkNode {
    pub sink_id: Uuid, // key
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Host,
    Items,
    Listener,
    Endpoint,
    Processor,
    Sink,
}

impl NodeKind {
    pub fn can_connect_to(self, target: NodeKind) -> bool {
        use NodeKind::*;

        matches!(
            (self, target),
            (Items, Processor) | (Processor, Processor) | (Processor, Sink)
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum NodeData {
    Host(HostNode),
    Items(ItemsNode),
    Listener(ListenerNode),
    Endpoint(EndpointNode),
    Processor(ProcessorNode),
    Sink(SinkNode),
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Node {
    pub id: u32,
    #[serde(flatten)]
    pub data: NodeData,
}

impl Node {
    pub fn kind(&self) -> NodeKind {
        self.data.kind()
    }

    pub fn can_connect_to(&self, other: &Node) -> bool {
        self.data.can_connect_to(other)
    }
}

impl NodeData {
    pub fn kind(&self) -> NodeKind {
        match self {
            NodeData::Host(_) => NodeKind::Host,
            NodeData::Items(_) => NodeKind::Items,
            NodeData::Listener(_) => NodeKind::Listener,
            NodeData::Endpoint(_) => NodeKind::Endpoint,
            NodeData::Processor(_) => NodeKind::Processor,
            NodeData::Sink(_) => NodeKind::Sink,
        }
    }

    pub fn can_connect_to(&self, other: &Node) -> bool {
        self.kind().can_connect_to(other.kind())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ItemsToProcessorEdgeKey {
    // pub items_key: ItemsNodeKey,
    pub processor_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ItemsToProcessorEdge {
    pub key: ItemsToProcessorEdgeKey,
    pub item_count: usize,
    pub enabled_count: usize, // Disabled count = item_count - enabled_count
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum EdgeData {
    None,
    ItemsToProcessor(ItemsToProcessorEdge),
}

#[derive(Default, Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Graph {
    nodes: HashMap<u32, Node>,

    #[schema(value_type = HashMap<u32, HashMap<u32, EdgeData>>)]
    adj_list: HashMap<u32, HashMap<u32, EdgeData>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GraphEdgeDto {
    source: u32,
    target: u32,
    data: EdgeData,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GraphDto {
    nodes: Vec<Node>,
    edges: Vec<GraphEdgeDto>,
}

impl From<Graph> for GraphDto {
    fn from(graph: Graph) -> Self {
        let nodes = graph.nodes.into_values().collect();

        let edges = graph
            .adj_list
            .into_iter()
            .flat_map(|(source, targets)| {
                targets.into_iter().map(move |(target, data)| GraphEdgeDto {
                    source,
                    target,
                    data,
                })
            })
            .collect();

        Self { nodes, edges }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error("node already exists")]
    NodeAlreadyExists,
    #[error("node not found")]
    NodeNotFound,
    #[error("invalid connection between nodes")]
    InvalidConnection,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, node: Node) -> Result<u32, GraphError> {
        let id = node.id;

        if self.nodes.contains_key(&id) {
            return Err(GraphError::NodeAlreadyExists);
        }

        self.nodes.insert(id, node);

        // initialize adjacency list so it's always present.
        self.adj_list.entry(id).or_default();

        Ok(id)
    }

    pub fn connect(&mut self, source: u32, target: u32, data: EdgeData) -> Result<(), GraphError> {
        let source_node = self.nodes.get(&source).ok_or(GraphError::NodeNotFound)?;
        let target_node = self.nodes.get(&target).ok_or(GraphError::NodeNotFound)?;

        if !source_node.can_connect_to(target_node) {
            return Err(GraphError::InvalidConnection);
        }

        self.adj_list
            .entry(source)
            .or_default()
            .insert(target, data);

        Ok(())
    }

    /// Removes empty adjacency lists from the graph. This is useful for serialization, as it reduces the size of the serialized graph.
    pub fn shrink_empty(&mut self) {
        self.adj_list.retain(|_, v| !v.is_empty());
    }

    pub fn into_graph_response(self) -> GraphDto {
        self.into()
    }
}
