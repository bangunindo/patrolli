// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The disk channel encoding of payloads: each record batch as a plain Arrow
//! IPC stream, exactly as held in memory.
//!
//! The OTAP transport encoding (`encode_transport_optimized`: every batch
//! sorted, ids delta-encoded) is opt-in, with a disk channel's
//! `transport_optimized`. It compresses smaller, mostly on metrics, but costs
//! most of an encode, has to be undone on the way back, and reorders
//! attributes.
//!
//! ```text
//! signal tag (u8), batch count n (u8), n payload types (u8), zero padding to 8
//! n Arrow IPC streams (schema, dictionaries, batch, end of stream)
//! ```
//!
//! IPC messages are 8-padded, so every stream starts 8-aligned and the
//! decoder reads arrays in place.

use super::{ForwarderPayload, Payload, ProcessorPayload};
use crate::chan::Codec;
use anyhow::{Context, Result, bail, ensure};
use arrow::buffer::Buffer;
use arrow_ipc::reader::StreamDecoder;
use arrow_ipc::writer::{IpcWriteOptions, StreamWriter};
use otel_arrow_dfe_pdata::OtapArrowRecords;
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
use std::io::Write;
use uuid::Uuid;

/// What ends an IPC stream: the continuation marker and a zero length.
const EOS: [u8; 8] = [0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0];

fn padded(len: usize) -> usize {
    len.next_multiple_of(8)
}

impl Codec for Payload {
    fn encode(&self, out: &mut dyn Write, transport_optimized: bool) -> Result<()> {
        let optimized;
        let data = if transport_optimized {
            let mut data = self.data.clone();
            data.encode_transport_optimized()
                .context("transport-optimize payload")?;
            optimized = data;
            &optimized
        } else {
            &self.data
        };
        let tag = match data {
            OtapArrowRecords::Logs(_) => 0u8,
            OtapArrowRecords::Metrics(_) => 1,
            OtapArrowRecords::Traces(_) => 2,
        };
        let present: Vec<ArrowPayloadType> = data
            .allowed_payload_types()
            .iter()
            .copied()
            .filter(|pt| data.get(*pt).is_some())
            .collect();
        let mut header = vec![tag, present.len() as u8];
        for pt in &present {
            header.push(u8::try_from(*pt as i32).context("payload type")?);
        }
        header.resize(padded(header.len()), 0);
        out.write_all(&header)?;
        let options = IpcWriteOptions::default();
        for pt in present {
            let batch = data.get(pt).expect("present");
            let mut writer =
                StreamWriter::try_new_with_options(&mut *out, &batch.schema(), options.clone())?;
            writer.write(batch)?;
            writer.finish()?;
        }
        Ok(())
    }

    fn size_hint(&self) -> usize {
        // Logical size plus each stream's schema and padding.
        self.data
            .allowed_payload_types()
            .iter()
            .filter_map(|pt| self.data.get(*pt))
            .map(|batch| batch.get_array_memory_size() + 1024)
            .sum()
    }

    fn decode(mut buf: Buffer, transport_optimized: bool) -> Result<Self> {
        ensure!(buf.len() >= 2, "payload header cut short");
        let (tag, n) = (buf[0], buf[1] as usize);
        let header = padded(2 + n);
        ensure!(buf.len() >= header, "payload header cut short");
        let types = buf[2..2 + n].to_vec();
        let mut data = match tag {
            0 => OtapArrowRecords::Logs(Default::default()),
            1 => OtapArrowRecords::Metrics(Default::default()),
            2 => OtapArrowRecords::Traces(Default::default()),
            t => bail!("unknown signal tag {t}"),
        };
        buf = buf.slice(header);
        for pt in types {
            let pt = ArrowPayloadType::try_from(i32::from(pt))
                .with_context(|| format!("unknown payload type {pt}"))?;
            // The decoder returns as soon as it has the batch, leaving the
            // rest of `buf`. The end marker is skipped here: a decoder that
            // reads it refuses whatever follows.
            let mut decoder = StreamDecoder::new();
            let batch = loop {
                ensure!(!buf.is_empty(), "{pt:?} stream cut short");
                if let Some(batch) = decoder.decode(&mut buf)? {
                    break batch;
                }
            };
            ensure!(
                buf.len() >= EOS.len() && buf[..EOS.len()] == EOS,
                "{pt:?} stream has more than one batch"
            );
            buf = buf.slice(EOS.len());
            data.set(pt, batch)?;
        }
        ensure!(buf.is_empty(), "{} bytes past the last stream", buf.len());
        if transport_optimized {
            data.decode_transport_optimized_ids()
                .context("decode transport-optimized ids")?;
        }
        Ok(Self { data })
    }
}

