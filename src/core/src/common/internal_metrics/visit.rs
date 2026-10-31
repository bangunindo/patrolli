// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Encoder-agnostic description of the internal metrics.
//!
//! There are two renderers of [`super::INTERNAL_METRICS`]: the Prometheus text
//! exporter in [`crate::common::prometheus`], and the OTAP metrics signal in
//! [`crate::listener`]'s `internal_metrics`. Both need the same catalogue — name,
//! help text, unit, counter-vs-gauge, and label set — for every one of the ~115
//! families, so the catalogue lives here exactly once and each renderer only
//! decides how to spell it.
//!
//! [`walk`] is the whole catalogue. Family names are dotted
//! (`sink.time_spent`, `lookup_table.processor.lookups`); the Prometheus
//! renderer maps `.` back to `_` and lets its registry supply the `patrolli`
//! prefix and the unit/`_total` suffixes, while the OTAP renderer prefixes
//! `patrolli.` and emits UCUM units. Both spellings are derivable from the dotted
//! name, which is why the catalogue stores only that.

use super::{
    Channel, ChannelKind, Forwarder, ForwarderNode, ForwarderServer, InternalMetrics, Listener,
    LookupTable, LookupTableProcessor, Monitor, Processor, ProcessorEdge, ProcessorNode, Sink,
    SkipCounts,
};
use smallvec::SmallVec;
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::time::Duration;

/// Unit of a family, in the abstract. Prometheus spells these as its
/// `registry::Unit` (which also appends a name suffix); OTLP spells them UCUM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    None,
    Seconds,
    Bytes,
}

impl Unit {
    /// UCUM code, for OTLP's `unit` field. `None` for a dimensionless count.
    pub fn ucum(self) -> Option<&'static str> {
        match self {
            Unit::None => None,
            Unit::Seconds => Some("s"),
            Unit::Bytes => Some("By"),
        }
    }
}

/// Whether a family is a monotonic cumulative total or an instantaneous
/// reading. Deliberately only two: Prometheus' `Info` type is not used
/// anywhere, identity is carried as ordinary labels instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Counter,
    Gauge,
}

/// One metric family's descriptor.
///
/// `name` and `help` are `&'static str` because the catalogue is entirely
/// literals — that lets both renderers carry them without allocating.
#[derive(Debug, Clone, Copy)]
pub struct Family {
    /// Dotted logical name, without the `patrolli` prefix and without a unit suffix.
    pub name: &'static str,
    pub help: &'static str,
    pub unit: Unit,
    pub kind: Kind,
}

/// A label set, ordered so both renderers emit labels in declaration order
/// (the exporter's golden assertions depend on that order).
pub type Labels = SmallVec<[(&'static str, Cow<'static, str>); 3]>;

/// Build a [`Labels`] from a fixed-size array of pairs.
pub fn labels<const N: usize>(pairs: [(&'static str, Cow<'static, str>); N]) -> Labels {
    Labels::from_iter(pairs)
}

/// The samples of one family.
///
/// The `*Scalar` variants carry a single unlabeled reading and exist so the
/// Prometheus renderer can emit `patrolli_monitor_items_total 5` rather than
/// `patrolli_monitor_items_total{} 5` — an empty label set is not the same text as
/// no label set.
pub enum Samples<'a> {
    U64Scalar(u64),
    F64Scalar(f64),
    U64(&'a [(Labels, u64)]),
    F64(&'a [(Labels, f64)]),
}

/// A renderer of the catalogue.
pub trait Visitor {
    type Error;

    /// Emit one family. A family with an empty `U64`/`F64` sample slice is
    /// skipped by the caller, so implementations never see one.
    fn family(&mut self, family: Family, samples: Samples<'_>) -> Result<(), Self::Error>;
}

/// A snapshot entity paired with the labels that identify it.
struct Labeled<'a, T> {
    labels: Labels,
    item: &'a T,
}

// Hand-written so cloning a sample does not require `T: Clone`: only the labels
// are owned, the item is borrowed.
impl<T> Clone for Labeled<'_, T> {
    fn clone(&self) -> Self {
        Self {
            labels: self.labels.clone(),
            item: self.item,
        }
    }
}

/// One channel topic within one channel group. The per-topic counters live in
/// maps on the group's [`Channel`], so the sample has to carry both.
struct ChannelTopicSample<'a> {
    labels: Labels,
    topic: &'a str,
    metrics: &'a Channel,
}

fn scalar_u64<V: Visitor>(
    v: &mut V,
    name: &'static str,
    help: &'static str,
    unit: Unit,
    kind: Kind,
    value: u64,
) -> Result<(), V::Error> {
    v.family(
        Family {
            name,
            help,
            unit,
            kind,
        },
        Samples::U64Scalar(value),
    )
}

fn scalar_f64<V: Visitor>(
    v: &mut V,
    name: &'static str,
    help: &'static str,
    unit: Unit,
    kind: Kind,
    value: f64,
) -> Result<(), V::Error> {
    v.family(
        Family {
            name,
            help,
            unit,
            kind,
        },
        Samples::F64Scalar(value),
    )
}

fn family_u64<V: Visitor>(
    v: &mut V,
    name: &'static str,
    help: &'static str,
    unit: Unit,
    kind: Kind,
    samples: Vec<(Labels, u64)>,
) -> Result<(), V::Error> {
    if samples.is_empty() {
        return Ok(());
    }
    v.family(
        Family {
            name,
            help,
            unit,
            kind,
        },
        Samples::U64(&samples),
    )
}

