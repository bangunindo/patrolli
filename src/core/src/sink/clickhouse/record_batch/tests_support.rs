// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Fixtures and assertions shared by the per-signal builder tests.
//!
//! Payloads are built as OTLP protobuf and run through otap's own
//! `encode_{logs,spans,metrics}_otap_batch`, so the tests exercise the real encoder —
//! including the dictionary encoding and the all-null column omission that the builders have
//! to cope with — rather than a hand-assembled batch that happens to be convenient.

use crate::payload::Payload;
use arrow::array::{Array, AsArray, MapArray, RecordBatch};
use arrow::datatypes::{DataType, Float64Type, Int32Type, TimestampSecondType, UInt64Type};
use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::any_value::Value as AnyVal;
use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::{
    AnyValue, InstrumentationScope, KeyValue,
};
use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
    LogRecord, LogsData, ResourceLogs, ScopeLogs,
};
use otel_arrow_dfe_pdata::proto::opentelemetry::metrics::v1::{
    AggregationTemporality, Exemplar, ExponentialHistogram, ExponentialHistogramDataPoint, Gauge,
    Histogram, HistogramDataPoint, Metric, MetricsData, NumberDataPoint, ResourceMetrics,
    ScopeMetrics, Sum, Summary, SummaryDataPoint, exemplar, exponential_histogram_data_point,
    metric, number_data_point, summary_data_point,
};
use otel_arrow_dfe_pdata::proto::opentelemetry::resource::v1::Resource;
use otel_arrow_dfe_pdata::proto::opentelemetry::trace::v1::{
    ResourceSpans, ScopeSpans, Span, Status, TracesData, span, status,
};

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

pub(super) fn kv(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(AnyVal::StringValue(value.to_string())),
        }),
    }
}

fn resource(attrs: Vec<KeyValue>) -> Resource {
    Resource {
        attributes: attrs,
        ..Default::default()
    }
}

fn scope(attrs: Vec<KeyValue>) -> InstrumentationScope {
    InstrumentationScope {
        name: "patrolli.listener".to_string(),
        version: "1.2.3".to_string(),
        attributes: attrs,
        ..Default::default()
    }
}

fn trace_id(byte: u8) -> Vec<u8> {
    vec![byte; 16]
}

fn span_id(byte: u8) -> Vec<u8> {
    vec![byte; 8]
}

/// Two log records under one resource and one scope, each with its own record attribute.
pub(super) fn logs_payload() -> Payload {
    let records = vec![
        LogRecord {
            time_unix_nano: 1_700_000_000_000_000_001,
            severity_number: 9, // INFO
            severity_text: "INFO".to_string(),
            event_name: "ev".to_string(),
            trace_id: trace_id(0xaa),
            span_id: span_id(0xbb),
            flags: 1,
            body: Some(AnyValue {
                value: Some(AnyVal::StringValue("hello".to_string())),
            }),
            attributes: vec![kv("source.address", "10.0.0.1")],
            ..Default::default()
        },
        LogRecord {
            time_unix_nano: 1_700_000_000_000_000_002,
            severity_number: 17, // ERROR
            severity_text: "ERROR".to_string(),
            body: Some(AnyValue {
                value: Some(AnyVal::StringValue("world".to_string())),
            }),
            attributes: vec![kv("source.address", "10.0.0.2")],
            ..Default::default()
        },
    ];
    let req = LogsData {
        resource_logs: vec![ResourceLogs {
            resource: Some(resource(vec![
                kv("service.name", "patrolli"),
                kv("host.name", "node-a"),
            ])),
            scope_logs: vec![ScopeLogs {
                scope: Some(scope(vec![kv("patrolli.node.id", "n1")])),
                log_records: records,
                schema_url: "https://schemas/scope".to_string(),
            }],
            schema_url: "https://schemas/resource".to_string(),
        }],
    };
    Payload::new(otel_arrow_dfe_pdata::encode::encode_logs_otap_batch(&req).unwrap())
}

pub(super) fn empty_logs_payload() -> Payload {
    let req = LogsData {
        resource_logs: vec![],
    };
    Payload::new(otel_arrow_dfe_pdata::encode::encode_logs_otap_batch(&req).unwrap())
}