/// `processor_id` as a presence byte padded to 8, then its 16 bytes when set.
impl Codec for ProcessorPayload {
    fn encode(&self, out: &mut dyn Write, transport_optimized: bool) -> Result<()> {
        match self.processor_id {
            Some(id) => {
                out.write_all(&[1, 0, 0, 0, 0, 0, 0, 0])?;
                out.write_all(id.as_bytes())?;
            }
            None => out.write_all(&[0; 8])?,
        }
        self.payload.encode(out, transport_optimized)
    }

    fn size_hint(&self) -> usize {
        24 + self.payload.size_hint()
    }

    fn decode(buf: Buffer, transport_optimized: bool) -> Result<Self> {
        ensure!(buf.len() >= 8, "processor payload cut short");
        let (processor_id, rest) = match buf[0] {
            0 => (None, buf.slice(8)),
            1 => {
                ensure!(buf.len() >= 24, "processor payload cut short");
                let id = Uuid::from_slice(&buf[8..24])?;
                (Some(id), buf.slice(24))
            }
            b => bail!("bad processor id marker {b}"),
        };
        Ok(Self {
            processor_id,
            payload: Payload::decode(rest, transport_optimized)?,
        })
    }
}

impl Codec for ForwarderPayload {
    fn encode(&self, out: &mut dyn Write, transport_optimized: bool) -> Result<()> {
        out.write_all(self.node_id.as_bytes())?;
        self.payload.encode(out, transport_optimized)
    }

    fn size_hint(&self) -> usize {
        16 + self.payload.size_hint()
    }

