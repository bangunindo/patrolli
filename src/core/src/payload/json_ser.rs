// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Streaming JSON serializers for [`Payload`].
//!
//! Reads the OTAP Arrow `RecordBatch`es directly (no OTLP protobuf round-trip)
//! and **streams** JSON straight into the output buffer via `serde::Serialize`
//! wrappers — no intermediate `serde_json::Value` tree, no owned `String` keys.
//! Four shapes are selected by ([`Layout`], [`Mode`]):
//!
//! - `Layout::Denorm` — one JSON object per record, in an envelope
//!   `{kind, resource, resourceSchemaUrl, scope, scopeSchemaUrl, record}`.
//! - `Layout::Batch` — one JSON object for the whole payload, in the OTLP
//!   `{resourceLogs|resourceMetrics|resourceSpans: [...]}` hierarchy.
//! - `Mode::Nested` — every `Vec<KeyValue>` (attributes everywhere, plus metric
//!   `metadata`) has its dotted keys expanded to nested objects/arrays, and
//!   AnyValues are unwrapped to native JSON.
//! - `Mode::Otlp` — attributes as `[{key, value:{stringValue|...}}]` arrays
//!   (OTLP AnyValue wrappers), keys kept literal.
//!
//! Record shapes and conventions are proto-serde-faithful (matching
//! `opentelemetry-proto`): camelCase, hex trace/span ids, `*UnixNano`/u64 as
//! decimal strings, `NumberDataPoint.asInt` as a number, AnyValue `intValue` as
//! a string, bytes as base64. CBOR-encoded `ser` values (Map/Slice attributes
//! and log bodies) are decoded.
//!
//! Object key order follows code/data order, not the alphabetical order of the
//! previous `serde_json::Value` (`BTreeMap`) implementation; the output is
//! semantically identical JSON. Nothing downstream depends on key order.
//!
//! Self-contained — shares no code with `nested.rs`. Uses otel-arrow's *public*
//! building blocks only ([`OtapArrowRecords`], `schema::consts`,
//! `AttributeValueType`, the primitive [`MaybeDictArrayAccessor`]); string/binary
//! dict-aware accessors are defined here over the raw `arrow` crate.

use anyhow::{Context, Result};
use arrow::array::{
    Array, ArrayRef, AsArray, BooleanArray, DictionaryArray, Float64Array, ListArray,
    PrimitiveArray, RecordBatch, StringArray, StructArray, UInt64Array,
};
use arrow::datatypes::{
    ArrowPrimitiveType, DataType, DurationNanosecondType, Float64Type, Int32Type, Int64Type,
    TimeUnit, TimestampNanosecondType, UInt8Type, UInt16Type, UInt32Type, UInt64Type,
};
use base64::display::Base64Display;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use rustc_hash::FxHashMap;
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::ops::Range;

use otel_arrow_dfe_pdata::OtapArrowRecords;
use otel_arrow_dfe_pdata::arrays::{MaybeDictArrayAccessor, NullableArrayAccessor};
use otel_arrow_dfe_pdata::otlp::attributes::AttributeValueType;
use otel_arrow_dfe_pdata::otlp::metrics::MetricType;
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
use otel_arrow_dfe_pdata::schema::{consts, is_valid_timestamp_time_zone};

use super::Payload;

/// Output layout: one JSON per record, or one JSON per payload (OTLP hierarchy).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Layout {
    Denorm,
    Batch,
}

/// Attribute rendering: nested (dotted-expanded, native values) or OTLP
/// (`[{key, value:{...}}]` AnyValue wrappers).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Nested,
    Otlp,
}

/// Serialize `payload` as JSON in the given layout + attribute mode.
pub(super) fn to_json(payload: &Payload, layout: Layout, mode: Mode) -> Result<Vec<Bytes>> {
    let records = &payload.data;
    match records {
        OtapArrowRecords::Logs(_) => logs(records, layout, mode),
        OtapArrowRecords::Traces(_) => traces(records, layout, mode),
        OtapArrowRecords::Metrics(_) => metrics(records, layout, mode),
    }
}

// ============================ leaf value serializers ============================

/// A u64 (or timestamp) rendered as a JSON **string**, per the proto JSON spec.
/// `collect_str` writes the `Display` output straight into the writer (quoted)
/// with no `String` allocation.
struct U64Str(u64);
impl Serialize for U64Str {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&self.0)
    }
}

/// An i64 rendered as a JSON **string** (OTLP `intValue` / CBOR integer).
struct I64Str(i64);
impl Serialize for I64Str {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&self.0)
    }
}

/// An i128 rendered as a JSON **string** (CBOR integers can exceed i64/u64).
struct I128Str(i128);
impl Serialize for I128Str {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&self.0)
    }
}

/// Bytes rendered as a base64 JSON string, streamed (no intermediate `String`).
struct Base64Val<'a>(&'a [u8]);
impl Serialize for Base64Val<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&Base64Display::new(self.0, &STANDARD))
    }
}

/// An optional byte id rendered as lowercase hex; absent → `""` (proto default).
/// Ids are ≤16 bytes, so the hex fits a stack buffer with no allocation.
struct HexStr<'a>(Option<&'a [u8]>);
impl Serialize for HexStr<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            None => s.serialize_str(""),
            Some(b) => {
                let n = b.len() * 2;
                let mut buf = [0u8; 64];
                if n <= buf.len() && const_hex::encode_to_slice(b, &mut buf[..n]).is_ok() {
                    // encode_to_slice writes valid lowercase-hex ASCII.
                    let st = core::str::from_utf8(&buf[..n]).unwrap_or("");
                    s.serialize_str(st)
                } else {
                    s.serialize_str(&const_hex::encode(b))
                }
            }
        }
    }
}

/// f64 → JSON number, with the proto JSON convention for non-finite values
/// (NaN / Infinity / -Infinity serialize as strings).
struct F64Num(f64);
impl Serialize for F64Num {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let v = self.0;
        if v.is_nan() {
            s.serialize_str("NaN")
        } else if v.is_infinite() {
            s.serialize_str(if v > 0.0 { "Infinity" } else { "-Infinity" })
        } else {
            s.serialize_f64(v)
        }
    }
}

/// A proto3-optional double: the value as a number, or `null` if absent.
struct OptF64Num(Option<f64>);
impl Serialize for OptF64Num {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            None => s.serialize_none(),
            Some(v) => F64Num(v).serialize(s),
        }
    }
}

/// An empty JSON array `[]` (used for `entityRefs`).
struct EmptyArr;
impl Serialize for EmptyArr {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_seq(Some(0))?.end()
    }
}

/// A 0-or-1 entry object: `{}` or `{key: val}`. Used for OTLP AnyValue wrappers.
fn single<S: Serializer, T: Serialize>(
    s: S,
    key: &'static str,
    val: Option<T>,
) -> Result<S::Ok, S::Error> {
    let mut m = s.serialize_map(None)?;
    if let Some(v) = val {
        m.serialize_entry(key, &v)?;
    }
    m.end()
}

/// An empty JSON object `{}`.
fn empty_object<S: Serializer>(s: S) -> Result<S::Ok, S::Error> {
    s.serialize_map(None)?.end()
}

// small column readers that resolve a row to a plain Rust scalar ------------------

fn str_at<'a>(col: &Option<StrCol<'a>>, row: usize) -> &'a str {
    col.as_ref().and_then(|c| c.at(row)).unwrap_or("")
}

fn bin_at<'a>(col: &Option<BinCol<'a>>, row: usize) -> Option<&'a [u8]> {
    col.as_ref().and_then(|c| c.at(row))
}

fn u32_at(col: &Option<Prim<'_, UInt32Type>>, row: usize) -> u32 {
    col.as_ref().and_then(|a| a.value_at(row)).unwrap_or(0)
}

fn i32_at(col: &Option<Prim<'_, Int32Type>>, row: usize) -> i32 {
    col.as_ref().and_then(|a| a.value_at(row)).unwrap_or(0)
}

/// Timestamp (ns) column as u64; absent → 0.
fn ts_at(col: &Option<Prim<'_, TimestampNanosecondType>>, row: usize) -> u64 {
    col.as_ref().and_then(|a| a.value_at(row)).unwrap_or(0) as u64
}

// ============================ CBOR serializers ============================

/// Stream a CBOR `ser` value as native JSON (Nested mode): arrays stay arrays,
/// maps become objects with keys kept verbatim, bytes become base64.
pub(crate) struct CborNativeSer<'a>(pub(crate) &'a ciborium::value::Value);
impl Serialize for CborNativeSer<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use ciborium::value::Value as C;
        match self.0 {
            C::Null => s.serialize_none(),
            C::Bool(b) => s.serialize_bool(*b),
            C::Integer(i) => {
                let n = i128::from(*i);
                if let Ok(v) = i64::try_from(n) {
                    s.serialize_i64(v)
                } else if let Ok(v) = u64::try_from(n) {
                    s.serialize_u64(v)
                } else {
                    F64Num(n as f64).serialize(s)
                }
            }
            C::Float(f) => F64Num(*f).serialize(s),
            C::Bytes(b) => Base64Val(b).serialize(s),
            C::Text(t) => s.serialize_str(t),
            C::Array(a) => {
                let mut seq = s.serialize_seq(Some(a.len()))?;
                for v in a {
                    seq.serialize_element(&CborNativeSer(v))?;
                }
                seq.end()
            }
            C::Map(m) => {
                let mut map = s.serialize_map(None)?;
                for (k, v) in m {
                    let key = match k {
                        C::Text(t) => t.as_str(),
                        C::Null => "",
                        _ => continue,
                    };
                    map.serialize_entry(key, &CborNativeSer(v))?;
                }
                map.end()
            }
            C::Tag(_, inner) => CborNativeSer(inner).serialize(s),
            _ => s.serialize_none(),
        }
    }
}

/// Stream a CBOR `ser` value as an OTLP AnyValue wrapper (Otlp mode).
struct CborOtlpSer<'a>(&'a ciborium::value::Value);
impl Serialize for CborOtlpSer<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use ciborium::value::Value as C;
        match self.0 {
            C::Null => empty_object(s),
            C::Bool(b) => single(s, "boolValue", Some(*b)),
            C::Integer(i) => single(s, "intValue", Some(I128Str(i128::from(*i)))),
            C::Float(f) => single(s, "doubleValue", Some(F64Num(*f))),
            C::Bytes(b) => single(s, "bytesValue", Some(Base64Val(b))),
            C::Text(t) => single(s, "stringValue", Some(t.as_str())),
            C::Array(a) => single(s, "arrayValue", Some(CborOtlpValues(a))),
            C::Map(m) => single(s, "kvlistValue", Some(CborOtlpKvValues(m))),
            C::Tag(_, inner) => CborOtlpSer(inner).serialize(s),
            _ => empty_object(s),
        }
    }
}

/// `{"values": [<cbor otlp>...]}` payload of an OTLP `arrayValue`.
struct CborOtlpValues<'a>(&'a [ciborium::value::Value]);
impl Serialize for CborOtlpValues<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("values", &CborOtlpArray(self.0))?;
        m.end()
    }
}

struct CborOtlpArray<'a>(&'a [ciborium::value::Value]);
impl Serialize for CborOtlpArray<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for v in self.0 {
            seq.serialize_element(&CborOtlpSer(v))?;
        }
        seq.end()
    }
}

/// `{"values": [{"key":k,"value":<cbor otlp>}...]}` payload of an OTLP `kvlistValue`.
struct CborOtlpKvValues<'a>(&'a [(ciborium::value::Value, ciborium::value::Value)]);
impl Serialize for CborOtlpKvValues<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("values", &CborOtlpKvArray(self.0))?;
        m.end()
    }
}

struct CborOtlpKvArray<'a>(&'a [(ciborium::value::Value, ciborium::value::Value)]);
impl Serialize for CborOtlpKvArray<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use ciborium::value::Value as C;
        let mut seq = s.serialize_seq(None)?;
        for (k, v) in self.0 {
            let key = match k {
                C::Text(t) => t.as_str(),
                C::Null => "",
                _ => continue,
            };
            seq.serialize_element(&OtlpKv {
                key,
                val: CborOtlpSer(v),
            })?;
        }
        seq.end()
    }
}

/// `{"key": k, "value": v}` OTLP KeyValue object (reused for attributes + CBOR).
struct OtlpKv<'a, T: Serialize> {
    key: &'a str,
    val: T,
}
impl<T: Serialize> Serialize for OtlpKv<'_, T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("key", self.key)?;
        m.serialize_entry("value", &self.val)?;
        m.end()
    }
}

// ============================ column accessors ============================

type Prim<'a, T> = MaybeDictArrayAccessor<'a, PrimitiveArray<T>>;

/// Build a dict-aware accessor for a primitive column, if present and typed as expected.
fn prim<'a, T: ArrowPrimitiveType>(arr: Option<&'a ArrayRef>) -> Option<Prim<'a, T>> {
    MaybeDictArrayAccessor::<PrimitiveArray<T>>::try_new(arr?).ok()
}

/// Accessor for a `Timestamp(Nanosecond)` column. [`prim`] matches the type exactly, time zone
/// included, but OTAP producers tag these `"UTC"` (or `"+00:00"`, or leave the zone off), so
/// the zone is checked here instead.
fn ts_col(arr: Option<&ArrayRef>) -> Option<Prim<'_, TimestampNanosecondType>> {
    let arr = arr?;
    match arr.data_type() {
        DataType::Timestamp(TimeUnit::Nanosecond, tz)
            if is_valid_timestamp_time_zone(tz.as_deref()) =>
        {
            arr.as_primitive_opt().map(MaybeDictArrayAccessor::Native)
        }
        _ => prim(Some(arr)),
    }
}

/// Dict-aware accessor for a Utf8 column (plain or dictionary-encoded).
enum StrCol<'a> {
    Plain(&'a StringArray),
    Dict8(&'a DictionaryArray<UInt8Type>, &'a StringArray),
    Dict16(&'a DictionaryArray<UInt16Type>, &'a StringArray),
}

