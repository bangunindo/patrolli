// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Common OTLP fixtures exercise resource broadcasts across all three signals.
use crate::payload::Payload;
use arrow::array::{Array, ArrayRef, AsArray, RecordBatch, StructArray, UInt16Array};
use arrow::buffer::NullBuffer;
use otel_arrow_dfe_pdata::encode::{
    encode_logs_otap_batch, encode_metrics_otap_batch, encode_spans_otap_batch,
};
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
use otel_arrow_dfe_pdata::proto::opentelemetry::{
    common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value::Value},
    logs::v1::{LogRecord, LogsData, ResourceLogs, ScopeLogs},
    metrics::v1::{
        Gauge, Metric, MetricsData, NumberDataPoint, ResourceMetrics, ScopeMetrics, metric::Data,
    },
    resource::v1::Resource,
    trace::v1::{
        ResourceSpans, ScopeSpans, Span, TracesData,
        span::{Event, Link},
    },
};
use otel_arrow_dfe_pdata::schema::consts;
use std::sync::Arc;

pub(super) fn signals() -> Vec<Payload> {
    let resource = || {
        Some(Resource {
            attributes: vec![KeyValue {
                key: "host".into(),
                value: Some(AnyValue {
                    value: Some(Value::StringValue("a".into())),
                }),
            }],
            ..Default::default()
        })
    };
    let logs = LogsData {
        resource_logs: vec![ResourceLogs {
            resource: resource(),
            scope_logs: vec![ScopeLogs {
                log_records: vec![
                    LogRecord {
                        time_unix_nano: 1,
                        ..Default::default()
                    };
                    2
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    let metrics = MetricsData {
        resource_metrics: vec![ResourceMetrics {
            resource: resource(),
            scope_metrics: vec![ScopeMetrics {
                metrics: vec![
                    Metric {
                        name: "requests".into(),
                        data: Some(Data::Gauge(Gauge {
                            data_points: vec![NumberDataPoint {
                                time_unix_nano: 1,
                                ..Default::default()
                            }],
                        })),
                        ..Default::default()
                    };
                    2
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    let traces = TracesData {
        resource_spans: vec![ResourceSpans {
            resource: resource(),
            scope_spans: vec![ScopeSpans {
                spans: vec![
                    Span {
                        trace_id: vec![1; 16],
                        span_id: vec![2; 8],
                        name: "span".into(),
                        start_time_unix_nano: 1,
                        end_time_unix_nano: 2,
                        events: vec![Event {
                            name: "event".into(),
                            time_unix_nano: 1,
                            ..Default::default()
                        }],
                        links: vec![Link {
                            trace_id: vec![3; 16],
                            span_id: vec![4; 8],
                            ..Default::default()
                        }],
                        ..Default::default()
                    };
                    2
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    vec![
        Payload::new(encode_logs_otap_batch(&logs).unwrap()),
        Payload::new(encode_metrics_otap_batch(&metrics).unwrap()),
        Payload::new(encode_spans_otap_batch(&traces).unwrap()),
    ]
}

/// Resource 0 owns host=a; the other record has no resource attribute owner.
/// Incoming OTAP may express that with a null ID or a null enclosing struct.
pub(super) fn logs_with_unowned_resource(null_struct: bool) -> Payload {
    let kv = |key: &str, value: &str| KeyValue {
        key: key.into(),
        value: Some(AnyValue {
            value: Some(Value::StringValue(value.into())),
        }),
    };
    let request = LogsData {
        resource_logs: ["correct", "wrong"]
            .into_iter()
            .enumerate()
            .map(|(index, site)| ResourceLogs {
                resource: Some(Resource {
                    attributes: if index == 0 {
                        vec![kv("host", "a")]
                    } else {
                        vec![]
                    },
                    ..Default::default()
                }),
                scope_logs: vec![ScopeLogs {
                    log_records: vec![LogRecord {
                        time_unix_nano: 1,
                        attributes: vec![kv("observed_site", site)],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            })
            .collect(),
    };
    let mut data = encode_logs_otap_batch(&request).unwrap();
    let root = data.get(ArrowPayloadType::Logs).unwrap();
    let index = root.schema().index_of(consts::RESOURCE).unwrap();
    let resource = root.column(index).as_struct();
    let mut children = resource.columns().to_vec();
    let id_index = resource
        .fields()
        .iter()
        .position(|f| f.name() == consts::ID)
        .unwrap();
    children[id_index] = Arc::new(UInt16Array::from(vec![Some(0), null_struct.then_some(0)]));
    let mut columns: Vec<ArrayRef> = root.columns().to_vec();
    columns[index] = Arc::new(StructArray::new(
        resource.fields().clone(),
        children,
        null_struct.then(|| NullBuffer::from(vec![true, false])),
    ));
    let root = RecordBatch::try_new(root.schema(), columns).unwrap();
    data.set(ArrowPayloadType::Logs, root).unwrap();
    Payload::new(data)
}

fn kv(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: Some(AnyValue {
            value: Some(Value::StringValue(value.into())),
        }),
    }
}

/// Logs, metrics and traces of two records each, `keep=yes` on the first and
/// `keep=no` on the second, under one resource (`host=a`) and one scope
/// (`lib=x`). Every record owns children carrying attributes of their own:
/// each metric two data points (`dp`), each span an event (`ev`) and a link
/// (`ln`), so a filter's cascade shows in every child table.
pub(super) fn tagged_signals() -> Vec<Payload> {
    let keep = |i: usize| vec![kv("keep", if i == 0 { "yes" } else { "no" })];
    let resource = || {
        Some(Resource {
            attributes: vec![kv("host", "a")],
            ..Default::default()
        })
    };
    let scope = || {
        Some(InstrumentationScope {
            name: "scope".into(),
            attributes: vec![kv("lib", "x")],
            ..Default::default()
        })
    };
    let logs = LogsData {
        resource_logs: vec![ResourceLogs {
            resource: resource(),
            scope_logs: vec![ScopeLogs {
                scope: scope(),
                log_records: (0..2)
                    .map(|i| LogRecord {
                        time_unix_nano: 1,
                        attributes: keep(i),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    let metrics = MetricsData {
        resource_metrics: vec![ResourceMetrics {
            resource: resource(),
            scope_metrics: vec![ScopeMetrics {
                scope: scope(),
                metrics: (0..2)
                    .map(|i| Metric {
                        name: format!("requests{i}"),
                        metadata: keep(i),
                        data: Some(Data::Gauge(Gauge {
                            data_points: (0..2)
                                .map(|_| NumberDataPoint {
                                    time_unix_nano: 1,
                                    attributes: vec![kv("dp", "v")],
                                    ..Default::default()
                                })
                                .collect(),
                        })),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    let traces = TracesData {
        resource_spans: vec![ResourceSpans {
            resource: resource(),
            scope_spans: vec![ScopeSpans {
                scope: scope(),
                spans: (0..2)
                    .map(|i| Span {
                        trace_id: vec![1; 16],
                        span_id: vec![2 + i as u8; 8],
                        name: "span".into(),
                        start_time_unix_nano: 1,
                        end_time_unix_nano: 2,
                        attributes: keep(i),
                        events: vec![Event {
                            name: "event".into(),
                            time_unix_nano: 1,
                            attributes: vec![kv("ev", "v")],
                            ..Default::default()
                        }],
                        links: vec![Link {
                            trace_id: vec![3; 16],
                            span_id: vec![4; 8],
                            attributes: vec![kv("ln", "v")],
                            ..Default::default()
                        }],
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    vec![
        Payload::new(encode_logs_otap_batch(&logs).unwrap()),
        Payload::new(encode_metrics_otap_batch(&metrics).unwrap()),
        Payload::new(encode_spans_otap_batch(&traces).unwrap()),
    ]
}

/// `n` log records with no attributes.
pub(super) fn logs(n: usize) -> Payload {
    let logs = LogsData {
        resource_logs: vec![ResourceLogs {
            scope_logs: vec![ScopeLogs {
                log_records: vec![
                    LogRecord {
                        time_unix_nano: 1,
                        ..Default::default()
                    };
                    n
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    Payload::new(encode_logs_otap_batch(&logs).unwrap())
}

/// The rows in the attribute table `table`, absent table included.
pub(super) fn rows(pl: &Payload, table: ArrowPayloadType) -> usize {
    pl.data.get(table).map_or(0, |batch| batch.num_rows())
}

/// The string values of `key` in the attribute table `table`.
pub(super) fn attr_strs(pl: &Payload, table: ArrowPayloadType, key: &str) -> Vec<String> {
    let Some(attrs) = pl.data.get(table) else {
        return vec![];
    };
    let keys = attrs.column_by_name(consts::ATTRIBUTE_KEY).unwrap();
    let keys = arrow::compute::cast(keys, &arrow::datatypes::DataType::Utf8).unwrap();
    let values = attrs.column_by_name(consts::ATTRIBUTE_STR).unwrap();
    let values = arrow::compute::cast(values, &arrow::datatypes::DataType::Utf8).unwrap();
    let (keys, values) = (keys.as_string::<i32>(), values.as_string::<i32>());
    (0..attrs.num_rows())
        .filter(|&i| keys.value(i) == key && values.is_valid(i))
        .map(|i| values.value(i).to_owned())
        .collect()
}
