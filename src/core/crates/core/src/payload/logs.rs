// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use arrayvec::ArrayString;
use arrow::array::{
    Array, ArrayRef, DictionaryArray, Int32Array, RecordBatch, StringArray, StringBuilder,
    StructArray, TimestampNanosecondArray, UInt8Array, UInt16Array,
};
use arrow::buffer::{BooleanBuffer, NullBuffer, ScalarBuffer};
use arrow::datatypes::{Field, Fields, Schema};
use arrow::error::ArrowError;
use chrono::{DateTime, Utc};
use opentelemetry_proto::tonic::logs::v1::SeverityNumber;
use otel_arrow_dfe_pdata::OtapArrowRecords;
use otel_arrow_dfe_pdata::otap::Logs;
use otel_arrow_dfe_pdata::otlp::attributes::AttributeValueType;
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
use otel_arrow_dfe_pdata::schema::{FieldExt, UTC_TIME_ZONE, consts};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fmt::Write;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use utoipa::ToSchema;

#[derive(Default)]
pub struct LogRow<'a> {
    pub observed_time: Option<DateTime<Utc>>,
    pub severity: Option<SeverityNumber>,
    pub body: Option<Cow<'a, str>>,
    pub source_ip: Option<IpAddr>,
    pub source_port: Option<u16>,
    pub attrs: &'a [(super::ZString<'a>, super::Value<'a>)],
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum SeverityNumberSchema {
    Unspecified = 0,
    Trace = 1,
    Trace2 = 2,
    Trace3 = 3,
    Trace4 = 4,
    Debug = 5,
    Debug2 = 6,
    Debug3 = 7,
    Debug4 = 8,
    Info = 9,
    Info2 = 10,
    Info3 = 11,
    Info4 = 12,
    Warn = 13,
    Warn2 = 14,
    Warn3 = 15,
    Warn4 = 16,
    Error = 17,
    Error2 = 18,
    Error3 = 19,
    Error4 = 20,
    Fatal = 21,
    Fatal2 = 22,
    Fatal3 = 23,
    Fatal4 = 24,
}

