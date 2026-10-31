// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Drives [`Worker`] over a real processor graph: a [`cache::Cache`] of its
//! own, and one memory topic per sink that the test reads back.
use super::*;
use crate::chan::AdminChannel;
use crate::cluster::inter::v1::CaptureRequest;
use crate::config::processor::{AttributeFilter, AttributeFilterExpr, AttributeTable, KeyValueOp};
use crate::config::{self, sink};
use crate::processor::test_payloads::{logs, signals};
use arc_swap::ArcSwap;
use payload::{Payload, ProcessorPayload};

const SINK_BUFFER: usize = 64;

struct Harness {
    cache: cache::Cache,
    admin: AdminChannel<Uuid, Payload>,
    worker: Worker,
    topics: FxHashMap<Uuid, Receiver<Payload>>,
}

impl Harness {
    fn new() -> Self {
        let remote = Arc::new(ArcSwap::from_pointee((Instant::now(), Default::default())));
        let (admin, sender) = chan::new(Duration::from_secs(60), remote);
        let cache = cache::Cache::new();
        // The tests hand the worker its input through `process_records`.
        let (_, receiver) = flume::bounded(1);
        let worker = Worker {
            idx: 0,
            internal_metrics: Arc::default(),
            sender,
            receiver,
            config_cache: cache.clone(),
            stop_waiting: CancellationToken::new(),
            dropped_on_stop: Cell::new(0),
            transforms: Default::default(),
        };
        Self {
            cache,
            admin,
            worker,
            topics: FxHashMap::default(),
        }
    }

    fn graph(&self, f: impl FnOnce(&mut cache::processor::ProcessorData)) {
        let mut draft = self.cache.processor.write();
        f(draft.as_mut().unwrap());
        self.cache.processor.compile(draft).unwrap();
    }

    /// A processor running `config` (validated) on what `inputs` emit.
    fn processor(&self, config: Config, inputs: &[Uuid]) -> Uuid {
        let mut processor = Processor {
            id: Uuid::now_v7(),
            name: "p".into(),
            config,
            inputs: inputs.to_vec(),
        };
        processor.validate().unwrap();
        let id = processor.id;
        self.graph(|g| g.insert_processor(processor));
        id
    }

    /// A sink in the graph, without a topic: sends to it fail.
    fn sink_without_topic(&self, inputs: &[Uuid]) -> Uuid {
        let id = Uuid::now_v7();
        let inputs: Vec<String> = inputs.iter().map(Uuid::to_string).collect();
        let conf: sink::Sink = serde_yaml_ng::from_str(&format!(
            "id: {id}\nname: s\ninputs: [{}]\ntype: file\nconfig:\n  path: /dev/null\n",
            inputs.join(", ")
        ))
        .unwrap();
        self.graph(|g| g.insert_sink(conf));
        id
    }

    async fn sink_with_buffer(&mut self, inputs: &[Uuid], buffer: usize) -> Uuid {
        let id = self.sink_without_topic(inputs);
        let conf = config::Channel::Memory(config::MemoryChannel {
            non_blocking: false,
            channel_buffer_size: buffer,
        });
        let rx = self.admin.get_receiver(&id, conf).await.unwrap();
        self.topics.insert(id, rx);
        id
    }

    async fn sink(&mut self, inputs: &[Uuid]) -> Uuid {
        self.sink_with_buffer(inputs, SINK_BUFFER).await
    }

    fn send(&self, processor_id: Option<Uuid>, pl: Payload) {
        self.worker
            .process_records(ProcessorPayload::new(processor_id, pl));
    }

    /// What `sink` was sent since the last call, as items per payload.
    fn received(&self, sink: Uuid) -> Vec<usize> {
        self.topics[&sink]
            .try_iter()
            .map(|pl| pl.num_items())
            .collect()
    }

    fn metrics(&self) -> ProcessorMetrics {
        self.worker.internal_metrics.get()
    }

    /// `(total_records, total_records_filtered, total_records_errored)` of a
    /// node, zeros for one never touched.
    fn node(&self, id: Uuid) -> (u64, u64, u64) {
        self.metrics()
            .nodes
            .into_iter()
            .find(|node| node.id == id)
            .map_or((0, 0, 0), |node| {
                (
                    node.total_records,
                    node.total_records_filtered,
                    node.total_records_errored,
                )
            })
    }

