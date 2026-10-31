// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub(crate) mod attr;
mod attr_builder;
mod batch;
mod builder;
mod codec;
pub(crate) mod json_ser;
mod logs;
mod metrics;
mod schema;

use anyhow::Result;
use arrow::array::ArrowPrimitiveType;
use bytes::Bytes;
use otel_arrow_dfe_pdata::{OtapArrowRecords, OtlpProtoBytes, TryFromWithOptions};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::borrow::Cow;
use std::fmt::Display;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::common::HasLength;
use crate::config::Format;
pub use attr::AttrLocation;
pub use attr_builder::AttrsBuilder;
pub use batch::PayloadBatch;
pub use logs::{LogRow, LogRowOwned, LogsBuilder};
pub use metrics::{
    BucketsOwned, DataPointsOwned, ExemplarOwned, ExponentialHistogramDataPointOwned,
    ExponentialHistogramOwned, GaugeOwned, HistogramDataPointOwned, HistogramOwned, MetricRowOwned,
    MetricsBuilder, NumberDataPointOwned, NumberValue, Quantile, SumOwned, SummaryDataPointOwned,
    SummaryOwned,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScopeAttrKey {
    ListenerId,
    ListenerType,
    ListenerName,
    MonitorType,
    NodeId,
}

impl ScopeAttrKey {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScopeAttrKey::ListenerId => "patrolli.listener.id",
            ScopeAttrKey::ListenerType => "patrolli.listener.type",
            ScopeAttrKey::ListenerName => "patrolli.listener.name",
            ScopeAttrKey::MonitorType => "patrolli.monitor.type",
            ScopeAttrKey::NodeId => "patrolli.node.id",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordAttrKey {
    SourceAddress,
    SourcePort,
}

impl RecordAttrKey {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecordAttrKey::SourceAddress => "source.address",
            RecordAttrKey::SourcePort => "source.port",
        }
    }

    pub fn as_bytes(&self) -> &'static [u8] {
        self.as_str().as_bytes()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceAttrKey {
    ServiceName,
    ServiceInstanceId,
    HostName,
    HostId,
    HostTimezone,
    EndpointName,
    EndpointId,
    EndpointType,
    ItemName,
    ItemId,
}

impl ResourceAttrKey {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResourceAttrKey::ServiceName => "service.name",
            ResourceAttrKey::ServiceInstanceId => "service.instance.id",
            ResourceAttrKey::HostName => "host.name",
            ResourceAttrKey::HostId => "host.id",
            ResourceAttrKey::HostTimezone => "patrolli.host.timezone",
            ResourceAttrKey::EndpointName => "patrolli.endpoint.name",
            ResourceAttrKey::EndpointId => "patrolli.endpoint.id",
            ResourceAttrKey::EndpointType => "patrolli.endpoint.type",
            ResourceAttrKey::ItemName => "patrolli.item.name",
            ResourceAttrKey::ItemId => "patrolli.item.id",
        }
    }
}

pub enum ZString<'a> {
    Cow(Cow<'a, str>),
    SmolStr(SmolStr),
}

impl<'a> ZString<'a> {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Cow(v) => v.as_bytes(),
            Self::SmolStr(v) => v.as_bytes(),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Cow(v) => v,
            Self::SmolStr(v) => v.as_str(),
        }
    }

    pub fn from_ref(v: &'a str) -> Self {
        Self::Cow(Cow::Borrowed(v))
    }

    pub fn from_static(v: &'static str) -> Self {
        Self::SmolStr(SmolStr::new_static(v))
    }

    pub fn from_owned(v: String) -> Self {
        Self::Cow(Cow::Owned(v))
    }

    pub fn from_smol(v: SmolStr) -> Self {
        Self::SmolStr(v)
    }

    pub fn into_owned(self) -> ZStringOwned {
        match self {
            Self::Cow(v) => ZStringOwned::String(v.into_owned()),
            Self::SmolStr(v) => ZStringOwned::SmolStr(v),
        }
    }
}