    fn decode(buf: Buffer, transport_optimized: bool) -> Result<Self> {
        ensure!(buf.len() >= 16, "forwarder payload cut short");
        Ok(Self {
            node_id: Uuid::from_slice(&buf[..16])?,
            payload: Payload::decode(buf.slice(16), transport_optimized)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chan::codec::{decode, encode};
    use crate::config::OtapCompression;
    use crate::payload::{LogRow, LogsBuilder, Scope, Value, ZString};
    use bytes::Bytes;
    use otel_arrow_dfe_pdata::encode::{
        encode_logs_otap_batch, encode_metrics_otap_batch, encode_spans_otap_batch,
    };
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::{
        AnyValue, ArrayValue as Array, InstrumentationScope, KeyValue, KeyValueList, any_value,
    };
    use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
        LogRecord, LogsData, ResourceLogs, ScopeLogs,
    };
    use otel_arrow_dfe_pdata::proto::opentelemetry::metrics::v1::{
        Exemplar, ExponentialHistogram, ExponentialHistogramDataPoint, Gauge, Histogram,
        HistogramDataPoint, Metric, MetricsData, NumberDataPoint, ResourceMetrics, ScopeMetrics,
        Sum, Summary, SummaryDataPoint, exponential_histogram_data_point::Buckets, metric,
        number_data_point,
    };
    use otel_arrow_dfe_pdata::proto::opentelemetry::resource::v1::Resource;
    use otel_arrow_dfe_pdata::proto::opentelemetry::trace::v1::{
        ResourceSpans, ScopeSpans, Span, TracesData, span,
    };
    use std::borrow::Cow;
    use std::net::IpAddr;

    const CODECS: [OtapCompression; 3] = [
        OtapCompression::None,
        OtapCompression::Lz4,
        OtapCompression::Zstd,
    ];

    fn kv(key: &str, value: any_value::Value) -> KeyValue {
        KeyValue {
            key: key.into(),
            value: Some(AnyValue { value: Some(value) }),
        }
    }

    /// One attribute of every kind.
    fn attrs(i: usize) -> Vec<KeyValue> {
        use any_value::Value::*;
        vec![
            kv("str", StringValue(format!("v{i}"))),
            kv("int", IntValue(i as i64)),
            kv("double", DoubleValue(i as f64 / 3.0)),
            kv("bool", BoolValue(i.is_multiple_of(2))),
            kv("bytes", BytesValue(vec![i as u8; 4])),
            kv(
                "array",
                ArrayValue(Array {
                    values: vec![AnyValue {
                        value: Some(IntValue(1)),
                    }],
                }),
            ),
            kv(
                "map",
                KvlistValue(KeyValueList {
                    values: vec![kv("k", StringValue("v".into()))],
                }),
            ),
        ]
    }

    fn resource() -> Option<Resource> {
        Some(Resource {
            attributes: attrs(0),
            ..Default::default()
        })
    }

    fn scope() -> Option<InstrumentationScope> {
        Some(InstrumentationScope {
            name: "scope".into(),
            version: "1".into(),
            attributes: attrs(1),
            ..Default::default()
        })
    }

    fn logs() -> Payload {
        let records = (0..5)
            .map(|i| LogRecord {
                time_unix_nano: 1_700_000_000_000_000_000 + i,
                severity_number: 9,
                severity_text: "INFO".into(),
                body: Some(AnyValue {
                    value: Some(any_value::Value::StringValue(format!("line {i}"))),
                }),
                attributes: attrs(i as usize),
                trace_id: vec![i as u8; 16],
                span_id: vec![i as u8; 8],
                ..Default::default()
            })
            .collect();
        let data = LogsData {
            resource_logs: vec![ResourceLogs {
                resource: resource(),
                scope_logs: vec![ScopeLogs {
                    scope: scope(),
                    log_records: records,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        Payload::new(encode_logs_otap_batch(&data).unwrap())
    }

    fn traces() -> Payload {
        let spans = (0..4)
            .map(|i| Span {
                trace_id: vec![i as u8; 16],
                span_id: vec![i as u8; 8],
                name: format!("span {i}"),
                start_time_unix_nano: 1,
                end_time_unix_nano: 2,
                attributes: attrs(i),
                events: vec![span::Event {
                    time_unix_nano: 1,
                    name: "event".into(),
                    attributes: attrs(i + 1),
                    ..Default::default()
                }],
                links: vec![span::Link {
                    trace_id: vec![9; 16],
                    span_id: vec![9; 8],
                    attributes: attrs(i + 2),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .collect();
        let data = TracesData {
            resource_spans: vec![ResourceSpans {
                resource: resource(),
                scope_spans: vec![ScopeSpans {
                    scope: scope(),
                    spans,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        Payload::new(encode_spans_otap_batch(&data).unwrap())
    }

    fn metrics() -> Payload {
        let exemplar = || Exemplar {
            filtered_attributes: attrs(3),
            time_unix_nano: 1,
            span_id: vec![1; 8],
            trace_id: vec![1; 16],
            value: Some(
                otel_arrow_dfe_pdata::proto::opentelemetry::metrics::v1::exemplar::Value::AsDouble(
                    1.5,
                ),
            ),
        };
        let number = |i: usize| NumberDataPoint {
            attributes: attrs(i),
            time_unix_nano: 1,
            value: Some(number_data_point::Value::AsInt(i as i64)),
            exemplars: vec![exemplar()],
            ..Default::default()
        };
        let metric = |name: &str, data| Metric {
            name: name.into(),
            data: Some(data),
            ..Default::default()
        };
        let metrics = vec![
            metric(
                "gauge",
                metric::Data::Gauge(Gauge {
                    data_points: vec![number(0), number(1)],
                }),
            ),
            metric(
                "sum",
                metric::Data::Sum(Sum {
                    data_points: vec![number(2)],
                    aggregation_temporality: 2,
                    is_monotonic: true,
                }),
            ),
            metric(
                "histogram",
                metric::Data::Histogram(Histogram {
                    data_points: vec![HistogramDataPoint {
                        attributes: attrs(4),
                        count: 3,
                        sum: Some(4.5),
                        bucket_counts: vec![1, 2],
                        explicit_bounds: vec![1.0],
                        exemplars: vec![exemplar()],
                        ..Default::default()
                    }],
                    aggregation_temporality: 1,
                }),
            ),
            metric(
                "exp",
                metric::Data::ExponentialHistogram(ExponentialHistogram {
                    data_points: vec![ExponentialHistogramDataPoint {
                        attributes: attrs(5),
                        count: 2,
                        scale: 1,
                        positive: Some(Buckets {
                            offset: 0,
                            bucket_counts: vec![1, 1],
                        }),
                        exemplars: vec![exemplar()],
                        ..Default::default()
                    }],
                    aggregation_temporality: 1,
                }),
            ),
            metric(
                "summary",
                metric::Data::Summary(Summary {
                    data_points: vec![SummaryDataPoint {
                        attributes: attrs(6),
                        count: 1,
                        sum: 2.0,
                        ..Default::default()
                    }],
                }),
            ),
        ];
        let data = MetricsData {
            resource_metrics: vec![ResourceMetrics {
                resource: resource(),
                scope_metrics: vec![ScopeMetrics {
                    scope: scope(),
                    metrics,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        Payload::new(encode_metrics_otap_batch(&data).unwrap())
    }

    /// What a listener builds: dictionary-encoded columns, nested values.
    fn built() -> Payload {
        let mut builder = LogsBuilder::new();
        builder.scope(Scope {
            name: "codec".into(),
            listener_id: Some(Uuid::now_v7()),
            listener_type: Some("syslog".into()),
            listener_name: Some("codec".into()),
            monitor_type: None,
            node_id: None,
        });
        for i in 0..50u16 {
            let attrs = [
                (
                    ZString::from_static("hostname"),
                    Value::String(ZString::from_ref(if i.is_multiple_of(2) {
                        "a"
                    } else {
                        "b"
                    })),
                ),
                (ZString::from_static("procid"), Value::Int64(i.into())),
                (
                    ZString::from_static("data"),
                    Value::Array(vec![Value::KvList(vec![(
                        ZString::from_ref("id"),
                        Value::String(ZString::from_ref("meta")),
                    )])]),
                ),
            ];
            builder.push_row(LogRow {
                body: Some(Cow::Owned(format!("message {i}"))),
                observed_time: None,
                severity: None,
                source_port: Some(i),
                source_ip: Some(IpAddr::from([127, 0, 0, 1])),
                attrs: &attrs,
            });
        }
        builder.finish()
    }

    fn fixtures() -> Vec<(&'static str, Payload)> {
        let sliced = built().split_items(20).unwrap().1.unwrap();
        vec![
            ("logs", logs()),
            ("traces", traces()),
            ("metrics", metrics()),
            ("built", built()),
            ("sliced", sliced),
        ]
    }

    #[test]
    fn every_signal_round_trips_exactly_with_every_codec() {
        for (name, payload) in fixtures() {
            assert!(payload.num_rows() > 0, "{name} fixture is empty");
            for codec in CODECS {
                let value = encode(&payload, codec, false).unwrap();
                let back: Payload = decode(value).unwrap();
                assert_eq!(back.data, payload.data, "{name} with {codec:?}");
            }
        }
    }

    /// `value` with every array sorted: the transport encoding reorders data
    /// points, events and attributes, never what is inside a value.
    fn sorted_deep(value: serde_json::Value) -> serde_json::Value {
        use serde_json::Value;
        match value {
            Value::Array(items) => {
                let mut items: Vec<Value> = items.into_iter().map(sorted_deep).collect();
                items.sort_by_cached_key(Value::to_string);
                Value::Array(items)
            }
            Value::Object(map) => {
                Value::Object(map.into_iter().map(|(k, v)| (k, sorted_deep(v))).collect())
            }
            other => other,
        }
    }

    /// The same records, whatever the order of their attributes: JSON with
    /// object keys sorted (serde_json's map is ordered), records sorted.
    fn normalized(payload: &Payload) -> Vec<String> {
        let mut records: Vec<String> = payload
            .serialize_as(crate::config::Format::JsonNestedDenormalized)
            .unwrap()
            .iter()
            .map(|b| sorted_deep(serde_json::from_slice(b).unwrap()).to_string())
            .collect();
        records.sort();
        records
    }

    #[test]
    fn every_signal_round_trips_transport_optimized_with_every_codec() {
        for (name, payload) in fixtures() {
            for codec in CODECS {
                let value = encode(&payload, codec, true).unwrap();
                let back: Payload = decode(value).unwrap();
                assert_eq!(
                    normalized(&back),
                    normalized(&payload),
                    "{name} with {codec:?}"
                );
            }
        }
    }

    /// The flag travels with the value: a reader needs no setting of its own.
    #[test]
    fn wrappers_round_trip_transport_optimized() {
        let payload = metrics();
        let msg = ProcessorPayload::new(Some(Uuid::now_v7()), payload.clone());
        let back: ProcessorPayload =
            decode(encode(&msg, OtapCompression::Zstd, true).unwrap()).unwrap();
        assert_eq!(back.processor_id, msg.processor_id);
        assert_eq!(normalized(&back.payload), normalized(&payload));
        let msg = ForwarderPayload::new(Uuid::now_v7(), payload.clone());
        let back: ForwarderPayload =
            decode(encode(&msg, OtapCompression::Lz4, true).unwrap()).unwrap();
        assert_eq!(back.node_id, msg.node_id);
        assert_eq!(normalized(&back.payload), normalized(&payload));
    }

    #[test]
    fn a_value_without_the_header_is_rejected() {
        let mut value = encode(&logs(), OtapCompression::None, false)
            .unwrap()
            .to_vec();
        value[0] ^= 0xff;
        assert!(decode::<Payload>(Bytes::from(value)).is_err());
        assert!(decode::<Payload>(Bytes::from_static(b"short")).is_err());
    }

    #[test]
    fn wrappers_round_trip() {
        let payload = built();
        for codec in CODECS {
            for processor_id in [None, Some(Uuid::now_v7())] {
                let msg = ProcessorPayload::new(processor_id, payload.clone());
                let back: ProcessorPayload = decode(encode(&msg, codec, false).unwrap()).unwrap();
                assert_eq!(back.processor_id, processor_id);
                assert_eq!(back.payload.data, payload.data);
            }
            let msg = ForwarderPayload::new(Uuid::now_v7(), payload.clone());
            let back: ForwarderPayload = decode(encode(&msg, codec, false).unwrap()).unwrap();
            assert_eq!(back.node_id, msg.node_id);
            assert_eq!(back.payload.data, payload.data);
        }
    }

    #[test]
    fn a_damaged_value_is_an_error_not_a_panic() {
        let payload = metrics();
        for (codec, transport) in CODECS.into_iter().flat_map(|c| [(c, false), (c, true)]) {
            let value = encode(&payload, codec, transport).unwrap();
            // lz4_flex takes EOF at a block boundary as the end of the frame, so
            // a cut inside the 4-byte end mark still decodes, with every content
            // byte present.
            let intact = match codec {
                OtapCompression::Lz4 => value.len() - 4,
                _ => value.len(),
            };
            for len in (0..intact).step_by(7) {
                assert!(
                    decode::<Payload>(value.slice(..len)).is_err(),
                    "{codec:?} transport={transport} cut at {len}"
                );
            }
            let mut flipped = value.to_vec();
            for i in (16..flipped.len()).step_by(13) {
                flipped[i] ^= 0x5a;
            }
            // Garbage may decode to garbage, but must not panic.
            let _ = decode::<Payload>(Bytes::from(flipped));
        }
    }
}
