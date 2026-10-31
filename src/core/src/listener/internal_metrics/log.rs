// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::common::internal_metrics::visit::SKIP_REASONS;
use crate::common::internal_metrics::{
    Channel, Forwarder, ForwarderNode, ForwarderServer, InternalMetrics, Listener, LookupTable,
    LookupTableProcessor, Monitor, Processor, Sink, SkipCounts,
};
use crate::payload::{ValueOwned, ZStringOwned};
use arc_swap::Guard;
use smol_str::SmolStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Build a `(key, value)` entry for a nested [`ValueOwned::KvList`] from a
/// static key. Keeps the per-metric construction terse now that maps are
/// `Vec<(ZStringOwned, ValueOwned)>` rather than `HashMap`.
#[inline]
fn kv(key: &'static str, value: ValueOwned) -> (ZStringOwned, ValueOwned) {
    (ZStringOwned::SmolStr(SmolStr::new_static(key)), value)
}

fn _get_channel_throughput(
    channel_metric: Arc<(Instant, Channel)>,
    prev_value: &Arc<(Instant, Channel)>,
) -> (Arc<(Instant, Channel)>, Option<ValueOwned>) {
    if channel_metric.0 == prev_value.0 {
        return (channel_metric, None);
    }
    let elapsed = channel_metric.0.duration_since(prev_value.0).as_secs_f64();
    let mut chan_list = Vec::<ValueOwned>::with_capacity(channel_metric.1.channels.len());
    for (k, state) in channel_metric.1.channels.iter() {
        let mut chan_map = Vec::<(ZStringOwned, ValueOwned)>::with_capacity(12);
        chan_map.push(kv("len", ValueOwned::Int64(state.length as i64)));
        chan_map.push(kv("cap", ValueOwned::Int64(state.capacity as i64)));
        let prev_sent_time = *prev_value
            .1
            .send_wait_time
            .get(k)
            .unwrap_or(&Duration::default());
        chan_map.push(kv(
            "send_wait_time",
            ValueOwned::Double(
                (*channel_metric
                    .1
                    .send_wait_time
                    .get(k)
                    .unwrap_or(&Duration::default()))
                .saturating_sub(prev_sent_time)
                .as_secs_f64(),
            ),
        ));
        let prev_sent = (*prev_value.1.messages_sent.get(k).unwrap_or(&0)) as u64;
        chan_map.push(kv(
            "messages_sent",
            ValueOwned::Double(
                ((*channel_metric.1.messages_sent.get(k).unwrap_or(&0)) as u64)
                    .saturating_sub(prev_sent) as f64
                    / elapsed,
            ),
        ));
        let prev_dropped = (*prev_value.1.messages_dropped.get(k).unwrap_or(&0)) as u64;
        chan_map.push(kv(
            "messages_dropped",
            ValueOwned::Double(
                ((*channel_metric.1.messages_dropped.get(k).unwrap_or(&0)) as u64)
                    .saturating_sub(prev_dropped) as f64
                    / elapsed,
            ),
        ));
        let prev_batch_sent = (*prev_value.1.batches_sent.get(k).unwrap_or(&0)) as u64;
        chan_map.push(kv(
            "batches_sent",
            ValueOwned::Double(
                ((*channel_metric.1.batches_sent.get(k).unwrap_or(&0)) as u64)
                    .saturating_sub(prev_batch_sent) as f64
                    / elapsed,
            ),
        ));
        let prev_batch_dropped = (*prev_value.1.batches_dropped.get(k).unwrap_or(&0)) as u64;
        chan_map.push(kv(
            "batches_dropped",
            ValueOwned::Double(
                ((*channel_metric.1.batches_dropped.get(k).unwrap_or(&0)) as u64)
                    .saturating_sub(prev_batch_dropped) as f64
                    / elapsed,
            ),
        ));
        let prev_lost = (*prev_value.1.messages_lost.get(k).unwrap_or(&0)) as u64;
        chan_map.push(kv(
            "messages_lost",
            ValueOwned::Double(
                ((*channel_metric.1.messages_lost.get(k).unwrap_or(&0)) as u64)
                    .saturating_sub(prev_lost) as f64
                    / elapsed,
            ),
        ));
        let prev_batch_lost = (*prev_value.1.batches_lost.get(k).unwrap_or(&0)) as u64;
        chan_map.push(kv(
            "batches_lost",
            ValueOwned::Double(
                ((*channel_metric.1.batches_lost.get(k).unwrap_or(&0)) as u64)
                    .saturating_sub(prev_batch_lost) as f64
                    / elapsed,
            ),
        ));
        chan_map.push(kv("id", SmolStr::new(k).into()));
        chan_list.push(ValueOwned::KvList(chan_map));
    }
    let mut metric_map = Vec::<(ZStringOwned, ValueOwned)>::with_capacity(4);
    let prev_total_time = prev_value.1.total_time_spent;
    metric_map.push(kv(
        "total_time_spent",
        ValueOwned::Double(
            channel_metric
                .1
                .total_time_spent
                .saturating_sub(prev_total_time)
                .as_secs_f64(),
        ),
    ));
    let prev_total_messages = prev_value.1.total_messages as u64;
    metric_map.push(kv(
        "total_messages",
        ValueOwned::Double(
            (channel_metric.1.total_messages as u64).saturating_sub(prev_total_messages) as f64
                / elapsed,
        ),
    ));
    let prev_total_batches = prev_value.1.total_batches as u64;
    metric_map.push(kv(
        "total_batches",
        ValueOwned::Double(
            (channel_metric.1.total_batches as u64).saturating_sub(prev_total_batches) as f64
                / elapsed,
        ),
    ));
    metric_map.push(kv("channels", ValueOwned::Array(chan_list)));
    (channel_metric, Some(ValueOwned::KvList(metric_map)))
}

fn _get_channel_metric(channel_metric: Guard<Arc<(Instant, Channel)>>) -> ValueOwned {
    let mut chan_list = Vec::<ValueOwned>::with_capacity(channel_metric.1.channels.len());
    for (k, state) in channel_metric.1.channels.iter() {
        let mut chan_map = Vec::<(ZStringOwned, ValueOwned)>::with_capacity(12);
        chan_map.push(kv("len", ValueOwned::Int64(state.length as i64)));
        chan_map.push(kv("cap", ValueOwned::Int64(state.capacity as i64)));
        chan_map.push(kv(
            "send_wait_time",
            ValueOwned::Int64(
                (*channel_metric
                    .1
                    .send_wait_time
                    .get(k)
                    .unwrap_or(&Duration::default()))
                .as_nanos() as i64,
            ),
        ));
        chan_map.push(kv(
            "messages_sent",
            ValueOwned::Int64((*channel_metric.1.messages_sent.get(k).unwrap_or(&0)) as i64),
        ));
        chan_map.push(kv(
            "batches_sent",
            ValueOwned::Int64((*channel_metric.1.batches_sent.get(k).unwrap_or(&0)) as i64),
        ));
        chan_map.push(kv(
            "messages_dropped",
            ValueOwned::Int64((*channel_metric.1.messages_dropped.get(k).unwrap_or(&0)) as i64),
        ));
        chan_map.push(kv(
            "batches_dropped",
            ValueOwned::Int64((*channel_metric.1.batches_dropped.get(k).unwrap_or(&0)) as i64),
        ));
        chan_map.push(kv(
            "messages_lost",
            ValueOwned::Int64((*channel_metric.1.messages_lost.get(k).unwrap_or(&0)) as i64),
        ));
        chan_map.push(kv(
            "batches_lost",
            ValueOwned::Int64((*channel_metric.1.batches_lost.get(k).unwrap_or(&0)) as i64),
        ));
        chan_map.push(kv("id", SmolStr::new(k).into()));
        chan_list.push(ValueOwned::KvList(chan_map));
    }
    let metric_map: Vec<(ZStringOwned, ValueOwned)> = vec![
        kv(
            "total_time_spent",
            ValueOwned::Int64(channel_metric.1.total_time_spent.as_nanos() as i64),
        ),
        kv(
            "total_messages",
            ValueOwned::Int64(channel_metric.1.total_messages as i64),
        ),
        kv(
            "total_batches",
            ValueOwned::Int64(channel_metric.1.total_batches as i64),
        ),
        kv("channels", ValueOwned::Array(chan_list)),
    ];
    ValueOwned::KvList(metric_map)
}