impl<'a> StrCol<'a> {
    fn try_new(arr: Option<&'a ArrayRef>) -> Option<Self> {
        let arr = arr?;
        match arr.data_type() {
            DataType::Utf8 => arr
                .as_any()
                .downcast_ref::<StringArray>()
                .map(StrCol::Plain),
            DataType::Dictionary(k, v) if **v == DataType::Utf8 => match **k {
                DataType::UInt8 => {
                    let d = arr.as_any().downcast_ref::<DictionaryArray<UInt8Type>>()?;
                    let vals = d.values().as_any().downcast_ref::<StringArray>()?;
                    Some(StrCol::Dict8(d, vals))
                }
                DataType::UInt16 => {
                    let d = arr.as_any().downcast_ref::<DictionaryArray<UInt16Type>>()?;
                    let vals = d.values().as_any().downcast_ref::<StringArray>()?;
                    Some(StrCol::Dict16(d, vals))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Borrow the string at `i` for the column's full lifetime — so dotted-key
    /// expansion can keep `&'a str` keys in the nested tree without allocating.
    fn at(&self, i: usize) -> Option<&'a str> {
        match self {
            StrCol::Plain(a) => a.is_valid(i).then(|| a.value(i)),
            StrCol::Dict8(d, vals) => {
                if !d.is_valid(i) {
                    return None;
                }
                let k = d.keys().value(i) as usize;
                vals.is_valid(k).then(|| vals.value(k))
            }
            StrCol::Dict16(d, vals) => {
                if !d.is_valid(i) {
                    return None;
                }
                let k = d.keys().value(i) as usize;
                vals.is_valid(k).then(|| vals.value(k))
            }
        }
    }
}

/// Dict-aware accessor for Binary / FixedSizeBinary columns (plain or
/// dictionary-encoded). Used for ids and bytes/ser attribute values.
enum BinCol<'a> {
    Binary(&'a arrow::array::BinaryArray),
    Fixed(&'a arrow::array::FixedSizeBinaryArray),
    Dict8Bin(
        &'a DictionaryArray<UInt8Type>,
        &'a arrow::array::BinaryArray,
    ),
    Dict16Bin(
        &'a DictionaryArray<UInt16Type>,
        &'a arrow::array::BinaryArray,
    ),
    Dict8Fixed(
        &'a DictionaryArray<UInt8Type>,
        &'a arrow::array::FixedSizeBinaryArray,
    ),
    Dict16Fixed(
        &'a DictionaryArray<UInt16Type>,
        &'a arrow::array::FixedSizeBinaryArray,
    ),
}

impl<'a> BinCol<'a> {
    fn try_new(arr: Option<&'a ArrayRef>) -> Option<Self> {
        use arrow::array::{BinaryArray, FixedSizeBinaryArray};
        let arr = arr?;
        match arr.data_type() {
            DataType::Binary => arr
                .as_any()
                .downcast_ref::<BinaryArray>()
                .map(BinCol::Binary),
            DataType::FixedSizeBinary(_) => arr
                .as_any()
                .downcast_ref::<FixedSizeBinaryArray>()
                .map(BinCol::Fixed),
            DataType::Dictionary(k, v) => match (&**k, &**v) {
                (DataType::UInt8, DataType::Binary) => {
                    let d = arr.as_any().downcast_ref::<DictionaryArray<UInt8Type>>()?;
                    let vals = d.values().as_any().downcast_ref::<BinaryArray>()?;
                    Some(BinCol::Dict8Bin(d, vals))
                }
                (DataType::UInt16, DataType::Binary) => {
                    let d = arr.as_any().downcast_ref::<DictionaryArray<UInt16Type>>()?;
                    let vals = d.values().as_any().downcast_ref::<BinaryArray>()?;
                    Some(BinCol::Dict16Bin(d, vals))
                }
                (DataType::UInt8, DataType::FixedSizeBinary(_)) => {
                    let d = arr.as_any().downcast_ref::<DictionaryArray<UInt8Type>>()?;
                    let vals = d.values().as_any().downcast_ref::<FixedSizeBinaryArray>()?;
                    Some(BinCol::Dict8Fixed(d, vals))
                }
                (DataType::UInt16, DataType::FixedSizeBinary(_)) => {
                    let d = arr.as_any().downcast_ref::<DictionaryArray<UInt16Type>>()?;
                    let vals = d.values().as_any().downcast_ref::<FixedSizeBinaryArray>()?;
                    Some(BinCol::Dict16Fixed(d, vals))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn at(&self, i: usize) -> Option<&'a [u8]> {
        match self {
            BinCol::Binary(a) => a.is_valid(i).then(|| a.value(i)),
            BinCol::Fixed(a) => a.is_valid(i).then(|| a.value(i)),
            BinCol::Dict8Bin(d, vals) => {
                if !d.is_valid(i) {
                    return None;
                }
                vals.value_at_dict(d.keys().value(i) as usize)
            }
            BinCol::Dict16Bin(d, vals) => {
                if !d.is_valid(i) {
                    return None;
                }
                vals.value_at_dict(d.keys().value(i) as usize)
            }
            BinCol::Dict8Fixed(d, vals) => {
                if !d.is_valid(i) {
                    return None;
                }
                vals.value_at_dict(d.keys().value(i) as usize)
            }
            BinCol::Dict16Fixed(d, vals) => {
                if !d.is_valid(i) {
                    return None;
                }
                vals.value_at_dict(d.keys().value(i) as usize)
            }
        }
    }
}

/// Small helper trait so `BinCol::at` can share the dictionary value lookup
/// across `BinaryArray` and `FixedSizeBinaryArray`.
trait BinValue {
    fn value_at_dict(&self, k: usize) -> Option<&[u8]>;
}
impl BinValue for arrow::array::BinaryArray {
    fn value_at_dict(&self, k: usize) -> Option<&[u8]> {
        self.is_valid(k).then(|| self.value(k))
    }
}
impl BinValue for arrow::array::FixedSizeBinaryArray {
    fn value_at_dict(&self, k: usize) -> Option<&[u8]> {
        self.is_valid(k).then(|| self.value(k))
    }
}

// ============================ AnyValue columns ============================

/// The seven columns of an OTAP AnyValue (used by attribute tables and the log
/// body struct). Any may be absent.
struct AnyValCols<'a> {
    typ: Option<Prim<'a, UInt8Type>>,
    str: Option<StrCol<'a>>,
    int: Option<Prim<'a, Int64Type>>,
    dbl: Option<Prim<'a, Float64Type>>,
    bol: Option<&'a BooleanArray>,
    byt: Option<BinCol<'a>>,
    ser: Option<BinCol<'a>>,
}

impl<'a> AnyValCols<'a> {
    fn new(lookup: impl Fn(&str) -> Option<&'a ArrayRef>) -> Self {
        Self {
            typ: prim::<UInt8Type>(lookup(consts::ATTRIBUTE_TYPE)),
            str: StrCol::try_new(lookup(consts::ATTRIBUTE_STR)),
            int: prim::<Int64Type>(lookup(consts::ATTRIBUTE_INT)),
            dbl: prim::<Float64Type>(lookup(consts::ATTRIBUTE_DOUBLE)),
            bol: lookup(consts::ATTRIBUTE_BOOL)
                .and_then(|a| a.as_any().downcast_ref::<BooleanArray>()),
            byt: BinCol::try_new(lookup(consts::ATTRIBUTE_BYTES)),
            ser: BinCol::try_new(lookup(consts::ATTRIBUTE_SER)),
        }
    }

    fn typ_at(&self, row: usize) -> u8 {
        self.typ.as_ref().and_then(|a| a.value_at(row)).unwrap_or(0)
    }
}

/// Streams one AnyValue at `row` in the given mode.
struct AnyValSer<'a> {
    cols: &'a AnyValCols<'a>,
    row: usize,
    mode: Mode,
}

impl Serialize for AnyValSer<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.mode {
            Mode::Nested => self.ser_native(s),
            Mode::Otlp => self.ser_otlp(s),
        }
    }
}

impl AnyValSer<'_> {
    /// Native JSON (Nested mode): scalars unwrapped, absent → `null`.
    fn ser_native<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let c = self.cols;
        let row = self.row;
        match AttributeValueType::try_from(c.typ_at(row)) {
            Ok(AttributeValueType::Str) => match c.str.as_ref().and_then(|x| x.at(row)) {
                Some(v) => s.serialize_str(v),
                None => s.serialize_none(),
            },
            Ok(AttributeValueType::Int) => match c.int.as_ref().and_then(|x| x.value_at(row)) {
                Some(v) => s.serialize_i64(v),
                None => s.serialize_none(),
            },
            Ok(AttributeValueType::Double) => match c.dbl.as_ref().and_then(|x| x.value_at(row)) {
                Some(v) => F64Num(v).serialize(s),
                None => s.serialize_none(),
            },
            Ok(AttributeValueType::Bool) => match c.bol.and_then(|x| x.value_at(row)) {
                Some(v) => s.serialize_bool(v),
                None => s.serialize_none(),
            },
            Ok(AttributeValueType::Bytes) => match c.byt.as_ref().and_then(|x| x.at(row)) {
                Some(v) => Base64Val(v).serialize(s),
                None => s.serialize_none(),
            },
            Ok(AttributeValueType::Map) | Ok(AttributeValueType::Slice) => {
                match c.ser.as_ref().and_then(|x| x.at(row)) {
                    Some(bytes) => {
                        match ciborium::from_reader::<ciborium::value::Value, _>(bytes) {
                            Ok(v) => CborNativeSer(&v).serialize(s),
                            Err(_) => s.serialize_none(),
                        }
                    }
                    None => s.serialize_none(),
                }
            }
            _ => s.serialize_none(),
        }
    }

    /// OTLP AnyValue wrapper (Otlp mode): `{stringValue:…}` etc., absent → `{}`.
    fn ser_otlp<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let c = self.cols;
        let row = self.row;
        match AttributeValueType::try_from(c.typ_at(row)) {
            Ok(AttributeValueType::Str) => {
                single(s, "stringValue", c.str.as_ref().and_then(|x| x.at(row)))
            }
            Ok(AttributeValueType::Int) => single(
                s,
                "intValue",
                c.int.as_ref().and_then(|x| x.value_at(row)).map(I64Str),
            ),
            Ok(AttributeValueType::Double) => single(
                s,
                "doubleValue",
                c.dbl.as_ref().and_then(|x| x.value_at(row)).map(F64Num),
            ),
            Ok(AttributeValueType::Bool) => {
                single(s, "boolValue", c.bol.and_then(|x| x.value_at(row)))
            }
            Ok(AttributeValueType::Bytes) => single(
                s,
                "bytesValue",
                c.byt.as_ref().and_then(|x| x.at(row)).map(Base64Val),
            ),
            Ok(AttributeValueType::Map) | Ok(AttributeValueType::Slice) => {
                match c.ser.as_ref().and_then(|x| x.at(row)) {
                    Some(bytes) => {
                        match ciborium::from_reader::<ciborium::value::Value, _>(bytes) {
                            Ok(v) => CborOtlpSer(&v).serialize(s),
                            Err(_) => empty_object(s),
                        }
                    }
                    None => empty_object(s),
                }
            }
            _ => empty_object(s),
        }
    }
}

// ============================ attribute table ============================

#[derive(Clone, Copy)]
enum ParentWidth {
    U16,
    U32,
}

/// Build an index from a batch's `parent_id` column to the rows that carry each
/// value. Used for attribute tables and for child batches (events/links/data
/// points) that link to a parent record by id.
fn build_parent_index(batch: &RecordBatch, width: ParentWidth) -> FxHashMap<u64, Vec<usize>> {
    let mut index: FxHashMap<u64, Vec<usize>> = FxHashMap::default();
    let parent = batch.column_by_name(consts::PARENT_ID);
    match width {
        ParentWidth::U16 => {
            if let Some(acc) = prim::<UInt16Type>(parent) {
                for i in 0..batch.num_rows() {
                    if let Some(p) = acc.value_at(i) {
                        index.entry(p as u64).or_default().push(i);
                    }
                }
            }
        }
        ParentWidth::U32 => {
            if let Some(acc) = prim::<UInt32Type>(parent) {
                for i in 0..batch.num_rows() {
                    if let Some(p) = acc.value_at(i) {
                        index.entry(p as u64).or_default().push(i);
                    }
                }
            }
        }
    }
    index
}

/// A decoded attribute `RecordBatch`: the AnyValue columns, the key column, and
/// an index from parent id to the rows that belong to it.
struct AttrTable<'a> {
    key: Option<StrCol<'a>>,
    vals: AnyValCols<'a>,
    index: FxHashMap<u64, Vec<usize>>,
    /// Taken while a record's nested tree is built and written, put back after.
    pool: Cell<NestedPool<'a>>,
}

impl<'a> AttrTable<'a> {
    fn new(batch: &'a RecordBatch, width: ParentWidth) -> Self {
        Self {
            key: StrCol::try_new(batch.column_by_name(consts::ATTRIBUTE_KEY)),
            vals: AnyValCols::new(|n| batch.column_by_name(n)),
            index: build_parent_index(batch, width),
            pool: Cell::default(),
        }
    }

    fn from_records(
        records: &'a OtapArrowRecords,
        pt: ArrowPayloadType,
        width: ParentWidth,
    ) -> Option<Self> {
        records.get(pt).map(|b| Self::new(b, width))
    }
}

/// Streams an attribute collection for a parent id: an OTLP `[{key,value}]`
/// array (Otlp), or a dotted-key-expanded object (Nested).
struct AttrsSer<'t, 'a> {
    table: Option<&'t AttrTable<'a>>,
    parent_id: u64,
    mode: Mode,
}

impl Serialize for AttrsSer<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.mode {
            Mode::Otlp => {
                let mut seq = s.serialize_seq(None)?;
                if let Some(t) = self.table
                    && let (Some(key_col), Some(rows)) =
                        (t.key.as_ref(), t.index.get(&self.parent_id))
                {
                    for &row in rows {
                        if let Some(key) = key_col.at(row) {
                            seq.serialize_element(&OtlpKv {
                                key,
                                val: AnyValSer {
                                    cols: &t.vals,
                                    row,
                                    mode: Mode::Otlp,
                                },
                            })?;
                        }
                    }
                }
                seq.end()
            }
            Mode::Nested => {
                let Some(t) = self.table else {
                    return empty_object(s);
                };
                let mut pool = t.pool.take();
                let root = NestedNode::Object(build_nested(t, self.parent_id, &mut pool));
                let res = NestedSer {
                    node: &root,
                    cols: &t.vals,
                }
                .serialize(s);
                pool.recycle(root);
                t.pool.set(pool);
                res
            }
        }
    }
}

// ---- nested dotted-key tree (the one inherent allocation) ----
//
// Dotted keys (`a.b.c`) expand to nested objects, numeric segments to arrays.
// This is a genuine tree transform, so we build a lightweight tree — but with
// borrowed `&'a str` keys and lazy column-ref leaves, never `serde_json::Value`.
//
// A numeric segment orders an array's elements, it does not place them (as
// `serde_qs` does): `a.1` and `a.5` make a two-element array. Keys are
// untrusted, so `a.4000000000` must cost one element, not four billion
// padding nulls.

enum NestedNode<'a> {
    /// An unfilled slot of [`NestedArray::dense`], skipped on write.
    Hole,
    /// The attribute row holding the value. The tree borrows only key text,
    /// so its storage can outlive one record, see [`NestedPool`].
    Leaf(usize),
    Object(NestedObject<'a>),
    Array(NestedArray<'a>),
}

/// Writes a [`NestedNode`], reading its leaves from `cols`.
struct NestedSer<'n, 'a> {
    node: &'n NestedNode<'a>,
    cols: &'n AnyValCols<'a>,
}

impl Serialize for NestedSer<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let at = |node| NestedSer {
            node,
            cols: self.cols,
        };
        match self.node {
            NestedNode::Hole => s.serialize_none(),
            &NestedNode::Leaf(row) => AnyValSer {
                cols: self.cols,
                row,
                mode: Mode::Nested,
            }
            .serialize(s),
            NestedNode::Object(obj) => {
                let mut m = s.serialize_map(Some(obj.children.len()))?;
                for (k, v) in &obj.children {
                    m.serialize_entry(k, &at(v))?;
                }
                m.end()
            }
            NestedNode::Array(arr) => {
                let mut seq = s.serialize_seq(Some(arr.len()))?;
                for it in arr.elements() {
                    seq.serialize_element(&at(it))?;
                }
                seq.end()
            }
        }
    }
}

/// Children in insertion order. The keys of one value arrive together, so the
/// last child is checked first. Past [`INDEXED_FROM`] children a miss would
/// scan them all, which makes a record of many distinct keys quadratic, so a
/// hash index takes over.
#[derive(Default)]
struct NestedObject<'a> {
    children: Vec<(&'a str, NestedNode<'a>)>,
    /// Boxed: built for few objects, and every `NestedNode` is as large as
    /// this struct.
    #[allow(clippy::box_collection)]
    index: Option<Box<FxHashMap<&'a str, usize>>>,
}

const INDEXED_FROM: usize = 8;

impl<'a> NestedObject<'a> {
    fn position(&mut self, key: &str, pool: &mut NestedPool<'a>) -> Option<usize> {
        if let Some(index) = &self.index {
            return index.get(key).copied();
        }
        let found = self.children.iter().rposition(|(k, _)| *k == key);
        if found.is_none() && self.children.len() >= INDEXED_FROM {
            let mut index = pool.indexes.pop().unwrap_or_default();
            index.extend(self.children.iter().enumerate().map(|(i, (k, _))| (*k, i)));
            self.index = Some(index);
        }
        found
    }

    fn push(&mut self, key: &'a str, node: NestedNode<'a>) -> &mut NestedNode<'a> {
        if let Some(index) = &mut self.index {
            index.insert(key, self.children.len());
        }
        self.children.push((key, node));
        &mut self.children.last_mut().expect("just pushed").1
    }
}

/// Array elements in index order. An index is a position in `dense` while the
/// gap it opens fits the record's padding budget, its attribute count: a dense
/// array never pads more than that, whatever order its keys come in. The rest
/// go to `sparse`, whose keys all stay at or past `dense.len()`.
#[derive(Default)]
struct NestedArray<'a> {
    dense: Vec<NestedNode<'a>>,
    /// Boxed: rarely used, and every `NestedNode` is as large as this struct.
    #[allow(clippy::box_collection)]
    sparse: Option<Box<BTreeMap<Index<'a>, NestedNode<'a>>>>,
}

impl<'a> NestedArray<'a> {
    fn elements(&self) -> impl Iterator<Item = &NestedNode<'a>> {
        self.dense
            .iter()
            .filter(|n| !matches!(n, NestedNode::Hole))
            .chain(self.sparse.iter().flat_map(|s| s.values()))
    }

    fn len(&self) -> usize {
        self.dense
            .iter()
            .filter(|n| !matches!(n, NestedNode::Hole))
            .count()
            + self.sparse.as_ref().map_or(0, |s| s.len())
    }

    /// The slot for `idx`, a `Hole` if it is new.
    fn slot(&mut self, idx: Index<'a>, budget: &mut u64) -> &mut NestedNode<'a> {
        let len = self.dense.len() as u64;
        if idx.n < len {
            return &mut self.dense[idx.n as usize];
        }
        let pad = idx.n - len;
        let below_sparse = self
            .sparse
            .as_ref()
            .is_none_or(|s| s.first_key_value().is_none_or(|(first, _)| idx < *first));
        if pad <= *budget && below_sparse {
            *budget -= pad;
            // `pad` is within the budget, a row count, so this fits a usize.
            self.dense
                .resize_with(idx.n as usize + 1, || NestedNode::Hole);
            return self.dense.last_mut().expect("just grown");
        }
        self.sparse
            .get_or_insert_default()
            .entry(idx)
            .or_insert(NestedNode::Hole)
    }
}

/// An array index segment: a canonical decimal (digits only, no sign, no
/// leading zero, so `01` stays an object key instead of colliding with `1`),
/// ordered numerically.
#[derive(PartialEq, Eq)]
struct Index<'a> {
    /// The value, or `u64::MAX` past 19 digits, where `text` orders instead.
    n: u64,
    text: &'a str,
}

impl<'a> Index<'a> {
    fn parse(seg: &'a str) -> Option<Self> {
        if seg.is_empty() || (seg.len() > 1 && seg.starts_with('0')) {
            return None;
        }
        // One pass: this runs for every numeric segment of every attribute.
        // Past 19 digits the value wraps, and is then replaced.
        let mut n: u64 = 0;
        for b in seg.bytes() {
            let digit = b.wrapping_sub(b'0');
            if digit > 9 {
                return None;
            }
            n = n.wrapping_mul(10).wrapping_add(u64::from(digit));
        }
        Some(Index {
            n: if seg.len() <= 19 { n } else { u64::MAX },
            text: seg,
        })
    }
}

impl Ord for Index<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.n
            .cmp(&other.n)
            .then_with(|| (self.text.len(), self.text).cmp(&(other.text.len(), other.text)))
    }
}

impl PartialOrd for Index<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The storage of the trees already written. A tree is built and dropped per
/// record, so reusing it saves an allocation and a free per object and array
/// of every record.
#[derive(Default)]
struct NestedPool<'a> {
    children: Vec<Vec<(&'a str, NestedNode<'a>)>>,
    elements: Vec<Vec<NestedNode<'a>>>,
    #[allow(clippy::vec_box)]
    indexes: Vec<Box<FxHashMap<&'a str, usize>>>,
}

