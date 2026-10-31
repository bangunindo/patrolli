// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::cluster::inter::v1::{CaptureMessage, CaptureRequest};
use crate::config::{processor, sink};
use anyhow::Result;
use arc_swap::{ArcSwap, Guard};
use parking_lot::{Mutex, RawMutex, RwLock, lock_api::MutexGuard};
use petgraph::acyclic::Acyclic;
use petgraph::prelude::DiGraphMap;
use rustc_hash::FxHashMap;
use std::collections::BTreeSet;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, LazyLock, Weak};
use tokio::sync::broadcast::{Receiver, Sender, error::RecvError};
use tracing::{info, warn};
use uuid::Uuid;

pub(super) static PROCESSOR_CACHE: LazyLock<Arc<Processor>> =
    LazyLock::new(|| Arc::new(Processor::new()));

#[derive(Debug, Clone)]
pub enum Node {
    Processor(Arc<processor::Processor>),
    Sink(Arc<sink::Sink>),
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Node::Processor(a), Node::Processor(b)) => a.id == b.id,
            (Node::Sink(a), Node::Sink(b)) => a.id == b.id,
            _ => false,
        }
    }
}

impl Eq for Node {}

impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Node::Processor(a), Node::Processor(b)) => a.id.cmp(&b.id),
            (Node::Sink(a), Node::Sink(b)) => a.id.cmp(&b.id),
            (Node::Sink(_), Node::Processor(_)) => std::cmp::Ordering::Less,
            (Node::Processor(_), Node::Sink(_)) => std::cmp::Ordering::Greater,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Step {
    pub node: Node,
    pub next: Vec<Step>,
}