fn _get_sink_metric(snapshots: &InternalMetrics) -> ValueOwned {
    let sink_metrics = snapshots.sink.load();
    let mut sink_list = Vec::<ValueOwned>::with_capacity(sink_metrics.1.len());
    for sink_metric in sink_metrics.1.iter() {
        let mut metric_map = Vec::<(ZStringOwned, ValueOwned)>::with_capacity(14);
        metric_map.push(kv("id", sink_metric.id.to_string().into()));
        metric_map.push(kv("name", SmolStr::new(&sink_metric.name).into()));
        metric_map.push(kv(
            "total_workers",
            ValueOwned::Int64(sink_metric.total_workers as i64),
        ));
        metric_map.push(kv(
            "total_time_spent",
            ValueOwned::Int64(sink_metric.total_time_spent.as_nanos() as i64),
        ));
        metric_map.push(kv(
            "engine_time_spent",
            ValueOwned::Int64(sink_metric.engine_time_spent.as_nanos() as i64),
        ));
        metric_map.push(kv(
            "receive_wait_time",
            ValueOwned::Int64(sink_metric.receive_wait_time.as_nanos() as i64),
        ));
        metric_map.push(kv(
            "send_wait_time",
            ValueOwned::Int64(sink_metric.send_wait_time.as_nanos() as i64),
        ));
        metric_map.push(kv(
            "total_records",
            ValueOwned::Int64(sink_metric.total_records as i64),
        ));
        metric_map.push(kv(
            "total_records_sent",
            ValueOwned::Int64(sink_metric.total_records_sent as i64),
        ));
        metric_map.push(kv(
            "total_records_filtered",
            ValueOwned::Int64(sink_metric.total_records_filtered as i64),
        ));
        metric_map.push(kv(
            "total_records_errored",
            ValueOwned::Int64(sink_metric.total_records_errored as i64),
        ));
        metric_map.push(kv(
            "total_records_retried",
            ValueOwned::Int64(sink_metric.total_records_retried as i64),
        ));
        metric_map.push(kv(
            "records_pending_retry",
            ValueOwned::Int64(sink_metric.records_pending_retry as i64),
        ));
        if !sink_metric.endpoints.is_empty() {
            let endpoints = sink_metric
                .endpoints
                .iter()
                .map(|endpoint| {
                    ValueOwned::KvList(vec![
                        kv("endpoint", SmolStr::new(&endpoint.endpoint).into()),
                        kv("up", ValueOwned::Bool(endpoint.up)),
                    ])
                })
                .collect();
            metric_map.push(kv("endpoints", ValueOwned::Array(endpoints)));
        }
        sink_list.push(ValueOwned::KvList(metric_map));
    }
    ValueOwned::Array(sink_list)
}