impl AsRef<str> for ZString<'_> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Display for ZString<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZString::Cow(v) => v.fmt(f),
            ZString::SmolStr(v) => v.fmt(f),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub enum ZStringOwned {
    String(String),
    #[schema(value_type = String)]
    SmolStr(SmolStr),
}

impl AsRef<str> for ZStringOwned {
    fn as_ref(&self) -> &str {
        match self {
            ZStringOwned::String(v) => v.as_ref(),
            ZStringOwned::SmolStr(v) => v.as_ref(),
        }
    }
}

impl ZStringOwned {
    pub fn as_ref_z(&self) -> ZString<'_> {
        match self {
            ZStringOwned::String(v) => ZString::Cow(Cow::Borrowed(v)),
            ZStringOwned::SmolStr(v) => ZString::Cow(Cow::Borrowed(v.as_ref())),
        }
    }

    pub fn from_static(v: &'static str) -> Self {
        Self::SmolStr(SmolStr::new_static(v))
    }
}

impl From<SmolStr> for ZStringOwned {
    fn from(v: SmolStr) -> Self {
        ZStringOwned::SmolStr(v)
    }
}

impl From<String> for ZStringOwned {
    fn from(v: String) -> Self {
        ZStringOwned::String(v)
    }
}

pub enum Value<'a> {
    Null,
    Bool(bool),
    Int64(i64),
    Double(f64),
    String(ZString<'a>),
    Bytes(Cow<'a, [u8]>),
    // Array & KvList are inserted as dotted-format keys, not serialized as CBOR
    Array(Vec<Value<'a>>),
    KvList(Vec<(ZString<'a>, Value<'a>)>),
}

impl Value<'_> {
    pub fn into_owned(self) -> ValueOwned {
        match self {
            Value::Null => ValueOwned::Null,
            Value::Bool(v) => ValueOwned::Bool(v),
            Value::Int64(v) => ValueOwned::Int64(v),
            Value::Double(v) => ValueOwned::Double(v),
            Value::String(v) => ValueOwned::String(v.into_owned()),
            Value::Bytes(v) => ValueOwned::Bytes(match v {
                Cow::Borrowed(v) => Bytes::copy_from_slice(v),
                Cow::Owned(v) => Bytes::from(v),
            }),
            Value::Array(v) => ValueOwned::Array(v.into_iter().map(|v| v.into_owned()).collect()),
            Value::KvList(v) => ValueOwned::KvList(
                v.into_iter()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect(),
            ),
        }
    }

    pub fn append_to<P: ArrowPrimitiveType>(
        &self,
        batch: &mut AttrsBuilder<P>,
        parent_id: P::Native,
        key: &str,
    ) {
        match self {
            Value::Array(_) | Value::KvList(_) => {
                let mut path = String::with_capacity(key.len() + 32);
                path.push_str(key);
                self.append_nested(batch, parent_id, &mut path);
            }
            _ => self.append_scalar(batch, parent_id, key),
        }
    }

    fn append_scalar<P: ArrowPrimitiveType>(
        &self,
        batch: &mut AttrsBuilder<P>,
        parent_id: P::Native,
        key: &str,
    ) {
        match self {
            Value::Null => batch.append_empty(parent_id, key),
            Value::Bool(v) => batch.append_bool(parent_id, key, *v),
            Value::Int64(v) => batch.append_int(parent_id, key, *v),
            Value::Double(v) => batch.append_double(parent_id, key, *v),
            Value::String(v) => batch.append_str(parent_id, key, v.as_str()),
            Value::Bytes(v) => batch.append_bytes(parent_id, key, v),
            Value::Array(_) | Value::KvList(_) => unreachable!("nested values go to append_nested"),
        }
    }

    /// Appends under the dotted key in `path`, which nested levels extend and then restore, so
    /// the keys of a deep list are built in one buffer instead of formatted once per level.
    fn append_nested<P: ArrowPrimitiveType>(
        &self,
        batch: &mut AttrsBuilder<P>,
        parent_id: P::Native,
        path: &mut String,
    ) {
        match self {
            Value::Array(a) => {
                let len = path.len();
                for (idx, scalar) in a.iter().enumerate() {
                    path.push('.');
                    push_index(path, idx);
                    scalar.append_nested(batch, parent_id, path);
                    path.truncate(len);
                }
            }
            Value::KvList(l) => {
                let len = path.len();
                for (l_key, scalar) in l.iter() {
                    path.push('.');
                    path.push_str(l_key.as_str());
                    scalar.append_nested(batch, parent_id, path);
                    path.truncate(len);
                }
            }
            scalar => scalar.append_scalar(batch, parent_id, path),
        }
    }
}

/// `idx` in decimal, without going through `fmt`.
fn push_index(path: &mut String, mut idx: usize) {
    let mut digits = [0u8; 20];
    let mut at = digits.len();
    loop {
        at -= 1;
        digits[at] = b'0' + (idx % 10) as u8;
        idx /= 10;
        if idx == 0 {
            break;
        }
    }
    path.push_str(std::str::from_utf8(&digits[at..]).expect("ascii digits"));
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub enum ValueOwned {
    Null,
    Bool(bool),
    Int64(i64),
    Double(f64),
    String(ZStringOwned),
    #[schema(value_type = Vec<u8>)]
    Bytes(Bytes),
    #[schema(no_recursion)]
    Array(Vec<ValueOwned>),
    #[schema(no_recursion, value_type = Vec<KvPair>)]
    KvList(Vec<(ZStringOwned, ValueOwned)>),
}

#[derive(ToSchema)]
#[allow(dead_code)]
pub struct KvPair((ZStringOwned, ValueOwned)); // for OpenAPI schema generation; not actually used as a type anywhere

impl ValueOwned {
    pub fn as_ref_z(&self) -> Value<'_> {
        match self {
            ValueOwned::Null => Value::Null,
            ValueOwned::Bool(v) => Value::Bool(*v),
            ValueOwned::Int64(v) => Value::Int64(*v),
            ValueOwned::Double(v) => Value::Double(*v),
            ValueOwned::String(v) => Value::String(v.as_ref_z()),
            ValueOwned::Bytes(v) => Value::Bytes(Cow::Borrowed(v)),
            ValueOwned::Array(v) => Value::Array(v.iter().map(|v| v.as_ref_z()).collect()),
            ValueOwned::KvList(v) => Value::KvList(
                v.iter()
                    .map(|(k, v)| (k.as_ref_z(), v.as_ref_z()))
                    .collect(),
            ),
        }
    }
}

impl From<()> for ValueOwned {
    fn from(_: ()) -> Self {
        ValueOwned::Null
    }
}

impl From<bool> for ValueOwned {
    fn from(v: bool) -> Self {
        ValueOwned::Bool(v)
    }
}

impl From<i64> for ValueOwned {
    fn from(v: i64) -> Self {
        ValueOwned::Int64(v)
    }
}

impl From<f64> for ValueOwned {
    fn from(v: f64) -> Self {
        ValueOwned::Double(v)
    }
}

impl From<Bytes> for ValueOwned {
    fn from(v: Bytes) -> Self {
        ValueOwned::Bytes(v)
    }
}

impl From<Vec<u8>> for ValueOwned {
    fn from(v: Vec<u8>) -> Self {
        ValueOwned::Bytes(Bytes::from(v))
    }
}

impl From<SmolStr> for ValueOwned {
    fn from(v: SmolStr) -> Self {
        ValueOwned::String(ZStringOwned::SmolStr(v))
    }
}

impl From<String> for ValueOwned {
    fn from(v: String) -> Self {
        ValueOwned::String(ZStringOwned::String(v))
    }
}

impl From<Vec<ValueOwned>> for ValueOwned {
    fn from(v: Vec<ValueOwned>) -> Self {
        ValueOwned::Array(v)
    }
}

impl From<Vec<(ZStringOwned, ValueOwned)>> for ValueOwned {
    fn from(v: Vec<(ZStringOwned, ValueOwned)>) -> Self {
        ValueOwned::KvList(v)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct Scope {
    pub name: SmolStr,
    pub listener_id: Option<Uuid>,
    pub listener_type: Option<SmolStr>,
    pub listener_name: Option<SmolStr>,
    pub monitor_type: Option<SmolStr>,
    pub node_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Resource {
    pub host_id: Uuid,
    pub host_name: SmolStr,
    pub host_timezone: Option<SmolStr>,
    pub endpoint_id: Option<Uuid>,
    pub endpoint_name: Option<SmolStr>,
    pub endpoint_type: Option<SmolStr>,
    pub item_id: Uuid,
    pub item_name: SmolStr,
}

#[derive(Clone)]
pub struct Payload {
    pub data: OtapArrowRecords,
}

impl std::fmt::Debug for Payload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Keep this cheap: the backing Arrow batches make a full dump huge and
        // useless, so summarize the shape instead — signal, the two counts that
        // disagree for metrics, retained memory, and the row count of every
        // record batch present. All of it is Arrow *metadata*: no column data is
        // read and nothing is decoded, so this stays safe to log on hot paths.
        f.debug_struct("Payload")
            .field("signal", &Signal(&self.data))
            .field("items", &self.data.num_items())
            .field("rows", &self.num_rows())
            .field("retained_bytes", &self.data.retained_memory_bytes())
            .field("batches", &BatchRows(&self.data))
            .finish_non_exhaustive()
    }
}

/// Renders `OtapArrowRecords`' signal type as a bare word.
struct Signal<'a>(&'a OtapArrowRecords);

impl std::fmt::Debug for Signal<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0 {
            OtapArrowRecords::Logs(_) => "logs",
            OtapArrowRecords::Metrics(_) => "metrics",
            OtapArrowRecords::Traces(_) => "traces",
        })
    }
}