impl From<SeverityNumber> for SeverityNumberSchema {
    fn from(severity: SeverityNumber) -> Self {
        match severity {
            SeverityNumber::Unspecified => SeverityNumberSchema::Unspecified,
            SeverityNumber::Trace => SeverityNumberSchema::Trace,
            SeverityNumber::Trace2 => SeverityNumberSchema::Trace2,
            SeverityNumber::Trace3 => SeverityNumberSchema::Trace3,
            SeverityNumber::Trace4 => SeverityNumberSchema::Trace4,
            SeverityNumber::Debug => SeverityNumberSchema::Debug,
            SeverityNumber::Debug2 => SeverityNumberSchema::Debug2,
            SeverityNumber::Debug3 => SeverityNumberSchema::Debug3,
            SeverityNumber::Debug4 => SeverityNumberSchema::Debug4,
            SeverityNumber::Info => SeverityNumberSchema::Info,
            SeverityNumber::Info2 => SeverityNumberSchema::Info2,
            SeverityNumber::Info3 => SeverityNumberSchema::Info3,
            SeverityNumber::Info4 => SeverityNumberSchema::Info4,
            SeverityNumber::Warn => SeverityNumberSchema::Warn,
            SeverityNumber::Warn2 => SeverityNumberSchema::Warn2,
            SeverityNumber::Warn3 => SeverityNumberSchema::Warn3,
            SeverityNumber::Warn4 => SeverityNumberSchema::Warn4,
            SeverityNumber::Error => SeverityNumberSchema::Error,
            SeverityNumber::Error2 => SeverityNumberSchema::Error2,
            SeverityNumber::Error3 => SeverityNumberSchema::Error3,
            SeverityNumber::Error4 => SeverityNumberSchema::Error4,
            SeverityNumber::Fatal => SeverityNumberSchema::Fatal,
            SeverityNumber::Fatal2 => SeverityNumberSchema::Fatal2,
            SeverityNumber::Fatal3 => SeverityNumberSchema::Fatal3,
            SeverityNumber::Fatal4 => SeverityNumberSchema::Fatal4,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct LogRowOwned {
    pub observed_time: Option<DateTime<Utc>>,
    #[schema(value_type = Option<SeverityNumberSchema>)]
    pub severity: Option<SeverityNumber>,
    pub body: Option<super::ZStringOwned>,
    #[schema(value_type = Option<String>)]
    pub source_ip: Option<IpAddr>,
    pub source_port: Option<u16>,
    pub attrs: Vec<(super::ZStringOwned, super::ValueOwned)>,
}

pub struct LogsBuilder {
    base: super::builder::BaseBuilder,
    rows: LogColumns,
    /// The last source address and its text: a TCP stream, or a busy UDP
    /// exporter, sends one row after another from the same peer.
    source_ip: Option<(IpAddr, ArrayString<45>)>, // 45 = max IPv6 string len
}

impl Default for LogsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl LogsBuilder {
    pub fn new() -> Self {
        Self {
            rows: LogColumns::default(),
            base: Default::default(),
            source_ip: None,
        }
    }

    pub fn scope(&mut self, scope: super::Scope) {
        self.base.scope(scope);
    }

    pub fn resource(&mut self, resource: super::Resource) {
        self.base.resource(resource);
    }

    /// Log records appended so far. For logs a row *is* an item, so this is also
    /// [`Self::num_items`].
    pub fn num_rows(&self) -> usize {
        self.base.num_rows()
    }

    /// Items appended so far — identical to [`Self::num_rows`] for logs. Exists so
    /// callers batching on `batch_size` can use one name across builders (see
    /// [`super::MetricsBuilder::num_items`], where the two differ).
    pub fn num_items(&self) -> usize {
        self.base.num_rows()
    }

    pub fn is_empty(&self) -> bool {
        self.base.is_empty()
    }

    pub fn push_row<'r, 'v: 'r>(&mut self, row: LogRow) {
        let id = self.append_row_fields(
            row.observed_time,
            row.severity,
            row.body.as_ref().map(|b| b.as_ref()),
            row.source_ip,
            row.source_port,
        );
        for (k, v) in row.attrs {
            self.base.append_row_attr(id, k.as_ref(), v);
        }
    }

    pub fn push_row_owned(&mut self, row: LogRowOwned) {
        let id = self.append_row_fields(
            row.observed_time,
            row.severity,
            row.body.as_ref().map(|b| b.as_ref()),
            row.source_ip,
            row.source_port,
        );
        for (k, v) in &row.attrs {
            self.base.append_row_attr(id, k.as_ref(), &v.as_ref_z());
        }
    }

    /// Appends the built-in log columns plus the source ip/port attributes for
    /// one row, returning the row id that user attributes should be parented to.
    /// Callers must follow up with [`Self::advance_row_id`] once attributes are
    /// written.
    fn append_row_fields(
        &mut self,
        observed_time: Option<DateTime<Utc>>,
        severity: Option<SeverityNumber>,
        body: Option<&str>,
        source_ip: Option<IpAddr>,
        source_port: Option<u16>,
    ) -> u16 {
        let id = self.base.next_row_id();
        self.rows.push(
            id,
            self.base.resource_id(),
            self.base.scope_id(),
            observed_time
                .map(|t| t.timestamp_nanos_opt().unwrap_or_default())
                .unwrap_or_default(),
            severity,
            body,
        );

        if let Some(source_ip) = source_ip {
            let text = match &self.source_ip {
                Some((last, text)) if *last == source_ip => text,
                _ => {
                    let mut text = ArrayString::new();
                    match source_ip {
                        IpAddr::V4(v4) => write_ipv4(&mut text, v4),
                        IpAddr::V6(v6) => write!(&mut text, "{v6}").unwrap(),
                    }
                    &self.source_ip.insert((source_ip, text)).1
                }
            };
            self.base.append_row_attr(
                id,
                super::RecordAttrKey::SourceAddress.as_str(),
                &super::Value::String(super::ZString::from_ref(text.as_str())),
            );
        }
        if let Some(source_port) = source_port {
            self.base.append_row_attr(
                id,
                super::RecordAttrKey::SourcePort.as_str(),
                &super::Value::Int64(source_port as i64),
            );
        }
        id
    }

    pub fn finish(&mut self) -> super::Payload {
        let mut otap_batch = OtapArrowRecords::Logs(Logs::default());
        // An empty builder yields an empty `Logs` payload and drains nothing:
        // scope attributes appended by `BaseBuilder::scope` (which does not bump
        // the row id) have to survive to the next batch. Callers don't need to
        // pre-check `is_empty()`.
        if self.num_rows() > 0 {
            let logs = self.rows.finish(self.base.qualified_scope_names()).unwrap();
            let base = self.base.finish();

            // `Logs` is the root payload; attribute batches are only attached when
            // they carry rows (mirrors `encode_logs_otap_batch` upstream).
            if logs.num_rows() > 0 {
                otap_batch.set(ArrowPayloadType::Logs, logs).unwrap();
                if base.row_attr.num_rows() > 0 {
                    otap_batch
                        .set(ArrowPayloadType::LogAttrs, base.row_attr)
                        .unwrap();
                }
                if base.resource_attr.num_rows() > 0 {
                    otap_batch
                        .set(ArrowPayloadType::ResourceAttrs, base.resource_attr)
                        .unwrap();
                }
                if base.scope_attr.num_rows() > 0 {
                    otap_batch
                        .set(ArrowPayloadType::ScopeAttrs, base.scope_attr)
                        .unwrap();
                }
            }
        }
        super::Payload { data: otap_batch }
    }
}

/// Wall-clock nanoseconds since the epoch, as the `time_unix_nano` column
/// stores it. `Utc::now().timestamp_nanos_opt()` gives the same value through a
/// calendar-date round trip.
fn now_nanos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as i64)
}