impl<'a> NestedPool<'a> {
    fn object(&mut self) -> NestedObject<'a> {
        NestedObject {
            children: self.children.pop().unwrap_or_default(),
            index: None,
        }
    }

    fn array(&mut self) -> NestedArray<'a> {
        NestedArray {
            dense: self.elements.pop().unwrap_or_default(),
            sparse: None,
        }
    }

    /// Takes back the storage of `node` and everything under it, emptied.
    fn recycle(&mut self, node: NestedNode<'a>) {
        match node {
            NestedNode::Object(NestedObject {
                mut children,
                index,
            }) => {
                for (_, child) in children.drain(..) {
                    self.recycle(child);
                }
                self.children.push(children);
                if let Some(mut index) = index {
                    index.clear();
                    self.indexes.push(index);
                }
            }
            NestedNode::Array(NestedArray { mut dense, sparse }) => {
                for element in dense.drain(..) {
                    self.recycle(element);
                }
                self.elements.push(dense);
                for element in sparse.into_iter().flat_map(|s| s.into_values()) {
                    self.recycle(element);
                }
            }
            NestedNode::Hole | NestedNode::Leaf(_) => {}
        }
    }
}

/// What one record's tree is built with.
struct Builder<'p, 'a> {
    /// What the record's arrays may still pad, see [`NestedArray`].
    budget: u64,
    pool: &'p mut NestedPool<'a>,
}

impl<'a> Builder<'_, 'a> {
    /// The empty container a key continuing with segment `next` descends into.
    fn container_for(&mut self, next: &str) -> NestedNode<'a> {
        if Index::parse(next).is_some() {
            NestedNode::Array(self.pool.array())
        } else {
            NestedNode::Object(self.pool.object())
        }
    }
}

fn build_nested<'a>(
    table: &AttrTable<'a>,
    parent_id: u64,
    pool: &mut NestedPool<'a>,
) -> NestedObject<'a> {
    let mut root = pool.object();
    if let (Some(key_col), Some(rows)) = (table.key.as_ref(), table.index.get(&parent_id)) {
        let mut b = Builder {
            budget: rows.len() as u64,
            pool,
        };
        for &row in rows {
            if let Some(key) = key_col.at(row) {
                insert_into_obj(
                    &mut root,
                    &mut key.split('.').peekable(),
                    NestedNode::Leaf(row),
                    &mut b,
                );
            }
        }
    }
    root
}

/// The remaining segments of a dotted key, walked in place rather than
/// collected: this runs for every attribute of every record.
type KeyParts<'a> = std::iter::Peekable<std::str::Split<'a, char>>;

fn insert_into_obj<'a>(
    obj: &mut NestedObject<'a>,
    parts: &mut KeyParts<'a>,
    value: NestedNode<'a>,
    b: &mut Builder<'_, 'a>,
) {
    let Some(head) = parts.next() else {
        return;
    };
    let pos = obj.position(head, b.pool);
    let Some(next) = parts.peek() else {
        match pos {
            Some(i) => obj.children[i].1 = value,
            None => {
                obj.push(head, value);
            }
        }
        return;
    };
    let node = match pos {
        Some(i) => &mut obj.children[i].1,
        None => obj.push(head, b.container_for(next)),
    };
    descend(node, parts, value, b);
}

fn insert_into_arr<'a>(
    arr: &mut NestedArray<'a>,
    parts: &mut KeyParts<'a>,
    value: NestedNode<'a>,
    b: &mut Builder<'_, 'a>,
) {
    let Some(head) = parts.next() else {
        return;
    };
    let Some(idx) = Index::parse(head) else {
        return;
    };
    let slot = arr.slot(idx, &mut b.budget);
    let Some(next) = parts.peek() else {
        *slot = value;
        return;
    };
    if let NestedNode::Hole = slot {
        *slot = b.container_for(next);
    }
    descend(slot, parts, value, b);
}

/// Continues a key below `node`; a key that goes on under a leaf is dropped.
fn descend<'a>(
    node: &mut NestedNode<'a>,
    parts: &mut KeyParts<'a>,
    value: NestedNode<'a>,
    b: &mut Builder<'_, 'a>,
) {
    match node {
        NestedNode::Object(m) => insert_into_obj(m, parts, value, b),
        NestedNode::Array(a) => insert_into_arr(a, parts, value, b),
        NestedNode::Leaf(_) | NestedNode::Hole => {}
    }
}

// ============================ resource / scope ============================

/// Reads the embedded `resource` struct column shared by logs/spans/metrics.
struct ResourceCols<'a> {
    id: Option<Prim<'a, UInt16Type>>,
    dropped: Option<Prim<'a, UInt32Type>>,
    schema_url: Option<StrCol<'a>>,
}

impl<'a> ResourceCols<'a> {
    fn new(batch: &'a RecordBatch) -> Option<Self> {
        let st = batch
            .column_by_name(consts::RESOURCE)?
            .as_any()
            .downcast_ref::<StructArray>()?;
        Some(Self {
            id: prim::<UInt16Type>(st.column_by_name(consts::ID)),
            dropped: prim::<UInt32Type>(st.column_by_name(consts::DROPPED_ATTRIBUTES_COUNT)),
            schema_url: StrCol::try_new(st.column_by_name(consts::SCHEMA_URL)),
        })
    }

    fn id_at(&self, row: usize) -> u64 {
        self.id
            .as_ref()
            .and_then(|a| a.value_at(row))
            .map(|v| v as u64)
            .unwrap_or(0)
    }
}

/// Reads the embedded `scope` struct column shared by logs/spans/metrics.
struct ScopeCols<'a> {
    id: Option<Prim<'a, UInt16Type>>,
    name: Option<StrCol<'a>>,
    version: Option<StrCol<'a>>,
    dropped: Option<Prim<'a, UInt32Type>>,
}

impl<'a> ScopeCols<'a> {
    fn new(batch: &'a RecordBatch) -> Option<Self> {
        let st = batch
            .column_by_name(consts::SCOPE)?
            .as_any()
            .downcast_ref::<StructArray>()?;
        Some(Self {
            id: prim::<UInt16Type>(st.column_by_name(consts::ID)),
            name: StrCol::try_new(st.column_by_name(consts::NAME)),
            version: StrCol::try_new(st.column_by_name(consts::VERSION)),
            dropped: prim::<UInt32Type>(st.column_by_name(consts::DROPPED_ATTRIBUTES_COUNT)),
        })
    }

    fn id_at(&self, row: usize) -> u64 {
        self.id
            .as_ref()
            .and_then(|a| a.value_at(row))
            .map(|v| v as u64)
            .unwrap_or(0)
    }
}

/// Stream the resource object for a record. Always an object (never `null`): a
/// `None` OTLP resource round-trips through OTAP as an empty struct.
fn serialize_resource_obj<S: Serializer>(
    cols: &Option<ResourceCols<'_>>,
    row: usize,
    attrs: &Option<AttrTable<'_>>,
    mode: Mode,
    s: S,
) -> Result<S::Ok, S::Error> {
    let (dropped, attrs_ser) = match cols {
        Some(r) => (
            u32_at(&r.dropped, row),
            AttrsSer {
                table: attrs.as_ref(),
                parent_id: r.id_at(row),
                mode,
            },
        ),
        None => (
            0,
            AttrsSer {
                table: None,
                parent_id: 0,
                mode,
            },
        ),
    };
    let mut m = s.serialize_map(None)?;
    m.serialize_entry("attributes", &attrs_ser)?;
    m.serialize_entry("droppedAttributesCount", &dropped)?;
    m.serialize_entry("entityRefs", &EmptyArr)?;
    m.end()
}

fn resource_schema_url<'a>(cols: &Option<ResourceCols<'a>>, row: usize) -> &'a str {
    cols.as_ref()
        .and_then(|r| r.schema_url.as_ref())
        .and_then(|c| c.at(row))
        .unwrap_or("")
}

/// Stream the scope object for a record.
fn serialize_scope_obj<S: Serializer>(
    cols: &Option<ScopeCols<'_>>,
    row: usize,
    attrs: &Option<AttrTable<'_>>,
    mode: Mode,
    s: S,
) -> Result<S::Ok, S::Error> {
    let (name, version, dropped, attrs_ser) = match cols {
        Some(sc) => (
            str_at(&sc.name, row),
            str_at(&sc.version, row),
            u32_at(&sc.dropped, row),
            AttrsSer {
                table: attrs.as_ref(),
                parent_id: sc.id_at(row),
                mode,
            },
        ),
        None => (
            "",
            "",
            0,
            AttrsSer {
                table: None,
                parent_id: 0,
                mode,
            },
        ),
    };
    let mut m = s.serialize_map(None)?;
    m.serialize_entry("name", name)?;
    m.serialize_entry("version", version)?;
    m.serialize_entry("attributes", &attrs_ser)?;
    m.serialize_entry("droppedAttributesCount", &dropped)?;
    m.end()
}

/// Every per-row input of [`serialize_resource_obj`]: within one payload, rows
/// with equal keys serialize to identical bytes. The id alone should suffice,
/// but keying on everything the object reads keeps the cache from ever
/// changing the output.
type ResourceKey = (u64, u32);

fn resource_key(cols: &Option<ResourceCols<'_>>, row: usize) -> ResourceKey {
    cols.as_ref()
        .map_or((0, 0), |r| (r.id_at(row), u32_at(&r.dropped, row)))
}

/// Every per-row input of [`serialize_scope_obj`]; see [`ResourceKey`].
type ScopeKey<'a> = (u64, &'a str, &'a str, u32);

fn scope_key<'a>(cols: &Option<ScopeCols<'a>>, row: usize) -> ScopeKey<'a> {
    cols.as_ref().map_or((0, "", "", 0), |sc| {
        (
            sc.id_at(row),
            str_at(&sc.name, row),
            str_at(&sc.version, row),
            u32_at(&sc.dropped, row),
        )
    })
}

// ============================ layout drivers ============================

/// A per-signal serialization context: resolves resource/scope/record at a row.
trait Signal {
    fn num_rows(&self) -> usize;
    fn kind(&self) -> &'static str;
    /// `(resourceX, scopeX, recordsX)` e.g. `("resourceLogs","scopeLogs","logRecords")`.
    fn batch_keys(&self) -> (&'static str, &'static str, &'static str);
    fn resource_id(&self, row: usize) -> u64;
    fn scope_id(&self, row: usize) -> u64;
    fn ser_resource<S: Serializer>(&self, row: usize, s: S) -> Result<S::Ok, S::Error>;
    fn resource_schema_url(&self, row: usize) -> &str;
    fn ser_scope<S: Serializer>(&self, row: usize, s: S) -> Result<S::Ok, S::Error>;
    fn scope_schema_url(&self, row: usize) -> &str;
    fn resource_key(&self, row: usize) -> ResourceKey;
    fn scope_key(&self, row: usize) -> ScopeKey<'_>;
    fn ser_record<S: Serializer>(&self, row: usize, s: S) -> Result<S::Ok, S::Error>;
}

// Adapters bridging the trait's `ser_*` methods into serde's value model.
struct ResField<'a, C: Signal>(&'a C, usize);
impl<C: Signal> Serialize for ResField<'_, C> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.ser_resource(self.1, s)
    }
}
struct ScopeField<'a, C: Signal>(&'a C, usize);
impl<C: Signal> Serialize for ScopeField<'_, C> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.ser_scope(self.1, s)
    }
}
struct RecField<'a, C: Signal>(&'a C, usize);
impl<C: Signal> Serialize for RecField<'_, C> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.ser_record(self.1, s)
    }
}

/// The byte range of `v` serialized into `parts`, serialized only the first
/// time `key` is seen.
fn cached_part<K: Eq + std::hash::Hash>(
    seen: &mut FxHashMap<K, Range<usize>>,
    parts: &mut Vec<u8>,
    key: K,
    v: impl Serialize,
) -> serde_json::Result<Range<usize>> {
    if let Some(r) = seen.get(&key) {
        return Ok(r.clone());
    }
    let start = parts.len();
    serde_json::to_writer(&mut *parts, &v)?;
    seen.insert(key, start..parts.len());
    Ok(start..parts.len())
}

/// Denormalized layout: one JSON envelope per record,
/// `{kind, resource, resourceSchemaUrl, scope, scopeSchemaUrl, record}`.
///
/// Records sharing a resource or scope repeat the same object, so each distinct
/// one is serialized once per payload and its bytes copied into every envelope.
/// The envelope is written by hand (compact, same key order) because
/// serde_json cannot splice pre-serialized JSON without `raw_value`.
///
/// All envelopes go into one `Vec<u8>` that is sliced into per-record `Bytes`
/// at the end. serde_json issues a write per token, and `Vec`'s `io::Write`
/// inlines down to a store, where `bytes::buf::Writer<&mut BytesMut>` makes an
/// out-of-line `put_slice` call and a `memcpy` call even for a single byte.
fn emit_denorm<C: Signal>(ctx: &C) -> Result<Vec<Bytes>> {
    let n = ctx.num_rows();
    let mut ends = Vec::with_capacity(n);
    let mut buf = Vec::new();
    let mut parts = Vec::new();
    let mut resources = FxHashMap::default();
    let mut scopes = FxHashMap::default();
    for row in 0..n {
        let resource = cached_part(
            &mut resources,
            &mut parts,
            ctx.resource_key(row),
            ResField(ctx, row),
        )?;
        let scope = cached_part(
            &mut scopes,
            &mut parts,
            ctx.scope_key(row),
            ScopeField(ctx, row),
        )?;
        buf.extend_from_slice(b"{\"kind\":");
        serde_json::to_writer(&mut buf, ctx.kind())?;
        buf.extend_from_slice(b",\"resource\":");
        buf.extend_from_slice(&parts[resource]);
        buf.extend_from_slice(b",\"resourceSchemaUrl\":");
        serde_json::to_writer(&mut buf, ctx.resource_schema_url(row))?;
        buf.extend_from_slice(b",\"scope\":");
        buf.extend_from_slice(&parts[scope]);
        buf.extend_from_slice(b",\"scopeSchemaUrl\":");
        serde_json::to_writer(&mut buf, ctx.scope_schema_url(row))?;
        buf.extend_from_slice(b",\"record\":");
        serde_json::to_writer(&mut buf, &RecField(ctx, row))?;
        buf.push(b'}');
        if row == 0 {
            // Records of one payload are close in size, so the first one sizes
            // the buffer (plus an eighth of slack) and growth rarely copies it.
            buf.reserve((buf.len() + buf.len() / 8) * (n - 1));
        }
        ends.push(buf.len());
    }
    let all = Bytes::from(buf);
    let mut start = 0;
    Ok(ends
        .into_iter()
        .map(|end| {
            let b = all.slice(start..end);
            start = end;
            b
        })
        .collect())
}

// Batch layout: group rows by resource id, then scope id (first-appearance order).
struct ScopeGroup {
    first_row: usize,
    rows: Vec<usize>,
}
struct ResGroup {
    first_row: usize,
    scopes: Vec<ScopeGroup>,
}

fn group_rows<C: Signal>(ctx: &C) -> Vec<ResGroup> {
    let mut resources: Vec<ResGroup> = Vec::new();
    let mut res_pos: FxHashMap<u64, usize> = FxHashMap::default();
    let mut scope_pos: Vec<FxHashMap<u64, usize>> = Vec::new();
    for row in 0..ctx.num_rows() {
        let rid = ctx.resource_id(row);
        let sid = ctx.scope_id(row);
        let ri = *res_pos.entry(rid).or_insert_with(|| {
            resources.push(ResGroup {
                first_row: row,
                scopes: Vec::new(),
            });
            scope_pos.push(FxHashMap::default());
            resources.len() - 1
        });
        let spos = &mut scope_pos[ri];
        match spos.get(&sid) {
            Some(&si) => resources[ri].scopes[si].rows.push(row),
            None => {
                let si = resources[ri].scopes.len();
                resources[ri].scopes.push(ScopeGroup {
                    first_row: row,
                    rows: vec![row],
                });
                spos.insert(sid, si);
            }
        }
    }
    resources
}

struct BatchSer<'a, C: Signal> {
    ctx: &'a C,
    groups: Vec<ResGroup>,
}
impl<C: Signal> Serialize for BatchSer<'_, C> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let (res_key, _, _) = self.ctx.batch_keys();
        let mut m = s.serialize_map(None)?;
        m.serialize_entry(
            res_key,
            &ResGroupsSeq {
                ctx: self.ctx,
                groups: &self.groups,
            },
        )?;
        m.end()
    }
}

struct ResGroupsSeq<'a, C: Signal> {
    ctx: &'a C,
    groups: &'a [ResGroup],
}
impl<C: Signal> Serialize for ResGroupsSeq<'_, C> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.groups.len()))?;
        for g in self.groups {
            seq.serialize_element(&ResGroupSer { ctx: self.ctx, g })?;
        }
        seq.end()
    }
}

struct ResGroupSer<'a, C: Signal> {
    ctx: &'a C,
    g: &'a ResGroup,
}
impl<C: Signal> Serialize for ResGroupSer<'_, C> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let (_, scope_key, _) = self.ctx.batch_keys();
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("resource", &ResField(self.ctx, self.g.first_row))?;
        m.serialize_entry(
            scope_key,
            &ScopeGroupsSeq {
                ctx: self.ctx,
                scopes: &self.g.scopes,
            },
        )?;
        m.serialize_entry("schemaUrl", self.ctx.resource_schema_url(self.g.first_row))?;
        m.end()
    }
}

struct ScopeGroupsSeq<'a, C: Signal> {
    ctx: &'a C,
    scopes: &'a [ScopeGroup],
}
impl<C: Signal> Serialize for ScopeGroupsSeq<'_, C> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.scopes.len()))?;
        for sg in self.scopes {
            seq.serialize_element(&ScopeGroupSer { ctx: self.ctx, sg })?;
        }
        seq.end()
    }
}

