// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! OTLP payload fixtures for integration tests.
//!
//! Built as OTLP protobuf and run through otap's own `encode_{logs,spans,metrics}_otap_batch`,
//! so tests exercise the real encoder — including the dictionary encoding and the all-null
//! column omission a converter has to cope with — rather than a hand-assembled batch.

#![allow(dead_code)]

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
use patrolli::payload::Payload;

pub const START_NANOS: u64 = 1_700_000_000_000_000_000;
pub const TIME_NANOS: u64 = 1_700_000_060_000_000_000;

pub fn kv(key: &str, value: &str) -> KeyValue {
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
pub fn logs_payload() -> Payload {
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

/// Two spans, the first carrying one event and one link.
pub fn traces_payload() -> Payload {
    let spans = vec![
        Span {
            trace_id: trace_id(0x11),
            span_id: span_id(0x22),
            parent_span_id: span_id(0x33),
            name: "GET /x".to_string(),
            kind: span::SpanKind::Server as i32,
            start_time_unix_nano: START_NANOS,
            end_time_unix_nano: START_NANOS + 500,
            trace_state: "a=b".to_string(),
            attributes: vec![kv("http.method", "GET")],
            status: Some(Status {
                code: status::StatusCode::Error as i32,
                message: "boom".to_string(),
            }),
            events: vec![span::Event {
                time_unix_nano: START_NANOS + 100,
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
            start_time_unix_nano: START_NANOS + 1000,
            end_time_unix_nano: START_NANOS + 1250,
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
        time_unix_nano: START_NANOS,
        span_id: span_id(0x99),
        trace_id: trace_id(0x88),
        value: Some(exemplar::Value::AsDouble(value)),
    }
}

/// One metric of every type, so a single payload exercises all five destination tables and
/// the gauge/sum split out of the shared `NumberDataPoints` batch.
pub fn metrics_payload() -> Payload {
    let gauge = Metric {
        name: "cpu.util".to_string(),
        description: "cpu utilization".to_string(),
        unit: "1".to_string(),
        data: Some(metric::Data::Gauge(Gauge {
            data_points: vec![NumberDataPoint {
                attributes: vec![kv("cpu", "0")],
                start_time_unix_nano: START_NANOS,
                time_unix_nano: TIME_NANOS,
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
                    start_time_unix_nano: START_NANOS,
                    time_unix_nano: TIME_NANOS,
                    // Int-valued, to exercise the int/double merge into Float64.
                    value: Some(number_data_point::Value::AsInt(4096)),
                    ..Default::default()
                },
                NumberDataPoint {
                    attributes: vec![kv("iface", "eth1")],
                    start_time_unix_nano: START_NANOS,
                    time_unix_nano: TIME_NANOS,
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
                start_time_unix_nano: START_NANOS,
                time_unix_nano: TIME_NANOS,
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
                start_time_unix_nano: START_NANOS,
                time_unix_nano: TIME_NANOS,
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
                start_time_unix_nano: START_NANOS,
                time_unix_nano: TIME_NANOS,
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