/// Dotted-quad text of `ip`, without going through `fmt`.
fn write_ipv4(out: &mut ArrayString<45>, ip: Ipv4Addr) {
    for (i, octet) in ip.octets().into_iter().enumerate() {
        if i > 0 {
            out.push('.');
        }
        if octet >= 100 {
            out.push((b'0' + octet / 100) as char);
        }
        if octet >= 10 {
            out.push((b'0' + octet / 10 % 10) as char);
        }
        out.push((b'0' + octet % 10) as char);
    }
}

/// Severity numbers run 0..=24.
const SEVERITY_SLOTS: usize = SeverityNumber::Fatal4 as usize + 1;

/// The `Logs` root table, as upstream's `LogsRecordBatchBuilder` lays out the
/// columns patrolli writes: same names, types and order, and the optional columns
/// left out when no row has a value. Values are kept plain per row and encoded
/// once in [`Self::finish`]:
///
/// - `scope.name` / `scope.version` are dictionaries keyed by the scope id, so a
///   row stores one id instead of hashing the same name every time.
/// - `severity_number` / `severity_text` are dictionaries over the distinct
///   severities, sharing one key column.
/// - `body.str` is plain: bodies are nearly all distinct, so deduplicating them
///   cost a hash and a probe per row for nothing.
#[derive(Default)]
struct LogColumns {
    id: Vec<u16>,
    resource_id: Vec<u16>,
    scope_id: Vec<u16>,
    time: Vec<i64>,
    observed_time: Vec<i64>,
    /// Per row, 1 + the index of its severity in `severities`; 0 for none.
    severity: Vec<u8>,
    /// Distinct severities, in first-seen order.
    severities: Vec<SeverityNumber>,
    /// Severity number → its `severity` key (0 when not seen yet).
    severity_slot: [u8; SEVERITY_SLOTS],
    /// Null for rows without a body.
    body: StringBuilder,
    any_body: bool,
}

impl LogColumns {
    #[inline]
    fn push(
        &mut self,
        id: u16,
        resource_id: u16,
        scope_id: u16,
        observed_time: i64,
        severity: Option<SeverityNumber>,
        body: Option<&str>,
    ) {
        self.id.push(id);
        self.resource_id.push(resource_id);
        self.scope_id.push(scope_id);
        self.time.push(now_nanos());
        self.observed_time.push(observed_time);
        self.severity.push(match severity {
            None => 0,
            Some(sev) => {
                let slot = &mut self.severity_slot[sev as usize];
                if *slot == 0 {
                    self.severities.push(sev);
                    *slot = self.severities.len() as u8;
                }
                *slot
            }
        });
        match body {
            Some(body) => {
                self.body.append_value(body);
                self.any_body = true;
            }
            None => self.body.append_null(),
        }
    }