struct ScopeGroupSer<'a, C: Signal> {
    ctx: &'a C,
    sg: &'a ScopeGroup,
}
impl<C: Signal> Serialize for ScopeGroupSer<'_, C> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let (_, _, records_key) = self.ctx.batch_keys();
        let mut m = s.serialize_map(None)?;
        m.serialize_entry(
            records_key,
            &RecordsSeq {
                ctx: self.ctx,
                rows: &self.sg.rows,
            },
        )?;
        m.serialize_entry("schemaUrl", self.ctx.scope_schema_url(self.sg.first_row))?;
        m.serialize_entry("scope", &ScopeField(self.ctx, self.sg.first_row))?;
        m.end()
    }
}

struct RecordsSeq<'a, C: Signal> {
    ctx: &'a C,
    rows: &'a [usize],
}
impl<C: Signal> Serialize for RecordsSeq<'_, C> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.rows.len()))?;
        for &r in self.rows {
            seq.serialize_element(&RecField(self.ctx, r))?;
        }
        seq.end()
    }
}

fn emit_batch<C: Signal>(ctx: &C) -> Result<Vec<Bytes>> {
    let groups = group_rows(ctx);
    let mut buf = Vec::with_capacity(4096);
    serde_json::to_writer(&mut buf, &BatchSer { ctx, groups })?;
    Ok(vec![Bytes::from(buf)])
}

fn emit<C: Signal>(ctx: &C, layout: Layout) -> Result<Vec<Bytes>> {
    match layout {
        Layout::Denorm => emit_denorm(ctx),
        Layout::Batch => emit_batch(ctx),
    }
}

// ============================ logs ============================

struct LogsCtx<'a> {
    num_rows: usize,
    mode: Mode,
    resource: Option<ResourceCols<'a>>,
    scope: Option<ScopeCols<'a>>,
    resource_attrs: Option<AttrTable<'a>>,
    scope_attrs: Option<AttrTable<'a>>,
    log_attrs: Option<AttrTable<'a>>,
    id: Option<Prim<'a, UInt16Type>>,
    scope_schema_url: Option<StrCol<'a>>,
    time: Option<Prim<'a, TimestampNanosecondType>>,
    observed: Option<Prim<'a, TimestampNanosecondType>>,
    severity_number: Option<Prim<'a, Int32Type>>,
    severity_text: Option<StrCol<'a>>,
    trace_id: Option<BinCol<'a>>,
    span_id: Option<BinCol<'a>>,
    flags: Option<Prim<'a, UInt32Type>>,
    dropped: Option<Prim<'a, UInt32Type>>,
    event_name: Option<StrCol<'a>>,
    body: Option<(&'a StructArray, AnyValCols<'a>)>,
}

impl Signal for LogsCtx<'_> {
    fn num_rows(&self) -> usize {
        self.num_rows
    }
    fn kind(&self) -> &'static str {
        "logs"
    }
    fn batch_keys(&self) -> (&'static str, &'static str, &'static str) {
        ("resourceLogs", "scopeLogs", "logRecords")
    }
    fn resource_id(&self, row: usize) -> u64 {
        self.resource.as_ref().map(|r| r.id_at(row)).unwrap_or(0)
    }
    fn scope_id(&self, row: usize) -> u64 {
        self.scope.as_ref().map(|s| s.id_at(row)).unwrap_or(0)
    }
    fn ser_resource<S: Serializer>(&self, row: usize, s: S) -> Result<S::Ok, S::Error> {
        serialize_resource_obj(&self.resource, row, &self.resource_attrs, self.mode, s)
    }
    fn resource_schema_url(&self, row: usize) -> &str {
        resource_schema_url(&self.resource, row)
    }
    fn ser_scope<S: Serializer>(&self, row: usize, s: S) -> Result<S::Ok, S::Error> {
        serialize_scope_obj(&self.scope, row, &self.scope_attrs, self.mode, s)
    }
    fn resource_key(&self, row: usize) -> ResourceKey {
        resource_key(&self.resource, row)
    }
    fn scope_key(&self, row: usize) -> ScopeKey<'_> {
        scope_key(&self.scope, row)
    }
    fn scope_schema_url(&self, row: usize) -> &str {
        str_at(&self.scope_schema_url, row)
    }
    fn ser_record<S: Serializer>(&self, row: usize, s: S) -> Result<S::Ok, S::Error> {
        let log_id = self
            .id
            .as_ref()
            .and_then(|a| a.value_at(row))
            .map(|v| v as u64)
            .unwrap_or(row as u64);
        let mut m = s.serialize_map(None)?;
        m.serialize_entry(
            "attributes",
            &AttrsSer {
                table: self.log_attrs.as_ref(),
                parent_id: log_id,
                mode: self.mode,
            },
        )?;
        match &self.body {
            Some((st, cols)) if st.is_valid(row) => m.serialize_entry(
                "body",
                &AnyValSer {
                    cols,
                    row,
                    mode: self.mode,
                },
            )?,
            _ => m.serialize_entry("body", &None::<()>)?,
        }
        m.serialize_entry("droppedAttributesCount", &u32_at(&self.dropped, row))?;
        m.serialize_entry("eventName", str_at(&self.event_name, row))?;
        m.serialize_entry("flags", &u32_at(&self.flags, row))?;
        m.serialize_entry("observedTimeUnixNano", &U64Str(ts_at(&self.observed, row)))?;
        m.serialize_entry("severityNumber", &i32_at(&self.severity_number, row))?;
        m.serialize_entry("severityText", str_at(&self.severity_text, row))?;
        m.serialize_entry("spanId", &HexStr(bin_at(&self.span_id, row)))?;
        m.serialize_entry("timeUnixNano", &U64Str(ts_at(&self.time, row)))?;
        m.serialize_entry("traceId", &HexStr(bin_at(&self.trace_id, row)))?;
        m.end()
    }
}

fn logs(records: &OtapArrowRecords, layout: Layout, mode: Mode) -> Result<Vec<Bytes>> {
    let batch = records
        .get(ArrowPayloadType::Logs)
        .context("logs payload missing Logs batch")?;
    let body = batch
        .column_by_name(consts::BODY)
        .and_then(|a| a.as_any().downcast_ref::<StructArray>())
        .map(|st| (st, AnyValCols::new(|n| st.column_by_name(n))));
    let ctx = LogsCtx {
        num_rows: batch.num_rows(),
        mode,
        resource: ResourceCols::new(batch),
        scope: ScopeCols::new(batch),
        resource_attrs: AttrTable::from_records(
            records,
            ArrowPayloadType::ResourceAttrs,
            ParentWidth::U16,
        ),
        scope_attrs: AttrTable::from_records(
            records,
            ArrowPayloadType::ScopeAttrs,
            ParentWidth::U16,
        ),
        log_attrs: AttrTable::from_records(records, ArrowPayloadType::LogAttrs, ParentWidth::U16),
        id: prim::<UInt16Type>(batch.column_by_name(consts::ID)),
        scope_schema_url: StrCol::try_new(batch.column_by_name(consts::SCHEMA_URL)),
        time: ts_col(batch.column_by_name(consts::TIME_UNIX_NANO)),
        observed: ts_col(batch.column_by_name(consts::OBSERVED_TIME_UNIX_NANO)),
        severity_number: prim::<Int32Type>(batch.column_by_name(consts::SEVERITY_NUMBER)),
        severity_text: StrCol::try_new(batch.column_by_name(consts::SEVERITY_TEXT)),
        trace_id: BinCol::try_new(batch.column_by_name(consts::TRACE_ID)),
        span_id: BinCol::try_new(batch.column_by_name(consts::SPAN_ID)),
        flags: prim::<UInt32Type>(batch.column_by_name(consts::FLAGS)),
        dropped: prim::<UInt32Type>(batch.column_by_name(consts::DROPPED_ATTRIBUTES_COUNT)),
        event_name: StrCol::try_new(batch.column_by_name(consts::EVENT_NAME)),
        body,
    };
    emit(&ctx, layout)
}

// ============================ traces ============================

/// Columns of the `SpanEvents` batch.
struct EventCols<'a> {
    id: Option<Prim<'a, UInt32Type>>,
    time: Option<Prim<'a, TimestampNanosecondType>>,
    name: Option<StrCol<'a>>,
    dropped: Option<Prim<'a, UInt32Type>>,
}

impl<'a> EventCols<'a> {
    fn new(batch: &'a RecordBatch) -> Self {
        Self {
            id: prim::<UInt32Type>(batch.column_by_name(consts::ID)),
            time: ts_col(batch.column_by_name(consts::TIME_UNIX_NANO)),
            name: StrCol::try_new(batch.column_by_name(consts::NAME)),
            dropped: prim::<UInt32Type>(batch.column_by_name(consts::DROPPED_ATTRIBUTES_COUNT)),
        }
    }
}

struct EventSer<'t, 'a> {
    cols: &'t EventCols<'a>,
    row: usize,
    attrs: &'t Option<AttrTable<'a>>,
    mode: Mode,
}
impl Serialize for EventSer<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let event_id = self
            .cols
            .id
            .as_ref()
            .and_then(|a| a.value_at(self.row))
            .map(|v| v as u64)
            .unwrap_or(self.row as u64);
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("timeUnixNano", &U64Str(ts_at(&self.cols.time, self.row)))?;
        m.serialize_entry("name", str_at(&self.cols.name, self.row))?;
        m.serialize_entry(
            "attributes",
            &AttrsSer {
                table: self.attrs.as_ref(),
                parent_id: event_id,
                mode: self.mode,
            },
        )?;
        m.serialize_entry(
            "droppedAttributesCount",
            &u32_at(&self.cols.dropped, self.row),
        )?;
        m.end()
    }
}

/// Columns of the `SpanLinks` batch.
struct LinkCols<'a> {
    id: Option<Prim<'a, UInt32Type>>,
    trace_id: Option<BinCol<'a>>,
    span_id: Option<BinCol<'a>>,
    trace_state: Option<StrCol<'a>>,
    flags: Option<Prim<'a, UInt32Type>>,
    dropped: Option<Prim<'a, UInt32Type>>,
}

impl<'a> LinkCols<'a> {
    fn new(batch: &'a RecordBatch) -> Self {
        Self {
            id: prim::<UInt32Type>(batch.column_by_name(consts::ID)),
            trace_id: BinCol::try_new(batch.column_by_name(consts::TRACE_ID)),
            span_id: BinCol::try_new(batch.column_by_name(consts::SPAN_ID)),
            trace_state: StrCol::try_new(batch.column_by_name(consts::TRACE_STATE)),
            flags: prim::<UInt32Type>(batch.column_by_name(consts::FLAGS)),
            dropped: prim::<UInt32Type>(batch.column_by_name(consts::DROPPED_ATTRIBUTES_COUNT)),
        }
    }
}

struct LinkSer<'t, 'a> {
    cols: &'t LinkCols<'a>,
    row: usize,
    attrs: &'t Option<AttrTable<'a>>,
    mode: Mode,
}
impl Serialize for LinkSer<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let link_id = self
            .cols
            .id
            .as_ref()
            .and_then(|a| a.value_at(self.row))
            .map(|v| v as u64)
            .unwrap_or(self.row as u64);
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("traceId", &HexStr(bin_at(&self.cols.trace_id, self.row)))?;
        m.serialize_entry("spanId", &HexStr(bin_at(&self.cols.span_id, self.row)))?;
        m.serialize_entry("traceState", str_at(&self.cols.trace_state, self.row))?;
        m.serialize_entry(
            "attributes",
            &AttrsSer {
                table: self.attrs.as_ref(),
                parent_id: link_id,
                mode: self.mode,
            },
        )?;
        m.serialize_entry(
            "droppedAttributesCount",
            &u32_at(&self.cols.dropped, self.row),
        )?;
        m.serialize_entry("flags", &u32_at(&self.cols.flags, self.row))?;
        m.end()
    }
}

/// Columns of the span `status` struct.
struct StatusCols<'a> {
    st: &'a StructArray,
    code: Option<Prim<'a, Int32Type>>,
    message: Option<StrCol<'a>>,
}

impl<'a> StatusCols<'a> {
    fn new(batch: &'a RecordBatch) -> Option<Self> {
        let st = batch
            .column_by_name(consts::STATUS)?
            .as_any()
            .downcast_ref::<StructArray>()?;
        Some(Self {
            st,
            code: prim::<Int32Type>(st.column_by_name(consts::STATUS_CODE)),
            message: StrCol::try_new(st.column_by_name(consts::STATUS_MESSAGE)),
        })
    }

    /// The `status` field is present (a non-null struct) at `row`.
    fn present(&self, row: usize) -> bool {
        self.st.is_valid(row)
    }
}

struct StatusSer<'a> {
    cols: &'a StatusCols<'a>,
    row: usize,
}
impl Serialize for StatusSer<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("message", str_at(&self.cols.message, self.row))?;
        m.serialize_entry("code", &i32_at(&self.cols.code, self.row))?;
        m.end()
    }
}

/// Seq of child rows (events or links) linked to a span by parent id.
struct EventsSeq<'t, 'a> {
    cols: &'t Option<EventCols<'a>>,
    idx: &'t Option<FxHashMap<u64, Vec<usize>>>,
    parent_id: u64,
    attrs: &'t Option<AttrTable<'a>>,
    mode: Mode,
}
impl Serialize for EventsSeq<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(None)?;
        if let Some(idx) = self.idx.as_ref()
            && let Some(rows) = idx.get(&self.parent_id)
        {
            for &row in rows {
                match self.cols.as_ref() {
                    Some(cols) => seq.serialize_element(&EventSer {
                        cols,
                        row,
                        attrs: self.attrs,
                        mode: self.mode,
                    })?,
                    None => seq.serialize_element(&None::<()>)?,
                }
            }
        }
        seq.end()
    }
}

struct LinksSeq<'t, 'a> {
    cols: &'t Option<LinkCols<'a>>,
    idx: &'t Option<FxHashMap<u64, Vec<usize>>>,
    parent_id: u64,
    attrs: &'t Option<AttrTable<'a>>,
    mode: Mode,
}
impl Serialize for LinksSeq<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(None)?;
        if let Some(idx) = self.idx.as_ref()
            && let Some(rows) = idx.get(&self.parent_id)
        {
            for &row in rows {
                match self.cols.as_ref() {
                    Some(cols) => seq.serialize_element(&LinkSer {
                        cols,
                        row,
                        attrs: self.attrs,
                        mode: self.mode,
                    })?,
                    None => seq.serialize_element(&None::<()>)?,
                }
            }
        }
        seq.end()
    }
}

struct SpansCtx<'a> {
    num_rows: usize,
    mode: Mode,
    resource: Option<ResourceCols<'a>>,
    scope: Option<ScopeCols<'a>>,
    status: Option<StatusCols<'a>>,
    resource_attrs: Option<AttrTable<'a>>,
    scope_attrs: Option<AttrTable<'a>>,
    span_attrs: Option<AttrTable<'a>>,
    events_cols: Option<EventCols<'a>>,
    events_idx: Option<FxHashMap<u64, Vec<usize>>>,
    event_attrs: Option<AttrTable<'a>>,
    links_cols: Option<LinkCols<'a>>,
    links_idx: Option<FxHashMap<u64, Vec<usize>>>,
    link_attrs: Option<AttrTable<'a>>,
    id: Option<Prim<'a, UInt16Type>>,
    scope_schema_url: Option<StrCol<'a>>,
    trace_id: Option<BinCol<'a>>,
    span_id: Option<BinCol<'a>>,
    parent_span_id: Option<BinCol<'a>>,
    trace_state: Option<StrCol<'a>>,
    name: Option<StrCol<'a>>,
    kind: Option<Prim<'a, Int32Type>>,
    start: Option<Prim<'a, TimestampNanosecondType>>,
    duration: Option<Prim<'a, DurationNanosecondType>>,
    dropped: Option<Prim<'a, UInt32Type>>,
    flags: Option<Prim<'a, UInt32Type>>,
    dropped_events: Option<Prim<'a, UInt32Type>>,
    dropped_links: Option<Prim<'a, UInt32Type>>,
}

impl SpansCtx<'_> {
    fn span_pid(&self, row: usize) -> u64 {
        self.id
            .as_ref()
            .and_then(|a| a.value_at(row))
            .map(|v| v as u64)
            .unwrap_or(row as u64)
    }
}

