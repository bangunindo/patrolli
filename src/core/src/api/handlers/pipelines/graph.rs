// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::config::item::ItemKind;
use crate::config::processor::{Config, ProcessorKind};
use crate::config::sink::SinkKind;
use crate::db::schema::{AdmStatus, default_adm_status};
use anyhow::{Context, bail};
use petgraph::acyclic::Acyclic;
use petgraph::prelude::DiGraphMap;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use strum_macros::{EnumDiscriminants, EnumString, IntoStaticStr};
use utoipa::ToSchema;
use uuid::Uuid;

/// The structure of a canvas. What a node holds is edited per node; the graph
/// only carries a read-only summary of it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PipelineGraph {
    pub nodes: Vec<GraphNode>,
    pub relations: Vec<GraphRelation>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GraphNode {
    pub node_id: Uuid,
    pub position_x: i32,
    pub position_y: i32,
    #[serde(flatten)]
    pub data: GraphNodeData,
}

#[derive(Debug, Clone, Serialize, ToSchema, EnumDiscriminants)]
#[serde(rename_all = "snake_case", tag = "kind", content = "data")]
#[strum_discriminants(
    name(GraphNodeKind),
    derive(EnumString, IntoStaticStr, Deserialize, ToSchema),
    strum(serialize_all = "snake_case"),
    serde(rename_all = "snake_case")
)]
pub enum GraphNodeData {
    Source(GraphSourceNode),
    ProcessorGroup(GraphProcessorGroupNode),
    /// Null until a sink is chosen for the node.
    Sink(Option<GraphSinkNode>),
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GraphSourceNode {
    pub item_count: u32,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GraphProcessorGroupNode {
    /// In execution order.
    pub processors: Vec<GraphProcessorSummary>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GraphProcessorSummary {
    pub processor_id: Uuid,
    pub name: String,
    pub r#type: ProcessorKind,
    #[schema(value_type = i16)]
    pub adm_status: AdmStatus,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GraphSinkNode {
    pub sink_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GraphRelation {
    pub from_node_id: Uuid,
    pub to_node_id: Uuid,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "snake_case", tag = "kind", content = "data")]
pub enum GraphNodeContent {
    Source(GraphSourceContent),
    ProcessorGroup(GraphProcessorGroupContent),
    /// Null until a sink is chosen for the node.
    Sink(Option<GraphSinkContent>),
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GraphSourceContent {
    /// One page, ordered by name.
    pub items: Vec<GraphSourceItem>,
    /// All matching items, ignoring limit and offset.
    pub total: u32,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GraphSourceItem {
    pub item_id: Uuid,
    /// The item's own name, or the name of the template item it comes from.
    pub name: Option<String>,
    pub host_id: Uuid,
    pub host_name: String,
    pub r#type: ItemKind,
    #[schema(value_type = i16)]
    pub adm_status: AdmStatus,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GraphProcessorGroupContent {
    /// In execution order.
    pub processors: Vec<GraphProcessor>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GraphProcessor {
    pub processor_id: Uuid,
    pub name: String,
    #[schema(value_type = i16)]
    pub adm_status: AdmStatus,
    pub config: Config,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GraphSinkContent {
    pub sink_id: Uuid,
    pub name: String,
    pub r#type: SinkKind,
    #[schema(value_type = i16)]
    pub adm_status: AdmStatus,
}

/// The content of a node to save. Its kind must be the node's kind.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case", tag = "kind", content = "data")]
pub enum GraphNodeContentInput {
    Source(GraphSourceContentInput),
    ProcessorGroup(GraphProcessorGroupContentInput),
    /// Null empties the node.
    #[serde(deserialize_with = "Option::deserialize")]
    Sink(Option<GraphSinkContentInput>),
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct GraphSourceContentInput {
    /// Items to add. Items already in the source are skipped.
    #[serde(default)]
    pub add: BTreeSet<Uuid>,
    /// Items to remove. Items not in the source are skipped.
    #[serde(default)]
    pub remove: BTreeSet<Uuid>,
}

impl GraphSourceContentInput {
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(id) = self.add.intersection(&self.remove).next() {
            bail!("Item {id} is both added and removed");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct GraphProcessorGroupContentInput {
    /// Every processor of the group, in execution order.
    pub processors: Vec<GraphProcessorInput>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct GraphProcessorInput {
    /// A new id creates the processor, an id of this group updates it.
    pub processor_id: Uuid,
    pub name: String,
    #[serde(default = "default_adm_status")]
    #[schema(default = default_adm_status, value_type = i16)]
    pub adm_status: AdmStatus,
    /// Read with [`parse_processor`].
    #[schema(value_type = Config)]
    pub config: serde_json::Value,
}

/// A processor of a group that passed [`GraphProcessorGroupContentInput::validate`].
#[derive(Debug, Clone)]
pub struct GroupProcessor {
    pub processor_id: Uuid,
    pub name: String,
    pub adm_status: AdmStatus,
    pub config: Config,
}

impl GraphProcessorGroupContentInput {
    /// Checks what can be checked without the database.
    pub fn validate(self) -> anyhow::Result<Vec<GroupProcessor>> {
        let mut ids = HashSet::new();
        self.processors
            .into_iter()
            .map(|input| {
                let id = input.processor_id;
                if !ids.insert(id) {
                    bail!("Duplicate processor id: {id}");
                }
                if input.name.trim().is_empty() {
                    bail!("Processor {id} needs a name");
                }
                let mut processor = parse_processor(&input.config)
                    .with_context(|| format!("Invalid config of processor {id}"))?;
                processor
                    .validate()
                    .with_context(|| format!("Invalid config of processor {id}"))?;
                Ok(GroupProcessor {
                    processor_id: id,
                    name: input.name,
                    adm_status: input.adm_status,
                    config: processor,
                })
            })
            .collect()
    }
}

/// Parses `{ "type": ..., "config": ... }` as if `type` came first.
// A Value keeps its keys sorted, so `config` comes first, serde buffers it,
// and with arbitrary_precision a buffered decimal only parses when it is
// written the shortest way: 1.5 works, 1.50 and 2.5e-3 do not.
pub fn parse_processor(value: &serde_json::Value) -> serde_json::Result<Config> {
    serde_json::from_str(&format!(
        r#"{{"type":{},"config":{}}}"#,
        value["type"], value["config"]
    ))
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct GraphSinkContentInput {
    /// Must be in the pipeline's sink subset.
    pub sink_id: Uuid,
}

/// A graph to save. It has the shape GET returns, so a client can send back
/// what it read after editing it. The `data` of each node is ignored.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PipelineGraphInput {
    pub nodes: Vec<GraphNodeInput>,
    pub relations: Vec<GraphRelation>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct GraphNodeInput {
    pub node_id: Uuid,
    pub position_x: i32,
    pub position_y: i32,
    pub kind: GraphNodeKind,
}

impl PipelineGraphInput {
    /// Checks what can be checked without the database.
    pub fn validate(&self) -> anyhow::Result<()> {
        let mut kinds = HashMap::new();
        for node in &self.nodes {
            if kinds.insert(node.node_id, node.kind).is_some() {
                bail!("Duplicate node id: {}", node.node_id);
            }
        }
        if kinds
            .values()
            .filter(|kind| **kind == GraphNodeKind::Source)
            .count()
            > 1
        {
            bail!("A canvas can have only one source node");
        }

        let mut relations = HashSet::new();
        let mut graph = DiGraphMap::<Uuid, ()>::new();
        for relation in &self.relations {
            let (from, to) = (relation.from_node_id, relation.to_node_id);
            let (Some(from_kind), Some(to_kind)) = (kinds.get(&from), kinds.get(&to)) else {
                bail!("Relation {from} -> {to} points at a node that is not in the graph");
            };
            if from == to {
                bail!("Relation {from} -> {to} connects a node to itself");
            }
            if *from_kind == GraphNodeKind::Sink {
                bail!("Relation {from} -> {to} starts at a sink node");
            }
            if *to_kind == GraphNodeKind::Source {
                bail!("Relation {from} -> {to} ends at a source node");
            }
            if !relations.insert((from, to)) {
                bail!("Duplicate relation {from} -> {to}");
            }
            graph.add_edge(from, to, ());
        }
        if let Err(cycle) = Acyclic::try_from_graph(&graph) {
            bail!("Relations form a cycle through node {}", cycle.node_id());
        }

        Ok(())
    }
}

/// What a canvas holds, as far as the runtime wiring goes.
#[derive(Debug, Clone, Default)]
pub struct CanvasRuntime {
    pub nodes: HashMap<Uuid, RuntimeNode>,
    /// `(from_node_id, to_node_id)`
    pub relations: BTreeSet<(Uuid, Uuid)>,
}

#[derive(Debug, Clone)]
pub enum RuntimeNode {
    Source {
        source: Uuid,
        gate: Uuid,
    },
    /// Processor ids in execution order.
    ProcessorGroup(Vec<Uuid>),
    /// None while the node is empty.
    Sink(Option<Uuid>),
}

impl CanvasRuntime {
    /// Every processor the canvas owns.
    pub fn processor_ids(&self) -> Vec<Uuid> {
        self.nodes
            .values()
            .flat_map(|node| match node {
                RuntimeNode::Source { source, gate } => vec![*source, *gate],
                RuntimeNode::ProcessorGroup(processors) => processors.clone(),
                RuntimeNode::Sink(_) => vec![],
            })
            .collect()
    }
}

/// Runtime rows between processors and into sinks. Items are wired apart,
/// each into the hidden source processor of its node.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuntimeEdges {
    /// `(processor_id, input_id)`
    pub processor_inputs: BTreeSet<(Uuid, Uuid)>,
    /// `(sink_id, input_id)`
    pub sink_inputs: BTreeSet<(Uuid, Uuid)>,
}

/// Wires a canvas: source -> gate only while the pipeline is enabled, the
/// processors of a group one after another, and each relation from the last
/// processor before it to the first processor or sink after it. An empty
/// group hands on whatever reaches it.
pub fn compile(canvas: &CanvasRuntime, enabled: bool) -> RuntimeEdges {
    let mut edges = RuntimeEdges::default();
    let mut inputs: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for (from, to) in &canvas.relations {
        inputs.entry(*to).or_default().push(*from);
    }
    for node in canvas.nodes.values() {
        match node {
            RuntimeNode::Source { source, gate } if enabled => {
                edges.processor_inputs.insert((*gate, *source));
            }
            RuntimeNode::ProcessorGroup(processors) => {
                for pair in processors.windows(2) {
                    edges.processor_inputs.insert((pair[1], pair[0]));
                }
            }
            _ => {}
        }
    }

    let mut outputs = HashMap::new();
    for (from, to) in &canvas.relations {
        let from_outputs = node_outputs(canvas, &inputs, &mut outputs, *from);
        match canvas.nodes.get(to) {
            Some(RuntimeNode::ProcessorGroup(processors)) if !processors.is_empty() => {
                for output in from_outputs {
                    edges.processor_inputs.insert((processors[0], output));
                }
            }
            Some(RuntimeNode::Sink(Some(sink_id))) => {
                for output in from_outputs {
                    edges.sink_inputs.insert((*sink_id, output));
                }
            }
            // an empty group: its successors take its outputs; an empty sink takes nothing
            _ => {}
        }
    }
    edges
}

/// Finds a cycle in `edges` the way the engine does when it loads the
/// pipeline graph.
pub fn find_cycle(edges: &RuntimeEdges) -> Option<Uuid> {
    let mut graph = DiGraphMap::<Uuid, ()>::new();
    for (to, from) in edges.processor_inputs.iter().chain(&edges.sink_inputs) {
        graph.add_edge(*from, *to, ());
    }
    Acyclic::try_from_graph(&graph)
        .err()
        .map(|cycle| cycle.node_id())
}

/// The processors whose output leaves `node_id`.
fn node_outputs(
    canvas: &CanvasRuntime,
    inputs: &HashMap<Uuid, Vec<Uuid>>,
    memo: &mut HashMap<Uuid, BTreeSet<Uuid>>,
    node_id: Uuid,
) -> BTreeSet<Uuid> {
    if let Some(outputs) = memo.get(&node_id) {
        return outputs.clone();
    }
    // stops a cycle in hand-edited rows; saved graphs have none
    memo.insert(node_id, BTreeSet::new());
    let outputs = match canvas.nodes.get(&node_id) {
        Some(RuntimeNode::Source { gate, .. }) => BTreeSet::from([*gate]),
        Some(RuntimeNode::ProcessorGroup(processors)) => match processors.last() {
            Some(last) => BTreeSet::from([*last]),
            None => inputs
                .get(&node_id)
                .into_iter()
                .flatten()
                .flat_map(|input| node_outputs(canvas, inputs, memo, *input))
                .collect(),
        },
        Some(RuntimeNode::Sink(_)) | None => BTreeSet::new(),
    };
    memo.insert(node_id, outputs.clone());
    outputs
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn graph_serializes_kind_and_data_next_to_node_fields() {
        let ids: Vec<Uuid> = (0..6).map(|_| Uuid::now_v7()).collect();
        let graph = PipelineGraph {
            nodes: vec![
                GraphNode {
                    node_id: ids[0],
                    position_x: -5,
                    position_y: 7,
                    data: GraphNodeData::Source(GraphSourceNode { item_count: 3 }),
                },
                GraphNode {
                    node_id: ids[1],
                    position_x: 0,
                    position_y: 0,
                    data: GraphNodeData::ProcessorGroup(GraphProcessorGroupNode {
                        processors: vec![GraphProcessorSummary {
                            processor_id: ids[2],
                            name: "drop x".to_string(),
                            r#type: ProcessorKind::DropAttributes,
                            adm_status: AdmStatus::Disabled,
                        }],
                    }),
                },
                GraphNode {
                    node_id: ids[3],
                    position_x: 1,
                    position_y: 2,
                    data: GraphNodeData::Sink(Some(GraphSinkNode { sink_id: ids[4] })),
                },
            ],
            relations: vec![GraphRelation {
                from_node_id: ids[0],
                to_node_id: ids[1],
            }],
        };

        assert_eq!(
            serde_json::to_value(&graph).unwrap(),
            json!({
                "nodes": [
                    {
                        "node_id": ids[0], "position_x": -5, "position_y": 7,
                        "kind": "source", "data": { "item_count": 3 }
                    },
                    {
                        "node_id": ids[1], "position_x": 0, "position_y": 0,
                        "kind": "processor_group",
                        "data": { "processors": [{
                            "processor_id": ids[2], "name": "drop x",
                            "type": "drop_attributes", "adm_status": 0
                        }] }
                    },
                    {
                        "node_id": ids[3], "position_x": 1, "position_y": 2,
                        "kind": "sink", "data": { "sink_id": ids[4] }
                    }
                ],
                "relations": [{ "from_node_id": ids[0], "to_node_id": ids[1] }]
            })
        );
    }

    #[test]
    fn node_kind_parses_the_stored_column_value() {
        assert_eq!("source".parse(), Ok(GraphNodeKind::Source));
        assert_eq!("processor_group".parse(), Ok(GraphNodeKind::ProcessorGroup));
        assert_eq!("sink".parse(), Ok(GraphNodeKind::Sink));
        assert!("processorgroup".parse::<GraphNodeKind>().is_err());
    }

    #[test]
    fn graph_input_reads_what_get_returns() {
        let [source, group, sink, empty_sink, sink_id, processor] =
            std::array::from_fn(|_| Uuid::now_v7());
        let read = PipelineGraph {
            nodes: vec![
                GraphNode {
                    node_id: source,
                    position_x: 0,
                    position_y: 0,
                    data: GraphNodeData::Source(GraphSourceNode { item_count: 2 }),
                },
                GraphNode {
                    node_id: group,
                    position_x: 1,
                    position_y: -2,
                    data: GraphNodeData::ProcessorGroup(GraphProcessorGroupNode {
                        processors: vec![GraphProcessorSummary {
                            processor_id: processor,
                            name: "p".to_string(),
                            r#type: ProcessorKind::Null,
                            adm_status: AdmStatus::Enabled,
                        }],
                    }),
                },
                GraphNode {
                    node_id: sink,
                    position_x: 9,
                    position_y: 9,
                    data: GraphNodeData::Sink(Some(GraphSinkNode { sink_id })),
                },
                GraphNode {
                    node_id: empty_sink,
                    position_x: 3,
                    position_y: 4,
                    data: GraphNodeData::Sink(None),
                },
            ],
            relations: vec![
                GraphRelation {
                    from_node_id: source,
                    to_node_id: group,
                },
                GraphRelation {
                    from_node_id: group,
                    to_node_id: sink,
                },
            ],
        };
        // what GET returned, plus a new node without data
        let new_group = Uuid::now_v7();
        let mut value = serde_json::to_value(&read).unwrap();
        value["nodes"].as_array_mut().unwrap().push(
            json!({ "node_id": new_group, "position_x": 5, "position_y": 6, "kind": "processor_group" }),
        );
        value["relations"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "from_node_id": source, "to_node_id": new_group }));

        let input: PipelineGraphInput = serde_json::from_value(value).unwrap();
        input.validate().unwrap();
        let nodes: Vec<_> = input
            .nodes
            .iter()
            .map(|node| (node.node_id, node.position_x, node.position_y, node.kind))
            .collect();
        assert_eq!(
            nodes,
            [
                (source, 0, 0, GraphNodeKind::Source),
                (group, 1, -2, GraphNodeKind::ProcessorGroup),
                (sink, 9, 9, GraphNodeKind::Sink),
                (empty_sink, 3, 4, GraphNodeKind::Sink),
                (new_group, 5, 6, GraphNodeKind::ProcessorGroup),
            ]
        );
        let relations: Vec<_> = input
            .relations
            .iter()
            .map(|relation| (relation.from_node_id, relation.to_node_id))
            .collect();
        assert_eq!(
            relations,
            [(source, group), (group, sink), (source, new_group)]
        );
    }

    #[test]
    fn validate_rejects_broken_graphs() {
        let [source, source_2, group_a, group_b, sink, unknown] =
            std::array::from_fn(|_| Uuid::now_v7());
        let node = |id: Uuid, kind: &str| json!({ "node_id": id, "position_x": 0, "position_y": 0, "kind": kind });
        let rel = |from: Uuid, to: Uuid| json!({ "from_node_id": from, "to_node_id": to });
        let nodes = || {
            vec![
                node(source, "source"),
                node(group_a, "processor_group"),
                node(group_b, "processor_group"),
                node(sink, "sink"),
            ]
        };
        let relations = || {
            vec![
                rel(source, group_a),
                rel(group_a, group_b),
                rel(group_b, sink),
                rel(source, sink),
            ]
        };
        let with = |mut list: Vec<serde_json::Value>, extra: serde_json::Value| {
            list.push(extra);
            list
        };
        let input = |nodes: Vec<serde_json::Value>, relations: Vec<serde_json::Value>| {
            serde_json::from_value::<PipelineGraphInput>(
                json!({ "nodes": nodes, "relations": relations }),
            )
            .unwrap()
        };

        // the graph every case breaks is fine on its own
        input(nodes(), relations()).validate().unwrap();

        for (case, nodes, relations, error) in [
            (
                "duplicate node",
                with(nodes(), node(group_a, "processor_group")),
                relations(),
                format!("Duplicate node id: {group_a}"),
            ),
            (
                "two sources",
                with(nodes(), node(source_2, "source")),
                relations(),
                "A canvas can have only one source node".to_string(),
            ),
            (
                "unknown end",
                nodes(),
                with(relations(), rel(group_a, unknown)),
                format!(
                    "Relation {group_a} -> {unknown} points at a node that is not in the graph"
                ),
            ),
            (
                "self loop",
                nodes(),
                with(relations(), rel(group_a, group_a)),
                format!("Relation {group_a} -> {group_a} connects a node to itself"),
            ),
            (
                "out of a sink",
                nodes(),
                with(relations(), rel(sink, group_a)),
                format!("Relation {sink} -> {group_a} starts at a sink node"),
            ),
            (
                "into a source",
                nodes(),
                with(relations(), rel(group_a, source)),
                format!("Relation {group_a} -> {source} ends at a source node"),
            ),
            (
                "duplicate relation",
                nodes(),
                with(relations(), rel(source, group_a)),
                format!("Duplicate relation {source} -> {group_a}"),
            ),
            (
                "cycle",
                nodes(),
                with(relations(), rel(group_b, group_a)),
                "Relations form a cycle through node".to_string(),
            ),
        ] {
            let result = input(nodes, relations).validate();
            let message = format!("{:#}", result.expect_err(case));
            assert!(message.starts_with(&error), "{case}: {message}");
        }
    }

    #[test]
    fn parse_processor_reads_decimals_in_any_form_and_key_order() {
        for number in ["1.5", "1.50", "1E3", "2.5e-3", "123456789.123456789"] {
            let config = format!(
                r#"{{"match":{{"table":"record","key":"x","key_is_regex":false,"op":">","value":{{"float":{number}}}}}}}"#
            );
            for text in [
                format!(r#"{{"type":"attribute_filter","config":{config}}}"#),
                format!(r#"{{"config":{config},"type":"attribute_filter"}}"#),
            ] {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                let processor = parse_processor(&value).unwrap();
                assert!(matches!(processor, Config::AttributeFilter(_)), "{text}");
            }
        }

        // the plain way that parse_processor works around
        let value: serde_json::Value = serde_json::from_str(
            r#"{"type":"attribute_filter","config":{"match":{"table":"record","key":"x","key_is_regex":false,"op":">","value":{"float":1.50}}}}"#,
        )
        .unwrap();
        assert!(serde_json::from_value::<Config>(value).is_err());
    }

    #[test]
    fn compile_wires_chains_passthroughs_and_fan_in() {
        let [s, g1, g2, g3, e1, e2, k, empty_k] = std::array::from_fn(|_| Uuid::now_v7());
        let [src, gate, a, b, c, d, sink_id] = std::array::from_fn(|_| Uuid::now_v7());
        let canvas = |nodes: Vec<(Uuid, RuntimeNode)>, relations: &[(Uuid, Uuid)]| CanvasRuntime {
            nodes: nodes.into_iter().collect(),
            relations: relations.iter().copied().collect(),
        };
        let source = (s, RuntimeNode::Source { source: src, gate });
        let sink = (k, RuntimeNode::Sink(Some(sink_id)));
        let group =
            |id, processors: &[Uuid]| (id, RuntimeNode::ProcessorGroup(processors.to_vec()));
        let edges =
            |processor_inputs: &[(Uuid, Uuid)], sink_inputs: &[(Uuid, Uuid)]| RuntimeEdges {
                processor_inputs: processor_inputs.iter().copied().collect(),
                sink_inputs: sink_inputs.iter().copied().collect(),
            };

        // a chain, then the same chain disabled
        let chain = canvas(
            vec![source.clone(), group(g1, &[a, b]), sink.clone()],
            &[(s, g1), (g1, k)],
        );
        assert_eq!(
            compile(&chain, true),
            edges(&[(gate, src), (a, gate), (b, a)], &[(sink_id, b)])
        );
        assert_eq!(
            compile(&chain, false),
            edges(&[(a, gate), (b, a)], &[(sink_id, b)])
        );

        // empty groups hand on what reaches them, and a second path to the
        // same sink gives the same row once
        let passthrough = canvas(
            vec![
                source.clone(),
                group(e1, &[]),
                group(e2, &[]),
                group(g1, &[a]),
                sink.clone(),
            ],
            &[(s, e1), (e1, e2), (e2, g1), (e1, k), (s, k), (g1, k)],
        );
        assert_eq!(
            compile(&passthrough, true),
            edges(&[(gate, src), (a, gate)], &[(sink_id, gate), (sink_id, a)])
        );

        // fan out and back in
        let diamond = canvas(
            vec![
                source.clone(),
                group(g1, &[a]),
                group(g2, &[b]),
                group(g3, &[c, d]),
                sink.clone(),
            ],
            &[(s, g1), (s, g2), (g1, g3), (g2, g3), (g3, k)],
        );
        assert_eq!(
            compile(&diamond, true),
            edges(
                &[(gate, src), (a, gate), (b, gate), (c, a), (c, b), (d, c)],
                &[(sink_id, d)]
            )
        );

        // nothing reaches a sink behind an empty group with no input, and a
        // group with no input is still chained
        let detached = canvas(
            vec![group(e1, &[]), group(g1, &[a, b]), sink.clone()],
            &[(e1, k)],
        );
        assert_eq!(compile(&detached, true), edges(&[(b, a)], &[]));

        // an empty sink node takes nothing
        let into_empty_sink = canvas(
            vec![
                source.clone(),
                group(g1, &[a]),
                (empty_k, RuntimeNode::Sink(None)),
            ],
            &[(s, g1), (g1, empty_k)],
        );
        assert_eq!(
            compile(&into_empty_sink, true),
            edges(&[(gate, src), (a, gate)], &[])
        );
    }

    #[test]
    fn compile_of_any_valid_canvas_is_acyclic() {
        for seed in 1..=500u64 {
            // xorshift, so every run checks the same canvases
            let mut state = seed;
            let mut next = |n: u64| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state % n
            };

            // nodes in a random order of kinds; relations only go forward in
            // that order, which is what makes them a valid DAG
            let node_count = 2 + next(12);
            let mut nodes = Vec::new();
            for index in 0..node_count {
                let node = match next(3) {
                    _ if index == 0 && next(4) != 0 => RuntimeNode::Source {
                        source: Uuid::now_v7(),
                        gate: Uuid::now_v7(),
                    },
                    0 => RuntimeNode::Sink((next(3) != 0).then(Uuid::now_v7)),
                    _ => {
                        RuntimeNode::ProcessorGroup((0..next(4)).map(|_| Uuid::now_v7()).collect())
                    }
                };
                nodes.push((Uuid::now_v7(), node));
            }
            let mut relations = BTreeSet::new();
            for (i, (from, from_node)) in nodes.iter().enumerate() {
                for (to, to_node) in &nodes[i + 1..] {
                    if !matches!(from_node, RuntimeNode::Sink(_))
                        && !matches!(to_node, RuntimeNode::Source { .. })
                        && next(3) == 0
                    {
                        relations.insert((*from, *to));
                    }
                }
            }
            let canvas = CanvasRuntime {
                nodes: nodes.into_iter().collect(),
                relations,
            };

            let edges = compile(&canvas, next(2) == 0);
            let processors: HashSet<Uuid> = canvas.processor_ids().into_iter().collect();
            let sinks: HashSet<Uuid> = canvas
                .nodes
                .values()
                .filter_map(|node| match node {
                    RuntimeNode::Sink(sink_id) => *sink_id,
                    _ => None,
                })
                .collect();
            let mut graph = DiGraphMap::<Uuid, ()>::new();
            for (processor_id, input_id) in &edges.processor_inputs {
                assert!(processors.contains(processor_id) && processors.contains(input_id));
                graph.add_edge(*input_id, *processor_id, ());
            }
            for (sink_id, input_id) in &edges.sink_inputs {
                assert!(sinks.contains(sink_id) && processors.contains(input_id));
                graph.add_edge(*input_id, *sink_id, ());
            }
            assert!(
                Acyclic::try_from_graph(&graph).is_ok(),
                "seed {seed}: {canvas:?} -> {edges:?}"
            );
        }
    }

    #[test]
    fn find_cycle_finds_cycles_across_both_tables() {
        let [a, b, c, k] = std::array::from_fn(|_| Uuid::now_v7());
        let edges =
            |processor_inputs: &[(Uuid, Uuid)], sink_inputs: &[(Uuid, Uuid)]| RuntimeEdges {
                processor_inputs: processor_inputs.iter().copied().collect(),
                sink_inputs: sink_inputs.iter().copied().collect(),
            };

        // b reads a, c reads b, sink k reads c
        assert_eq!(find_cycle(&edges(&[(b, a), (c, b)], &[(k, c)])), None);
        let found = find_cycle(&edges(&[(b, a), (c, b), (a, c)], &[]));
        assert!(found.is_some_and(|id| [a, b, c].contains(&id)), "{found:?}");
        // sink rows are edges of the same graph, as in the engine
        assert!(find_cycle(&edges(&[(k, c)], &[(c, k)])).is_some());
    }
}