    fn edge(&self, source: Uuid, target: Uuid) -> Option<u64> {
        self.metrics()
            .edges
            .into_iter()
            .find(|edge| edge.source == source && edge.target == target)
            .map(|edge| edge.total_records)
    }
}

fn filter(op: KeyValueOp, value: &str) -> Config {
    Config::AttributeFilter(AttributeFilter::Match(AttributeFilterExpr {
        table: AttributeTable::Resource,
        key: "host".into(),
        key_is_regex: false,
        op,
        value: config::processor::Value::String(value.into()),
    }))
}

#[tokio::test]
async fn fan_out_counts_input_once_and_sends_once_per_sink() {
    let mut h = Harness::new();
    let p = h.processor(Config::Null, &[]);
    let (s1, s2) = (h.sink(&[p]).await, h.sink(&[p]).await);

    h.send(Some(p), logs(5));

    assert_eq!(h.received(s1), [5]);
    assert_eq!(h.received(s2), [5]);
    let m = h.metrics();
    assert_eq!(m.total_records, 5);
    assert_eq!(m.total_records_sent, 10);
    assert_eq!(m.total_records_errored, 0);
    assert_eq!(m.total_records_unrouted, 0);
    assert_eq!(h.node(p), (5, 0, 0));
    assert_eq!((h.edge(p, s1), h.edge(p, s2)), (Some(5), Some(5)));
    assert_eq!((h.node(s1), h.node(s2)), ((5, 0, 0), (5, 0, 0)));
}

#[tokio::test]
async fn chain_processor_to_processor_to_sink_delivers_and_counts_edges() {
    let mut h = Harness::new();
    let p1 = h.processor(Config::Null, &[]);
    let p2 = h.processor(Config::Null, &[p1]);
    let s = h.sink(&[p2]).await;

    h.send(Some(p1), logs(3));

    assert_eq!(h.received(s), [3]);
    assert_eq!((h.node(p1), h.node(p2)), ((3, 0, 0), (3, 0, 0)));
    assert_eq!((h.edge(p1, p2), h.edge(p2, s)), (Some(3), Some(3)));
    assert_eq!(h.metrics().total_records_sent, 3);
}

/// A diamond: what reaches one sink down two paths is sent to it as one
/// batch, once per path's worth of records.
#[tokio::test]
async fn fan_in_two_processors_into_one_sink_batches_both() {
    let mut h = Harness::new();
    let p = h.processor(Config::Null, &[]);
    let (a, b) = (
        h.processor(Config::Null, &[p]),
        h.processor(Config::Null, &[p]),
    );
    let s = h.sink(&[a, b]).await;

    h.send(Some(p), logs(4));

    assert_eq!(h.received(s), [8]);
    assert_eq!((h.edge(a, s), h.edge(b, s)), (Some(4), Some(4)));
    assert_eq!(h.node(s), (8, 0, 0));
    let m = h.metrics();
    assert_eq!((m.total_records, m.total_records_sent), (4, 8));
}

#[tokio::test]
async fn every_signal_is_routed() {
    let mut h = Harness::new();
    let p = h.processor(Config::Null, &[]);
    let s = h.sink(&[p]).await;
    for pl in signals() {
        h.send(Some(p), pl);
    }
    assert_eq!(h.received(s), [2, 2, 2]);
}

#[tokio::test]
async fn attribute_filter_drop_all_counts_filtered_and_stops_routing() {
    let mut h = Harness::new();
    let p = h.processor(filter(KeyValueOp::Equal, "nope"), &[]);
    let s = h.sink(&[p]).await;

    for pl in signals() {
        h.send(Some(p), pl);
    }

    assert!(h.received(s).is_empty());
    assert_eq!(h.node(p), (6, 6, 0));
    assert_eq!(h.edge(p, s), None);
    let m = h.metrics();
    assert_eq!(
        (
            m.total_records,
            m.total_records_filtered,
            m.total_records_sent
        ),
        (6, 6, 0)
    );
}