/// Renders `{table: rows, …}` over the record batches the payload actually
/// carries — the attr/data-point tables OTAP omits simply don't appear.
struct BatchRows<'a>(&'a OtapArrowRecords);

impl std::fmt::Debug for BatchRows<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        /// `Debug` that prints a `&str` unquoted, so map keys read as identifiers.
        struct Bare(&'static str);
        impl std::fmt::Debug for Bare {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.0)
            }
        }

        let mut map = f.debug_map();
        for &pt in self.0.allowed_payload_types() {
            let Some(batch) = self.0.get(pt) else {
                continue;
            };
            match schema::table_name(pt) {
                Some(name) => map.entry(&Bare(name), &batch.num_rows()),
                // No table name of our own (e.g. multivariate metrics): fall
                // back to the OTAP payload-type name.
                None => map.entry(&pt, &batch.num_rows()),
            };
        }
        map.finish()
    }
}

impl Payload {
    pub fn new(data: OtapArrowRecords) -> Self {
        Self { data }
    }

    /// Number of *items* in the payload. For logs and traces an item is a root
    /// row, but for metrics it is a **data point**: the sum of rows across the
    /// four data-point tables, which is `>=` the metric row count (and `0` for a
    /// metric row carrying no data points).
    ///
    /// This is the unit `otel_arrow_dfe_pdata`'s batching API is denominated in, so it
    /// is the right count for anything that feeds `make_item_batches` (batch
    /// sizing, [`Payload::split_items`]) and for data-volume metrics. It is
    /// **not** the right count for indexing the root record batch — use
    /// [`Payload::num_rows`] for that.
    pub fn num_items(&self) -> usize {
        self.data.num_items()
    }