fn _get_sink_metric_throughput(
    snapshots: &InternalMetrics,
    prev_value: &Arc<(Instant, Vec<Sink>)>,
) -> (Arc<(Instant, Vec<Sink>)>, Option<ValueOwned>) {
    let sink_metrics = snapshots.sink.load_full();
    if sink_metrics.0 == prev_value.0 {
        return (sink_metrics, None);
    }
    let elapsed = sink_metrics.0.duration_since(prev_value.0).as_secs_f64();
    let mut sink_list = Vec::<ValueOwned>::with_capacity(sink_metrics.1.len());

    for sink_metric in sink_metrics.1.iter() {
        let prev_sink = prev_value.1.iter().find(|s| s.id == sink_metric.id);
        let mut metric_map = Vec::<(ZStringOwned, ValueOwned)>::with_capacity(11);

        metric_map.push(kv("id", sink_metric.id.to_string().into()));
        metric_map.push(kv("name", SmolStr::new(&sink_metric.name).into()));
        metric_map.push(kv(
            "total_workers",
            ValueOwned::Int64(sink_metric.total_workers as i64),
        ));
        metric_map.push(kv(
            "records_pending_retry",
            ValueOwned::Int64(sink_metric.records_pending_retry as i64),
        ));

        if let Some(prev) = prev_sink {
            metric_map.push(kv(
                "total_time_spent",
                ValueOwned::Double(
                    sink_metric
                        .total_time_spent
                        .saturating_sub(prev.total_time_spent)
                        .as_secs_f64(),
                ),
            ));
            metric_map.push(kv(
                "engine_time_spent",
                ValueOwned::Double(
                    sink_metric
                        .engine_time_spent
                        .saturating_sub(prev.engine_time_spent)
                        .as_secs_f64(),
                ),
            ));
            metric_map.push(kv(
                "receive_wait_time",
                ValueOwned::Double(
                    sink_metric
                        .receive_wait_time
                        .saturating_sub(prev.receive_wait_time)
                        .as_secs_f64(),
                ),
            ));
            metric_map.push(kv(
                "send_wait_time",
                ValueOwned::Double(
                    sink_metric
                        .send_wait_time
                        .saturating_sub(prev.send_wait_time)
                        .as_secs_f64(),
                ),
            ));
            metric_map.push(kv(
                "total_records",
                ValueOwned::Double(
                    (sink_metric.total_records.saturating_sub(prev.total_records)) as f64 / elapsed,
                ),
            ));
            metric_map.push(kv(
                "total_records_sent",
                ValueOwned::Double(
                    (sink_metric
                        .total_records_sent
                        .saturating_sub(prev.total_records_sent)) as f64
                        / elapsed,
                ),
            ));
            metric_map.push(kv(
                "total_records_filtered",
                ValueOwned::Double(
                    (sink_metric
                        .total_records_filtered
                        .saturating_sub(prev.total_records_filtered)) as f64
                        / elapsed,
                ),
            ));
            metric_map.push(kv(
                "total_records_errored",
                ValueOwned::Double(
                    (sink_metric
                        .total_records_errored
                        .saturating_sub(prev.total_records_errored)) as f64
                        / elapsed,
                ),
            ));
            metric_map.push(kv(
                "total_records_retried",
                ValueOwned::Double(
                    (sink_metric
                        .total_records_retried
                        .saturating_sub(prev.total_records_retried)) as f64
                        / elapsed,
                ),
            ));
        } else {
            // No previous value, use zeros
            metric_map.push(kv("total_time_spent", ValueOwned::Double(0.0)));
            metric_map.push(kv("engine_time_spent", ValueOwned::Double(0.0)));
            metric_map.push(kv("receive_wait_time", ValueOwned::Double(0.0)));
            metric_map.push(kv("send_wait_time", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_items", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_item_sent", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_item_filtered", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_items_errored", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_records_retried", ValueOwned::Double(0.0)));
        }

        sink_list.push(ValueOwned::KvList(metric_map));
    }

    (sink_metrics, Some(ValueOwned::Array(sink_list)))
}

fn _get_listener_metric(snapshots: &InternalMetrics) -> ValueOwned {
    let listener_metrics = snapshots.listener.load();
    let mut listener_list = Vec::<ValueOwned>::with_capacity(listener_metrics.1.len());
    for listener_metric in listener_metrics.1.iter() {
        let metric_map: Vec<(ZStringOwned, ValueOwned)> = vec![
            kv("id", listener_metric.id.to_string().into()),
            kv("name", SmolStr::new(&listener_metric.name).into()),
            kv(
                "total_workers",
                ValueOwned::Int64(listener_metric.total_workers as i64),
            ),
            kv(
                "total_time_spent",
                ValueOwned::Int64(listener_metric.total_time_spent.as_nanos() as i64),
            ),
            kv(
                "engine_time_spent",
                ValueOwned::Int64(listener_metric.engine_time_spent.as_nanos() as i64),
            ),
            kv(
                "receive_wait_time",
                ValueOwned::Int64(listener_metric.receive_wait_time.as_nanos() as i64),
            ),
            kv(
                "send_wait_time",
                ValueOwned::Int64(listener_metric.send_wait_time.as_nanos() as i64),
            ),
            kv(
                "total_bytes",
                ValueOwned::Int64(listener_metric.total_bytes as i64),
            ),
            kv(
                "total_bytes_succeeded",
                ValueOwned::Int64(listener_metric.total_bytes_succeeded as i64),
            ),
            kv(
                "total_bytes_dropped",
                ValueOwned::Int64(listener_metric.total_bytes_dropped as i64),
            ),
            kv(
                "total_bytes_errored",
                ValueOwned::Int64(listener_metric.total_bytes_errored as i64),
            ),
            kv(
                "total_items",
                ValueOwned::Int64(listener_metric.total_items as i64),
            ),
            kv(
                "total_items_succeeded",
                ValueOwned::Int64(listener_metric.total_items_succeeded as i64),
            ),
            kv(
                "total_items_dropped",
                ValueOwned::Int64(listener_metric.total_items_dropped as i64),
            ),
            kv(
                "total_items_errored",
                ValueOwned::Int64(listener_metric.total_items_errored as i64),
            ),
            kv(
                "total_records",
                ValueOwned::Int64(listener_metric.total_records as i64),
            ),
            kv(
                "total_records_sent",
                ValueOwned::Int64(listener_metric.total_records_sent as i64),
            ),
            kv(
                "total_records_forwarded",
                ValueOwned::Int64(listener_metric.total_records_forwarded as i64),
            ),
            kv(
                "total_records_forward_failed",
                ValueOwned::Int64(listener_metric.total_records_forward_failed as i64),
            ),
            kv(
                "total_records_annotate_failed",
                ValueOwned::Int64(listener_metric.total_records_annotate_failed as i64),
            ),
            kv(
                "total_records_dropped",
                ValueOwned::Int64(listener_metric.total_records_dropped as i64),
            ),
            kv(
                "total_records_errored",
                ValueOwned::Int64(listener_metric.total_records_errored as i64),
            ),
        ];
        listener_list.push(ValueOwned::KvList(metric_map));
    }
    ValueOwned::Array(listener_list)
}

fn _get_listener_metric_throughput(
    snapshots: &InternalMetrics,
    prev_value: &Arc<(Instant, Vec<Listener>)>,
) -> (Arc<(Instant, Vec<Listener>)>, Option<ValueOwned>) {
    let listener_metrics = snapshots.listener.load_full();
    if listener_metrics.0 == prev_value.0 {
        return (listener_metrics, None);
    }
    let elapsed = listener_metrics
        .0
        .duration_since(prev_value.0)
        .as_secs_f64();
    let mut listener_list = Vec::<ValueOwned>::with_capacity(listener_metrics.1.len());

    for listener_metric in listener_metrics.1.iter() {
        let prev_listener = prev_value.1.iter().find(|l| l.id == listener_metric.id);
        let mut metric_map = Vec::<(ZStringOwned, ValueOwned)>::with_capacity(22);

        metric_map.push(kv("id", listener_metric.id.to_string().into()));
        metric_map.push(kv("name", SmolStr::new(&listener_metric.name).into()));
        metric_map.push(kv(
            "total_workers",
            ValueOwned::Int64(listener_metric.total_workers as i64),
        ));

        if let Some(prev) = prev_listener {
            metric_map.push(kv(
                "total_time_spent",
                ValueOwned::Double(
                    listener_metric
                        .total_time_spent
                        .saturating_sub(prev.total_time_spent)
                        .as_secs_f64(),
                ),
            ));
            metric_map.push(kv(
                "engine_time_spent",
                ValueOwned::Double(
                    listener_metric
                        .engine_time_spent
                        .saturating_sub(prev.engine_time_spent)
                        .as_secs_f64(),
                ),
            ));
            metric_map.push(kv(
                "receive_wait_time",
                ValueOwned::Double(
                    listener_metric
                        .receive_wait_time
                        .saturating_sub(prev.receive_wait_time)
                        .as_secs_f64(),
                ),
            ));
            metric_map.push(kv(
                "send_wait_time",
                ValueOwned::Double(
                    listener_metric
                        .send_wait_time
                        .saturating_sub(prev.send_wait_time)
                        .as_secs_f64(),
                ),
            ));
            metric_map.push(kv(
                "total_bytes",
                ValueOwned::Double(
                    (listener_metric.total_bytes.saturating_sub(prev.total_bytes)) as f64 / elapsed,
                ),
            ));
            metric_map.push(kv(
                "total_bytes_succeeded",
                ValueOwned::Double(
                    (listener_metric
                        .total_bytes_succeeded
                        .saturating_sub(prev.total_bytes_succeeded)) as f64
                        / elapsed,
                ),
            ));
            metric_map.push(kv(
                "total_bytes_dropped",
                ValueOwned::Double(
                    (listener_metric
                        .total_bytes_dropped
                        .saturating_sub(prev.total_bytes_dropped)) as f64
                        / elapsed,
                ),
            ));
            metric_map.push(kv(
                "total_bytes_errored",
                ValueOwned::Double(
                    (listener_metric
                        .total_bytes_errored
                        .saturating_sub(prev.total_bytes_errored)) as f64
                        / elapsed,
                ),
            ));
            metric_map.push(kv(
                "total_items",
                ValueOwned::Double(
                    (listener_metric.total_items.saturating_sub(prev.total_items)) as f64 / elapsed,
                ),
            ));
            metric_map.push(kv(
                "total_items_succeeded",
                ValueOwned::Double(
                    (listener_metric
                        .total_items_succeeded
                        .saturating_sub(prev.total_items_succeeded)) as f64
                        / elapsed,
                ),
            ));
            metric_map.push(kv(
                "total_items_dropped",
                ValueOwned::Double(
                    (listener_metric
                        .total_items_dropped
                        .saturating_sub(prev.total_items_dropped)) as f64
                        / elapsed,
                ),
            ));
            metric_map.push(kv(
                "total_items_errored",
                ValueOwned::Double(
                    (listener_metric
                        .total_items_errored
                        .saturating_sub(prev.total_items_errored)) as f64
                        / elapsed,
                ),
            ));
            metric_map.push(kv(
                "total_records",
                ValueOwned::Double(
                    (listener_metric
                        .total_records
                        .saturating_sub(prev.total_records)) as f64
                        / elapsed,
                ),
            ));
            metric_map.push(kv(
                "total_records_sent",
                ValueOwned::Double(
                    (listener_metric
                        .total_records_sent
                        .saturating_sub(prev.total_records_sent)) as f64
                        / elapsed,
                ),
            ));
            metric_map.push(kv(
                "total_records_forwarded",
                ValueOwned::Double(
                    (listener_metric
                        .total_records_forwarded
                        .saturating_sub(prev.total_records_forwarded)) as f64
                        / elapsed,
                ),
            ));
            metric_map.push(kv(
                "total_records_forward_failed",
                ValueOwned::Double(
                    (listener_metric
                        .total_records_forward_failed
                        .saturating_sub(prev.total_records_forward_failed))
                        as f64
                        / elapsed,
                ),
            ));
            metric_map.push(kv(
                "total_records_annotate_failed",
                ValueOwned::Double(
                    (listener_metric
                        .total_records_annotate_failed
                        .saturating_sub(prev.total_records_annotate_failed))
                        as f64
                        / elapsed,
                ),
            ));
            metric_map.push(kv(
                "total_records_dropped",
                ValueOwned::Double(
                    (listener_metric
                        .total_records_dropped
                        .saturating_sub(prev.total_records_dropped)) as f64
                        / elapsed,
                ),
            ));
            metric_map.push(kv(
                "total_records_errored",
                ValueOwned::Double(
                    (listener_metric
                        .total_records_errored
                        .saturating_sub(prev.total_records_errored)) as f64
                        / elapsed,
                ),
            ));
        } else {
            // No previous value, use zeros
            metric_map.push(kv("total_time_spent", ValueOwned::Double(0.0)));
            metric_map.push(kv("engine_time_spent", ValueOwned::Double(0.0)));
            metric_map.push(kv("receive_wait_time", ValueOwned::Double(0.0)));
            metric_map.push(kv("send_wait_time", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_bytes", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_bytes_succeeded", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_bytes_dropped", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_bytes_errored", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_items", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_items_succeeded", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_items_dropped", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_items_errored", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_records", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_records_sent", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_records_forwarded", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_records_forward_failed", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_records_annotate_failed", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_records_dropped", ValueOwned::Double(0.0)));
            metric_map.push(kv("total_records_errored", ValueOwned::Double(0.0)));
        }

        listener_list.push(ValueOwned::KvList(metric_map));
    }

    (listener_metrics, Some(ValueOwned::Array(listener_list)))
}

fn _get_monitor_metric(snapshots: &InternalMetrics) -> ValueOwned {
    let monitor_metric = snapshots.monitor.load();
    let metric_map: Vec<(ZStringOwned, ValueOwned)> = vec![
        kv(
            "total_workers",
            ValueOwned::Int64(monitor_metric.1.total_workers as i64),
        ),
        kv(
            "total_time_spent",
            ValueOwned::Int64(monitor_metric.1.total_time_spent.as_nanos() as i64),
        ),
        kv(
            "engine_time_spent",
            ValueOwned::Int64(monitor_metric.1.engine_time_spent.as_nanos() as i64),
        ),
        kv(
            "send_wait_time",
            ValueOwned::Int64(monitor_metric.1.send_wait_time.as_nanos() as i64),
        ),
        kv(
            "receive_wait_time",
            ValueOwned::Int64(monitor_metric.1.receive_wait_time.as_nanos() as i64),
        ),
        kv(
            "total_items",
            ValueOwned::Int64(monitor_metric.1.total_items as i64),
        ),
        kv(
            "total_items_dropped",
            ValueOwned::Int64(monitor_metric.1.total_items_dropped as i64),
        ),
        kv(
            "total_items_errored",
            ValueOwned::Int64(monitor_metric.1.total_items_errored as i64),
        ),
        kv(
            "total_items_retry_dropped",
            ValueOwned::Int64(monitor_metric.1.total_items_retry_dropped as i64),
        ),
    ];
    ValueOwned::KvList(metric_map)
}

fn _get_monitor_metric_throughput(
    snapshots: &InternalMetrics,
    prev_value: &Arc<(Instant, Monitor)>,
) -> (Arc<(Instant, Monitor)>, Option<ValueOwned>) {
    let monitor_metric = snapshots.monitor.load_full();
    if monitor_metric.0 == prev_value.0 {
        return (monitor_metric, None);
    }
    let elapsed = monitor_metric.0.duration_since(prev_value.0).as_secs_f64();
    let metric_map: Vec<(ZStringOwned, ValueOwned)> = vec![
        kv(
            "total_workers",
            ValueOwned::Int64(monitor_metric.1.total_workers as i64),
        ),
        kv(
            "total_time_spent",
            ValueOwned::Double(
                monitor_metric
                    .1
                    .total_time_spent
                    .saturating_sub(prev_value.1.total_time_spent)
                    .as_secs_f64(),
            ),
        ),
        kv(
            "engine_time_spent",
            ValueOwned::Double(
                monitor_metric
                    .1
                    .engine_time_spent
                    .saturating_sub(prev_value.1.engine_time_spent)
                    .as_secs_f64(),
            ),
        ),
        kv(
            "send_wait_time",
            ValueOwned::Double(
                monitor_metric
                    .1
                    .send_wait_time
                    .saturating_sub(prev_value.1.send_wait_time)
                    .as_secs_f64(),
            ),
        ),
        kv(
            "receive_wait_time",
            ValueOwned::Double(
                monitor_metric
                    .1
                    .receive_wait_time
                    .saturating_sub(prev_value.1.receive_wait_time)
                    .as_secs_f64(),
            ),
        ),
        kv(
            "total_items",
            ValueOwned::Double(
                (monitor_metric
                    .1
                    .total_items
                    .saturating_sub(prev_value.1.total_items)) as f64
                    / elapsed,
            ),
        ),
        kv(
            "total_items_dropped",
            ValueOwned::Double(
                (monitor_metric
                    .1
                    .total_items_dropped
                    .saturating_sub(prev_value.1.total_items_dropped)) as f64
                    / elapsed,
            ),
        ),
        kv(
            "total_items_errored",
            ValueOwned::Double(
                (monitor_metric
                    .1
                    .total_items_errored
                    .saturating_sub(prev_value.1.total_items_errored)) as f64
                    / elapsed,
            ),
        ),
        kv(
            "total_items_retry_dropped",
            ValueOwned::Double(
                (monitor_metric
                    .1
                    .total_items_retry_dropped
                    .saturating_sub(prev_value.1.total_items_retry_dropped)) as f64
                    / elapsed,
            ),
        ),
    ];
    (monitor_metric, Some(ValueOwned::KvList(metric_map)))
}

fn _get_processor_metric(snapshots: &InternalMetrics) -> ValueOwned {
    let processor_metric = snapshots.processor.load();
    let mut metric_map = Vec::<(ZStringOwned, ValueOwned)>::with_capacity(12);
    metric_map.push(kv(
        "total_workers",
        ValueOwned::Int64(processor_metric.1.total_workers as i64),
    ));
    metric_map.push(kv(
        "total_time_spent",
        ValueOwned::Int64(processor_metric.1.total_time_spent.as_nanos() as i64),
    ));
    metric_map.push(kv(
        "engine_time_spent",
        ValueOwned::Int64(processor_metric.1.engine_time_spent.as_nanos() as i64),
    ));
    metric_map.push(kv(
        "send_wait_time",
        ValueOwned::Int64(processor_metric.1.send_wait_time.as_nanos() as i64),
    ));
    metric_map.push(kv(
        "receive_wait_time",
        ValueOwned::Int64(processor_metric.1.receive_wait_time.as_nanos() as i64),
    ));
    metric_map.push(kv(
        "total_records",
        ValueOwned::Int64(processor_metric.1.total_records as i64),
    ));
    metric_map.push(kv(
        "total_records_sent",
        ValueOwned::Int64(processor_metric.1.total_records_sent as i64),
    ));
    metric_map.push(kv(
        "total_records_filtered",
        ValueOwned::Int64(processor_metric.1.total_records_filtered as i64),
    ));
    metric_map.push(kv(
        "total_records_errored",
        ValueOwned::Int64(processor_metric.1.total_records_errored as i64),
    ));
    metric_map.push(kv(
        "total_records_unrouted",
        ValueOwned::Int64(processor_metric.1.total_records_unrouted as i64),
    ));

    let mut node_list = Vec::<ValueOwned>::with_capacity(processor_metric.1.nodes.len());
    for node in processor_metric.1.nodes.iter() {
        let node_map: Vec<(ZStringOwned, ValueOwned)> = vec![
            kv("id", node.id.to_string().into()),
            kv(
                "total_time_spent",
                ValueOwned::Int64(node.total_time_spent.as_nanos() as i64),
            ),
            kv(
                "engine_time_spent",
                ValueOwned::Int64(node.engine_time_spent.as_nanos() as i64),
            ),
            kv(
                "send_wait_time",
                ValueOwned::Int64(node.send_wait_time.as_nanos() as i64),
            ),
            kv(
                "total_records",
                ValueOwned::Int64(node.total_records as i64),
            ),
            kv(
                "total_records_filtered",
                ValueOwned::Int64(node.total_records_filtered as i64),
            ),
            kv(
                "total_records_errored",
                ValueOwned::Int64(node.total_records_errored as i64),
            ),
        ];
        node_list.push(ValueOwned::KvList(node_map));
    }
    metric_map.push(kv("nodes", ValueOwned::Array(node_list)));

    let mut edge_list = Vec::<ValueOwned>::with_capacity(processor_metric.1.edges.len());
    for edge in processor_metric.1.edges.iter() {
        let edge_map: Vec<(ZStringOwned, ValueOwned)> = vec![
            kv("source", edge.source.to_string().into()),
            kv("target", edge.target.to_string().into()),
            kv("target_is_sink", ValueOwned::Bool(edge.target_is_sink)),
            kv(
                "total_records",
                ValueOwned::Int64(edge.total_records as i64),
            ),
        ];
        edge_list.push(ValueOwned::KvList(edge_map));
    }
    metric_map.push(kv("edges", ValueOwned::Array(edge_list)));

    ValueOwned::KvList(metric_map)
}

fn _get_processor_metric_throughput(
    snapshots: &InternalMetrics,
    prev_value: &Arc<(Instant, Processor)>,
) -> (Arc<(Instant, Processor)>, Option<ValueOwned>) {
    let processor_metric = snapshots.processor.load_full();
    if processor_metric.0 == prev_value.0 {
        return (processor_metric, None);
    }
    let elapsed = processor_metric
        .0
        .duration_since(prev_value.0)
        .as_secs_f64();
    let mut metric_map = Vec::<(ZStringOwned, ValueOwned)>::with_capacity(11);

    metric_map.push(kv(
        "total_workers",
        ValueOwned::Int64(processor_metric.1.total_workers as i64),
    ));
    metric_map.push(kv(
        "total_time_spent",
        ValueOwned::Double(
            processor_metric
                .1
                .total_time_spent
                .saturating_sub(prev_value.1.total_time_spent)
                .as_secs_f64(),
        ),
    ));
    metric_map.push(kv(
        "engine_time_spent",
        ValueOwned::Double(
            processor_metric
                .1
                .engine_time_spent
                .saturating_sub(prev_value.1.engine_time_spent)
                .as_secs_f64(),
        ),
    ));
    metric_map.push(kv(
        "send_wait_time",
        ValueOwned::Double(
            processor_metric
                .1
                .send_wait_time
                .saturating_sub(prev_value.1.send_wait_time)
                .as_secs_f64(),
        ),
    ));
    metric_map.push(kv(
        "receive_wait_time",
        ValueOwned::Double(
            processor_metric
                .1
                .receive_wait_time
                .saturating_sub(prev_value.1.receive_wait_time)
                .as_secs_f64(),
        ),
    ));
    metric_map.push(kv(
        "total_records",
        ValueOwned::Double(
            (processor_metric
                .1
                .total_records
                .saturating_sub(prev_value.1.total_records)) as f64
                / elapsed,
        ),
    ));
    metric_map.push(kv(
        "total_records_sent",
        ValueOwned::Double(
            (processor_metric
                .1
                .total_records_sent
                .saturating_sub(prev_value.1.total_records_sent)) as f64
                / elapsed,
        ),
    ));
    metric_map.push(kv(
        "total_records_filtered",
        ValueOwned::Double(
            (processor_metric
                .1
                .total_records_filtered
                .saturating_sub(prev_value.1.total_records_filtered)) as f64
                / elapsed,
        ),
    ));
    metric_map.push(kv(
        "total_records_errored",
        ValueOwned::Double(
            (processor_metric
                .1
                .total_records_errored
                .saturating_sub(prev_value.1.total_records_errored)) as f64
                / elapsed,
        ),
    ));

    let mut node_list = Vec::<ValueOwned>::with_capacity(processor_metric.1.nodes.len());
    for node in processor_metric.1.nodes.iter() {
        let prev_node = prev_value.1.nodes.iter().find(|n| n.id == node.id);
        let mut node_map = Vec::<(ZStringOwned, ValueOwned)>::with_capacity(7);
        node_map.push(kv("id", node.id.to_string().into()));
        if let Some(prev) = prev_node {
            node_map.push(kv(
                "total_time_spent",
                ValueOwned::Double(
                    node.total_time_spent
                        .saturating_sub(prev.total_time_spent)
                        .as_secs_f64(),
                ),
            ));
            node_map.push(kv(
                "engine_time_spent",
                ValueOwned::Double(
                    node.engine_time_spent
                        .saturating_sub(prev.engine_time_spent)
                        .as_secs_f64(),
                ),
            ));
            node_map.push(kv(
                "send_wait_time",
                ValueOwned::Double(
                    node.send_wait_time
                        .saturating_sub(prev.send_wait_time)
                        .as_secs_f64(),
                ),
            ));
            node_map.push(kv(
                "total_records",
                ValueOwned::Double(
                    node.total_records.saturating_sub(prev.total_records) as f64 / elapsed,
                ),
            ));
            node_map.push(kv(
                "total_records_filtered",
                ValueOwned::Double(
                    node.total_records_filtered
                        .saturating_sub(prev.total_records_filtered) as f64
                        / elapsed,
                ),
            ));
            node_map.push(kv(
                "total_records_errored",
                ValueOwned::Double(
                    node.total_records_errored
                        .saturating_sub(prev.total_records_errored) as f64
                        / elapsed,
                ),
            ));
        } else {
            node_map.push(kv("total_time_spent", ValueOwned::Double(0.0)));
            node_map.push(kv("engine_time_spent", ValueOwned::Double(0.0)));
            node_map.push(kv("send_wait_time", ValueOwned::Double(0.0)));
            node_map.push(kv("total_records", ValueOwned::Double(0.0)));
            node_map.push(kv("total_records_filtered", ValueOwned::Double(0.0)));
            node_map.push(kv("total_records_errored", ValueOwned::Double(0.0)));
        }
        node_list.push(ValueOwned::KvList(node_map));
    }
    metric_map.push(kv("nodes", ValueOwned::Array(node_list)));

    let mut edge_list = Vec::<ValueOwned>::with_capacity(processor_metric.1.edges.len());
    for edge in processor_metric.1.edges.iter() {
        let prev_edge = prev_value
            .1
            .edges
            .iter()
            .find(|e| e.source == edge.source && e.target == edge.target);
        let mut edge_map = Vec::<(ZStringOwned, ValueOwned)>::with_capacity(4);
        edge_map.push(kv("source", edge.source.to_string().into()));
        edge_map.push(kv("target", edge.target.to_string().into()));
        edge_map.push(kv("target_is_sink", ValueOwned::Bool(edge.target_is_sink)));
        if let Some(prev) = prev_edge {
            edge_map.push(kv(
                "total_records",
                ValueOwned::Double(
                    edge.total_records.saturating_sub(prev.total_records) as f64 / elapsed,
                ),
            ));
        } else {
            edge_map.push(kv("total_time_spent", ValueOwned::Double(0.0)));
            edge_map.push(kv("engine_time_spent", ValueOwned::Double(0.0)));
            edge_map.push(kv("send_wait_time", ValueOwned::Double(0.0)));
            edge_map.push(kv("total_records", ValueOwned::Double(0.0)));
        }
        edge_list.push(ValueOwned::KvList(edge_map));
    }
    metric_map.push(kv("edges", ValueOwned::Array(edge_list)));

    (processor_metric, Some(ValueOwned::KvList(metric_map)))
}

fn _forwarder_node_map(node: &ForwarderNode) -> Vec<(ZStringOwned, ValueOwned)> {
    let node_map: Vec<(ZStringOwned, ValueOwned)> = vec![
        kv("id", node.id.to_string().into()),
        kv(
            "total_connections",
            ValueOwned::Int64(node.total_connections as i64),
        ),
        kv(
            "total_time_spent",
            ValueOwned::Int64(node.total_time_spent.as_nanos() as i64),
        ),
        kv(
            "engine_time_spent",
            ValueOwned::Int64(node.engine_time_spent.as_nanos() as i64),
        ),
        kv(
            "send_wait_time",
            ValueOwned::Int64(node.send_wait_time.as_nanos() as i64),
        ),
        kv(
            "receive_wait_time",
            ValueOwned::Int64(node.receive_wait_time.as_nanos() as i64),
        ),
        kv(
            "total_records",
            ValueOwned::Int64(node.total_records as i64),
        ),
        kv(
            "total_records_sent",
            ValueOwned::Int64(node.total_records_sent as i64),
        ),
        kv(
            "total_records_dropped",
            ValueOwned::Int64(node.total_records_dropped as i64),
        ),
        kv(
            "total_records_errored",
            ValueOwned::Int64(node.total_records_errored as i64),
        ),
        kv("total_bytes", ValueOwned::Int64(node.total_bytes as i64)),
        kv(
            "total_bytes_sent",
            ValueOwned::Int64(node.total_bytes_sent as i64),
        ),
        kv(
            "total_bytes_dropped",
            ValueOwned::Int64(node.total_bytes_dropped as i64),
        ),
        kv(
            "total_bytes_errored",
            ValueOwned::Int64(node.total_bytes_errored as i64),
        ),
    ];
    node_map
}

fn _get_forwarder_metric(snapshots: &InternalMetrics) -> ValueOwned {
    let forwarder_metric = snapshots.forwarder.load();
    let forwarder = &forwarder_metric.1;
    let mut metric_map = Vec::<(ZStringOwned, ValueOwned)>::with_capacity(13);
    metric_map.push(kv(
        "total_time_spent",
        ValueOwned::Int64(forwarder.total_time_spent.as_nanos() as i64),
    ));
    metric_map.push(kv(
        "engine_time_spent",
        ValueOwned::Int64(forwarder.engine_time_spent.as_nanos() as i64),
    ));
    metric_map.push(kv(
        "send_wait_time",
        ValueOwned::Int64(forwarder.send_wait_time.as_nanos() as i64),
    ));
    metric_map.push(kv(
        "receive_wait_time",
        ValueOwned::Int64(forwarder.receive_wait_time.as_nanos() as i64),
    ));
    metric_map.push(kv(
        "total_records",
        ValueOwned::Int64(forwarder.total_records as i64),
    ));
    metric_map.push(kv(
        "total_records_sent",
        ValueOwned::Int64(forwarder.total_records_sent as i64),
    ));
    metric_map.push(kv(
        "total_records_dropped",
        ValueOwned::Int64(forwarder.total_records_dropped as i64),
    ));
    metric_map.push(kv(
        "total_records_errored",
        ValueOwned::Int64(forwarder.total_records_errored as i64),
    ));
    let node_list: Vec<ValueOwned> = forwarder
        .nodes
        .iter()
        .map(|node| ValueOwned::KvList(_forwarder_node_map(node)))
        .collect();
    metric_map.push(kv("nodes", ValueOwned::Array(node_list)));
    ValueOwned::KvList(metric_map)
}

fn _forwarder_node_throughput_map(
    node: &ForwarderNode,
    prev: Option<&ForwarderNode>,
    elapsed: f64,
) -> Vec<(ZStringOwned, ValueOwned)> {
    let mut node_map = Vec::<(ZStringOwned, ValueOwned)>::with_capacity(14);
    node_map.push(kv("id", node.id.to_string().into()));
    node_map.push(kv(
        "total_connections",
        ValueOwned::Int64(node.total_connections as i64),
    ));
    if let Some(prev) = prev {
        node_map.push(kv(
            "total_time_spent",
            ValueOwned::Double(
                node.total_time_spent
                    .saturating_sub(prev.total_time_spent)
                    .as_secs_f64(),
            ),
        ));
        node_map.push(kv(
            "engine_time_spent",
            ValueOwned::Double(
                node.engine_time_spent
                    .saturating_sub(prev.engine_time_spent)
                    .as_secs_f64(),
            ),
        ));
        node_map.push(kv(
            "send_wait_time",
            ValueOwned::Double(
                node.send_wait_time
                    .saturating_sub(prev.send_wait_time)
                    .as_secs_f64(),
            ),
        ));
        node_map.push(kv(
            "receive_wait_time",
            ValueOwned::Double(
                node.receive_wait_time
                    .saturating_sub(prev.receive_wait_time)
                    .as_secs_f64(),
            ),
        ));
        node_map.push(kv(
            "total_records",
            ValueOwned::Double(
                node.total_records.saturating_sub(prev.total_records) as f64 / elapsed,
            ),
        ));
        node_map.push(kv(
            "total_records_sent",
            ValueOwned::Double(
                node.total_records_sent
                    .saturating_sub(prev.total_records_sent) as f64
                    / elapsed,
            ),
        ));
        node_map.push(kv(
            "total_records_dropped",
            ValueOwned::Double(
                node.total_records_dropped
                    .saturating_sub(prev.total_records_dropped) as f64
                    / elapsed,
            ),
        ));
        node_map.push(kv(
            "total_records_errored",
            ValueOwned::Double(
                node.total_records_errored
                    .saturating_sub(prev.total_records_errored) as f64
                    / elapsed,
            ),
        ));
        node_map.push(kv(
            "total_bytes",
            ValueOwned::Double(node.total_bytes.saturating_sub(prev.total_bytes) as f64 / elapsed),
        ));
        node_map.push(kv(
            "total_bytes_sent",
            ValueOwned::Double(
                node.total_bytes_sent.saturating_sub(prev.total_bytes_sent) as f64 / elapsed,
            ),
        ));
        node_map.push(kv(
            "total_bytes_dropped",
            ValueOwned::Double(
                node.total_bytes_dropped
                    .saturating_sub(prev.total_bytes_dropped) as f64
                    / elapsed,
            ),
        ));
        node_map.push(kv(
            "total_bytes_errored",
            ValueOwned::Double(
                node.total_bytes_errored
                    .saturating_sub(prev.total_bytes_errored) as f64
                    / elapsed,
            ),
        ));
    } else {
        for key in [
            "total_time_spent",
            "engine_time_spent",
            "send_wait_time",
            "receive_wait_time",
            "total_records",
            "total_records_sent",
            "total_records_dropped",
            "total_records_errored",
            "total_bytes",
            "total_bytes_sent",
            "total_bytes_dropped",
            "total_bytes_errored",
        ] {
            node_map.push((SmolStr::new(key).into(), ValueOwned::Double(0.0)));
        }
    }
    node_map
}

fn _get_forwarder_metric_throughput(
    snapshots: &InternalMetrics,
    prev_value: &Arc<(Instant, Forwarder)>,
) -> (Arc<(Instant, Forwarder)>, Option<ValueOwned>) {
    let forwarder_metric = snapshots.forwarder.load_full();
    if forwarder_metric.0 == prev_value.0 {
        return (forwarder_metric, None);
    }
    let elapsed = forwarder_metric
        .0
        .duration_since(prev_value.0)
        .as_secs_f64();
    let forwarder = &forwarder_metric.1;
    let prev = &prev_value.1;
    let mut metric_map = Vec::<(ZStringOwned, ValueOwned)>::with_capacity(13);
    metric_map.push(kv(
        "total_time_spent",
        ValueOwned::Double(
            forwarder
                .total_time_spent
                .saturating_sub(prev.total_time_spent)
                .as_secs_f64(),
        ),
    ));
    metric_map.push(kv(
        "engine_time_spent",
        ValueOwned::Double(
            forwarder
                .engine_time_spent
                .saturating_sub(prev.engine_time_spent)
                .as_secs_f64(),
        ),
    ));
    metric_map.push(kv(
        "send_wait_time",
        ValueOwned::Double(
            forwarder
                .send_wait_time
                .saturating_sub(prev.send_wait_time)
                .as_secs_f64(),
        ),
    ));
    metric_map.push(kv(
        "receive_wait_time",
        ValueOwned::Double(
            forwarder
                .receive_wait_time
                .saturating_sub(prev.receive_wait_time)
                .as_secs_f64(),
        ),
    ));
    metric_map.push(kv(
        "total_records",
        ValueOwned::Double(
            forwarder.total_records.saturating_sub(prev.total_records) as f64 / elapsed,
        ),
    ));
    metric_map.push(kv(
        "total_records_sent",
        ValueOwned::Double(
            forwarder
                .total_records_sent
                .saturating_sub(prev.total_records_sent) as f64
                / elapsed,
        ),
    ));
    metric_map.push(kv(
        "total_records_dropped",
        ValueOwned::Double(
            forwarder
                .total_records_dropped
                .saturating_sub(prev.total_records_dropped) as f64
                / elapsed,
        ),
    ));
    metric_map.push(kv(
        "total_records_errored",
        ValueOwned::Double(
            forwarder
                .total_records_errored
                .saturating_sub(prev.total_records_errored) as f64
                / elapsed,
        ),
    ));
    let node_list: Vec<ValueOwned> = forwarder
        .nodes
        .iter()
        .map(|node| {
            let prev_node = prev.nodes.iter().find(|n| n.id == node.id);
            ValueOwned::KvList(_forwarder_node_throughput_map(node, prev_node, elapsed))
        })
        .collect();
    metric_map.push(kv("nodes", ValueOwned::Array(node_list)));
    (forwarder_metric, Some(ValueOwned::KvList(metric_map)))
}

fn _get_forwarder_server_metric(snapshots: &InternalMetrics) -> ValueOwned {
    let server_metric = snapshots.forwarder_server.load();
    let server = &server_metric.1;
    let metric_map: Vec<(ZStringOwned, ValueOwned)> = vec![
        kv(
            "engine_time_spent",
            ValueOwned::Int64(server.engine_time_spent.as_nanos() as i64),
        ),
        kv(
            "send_wait_time",
            ValueOwned::Int64(server.send_wait_time.as_nanos() as i64),
        ),
        kv(
            "total_records",
            ValueOwned::Int64(server.total_records as i64),
        ),
        kv(
            "total_records_sent",
            ValueOwned::Int64(server.total_records_sent as i64),
        ),
        kv(
            "total_records_errored",
            ValueOwned::Int64(server.total_records_errored as i64),
        ),
        kv("total_bytes", ValueOwned::Int64(server.total_bytes as i64)),
    ];
    ValueOwned::KvList(metric_map)
}

fn _get_forwarder_server_metric_throughput(
    snapshots: &InternalMetrics,
    prev_value: &Arc<(Instant, ForwarderServer)>,
) -> (Arc<(Instant, ForwarderServer)>, Option<ValueOwned>) {
    let server_metric = snapshots.forwarder_server.load_full();
    if server_metric.0 == prev_value.0 {
        return (server_metric, None);
    }
    let elapsed = server_metric.0.duration_since(prev_value.0).as_secs_f64();
    let metric_map: Vec<(ZStringOwned, ValueOwned)> = vec![
        kv(
            "engine_time_spent",
            ValueOwned::Double(
                server_metric
                    .1
                    .engine_time_spent
                    .saturating_sub(prev_value.1.engine_time_spent)
                    .as_secs_f64(),
            ),
        ),
        kv(
            "send_wait_time",
            ValueOwned::Double(
                server_metric
                    .1
                    .send_wait_time
                    .saturating_sub(prev_value.1.send_wait_time)
                    .as_secs_f64(),
            ),
        ),
        kv(
            "total_records",
            ValueOwned::Double(
                (server_metric
                    .1
                    .total_records
                    .saturating_sub(prev_value.1.total_records)) as f64
                    / elapsed,
            ),
        ),
        kv(
            "total_records_sent",
            ValueOwned::Double(
                (server_metric
                    .1
                    .total_records_sent
                    .saturating_sub(prev_value.1.total_records_sent)) as f64
                    / elapsed,
            ),
        ),
        kv(
            "total_records_errored",
            ValueOwned::Double(
                (server_metric
                    .1
                    .total_records_errored
                    .saturating_sub(prev_value.1.total_records_errored)) as f64
                    / elapsed,
            ),
        ),
        kv(
            "total_bytes",
            ValueOwned::Double(
                (server_metric
                    .1
                    .total_bytes
                    .saturating_sub(prev_value.1.total_bytes)) as f64
                    / elapsed,
            ),
        ),
    ];
    (server_metric, Some(ValueOwned::KvList(metric_map)))
}

/// Counter fields of a table, driving the cumulative and per-second rendering
/// from one list. Every other section writes its two branches out twice, which
/// is how the sink section came to spell its fallback keys differently from
/// its live ones.
/// One named field of a snapshot entity, read as a `u64`.
type Field<T> = (&'static str, fn(&T) -> u64);

const TABLE_COUNTERS: [Field<LookupTable>; 7] = [
    ("loads", |t| t.loads),
    ("load_failures", |t| t.load_failures),
    ("rows_read", |t| t.rows_read),
    ("rows_skipped", |t| t.rows_skipped.total()),
    ("rows_rejected", |t| t.rows_rejected),
    ("rows_expired", |t| t.rows_expired),
    ("strings_released", |t| t.strings_released),
];

/// Gauge fields of a table. Reported absolutely in both modes: a rate of change
/// of a row count is not what an operator wants to read.
const TABLE_GAUGES: [Field<LookupTable>; 7] = [
    ("rows", |t| t.rows),
    ("index_entries", |t| t.index_entries),
    ("max_rows", |t| t.max_rows),
    ("pooled_strings", |t| t.pooled_strings),
    ("generation", |t| t.generation),
    ("generation_rows_read", |t| t.generation_rows_read),
    ("generation_rows_skipped", |t| {
        t.generation_rows_skipped.total()
    }),
];

const PROCESSOR_COUNTERS: [Field<LookupTableProcessor>; 10] = [
    ("lookups", |p| p.lookups),
    ("lookups_hit", |p| p.lookups_hit),
    ("lookups_missed", |p| p.lookups_missed),
    ("lookups_errored", |p| p.lookups_errored),
    ("writes", |p| p.writes),
    ("writes_inserted", |p| p.writes_inserted),
    ("writes_updated", |p| p.writes_updated),
    ("writes_rejected", |p| p.writes_rejected),
    ("writes_errored", |p| p.writes_errored),
    ("removes", |p| p.removes),
];

/// How to render a counter: as its running total, or as a per-second rate
/// against the previous snapshot.
enum Mode<'a> {
    Total,
    Rate {
        prev: Option<&'a LookupTable>,
        elapsed: f64,
    },
}

impl Mode<'_> {
    /// A counter, either absolute or as a rate. A field that did not exist in
    /// the previous snapshot rates from zero, which is right: the table, or the
    /// processor, is new.
    fn counter(&self, current: u64, previous: impl FnOnce(&LookupTable) -> u64) -> ValueOwned {
        match self {
            Mode::Total => ValueOwned::Int64(current as i64),
            Mode::Rate { prev, elapsed } => {
                let base = prev.map_or(0, previous);
                ValueOwned::Double(current.saturating_sub(base) as f64 / elapsed)
            }
        }
    }

    fn duration(&self, current: Duration, previous: fn(&LookupTable) -> Duration) -> ValueOwned {
        match self {
            Mode::Total => ValueOwned::Int64(current.as_nanos() as i64),
            Mode::Rate { prev, .. } => {
                let base = prev.map_or(Duration::ZERO, previous);
                ValueOwned::Double(current.saturating_sub(base).as_secs_f64())
            }
        }
    }
}

/// Per-reason skip counts as a nested map, so a reason that has never fired
/// still reads as zero rather than being absent.
fn _skip_map(counts: &SkipCounts, mode: &Mode<'_>) -> ValueOwned {
    ValueOwned::KvList(
        SKIP_REASONS
            .iter()
            .map(|(name, read)| {
                kv(
                    name,
                    mode.counter(read(counts), |table| read(&table.rows_skipped)),
                )
            })
            .collect(),
    )
}

fn _lookup_table_entry(table: &LookupTable, mode: Mode<'_>) -> ValueOwned {
    let mut metric_map = Vec::<(ZStringOwned, ValueOwned)>::with_capacity(
        4 + TABLE_GAUGES.len() + TABLE_COUNTERS.len() + 3,
    );
    metric_map.push(kv("id", table.id.to_string().into()));
    metric_map.push(kv("name", SmolStr::new(&table.name).into()));
    metric_map.push(kv("kind", SmolStr::new_static(table.kind).into()));

    for (name, read) in TABLE_GAUGES {
        metric_map.push(kv(name, ValueOwned::Int64(read(table) as i64)));
    }
    for (name, read) in TABLE_COUNTERS {
        metric_map.push(kv(name, mode.counter(read(table), read)));
    }
    metric_map.push(kv(
        "load_duration",
        mode.duration(table.load_duration, |t| t.load_duration),
    ));
    // absolute in both modes, so nanoseconds throughout, as every other
    // absolute duration in this document is
    metric_map.push(kv(
        "generation_load_duration",
        ValueOwned::Int64(table.generation_load_duration.as_nanos() as i64),
    ));
    metric_map.push(kv(
        "rows_skipped_by_reason",
        _skip_map(&table.rows_skipped, &mode),
    ));
    metric_map.push(kv(
        "generation_rows_skipped_by_reason",
        _skip_map(&table.generation_rows_skipped, &Mode::Total),
    ));
    if table.kind == "immutable" {
        metric_map.push(kv(
            "load_error",
            ValueOwned::Int64(i64::from(table.load_error)),
        ));
        // Unix seconds in both modes, matching the metrics-signal gauge.
        metric_map.push(kv(
            "last_success_timestamp",
            ValueOwned::Double(table.last_success_timestamp),
        ));
    }

    let processors: Vec<ValueOwned> = table
        .processors
        .iter()
        .map(|processor| {
            let prev = match &mode {
                Mode::Total => None,
                Mode::Rate { prev, .. } => {
                    prev.and_then(|p| p.processors.iter().find(|q| q.id == processor.id))
                }
            };
            let mut map =
                Vec::<(ZStringOwned, ValueOwned)>::with_capacity(1 + PROCESSOR_COUNTERS.len());
            map.push(kv("id", processor.id.to_string().into()));
            for (name, read) in PROCESSOR_COUNTERS {
                let value = match &mode {
                    Mode::Total => ValueOwned::Int64(read(processor) as i64),
                    Mode::Rate { elapsed, .. } => ValueOwned::Double(
                        read(processor).saturating_sub(prev.map_or(0, read)) as f64 / elapsed,
                    ),
                };
                map.push(kv(name, value));
            }
            ValueOwned::KvList(map)
        })
        .collect();
    metric_map.push(kv("processors", ValueOwned::Array(processors)));

    ValueOwned::KvList(metric_map)
}

fn _get_lookup_table_metric(snapshots: &InternalMetrics) -> ValueOwned {
    let metrics = snapshots.lookup_table.load();
    ValueOwned::Array(
        metrics
            .1
            .iter()
            .map(|table| _lookup_table_entry(table, Mode::Total))
            .collect(),
    )
}

fn _get_lookup_table_metric_throughput(
    snapshots: &InternalMetrics,
    prev_value: &Arc<(Instant, Vec<LookupTable>)>,
) -> (Arc<(Instant, Vec<LookupTable>)>, Option<ValueOwned>) {
    let metrics = snapshots.lookup_table.load_full();
    if metrics.0 == prev_value.0 {
        return (metrics, None);
    }
    let elapsed = metrics.0.duration_since(prev_value.0).as_secs_f64();
    let list: Vec<ValueOwned> = metrics
        .1
        .iter()
        .map(|table| {
            let prev = prev_value.1.iter().find(|t| t.id == table.id);
            _lookup_table_entry(table, Mode::Rate { prev, elapsed })
        })
        .collect();
    (metrics, Some(ValueOwned::Array(list)))
}

/// The snapshots the previous throughput pass diffed against. One per
/// listener, so two listeners never consume each other's window.
pub(super) struct PrevValue {
    sink_channel: Arc<(Instant, Channel)>,
    processor_channel: Arc<(Instant, Channel)>,
    forwarder_channel: Arc<(Instant, Channel)>,
    sink: Arc<(Instant, Vec<Sink>)>,
    listener: Arc<(Instant, Vec<Listener>)>,
    monitor: Arc<(Instant, Monitor)>,
    processor: Arc<(Instant, Processor)>,
    forwarder: Arc<(Instant, Forwarder)>,
    forwarder_server: Arc<(Instant, ForwarderServer)>,
    lookup_table: Arc<(Instant, Vec<LookupTable>)>,
}

/// One record's attributes: a nested document of every subsystem's snapshot,
/// or with `report_throughput` its per-second rates against `baseline`, which
/// is advanced in place. The first throughput pass only primes the baseline
/// and returns `None`: there is nothing to diff against yet.
pub(super) fn render(
    snapshots: &InternalMetrics,
    report_throughput: bool,
    baseline: &mut Option<PrevValue>,
) -> Option<Vec<(ZStringOwned, ValueOwned)>> {
    let mut attrs = Vec::<(ZStringOwned, ValueOwned)>::with_capacity(10);
    if report_throughput {
        if let Some(prev_value) = baseline {
            let (sink_channel_prev, sink_channel) = _get_channel_throughput(
                snapshots.sink_channel.load_full(),
                &prev_value.sink_channel,
            );
            if let Some(sink_channel) = sink_channel {
                prev_value.sink_channel = sink_channel_prev;
                attrs.push((ZStringOwned::from_static("sink_channel"), sink_channel));
            }

            let (processor_channel_prev, processor_channel) = _get_channel_throughput(
                snapshots.processor_channel.load_full(),
                &prev_value.processor_channel,
            );
            if let Some(processor_channel) = processor_channel {
                prev_value.processor_channel = processor_channel_prev;
                attrs.push((
                    ZStringOwned::from_static("processor_channel"),
                    processor_channel,
                ));
            }

            let (forwarder_channel_prev, forwarder_channel) = _get_channel_throughput(
                snapshots.forwarder_channel.load_full(),
                &prev_value.forwarder_channel,
            );
            if let Some(forwarder_channel) = forwarder_channel {
                prev_value.forwarder_channel = forwarder_channel_prev;
                attrs.push((
                    ZStringOwned::from_static("forwarder_channel"),
                    forwarder_channel,
                ));
            }

            let (sink_prev, sink) = _get_sink_metric_throughput(snapshots, &prev_value.sink);
            if let Some(sink) = sink {
                prev_value.sink = sink_prev;
                attrs.push((ZStringOwned::from_static("sink"), sink));
            }

            let (listener_prev, listener) =
                _get_listener_metric_throughput(snapshots, &prev_value.listener);
            if let Some(listener) = listener {
                prev_value.listener = listener_prev;
                attrs.push((ZStringOwned::from_static("listener"), listener));
            }

            let (monitor_prev, monitor) =
                _get_monitor_metric_throughput(snapshots, &prev_value.monitor);
            if let Some(monitor) = monitor {
                prev_value.monitor = monitor_prev;
                attrs.push((ZStringOwned::from_static("monitor"), monitor));
            }

            let (processor_prev, processor) =
                _get_processor_metric_throughput(snapshots, &prev_value.processor);
            if let Some(processor) = processor {
                prev_value.processor = processor_prev;
                attrs.push((ZStringOwned::from_static("processor"), processor));
            }

            let (forwarder_prev, forwarder) =
                _get_forwarder_metric_throughput(snapshots, &prev_value.forwarder);
            if let Some(forwarder) = forwarder {
                prev_value.forwarder = forwarder_prev;
                attrs.push((ZStringOwned::from_static("forwarder"), forwarder));
            }

            let (forwarder_server_prev, forwarder_server) =
                _get_forwarder_server_metric_throughput(snapshots, &prev_value.forwarder_server);
            if let Some(forwarder_server) = forwarder_server {
                prev_value.forwarder_server = forwarder_server_prev;
                attrs.push((
                    ZStringOwned::from_static("forwarder_server"),
                    forwarder_server,
                ));
            }
            let (lookup_table_prev, lookup_table) =
                _get_lookup_table_metric_throughput(snapshots, &prev_value.lookup_table);
            if let Some(lookup_table) = lookup_table {
                prev_value.lookup_table = lookup_table_prev;
                attrs.push((ZStringOwned::from_static("lookup_table"), lookup_table));
            }
        } else {
            *baseline = Some(PrevValue {
                sink_channel: snapshots.sink_channel.load_full(),
                processor_channel: snapshots.processor_channel.load_full(),
                forwarder_channel: snapshots.forwarder_channel.load_full(),
                sink: snapshots.sink.load_full(),
                listener: snapshots.listener.load_full(),
                monitor: snapshots.monitor.load_full(),
                processor: snapshots.processor.load_full(),
                forwarder: snapshots.forwarder.load_full(),
                forwarder_server: snapshots.forwarder_server.load_full(),
                lookup_table: snapshots.lookup_table.load_full(),
            });
            return None;
        }
    } else {
        let sink_channel = _get_channel_metric(snapshots.sink_channel.load());
        attrs.push((ZStringOwned::from_static("sink_channel"), sink_channel));

        let processor_channel = _get_channel_metric(snapshots.processor_channel.load());
        attrs.push((
            ZStringOwned::from_static("processor_channel"),
            processor_channel,
        ));

        let forwarder_channel = _get_channel_metric(snapshots.forwarder_channel.load());
        attrs.push((
            ZStringOwned::from_static("forwarder_channel"),
            forwarder_channel,
        ));

        let sink = _get_sink_metric(snapshots);
        attrs.push((ZStringOwned::from_static("sink"), sink));

        let listener = _get_listener_metric(snapshots);
        attrs.push((ZStringOwned::from_static("listener"), listener));

        let monitor = _get_monitor_metric(snapshots);
        attrs.push((ZStringOwned::from_static("monitor"), monitor));

        let processor = _get_processor_metric(snapshots);
        attrs.push((ZStringOwned::from_static("processor"), processor));

        let forwarder = _get_forwarder_metric(snapshots);
        attrs.push((ZStringOwned::from_static("forwarder"), forwarder));

        let forwarder_server = _get_forwarder_server_metric(snapshots);
        attrs.push((
            ZStringOwned::from_static("forwarder_server"),
            forwarder_server,
        ));

        let lookup_table = _get_lookup_table_metric(snapshots);
        attrs.push((ZStringOwned::from_static("lookup_table"), lookup_table));
    }
    Some(attrs)
}

#[cfg(test)]
mod lookup_table_tests {
    use super::*;

    fn field<'a>(value: &'a ValueOwned, name: &str) -> &'a ValueOwned {
        let ValueOwned::KvList(fields) = value else {
            panic!("expected a map")
        };
        &fields
            .iter()
            .find(|(key, _)| key.as_ref() == name)
            .unwrap()
            .1
    }

    #[test]
    fn skip_reasons_follow_throughput_mode_but_generation_and_health_are_absolute() {
        let previous = LookupTable {
            kind: "immutable",
            rows_skipped: SkipCounts {
                bad_cell: 10,
                ..Default::default()
            },
            ..Default::default()
        };
        let current = LookupTable {
            kind: "immutable",
            rows_skipped: SkipCounts {
                bad_cell: 20,
                ..Default::default()
            },
            generation_rows_skipped: SkipCounts {
                bad_cell: 10,
                ..Default::default()
            },
            generation: 2,
            last_success_timestamp: 1_700_000_000.5,
            load_error: true,
            ..Default::default()
        };
        let rate = _lookup_table_entry(
            &current,
            Mode::Rate {
                prev: Some(&previous),
                elapsed: 5.0,
            },
        );
        assert!(matches!(
            field(&rate, "rows_skipped"),
            ValueOwned::Double(2.0)
        ));
        assert!(matches!(
            field(field(&rate, "rows_skipped_by_reason"), "cast"),
            ValueOwned::Double(2.0)
        ));
        assert!(matches!(
            field(field(&rate, "rows_skipped_by_reason"), "overlap"),
            ValueOwned::Double(0.0)
        ));
        assert!(matches!(
            field(field(&rate, "generation_rows_skipped_by_reason"), "cast"),
            ValueOwned::Int64(10)
        ));
        assert!(matches!(field(&rate, "generation"), ValueOwned::Int64(2)));
        assert!(matches!(field(&rate, "load_error"), ValueOwned::Int64(1)));
        assert!(matches!(
            field(&rate, "last_success_timestamp"),
            ValueOwned::Double(1_700_000_000.5)
        ));

        let total = _lookup_table_entry(&current, Mode::Total);
        assert!(matches!(
            field(field(&total, "rows_skipped_by_reason"), "cast"),
            ValueOwned::Int64(20)
        ));
        let idle = _lookup_table_entry(
            &current,
            Mode::Rate {
                prev: Some(&current),
                elapsed: 5.0,
            },
        );
        assert!(matches!(
            field(field(&idle, "rows_skipped_by_reason"), "cast"),
            ValueOwned::Double(0.0)
        ));
        let new = _lookup_table_entry(
            &current,
            Mode::Rate {
                prev: None,
                elapsed: 5.0,
            },
        );
        assert!(matches!(
            field(field(&new, "rows_skipped_by_reason"), "cast"),
            ValueOwned::Double(4.0)
        ));
    }
}