impl Signal for SpansCtx<'_> {
    fn num_rows(&self) -> usize {
        self.num_rows
    }
    fn kind(&self) -> &'static str {
        "traces"
    }
    fn batch_keys(&self) -> (&'static str, &'static str, &'static str) {
        ("resourceSpans", "scopeSpans", "spans")
    }
    fn resource_id(&self, row: usize) -> u64 {
        self.resource.as_ref().map(|r| r.id_at(row)).unwrap_or(0)
    }
    fn scope_id(&self, row: usize) -> u64 {
        self.scope.as_ref().map(|s| s.id_at(row)).unwrap_or(0)
    }
    fn ser_resource<S: Serializer>(&self, row: usize, s: S) -> Result<S::Ok, S::Error> {
        serialize_resource_obj(&self.resource, row, &self.resource_attrs, self.mode, s)
    }
    fn resource_schema_url(&self, row: usize) -> &str {
        resource_schema_url(&self.resource, row)
    }
    fn ser_scope<S: Serializer>(&self, row: usize, s: S) -> Result<S::Ok, S::Error> {
        serialize_scope_obj(&self.scope, row, &self.scope_attrs, self.mode, s)
    }
    fn resource_key(&self, row: usize) -> ResourceKey {
        resource_key(&self.resource, row)
    }
    fn scope_key(&self, row: usize) -> ScopeKey<'_> {
        scope_key(&self.scope, row)
    }
    fn scope_schema_url(&self, row: usize) -> &str {
        str_at(&self.scope_schema_url, row)
    }
    fn ser_record<S: Serializer>(&self, row: usize, s: S) -> Result<S::Ok, S::Error> {
        let pid = self.span_pid(row);
        let start_ns = ts_at(&self.start, row);
        let dur_ns = self
            .duration
            .as_ref()
            .and_then(|a| a.value_at(row))
            .unwrap_or(0) as u64;
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("traceId", &HexStr(bin_at(&self.trace_id, row)))?;
        m.serialize_entry("spanId", &HexStr(bin_at(&self.span_id, row)))?;
        m.serialize_entry("traceState", str_at(&self.trace_state, row))?;
        m.serialize_entry("parentSpanId", &HexStr(bin_at(&self.parent_span_id, row)))?;
        m.serialize_entry("name", str_at(&self.name, row))?;
        m.serialize_entry("kind", &i32_at(&self.kind, row))?;
        m.serialize_entry("startTimeUnixNano", &U64Str(start_ns))?;
        m.serialize_entry("endTimeUnixNano", &U64Str(start_ns.wrapping_add(dur_ns)))?;
        m.serialize_entry(
            "attributes",
            &AttrsSer {
                table: self.span_attrs.as_ref(),
                parent_id: pid,
                mode: self.mode,
            },
        )?;
        m.serialize_entry("droppedAttributesCount", &u32_at(&self.dropped, row))?;
        m.serialize_entry(
            "events",
            &EventsSeq {
                cols: &self.events_cols,
                idx: &self.events_idx,
                parent_id: pid,
                attrs: &self.event_attrs,
                mode: self.mode,
            },
        )?;
        m.serialize_entry("droppedEventsCount", &u32_at(&self.dropped_events, row))?;
        m.serialize_entry(
            "links",
            &LinksSeq {
                cols: &self.links_cols,
                idx: &self.links_idx,
                parent_id: pid,
                attrs: &self.link_attrs,
                mode: self.mode,
            },
        )?;
        m.serialize_entry("droppedLinksCount", &u32_at(&self.dropped_links, row))?;
        m.serialize_entry("flags", &u32_at(&self.flags, row))?;
        if let Some(st) = self.status.as_ref()
            && st.present(row)
        {
            m.serialize_entry("status", &StatusSer { cols: st, row })?;
        }
        m.end()
    }
}

fn traces(records: &OtapArrowRecords, layout: Layout, mode: Mode) -> Result<Vec<Bytes>> {
    let batch = records
        .get(ArrowPayloadType::Spans)
        .context("traces payload missing Spans batch")?;

    let events_batch = records.get(ArrowPayloadType::SpanEvents);
    let links_batch = records.get(ArrowPayloadType::SpanLinks);

    let ctx = SpansCtx {
        num_rows: batch.num_rows(),
        mode,
        resource: ResourceCols::new(batch),
        scope: ScopeCols::new(batch),
        status: StatusCols::new(batch),
        resource_attrs: AttrTable::from_records(
            records,
            ArrowPayloadType::ResourceAttrs,
            ParentWidth::U16,
        ),
        scope_attrs: AttrTable::from_records(
            records,
            ArrowPayloadType::ScopeAttrs,
            ParentWidth::U16,
        ),
        span_attrs: AttrTable::from_records(records, ArrowPayloadType::SpanAttrs, ParentWidth::U16),
        events_cols: events_batch.map(EventCols::new),
        events_idx: events_batch.map(|b| build_parent_index(b, ParentWidth::U16)),
        event_attrs: AttrTable::from_records(
            records,
            ArrowPayloadType::SpanEventAttrs,
            ParentWidth::U32,
        ),
        links_cols: links_batch.map(LinkCols::new),
        links_idx: links_batch.map(|b| build_parent_index(b, ParentWidth::U16)),
        link_attrs: AttrTable::from_records(
            records,
            ArrowPayloadType::SpanLinkAttrs,
            ParentWidth::U32,
        ),
        id: prim::<UInt16Type>(batch.column_by_name(consts::ID)),
        scope_schema_url: StrCol::try_new(batch.column_by_name(consts::SCHEMA_URL)),
        trace_id: BinCol::try_new(batch.column_by_name(consts::TRACE_ID)),
        span_id: BinCol::try_new(batch.column_by_name(consts::SPAN_ID)),
        parent_span_id: BinCol::try_new(batch.column_by_name(consts::PARENT_SPAN_ID)),
        trace_state: StrCol::try_new(batch.column_by_name(consts::TRACE_STATE)),
        name: StrCol::try_new(batch.column_by_name(consts::NAME)),
        kind: prim::<Int32Type>(batch.column_by_name(consts::KIND)),
        start: ts_col(batch.column_by_name(consts::START_TIME_UNIX_NANO)),
        duration: prim::<DurationNanosecondType>(
            batch.column_by_name(consts::DURATION_TIME_UNIX_NANO),
        ),
        dropped: prim::<UInt32Type>(batch.column_by_name(consts::DROPPED_ATTRIBUTES_COUNT)),
        flags: prim::<UInt32Type>(batch.column_by_name(consts::FLAGS)),
        dropped_events: prim::<UInt32Type>(batch.column_by_name(consts::DROPPED_EVENTS_COUNT)),
        dropped_links: prim::<UInt32Type>(batch.column_by_name(consts::DROPPED_LINKS_COUNT)),
    };
    emit(&ctx, layout)
}

// ============================ metrics ============================

fn bool_col(arr: Option<&ArrayRef>) -> Option<&BooleanArray> {
    arr?.as_any().downcast_ref::<BooleanArray>()
}

/// Read the sub-list of a `List<...>` column at `row`, if present and non-null.
fn list_value(col: Option<&ArrayRef>, row: usize) -> Option<ArrayRef> {
    let la = col?.as_any().downcast_ref::<ListArray>()?;
    la.is_valid(row).then(|| la.value(row))
}

/// `List<UInt64>` at `row` → array of decimal strings (null for invalid slots).
struct U64ListStr<'a>(Option<&'a ArrayRef>, usize);
impl Serialize for U64ListStr<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(None)?;
        if let Some(sub) = list_value(self.0, self.1)
            && let Some(a) = sub.as_any().downcast_ref::<UInt64Array>()
        {
            for i in 0..a.len() {
                if a.is_valid(i) {
                    seq.serialize_element(&U64Str(a.value(i)))?;
                } else {
                    seq.serialize_element(&None::<()>)?;
                }
            }
        }
        seq.end()
    }
}

/// `List<Float64>` at `row` → array of numbers (null for invalid slots).
struct F64ListNum<'a>(Option<&'a ArrayRef>, usize);
impl Serialize for F64ListNum<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(None)?;
        if let Some(sub) = list_value(self.0, self.1)
            && let Some(a) = sub.as_any().downcast_ref::<Float64Array>()
        {
            for i in 0..a.len() {
                if a.is_valid(i) {
                    seq.serialize_element(&F64Num(a.value(i)))?;
                } else {
                    seq.serialize_element(&None::<()>)?;
                }
            }
        }
        seq.end()
    }
}

/// `List<Struct{quantile, value}>` at `row` → array of quantile objects.
struct QuantileSeq<'a>(Option<&'a ArrayRef>, usize);
impl Serialize for QuantileSeq<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(None)?;
        if let Some(sub) = list_value(self.0, self.1)
            && let Some(st) = sub.as_any().downcast_ref::<StructArray>()
        {
            let q = prim::<Float64Type>(st.column_by_name(consts::SUMMARY_QUANTILE));
            let v = prim::<Float64Type>(st.column_by_name(consts::SUMMARY_VALUE));
            for i in 0..st.len() {
                seq.serialize_element(&QuantileObj {
                    quantile: q.as_ref().and_then(|a| a.value_at(i)),
                    value: v.as_ref().and_then(|a| a.value_at(i)),
                })?;
            }
        }
        seq.end()
    }
}

struct QuantileObj {
    quantile: Option<f64>,
    value: Option<f64>,
}
impl Serialize for QuantileObj {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("quantile", &OptF64Num(self.quantile))?;
        m.serialize_entry("value", &OptF64Num(self.value))?;
        m.end()
    }
}

/// Exponential-histogram `positive`/`negative` `Struct{offset, bucket_counts}`.
struct ExpBucketsObj<'a>(Option<&'a ArrayRef>, usize);
impl Serialize for ExpBucketsObj<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let st = self
            .0
            .and_then(|c| c.as_any().downcast_ref::<StructArray>());
        let offset = st
            .and_then(|s| prim::<Int32Type>(s.column_by_name(consts::EXP_HISTOGRAM_OFFSET)))
            .and_then(|a| a.value_at(self.1))
            .unwrap_or(0);
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("offset", &offset)?;
        m.serialize_entry(
            "bucketCounts",
            &U64ListStr(
                st.and_then(|s| s.column_by_name(consts::EXP_HISTOGRAM_BUCKET_COUNTS)),
                self.1,
            ),
        )?;
        m.end()
    }
}

/// Emit the `asDouble`/`asInt` value (double takes precedence; nothing if both
/// null). `asInt` is a JSON number, matching `opentelemetry-proto`.
fn ser_number_value<M: SerializeMap>(
    m: &mut M,
    int: &Option<Prim<'_, Int64Type>>,
    double: &Option<Prim<'_, Float64Type>>,
    row: usize,
) -> Result<(), M::Error> {
    if let Some(d) = double.as_ref().and_then(|a| a.value_at(row)) {
        m.serialize_entry("asDouble", &F64Num(d))?;
    } else if let Some(i) = int.as_ref().and_then(|a| a.value_at(row)) {
        m.serialize_entry("asInt", &i)?;
    }
    Ok(())
}

/// Shared exemplar table for a data-point kind (NumberDpExemplars etc.).
struct Exemplars<'a> {
    id: Option<Prim<'a, UInt32Type>>,
    time: Option<Prim<'a, TimestampNanosecondType>>,
    int: Option<Prim<'a, Int64Type>>,
    double: Option<Prim<'a, Float64Type>>,
    trace_id: Option<BinCol<'a>>,
    span_id: Option<BinCol<'a>>,
    index: FxHashMap<u64, Vec<usize>>,
    attrs: Option<AttrTable<'a>>,
}

impl<'a> Exemplars<'a> {
    fn new(
        records: &'a OtapArrowRecords,
        ex_pt: ArrowPayloadType,
        attr_pt: ArrowPayloadType,
    ) -> Option<Self> {
        let b = records.get(ex_pt)?;
        Some(Self {
            id: prim::<UInt32Type>(b.column_by_name(consts::ID)),
            time: ts_col(b.column_by_name(consts::TIME_UNIX_NANO)),
            int: prim::<Int64Type>(b.column_by_name(consts::INT_VALUE)),
            double: prim::<Float64Type>(b.column_by_name(consts::DOUBLE_VALUE)),
            trace_id: BinCol::try_new(b.column_by_name(consts::TRACE_ID)),
            span_id: BinCol::try_new(b.column_by_name(consts::SPAN_ID)),
            index: build_parent_index(b, ParentWidth::U32),
            attrs: AttrTable::from_records(records, attr_pt, ParentWidth::U32),
        })
    }
}

struct ExemplarSer<'t, 'a> {
    ex: &'t Exemplars<'a>,
    row: usize,
    mode: Mode,
}
impl Serialize for ExemplarSer<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let ex_id = self
            .ex
            .id
            .as_ref()
            .and_then(|a| a.value_at(self.row))
            .map(|v| v as u64)
            .unwrap_or(self.row as u64);
        let mut m = s.serialize_map(None)?;
        m.serialize_entry(
            "filteredAttributes",
            &AttrsSer {
                table: self.ex.attrs.as_ref(),
                parent_id: ex_id,
                mode: self.mode,
            },
        )?;
        m.serialize_entry("timeUnixNano", &U64Str(ts_at(&self.ex.time, self.row)))?;
        ser_number_value(&mut m, &self.ex.int, &self.ex.double, self.row)?;
        m.serialize_entry("spanId", &HexStr(bin_at(&self.ex.span_id, self.row)))?;
        m.serialize_entry("traceId", &HexStr(bin_at(&self.ex.trace_id, self.row)))?;
        m.end()
    }
}

struct ExemplarsSeq<'t, 'a> {
    ex: &'t Option<Exemplars<'a>>,
    dp_id: u64,
    mode: Mode,
}
impl Serialize for ExemplarsSeq<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(None)?;
        if let Some(e) = self.ex.as_ref()
            && let Some(rows) = e.index.get(&self.dp_id)
        {
            for &row in rows {
                seq.serialize_element(&ExemplarSer {
                    ex: e,
                    row,
                    mode: self.mode,
                })?;
            }
        }
        seq.end()
    }
}

/// Common per-data-point fields: id resolution, attributes, start/time, exemplars, flags.
struct DpCommon<'a> {
    id: Option<Prim<'a, UInt32Type>>,
    start: Option<Prim<'a, TimestampNanosecondType>>,
    time: Option<Prim<'a, TimestampNanosecondType>>,
    flags: Option<Prim<'a, UInt32Type>>,
    index: FxHashMap<u64, Vec<usize>>,
    attrs: Option<AttrTable<'a>>,
    exemplars: Option<Exemplars<'a>>,
}

impl<'a> DpCommon<'a> {
    fn new(
        records: &'a OtapArrowRecords,
        batch: &'a RecordBatch,
        attr_pt: ArrowPayloadType,
        ex: Option<(ArrowPayloadType, ArrowPayloadType)>,
    ) -> Self {
        Self {
            id: prim::<UInt32Type>(batch.column_by_name(consts::ID)),
            start: ts_col(batch.column_by_name(consts::START_TIME_UNIX_NANO)),
            time: ts_col(batch.column_by_name(consts::TIME_UNIX_NANO)),
            flags: prim::<UInt32Type>(batch.column_by_name(consts::FLAGS)),
            index: build_parent_index(batch, ParentWidth::U16),
            attrs: AttrTable::from_records(records, attr_pt, ParentWidth::U32),
            exemplars: ex
                .and_then(|(ex_pt, ex_attr_pt)| Exemplars::new(records, ex_pt, ex_attr_pt)),
        }
    }

    fn dp_id(&self, row: usize) -> u64 {
        self.id
            .as_ref()
            .and_then(|a| a.value_at(row))
            .map(|v| v as u64)
            .unwrap_or(row as u64)
    }

    fn rows_for(&self, metric_id: u64) -> &[usize] {
        self.index.get(&metric_id).map_or(&[][..], |v| v.as_slice())
    }

    /// Emit the seed fields shared by every data point: attributes, start, time.
    fn ser_seed<M: SerializeMap>(
        &self,
        m: &mut M,
        row: usize,
        dp_id: u64,
        mode: Mode,
    ) -> Result<(), M::Error> {
        m.serialize_entry(
            "attributes",
            &AttrsSer {
                table: self.attrs.as_ref(),
                parent_id: dp_id,
                mode,
            },
        )?;
        m.serialize_entry("startTimeUnixNano", &U64Str(ts_at(&self.start, row)))?;
        m.serialize_entry("timeUnixNano", &U64Str(ts_at(&self.time, row)))?;
        Ok(())
    }

    /// Emit the trailing fields shared by exemplar-bearing data points.
    fn ser_finish<M: SerializeMap>(
        &self,
        m: &mut M,
        row: usize,
        dp_id: u64,
        mode: Mode,
    ) -> Result<(), M::Error> {
        m.serialize_entry(
            "exemplars",
            &ExemplarsSeq {
                ex: &self.exemplars,
                dp_id,
                mode,
            },
        )?;
        m.serialize_entry("flags", &u32_at(&self.flags, row))?;
        Ok(())
    }
}

/// Number data points (shared by Gauge and Sum).
struct NumberDp<'a> {
    common: DpCommon<'a>,
    int: Option<Prim<'a, Int64Type>>,
    double: Option<Prim<'a, Float64Type>>,
}

impl<'a> NumberDp<'a> {
    fn new(records: &'a OtapArrowRecords) -> Option<Self> {
        let b = records.get(ArrowPayloadType::NumberDataPoints)?;
        Some(Self {
            common: DpCommon::new(
                records,
                b,
                ArrowPayloadType::NumberDpAttrs,
                Some((
                    ArrowPayloadType::NumberDpExemplars,
                    ArrowPayloadType::NumberDpExemplarAttrs,
                )),
            ),
            int: prim::<Int64Type>(b.column_by_name(consts::INT_VALUE)),
            double: prim::<Float64Type>(b.column_by_name(consts::DOUBLE_VALUE)),
        })
    }
}

struct NumberDpSer<'t, 'a> {
    dp: &'t NumberDp<'a>,
    row: usize,
    mode: Mode,
}
impl Serialize for NumberDpSer<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let dp_id = self.dp.common.dp_id(self.row);
        let mut m = s.serialize_map(None)?;
        self.dp
            .common
            .ser_seed(&mut m, self.row, dp_id, self.mode)?;
        ser_number_value(&mut m, &self.dp.int, &self.dp.double, self.row)?;
        self.dp
            .common
            .ser_finish(&mut m, self.row, dp_id, self.mode)?;
        m.end()
    }
}

struct NumberDps<'t, 'a> {
    dp: &'t Option<NumberDp<'a>>,
    metric_id: u64,
    mode: Mode,
}
impl Serialize for NumberDps<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(None)?;
        if let Some(dp) = self.dp.as_ref() {
            for &row in dp.common.rows_for(self.metric_id) {
                seq.serialize_element(&NumberDpSer {
                    dp,
                    row,
                    mode: self.mode,
                })?;
            }
        }
        seq.end()
    }
}

/// Histogram data points.
struct HistogramDp<'a> {
    common: DpCommon<'a>,
    count: Option<Prim<'a, UInt64Type>>,
    sum: Option<Prim<'a, Float64Type>>,
    min: Option<Prim<'a, Float64Type>>,
    max: Option<Prim<'a, Float64Type>>,
    bucket_counts: Option<&'a ArrayRef>,
    explicit_bounds: Option<&'a ArrayRef>,
}