pub(super) fn empty_traces_payload() -> Payload {
    let req = TracesData {
        resource_spans: vec![],
    };
    Payload::new(otel_arrow_dfe_pdata::encode::encode_spans_otap_batch(&req).unwrap())
}

pub(super) fn empty_metrics_payload() -> Payload {
    let req = MetricsData {
        resource_metrics: vec![],
    };
    Payload::new(otel_arrow_dfe_pdata::encode::encode_metrics_otap_batch(&req).unwrap())
}

/// Two spans, the first carrying one event and one link.
pub(super) fn traces_payload() -> Payload {
    let spans = vec![
        Span {
            trace_id: trace_id(0x11),
            span_id: span_id(0x22),
            parent_span_id: span_id(0x33),
            name: "GET /x".to_string(),
            kind: span::SpanKind::Server as i32,
            start_time_unix_nano: 1_700_000_000_000_000_000,
            end_time_unix_nano: 1_700_000_000_000_000_500,
            trace_state: "a=b".to_string(),
            attributes: vec![kv("http.method", "GET")],
            status: Some(Status {
                code: status::StatusCode::Error as i32,
                message: "boom".to_string(),
            }),
            events: vec![span::Event {
                time_unix_nano: 1_700_000_000_000_000_100,
                name: "exception".to_string(),
                attributes: vec![kv("exception.type", "IOError")],
                ..Default::default()
            }],
            links: vec![span::Link {
                trace_id: trace_id(0x44),
                span_id: span_id(0x55),
                trace_state: "c=d".to_string(),
                attributes: vec![kv("link.kind", "follows")],
                ..Default::default()
            }],
            ..Default::default()
        },
        Span {
            trace_id: trace_id(0x66),
            span_id: span_id(0x77),
            name: "internal".to_string(),
            kind: span::SpanKind::Internal as i32,
            start_time_unix_nano: 1_700_000_000_000_001_000,
            end_time_unix_nano: 1_700_000_000_000_001_250,
            ..Default::default()
        },
    ];
    let req = TracesData {
        resource_spans: vec![ResourceSpans {
            resource: Some(resource(vec![kv("service.name", "patrolli")])),
            scope_spans: vec![ScopeSpans {
                scope: Some(scope(vec![])),
                spans,
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
    };
    Payload::new(otel_arrow_dfe_pdata::encode::encode_spans_otap_batch(&req).unwrap())
}

fn exemplar(value: f64) -> Exemplar {
    Exemplar {
        filtered_attributes: vec![kv("ex.key", "ex.val")],
        time_unix_nano: 1_700_000_000_000_000_000,
        span_id: span_id(0x99),
        trace_id: trace_id(0x88),
        value: Some(exemplar::Value::AsDouble(value)),
    }
}

/// One metric of every type, so a single payload exercises all five destination tables and
/// the gauge/sum split out of the shared `NumberDataPoints` batch.
pub(super) fn metrics_payload() -> Payload {
    let start = 1_700_000_000_000_000_000u64;
    let time = 1_700_000_060_000_000_000u64;

    let gauge = Metric {
        name: "cpu.util".to_string(),
        description: "cpu utilization".to_string(),
        unit: "1".to_string(),
        data: Some(metric::Data::Gauge(Gauge {
            data_points: vec![NumberDataPoint {
                attributes: vec![kv("cpu", "0")],
                start_time_unix_nano: start,
                time_unix_nano: time,
                value: Some(number_data_point::Value::AsDouble(0.75)),
                exemplars: vec![exemplar(0.75)],
                flags: 0,
            }],
        })),
        ..Default::default()
    };

    let sum = Metric {
        name: "net.bytes".to_string(),
        unit: "By".to_string(),
        data: Some(metric::Data::Sum(Sum {
            data_points: vec![
                NumberDataPoint {
                    attributes: vec![kv("iface", "eth0")],
                    start_time_unix_nano: start,
                    time_unix_nano: time,
                    // Int-valued, to exercise the int/double merge into Float64.
                    value: Some(number_data_point::Value::AsInt(4096)),
                    ..Default::default()
                },
                NumberDataPoint {
                    attributes: vec![kv("iface", "eth1")],
                    start_time_unix_nano: start,
                    time_unix_nano: time,
                    value: Some(number_data_point::Value::AsInt(8192)),
                    ..Default::default()
                },
            ],
            aggregation_temporality: AggregationTemporality::Cumulative as i32,
            is_monotonic: true,
        })),
        ..Default::default()
    };

    let histogram = Metric {
        name: "latency".to_string(),
        unit: "ms".to_string(),
        data: Some(metric::Data::Histogram(Histogram {
            data_points: vec![HistogramDataPoint {
                attributes: vec![kv("route", "/x")],
                start_time_unix_nano: start,
                time_unix_nano: time,
                count: 3,
                sum: Some(30.0),
                bucket_counts: vec![1, 2],
                explicit_bounds: vec![10.0],
                min: Some(5.0),
                max: Some(20.0),
                exemplars: vec![exemplar(12.0)],
                flags: 0,
            }],
            aggregation_temporality: AggregationTemporality::Delta as i32,
        })),
        ..Default::default()
    };

    let exp_histogram = Metric {
        name: "latency.exp".to_string(),
        unit: "ms".to_string(),
        data: Some(metric::Data::ExponentialHistogram(ExponentialHistogram {
            data_points: vec![ExponentialHistogramDataPoint {
                attributes: vec![kv("route", "/y")],
                start_time_unix_nano: start,
                time_unix_nano: time,
                count: 5,
                sum: Some(50.0),
                scale: 2,
                zero_count: 1,
                positive: Some(exponential_histogram_data_point::Buckets {
                    offset: 3,
                    bucket_counts: vec![2, 2],
                }),
                negative: Some(exponential_histogram_data_point::Buckets {
                    offset: -1,
                    bucket_counts: vec![1],
                }),
                min: Some(1.0),
                max: Some(40.0),
                exemplars: vec![exemplar(9.0)],
                ..Default::default()
            }],
            aggregation_temporality: AggregationTemporality::Cumulative as i32,
        })),
        ..Default::default()
    };

    let summary = Metric {
        name: "sizes".to_string(),
        unit: "By".to_string(),
        data: Some(metric::Data::Summary(Summary {
            data_points: vec![SummaryDataPoint {
                attributes: vec![kv("kind", "req")],
                start_time_unix_nano: start,
                time_unix_nano: time,
                count: 4,
                sum: 400.0,
                quantile_values: vec![
                    summary_data_point::ValueAtQuantile {
                        quantile: 0.5,
                        value: 90.0,
                    },
                    summary_data_point::ValueAtQuantile {
                        quantile: 0.99,
                        value: 190.0,
                    },
                ],
                flags: 0,
            }],
        })),
        ..Default::default()
    };

    let req = MetricsData {
        resource_metrics: vec![ResourceMetrics {
            resource: Some(resource(vec![kv("service.name", "patrolli")])),
            scope_metrics: vec![ScopeMetrics {
                scope: Some(scope(vec![kv("patrolli.node.id", "n1")])),
                metrics: vec![gauge, sum, histogram, exp_histogram, summary],
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
    };
    Payload::new(otel_arrow_dfe_pdata::encode::encode_metrics_otap_batch(&req).unwrap())
}

/// A metrics payload with only a gauge, so the sum half of the `NumberDataPoints` split has
/// nothing to emit.
pub(super) fn gauge_only_payload() -> Payload {
    let gauge = Metric {
        name: "cpu.util".to_string(),
        unit: "1".to_string(),
        data: Some(metric::Data::Gauge(Gauge {
            data_points: vec![NumberDataPoint {
                attributes: vec![kv("cpu", "0")],
                start_time_unix_nano: 1_700_000_000_000_000_000,
                time_unix_nano: 1_700_000_060_000_000_000,
                value: Some(number_data_point::Value::AsDouble(0.5)),
                ..Default::default()
            }],
        })),
        ..Default::default()
    };
    let req = MetricsData {
        resource_metrics: vec![ResourceMetrics {
            resource: Some(resource(vec![kv("service.name", "patrolli")])),
            scope_metrics: vec![ScopeMetrics {
                scope: Some(scope(vec![])),
                metrics: vec![gauge],
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
    };
    Payload::new(otel_arrow_dfe_pdata::encode::encode_metrics_otap_batch(&req).unwrap())
}

/// Round-trip a payload through the OTAP transport encoding, as the cluster forwarder sends
/// it, which applies (and must then undo) the transport-optimized id encoding.
pub(super) fn round_trip(payload: &Payload) -> Payload {
    use otel_arrow_dfe_pdata::{Consumer, OtapArrowRecords, Producer, otap::from_record_messages};
    let mut data = payload.data.clone();
    let mut bar = Producer::new().produce_bar(&mut data).expect("produce");
    let msgs = Consumer::default().consume_bar(&mut bar).expect("consume");
    let mut data = match &payload.data {
        OtapArrowRecords::Logs(_) => OtapArrowRecords::Logs(from_record_messages(msgs).unwrap()),
        OtapArrowRecords::Metrics(_) => {
            OtapArrowRecords::Metrics(from_record_messages(msgs).unwrap())
        }
        OtapArrowRecords::Traces(_) => {
            OtapArrowRecords::Traces(from_record_messages(msgs).unwrap())
        }
    };
    data.decode_transport_optimized_ids().expect("decode ids");
    Payload::new(data)
}

// ---------------------------------------------------------------------------
// Assertions / readers
// ---------------------------------------------------------------------------

/// No ClickHouse column in the exporter schema is `Nullable`, so a null anywhere means the
/// builder leaked one instead of materializing a default.
pub(super) fn assert_no_nulls(batch: &RecordBatch) {
    for (i, column) in batch.columns().iter().enumerate() {
        assert_eq!(
            column.null_count(),
            0,
            "column {} has {} nulls",
            batch.schema().field(i).name(),
            column.null_count()
        );
    }
}

/// Assert two batches carry the same data, treating `Map` entries as unordered.
///
/// Byte equality is the wrong bar for a round-trip comparison: the transport-optimized
/// encoding *sorts* attribute rows — that is how quasi-delta encoding gets its runs — and
/// decoding restores the `parent_id`s but not the original row order. So a payload that has
/// been through a disk channel emits the same attributes per record in a different order.
/// Entry order inside a ClickHouse `Map` carries no meaning, so equivalence is what matters.
pub(super) fn assert_equivalent(left: &RecordBatch, right: &RecordBatch) {
    assert_eq!(left.schema(), right.schema(), "schemas differ");
    assert_eq!(left.num_rows(), right.num_rows(), "row counts differ");

    for (i, (l, r)) in left.columns().iter().zip(right.columns()).enumerate() {
        let name = left.schema().field(i).name().clone();
        match l.data_type() {
            DataType::Map(_, _) => {
                let (lm, rm) = (
                    l.as_any().downcast_ref::<MapArray>().unwrap(),
                    r.as_any().downcast_ref::<MapArray>().unwrap(),
                );
                for row in 0..lm.len() {
                    assert_eq!(
                        sorted(map_entries(lm, row)),
                        sorted(map_entries(rm, row)),
                        "{name} differs at row {row}"
                    );
                }
            }
            DataType::List(field) if matches!(field.data_type(), DataType::Map(_, _)) => {
                let (ll, rl) = (l.as_list::<i32>(), r.as_list::<i32>());
                for row in 0..ll.len() {
                    let (lv, rv) = (ll.value(row), rl.value(row));
                    let (lm, rm) = (
                        lv.as_any().downcast_ref::<MapArray>().unwrap(),
                        rv.as_any().downcast_ref::<MapArray>().unwrap(),
                    );
                    assert_eq!(lm.len(), rm.len(), "{name} length differs at row {row}");
                    for j in 0..lm.len() {
                        assert_eq!(
                            sorted(map_entries(lm, j)),
                            sorted(map_entries(rm, j)),
                            "{name} differs at row {row} element {j}"
                        );
                    }
                }
            }
            _ => assert_eq!(l, r, "column {name} differs"),
        }
    }
}

fn sorted(mut v: Vec<(String, String)>) -> Vec<(String, String)> {
    v.sort();
    v
}

pub(super) fn str_col(batch: &RecordBatch, name: &str) -> Vec<String> {
    let c = batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("no column {name}"));
    let a = c.as_string::<i32>();
    (0..a.len()).map(|i| a.value(i).to_string()).collect()
}

pub(super) fn f64_col(batch: &RecordBatch, name: &str) -> Vec<f64> {
    let c = batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("no column {name}"));
    c.as_primitive::<Float64Type>().values().to_vec()
}

pub(super) fn u64_col(batch: &RecordBatch, name: &str) -> Vec<u64> {
    let c = batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("no column {name}"));
    c.as_primitive::<UInt64Type>().values().to_vec()
}

pub(super) fn i32_col(batch: &RecordBatch, name: &str) -> Vec<i32> {
    let c = batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("no column {name}"));
    c.as_primitive::<Int32Type>().values().to_vec()
}

pub(super) fn bool_col(batch: &RecordBatch, name: &str) -> Vec<bool> {
    let c = batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("no column {name}"));
    let a = c.as_boolean();
    (0..a.len()).map(|i| a.value(i)).collect()
}

/// Epoch seconds of a `DateTime` column.
pub(super) fn ts_secs_col(batch: &RecordBatch, name: &str) -> Vec<i64> {
    let c = batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("no column {name}"));
    c.as_primitive::<TimestampSecondType>().values().to_vec()
}

/// The key/value pairs of a `Map` column at one row, in stored order.
pub(super) fn map_col(batch: &RecordBatch, name: &str, row: usize) -> Vec<(String, String)> {
    let c = batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("no column {name}"));
    let m = c.as_any().downcast_ref::<MapArray>().expect("map column");
    map_entries(m, row)
}

pub(super) fn map_entries(m: &MapArray, row: usize) -> Vec<(String, String)> {
    let keys = m.keys().as_string::<i32>();
    let vals = m.values().as_string::<i32>();
    let o = m.offsets();
    (o[row] as usize..o[row + 1] as usize)
        .map(|i| (keys.value(i).to_string(), vals.value(i).to_string()))
        .collect()
}

/// The `f64` elements of a `List<Float64>` column at one row.
pub(super) fn list_f64(batch: &RecordBatch, name: &str, row: usize) -> Vec<f64> {
    let c = batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("no column {name}"));
    let l = c.as_list::<i32>();
    l.value(row).as_primitive::<Float64Type>().values().to_vec()
}

/// The `u64` elements of a `List<UInt64>` column at one row.
pub(super) fn list_u64(batch: &RecordBatch, name: &str, row: usize) -> Vec<u64> {
    let c = batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("no column {name}"));
    let l = c.as_list::<i32>();
    l.value(row).as_primitive::<UInt64Type>().values().to_vec()
}

/// The string elements of a `List<Utf8>` column at one row.
pub(super) fn list_str(batch: &RecordBatch, name: &str, row: usize) -> Vec<String> {
    let c = batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("no column {name}"));
    let l = c.as_list::<i32>();
    let v = l.value(row);
    let a = v.as_string::<i32>();
    (0..a.len()).map(|i| a.value(i).to_string()).collect()
}

/// The maps of a `List<Map>` column at one row (e.g. `Events.Attributes`).
pub(super) fn list_maps(batch: &RecordBatch, name: &str, row: usize) -> Vec<Vec<(String, String)>> {
    let c = batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("no column {name}"));
    let l = c.as_list::<i32>();
    let v = l.value(row);
    let m = v.as_any().downcast_ref::<MapArray>().expect("list of maps");
    (0..m.len()).map(|i| map_entries(m, i)).collect()
}

/// Row index of the first row whose `name` column equals `value`.
pub(super) fn row_of(batch: &RecordBatch, name: &str, value: &str) -> usize {
    str_col(batch, name)
        .iter()
        .position(|v| v == value)
        .unwrap_or_else(|| panic!("no row where {name} = {value}"))
}