#[tokio::test]
async fn attribute_filter_passes_what_matches() {
    let mut h = Harness::new();
    let p = h.processor(filter(KeyValueOp::Equal, "a"), &[]);
    let s = h.sink(&[p]).await;
    h.send(Some(p), signals().remove(0));
    assert_eq!(h.received(s), [2]);
    assert_eq!(h.node(p), (2, 0, 0));
}

#[tokio::test]
async fn invalid_regex_filter_drops_batch_counts_errored() {
    let mut h = Harness::new();
    let p = h.processor(filter(KeyValueOp::RegexEqual, "("), &[]);
    let s = h.sink(&[p]).await;

    h.send(Some(p), signals().remove(0));

    assert!(h.received(s).is_empty());
    assert_eq!(h.node(p), (2, 0, 2));
    assert_eq!(h.metrics().total_records_errored, 2);
}

#[tokio::test]
async fn transform_filtered_records_are_counted() {
    let mut h = Harness::new();
    let transform = serde_json::from_value(serde_json::json!({
        "opl": r#"logs | where resource.attributes["host"] == "b""#,
    }))
    .unwrap();
    let p = h.processor(Config::Transform(transform), &[]);
    let s = h.sink(&[p]).await;

    h.send(Some(p), signals().remove(0));

    assert!(h.received(s).is_empty());
    assert_eq!(h.node(p), (2, 2, 0));
}

/// #41: a processor whose transforms were never compiled passes the batch on
/// unchanged, so nothing is lost and nothing is errored.
#[tokio::test]
async fn uninitialized_transforms_pass_through_without_errored() {
    let mut h = Harness::new();
    let p = Uuid::now_v7();
    let config: Config = serde_json::from_value(serde_json::json!({
        "type": "drop_attributes",
        "config": {"keys": {"resource": ["host"]}},
    }))
    .unwrap();
    // inserted without `validate`, which is what compiles the transforms
    h.graph(|g| {
        g.insert_processor(Processor {
            id: p,
            name: "p".into(),
            config,
            inputs: vec![],
        })
    });
    let s = h.sink(&[p]).await;

    h.send(Some(p), signals().remove(0));

    assert_eq!(h.received(s), [2]);
    assert_eq!(h.node(p), (2, 0, 0));
    assert_eq!(h.metrics().total_records_errored, 0);
}

#[tokio::test]
async fn processor_without_downstream_drops_output() {
    let mut h = Harness::new();
    let p = h.processor(Config::Null, &[]);
    let other = h.processor(Config::Null, &[]);
    let s = h.sink(&[other]).await;

    h.send(Some(p), logs(2));

    assert!(h.received(s).is_empty());
    assert_eq!(h.node(p), (2, 0, 0));
    let m = h.metrics();
    assert_eq!(m.total_records_sent, 0);
    assert_eq!(m.total_records_unrouted, 2);
    assert_eq!(m.total_records_errored, 0);
}

/// Counted per drop, like `total_records_sent` per sink: output fanned out to
/// two dead ends is lost twice.
#[tokio::test]
async fn fan_out_to_two_dead_ends_counts_each() {
    let mut h = Harness::new();
    let p = h.processor(Config::Null, &[]);
    let (end1, end2) = (
        h.processor(Config::Null, &[p]),
        h.processor(Config::Null, &[p]),
    );
    let s = h.sink(&[p]).await;

    h.send(Some(p), logs(3));

    assert_eq!(h.received(s), [3]);
    assert_eq!((h.node(end1), h.node(end2)), ((3, 0, 0), (3, 0, 0)));
    let m = h.metrics();
    assert_eq!(m.total_records_sent, 3);
    assert_eq!(m.total_records_unrouted, 6);
}

/// A processor id the cache does not hold, as when a processor is deleted while
/// its batches are still queued.
#[tokio::test]
async fn unknown_processor_id_counts_unrouted() {
    let mut h = Harness::new();
    let p = h.processor(Config::Null, &[]);
    let s = h.sink(&[p]).await;

    h.send(Some(Uuid::now_v7()), logs(4));

    assert!(h.received(s).is_empty());
    let m = h.metrics();
    assert_eq!(m.total_records, 4);
    assert_eq!(m.total_records_unrouted, 4);
    assert_eq!(m.total_records_errored, 0);
}