impl<'a> HistogramDp<'a> {
    fn new(records: &'a OtapArrowRecords) -> Option<Self> {
        let b = records.get(ArrowPayloadType::HistogramDataPoints)?;
        Some(Self {
            common: DpCommon::new(
                records,
                b,
                ArrowPayloadType::HistogramDpAttrs,
                Some((
                    ArrowPayloadType::HistogramDpExemplars,
                    ArrowPayloadType::HistogramDpExemplarAttrs,
                )),
            ),
            count: prim::<UInt64Type>(b.column_by_name(consts::HISTOGRAM_COUNT)),
            sum: prim::<Float64Type>(b.column_by_name(consts::HISTOGRAM_SUM)),
            min: prim::<Float64Type>(b.column_by_name(consts::HISTOGRAM_MIN)),
            max: prim::<Float64Type>(b.column_by_name(consts::HISTOGRAM_MAX)),
            bucket_counts: b.column_by_name(consts::HISTOGRAM_BUCKET_COUNTS),
            explicit_bounds: b.column_by_name(consts::HISTOGRAM_EXPLICIT_BOUNDS),
        })
    }
}

struct HistogramDpSer<'t, 'a> {
    dp: &'t HistogramDp<'a>,
    row: usize,
    mode: Mode,
}
impl Serialize for HistogramDpSer<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let dp_id = self.dp.common.dp_id(self.row);
        let row = self.row;
        let mut m = s.serialize_map(None)?;
        self.dp.common.ser_seed(&mut m, row, dp_id, self.mode)?;
        m.serialize_entry(
            "count",
            &U64Str(
                self.dp
                    .count
                    .as_ref()
                    .and_then(|a| a.value_at(row))
                    .unwrap_or(0),
            ),
        )?;
        m.serialize_entry(
            "sum",
            &OptF64Num(self.dp.sum.as_ref().and_then(|a| a.value_at(row))),
        )?;
        m.serialize_entry("bucketCounts", &U64ListStr(self.dp.bucket_counts, row))?;
        m.serialize_entry("explicitBounds", &F64ListNum(self.dp.explicit_bounds, row))?;
        m.serialize_entry(
            "min",
            &OptF64Num(self.dp.min.as_ref().and_then(|a| a.value_at(row))),
        )?;
        m.serialize_entry(
            "max",
            &OptF64Num(self.dp.max.as_ref().and_then(|a| a.value_at(row))),
        )?;
        self.dp.common.ser_finish(&mut m, row, dp_id, self.mode)?;
        m.end()
    }
}

struct HistogramDps<'t, 'a> {
    dp: &'t Option<HistogramDp<'a>>,
    metric_id: u64,
    mode: Mode,
}
impl Serialize for HistogramDps<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(None)?;
        if let Some(dp) = self.dp.as_ref() {
            for &row in dp.common.rows_for(self.metric_id) {
                seq.serialize_element(&HistogramDpSer {
                    dp,
                    row,
                    mode: self.mode,
                })?;
            }
        }
        seq.end()
    }
}

/// Exponential-histogram data points.
struct ExpHistogramDp<'a> {
    common: DpCommon<'a>,
    count: Option<Prim<'a, UInt64Type>>,
    sum: Option<Prim<'a, Float64Type>>,
    min: Option<Prim<'a, Float64Type>>,
    max: Option<Prim<'a, Float64Type>>,
    scale: Option<Prim<'a, Int32Type>>,
    zero_count: Option<Prim<'a, UInt64Type>>,
    zero_threshold: Option<Prim<'a, Float64Type>>,
    positive: Option<&'a ArrayRef>,
    negative: Option<&'a ArrayRef>,
}

impl<'a> ExpHistogramDp<'a> {
    fn new(records: &'a OtapArrowRecords) -> Option<Self> {
        let b = records.get(ArrowPayloadType::ExpHistogramDataPoints)?;
        Some(Self {
            common: DpCommon::new(
                records,
                b,
                ArrowPayloadType::ExpHistogramDpAttrs,
                Some((
                    ArrowPayloadType::ExpHistogramDpExemplars,
                    ArrowPayloadType::ExpHistogramDpExemplarAttrs,
                )),
            ),
            count: prim::<UInt64Type>(b.column_by_name(consts::HISTOGRAM_COUNT)),
            sum: prim::<Float64Type>(b.column_by_name(consts::HISTOGRAM_SUM)),
            min: prim::<Float64Type>(b.column_by_name(consts::HISTOGRAM_MIN)),
            max: prim::<Float64Type>(b.column_by_name(consts::HISTOGRAM_MAX)),
            scale: prim::<Int32Type>(b.column_by_name(consts::EXP_HISTOGRAM_SCALE)),
            zero_count: prim::<UInt64Type>(b.column_by_name(consts::EXP_HISTOGRAM_ZERO_COUNT)),
            zero_threshold: prim::<Float64Type>(
                b.column_by_name(consts::EXP_HISTOGRAM_ZERO_THRESHOLD),
            ),
            positive: b.column_by_name(consts::EXP_HISTOGRAM_POSITIVE),
            negative: b.column_by_name(consts::EXP_HISTOGRAM_NEGATIVE),
        })
    }
}

struct ExpHistogramDpSer<'t, 'a> {
    dp: &'t ExpHistogramDp<'a>,
    row: usize,
    mode: Mode,
}
impl Serialize for ExpHistogramDpSer<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let dp_id = self.dp.common.dp_id(self.row);
        let row = self.row;
        let mut m = s.serialize_map(None)?;
        self.dp.common.ser_seed(&mut m, row, dp_id, self.mode)?;
        m.serialize_entry(
            "count",
            &U64Str(
                self.dp
                    .count
                    .as_ref()
                    .and_then(|a| a.value_at(row))
                    .unwrap_or(0),
            ),
        )?;
        m.serialize_entry(
            "sum",
            &OptF64Num(self.dp.sum.as_ref().and_then(|a| a.value_at(row))),
        )?;
        m.serialize_entry("scale", &i32_at(&self.dp.scale, row))?;
        m.serialize_entry(
            "zeroCount",
            &U64Str(
                self.dp
                    .zero_count
                    .as_ref()
                    .and_then(|a| a.value_at(row))
                    .unwrap_or(0),
            ),
        )?;
        m.serialize_entry("positive", &ExpBucketsObj(self.dp.positive, row))?;
        m.serialize_entry("negative", &ExpBucketsObj(self.dp.negative, row))?;
        m.serialize_entry(
            "min",
            &OptF64Num(self.dp.min.as_ref().and_then(|a| a.value_at(row))),
        )?;
        m.serialize_entry(
            "max",
            &OptF64Num(self.dp.max.as_ref().and_then(|a| a.value_at(row))),
        )?;
        m.serialize_entry(
            "zeroThreshold",
            &F64Num(
                self.dp
                    .zero_threshold
                    .as_ref()
                    .and_then(|a| a.value_at(row))
                    .unwrap_or(0.0),
            ),
        )?;
        self.dp.common.ser_finish(&mut m, row, dp_id, self.mode)?;
        m.end()
    }
}

struct ExpHistogramDps<'t, 'a> {
    dp: &'t Option<ExpHistogramDp<'a>>,
    metric_id: u64,
    mode: Mode,
}
impl Serialize for ExpHistogramDps<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(None)?;
        if let Some(dp) = self.dp.as_ref() {
            for &row in dp.common.rows_for(self.metric_id) {
                seq.serialize_element(&ExpHistogramDpSer {
                    dp,
                    row,
                    mode: self.mode,
                })?;
            }
        }
        seq.end()
    }
}

/// Summary data points (no exemplars in OTLP).
struct SummaryDp<'a> {
    common: DpCommon<'a>,
    count: Option<Prim<'a, UInt64Type>>,
    sum: Option<Prim<'a, Float64Type>>,
    quantile_values: Option<&'a ArrayRef>,
}

impl<'a> SummaryDp<'a> {
    fn new(records: &'a OtapArrowRecords) -> Option<Self> {
        let b = records.get(ArrowPayloadType::SummaryDataPoints)?;
        Some(Self {
            common: DpCommon::new(records, b, ArrowPayloadType::SummaryDpAttrs, None),
            count: prim::<UInt64Type>(b.column_by_name(consts::SUMMARY_COUNT)),
            sum: prim::<Float64Type>(b.column_by_name(consts::SUMMARY_SUM)),
            quantile_values: b.column_by_name(consts::SUMMARY_QUANTILE_VALUES),
        })
    }
}

struct SummaryDpSer<'t, 'a> {
    dp: &'t SummaryDp<'a>,
    row: usize,
    mode: Mode,
}
impl Serialize for SummaryDpSer<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let dp_id = self.dp.common.dp_id(self.row);
        let row = self.row;
        let mut m = s.serialize_map(None)?;
        self.dp.common.ser_seed(&mut m, row, dp_id, self.mode)?;
        m.serialize_entry(
            "count",
            &U64Str(
                self.dp
                    .count
                    .as_ref()
                    .and_then(|a| a.value_at(row))
                    .unwrap_or(0),
            ),
        )?;
        m.serialize_entry(
            "sum",
            &F64Num(
                self.dp
                    .sum
                    .as_ref()
                    .and_then(|a| a.value_at(row))
                    .unwrap_or(0.0),
            ),
        )?;
        m.serialize_entry("quantileValues", &QuantileSeq(self.dp.quantile_values, row))?;
        m.serialize_entry("flags", &u32_at(&self.dp.common.flags, row))?;
        m.end()
    }
}

struct SummaryDps<'t, 'a> {
    dp: &'t Option<SummaryDp<'a>>,
    metric_id: u64,
    mode: Mode,
}
impl Serialize for SummaryDps<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(None)?;
        if let Some(dp) = self.dp.as_ref() {
            for &row in dp.common.rows_for(self.metric_id) {
                seq.serialize_element(&SummaryDpSer {
                    dp,
                    row,
                    mode: self.mode,
                })?;
            }
        }
        seq.end()
    }
}

struct MetricsCtx<'a> {
    num_rows: usize,
    mode: Mode,
    resource: Option<ResourceCols<'a>>,
    scope: Option<ScopeCols<'a>>,
    resource_attrs: Option<AttrTable<'a>>,
    scope_attrs: Option<AttrTable<'a>>,
    metric_attrs: Option<AttrTable<'a>>,
    id: Option<Prim<'a, UInt16Type>>,
    name: Option<StrCol<'a>>,
    description: Option<StrCol<'a>>,
    unit: Option<StrCol<'a>>,
    metric_type: Option<Prim<'a, UInt8Type>>,
    aggr: Option<Prim<'a, Int32Type>>,
    is_monotonic: Option<&'a BooleanArray>,
    scope_schema_url: Option<StrCol<'a>>,
    number: Option<NumberDp<'a>>,
    histogram: Option<HistogramDp<'a>>,
    exp: Option<ExpHistogramDp<'a>>,
    summary: Option<SummaryDp<'a>>,
}

impl MetricsCtx<'_> {
    fn metric_id(&self, row: usize) -> u64 {
        self.id
            .as_ref()
            .and_then(|a| a.value_at(row))
            .map(|v| v as u64)
            .unwrap_or(row as u64)
    }
}

impl Signal for MetricsCtx<'_> {
    fn num_rows(&self) -> usize {
        self.num_rows
    }
    fn kind(&self) -> &'static str {
        "metrics"
    }
    fn batch_keys(&self) -> (&'static str, &'static str, &'static str) {
        ("resourceMetrics", "scopeMetrics", "metrics")
    }
    fn resource_id(&self, row: usize) -> u64 {
        self.resource.as_ref().map(|r| r.id_at(row)).unwrap_or(0)
    }
    fn scope_id(&self, row: usize) -> u64 {
        self.scope.as_ref().map(|s| s.id_at(row)).unwrap_or(0)
    }
    fn ser_resource<S: Serializer>(&self, row: usize, s: S) -> Result<S::Ok, S::Error> {
        serialize_resource_obj(&self.resource, row, &self.resource_attrs, self.mode, s)
    }
    fn resource_schema_url(&self, row: usize) -> &str {
        resource_schema_url(&self.resource, row)
    }
    fn ser_scope<S: Serializer>(&self, row: usize, s: S) -> Result<S::Ok, S::Error> {
        serialize_scope_obj(&self.scope, row, &self.scope_attrs, self.mode, s)
    }
    fn resource_key(&self, row: usize) -> ResourceKey {
        resource_key(&self.resource, row)
    }
    fn scope_key(&self, row: usize) -> ScopeKey<'_> {
        scope_key(&self.scope, row)
    }
    fn scope_schema_url(&self, row: usize) -> &str {
        str_at(&self.scope_schema_url, row)
    }
    fn ser_record<S: Serializer>(&self, row: usize, s: S) -> Result<S::Ok, S::Error> {
        let mid = self.metric_id(row);
        let mt = self
            .metric_type
            .as_ref()
            .and_then(|a| a.value_at(row))
            .unwrap_or(0);
        let aggr = i32_at(&self.aggr, row);
        let mono = self
            .is_monotonic
            .and_then(|a| a.value_at(row))
            .unwrap_or(false);

        let mut m = s.serialize_map(None)?;
        m.serialize_entry("name", str_at(&self.name, row))?;
        m.serialize_entry("description", str_at(&self.description, row))?;
        m.serialize_entry("unit", str_at(&self.unit, row))?;
        // metadata follows `mode` like every other Vec<KeyValue>: nested-expanded
        // in Nested mode, OTLP KeyValue array in Otlp mode.
        m.serialize_entry(
            "metadata",
            &AttrsSer {
                table: self.metric_attrs.as_ref(),
                parent_id: mid,
                mode: self.mode,
            },
        )?;

        match mt {
            x if x == MetricType::Gauge as u8 => {
                m.serialize_entry(
                    "gauge",
                    &DataPointsObj {
                        points: NumberDps {
                            dp: &self.number,
                            metric_id: mid,
                            mode: self.mode,
                        },
                    },
                )?;
            }
            x if x == MetricType::Sum as u8 => {
                m.serialize_entry(
                    "sum",
                    &SumObj {
                        points: NumberDps {
                            dp: &self.number,
                            metric_id: mid,
                            mode: self.mode,
                        },
                        aggr,
                        mono,
                    },
                )?;
            }
            x if x == MetricType::Histogram as u8 => {
                m.serialize_entry(
                    "histogram",
                    &AggDataPointsObj {
                        points: HistogramDps {
                            dp: &self.histogram,
                            metric_id: mid,
                            mode: self.mode,
                        },
                        aggr,
                    },
                )?;
            }
            x if x == MetricType::ExponentialHistogram as u8 => {
                m.serialize_entry(
                    "exponentialHistogram",
                    &AggDataPointsObj {
                        points: ExpHistogramDps {
                            dp: &self.exp,
                            metric_id: mid,
                            mode: self.mode,
                        },
                        aggr,
                    },
                )?;
            }
            x if x == MetricType::Summary as u8 => {
                m.serialize_entry(
                    "summary",
                    &DataPointsObj {
                        points: SummaryDps {
                            dp: &self.summary,
                            metric_id: mid,
                            mode: self.mode,
                        },
                    },
                )?;
            }
            _ => {}
        }
        m.end()
    }
}

/// `{dataPoints: [...]}` (Gauge, Summary).
struct DataPointsObj<P: Serialize> {
    points: P,
}
impl<P: Serialize> Serialize for DataPointsObj<P> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("dataPoints", &self.points)?;
        m.end()
    }
}

/// `{dataPoints: [...], aggregationTemporality}` (Histogram, ExpHistogram).
struct AggDataPointsObj<P: Serialize> {
    points: P,
    aggr: i32,
}
impl<P: Serialize> Serialize for AggDataPointsObj<P> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("dataPoints", &self.points)?;
        m.serialize_entry("aggregationTemporality", &self.aggr)?;
        m.end()
    }
}

/// `{dataPoints: [...], aggregationTemporality, isMonotonic}` (Sum).
struct SumObj<P: Serialize> {
    points: P,
    aggr: i32,
    mono: bool,
}
impl<P: Serialize> Serialize for SumObj<P> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("dataPoints", &self.points)?;
        m.serialize_entry("aggregationTemporality", &self.aggr)?;
        m.serialize_entry("isMonotonic", &self.mono)?;
        m.end()
    }
}

fn metrics(records: &OtapArrowRecords, layout: Layout, mode: Mode) -> Result<Vec<Bytes>> {
    let batch = records
        .get(ArrowPayloadType::UnivariateMetrics)
        .context("metrics payload missing UnivariateMetrics batch")?;
    let ctx = MetricsCtx {
        num_rows: batch.num_rows(),
        mode,
        resource: ResourceCols::new(batch),
        scope: ScopeCols::new(batch),
        resource_attrs: AttrTable::from_records(
            records,
            ArrowPayloadType::ResourceAttrs,
            ParentWidth::U16,
        ),
        scope_attrs: AttrTable::from_records(
            records,
            ArrowPayloadType::ScopeAttrs,
            ParentWidth::U16,
        ),
        metric_attrs: AttrTable::from_records(
            records,
            ArrowPayloadType::MetricAttrs,
            ParentWidth::U16,
        ),
        id: prim::<UInt16Type>(batch.column_by_name(consts::ID)),
        name: StrCol::try_new(batch.column_by_name(consts::NAME)),
        description: StrCol::try_new(batch.column_by_name(consts::DESCRIPTION)),
        unit: StrCol::try_new(batch.column_by_name(consts::UNIT)),
        metric_type: prim::<UInt8Type>(batch.column_by_name(consts::METRIC_TYPE)),
        aggr: prim::<Int32Type>(batch.column_by_name(consts::AGGREGATION_TEMPORALITY)),
        is_monotonic: bool_col(batch.column_by_name(consts::IS_MONOTONIC)),
        scope_schema_url: StrCol::try_new(batch.column_by_name(consts::SCHEMA_URL)),
        number: NumberDp::new(records),
        histogram: HistogramDp::new(records),
        exp: ExpHistogramDp::new(records),
        summary: SummaryDp::new(records),
    };
    emit(&ctx, layout)
}