    /// Number of rows in the root record batch (Logs / UnivariateMetrics /
    /// Spans), or `0` when the payload has no root batch.
    ///
    /// This is the length any per-record mask must have before it can be handed
    /// to `filter_otap_batch`, and the number of records the denormalized JSON
    /// serializer emits. It differs from [`Payload::num_items`] for metrics.
    pub fn num_rows(&self) -> usize {
        self.data.root_record_batch().map_or(0, |b| b.num_rows())
    }

    /// Whether the payload carries no records at all.
    ///
    /// Deliberately `num_rows() == 0` and not `num_items() == 0`: callers use
    /// this to decide whether there is anything worth sending, processing or
    /// writing, and a metrics payload whose rows carry no data points still
    /// carries records. Gating those on the item count silently discards them.
    pub fn is_empty(&self) -> bool {
        self.num_rows() == 0
    }

    pub fn serialize_as(&self, format: Format) -> Result<Vec<Bytes>> {
        match format {
            // All four JSON shapes are served by the streaming `json_ser`
            // serializer (reads Arrow directly, writes JSON straight to the
            // buffer — no OTLP protobuf round-trip, no `serde_json::Value`
            // tree), selected by (Layout, Mode).
            Format::JsonOtlpBatch => {
                json_ser::to_json(self, json_ser::Layout::Batch, json_ser::Mode::Otlp)
            }
            Format::JsonOtlpDenormalized => {
                json_ser::to_json(self, json_ser::Layout::Denorm, json_ser::Mode::Otlp)
            }
            Format::JsonNestedBatch => {
                json_ser::to_json(self, json_ser::Layout::Batch, json_ser::Mode::Nested)
            }
            Format::JsonNestedDenormalized => {
                json_ser::to_json(self, json_ser::Layout::Denorm, json_ser::Mode::Nested)
            }
            Format::ProtoOtlp => {
                let otlp = OtlpProtoBytes::try_from_with_default(self.data.clone())?;
                Ok(match otlp {
                    OtlpProtoBytes::ExportLogsRequest(b) => vec![b],
                    OtlpProtoBytes::ExportMetricsRequest(b) => vec![b],
                    OtlpProtoBytes::ExportTracesRequest(b) => vec![b],
                })
            }
        }
    }
}