fn family_f64<V: Visitor>(
    v: &mut V,
    name: &'static str,
    help: &'static str,
    unit: Unit,
    kind: Kind,
    samples: Vec<(Labels, f64)>,
) -> Result<(), V::Error> {
    if samples.is_empty() {
        return Ok(());
    }
    v.family(
        Family {
            name,
            help,
            unit,
            kind,
        },
        Samples::F64(&samples),
    )
}

fn counter_field<V, T, F>(
    v: &mut V,
    name: &'static str,
    help: &'static str,
    samples: &[Labeled<'_, T>],
    value: F,
) -> Result<(), V::Error>
where
    V: Visitor,
    F: Fn(&T) -> u64,
{
    family_u64(
        v,
        name,
        help,
        Unit::None,
        Kind::Counter,
        samples
            .iter()
            .map(|sample| (sample.labels.clone(), value(sample.item)))
            .collect(),
    )
}

fn duration_field<V, T, F>(
    v: &mut V,
    name: &'static str,
    help: &'static str,
    samples: &[Labeled<'_, T>],
    value: F,
) -> Result<(), V::Error>
where
    V: Visitor,
    F: Fn(&T) -> Duration,
{
    family_f64(
        v,
        name,
        help,
        Unit::Seconds,
        Kind::Counter,
        samples
            .iter()
            .map(|sample| (sample.labels.clone(), value(sample.item).as_secs_f64()))
            .collect(),
    )
}

fn gauge_field<V, T, F>(
    v: &mut V,
    name: &'static str,
    help: &'static str,
    unit: Unit,
    samples: &[Labeled<'_, T>],
    value: F,
) -> Result<(), V::Error>
where
    V: Visitor,
    F: Fn(&T) -> u64,
{
    family_u64(
        v,
        name,
        help,
        unit,
        Kind::Gauge,
        samples
            .iter()
            .map(|sample| (sample.labels.clone(), value(sample.item)))
            .collect(),
    )
}

macro_rules! duration_fields {
    ($v:expr, $samples:expr, [$(($name:literal, $help:literal, $field:ident)),+ $(,)?]) => {
        $(
            duration_field($v, $name, $help, $samples, |item| item.$field)?;
        )+
    };
}

macro_rules! counter_fields {
    ($v:expr, $samples:expr, [$(($name:literal, $help:literal, $field:ident)),+ $(,)?]) => {
        $(
            counter_field($v, $name, $help, $samples, |item| item.$field)?;
        )+
    };
}

macro_rules! duration_metrics {
    ($v:expr, $metrics:expr, [$(($name:literal, $help:literal, $field:ident)),+ $(,)?]) => {
        $(
            scalar_f64(
                $v,
                $name,
                $help,
                Unit::Seconds,
                Kind::Counter,
                $metrics.$field.as_secs_f64(),
            )?;
        )+
    };
}

macro_rules! counter_metrics {
    ($v:expr, $metrics:expr, [$(($name:literal, $help:literal, $field:ident)),+ $(,)?]) => {
        $(
            scalar_u64($v, $name, $help, Unit::None, Kind::Counter, $metrics.$field)?;
        )+
    };
}

/// Walk every internal-metrics family, handing each to `v`.
///
/// `include_forwarder` gates the cluster-only families (the inter-node
/// forwarder, its peers, and the forwarder server), which are all zero in local
/// mode and would only be noise there.
pub fn walk<V: Visitor>(
    metrics: &InternalMetrics,
    include_forwarder: bool,
    v: &mut V,
) -> Result<(), V::Error> {
    let sink_channel = metrics.sink_channel.load_full();
    let processor_channel = metrics.processor_channel.load_full();
    let sink = metrics.sink.load_full();
    let listener = metrics.listener.load_full();
    let monitor = metrics.monitor.load_full();
    let processor = metrics.processor.load_full();
    let lookup_table = metrics.lookup_table.load_full();
    let forwarder_channel = include_forwarder.then(|| metrics.forwarder_channel.load_full());
    let forwarder = include_forwarder.then(|| metrics.forwarder.load_full());
    let forwarder_server = include_forwarder.then(|| metrics.forwarder_server.load_full());

    let mut channel_groups = vec![
        ("sink", &sink_channel.1),
        ("processor", &processor_channel.1),
    ];
    if let Some(forwarder_channel) = &forwarder_channel {
        channel_groups.push(("forwarder", &forwarder_channel.1));
    }

    walk_channels(v, &channel_groups)?;
    walk_sinks(v, &sink.1)?;
    walk_listeners(v, &listener.1)?;
    walk_monitor(v, &monitor.1)?;
    walk_processor(v, &processor.1)?;
    walk_lookup_tables(v, &lookup_table.1)?;

    let mut ages = vec![
        ("sink_channel", sink_channel.0.elapsed().as_secs_f64()),
        (
            "processor_channel",
            processor_channel.0.elapsed().as_secs_f64(),
        ),
        ("sink", sink.0.elapsed().as_secs_f64()),
        ("listener", listener.0.elapsed().as_secs_f64()),
        ("monitor", monitor.0.elapsed().as_secs_f64()),
        ("processor", processor.0.elapsed().as_secs_f64()),
        ("lookup_table", lookup_table.0.elapsed().as_secs_f64()),
    ];

    if let Some(forwarder) = &forwarder {
        walk_forwarder(v, &forwarder.1)?;
    }
    if let Some(forwarder_server) = &forwarder_server {
        walk_forwarder_server(v, &forwarder_server.1)?;
    }
    if let Some(forwarder_channel) = &forwarder_channel {
        ages.push((
            "forwarder_channel",
            forwarder_channel.0.elapsed().as_secs_f64(),
        ));
    }
    if let Some(forwarder) = &forwarder {
        ages.push(("forwarder", forwarder.0.elapsed().as_secs_f64()));
    }
    if let Some(forwarder_server) = &forwarder_server {
        ages.push((
            "forwarder_server",
            forwarder_server.0.elapsed().as_secs_f64(),
        ));
    }

    family_f64(
        v,
        "internal_metrics.snapshot_age",
        "Age of the most recently published internal-metrics snapshot.",
        Unit::Seconds,
        Kind::Gauge,
        ages.into_iter()
            .map(|(snapshot, age)| (labels([("snapshot", Cow::Borrowed(snapshot))]), age))
            .collect(),
    )
}

fn walk_channels<V: Visitor>(
    v: &mut V,
    channels: &[(&'static str, &Channel)],
) -> Result<(), V::Error> {
    family_u64(
        v,
        "channel.messages",
        "Messages processed by a channel group.",
        Unit::None,
        Kind::Counter,
        channels
            .iter()
            .map(|(group, channel)| {
                (
                    labels([("channel_group", Cow::Borrowed(*group))]),
                    channel.total_messages as u64,
                )
            })
            .collect(),
    )?;
    family_u64(
        v,
        "channel.batches",
        "Batches processed by a channel group.",
        Unit::None,
        Kind::Counter,
        channels
            .iter()
            .map(|(group, channel)| {
                (
                    labels([("channel_group", Cow::Borrowed(*group))]),
                    channel.total_batches as u64,
                )
            })
            .collect(),
    )?;
    family_f64(
        v,
        "channel.time_spent",
        "Time spent processing channel group operations.",
        Unit::Seconds,
        Kind::Counter,
        channels
            .iter()
            .map(|(group, channel)| {
                (
                    labels([("channel_group", Cow::Borrowed(*group))]),
                    channel.total_time_spent.as_secs_f64(),
                )
            })
            .collect(),
    )?;

    let mut state_samples = Vec::new();
    let mut topic_samples = Vec::new();
    for &(group, channel) in channels {
        let mut states: Vec<_> = channel.channels.iter().collect();
        states.sort_unstable_by(|left, right| left.0.cmp(right.0));
        state_samples.extend(states.into_iter().map(|(topic, state)| Labeled {
            labels: labels([
                ("channel_group", Cow::Borrowed(group)),
                ("channel", Cow::Owned(topic.clone())),
            ]),
            item: state,
        }));

        let mut topics = BTreeSet::new();
        topics.extend(channel.channels.keys().map(String::as_str));
        topics.extend(channel.messages_sent.keys().map(String::as_str));
        topics.extend(channel.messages_dropped.keys().map(String::as_str));
        topics.extend(channel.batches_sent.keys().map(String::as_str));
        topics.extend(channel.batches_dropped.keys().map(String::as_str));
        topics.extend(channel.messages_lost.keys().map(String::as_str));
        topics.extend(channel.batches_lost.keys().map(String::as_str));
        topics.extend(channel.send_wait_time.keys().map(String::as_str));
        topic_samples.extend(topics.into_iter().map(|topic| ChannelTopicSample {
            labels: labels([
                ("channel_group", Cow::Borrowed(group)),
                ("channel", Cow::Owned(topic.to_string())),
            ]),
            topic,
            metrics: channel,
        }));
    }

    family_u64(
        v,
        "channel.queue_length",
        "Current number of batches queued in a memory channel.",
        Unit::None,
        Kind::Gauge,
        state_samples
            .iter()
            .filter(|sample| sample.item.kind == ChannelKind::Memory)
            .map(|sample| (sample.labels.clone(), sample.item.length))
            .collect(),
    )?;
    family_u64(
        v,
        "channel.queue_capacity",
        "Configured batch capacity of a memory channel.",
        Unit::None,
        Kind::Gauge,
        state_samples
            .iter()
            .filter(|sample| sample.item.kind == ChannelKind::Memory)
            .map(|sample| (sample.labels.clone(), sample.item.capacity))
            .collect(),
    )?;
    family_u64(
        v,
        "channel.disk_usage",
        "Current disk space used by a disk channel.",
        Unit::Bytes,
        Kind::Gauge,
        state_samples
            .iter()
            .filter(|sample| sample.item.kind == ChannelKind::Disk)
            .map(|sample| (sample.labels.clone(), sample.item.length))
            .collect(),
    )?;
    family_u64(
        v,
        "channel.disk_limit",
        "Configured maximum bytes of payloads a disk channel holds; its files run past it until compaction drops what was delivered.",
        Unit::Bytes,
        Kind::Gauge,
        state_samples
            .iter()
            .filter(|sample| sample.item.kind == ChannelKind::Disk)
            .map(|sample| (sample.labels.clone(), sample.item.capacity))
            .collect(),
    )?;

    family_u64(
        v,
        "channel.messages_sent",
        "Messages successfully sent to a channel.",
        Unit::None,
        Kind::Counter,
        topic_samples
            .iter()
            .map(|sample| {
                (
                    sample.labels.clone(),
                    *sample.metrics.messages_sent.get(sample.topic).unwrap_or(&0) as u64,
                )
            })
            .collect(),
    )?;
    family_u64(
        v,
        "channel.messages_dropped",
        "Messages dropped while sending to a channel.",
        Unit::None,
        Kind::Counter,
        topic_samples
            .iter()
            .map(|sample| {
                (
                    sample.labels.clone(),
                    *sample
                        .metrics
                        .messages_dropped
                        .get(sample.topic)
                        .unwrap_or(&0) as u64,
                )
            })
            .collect(),
    )?;
    family_u64(
        v,
        "channel.batches_sent",
        "Batches successfully sent to a channel.",
        Unit::None,
        Kind::Counter,
        topic_samples
            .iter()
            .map(|sample| {
                (
                    sample.labels.clone(),
                    *sample.metrics.batches_sent.get(sample.topic).unwrap_or(&0) as u64,
                )
            })
            .collect(),
    )?;
    family_u64(
        v,
        "channel.batches_dropped",
        "Batches dropped while sending to a channel.",
        Unit::None,
        Kind::Counter,
        topic_samples
            .iter()
            .map(|sample| {
                (
                    sample.labels.clone(),
                    *sample
                        .metrics
                        .batches_dropped
                        .get(sample.topic)
                        .unwrap_or(&0) as u64,
                )
            })
            .collect(),
    )?;
    family_u64(
        v,
        "channel.messages_lost",
        "Messages a channel accepted, then lost: a disk write that failed with no consumer left to take them.",
        Unit::None,
        Kind::Counter,
        topic_samples
            .iter()
            .map(|sample| {
                (
                    sample.labels.clone(),
                    *sample.metrics.messages_lost.get(sample.topic).unwrap_or(&0) as u64,
                )
            })
            .collect(),
    )?;
    family_u64(
        v,
        "channel.batches_lost",
        "Batches a channel accepted, then lost: a disk value or key that does not decode, or a disk write that failed with no consumer left to take it.",
        Unit::None,
        Kind::Counter,
        topic_samples
            .iter()
            .map(|sample| {
                (
                    sample.labels.clone(),
                    *sample.metrics.batches_lost.get(sample.topic).unwrap_or(&0) as u64,
                )
            })
            .collect(),
    )?;
    family_f64(
        v,
        "channel.send_wait_time",
        "Time spent waiting to send to a channel.",
        Unit::Seconds,
        Kind::Counter,
        topic_samples
            .iter()
            .map(|sample| {
                (
                    sample.labels.clone(),
                    sample
                        .metrics
                        .send_wait_time
                        .get(sample.topic)
                        .copied()
                        .unwrap_or_default()
                        .as_secs_f64(),
                )
            })
            .collect(),
    )
}

fn walk_sinks<V: Visitor>(v: &mut V, sinks: &[Sink]) -> Result<(), V::Error> {
    let mut sinks: Vec<_> = sinks.iter().collect();
    sinks.sort_unstable_by_key(|sink| sink.id);
    let samples: Vec<_> = sinks
        .into_iter()
        .map(|sink| Labeled {
            labels: labels([
                ("sink_id", Cow::Owned(sink.id.to_string())),
                ("sink_name", Cow::Owned(sink.name.clone())),
            ]),
            item: sink,
        })
        .collect();

    gauge_field(
        v,
        "sink.workers",
        "Current number of workers for a sink.",
        Unit::None,
        &samples,
        |sink| sink.total_workers as u64,
    )?;
    duration_fields!(
        v,
        &samples,
        [
            (
                "sink.time_spent",
                "Time spent processing sink work.",
                total_time_spent
            ),
            (
                "sink.engine_time_spent",
                "Time spent running the sink engine.",
                engine_time_spent
            ),
            (
                "sink.receive_wait_time",
                "Time spent waiting to receive sink work.",
                receive_wait_time
            ),
            (
                "sink.send_wait_time",
                "Time spent waiting to send sink output.",
                send_wait_time
            )
        ]
    );
    counter_fields!(
        v,
        &samples,
        [
            ("sink.records", "Records received by a sink.", total_records),
            (
                "sink.records_sent",
                "Records successfully sent by a sink.",
                total_records_sent
            ),
            (
                "sink.records_filtered",
                "Records filtered by a sink.",
                total_records_filtered
            ),
            (
                "sink.records_errored",
                "Records that encountered a sink error.",
                total_records_errored
            ),
            (
                "sink.records_retried",
                "Records a sink sent again after a failed attempt, once per attempt.",
                total_records_retried
            )
        ]
    );
    gauge_field(
        v,
        "sink.records_pending_retry",
        "Records a sink holds for its next attempt.",
        Unit::None,
        &samples,
        |sink| sink.records_pending_retry,
    )?;
    family_u64(
        v,
        "sink.endpoint_up",
        "Whether a sink's endpoint takes sends (1) or is backing off after a failure (0).",
        Unit::None,
        Kind::Gauge,
        samples
            .iter()
            .flat_map(|sample| {
                sample.item.endpoints.iter().map(|endpoint| {
                    let mut labels = sample.labels.clone();
                    labels.push(("endpoint", Cow::Owned(endpoint.endpoint.clone())));
                    (labels, u64::from(endpoint.up))
                })
            })
            .collect(),
    )?;
    Ok(())
}

fn walk_listeners<V: Visitor>(v: &mut V, listeners: &[Listener]) -> Result<(), V::Error> {
    let mut listeners: Vec<_> = listeners.iter().collect();
    listeners.sort_unstable_by_key(|listener| listener.id);
    let samples: Vec<_> = listeners
        .into_iter()
        .map(|listener| Labeled {
            labels: labels([
                ("listener_id", Cow::Owned(listener.id.to_string())),
                ("listener_name", Cow::Owned(listener.name.clone())),
            ]),
            item: listener,
        })
        .collect();

    gauge_field(
        v,
        "listener.workers",
        "Current number of workers for a listener.",
        Unit::None,
        &samples,
        |listener| listener.total_workers as u64,
    )?;
    duration_fields!(
        v,
        &samples,
        [
            (
                "listener.time_spent",
                "Time spent processing listener work.",
                total_time_spent
            ),
            (
                "listener.engine_time_spent",
                "Time spent running the listener engine.",
                engine_time_spent
            ),
            (
                "listener.receive_wait_time",
                "Time spent waiting to receive listener work.",
                receive_wait_time
            ),
            (
                "listener.send_wait_time",
                "Time spent waiting to send listener output.",
                send_wait_time
            )
        ]
    );
    counter_fields!(
        v,
        &samples,
        [
            (
                "listener.bytes",
                "Bytes received by a listener.",
                total_bytes
            ),
            (
                "listener.bytes_succeeded",
                "Bytes successfully processed by a listener.",
                total_bytes_succeeded
            ),
            (
                "listener.bytes_dropped",
                "Bytes dropped by a listener.",
                total_bytes_dropped
            ),
            (
                "listener.bytes_errored",
                "Bytes that encountered a listener error.",
                total_bytes_errored
            ),
            (
                "listener.items",
                "Items received by a listener.",
                total_items
            ),
            (
                "listener.items_succeeded",
                "Items successfully processed by a listener.",
                total_items_succeeded
            ),
            (
                "listener.items_dropped",
                "Items dropped by a listener.",
                total_items_dropped
            ),
            (
                "listener.items_errored",
                "Items that encountered a listener error.",
                total_items_errored
            ),
            (
                "listener.records",
                "Records produced by a listener.",
                total_records
            ),
            (
                "listener.records_sent",
                "Records successfully sent by a listener.",
                total_records_sent
            ),
            (
                "listener.records_forwarded",
                "Records forwarded by a listener.",
                total_records_forwarded
            ),
            (
                "listener.records_forward_failed",
                "Records that failed listener forwarding.",
                total_records_forward_failed
            ),
            (
                "listener.records_annotate_failed",
                "Records that failed listener annotation.",
                total_records_annotate_failed
            ),
            (
                "listener.records_dropped",
                "Records dropped by a listener.",
                total_records_dropped
            ),
            (
                "listener.records_errored",
                "Records that encountered a listener error.",
                total_records_errored
            )
        ]
    );
    Ok(())
}

fn walk_monitor<V: Visitor>(v: &mut V, monitor: &Monitor) -> Result<(), V::Error> {
    scalar_u64(
        v,
        "monitor.workers",
        "Current number of monitor workers.",
        Unit::None,
        Kind::Gauge,
        monitor.total_workers as u64,
    )?;
    duration_metrics!(
        v,
        monitor,
        [
            (
                "monitor.time_spent",
                "Time spent processing monitor work.",
                total_time_spent
            ),
            (
                "monitor.engine_time_spent",
                "Time spent running the monitor engine.",
                engine_time_spent
            ),
            (
                "monitor.send_wait_time",
                "Time spent waiting to send monitor output.",
                send_wait_time
            ),
            (
                "monitor.receive_wait_time",
                "Time spent waiting to receive monitor work.",
                receive_wait_time
            )
        ]
    );
    counter_metrics!(
        v,
        monitor,
        [
            ("monitor.items", "Items processed by monitors.", total_items),
            (
                "monitor.items_dropped",
                "Items dropped by monitors.",
                total_items_dropped
            ),
            (
                "monitor.items_errored",
                "Items that encountered a monitor error.",
                total_items_errored
            ),
            (
                "monitor.items_retry_dropped",
                "Retried items ultimately dropped by monitors.",
                total_items_retry_dropped
            )
        ]
    );
    Ok(())
}

fn walk_processor<V: Visitor>(v: &mut V, processor: &Processor) -> Result<(), V::Error> {
    scalar_u64(
        v,
        "processor.workers",
        "Current number of processor workers.",
        Unit::None,
        Kind::Gauge,
        processor.total_workers as u64,
    )?;
    duration_metrics!(
        v,
        processor,
        [
            (
                "processor.time_spent",
                "Time spent processing processor work.",
                total_time_spent
            ),
            (
                "processor.engine_time_spent",
                "Time spent running the processor engine.",
                engine_time_spent
            ),
            (
                "processor.send_wait_time",
                "Time spent waiting to send processor output.",
                send_wait_time
            ),
            (
                "processor.receive_wait_time",
                "Time spent waiting to receive processor work.",
                receive_wait_time
            )
        ]
    );
    counter_metrics!(
        v,
        processor,
        [
            (
                "processor.records",
                "Records processed by processors.",
                total_records
            ),
            (
                "processor.records_sent",
                "Records successfully sent by processors.",
                total_records_sent
            ),
            (
                "processor.records_filtered",
                "Records filtered by processors.",
                total_records_filtered
            ),
            (
                "processor.records_errored",
                "Records that encountered a processor error.",
                total_records_errored
            ),
            (
                "processor.records_unrouted",
                "Records dropped with no processor or sink downstream.",
                total_records_unrouted
            )
        ]
    );

    walk_processor_nodes(v, &processor.nodes)?;
    walk_processor_edges(v, &processor.edges)
}

fn walk_processor_nodes<V: Visitor>(v: &mut V, nodes: &[ProcessorNode]) -> Result<(), V::Error> {
    let mut nodes: Vec<_> = nodes.iter().collect();
    nodes.sort_unstable_by_key(|node| node.id);
    let samples: Vec<_> = nodes
        .into_iter()
        .map(|node| Labeled {
            labels: labels([("node_id", Cow::Owned(node.id.to_string()))]),
            item: node,
        })
        .collect();

    duration_fields!(
        v,
        &samples,
        [
            (
                "processor.node.time_spent",
                "Time spent processing work in a processor node.",
                total_time_spent
            ),
            (
                "processor.node.engine_time_spent",
                "Time spent running a processor node engine.",
                engine_time_spent
            ),
            (
                "processor.node.send_wait_time",
                "Time spent waiting to send processor node output.",
                send_wait_time
            )
        ]
    );
    counter_fields!(
        v,
        &samples,
        [
            (
                "processor.node.records",
                "Records processed by a processor node.",
                total_records
            ),
            (
                "processor.node.records_filtered",
                "Records filtered by a processor node.",
                total_records_filtered
            ),
            (
                "processor.node.records_errored",
                "Records that encountered a processor node error.",
                total_records_errored
            )
        ]
    );
    Ok(())
}

fn walk_processor_edges<V: Visitor>(v: &mut V, edges: &[ProcessorEdge]) -> Result<(), V::Error> {
    let mut edges: Vec<_> = edges.iter().collect();
    edges.sort_unstable_by_key(|edge| (edge.source, edge.target, edge.target_is_sink));
    family_u64(
        v,
        "processor.edge.records",
        "Records sent across a processor edge.",
        Unit::None,
        Kind::Counter,
        edges
            .into_iter()
            .map(|edge| {
                (
                    labels([
                        ("source_id", Cow::Owned(edge.source.to_string())),
                        ("target_id", Cow::Owned(edge.target.to_string())),
                        (
                            "target_kind",
                            Cow::Borrowed(if edge.target_is_sink { "sink" } else { "node" }),
                        ),
                    ]),
                    edge.total_records,
                )
            })
            .collect(),
    )
}

/// The seven row-rejection reasons, in the order [`SkipCounts`] declares them,
/// paired with the label value each is spelled as. The label spellings match
/// `SkipCounts::summary`, so a log line and a metric name the same reason.
type SkipReason = (&'static str, fn(&SkipCounts) -> u64);

pub(crate) const SKIP_REASONS: [SkipReason; 8] = [
    ("field_count", |c| c.arity),
    ("cast", |c| c.bad_cell),
    ("range_bound", |c| c.bad_bound),
    ("null_key", |c| c.null_key),
    ("duplicate_key", |c| c.duplicate_key),
    ("overlap", |c| c.overlap),
    ("empty_range", |c| c.empty_range),
    ("family_mismatch", |c| c.mismatch),
];

/// One family per skip reason would multiply the catalogue by seven, so the
/// reason is a label instead. Emit zeros too: counters need a baseline before
/// their first failure, and generation gauges must report clean reloads.
fn skip_family<V, F>(
    v: &mut V,
    name: &'static str,
    help: &'static str,
    kind: Kind,
    tables: &[&LookupTable],
    counts: F,
) -> Result<(), V::Error>
where
    V: Visitor,
    F: Fn(&LookupTable) -> &SkipCounts,
{
    let samples: Vec<_> = tables
        .iter()
        .flat_map(|table| {
            let skipped = counts(table);
            SKIP_REASONS.iter().map(move |(reason, read)| {
                (
                    labels([
                        ("table_id", Cow::Owned(table.id.to_string())),
                        ("table_name", Cow::Owned(table.name.clone())),
                        ("reason", Cow::Borrowed(*reason)),
                    ]),
                    read(skipped),
                )
            })
        })
        .collect();
    family_u64(v, name, help, Unit::None, kind, samples)
}

fn walk_lookup_tables<V: Visitor>(v: &mut V, tables: &[LookupTable]) -> Result<(), V::Error> {
    let mut tables: Vec<_> = tables.iter().collect();
    tables.sort_unstable_by_key(|table| table.id);
    let samples: Vec<_> = tables
        .iter()
        .map(|table| Labeled {
            labels: labels([
                ("table_id", Cow::Owned(table.id.to_string())),
                ("table_name", Cow::Owned(table.name.clone())),
                ("kind", Cow::Borrowed(table.kind)),
            ]),
            item: *table,
        })
        .collect();

    gauge_field(
        v,
        "lookup_table.rows",
        "Rows currently stored in a lookup table.",
        Unit::None,
        &samples,
        |table| table.rows,
    )?;
    gauge_field(
        v,
        "lookup_table.index_entries",
        "Index entries currently stored in a lookup table. Exceeds the row \
         count when several keys or ranges share one row.",
        Unit::None,
        &samples,
        |table| table.index_entries,
    )?;
    gauge_field(
        v,
        "lookup_table.generation",
        "Current lookup table generation. Zero before the first successful load; one for mutable tables.",
        Unit::None,
        &samples,
        |table| table.generation,
    )?;
    // Both kinds hold strings, so this sits outside the disjoint halves below,
    // but they pool different populations: an immutable table interns every
    // distinct string-column value, a mutable one only those too long to live
    // inline. Comparable for one table over time, not between kinds.
    gauge_field(
        v,
        "lookup_table.pooled_strings",
        "Distinct strings held by a lookup table: an immutable table's per-generation interner, or a mutable table's shared pools.",
        Unit::None,
        &samples,
        |table| table.pooled_strings,
    )?;

    // The two kinds use disjoint halves of the snapshot, so each half is
    // emitted only for the kind it belongs to rather than as a permanent zero
    // series on the other.
    let mutable: Vec<_> = samples
        .iter()
        .filter(|sample| sample.item.kind == "mutable")
        .cloned()
        .collect();
    let immutable: Vec<_> = samples
        .iter()
        .filter(|sample| sample.item.kind == "immutable")
        .cloned()
        .collect();
    let loaded: Vec<_> = tables
        .iter()
        .copied()
        .filter(|table| table.kind == "immutable")
        .collect();
    gauge_field(
        v,
        "lookup_table.load_error",
        "Whether the most recent lookup table load attempt failed, including when old data remains available.",
        Unit::None,
        &immutable,
        |table| u64::from(table.load_error),
    )?;
    family_f64(
        v,
        "lookup_table.last_success_timestamp",
        "Unix timestamp of the last successful lookup table load. Zero if no load has succeeded.",
        Unit::Seconds,
        Kind::Gauge,
        immutable
            .iter()
            .map(|sample| (sample.labels.clone(), sample.item.last_success_timestamp))
            .collect(),
    )?;
    gauge_field(
        v,
        "lookup_table.max_rows",
        "Configured hard row cap of a mutable lookup table.",
        Unit::None,
        &mutable,
        |table| table.max_rows,
    )?;
    counter_fields!(
        v,
        &mutable,
        [
            (
                "lookup_table.rows_rejected",
                "Writes to a mutable lookup table turned away by its row cap.",
                rows_rejected
            ),
            (
                "lookup_table.rows_expired",
                "Rows dropped from a mutable lookup table by the TTL sweep.",
                rows_expired
            ),
            (
                "lookup_table.strings_released",
                "Pool entries released by a mutable lookup table's compaction.",
                strings_released
            )
        ]
    );

    counter_fields!(
        v,
        &immutable,
        [
            (
                "lookup_table.loads",
                "Loads of a lookup table that installed a new generation.",
                loads
            ),
            (
                "lookup_table.load_failures",
                "Load attempts for a lookup table that failed, leaving the \
                 previous generation in place.",
                load_failures
            ),
            (
                "lookup_table.rows_read",
                "Source rows read across every load of a lookup table.",
                rows_read
            )
        ]
    );
    duration_fields!(
        v,
        &immutable,
        [(
            "lookup_table.load_duration",
            "Time spent parsing and indexing a lookup table, across every load.",
            load_duration
        )]
    );
    skip_family(
        v,
        "lookup_table.rows_skipped",
        "Source rows a lookup table rejected, by reason, across every load.",
        Kind::Counter,
        &loaded,
        |table| &table.rows_skipped,
    )?;

    // The generation families describe the data actually in memory rather than
    // a running total, so they are gauges: a reload replaces them outright.
    gauge_field(
        v,
        "lookup_table.generation.rows_read",
        "Source rows read to build the lookup table generation now in memory.",
        Unit::None,
        &immutable,
        |table| table.generation_rows_read,
    )?;
    family_f64(
        v,
        "lookup_table.generation.load_duration",
        "Time spent building the lookup table generation now in memory.",
        Unit::Seconds,
        Kind::Gauge,
        immutable
            .iter()
            .map(|sample| {
                (
                    sample.labels.clone(),
                    sample.item.generation_load_duration.as_secs_f64(),
                )
            })
            .collect(),
    )?;
    skip_family(
        v,
        "lookup_table.generation.rows_skipped",
        "Source rows rejected while building the lookup table generation now \
         in memory, by reason.",
        Kind::Gauge,
        &loaded,
        |table| &table.generation_rows_skipped,
    )?;

    walk_lookup_table_processors(v, &tables)
}

/// One sample per (table, processor) pair. A processor appears only once it has
/// actually touched the table, so an unused table carries no per-processor
/// series at all.
fn walk_lookup_table_processors<V: Visitor>(
    v: &mut V,
    tables: &[&LookupTable],
) -> Result<(), V::Error> {
    let samples: Vec<Labeled<'_, LookupTableProcessor>> = tables
        .iter()
        .flat_map(|table| {
            table.processors.iter().map(move |processor| Labeled {
                labels: labels([
                    ("table_id", Cow::Owned(table.id.to_string())),
                    ("table_name", Cow::Owned(table.name.clone())),
                    ("node_id", Cow::Owned(processor.id.to_string())),
                ]),
                item: processor,
            })
        })
        .collect();

    counter_fields!(
        v,
        &samples,
        [
            (
                "lookup_table.processor.lookups",
                "Lookups a processor made against a lookup table, hits, misses \
                 and key errors together.",
                lookups
            ),
            (
                "lookup_table.processor.lookups_hit",
                "Lookups a processor made against a lookup table that matched a row.",
                lookups_hit
            ),
            (
                "lookup_table.processor.lookups_missed",
                "Lookups a processor made against a lookup table that matched nothing.",
                lookups_missed
            ),
            (
                "lookup_table.processor.lookups_errored",
                "Records a processor could not build a lookup key from at all.",
                lookups_errored
            ),
            (
                "lookup_table.processor.writes",
                "Writes a processor made to a mutable lookup table, of every outcome.",
                writes
            ),
            (
                "lookup_table.processor.writes_inserted",
                "Writes by a processor that added a row to a mutable lookup table.",
                writes_inserted
            ),
            (
                "lookup_table.processor.writes_updated",
                "Writes by a processor that refreshed an existing row of a \
                 mutable lookup table.",
                writes_updated
            ),
            (
                "lookup_table.processor.writes_rejected",
                "Writes by a processor turned away by a mutable lookup table's row cap.",
                writes_rejected
            ),
            (
                "lookup_table.processor.writes_errored",
                "Records a processor could not build a lookup table row from at all.",
                writes_errored
            ),
            (
                "lookup_table.processor.removes",
                "Rows a processor deleted from a mutable lookup table.",
                removes
            )
        ]
    );
    Ok(())
}

fn walk_forwarder<V: Visitor>(v: &mut V, forwarder: &Forwarder) -> Result<(), V::Error> {
    duration_metrics!(
        v,
        forwarder,
        [
            (
                "forwarder.time_spent",
                "Time spent processing forwarder work.",
                total_time_spent
            ),
            (
                "forwarder.engine_time_spent",
                "Time spent running the forwarder engine.",
                engine_time_spent
            ),
            (
                "forwarder.send_wait_time",
                "Time spent waiting to send forwarder output.",
                send_wait_time
            ),
            (
                "forwarder.receive_wait_time",
                "Time spent waiting to receive forwarder work.",
                receive_wait_time
            )
        ]
    );
    counter_metrics!(
        v,
        forwarder,
        [
            (
                "forwarder.records",
                "Records processed by the forwarder.",
                total_records
            ),
            (
                "forwarder.records_sent",
                "Records successfully sent by the forwarder.",
                total_records_sent
            ),
            (
                "forwarder.records_dropped",
                "Records dropped by the forwarder.",
                total_records_dropped
            ),
            (
                "forwarder.records_errored",
                "Records that encountered a forwarder error.",
                total_records_errored
            )
        ]
    );
    walk_forwarder_peers(v, &forwarder.nodes)
}

fn walk_forwarder_peers<V: Visitor>(v: &mut V, peers: &[ForwarderNode]) -> Result<(), V::Error> {
    let mut peers: Vec<_> = peers.iter().collect();
    peers.sort_unstable_by_key(|peer| peer.id);
    let samples: Vec<_> = peers
        .into_iter()
        .map(|peer| Labeled {
            labels: labels([("peer_id", Cow::Owned(peer.id.to_string()))]),
            item: peer,
        })
        .collect();

    gauge_field(
        v,
        "forwarder.peer.connections",
        "Current number of live connections to a forwarder peer.",
        Unit::None,
        &samples,
        |peer| peer.total_connections as u64,
    )?;
    duration_fields!(
        v,
        &samples,
        [
            (
                "forwarder.peer.time_spent",
                "Time spent processing work for a forwarder peer.",
                total_time_spent
            ),
            (
                "forwarder.peer.engine_time_spent",
                "Time spent running the engine for a forwarder peer.",
                engine_time_spent
            ),
            (
                "forwarder.peer.send_wait_time",
                "Time spent waiting to send to a forwarder peer.",
                send_wait_time
            ),
            (
                "forwarder.peer.receive_wait_time",
                "Time spent waiting to receive from a forwarder peer.",
                receive_wait_time
            )
        ]
    );
    counter_fields!(
        v,
        &samples,
        [
            (
                "forwarder.peer.records",
                "Records processed for a forwarder peer.",
                total_records
            ),
            (
                "forwarder.peer.records_sent",
                "Records successfully sent to a forwarder peer.",
                total_records_sent
            ),
            (
                "forwarder.peer.records_dropped",
                "Records dropped for a forwarder peer.",
                total_records_dropped
            ),
            (
                "forwarder.peer.records_errored",
                "Records that encountered a forwarder peer error.",
                total_records_errored
            ),
            (
                "forwarder.peer.bytes",
                "Bytes processed for a forwarder peer.",
                total_bytes
            ),
            (
                "forwarder.peer.bytes_sent",
                "Bytes successfully sent to a forwarder peer.",
                total_bytes_sent
            ),
            (
                "forwarder.peer.bytes_dropped",
                "Bytes dropped for a forwarder peer.",
                total_bytes_dropped
            ),
            (
                "forwarder.peer.bytes_errored",
                "Bytes that encountered a forwarder peer error.",
                total_bytes_errored
            )
        ]
    );
    Ok(())
}

fn walk_forwarder_server<V: Visitor>(v: &mut V, server: &ForwarderServer) -> Result<(), V::Error> {
    duration_metrics!(
        v,
        server,
        [
            (
                "forwarder.server.engine_time_spent",
                "Time spent running the forwarder server engine.",
                engine_time_spent
            ),
            (
                "forwarder.server.send_wait_time",
                "Time spent waiting to send forwarder server output.",
                send_wait_time
            )
        ]
    );
    counter_metrics!(
        v,
        server,
        [
            (
                "forwarder.server.records",
                "Records processed by the forwarder server.",
                total_records
            ),
            (
                "forwarder.server.records_sent",
                "Records successfully sent by the forwarder server.",
                total_records_sent
            ),
            (
                "forwarder.server.records_errored",
                "Records that encountered a forwarder server error.",
                total_records_errored
            ),
            (
                "forwarder.server.bytes",
                "Bytes processed by the forwarder server.",
                total_bytes
            )
        ]
    );
    Ok(())
}
