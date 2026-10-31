// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The metrics-signal renderer for the `internal_metrics` listener.
//!
//! Reads the shared catalogue in [`crate::common::internal_metrics::visit`] —
//! the same one the Prometheus exporter renders — and turns each family into one
//! [`MetricRowOwned`] carrying one data point per label set. Counters become
//! cumulative monotonic sums and gauges become gauges, so rates are the storage
//! backend's job; that is why this path has no `report_throughput`.

use crate::common;
use crate::common::internal_metrics::{InternalMetrics, visit};
use crate::config;
use crate::payload::{
    DataPointsOwned, GaugeOwned, MetricRowOwned, NumberDataPointOwned, NumberValue, SumOwned,
    ValueOwned, ZStringOwned,
};
use chrono::{DateTime, Utc};
use otel_arrow_dfe_pdata::otlp::metrics::MetricType;
use otel_arrow_dfe_pdata::proto::opentelemetry::metrics::v1::AggregationTemporality;
use smol_str::SmolStr;
use std::borrow::Cow;
use std::convert::Infallible;

/// Prepended to every catalogue name. The scope already says `patrolli.monitor`,
/// but a metric name has to stand on its own in a backend that indexes by name.
const NAME_PREFIX: &str = "patrolli.";

/// Rough family count, to size the row vector in one go.
const EXPECTED_FAMILIES: usize = 128;

/// Renders the catalogue as OTLP metric rows.
struct OtapVisitor {
    /// One timestamp for the whole tick, so every data point in the batch agrees.
    time: DateTime<Utc>,
    /// Process start: the beginning of the accumulation window shared by every
    /// cumulative sum, since all of these counters run from startup.
    start_time: DateTime<Utc>,
    rows: Vec<MetricRowOwned>,
}

impl OtapVisitor {
    fn new(time: DateTime<Utc>, start_time: DateTime<Utc>) -> Self {
        Self {
            time,
            start_time,
            rows: Vec::with_capacity(EXPECTED_FAMILIES),
        }
    }

    fn data_point(
        &self,
        kind: visit::Kind,
        value: NumberValue,
        attrs: Vec<(ZStringOwned, ValueOwned)>,
    ) -> NumberDataPointOwned {
        NumberDataPointOwned {
            // A gauge is an instantaneous reading, so it has no window.
            start_time: matches!(kind, visit::Kind::Counter).then_some(self.start_time),
            time: self.time,
            value,
            flags: 0,
            attrs,
            exemplars: Vec::new(),
        }
    }

    fn push(
        &mut self,
        family: visit::Family,
        data_points: Vec<NumberDataPointOwned>,
    ) -> Result<(), Infallible> {
        // `r#type` and the data-point variant carry the same information and the
        // builder does not cross-check them, so derive both from one match.
        let (r#type, data_points) = match family.kind {
            visit::Kind::Counter => (
                MetricType::Sum,
                DataPointsOwned::Sum(SumOwned {
                    data_points,
                    aggregation_temporality: AggregationTemporality::Cumulative,
                    is_monotonic: true,
                }),
            ),
            visit::Kind::Gauge => (
                MetricType::Gauge,
                DataPointsOwned::Gauge(GaugeOwned { data_points }),
            ),
        };
        self.rows.push(MetricRowOwned {
            name: ZStringOwned::String(format!("{NAME_PREFIX}{}", family.name)),
            r#type,
            description: Some(ZStringOwned::from_static(family.help)),
            unit: family.unit.ucum().map(ZStringOwned::from_static),
            source_ip: None,
            source_port: None,
            data_points,
            attrs: Vec::new(),
        });
        Ok(())
    }
}

/// Label set to data-point attributes. Values stay scalar strings: `Array` and
/// `KvList` would be dotted-expanded on append, which is exactly the shapeless
/// output the metrics signal exists to avoid.
fn attrs_of(labels: &visit::Labels) -> Vec<(ZStringOwned, ValueOwned)> {
    labels
        .iter()
        .map(|(key, value)| {
            let value = match value {
                Cow::Borrowed(v) => ZStringOwned::SmolStr(SmolStr::new_static(v)),
                Cow::Owned(v) => ZStringOwned::String(v.clone()),
            };
            (ZStringOwned::from_static(key), ValueOwned::String(value))
        })
        .collect()
}

impl visit::Visitor for OtapVisitor {
    type Error = Infallible;