impl HasLength for Payload {
    fn len(&self) -> usize {
        Self::num_items(self)
    }

    // Not the `len() == 0` default: emptiness is a question about records, and
    // the channel uses it to decide whether a send is worth doing. See
    // [`Payload::is_empty`].
    fn is_empty(&self) -> bool {
        Self::is_empty(self)
    }
}

#[derive(Clone, Debug)]
pub struct ProcessorPayload {
    pub processor_id: Option<Uuid>,
    pub payload: Payload,
}

impl HasLength for ProcessorPayload {
    fn len(&self) -> usize {
        self.payload.num_items()
    }

    fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }
}

impl ProcessorPayload {
    pub fn new(processor_id: Option<Uuid>, payload: Payload) -> Self {
        Self {
            processor_id,
            payload,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ForwarderPayload {
    pub node_id: Uuid,
    pub payload: Payload,
}

impl HasLength for ForwarderPayload {
    fn len(&self) -> usize {
        self.payload.num_items()
    }

    fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }
}

impl ForwarderPayload {
    pub fn new(node_id: Uuid, payload: Payload) -> Self {
        Self { node_id, payload }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otel_arrow_dfe_pdata::encode::encode_logs_otap_batch;
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::any_value::Value;
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::{AnyValue, KeyValue};
    use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
        LogRecord, LogsData, ResourceLogs, ScopeLogs,
    };
    use serde_json::Value as JsonValue;

    fn sample_logs_payload() -> Payload {
        let req = LogsData {
            resource_logs: vec![ResourceLogs {
                resource: None,
                scope_logs: vec![ScopeLogs {
                    scope: None,
                    log_records: vec![LogRecord {
                        attributes: vec![KeyValue {
                            key: "service.name".to_string(),
                            value: Some(AnyValue {
                                value: Some(Value::StringValue("patrolli".to_string())),
                            }),
                        }],
                        ..Default::default()
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        let otap = encode_logs_otap_batch(&req).unwrap();
        Payload { data: otap }
    }

    #[test]
    fn serialize_json_logs() {
        let payload = sample_logs_payload();
        let bytes = payload
            .serialize_as(Format::JsonOtlpBatch)
            .expect("serialize");
        let json: JsonValue = serde_json::from_slice(&bytes[0]).expect("valid json");
        assert!(
            json.get("resourceLogs").is_some(),
            "missing resourceLogs: {json}"
        );
        // The attribute value should round-trip into the JSON.
        assert!(
            serde_json::to_string(&json).unwrap().contains("patrolli"),
            "attribute value missing: {json}"
        );
    }

    #[test]
    fn push_index_writes_decimal() {
        for idx in [0, 7, 10, 99, 1234, usize::MAX] {
            let mut path = String::from("k.");
            super::push_index(&mut path, idx);
            assert_eq!(path, format!("k.{idx}"));
        }
    }
}