impl Step {
    pub fn id(&self) -> Uuid {
        match &self.node {
            Node::Processor(p) => p.id,
            Node::Sink(s) => s.id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProcessorData {
    sinks: FxHashMap<Uuid, Arc<sink::Sink>>,
    processors: FxHashMap<Uuid, (Arc<processor::Processor>, Step)>,
    by_input: FxHashMap<Uuid, BTreeSet<Node>>,
    graph: DiGraphMap<Uuid, ()>,
    no_input_steps: Vec<Step>,
}

impl ProcessorData {
    fn new() -> Self {
        Self {
            sinks: FxHashMap::default(),
            processors: FxHashMap::default(),
            by_input: FxHashMap::default(),
            graph: Default::default(),
            no_input_steps: vec![],
        }
    }

    fn resolve_routes_inner(&self, processor: &processor::Processor) -> Vec<Step> {
        match self.by_input.get(&processor.id) {
            // if the processor is not connected to any other processor, drop it
            None => Vec::new(),
            Some(nodes) => nodes
                .iter()
                .map(|node| Step {
                    node: node.clone(),
                    next: if let Node::Processor(processor) = node {
                        self.resolve_routes_inner(processor)
                    } else {
                        Vec::new()
                    },
                })
                .collect(),
        }
    }

    fn compile(&mut self) -> Result<()> {
        _ = Acyclic::try_from_graph(&self.graph)
            .map_err(|cycle| anyhow::anyhow!("cyclic node found: {}", cycle.node_id()))?;
        self.no_input_steps = self
            .sinks
            .values()
            .filter(|sink| sink.inputs.is_empty())
            .cloned()
            .map(|sink| Step {
                node: Node::Sink(sink),
                next: Vec::new(),
            })
            .collect();
        // build processor routes
        let mut map = std::mem::take(&mut self.processors);
        for (processor, step) in map.values_mut() {
            let new_step = self.resolve_routes_inner(processor).clone();
            step.next = new_step;
        }
        self.processors = map;
        self.warn_dangling();
        Ok(())
    }

    /// Log-only: reports graph shapes that compile accepts but that silently
    /// lose data, without changing the compiled routes.
    fn warn_dangling(&self) {
        for (input, nodes) in &self.by_input {
            if nodes.is_empty() || self.processors.contains_key(input) {
                continue;
            }
            for node in nodes {
                match node {
                    Node::Processor(p) => warn!(
                        processor_id = %p.id,
                        processor = %p.name,
                        input_id = %input,
                        "processor input references unknown processor"
                    ),
                    Node::Sink(s) => warn!(
                        sink_id = %s.id,
                        sink = %s.name,
                        input_id = %input,
                        "sink input references unknown processor"
                    ),
                }
            }
        }
        for (id, (processor, _)) in &self.processors {
            if self.by_input.get(id).is_none_or(|nodes| nodes.is_empty()) {
                warn!(
                    processor_id = %id,
                    processor = %processor.name,
                    "processor has no downstream nodes, its output is dropped"
                );
            }
        }
    }

    pub fn clear(&mut self) {
        *self = Self::new();
    }

    pub fn remove_sink(&mut self, sink_id: Uuid) {
        if let Some(sink) = self.sinks.remove(&sink_id) {
            for input in &sink.inputs {
                self.by_input
                    .entry(*input)
                    .or_default()
                    .remove(&Node::Sink(sink.clone()));
                self.graph.remove_edge(*input, sink.id);
            }
        }
    }
    pub fn insert_sink(&mut self, sink: sink::Sink) {
        let sink = Arc::new(sink);
        if let Some(existing) = self.sinks.get(&sink.id) {
            for input in &existing.inputs {
                self.by_input
                    .entry(*input)
                    .or_default()
                    .remove(&Node::Sink(existing.clone()));
                self.graph.remove_edge(*input, existing.id);
            }
        }
        for input in &sink.inputs {
            self.by_input
                .entry(*input)
                .or_default()
                .replace(Node::Sink(sink.clone()));
            self.graph.add_edge(*input, sink.id, ());
        }
        self.sinks.insert(sink.id, sink);
    }

    pub fn remove_processor(&mut self, processor_id: Uuid) {
        if let Some((processor, _)) = self.processors.remove(&processor_id) {
            for input in &processor.inputs {
                self.by_input
                    .entry(*input)
                    .or_default()
                    .remove(&Node::Processor(processor.clone()));
                self.graph.remove_edge(*input, processor.id);
            }
        }
    }

    pub fn insert_processor(&mut self, processor: processor::Processor) {
        let processor = Arc::new(processor);
        if let Some((existing, _)) = self.processors.get(&processor.id) {
            for input in &existing.inputs {
                self.by_input
                    .entry(*input)
                    .or_default()
                    .remove(&Node::Processor(existing.clone()));
                self.graph.remove_edge(*input, existing.id);
            }
        }
        for input in &processor.inputs {
            self.by_input
                .entry(*input)
                .or_default()
                .replace(Node::Processor(processor.clone()));
            self.graph.add_edge(*input, processor.id, ());
        }
        let step = Step {
            node: Node::Processor(processor.clone()),
            next: Vec::new(),
        };
        self.processors.insert(processor.id, (processor, step));
    }

    pub fn get_no_input_steps(&self) -> &[Step] {
        self.no_input_steps.as_ref()
    }

    pub fn get_step(&self, processor_id: Uuid) -> Option<&Step> {
        self.processors.get(&processor_id).map(|(_, step)| step)
    }

    /// The id of every configured processor node — also the set of live
    /// per-node counter identities, which is what lets
    /// [`crate::cache::lookup_table::metrics::snapshot`] drop the ones a
    /// deleted node left behind instead of rendering them forever.
    pub fn node_ids(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.processors.keys().copied()
    }

    /// The id of every configured sink. Together with [`Self::node_ids`] this
    /// is the full set of identities the processor keeps per-node counters
    /// under, since a sink is a node of the routing graph too.
    pub fn sink_ids(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.sinks.keys().copied()
    }

    /// Every configured edge whose ends both still exist.
    ///
    /// Removing a node only removes the edges into it; the ones out of it stay
    /// in the graph until each downstream node is re-inserted, so both ends are
    /// checked rather than trusting the graph alone.
    pub fn edge_ids(&self) -> impl Iterator<Item = (Uuid, Uuid)> + '_ {
        self.graph.all_edges().filter_map(|(source, target, _)| {
            let live = |id: Uuid| self.processors.contains_key(&id) || self.sinks.contains_key(&id);
            (live(source) && live(target)).then_some((source, target))
        })
    }
}

pub struct Capture {
    pub sender: Sender<CaptureMessage>,
    pub req: CaptureRequest,
    pub last_send: AtomicU64,
}

impl Capture {
    pub fn new(req: CaptureRequest) -> Self {
        Self {
            sender: Sender::new(16),
            req,
            last_send: AtomicU64::new(0),
        }
    }
}

/// A live subscription to a processor capture.
///
/// Dropping the last subscription removes its capture entry immediately. The
/// weak capture reference prevents an old subscription from removing a newer
/// entry for the same processor.
pub struct CaptureSubscription {
    receiver: Option<Receiver<CaptureMessage>>,
    processor: Weak<Processor>,
    capture: Weak<Capture>,
    processor_id: Uuid,
}

impl CaptureSubscription {
    pub async fn recv(&mut self) -> Result<CaptureMessage, RecvError> {
        self.receiver
            .as_mut()
            .expect("capture subscription receiver is missing")
            .recv()
            .await
    }
}

impl Drop for CaptureSubscription {
    fn drop(&mut self) {
        // The receiver count must be updated before deciding whether this was
        // the last subscriber.
        drop(self.receiver.take());
        let Some(processor) = self.processor.upgrade() else {
            return;
        };
        let Some(capture) = self.capture.upgrade() else {
            return;
        };
        processor.remove_capture_if_inactive(self.processor_id, &capture);
    }
}

pub struct Processor {
    draft: Mutex<Option<ProcessorData>>,
    finalized: ArcSwap<ProcessorData>,
    pub captures: RwLock<FxHashMap<Uuid, Arc<Capture>>>,
}

type ProcessorWriteGuard<'a> = MutexGuard<'a, RawMutex, Option<ProcessorData>>;
type ProcessorReadGuard = Guard<Arc<ProcessorData>>;

impl Default for Processor {
    fn default() -> Self {
        Self::new()
    }
}

impl Processor {
    pub fn new() -> Self {
        Self {
            draft: Mutex::new(None),
            finalized: ArcSwap::from_pointee(ProcessorData::new()),
            captures: RwLock::new(FxHashMap::default()),
        }
    }

    pub fn len(&self) -> usize {
        self.finalized.load().processors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.finalized.load().processors.is_empty()
    }

    pub fn write(&'_ self) -> ProcessorWriteGuard<'_> {
        let mut guard = self.draft.lock();
        let data = self.finalized.load();
        *guard = Some(data.as_ref().clone());
        guard
    }

    pub fn read(&self) -> ProcessorReadGuard {
        self.finalized.load()
    }

    /// Subscribe to the live-capture feed of a processor, creating the capture
    /// entry (which makes processors start capturing) if it doesn't exist yet.
    /// When a capture is already active, the existing entry's `req` wins.
    /// Dropping the last returned subscription removes the capture entry.
    /// Returns `None` when the processor is unknown.
    pub fn subscribe_capture(
        self: &Arc<Self>,
        processor_id: Uuid,
        req: &CaptureRequest,
    ) -> Option<CaptureSubscription> {
        self.read().get_step(processor_id)?;
        let mut captures = self.captures.write();
        let capture = captures
            .entry(processor_id)
            .and_modify(|capture| {
                // A zero-receiver entry can be left by older code or by a
                // sender racing the final subscription's Drop implementation.
                // Treat it as inactive so the new request takes effect.
                if capture.sender.receiver_count() == 0 {
                    *capture = Arc::new(Capture::new(req.clone()));
                }
            })
            .or_insert_with(|| Arc::new(Capture::new(req.clone())))
            .clone();
        let receiver = capture.sender.subscribe();
        Some(CaptureSubscription {
            receiver: Some(receiver),
            processor: Arc::downgrade(self),
            capture: Arc::downgrade(&capture),
            processor_id,
        })
    }

    pub(crate) fn remove_capture_if_inactive(&self, processor_id: Uuid, expected: &Arc<Capture>) {
        let mut captures = self.captures.write();
        let should_remove = captures.get(&processor_id).is_some_and(|capture| {
            Arc::ptr_eq(capture, expected) && capture.sender.receiver_count() == 0
        });
        if should_remove {
            captures.remove(&processor_id);
        }
    }

    pub fn compile(&self, mut draft: ProcessorWriteGuard) -> Result<()> {
        let draft = draft.take();
        let Some(mut draft) = draft else {
            return Ok(());
        };
        draft.compile()?;
        let (processors, sinks, edges) = (
            draft.processors.len(),
            draft.sinks.len(),
            draft.graph.edge_count(),
        );
        self.finalized.store(Arc::new(draft));
        info!(processors, sinks, edges, "processor graph compiled");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Processor;
    use crate::cluster::inter::v1::CaptureRequest;
    use crate::config::processor::{Config, Processor as ProcessorConfig};
    use std::sync::Arc;
    use uuid::Uuid;

    fn configured_processor() -> (Arc<Processor>, Uuid) {
        let processor_id = Uuid::now_v7();
        let cache = Arc::new(Processor::new());
        let mut draft = cache.write();
        draft.as_mut().unwrap().insert_processor(ProcessorConfig {
            id: processor_id,
            name: "capture-test".to_string(),
            config: Config::Null,
            inputs: Vec::new(),
        });
        cache.compile(draft).unwrap();
        (cache, processor_id)
    }

    fn request(processor_id: Uuid, max_count: u64) -> CaptureRequest {
        CaptureRequest {
            processor_id: processor_id.as_bytes().to_vec().into(),
            max_count,
        }
    }

    fn null_processor(id: Uuid, inputs: Vec<Uuid>) -> ProcessorConfig {
        ProcessorConfig {
            id,
            name: id.to_string(),
            config: Config::Null,
            inputs,
        }
    }

    #[test]
    fn reinserting_a_processor_drops_the_inputs_it_no_longer_has() {
        let cache = Arc::new(Processor::new());
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let mut draft = cache.write();
        {
            let data = draft.as_mut().unwrap();
            data.insert_processor(null_processor(a, vec![]));
            data.insert_processor(null_processor(b, vec![a]));
        }
        cache.compile(draft).unwrap();
        assert_eq!(cache.read().edge_ids().collect::<Vec<_>>(), vec![(a, b)]);
        assert_eq!(cache.read().get_step(a).unwrap().next.len(), 1);

        // b is reconfigured without its input: the route a -> b must go
        let mut draft = cache.write();
        draft
            .as_mut()
            .unwrap()
            .insert_processor(null_processor(b, vec![]));
        cache.compile(draft).unwrap();
        assert!(cache.read().edge_ids().next().is_none());
        assert!(cache.read().get_step(a).unwrap().next.is_empty());
    }

    #[test]
    fn edges_out_of_a_removed_processor_are_not_live() {
        let cache = Arc::new(Processor::new());
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let mut draft = cache.write();
        {
            let data = draft.as_mut().unwrap();
            data.insert_processor(null_processor(a, vec![]));
            data.insert_processor(null_processor(b, vec![a]));
            data.remove_processor(a);
        }
        cache.compile(draft).unwrap();
        let guard = cache.read();
        assert_eq!(guard.node_ids().collect::<Vec<_>>(), vec![b]);
        assert!(guard.edge_ids().next().is_none());
    }

    #[test]
    fn capture_entry_is_removed_after_last_subscription_drops() {
        let (cache, processor_id) = configured_processor();
        let req = request(processor_id, 10);
        let first = cache.subscribe_capture(processor_id, &req).unwrap();
        let second = cache.subscribe_capture(processor_id, &req).unwrap();

        assert_eq!(
            cache
                .captures
                .read()
                .get(&processor_id)
                .unwrap()
                .sender
                .receiver_count(),
            2
        );

        drop(first);
        assert!(cache.captures.read().contains_key(&processor_id));

        drop(second);
        assert!(!cache.captures.read().contains_key(&processor_id));
    }

    #[test]
    fn old_subscription_does_not_remove_replacement_capture() {
        let (cache, processor_id) = configured_processor();
        let old_req = request(processor_id, 10);
        let old_subscription = cache.subscribe_capture(processor_id, &old_req).unwrap();
        let old_capture = cache.captures.write().remove(&processor_id).unwrap();

        let new_req = request(processor_id, 20);
        let new_subscription = cache.subscribe_capture(processor_id, &new_req).unwrap();
        drop(old_subscription);

        let captures = cache.captures.read();
        let current = captures.get(&processor_id).unwrap();
        assert!(!Arc::ptr_eq(current, &old_capture));
        assert_eq!(current.req.max_count, 20);
        drop(captures);

        drop(new_subscription);
        assert!(!cache.captures.read().contains_key(&processor_id));
    }
}