/// Records with no processor go to the sinks that take no input, and only to
/// them.
#[tokio::test]
async fn no_processor_id_goes_only_to_input_less_sinks() {
    let mut h = Harness::new();
    let p = h.processor(Config::Null, &[]);
    let fed = h.sink(&[p]).await;
    let (open1, open2) = (h.sink(&[]).await, h.sink(&[]).await);

    h.send(None, logs(3));

    assert!(h.received(fed).is_empty());
    assert_eq!((h.received(open1), h.received(open2)), (vec![3], vec![3]));
    assert_eq!(h.node(p), (0, 0, 0));
    assert_eq!(h.metrics().total_records_sent, 6);
    assert_eq!(h.metrics().total_records_unrouted, 0);
}

#[tokio::test]
async fn no_processor_id_without_input_less_sinks_counts_unrouted() {
    let mut h = Harness::new();
    let p = h.processor(Config::Null, &[]);
    let fed = h.sink(&[p]).await;

    h.send(None, logs(3));

    assert!(h.received(fed).is_empty());
    let m = h.metrics();
    assert_eq!(m.total_records_sent, 0);
    assert_eq!(m.total_records_unrouted, 3);
}

#[tokio::test]
async fn empty_payload_is_not_routed() {
    let mut h = Harness::new();
    let p = h.processor(Config::Null, &[]);
    let s = h.sink(&[p]).await;

    h.send(Some(p), logs(0));

    assert!(h.received(s).is_empty());
    assert_eq!(h.node(p), (0, 0, 0));
}

#[tokio::test]
async fn send_to_removed_sink_counts_errored() {
    let mut h = Harness::new();
    let p = h.processor(Config::Null, &[]);
    let gone = h.sink_without_topic(&[p]);
    let live = h.sink(&[p]).await;

    h.send(Some(p), logs(2));

    assert_eq!(h.received(live), [2]);
    assert_eq!(h.node(gone), (2, 0, 2));
    let m = h.metrics();
    assert_eq!((m.total_records_sent, m.total_records_errored), (2, 2));
}

/// Once shutdown stops waiting, a full memory topic turns the batch away:
/// it is errored, and remembered for the worker's one stop warning.
#[tokio::test]
async fn full_topic_after_stop_waiting_counts_dropped_and_errored() {
    let mut h = Harness::new();
    let p = h.processor(Config::Null, &[]);
    let s = h.sink_with_buffer(&[p], 1).await;
    h.send(Some(p), logs(1));

    h.worker.stop_waiting.cancel();
    h.send(Some(p), logs(4));

    assert_eq!(h.received(s), [1]);
    assert_eq!(h.worker.dropped_on_stop.get(), 4);
    assert_eq!(h.node(s), (5, 0, 4));
    let m = h.metrics();
    assert_eq!((m.total_records_sent, m.total_records_errored), (1, 4));
}

/// The capture gate only opens once a second of uptime has passed.
async fn wait_for_capture_gate() {
    while common::uptime() < Duration::from_millis(1100) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn capture_request(processor_id: Uuid, max_count: u64) -> CaptureRequest {
    CaptureRequest {
        processor_id: processor_id.as_bytes().to_vec().into(),
        max_count,
    }
}

/// A capture takes a sample of at most `max_count` records at most once a
/// second, and every record still reaches the sink.
#[tokio::test]
async fn capture_sends_a_sample_once_per_second_and_routes_every_record() {
    wait_for_capture_gate().await;
    let mut h = Harness::new();
    let p = h.processor(Config::Null, &[]);
    let s = h.sink(&[p]).await;
    let mut sub = h
        .cache
        .processor
        .subscribe_capture(p, &capture_request(p, 2))
        .unwrap();

    h.send(Some(p), logs(5));
    h.send(Some(p), logs(5));

    let message = sub.recv().await.unwrap();
    assert!(message.message_out.is_some());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), sub.recv())
            .await
            .is_err(),
        "a second sample within the same second"
    );
    assert_eq!(h.received(s).iter().sum::<usize>(), 10);
    assert_eq!(h.node(p), (10, 0, 0));
}