    /// The `Logs` batch, leaving the columns empty for the next one.
    /// `scope_names[id]` is the name of scope `id`.
    fn finish(&mut self, scope_names: &[String]) -> Result<RecordBatch, ArrowError> {
        let len = self.id.len();
        let mut fields = Vec::with_capacity(8);
        let mut columns: Vec<ArrayRef> = Vec::with_capacity(8);

        let id = u16_column(&mut self.id);
        fields.push(Field::new(consts::ID, id.data_type().clone(), true).with_plain_encoding());
        columns.push(id);

        let resource_id = u16_column(&mut self.resource_id);
        let resource = StructArray::try_new_with_length(
            Fields::from(vec![
                Field::new(consts::ID, resource_id.data_type().clone(), true).with_plain_encoding(),
            ]),
            vec![resource_id],
            None,
            len,
        )?;
        fields.push(Field::new(
            consts::RESOURCE,
            resource.data_type().clone(),
            true,
        ));
        columns.push(Arc::new(resource));

        let scope = self.finish_scope(scope_names)?;
        fields.push(Field::new(consts::SCOPE, scope.data_type().clone(), true));
        columns.push(Arc::new(scope));

        for (name, values) in [
            (consts::TIME_UNIX_NANO, &mut self.time),
            (consts::OBSERVED_TIME_UNIX_NANO, &mut self.observed_time),
        ] {
            let array: ArrayRef = Arc::new(
                TimestampNanosecondArray::new(
                    ScalarBuffer::from(std::mem::replace(values, Vec::with_capacity(len))),
                    None,
                )
                .with_timezone(UTC_TIME_ZONE),
            );
            fields.push(Field::new(name, array.data_type().clone(), false));
            columns.push(array);
        }

        let severity = &self.severity;
        if !self.severities.is_empty() {
            let present =
                NullBuffer::new(BooleanBuffer::from_iter(severity.iter().map(|&s| s != 0)));
            let keys = UInt8Array::new(
                severity.iter().map(|&s| s.saturating_sub(1)).collect(),
                Some(present),
            );
            let number: ArrayRef = Arc::new(DictionaryArray::try_new(
                keys.clone(),
                Arc::new(Int32Array::from_iter_values(
                    self.severities.iter().map(|&s| s as i32),
                )),
            )?);
            let text: ArrayRef = Arc::new(DictionaryArray::try_new(
                keys,
                Arc::new(StringArray::from_iter_values(
                    self.severities.iter().map(|s| s.as_str_name()),
                )),
            )?);
            fields.push(Field::new(
                consts::SEVERITY_NUMBER,
                number.data_type().clone(),
                true,
            ));
            columns.push(number);
            fields.push(Field::new(
                consts::SEVERITY_TEXT,
                text.data_type().clone(),
                true,
            ));
            columns.push(text);
            self.severities.clear();
            self.severity_slot = [0; SEVERITY_SLOTS];
        }
        self.severity.clear();

        let body_bytes = self.body.values_slice().len();
        let body = std::mem::replace(
            &mut self.body,
            StringBuilder::with_capacity(len, body_bytes),
        )
        .finish();
        if std::mem::take(&mut self.any_body) {
            let present = body.nulls().cloned();
            let types: ArrayRef =
                Arc::new(UInt8Array::from_iter_values((0..len).map(
                    |i| match &present {
                        Some(nulls) if nulls.is_null(i) => AttributeValueType::Empty as u8,
                        _ => AttributeValueType::Str as u8,
                    },
                )));
            let body: ArrayRef = Arc::new(body);
            let body = StructArray::try_new(
                Fields::from(vec![
                    Field::new(consts::ATTRIBUTE_TYPE, types.data_type().clone(), false),
                    Field::new(consts::ATTRIBUTE_STR, body.data_type().clone(), true),
                ]),
                vec![types, body],
                present,
            )?;
            fields.push(Field::new(consts::BODY, body.data_type().clone(), true));
            columns.push(Arc::new(body));
        }

        RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)
    }

    /// `scope`: `{id, name, version}`, the last two dictionaries keyed by `id`.
    fn finish_scope(&mut self, scope_names: &[String]) -> Result<StructArray, ArrowError> {
        let len = self.scope_id.len();
        // Rows pushed before any scope was set carry id 0 with no name behind it.
        let distinct = self
            .scope_id
            .iter()
            .max()
            .map_or(0, |&m| m as usize + 1)
            .max(scope_names.len());
        let names = StringArray::from_iter_values(
            (0..distinct).map(|i| scope_names.get(i).map_or(super::builder::UNSCOPED, |n| n)),
        );
        let ids = std::mem::replace(&mut self.scope_id, Vec::with_capacity(len));
        let name: ArrayRef = if distinct <= u8::MAX as usize + 1 {
            let keys = UInt8Array::from_iter_values(ids.iter().map(|&i| i as u8));
            Arc::new(DictionaryArray::try_new(keys, Arc::new(names))?)
        } else {
            let keys = UInt16Array::from_iter_values(ids.iter().copied());
            Arc::new(DictionaryArray::try_new(keys, Arc::new(names))?)
        };
        let version: ArrayRef = Arc::new(DictionaryArray::try_new(
            UInt8Array::from(vec![0; len]),
            Arc::new(StringArray::from(vec![env!("CARGO_PKG_VERSION")])),
        )?);
        let id: ArrayRef = Arc::new(UInt16Array::new(ScalarBuffer::from(ids), None));
        StructArray::try_new_with_length(
            Fields::from(vec![
                Field::new(consts::ID, id.data_type().clone(), true).with_plain_encoding(),
                Field::new(consts::NAME, name.data_type().clone(), true),
                Field::new(consts::VERSION, version.data_type().clone(), true),
            ]),
            vec![id, name, version],
            None,
            len,
        )
    }
}