#[cfg(test)]
mod tests {
    // Standalone golden tests: the expected JSON strings were captured from
    // nested2's output after it was validated byte-for-byte against the (now
    // removed) `opentelemetry-proto` serde serializers. The metrics goldens also
    // encode the two intentional bug-fixes vs the old output: metric data-point
    // attributes are expanded in Nested mode, and `metadata` is present (and now
    // also follows `mode`, so it expands to `{}`/objects in Nested mode).
    use super::*;
    use otel_arrow_dfe_pdata::encode::{
        encode_logs_otap_batch, encode_metrics_otap_batch, encode_spans_otap_batch,
    };
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::{
        AnyValue, ArrayValue, InstrumentationScope, KeyValue, KeyValueList, any_value,
    };
    use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
        LogRecord, LogsData, ResourceLogs, ScopeLogs,
    };
    use otel_arrow_dfe_pdata::proto::opentelemetry::metrics::v1::{
        Gauge, Metric, MetricsData, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum, metric,
        number_data_point,
    };
    use otel_arrow_dfe_pdata::proto::opentelemetry::resource::v1::Resource;
    use otel_arrow_dfe_pdata::proto::opentelemetry::trace::v1::{
        ResourceSpans, ScopeSpans, Span, TracesData,
        span::{Event, Link},
    };
    use serde_json::Value;

    fn kv(key: &str, v: any_value::Value) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue { value: Some(v) }),
        }
    }

    fn n2(payload: &Payload, layout: Layout, mode: Mode) -> Vec<Value> {
        to_json(payload, layout, mode)
            .expect("nested2")
            .iter()
            .map(|b| serde_json::from_slice(b).expect("json"))
            .collect()
    }

    /// Parse a captured golden JSON string into a `Value` for comparison.
    fn g(s: &str) -> Value {
        serde_json::from_str(s).expect("golden json")
    }

    // ---- logs ----

    fn sample_logs() -> Payload {
        use any_value::Value as V;
        let req = LogsData {
            resource_logs: vec![ResourceLogs {
                resource: Some(Resource {
                    attributes: vec![
                        kv("service.name", V::StringValue("patrolli".to_string())),
                        kv("a.b", V::IntValue(7)),
                    ],
                    ..Default::default()
                }),
                scope_logs: vec![ScopeLogs {
                    scope: Some(InstrumentationScope {
                        name: "sc".to_string(),
                        version: "1.0".to_string(),
                        attributes: vec![kv("s.k", V::StringValue("v".to_string()))],
                        dropped_attributes_count: 0,
                    }),
                    log_records: vec![
                        LogRecord {
                            time_unix_nano: 5,
                            severity_number: 9,
                            severity_text: "INFO".to_string(),
                            body: Some(AnyValue {
                                value: Some(V::StringValue("hello".to_string())),
                            }),
                            attributes: vec![
                                kv("ok", V::BoolValue(true)),
                                kv("ratio", V::DoubleValue(3.5)),
                                kv("raw", V::BytesValue(vec![1, 2, 3])),
                            ],
                            ..Default::default()
                        },
                        LogRecord {
                            time_unix_nano: 6,
                            attributes: vec![kv("k2", V::StringValue("v2".to_string()))],
                            ..Default::default()
                        },
                    ],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        Payload::new(encode_logs_otap_batch(&req).expect("encode"))
    }

    #[test]
    fn logs_otlp_batch_golden() {
        let got = n2(&sample_logs(), Layout::Batch, Mode::Otlp);
        assert_eq!(
            got[0],
            g(
                r#"{"resourceLogs":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"patrolli"}},{"key":"a.b","value":{"intValue":"7"}}],"droppedAttributesCount":0,"entityRefs":[]},"schemaUrl":"","scopeLogs":[{"logRecords":[{"attributes":[{"key":"ok","value":{"boolValue":true}},{"key":"ratio","value":{"doubleValue":3.5}},{"key":"raw","value":{"bytesValue":"AQID"}}],"body":{"stringValue":"hello"},"droppedAttributesCount":0,"eventName":"","flags":0,"observedTimeUnixNano":"0","severityNumber":9,"severityText":"INFO","spanId":"","timeUnixNano":"5","traceId":""},{"attributes":[{"key":"k2","value":{"stringValue":"v2"}}],"body":null,"droppedAttributesCount":0,"eventName":"","flags":0,"observedTimeUnixNano":"0","severityNumber":0,"severityText":"","spanId":"","timeUnixNano":"6","traceId":""}],"schemaUrl":"","scope":{"attributes":[{"key":"s.k","value":{"stringValue":"v"}}],"droppedAttributesCount":0,"name":"sc","version":"1.0"}}]}]}"#
            )
        );
    }

    #[test]
    fn logs_batch_nested_golden() {
        let got = n2(&sample_logs(), Layout::Batch, Mode::Nested);
        assert_eq!(
            got[0],
            g(
                r#"{"resourceLogs":[{"resource":{"attributes":{"a":{"b":7},"service":{"name":"patrolli"}},"droppedAttributesCount":0,"entityRefs":[]},"schemaUrl":"","scopeLogs":[{"logRecords":[{"attributes":{"ok":true,"ratio":3.5,"raw":"AQID"},"body":"hello","droppedAttributesCount":0,"eventName":"","flags":0,"observedTimeUnixNano":"0","severityNumber":9,"severityText":"INFO","spanId":"","timeUnixNano":"5","traceId":""},{"attributes":{"k2":"v2"},"body":null,"droppedAttributesCount":0,"eventName":"","flags":0,"observedTimeUnixNano":"0","severityNumber":0,"severityText":"","spanId":"","timeUnixNano":"6","traceId":""}],"schemaUrl":"","scope":{"attributes":{"s":{"k":"v"}},"droppedAttributesCount":0,"name":"sc","version":"1.0"}}]}]}"#
            )
        );
    }

    #[test]
    fn logs_otlp_denorm_golden() {
        // Denormalized envelope uses camelCase schema-url keys (the ② fix).
        let got = n2(&sample_logs(), Layout::Denorm, Mode::Otlp);
        let want = vec![
            g(
                r#"{"kind":"logs","record":{"attributes":[{"key":"ok","value":{"boolValue":true}},{"key":"ratio","value":{"doubleValue":3.5}},{"key":"raw","value":{"bytesValue":"AQID"}}],"body":{"stringValue":"hello"},"droppedAttributesCount":0,"eventName":"","flags":0,"observedTimeUnixNano":"0","severityNumber":9,"severityText":"INFO","spanId":"","timeUnixNano":"5","traceId":""},"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"patrolli"}},{"key":"a.b","value":{"intValue":"7"}}],"droppedAttributesCount":0,"entityRefs":[]},"resourceSchemaUrl":"","scope":{"attributes":[{"key":"s.k","value":{"stringValue":"v"}}],"droppedAttributesCount":0,"name":"sc","version":"1.0"},"scopeSchemaUrl":""}"#,
            ),
            g(
                r#"{"kind":"logs","record":{"attributes":[{"key":"k2","value":{"stringValue":"v2"}}],"body":null,"droppedAttributesCount":0,"eventName":"","flags":0,"observedTimeUnixNano":"0","severityNumber":0,"severityText":"","spanId":"","timeUnixNano":"6","traceId":""},"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"patrolli"}},{"key":"a.b","value":{"intValue":"7"}}],"droppedAttributesCount":0,"entityRefs":[]},"resourceSchemaUrl":"","scope":{"attributes":[{"key":"s.k","value":{"stringValue":"v"}}],"droppedAttributesCount":0,"name":"sc","version":"1.0"},"scopeSchemaUrl":""}"#,
            ),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn logs_denorm_nested_golden() {
        let got = n2(&sample_logs(), Layout::Denorm, Mode::Nested);
        let want = vec![
            g(
                r#"{"kind":"logs","record":{"attributes":{"ok":true,"ratio":3.5,"raw":"AQID"},"body":"hello","droppedAttributesCount":0,"eventName":"","flags":0,"observedTimeUnixNano":"0","severityNumber":9,"severityText":"INFO","spanId":"","timeUnixNano":"5","traceId":""},"resource":{"attributes":{"a":{"b":7},"service":{"name":"patrolli"}},"droppedAttributesCount":0,"entityRefs":[]},"resourceSchemaUrl":"","scope":{"attributes":{"s":{"k":"v"}},"droppedAttributesCount":0,"name":"sc","version":"1.0"},"scopeSchemaUrl":""}"#,
            ),
            g(
                r#"{"kind":"logs","record":{"attributes":{"k2":"v2"},"body":null,"droppedAttributesCount":0,"eventName":"","flags":0,"observedTimeUnixNano":"0","severityNumber":0,"severityText":"","spanId":"","timeUnixNano":"6","traceId":""},"resource":{"attributes":{"a":{"b":7},"service":{"name":"patrolli"}},"droppedAttributesCount":0,"entityRefs":[]},"resourceSchemaUrl":"","scope":{"attributes":{"s":{"k":"v"}},"droppedAttributesCount":0,"name":"sc","version":"1.0"},"scopeSchemaUrl":""}"#,
            ),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn denorm_record_matches_batch_all_signals_both_modes() {
        // The denormalized record content equals the batched record content for
        // every signal and both attribute modes (only the layout differs).
        let logs = sample_logs();
        let traces = sample_traces();
        let metrics = sample_metrics();
        let cases = [
            (&logs, ("resourceLogs", "scopeLogs", "logRecords")),
            (&traces, ("resourceSpans", "scopeSpans", "spans")),
            (&metrics, ("resourceMetrics", "scopeMetrics", "metrics")),
        ];
        for (p, keys) in cases {
            for mode in [Mode::Nested, Mode::Otlp] {
                let denorm = n2(p, Layout::Denorm, mode);
                let batch = n2(p, Layout::Batch, mode);
                let batch_records = flatten_batch(&batch[0], keys.0, keys.1, keys.2);
                let denorm_records: Vec<Value> =
                    denorm.iter().map(|e| e["record"].clone()).collect();
                assert_eq!(
                    denorm_records, batch_records,
                    "mismatch for {} / {:?}",
                    keys.0, mode as u8
                );
            }
        }
    }

    #[test]
    fn logs_cbor_ser_both_modes() {
        use any_value::Value as V;
        // A kvlist attribute and an array body are CBOR-encoded into the `ser`
        // column by the encoder; both modes must decode them correctly.
        let kvlist = V::KvlistValue(KeyValueList {
            values: vec![
                kv("inner", V::StringValue("v".to_string())),
                kv("n", V::IntValue(3)),
            ],
        });
        let array_body = AnyValue {
            value: Some(V::ArrayValue(ArrayValue {
                values: vec![
                    AnyValue {
                        value: Some(V::IntValue(1)),
                    },
                    AnyValue {
                        value: Some(V::StringValue("two".to_string())),
                    },
                ],
            })),
        };
        let req = LogsData {
            resource_logs: vec![ResourceLogs {
                resource: None,
                scope_logs: vec![ScopeLogs {
                    scope: None,
                    log_records: vec![LogRecord {
                        body: Some(array_body),
                        attributes: vec![kv("map", kvlist)],
                        ..Default::default()
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        let p = Payload::new(encode_logs_otap_batch(&req).expect("encode"));

        let nested = n2(&p, Layout::Denorm, Mode::Nested);
        let rec = &nested[0]["record"];
        assert_eq!(
            rec["attributes"]["map"],
            serde_json::json!({"inner": "v", "n": 3})
        );
        assert_eq!(rec["body"], serde_json::json!([1, "two"]));

        let otlp = n2(&p, Layout::Denorm, Mode::Otlp);
        let rec = &otlp[0]["record"];
        assert_eq!(
            rec["attributes"],
            serde_json::json!([{
                "key": "map",
                "value": {"kvlistValue": {"values": [
                    {"key": "inner", "value": {"stringValue": "v"}},
                    {"key": "n", "value": {"intValue": "3"}}
                ]}}
            }])
        );
        assert_eq!(
            rec["body"],
            serde_json::json!({"arrayValue": {"values": [{"intValue": "1"}, {"stringValue": "two"}]}})
        );
    }

    // ---- traces ----

    fn sample_traces() -> Payload {
        use any_value::Value as V;
        let span = |name: &str, sid: u8| Span {
            trace_id: vec![1u8; 16],
            span_id: vec![sid; 8],
            trace_state: "ts".to_string(),
            parent_span_id: vec![3u8; 8],
            name: name.to_string(),
            kind: 2,
            start_time_unix_nano: 10,
            end_time_unix_nano: 30,
            attributes: vec![kv("http.method", V::StringValue("GET".to_string()))],
            dropped_attributes_count: 0,
            events: vec![Event {
                time_unix_nano: 15,
                name: "ev".to_string(),
                attributes: vec![kv("k", V::StringValue("v".to_string()))],
                dropped_attributes_count: 0,
            }],
            links: vec![Link {
                trace_id: vec![4u8; 16],
                span_id: vec![5u8; 8],
                trace_state: "ls".to_string(),
                attributes: vec![kv("lk", V::StringValue("lv".to_string()))],
                dropped_attributes_count: 0,
                flags: 0,
            }],
            status: None,
            flags: 0,
            dropped_events_count: 0,
            dropped_links_count: 0,
        };
        let req = TracesData {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource {
                    attributes: vec![kv("service.name", V::StringValue("svc".to_string()))],
                    ..Default::default()
                }),
                scope_spans: vec![ScopeSpans {
                    scope: Some(InstrumentationScope {
                        name: "s".to_string(),
                        version: "v".to_string(),
                        ..Default::default()
                    }),
                    spans: vec![span("a", 2), span("b", 9)],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        Payload::new(encode_spans_otap_batch(&req).expect("encode"))
    }

    #[test]
    fn traces_otlp_batch_golden() {
        let got = n2(&sample_traces(), Layout::Batch, Mode::Otlp);
        assert_eq!(
            got[0],
            g(
                r#"{"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"svc"}}],"droppedAttributesCount":0,"entityRefs":[]},"schemaUrl":"","scopeSpans":[{"schemaUrl":"","scope":{"attributes":[],"droppedAttributesCount":0,"name":"s","version":"v"},"spans":[{"attributes":[{"key":"http.method","value":{"stringValue":"GET"}}],"droppedAttributesCount":0,"droppedEventsCount":0,"droppedLinksCount":0,"endTimeUnixNano":"30","events":[{"attributes":[{"key":"k","value":{"stringValue":"v"}}],"droppedAttributesCount":0,"name":"ev","timeUnixNano":"15"}],"flags":0,"kind":2,"links":[{"attributes":[{"key":"lk","value":{"stringValue":"lv"}}],"droppedAttributesCount":0,"flags":0,"spanId":"0505050505050505","traceId":"04040404040404040404040404040404","traceState":"ls"}],"name":"a","parentSpanId":"0303030303030303","spanId":"0202020202020202","startTimeUnixNano":"10","status":{"code":0,"message":""},"traceId":"01010101010101010101010101010101","traceState":"ts"},{"attributes":[{"key":"http.method","value":{"stringValue":"GET"}}],"droppedAttributesCount":0,"droppedEventsCount":0,"droppedLinksCount":0,"endTimeUnixNano":"30","events":[{"attributes":[{"key":"k","value":{"stringValue":"v"}}],"droppedAttributesCount":0,"name":"ev","timeUnixNano":"15"}],"flags":0,"kind":2,"links":[{"attributes":[{"key":"lk","value":{"stringValue":"lv"}}],"droppedAttributesCount":0,"flags":0,"spanId":"0505050505050505","traceId":"04040404040404040404040404040404","traceState":"ls"}],"name":"b","parentSpanId":"0303030303030303","spanId":"0909090909090909","startTimeUnixNano":"10","status":{"code":0,"message":""},"traceId":"01010101010101010101010101010101","traceState":"ts"}]}]}]}"#
            )
        );
    }

    #[test]
    fn traces_batch_nested_golden() {
        let got = n2(&sample_traces(), Layout::Batch, Mode::Nested);
        assert_eq!(
            got[0],
            g(
                r#"{"resourceSpans":[{"resource":{"attributes":{"service":{"name":"svc"}},"droppedAttributesCount":0,"entityRefs":[]},"schemaUrl":"","scopeSpans":[{"schemaUrl":"","scope":{"attributes":{},"droppedAttributesCount":0,"name":"s","version":"v"},"spans":[{"attributes":{"http":{"method":"GET"}},"droppedAttributesCount":0,"droppedEventsCount":0,"droppedLinksCount":0,"endTimeUnixNano":"30","events":[{"attributes":{"k":"v"},"droppedAttributesCount":0,"name":"ev","timeUnixNano":"15"}],"flags":0,"kind":2,"links":[{"attributes":{"lk":"lv"},"droppedAttributesCount":0,"flags":0,"spanId":"0505050505050505","traceId":"04040404040404040404040404040404","traceState":"ls"}],"name":"a","parentSpanId":"0303030303030303","spanId":"0202020202020202","startTimeUnixNano":"10","status":{"code":0,"message":""},"traceId":"01010101010101010101010101010101","traceState":"ts"},{"attributes":{"http":{"method":"GET"}},"droppedAttributesCount":0,"droppedEventsCount":0,"droppedLinksCount":0,"endTimeUnixNano":"30","events":[{"attributes":{"k":"v"},"droppedAttributesCount":0,"name":"ev","timeUnixNano":"15"}],"flags":0,"kind":2,"links":[{"attributes":{"lk":"lv"},"droppedAttributesCount":0,"flags":0,"spanId":"0505050505050505","traceId":"04040404040404040404040404040404","traceState":"ls"}],"name":"b","parentSpanId":"0303030303030303","spanId":"0909090909090909","startTimeUnixNano":"10","status":{"code":0,"message":""},"traceId":"01010101010101010101010101010101","traceState":"ts"}]}]}]}"#
            )
        );
    }

    #[test]
    fn traces_otlp_denorm_golden() {
        let got = n2(&sample_traces(), Layout::Denorm, Mode::Otlp);
        let want = vec![
            g(
                r#"{"kind":"traces","record":{"attributes":[{"key":"http.method","value":{"stringValue":"GET"}}],"droppedAttributesCount":0,"droppedEventsCount":0,"droppedLinksCount":0,"endTimeUnixNano":"30","events":[{"attributes":[{"key":"k","value":{"stringValue":"v"}}],"droppedAttributesCount":0,"name":"ev","timeUnixNano":"15"}],"flags":0,"kind":2,"links":[{"attributes":[{"key":"lk","value":{"stringValue":"lv"}}],"droppedAttributesCount":0,"flags":0,"spanId":"0505050505050505","traceId":"04040404040404040404040404040404","traceState":"ls"}],"name":"a","parentSpanId":"0303030303030303","spanId":"0202020202020202","startTimeUnixNano":"10","status":{"code":0,"message":""},"traceId":"01010101010101010101010101010101","traceState":"ts"},"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"svc"}}],"droppedAttributesCount":0,"entityRefs":[]},"resourceSchemaUrl":"","scope":{"attributes":[],"droppedAttributesCount":0,"name":"s","version":"v"},"scopeSchemaUrl":""}"#,
            ),
            g(
                r#"{"kind":"traces","record":{"attributes":[{"key":"http.method","value":{"stringValue":"GET"}}],"droppedAttributesCount":0,"droppedEventsCount":0,"droppedLinksCount":0,"endTimeUnixNano":"30","events":[{"attributes":[{"key":"k","value":{"stringValue":"v"}}],"droppedAttributesCount":0,"name":"ev","timeUnixNano":"15"}],"flags":0,"kind":2,"links":[{"attributes":[{"key":"lk","value":{"stringValue":"lv"}}],"droppedAttributesCount":0,"flags":0,"spanId":"0505050505050505","traceId":"04040404040404040404040404040404","traceState":"ls"}],"name":"b","parentSpanId":"0303030303030303","spanId":"0909090909090909","startTimeUnixNano":"10","status":{"code":0,"message":""},"traceId":"01010101010101010101010101010101","traceState":"ts"},"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"svc"}}],"droppedAttributesCount":0,"entityRefs":[]},"resourceSchemaUrl":"","scope":{"attributes":[],"droppedAttributesCount":0,"name":"s","version":"v"},"scopeSchemaUrl":""}"#,
            ),
        ];
        assert_eq!(got, want);
    }

    // ---- metrics ----

    fn sample_metrics() -> Payload {
        use any_value::Value as V;
        use number_data_point::Value as NV;
        let gauge = Metric {
            name: "g".to_string(),
            description: "gd".to_string(),
            unit: "gu".to_string(),
            metadata: vec![],
            data: Some(metric::Data::Gauge(Gauge {
                data_points: vec![NumberDataPoint {
                    attributes: vec![kv("a.b", V::StringValue("v".to_string()))],
                    start_time_unix_nano: 1,
                    time_unix_nano: 5,
                    exemplars: vec![],
                    flags: 0,
                    value: Some(NV::AsInt(7)),
                }],
            })),
        };
        let sum = Metric {
            name: "s".to_string(),
            description: String::new(),
            unit: String::new(),
            metadata: vec![],
            data: Some(metric::Data::Sum(Sum {
                data_points: vec![NumberDataPoint {
                    attributes: vec![],
                    start_time_unix_nano: 2,
                    time_unix_nano: 6,
                    exemplars: vec![],
                    flags: 0,
                    value: Some(NV::AsDouble(2.5)),
                }],
                aggregation_temporality: 2,
                is_monotonic: true,
            })),
        };
        let req = MetricsData {
            resource_metrics: vec![ResourceMetrics {
                resource: None,
                scope_metrics: vec![ScopeMetrics {
                    scope: None,
                    metrics: vec![gauge, sum],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        Payload::new(encode_metrics_otap_batch(&req).expect("encode"))
    }

    #[test]
    fn metrics_otlp_batch_golden() {
        let got = n2(&sample_metrics(), Layout::Batch, Mode::Otlp);
        assert_eq!(
            got[0],
            g(
                r#"{"resourceMetrics":[{"resource":{"attributes":[],"droppedAttributesCount":0,"entityRefs":[]},"schemaUrl":"","scopeMetrics":[{"metrics":[{"description":"gd","gauge":{"dataPoints":[{"asInt":7,"attributes":[{"key":"a.b","value":{"stringValue":"v"}}],"exemplars":[],"flags":0,"startTimeUnixNano":"1","timeUnixNano":"5"}]},"metadata":[],"name":"g","unit":"gu"},{"description":"","metadata":[],"name":"s","sum":{"aggregationTemporality":2,"dataPoints":[{"asDouble":2.5,"attributes":[],"exemplars":[],"flags":0,"startTimeUnixNano":"2","timeUnixNano":"6"}],"isMonotonic":true},"unit":""}],"schemaUrl":"","scope":{"attributes":[],"droppedAttributesCount":0,"name":"","version":""}}]}]}"#
            )
        );
    }

    #[test]
    fn metrics_batch_nested_golden() {
        // Standalone golden (no oracle): the current `to_json_batch_nested` has a
        // bug — it never expands metric data-point attributes (it treats the
        // gauge/sum variant object as an array). nested2 correctly expands them in
        // Nested mode, so this asserts the corrected output directly. `metadata`
        // also follows `mode` now, so it is an empty object `{}` in Nested mode.
        let p = sample_metrics();
        let got = n2(&p, Layout::Batch, Mode::Nested);
        let expected = serde_json::json!({
            "resourceMetrics": [{
                "resource": {"attributes": {}, "droppedAttributesCount": 0, "entityRefs": []},
                "schemaUrl": "",
                "scopeMetrics": [{
                    "scope": {"attributes": {}, "droppedAttributesCount": 0, "name": "", "version": ""},
                    "schemaUrl": "",
                    "metrics": [
                        {
                            "name": "g", "description": "gd", "unit": "gu", "metadata": {},
                            "gauge": {"dataPoints": [{
                                "attributes": {"a": {"b": "v"}},
                                "startTimeUnixNano": "1", "timeUnixNano": "5",
                                "asInt": 7, "exemplars": [], "flags": 0
                            }]}
                        },
                        {
                            "name": "s", "description": "", "unit": "", "metadata": {},
                            "sum": {
                                "dataPoints": [{
                                    "attributes": {},
                                    "startTimeUnixNano": "2", "timeUnixNano": "6",
                                    "asDouble": 2.5, "exemplars": [], "flags": 0
                                }],
                                "aggregationTemporality": 2, "isMonotonic": true
                            }
                        }
                    ]
                }]
            }]
        });
        assert_eq!(got[0], expected);
    }

    #[test]
    fn metrics_otlp_denorm_golden() {
        let got = n2(&sample_metrics(), Layout::Denorm, Mode::Otlp);
        let want = vec![
            g(
                r#"{"kind":"metrics","record":{"description":"gd","gauge":{"dataPoints":[{"asInt":7,"attributes":[{"key":"a.b","value":{"stringValue":"v"}}],"exemplars":[],"flags":0,"startTimeUnixNano":"1","timeUnixNano":"5"}]},"metadata":[],"name":"g","unit":"gu"},"resource":{"attributes":[],"droppedAttributesCount":0,"entityRefs":[]},"resourceSchemaUrl":"","scope":{"attributes":[],"droppedAttributesCount":0,"name":"","version":""},"scopeSchemaUrl":""}"#,
            ),
            g(
                r#"{"kind":"metrics","record":{"description":"","metadata":[],"name":"s","sum":{"aggregationTemporality":2,"dataPoints":[{"asDouble":2.5,"attributes":[],"exemplars":[],"flags":0,"startTimeUnixNano":"2","timeUnixNano":"6"}],"isMonotonic":true},"unit":""},"resource":{"attributes":[],"droppedAttributesCount":0,"entityRefs":[]},"resourceSchemaUrl":"","scope":{"attributes":[],"droppedAttributesCount":0,"name":"","version":""},"scopeSchemaUrl":""}"#,
            ),
        ];
        assert_eq!(got, want);
    }

    // ---- helpers ----

    /// Flatten a batch hierarchy back into the per-record list (resource/scope order).
    /// Two resources with two scopes each; the second resource reuses the first
    /// one's scope name. Dropped counts and schema URLs differ throughout.
    fn sample_logs_multi() -> Payload {
        use any_value::Value as V;
        let records = |tag: &str| {
            (0..2)
                .map(|i| LogRecord {
                    time_unix_nano: i,
                    body: Some(AnyValue {
                        value: Some(V::StringValue(format!("{tag}-{i}"))),
                    }),
                    attributes: vec![kv("rec.i", V::IntValue(i as i64))],
                    ..Default::default()
                })
                .collect::<Vec<_>>()
        };
        let scope = |name: &str, dropped: u32, tag: &str| ScopeLogs {
            scope: Some(InstrumentationScope {
                name: name.to_string(),
                version: "1.0".to_string(),
                attributes: vec![kv("scope.tag", V::StringValue(tag.to_string()))],
                dropped_attributes_count: dropped,
            }),
            log_records: records(tag),
            schema_url: format!("https://scope/{tag}"),
        };
        let resource = |svc: &str, dropped: u32, scopes: Vec<ScopeLogs>| ResourceLogs {
            resource: Some(Resource {
                attributes: vec![kv("service.name", V::StringValue(svc.to_string()))],
                dropped_attributes_count: dropped,
                ..Default::default()
            }),
            scope_logs: scopes,
            schema_url: format!("https://resource/{svc}"),
        };
        let req = LogsData {
            resource_logs: vec![
                resource("a", 0, vec![scope("s1", 0, "a1"), scope("s2", 2, "a2")]),
                resource("b", 3, vec![scope("s1", 1, "b1"), scope("s3", 0, "b2")]),
            ],
        };
        Payload::new(encode_logs_otap_batch(&req).expect("encode"))
    }

    #[test]
    fn denorm_envelope_matches_batch_with_several_resources_and_scopes() {
        // The batch layout serializes each resource and scope once per group,
        // so every denormalized envelope must equal its group's objects.
        let p = sample_logs_multi();
        for mode in [Mode::Nested, Mode::Otlp] {
            let batch = n2(&p, Layout::Batch, mode);
            let mut want = Vec::new();
            for r in batch[0]["resourceLogs"].as_array().expect("resourceLogs") {
                for sc in r["scopeLogs"].as_array().expect("scopeLogs") {
                    for rec in sc["logRecords"].as_array().expect("logRecords") {
                        want.push(serde_json::json!({
                            "kind": "logs",
                            "resource": r["resource"],
                            "resourceSchemaUrl": r["schemaUrl"],
                            "scope": sc["scope"],
                            "scopeSchemaUrl": sc["schemaUrl"],
                            "record": rec,
                        }));
                    }
                }
            }
            assert_eq!(want.len(), 8);
            assert_eq!(n2(&p, Layout::Denorm, mode), want, "mode {}", mode as u8);
        }
    }

    #[test]
    fn nested_dotted_key_edge_cases() {
        use any_value::Value as V;
        let keys = [
            "svc.name",
            "x.0.y",
            "x.1",
            "x.0.z",
            "arr.2",
            "arr.0.k",
            "a",
            "a.b",
            "c.d",
            "c",
            "",
            "e..f",
            "g.",
            ".h",
            "k.01",
            "k.1",
            "m.0.0",
            "m.0.1.n",
            "n.x",
            "n.0",
            "p.0",
            "p.x",
            "dup",
            "dup",
            "deep.a.b.c.d.e",
        ];
        let req = LogsData {
            resource_logs: vec![ResourceLogs {
                resource: Some(Resource::default()),
                scope_logs: vec![ScopeLogs {
                    scope: None,
                    log_records: vec![LogRecord {
                        attributes: keys
                            .iter()
                            .enumerate()
                            .map(|(i, k)| kv(k, V::IntValue(i as i64)))
                            .collect(),
                        ..Default::default()
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        let p = Payload::new(encode_logs_otap_batch(&req).expect("encode"));
        // Numeric segments make arrays only when they come first under a key,
        // and order the elements rather than place them (no null padding). A
        // non-canonical number (`01`) is an object key. A key under an existing
        // leaf is dropped, a leaf replaces an existing subtree, a later
        // duplicate wins, and empty segments are ordinary keys.
        let got = n2(&p, Layout::Denorm, Mode::Nested);
        assert_eq!(
            got[0]["record"]["attributes"],
            g(
                r#"{"svc":{"name":0},"x":[{"y":1,"z":3},2],"arr":[{"k":5},4],"a":6,"c":9,"":10,"e":{"":{"f":11}},"g":{"":12},"k":{"01":14,"1":15},"m":[[16,{"n":17}]],"n":{"x":18,"0":19},"p":[20],"dup":23,"deep":{"a":{"b":{"c":{"d":{"e":24}}}}}}"#
            )
        );
    }

    /// The Nested-mode attributes of one log record carrying `keys`, each
    /// valued by its position in `keys`.
    fn nested_attrs(keys: &[&str]) -> Value {
        nested_records(&[keys]).remove(0)
    }

    /// The nested attributes of one record per key list, all in one payload.
    fn nested_records(records: &[&[&str]]) -> Vec<Value> {
        use any_value::Value as V;
        let req = LogsData {
            resource_logs: vec![ResourceLogs {
                resource: Some(Resource::default()),
                scope_logs: vec![ScopeLogs {
                    scope: None,
                    log_records: records
                        .iter()
                        .map(|keys| LogRecord {
                            attributes: keys
                                .iter()
                                .enumerate()
                                .map(|(i, k)| kv(k, V::IntValue(i as i64)))
                                .collect(),
                            ..Default::default()
                        })
                        .collect(),
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        let p = Payload::new(encode_logs_otap_batch(&req).expect("encode"));
        n2(&p, Layout::Denorm, Mode::Nested)
            .iter()
            .map(|r| r["record"]["attributes"].clone())
            .collect()
    }

    #[test]
    fn huge_index_is_order_not_position() {
        // N2: this used to pad the array with four billion nulls.
        assert_eq!(nested_attrs(&["a.4000000000", "a.7"]), g(r#"{"a":[1,0]}"#));
    }

    #[test]
    fn indices_sort_numerically_not_lexicographically() {
        assert_eq!(
            nested_attrs(&["a.10", "a.2", "a.1", "a.0.x", "a.0.y"]),
            g(r#"{"a":[{"x":3,"y":4},2,1,0]}"#)
        );
    }

    #[test]
    fn overlong_index_does_not_overflow() {
        assert_eq!(
            nested_attrs(&["a.99999999999999999999", "a.18446744073709551615", "a.+5"]),
            g(r#"{"a":[1,0]}"#)
        );
    }

    #[test]
    fn indices_past_the_padding_budget_still_sort_in() {
        // Five attributes, so `a.100` and `a.150` go to the sparse part; `a.2`
        // is below them and pads within the budget, and `a.100.y` finds the
        // sparse element `a.100.x` made.
        assert_eq!(
            nested_attrs(&["a.100.x", "a.0", "a.150", "a.100.y", "a.2"]),
            g(r#"{"a":[1,4,{"x":0,"y":3},2]}"#)
        );
    }

    #[test]
    fn wide_objects_keep_finding_their_keys_once_indexed() {
        // Past `INDEXED_FROM` children an object looks keys up through a hash
        // index: a later duplicate still wins, and a key still descends into
        // the child an early key made.
        let mut keys: Vec<String> = vec!["o.p.x".into()];
        keys.extend((0..INDEXED_FROM + 4).map(|i| format!("o.k{i}")));
        keys.push("o.k3".into());
        keys.push("o.p.y".into());
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        let got = nested_attrs(&keys);
        let o = &got["o"];
        let n = INDEXED_FROM;
        assert_eq!(o["p"], g(&format!(r#"{{"x":0,"y":{}}}"#, n + 6)));
        assert_eq!(o["k3"], n + 5);
        assert_eq!(o[format!("k{}", n + 3)], n + 4);
        assert_eq!(o.as_object().map(|m| m.len()), Some(n + 5));
    }

    #[test]
    fn records_do_not_inherit_the_trees_before_them() {
        // The records of a payload reuse each other's tree storage: an indexed
        // object, holes, a sparse part and nested containers must not carry
        // over into the next record.
        let mut wide: Vec<String> = (0..INDEXED_FROM + 2).map(|i| format!("o.k{i}")).collect();
        wide.extend(["a.3", "a.1000", "x.0.y", "x.1.z"].map(String::from));
        let wide: Vec<&str> = wide.iter().map(String::as_str).collect();
        let next: &[&str] = &["o.k1", "a.1", "x.y", "a.0.b"];
        let got = nested_records(&[&wide, next, &wide]);
        assert_eq!(got[0], nested_attrs(&wide));
        assert_eq!(got[1], nested_attrs(next));
        assert_eq!(got[1], g(r#"{"o":{"k1":0},"a":[{"b":3},1],"x":{"y":2}}"#));
        assert_eq!(got[2], got[0]);
    }

    fn flatten_batch(v: &Value, res_key: &str, scope_key: &str, records_key: &str) -> Vec<Value> {
        let mut out = Vec::new();
        for r in v[res_key].as_array().into_iter().flatten() {
            for s in r[scope_key].as_array().into_iter().flatten() {
                for rec in s[records_key].as_array().into_iter().flatten() {
                    out.push(rec.clone());
                }
            }
        }
        out
    }
}