#[tokio::test]
async fn capture_when_processor_drops_batch_has_no_output() {
    wait_for_capture_gate().await;
    let mut h = Harness::new();
    let p = h.processor(filter(KeyValueOp::RegexEqual, "("), &[]);
    let s = h.sink(&[p]).await;
    let mut sub = h
        .cache
        .processor
        .subscribe_capture(p, &capture_request(p, 1))
        .unwrap();

    h.send(Some(p), signals().remove(0));

    let message = sub.recv().await.unwrap();
    assert!(message.message_out.is_none());
    assert!(h.received(s).is_empty());
    assert_eq!(h.node(p), (2, 0, 2));
}

/// An entry nobody listens to any more (its last subscriber raced the send)
/// is removed by the worker whose send found no receiver.
#[tokio::test]
async fn capture_with_dropped_subscriber_removes_entry() {
    wait_for_capture_gate().await;
    let mut h = Harness::new();
    let p = h.processor(Config::Null, &[]);
    let s = h.sink(&[p]).await;
    h.cache
        .processor
        .captures
        .write()
        .insert(p, Arc::new(Capture::new(capture_request(p, 1))));

    h.send(Some(p), logs(3));

    assert!(!h.cache.processor.captures.read().contains_key(&p));
    assert_eq!(h.received(s).iter().sum::<usize>(), 3);
}

/// #9: the sample and the rest used to be merged back into one payload, which
/// panicked the worker once together they held more than a payload can.
#[tokio::test]
async fn capture_with_rebatch_over_u16_does_not_panic() {
    wait_for_capture_gate().await;
    let mut h = Harness::new();
    let p = h.processor(Config::Null, &[]);
    let s = h.sink(&[p]).await;
    let mut sub = h
        .cache
        .processor
        .subscribe_capture(p, &capture_request(p, 10))
        .unwrap();
    let n = u16::MAX as usize + 10;

    h.send(Some(p), logs(n));

    sub.recv().await.unwrap();
    let received = h.received(s);
    assert_eq!(received.iter().sum::<usize>(), n);
    assert!(received.iter().all(|&items| items <= u16::MAX as usize));
}

/// Shutdown with a sink that stopped consuming: `close` waits out its drain
/// budget, then stops waiting on the full topic, and the workers end within
/// the grace instead of blocking forever.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_returns_after_drain_timeout_then_stop_grace() {
    let mut h = Harness::new();
    let p = h.processor(Config::Null, &[]);
    // never read: the sink is down
    h.sink_with_buffer(&[p], 1).await;

    let (input, receiver) = flume::unbounded();
    let mut group = WorkerGroup::new(h.worker.sender.clone(), receiver, h.cache.clone());
    group.run().await;
    let batches = 4;
    for _ in 0..batches {
        input.send(ProcessorPayload::new(Some(p), logs(1))).unwrap();
    }
    drop(input);
    let metrics = group.internal_metrics.clone();
    // the first batch fills the topic, the next one blocks its worker
    while metrics.total_records.load(Ordering::Relaxed) < 2 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let drain = Duration::from_millis(200);
    let started = Instant::now();
    group.close(Deadline::at(started + drain)).await;
    let took = started.elapsed();

    assert!(took >= drain, "{took:?}");
    assert!(took < drain + STOP_GRACE, "{took:?}");
    let m = metrics.get();
    assert_eq!(m.total_workers, 0);
    assert_eq!(m.total_records, batches);
    assert_eq!(m.total_records_sent, 1);
    assert_eq!(m.total_records_errored, batches - 1);
}

/// A sink is a node of the graph too: deleting it drops the counters the
/// worker kept for it and for the edge into it.
#[tokio::test]
async fn retain_live_drops_sink_counters_of_deleted_sinks() {
    let mut h = Harness::new();
    let p = h.processor(Config::Null, &[]);
    let (kept, deleted) = (h.sink(&[p]).await, h.sink(&[p]).await);
    h.send(Some(p), logs(1));
    assert_eq!(h.node(deleted), (1, 0, 0));

    h.graph(|g| g.remove_sink(deleted));
    h.worker.internal_metrics.retain_live(&h.cache);

    assert_eq!(h.node(deleted), (0, 0, 0));
    assert_eq!(h.edge(p, deleted), None);
    assert_eq!(h.node(kept), (1, 0, 0));
    assert_eq!(h.edge(p, kept), Some(1));
}