    fn family(
        &mut self,
        family: visit::Family,
        samples: visit::Samples<'_>,
    ) -> Result<(), Infallible> {
        let kind = family.kind;
        let data_points = match samples {
            visit::Samples::U64Scalar(value) => {
                vec![self.data_point(kind, NumberValue::Int(value as i64), Vec::new())]
            }
            visit::Samples::F64Scalar(value) => {
                vec![self.data_point(kind, NumberValue::Float(value), Vec::new())]
            }
            visit::Samples::U64(samples) => samples
                .iter()
                .map(|(labels, value)| {
                    self.data_point(kind, NumberValue::Int(*value as i64), attrs_of(labels))
                })
                .collect(),
            visit::Samples::F64(samples) => samples
                .iter()
                .map(|(labels, value)| {
                    self.data_point(kind, NumberValue::Float(*value), attrs_of(labels))
                })
                .collect(),
        };
        self.push(family, data_points)
    }
}

/// Render every family of one snapshot set into metric rows.
fn collect_rows(
    metrics: &InternalMetrics,
    include_forwarder: bool,
    time: DateTime<Utc>,
    start_time: DateTime<Utc>,
) -> Vec<MetricRowOwned> {
    let mut visitor = OtapVisitor::new(time, start_time);
    match visit::walk(metrics, include_forwarder, &mut visitor) {
        Ok(()) => {}
        // `OtapVisitor::Error` is `Infallible`, so this arm is uninhabited.
        Err(never) => match never {},
    }
    visitor.rows
}