/// `values` as a column, leaving room for as many in their place.
fn u16_column(values: &mut Vec<u16>) -> ArrayRef {
    let len = values.len();
    Arc::new(UInt16Array::new(
        ScalarBuffer::from(std::mem::replace(values, Vec::with_capacity(len))),
        None,
    ))
}

#[cfg(test)]
mod tests {
    use super::{LogRow, LogsBuilder};
    use crate::payload::attr::{get_record_attr, parent_id_values};
    use crate::payload::{Resource, Scope, Value, ZString};
    use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
    use smol_str::SmolStr;
    use std::borrow::Cow;
    use std::net::IpAddr;
    use uuid::Uuid;

    fn scope() -> Scope {
        Scope {
            name: SmolStr::new_static("test"),
            listener_id: None,
            listener_type: None,
            listener_name: None,
            monitor_type: None,
            node_id: None,
        }
    }

    fn resource(host: &str) -> Resource {
        Resource {
            host_id: Uuid::nil(),
            host_name: SmolStr::new(host),
            host_timezone: None,
            endpoint_id: None,
            endpoint_name: None,
            endpoint_type: None,
            item_id: Uuid::nil(),
            item_name: SmolStr::new_static("item"),
        }
    }

    fn plain_row() -> LogRow<'static> {
        LogRow {
            observed_time: None,
            severity: None,
            body: Some(Cow::Borrowed("msg")),
            source_ip: None,
            source_port: None,
            attrs: &[],
        }
    }

    fn push(builder: &mut LogsBuilder, row: LogRow) {
        builder.push_row(row);
    }

    /// `(payload type, arrow schema, row count)` for every batch a payload carries —
    /// the part of the output that replacing the builders in `finish` could plausibly
    /// disturb. Deliberately not the values, which carry `Utc::now()` timestamps.
    fn shape(pl: &crate::payload::Payload) -> Vec<(String, String, usize)> {
        pl.data
            .allowed_payload_types()
            .iter()
            .filter_map(|&pt| {
                pl.data
                    .get(pt)
                    .map(|b| (format!("{pt:?}"), format!("{:?}", b.schema()), b.num_rows()))
            })
            .collect()
    }

    fn resource_attr_rows(pl: &crate::payload::Payload) -> usize {
        pl.data
            .get(ArrowPayloadType::ResourceAttrs)
            .map(|b| b.num_rows())
            .unwrap_or(0)
    }

    #[test]
    fn empty_builder_finishes_empty() {
        let mut b = LogsBuilder::new();
        assert!(b.is_empty());
        assert_eq!(b.num_rows(), 0);
        let pl = b.finish();
        assert!(pl.is_empty());
        assert_eq!(pl.num_items(), 0);
    }

    #[test]
    fn counts_rows_and_finishes() {
        let mut b = LogsBuilder::new();
        b.scope(scope());
        for _ in 0..3 {
            push(&mut b, plain_row());
        }
        assert_eq!(b.num_rows(), 3);
        assert!(!b.is_empty());
        assert_eq!(b.finish().num_items(), 3);
    }

    #[test]
    fn finish_resets_for_reuse() {
        let mut b = LogsBuilder::new();
        b.scope(scope());
        push(&mut b, plain_row());
        push(&mut b, plain_row());
        assert_eq!(b.finish().num_items(), 2);
        // The builder resets its bookkeeping so it can be reused for the next batch.
        assert!(b.is_empty());
        assert_eq!(b.num_rows(), 0);
        push(&mut b, plain_row());
        assert_eq!(b.finish().num_items(), 1);
    }

    /// Reusing a builder has to be indistinguishable from starting a fresh one, which
    /// is what makes replacing the inner builders in `finish` safe: it pins that the
    /// replacement neither drops a column nor adds one back.
    #[test]
    fn reused_builder_emits_the_same_shape_as_a_fresh_one() {
        fn batch(b: &mut LogsBuilder) -> crate::payload::Payload {
            // A scope with an optional field set, and rows carrying attributes, so that
            // every one of the four batches is populated — the shared `scope()` fixture
            // is all-`None` and `plain_row()` has no attributes.
            b.scope(Scope {
                listener_type: Some(SmolStr::new_static("netflow")),
                ..scope()
            });
            b.resource(resource("h"));
            let attrs = [(
                ZString::from_static("k"),
                Value::String(ZString::from_static("v")),
            )];
            for _ in 0..3 {
                push(
                    b,
                    LogRow {
                        attrs: &attrs,
                        ..plain_row()
                    },
                );
            }
            b.finish()
        }

        let mut reused = LogsBuilder::new();
        drop(batch(&mut reused));
        let second = batch(&mut reused);

        let first = batch(&mut LogsBuilder::new());
        let want = shape(&first);
        // Guard against a vacuous pass: logs plus the three attribute batches.
        assert_eq!(
            want.len(),
            4,
            "expected all log batches to be present, got {:?}",
            want.iter().map(|(t, ..)| t).collect::<Vec<_>>()
        );
        assert_eq!(shape(&second), want);
    }

    /// A reused builder must not carry per-column state from earlier batches.
    ///
    /// The OTAP builders keep an "only nulls/defaults so far" prefix for optional
    /// columns nothing has written to yet, and `finish` does not clear it. A batch of
    /// string-only attributes therefore leaves a null bit per attribute row parked in
    /// the unused `bytes`/`ser` value columns — leaked for the process lifetime, and
    /// back-filled into the array the first time a batch does use that column, making
    /// it longer than the batch it belongs to.
    #[test]
    fn attr_value_columns_do_not_accumulate_across_batches() {
        let mut b = LogsBuilder::new();
        b.scope(scope());
        let string_attr = [(
            ZString::from_static("k"),
            Value::String(ZString::from_static("v")),
        )];
        for _ in 0..8 {
            b.push_row(LogRow {
                // `body: None` is what every listener passes.
                body: None,
                attrs: &string_attr,
                ..Default::default()
            });
        }
        assert_eq!(b.finish().num_items(), 8);

        // First use of the `bytes` value column, in a one-row batch.
        let bytes_attr = [(
            ZString::from_static("k"),
            Value::Bytes(Cow::Borrowed(b"raw".as_slice())),
        )];
        b.push_row(LogRow {
            body: None,
            attrs: &bytes_attr,
            ..Default::default()
        });
        let pl = b.finish();
        let attr = get_record_attr(&pl).expect("log attrs present");
        assert_eq!(
            attr.num_rows(),
            1,
            "attribute batch must only hold this batch's rows"
        );
    }

    #[test]
    fn repeated_resource_is_deduped() {
        // Same resource referenced by both rows → one resource's worth of attrs.
        let mut deduped = LogsBuilder::new();
        deduped.resource(resource("h"));
        push(&mut deduped, plain_row());
        deduped.resource(resource("h"));
        push(&mut deduped, plain_row());
        let one = resource_attr_rows(&deduped.finish());

        // Two distinct resources → twice as many resource attr rows.
        let mut distinct = LogsBuilder::new();
        distinct.resource(resource("h"));
        push(&mut distinct, plain_row());
        distinct.resource(resource("other"));
        push(&mut distinct, plain_row());
        let two = resource_attr_rows(&distinct.finish());

        assert!(one > 0, "resource attrs should be emitted");
        assert_eq!(
            two,
            2 * one,
            "distinct resources must not collapse together"
        );
    }

    #[test]
    fn source_ip_and_port_become_record_attributes() {
        let mut b = LogsBuilder::new();
        b.scope(scope());
        push(
            &mut b,
            LogRow {
                observed_time: None,
                severity: None,
                body: None,
                source_ip: Some(IpAddr::from([10, 0, 0, 1])),
                source_port: Some(443),
                attrs: &[],
            },
        );
        let pl = b.finish();
        let attr = get_record_attr(&pl).expect("log attrs present");
        // source.address is written as a string, source.port as an int → decimal.
        let addr = parent_id_values(attr, "source.address").unwrap();
        assert_eq!(addr.get(&0).map(|s| s.as_str()), Some("10.0.0.1"));
        let port = parent_id_values(attr, "source.port").unwrap();
        assert_eq!(port.get(&0).map(|s| s.as_str()), Some("443"));
    }

    /// A log record as OTLP sees it, after the payload is decoded by upstream
    /// (`OtapArrowRecords` → `ExportLogsServiceRequest`): what the batches mean,
    /// independent of how the columns are encoded. `time_unix_nano` is left out,
    /// being the push time.
    #[derive(Debug, PartialEq)]
    struct OtlpRecord {
        resource: std::collections::BTreeMap<String, String>,
        scope: (String, String, std::collections::BTreeMap<String, String>),
        severity: (i32, String),
        body: Option<String>,
        observed_time: u64,
        attrs: std::collections::BTreeMap<String, String>,
    }

    fn otlp_records(pl: &crate::payload::Payload) -> Vec<OtlpRecord> {
        use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::{
            AnyValue, KeyValue, any_value,
        };
        use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::LogsData;
        use prost::Message;

        fn val(v: &Option<AnyValue>) -> String {
            match v.as_ref().and_then(|v| v.value.as_ref()) {
                None => "empty".into(),
                Some(any_value::Value::StringValue(s)) => format!("str:{s}"),
                Some(any_value::Value::IntValue(i)) => format!("int:{i}"),
                Some(any_value::Value::DoubleValue(d)) => format!("double:{d}"),
                Some(any_value::Value::BoolValue(b)) => format!("bool:{b}"),
                Some(any_value::Value::BytesValue(b)) => format!("bytes:{b:?}"),
                Some(other) => format!("{other:?}"),
            }
        }
        fn kvs(kvs: &[KeyValue]) -> std::collections::BTreeMap<String, String> {
            kvs.iter()
                .map(|kv| (kv.key.clone(), val(&kv.value)))
                .collect()
        }

        let bytes = pl
            .serialize_as(crate::config::Format::ProtoOtlp)
            .unwrap()
            .remove(0);
        let data = LogsData::decode(bytes.as_ref()).unwrap();
        let mut out = Vec::new();
        for rl in &data.resource_logs {
            let resource = kvs(&rl.resource.as_ref().unwrap().attributes);
            for sl in &rl.scope_logs {
                let scope = sl.scope.as_ref().unwrap();
                for r in &sl.log_records {
                    out.push(OtlpRecord {
                        resource: resource.clone(),
                        scope: (
                            scope.name.clone(),
                            scope.version.clone(),
                            kvs(&scope.attributes),
                        ),
                        severity: (r.severity_number, r.severity_text.clone()),
                        body: r.body.as_ref().map(|b| val(&Some(b.clone()))),
                        observed_time: r.observed_time_unix_nano,
                        attrs: kvs(&r.attributes),
                    });
                }
            }
        }
        out.sort_by(|a, b| a.attrs["n"].cmp(&b.attrs["n"]));
        out
    }

    /// The batches decode to the records that were pushed, through every column
    /// the builder writes: two scopes and two resources (one of them revisited),
    /// every value type, nested values, missing severity/body and both address
    /// families.
    #[test]
    fn decodes_to_the_pushed_records() {
        use chrono::TimeZone;
        use opentelemetry_proto::tonic::logs::v1::SeverityNumber;
        use std::collections::BTreeMap;

        let observed = chrono::Utc.timestamp_opt(1_700_000_000, 5).unwrap();
        let mut b = LogsBuilder::new();
        b.scope(Scope {
            listener_type: Some(SmolStr::new_static("syslog")),
            listener_id: Some(Uuid::nil()),
            ..scope()
        });
        b.resource(resource("h1"));
        let attrs = [
            (ZString::from_static("n"), Value::Int64(1)),
            (
                ZString::from_static("s"),
                Value::String(ZString::from_ref("x")),
            ),
            (ZString::from_static("d"), Value::Double(1.5)),
            (ZString::from_static("b"), Value::Bool(true)),
            (
                ZString::from_static("y"),
                Value::Bytes(Cow::Borrowed(b"\x01\x02".as_slice())),
            ),
            (ZString::from_static("e"), Value::Null),
            (
                ZString::from_static("arr"),
                Value::Array(vec![Value::KvList(vec![
                    (
                        ZString::from_static("k"),
                        Value::String(ZString::from_ref("v")),
                    ),
                    (ZString::from_static("i"), Value::Int64(-3)),
                ])]),
            ),
        ];
        b.push_row(LogRow {
            observed_time: None,
            severity: Some(SeverityNumber::Info),
            body: None,
            source_ip: Some(IpAddr::from([10, 0, 0, 255])),
            source_port: Some(514),
            attrs: &attrs,
        });

        b.scope(Scope {
            name: SmolStr::new_static("other"),
            ..scope()
        });
        b.resource(resource("h2"));
        let attrs = [(ZString::from_static("n"), Value::Int64(2))];
        b.push_row(LogRow {
            observed_time: Some(observed),
            severity: None,
            body: Some(Cow::Borrowed("third")),
            source_ip: Some("2001:db8::1".parse().unwrap()),
            source_port: None,
            attrs: &attrs,
        });

        // Back to the first scope and resource.
        b.scope(Scope {
            listener_type: Some(SmolStr::new_static("syslog")),
            listener_id: Some(Uuid::nil()),
            ..scope()
        });
        b.resource(resource("h1"));
        let attrs = [(ZString::from_static("n"), Value::Int64(3))];
        b.push_row(LogRow {
            observed_time: None,
            severity: Some(SeverityNumber::Info),
            body: Some(Cow::Borrowed("fourth")),
            source_ip: Some(IpAddr::from([10, 0, 0, 255])),
            source_port: Some(515),
            attrs: &attrs,
        });

        let map = |kv: &[(&str, &str)]| -> BTreeMap<String, String> {
            kv.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        let nil = Uuid::nil().to_string();
        let res = |host: &str| {
            map(&[
                ("service.name", &format!("str:{host}")),
                ("service.instance.id", &format!("str:{nil}")),
                ("host.name", &format!("str:{host}")),
                ("host.id", &format!("str:{nil}")),
                ("patrolli.item.name", "str:item"),
                ("patrolli.item.id", &format!("str:{nil}")),
            ])
        };
        let ver = env!("CARGO_PKG_VERSION").to_string();
        let syslog_scope = (
            "patrolli.test".to_string(),
            ver.clone(),
            map(&[
                ("patrolli.listener.id", &format!("str:{nil}")),
                ("patrolli.listener.type", "str:syslog"),
            ]),
        );
        let want = vec![
            OtlpRecord {
                resource: res("h1"),
                scope: syslog_scope.clone(),
                severity: (SeverityNumber::Info as i32, "SEVERITY_NUMBER_INFO".into()),
                body: None,
                observed_time: 0,
                attrs: map(&[
                    ("n", "int:1"),
                    ("s", "str:x"),
                    ("d", "double:1.5"),
                    ("b", "bool:true"),
                    ("y", "bytes:[1, 2]"),
                    ("e", "empty"),
                    ("arr.0.k", "str:v"),
                    ("arr.0.i", "int:-3"),
                    ("source.address", "str:10.0.0.255"),
                    ("source.port", "int:514"),
                ]),
            },
            OtlpRecord {
                resource: res("h2"),
                scope: ("patrolli.other".into(), ver.clone(), BTreeMap::new()),
                severity: (0, String::new()),
                body: Some("str:third".into()),
                observed_time: observed.timestamp_nanos_opt().unwrap() as u64,
                attrs: map(&[("n", "int:2"), ("source.address", "str:2001:db8::1")]),
            },
            OtlpRecord {
                resource: res("h1"),
                scope: syslog_scope,
                severity: (SeverityNumber::Info as i32, "SEVERITY_NUMBER_INFO".into()),
                body: Some("str:fourth".into()),
                observed_time: 0,
                attrs: map(&[
                    ("n", "int:3"),
                    ("source.address", "str:10.0.0.255"),
                    ("source.port", "int:515"),
                ]),
            },
        ];
        assert_eq!(otlp_records(&b.finish()), want);
    }

    #[test]
    fn write_ipv4_matches_display() {
        for ip in [
            [0, 0, 0, 0],
            [255, 255, 255, 255],
            [10, 9, 100, 99],
            [1, 20, 199, 200],
        ] {
            let ip = std::net::Ipv4Addr::from(ip);
            let mut out = arrayvec::ArrayString::new();
            super::write_ipv4(&mut out, ip);
            assert_eq!(out.as_str(), ip.to_string());
        }
    }
}