/// Every family of one snapshot set, as it stands now.
pub(super) fn render(metrics: &InternalMetrics) -> Vec<MetricRowOwned> {
    // The forwarder families only exist in cluster mode, the same condition the
    // Prometheus exporter is handed explicitly at startup. `NODE_ID` is set only
    // by `backend_run`, which makes it the local/backend discriminator here.
    collect_rows(
        metrics,
        config::NODE_ID.get().is_some(),
        Utc::now(),
        common::get_start_datetime().with_timezone(&Utc),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::internal_metrics::{
        Channel, ChannelKind, ChannelState, Forwarder, ForwarderNode, Sink,
    };
    use crate::config::Format;
    use crate::payload::{MetricsBuilder, Resource, Scope};
    use std::sync::Arc;
    use std::time::Instant;
    use uuid::Uuid;

    fn seeded() -> (Arc<InternalMetrics>, Uuid) {
        let metrics = Arc::new(InternalMetrics::default());
        let sink_id = Uuid::now_v7();
        metrics.sink.store(Arc::new((
            Instant::now(),
            vec![Sink {
                id: sink_id,
                name: "primary".to_string(),
                total_workers: 3,
                total_time_spent: std::time::Duration::from_millis(1500),
                total_records: 12,
                ..Default::default()
            }],
        )));

        let mut channel = Channel {
            total_messages: 9,
            ..Default::default()
        };
        channel.channels.insert(
            "memory".to_string(),
            ChannelState {
                kind: ChannelKind::Memory,
                capacity: 100,
                length: 7,
            },
        );
        channel.channels.insert(
            "disk".to_string(),
            ChannelState {
                kind: ChannelKind::Disk,
                capacity: 4096,
                length: 512,
            },
        );
        metrics
            .sink_channel
            .store(Arc::new((Instant::now(), channel)));

        metrics.forwarder.store(Arc::new((
            Instant::now(),
            Forwarder {
                nodes: vec![ForwarderNode {
                    id: Uuid::now_v7(),
                    total_connections: 4,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )));

        (metrics, sink_id)
    }

    fn row<'a>(rows: &'a [MetricRowOwned], name: &str) -> &'a MetricRowOwned {
        rows.iter()
            .find(|row| row.name.as_ref() == name)
            .unwrap_or_else(|| panic!("no metric named {name}"))
    }

    fn number_dps(row: &MetricRowOwned) -> &[NumberDataPointOwned] {
        match &row.data_points {
            DataPointsOwned::Sum(sum) => &sum.data_points,
            DataPointsOwned::Gauge(gauge) => &gauge.data_points,
            _ => panic!("{} is not a number metric", row.name.as_ref()),
        }
    }

    fn attr<'a>(dp: &'a NumberDataPointOwned, key: &str) -> &'a str {
        dp.attrs
            .iter()
            .find(|(k, _)| k.as_ref() == key)
            .map(|(_, v)| match v {
                ValueOwned::String(s) => s.as_ref(),
                other => panic!("attr {key} is not a string: {other:?}"),
            })
            .unwrap_or_else(|| panic!("no attr {key}"))
    }

    #[test]
    fn counters_are_cumulative_monotonic_sums_with_identity_labels() {
        let (metrics, sink_id) = seeded();
        let now = Utc::now();
        let start = now - chrono::Duration::seconds(60);
        let rows = collect_rows(&metrics, false, now, start);

        let records = row(&rows, "patrolli.sink.records");
        assert_eq!(records.r#type, MetricType::Sum);
        let DataPointsOwned::Sum(sum) = &records.data_points else {
            panic!("sink.records is not a sum");
        };
        assert_eq!(
            sum.aggregation_temporality,
            AggregationTemporality::Cumulative
        );
        assert!(sum.is_monotonic);
        assert_eq!(sum.data_points.len(), 1);

        let dp = &sum.data_points[0];
        assert!(matches!(dp.value, NumberValue::Int(12)));
        assert_eq!(dp.time, now);
        // Cumulative sums need the window they accumulated over.
        assert_eq!(dp.start_time, Some(start));
        // Identity is folded onto the series, not carried by a separate Info metric.
        assert_eq!(attr(dp, "sink_id"), sink_id.to_string());
        assert_eq!(attr(dp, "sink_name"), "primary");
    }

    #[test]
    fn gauges_are_gauges_without_a_start_time() {
        let (metrics, _) = seeded();
        let now = Utc::now();
        let rows = collect_rows(&metrics, false, now, now - chrono::Duration::seconds(60));

        let workers = row(&rows, "patrolli.sink.workers");
        assert_eq!(workers.r#type, MetricType::Gauge);
        assert!(matches!(workers.data_points, DataPointsOwned::Gauge(_)));
        let dp = &number_dps(workers)[0];
        assert!(matches!(dp.value, NumberValue::Int(3)));
        assert_eq!(dp.start_time, None);

        let queue = row(&rows, "patrolli.channel.queue_length");
        assert_eq!(queue.r#type, MetricType::Gauge);
        let dp = &number_dps(queue)[0];
        assert!(matches!(dp.value, NumberValue::Int(7)));
        assert_eq!(attr(dp, "channel_group"), "sink");
        assert_eq!(attr(dp, "channel"), "memory");
    }

    #[test]
    fn durations_are_ucum_seconds_and_bytes_are_ucum_by() {
        let (metrics, _) = seeded();
        let now = Utc::now();
        let rows = collect_rows(&metrics, false, now, now);

        let time_spent = row(&rows, "patrolli.sink.time_spent");
        assert_eq!(time_spent.unit.as_ref().map(|u| u.as_ref()), Some("s"));
        assert!(matches!(
            number_dps(time_spent)[0].value,
            NumberValue::Float(v) if (v - 1.5).abs() < f64::EPSILON
        ));
        assert!(time_spent.description.is_some());

        let disk = row(&rows, "patrolli.channel.disk_limit");
        assert_eq!(disk.unit.as_ref().map(|u| u.as_ref()), Some("By"));

        // A plain count carries no unit at all.
        assert!(row(&rows, "patrolli.sink.records").unit.is_none());
    }

    #[test]
    fn scalar_families_have_one_unlabeled_data_point() {
        let (metrics, _) = seeded();
        let now = Utc::now();
        let rows = collect_rows(&metrics, false, now, now);

        let items = row(&rows, "patrolli.monitor.items");
        let dps = number_dps(items);
        assert_eq!(dps.len(), 1);
        assert!(dps[0].attrs.is_empty());
    }

    #[test]
    fn lookup_table_health_and_zero_skip_counts_are_exported_with_units() {
        use crate::common::internal_metrics::{LookupTable, SkipCounts};
        let metrics = InternalMetrics::default();
        let now = Utc::now();
        for (cumulative, generation, load_error) in [(0, 0, false), (5, 5, true), (5, 0, false)] {
            metrics.lookup_table.store(Arc::new((
                Instant::now(),
                vec![LookupTable {
                    id: Uuid::now_v7(),
                    name: "hosts".into(),
                    kind: "immutable",
                    rows_skipped: SkipCounts {
                        bad_cell: cumulative,
                        ..Default::default()
                    },
                    generation_rows_skipped: SkipCounts {
                        bad_cell: generation,
                        ..Default::default()
                    },
                    generation: 2,
                    last_success_timestamp: 1_700_000_000.5,
                    load_error,
                    ..Default::default()
                }],
            )));
            let rows = collect_rows(&metrics, false, now, now);
            let success = row(&rows, "patrolli.lookup_table.last_success_timestamp");
            assert_eq!(success.r#type, MetricType::Gauge);
            assert_eq!(success.unit.as_ref().unwrap().as_ref(), "s");
            assert!(number_dps(success)[0].start_time.is_none());
            assert!(matches!(
                number_dps(success)[0].value,
                NumberValue::Float(1_700_000_000.5)
            ));
            let error = row(&rows, "patrolli.lookup_table.load_error");
            assert_eq!(error.r#type, MetricType::Gauge);
            assert!(
                matches!(number_dps(error)[0].value, NumberValue::Int(n) if n == i64::from(load_error))
            );
            for (name, value, kind) in [
                (
                    "patrolli.lookup_table.rows_skipped",
                    cumulative,
                    MetricType::Sum,
                ),
                (
                    "patrolli.lookup_table.generation.rows_skipped",
                    generation,
                    MetricType::Gauge,
                ),
            ] {
                let metric = row(&rows, name);
                assert_eq!(metric.r#type, kind);
                let points = number_dps(metric);
                assert_eq!(
                    points.len(),
                    crate::common::internal_metrics::visit::SKIP_REASONS.len()
                );
                let cast = points
                    .iter()
                    .find(|point| {
                        point.attrs.iter().any(|(key, value)| {
                            key.as_ref() == "reason"
                                && matches!(value, ValueOwned::String(s) if s.as_ref() == "cast")
                        })
                    })
                    .unwrap();
                assert!(matches!(cast.value, NumberValue::Int(n) if n == value as i64));
            }
        }
    }

    #[test]
    fn forwarder_families_are_gated_on_cluster_mode() {
        let (metrics, _) = seeded();
        let now = Utc::now();

        let local = collect_rows(&metrics, false, now, now);
        assert!(
            !local
                .iter()
                .any(|r| r.name.as_ref().starts_with("patrolli.forwarder"))
        );

        let cluster = collect_rows(&metrics, true, now, now);
        let peers = row(&cluster, "patrolli.forwarder.peer.connections");
        assert_eq!(peers.r#type, MetricType::Gauge);
        assert!(matches!(number_dps(peers)[0].value, NumberValue::Int(4)));
        // Every local family is still present in cluster mode.
        assert!(cluster.len() > local.len());
    }

    /// Mixing gauges and monotonic sums in one builder is the exact case
    /// `MetricsBuilder::reset_builders` exists for, so exercise the whole batch
    /// through `finish()` rather than trusting the rows alone.
    #[test]
    fn the_batch_round_trips_through_the_metrics_builder() {
        let (metrics, _) = seeded();
        let now = Utc::now();
        let rows = collect_rows(&metrics, true, now, now);
        assert!(
            rows.len() > 50,
            "expected the full catalogue, got {}",
            rows.len()
        );

        let mut builder = MetricsBuilder::new();
        builder.scope(Scope {
            name: smol_str::SmolStr::new_static("monitor"),
            listener_id: None,
            listener_type: None,
            listener_name: None,
            monitor_type: Some(smol_str::SmolStr::new_static("internal_metrics")),
            node_id: None,
        });
        builder.resource(Resource {
            host_id: Uuid::now_v7(),
            host_name: smol_str::SmolStr::new_static("localhost"),
            host_timezone: None,
            endpoint_id: None,
            endpoint_name: None,
            endpoint_type: None,
            item_id: Uuid::now_v7(),
            item_name: smol_str::SmolStr::new_static("internal_metrics"),
        });
        let expected_rows = rows.len();
        let expected_dps: usize = rows.iter().map(|r| number_dps(r).len()).sum();
        for row in rows {
            builder.push_row_owned(row);
        }
        assert_eq!(builder.num_rows(), expected_rows);
        assert_eq!(builder.num_items(), expected_dps);

        let payload = builder.finish();
        assert_eq!(payload.num_rows(), expected_rows);
        assert_eq!(payload.num_items(), expected_dps);

        let json = payload.serialize_as(Format::JsonOtlpBatch).unwrap();
        let text = json
            .iter()
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .collect::<String>();
        assert!(text.contains("\"sum\""));
        assert!(text.contains("\"gauge\""));
        assert!(text.contains("\"isMonotonic\""));
        assert!(text.contains("\"aggregationTemporality\""));
        assert!(text.contains("patrolli.sink.records"));
        assert!(text.contains("sink_name"));

        // A second batch on the same builder is what trips the upstream
        // adaptive-builder bug if the sub-builders were not replaced.
        let again = collect_rows(&metrics, true, now, now);
        for row in again {
            builder.push_row_owned(row);
        }
        assert_eq!(builder.num_rows(), expected_rows);
        assert!(!builder.finish().is_empty());
    }
}
