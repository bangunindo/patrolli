// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Shared helpers for reading OTAP attribute tables and building row masks over
//! a [`Payload`](super::Payload)'s root record batch.
//!
//! These are the low-level primitives used both by the listener's annotate/match
//! path (`src/listener/annotate.rs`) and by [`PayloadBatch::shard`](super::PayloadBatch::shard).
//! They operate directly on Arrow with `arrow` kernels rather than DataFusion.

use crate::config;
use anyhow::Result;
use arrow::array::{
    Array, ArrayAccessor, ArrayRef, AsArray, BooleanArray, Datum, Float64Array, Int64Array,
    RecordBatch, StringArray, TypedDictionaryArray, UInt8Array, UInt16Array,
};
use arrow::compute::kernels;
use arrow::datatypes::{DataType, Float64Type, Int64Type, UInt8Type, UInt16Type};
use otel_arrow_dfe_pdata::OtapArrowRecords;
use otel_arrow_dfe_pdata::otap::OtapBatchStore;
use otel_arrow_dfe_pdata::otlp::attributes::AttributeValueType;
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
use otel_arrow_dfe_pdata::schema::consts;
use rustc_hash::{FxHashMap, FxHashSet};
use smol_str::{SmolStr, format_smolstr};

use super::{Payload, Value, ZString};

/// A plain-or-dictionary Utf8 column read positionally.
pub(crate) enum StrCol<'a> {
    Plain(&'a StringArray),
    Dict8(TypedDictionaryArray<'a, UInt8Type, StringArray>),
    Dict16(TypedDictionaryArray<'a, UInt16Type, StringArray>),
}

impl<'a> StrCol<'a> {
    pub(crate) fn try_new(arr: &'a dyn Array) -> Result<Self> {
        match arr.data_type() {
            DataType::Utf8 => Ok(Self::Plain(arr.as_string::<i32>())),
            DataType::Dictionary(k, v) if **k == DataType::UInt8 && **v == DataType::Utf8 => {
                let d = arr
                    .as_dictionary::<UInt8Type>()
                    .downcast_dict::<StringArray>()
                    .ok_or_else(|| anyhow::anyhow!("dict values not Utf8"))?;
                Ok(Self::Dict8(d))
            }
            DataType::Dictionary(k, v) if **k == DataType::UInt16 && **v == DataType::Utf8 => {
                let d = arr
                    .as_dictionary::<UInt16Type>()
                    .downcast_dict::<StringArray>()
                    .ok_or_else(|| anyhow::anyhow!("dict values not Utf8"))?;
                Ok(Self::Dict16(d))
            }
            dt => Err(anyhow::anyhow!("unexpected attr_str type: {dt:?}")),
        }
    }

    pub(crate) fn is_valid(&self, i: usize) -> bool {
        match self {
            Self::Plain(a) => a.is_valid(i),
            Self::Dict8(a) => a.is_valid(i),
            Self::Dict16(a) => a.is_valid(i),
        }
    }

    /// Borrows from the batch, not from `self`, so a reader may hand the `&str`
    /// back to a caller that has already dropped the column wrapper.
    pub(crate) fn value(&self, i: usize) -> &'a str {
        match self {
            Self::Plain(a) => a.value(i),
            Self::Dict8(a) => a.value(i),
            Self::Dict16(a) => a.value(i),
        }
    }

    /// Whether any of `needles` can be on a row of this column.
    ///
    /// A dictionary answers for the whole column from its values — a string
    /// that is not in the dictionary is on no row — which lets a caller skip a
    /// per-row scan for the usual case of looking for keys the batch does not
    /// carry. A plain column has no such summary and always answers yes, which
    /// is the conservative answer: `false` is a promise, `true` is a maybe.
    pub(crate) fn may_hold_any<'n>(&self, needles: impl IntoIterator<Item = &'n str>) -> bool {
        let values = match self {
            Self::Plain(_) => return true,
            Self::Dict8(a) => a.values(),
            Self::Dict16(a) => a.values(),
        };
        needles
            .into_iter()
            .any(|needle| values.iter().flatten().any(|value| value == needle))
    }
}

/// A plain-or-dictionary Int64 column read positionally. The OTAP encoder
/// adaptively dictionary-encodes the `attribute.int` column (see
/// `AnyValuesRecordsBuilder`), so it can arrive as a plain `Int64Array` or a
/// `Dictionary(UInt8|UInt16, Int64)`. Mirrors [`StrCol`] so callers index it the
/// same way regardless of encoding.
pub(crate) enum IntCol<'a> {
    Plain(&'a Int64Array),
    Dict8(TypedDictionaryArray<'a, UInt8Type, Int64Array>),
    Dict16(TypedDictionaryArray<'a, UInt16Type, Int64Array>),
}

impl<'a> IntCol<'a> {
    pub(crate) fn try_new(arr: &'a dyn Array) -> Result<Self> {
        match arr.data_type() {
            DataType::Int64 => Ok(Self::Plain(arr.as_primitive::<Int64Type>())),
            DataType::Dictionary(k, v) if **k == DataType::UInt8 && **v == DataType::Int64 => {
                let d = arr
                    .as_dictionary::<UInt8Type>()
                    .downcast_dict::<Int64Array>()
                    .ok_or_else(|| anyhow::anyhow!("dict values not Int64"))?;
                Ok(Self::Dict8(d))
            }
            DataType::Dictionary(k, v) if **k == DataType::UInt16 && **v == DataType::Int64 => {
                let d = arr
                    .as_dictionary::<UInt16Type>()
                    .downcast_dict::<Int64Array>()
                    .ok_or_else(|| anyhow::anyhow!("dict values not Int64"))?;
                Ok(Self::Dict16(d))
            }
            dt => Err(anyhow::anyhow!("unexpected attr_int type: {dt:?}")),
        }
    }

    pub(crate) fn is_valid(&self, i: usize) -> bool {
        match self {
            Self::Plain(a) => a.is_valid(i),
            Self::Dict8(a) => a.is_valid(i),
            Self::Dict16(a) => a.is_valid(i),
        }
    }

    pub(crate) fn value(&self, i: usize) -> i64 {
        match self {
            Self::Plain(a) => a.value(i),
            Self::Dict8(a) => a.value(i),
            Self::Dict16(a) => a.value(i),
        }
    }
}

/// Which attribute table an attribute lives in. Each location's `parent_id`
/// references a different root-record id: the record's own `id` (Record), or the
/// `resource`/`scope` struct's `id` it points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrLocation {
    Resource,
    Scope,
    Record,
}

/// The root record batch (Logs / UnivariateMetrics / Spans).
pub(crate) fn get_record(pl: &Payload) -> Option<&RecordBatch> {
    match &pl.data {
        OtapArrowRecords::Logs(records) => records.get(ArrowPayloadType::Logs),
        OtapArrowRecords::Metrics(records) => records.get(ArrowPayloadType::UnivariateMetrics),
        OtapArrowRecords::Traces(records) => records.get(ArrowPayloadType::Spans),
    }
}

/// The record-level attribute batch (LogAttrs / MetricAttrs / SpanAttrs).
pub(crate) fn get_record_attr(pl: &Payload) -> Option<&RecordBatch> {
    match &pl.data {
        OtapArrowRecords::Logs(records) => records.get(ArrowPayloadType::LogAttrs),
        OtapArrowRecords::Metrics(records) => records.get(ArrowPayloadType::MetricAttrs),
        OtapArrowRecords::Traces(records) => records.get(ArrowPayloadType::SpanAttrs),
    }
}

/// `record.<struct_name>.id` column (e.g. the resource/scope id referenced by a
/// record), or `None` if the struct or its id column is absent.
pub(crate) fn struct_id_column<'a>(
    record: &'a RecordBatch,
    struct_name: &str,
) -> Option<&'a ArrayRef> {
    record
        .column_by_name(struct_name)?
        .as_struct_opt()?
        .column_by_name(consts::ID)
}

/// Per-record mask (`true` where the record's id for `loc` is in `id_set`),
/// length `pl.num_rows()` — the root row count, which is what `filter_otap_batch`
/// requires. Note this is *not* `pl.num_items()`: for metrics that counts data
/// points, not metric rows.
pub(crate) fn get_mask_by_id(
    pl: &Payload,
    loc: AttrLocation,
    id_set: &FxHashSet<u16>,
) -> Result<BooleanArray> {
    let mut masks = vec![false; pl.num_rows()];
    let Some(record) = get_record(pl) else {
        return Ok(BooleanArray::from(masks));
    };
    let id_col = match loc {
        AttrLocation::Resource => struct_id_column(record, consts::RESOURCE),
        AttrLocation::Scope => struct_id_column(record, consts::SCOPE),
        AttrLocation::Record => record.column_by_name(consts::ID),
    };
    let Some(id_col) = id_col else {
        return Ok(BooleanArray::from(masks));
    };
    let id_col_ref = id_col
        .as_primitive_opt::<UInt16Type>()
        .ok_or_else(|| anyhow::anyhow!("Failed to convert id column to UInt16Type"))?;
    for (i, id) in id_col_ref.iter().enumerate() {
        if let Some(id) = id
            && id_set.contains(&id)
        {
            masks[i] = true;
        }
    }
    Ok(BooleanArray::from(masks))
}

/// Per-root-row parent id for `loc`, or `None` where the row cannot own an
/// attribute in that table.
///
/// Missing/null IDs and null resource/scope structs have no attribute owner.
/// ID 0 is a real owner and must never stand in for an absent one.
pub(crate) fn location_parent_ids(pl: &Payload, loc: AttrLocation) -> Result<Vec<Option<u16>>> {
    let n = pl.num_rows();
    let Some(record) = get_record(pl) else {
        return Ok(vec![None; n]);
    };
    let owner = match loc {
        AttrLocation::Record => None,
        AttrLocation::Resource => record.column_by_name(consts::RESOURCE),
        AttrLocation::Scope => record.column_by_name(consts::SCOPE),
    }
    .and_then(|column| column.as_struct_opt());
    let id_col = match loc {
        AttrLocation::Resource | AttrLocation::Scope => {
            owner.and_then(|owner| owner.column_by_name(consts::ID))
        }
        AttrLocation::Record => record.column_by_name(consts::ID),
    };
    let Some(id_col) = id_col else {
        return Ok(vec![None; n]);
    };
    let id_col = id_col
        .as_primitive_opt::<UInt16Type>()
        .ok_or_else(|| anyhow::anyhow!("Failed to convert id column to UInt16Type"))?;
    Ok((0..n)
        .map(|i| {
            if i < id_col.len() && id_col.is_valid(i) && owner.is_none_or(|owner| owner.is_valid(i))
            {
                Some(id_col.value(i))
            } else {
                None
            }
        })
        .collect())
}

/// Map each `parent_id` to the stringified value of the attribute `key`, for rows
/// in `attr_batch` whose key matches and whose `type` is `Str` or `Int` (ints are
/// rendered as their decimal string). Rows with any other type, a missing key, or
/// a null value are simply absent from the map — their records fall in the `None`
/// shard. When a `parent_id` carries `key` more than once, the last row wins.
pub(crate) fn parent_id_values(
    attr_batch: &RecordBatch,
    key: &str,
) -> Result<FxHashMap<u16, SmolStr>> {
    let mut out = FxHashMap::default();
    if attr_batch.num_rows() == 0 {
        return Ok(out);
    }
    let Some(parent_id_col) = attr_batch.column_by_name(consts::PARENT_ID) else {
        return Ok(out);
    };
    let parent_id_col = parent_id_col
        .as_primitive_opt::<UInt16Type>()
        .ok_or_else(|| anyhow::anyhow!("Failed to convert parent ID column to UInt16Type"))?;
    let Some(key_col) = attr_batch.column_by_name(consts::ATTRIBUTE_KEY) else {
        return Ok(out);
    };
    // Match the key column against `key` with an arrow comparison kernel — it has a
    // dictionary fast path, so plain and dictionary-encoded Utf8 keys are both
    // handled without decoding the whole column row by row.
    let key_mask = kernels::cmp::eq(&key_col, &StringArray::new_scalar(key))?;
    // `type == Str` / `type == Int`, each ANDed with the key match → one final
    // boolean mask per shardable value type. When the `type` column is absent
    // (defensive — OTAP normally always carries it) we fall back to reading
    // whichever value column is populated on a key-matching row.
    let type_masks = attr_batch
        .column_by_name(consts::ATTRIBUTE_TYPE)
        .map(|type_col| -> Result<(BooleanArray, BooleanArray)> {
            let str_eq = kernels::cmp::eq(
                &type_col,
                &UInt8Array::new_scalar(AttributeValueType::Str as u8),
            )?;
            let int_eq = kernels::cmp::eq(
                &type_col,
                &UInt8Array::new_scalar(AttributeValueType::Int as u8),
            )?;
            Ok((
                kernels::boolean::and(&key_mask, &str_eq)?,
                kernels::boolean::and(&key_mask, &int_eq)?,
            ))
        })
        .transpose()?;
    let str_col = attr_batch
        .column_by_name(consts::ATTRIBUTE_STR)
        .map(|c| StrCol::try_new(c.as_ref()))
        .transpose()?;
    let int_col = attr_batch
        .column_by_name(consts::ATTRIBUTE_INT)
        .map(|c| IntCol::try_new(c.as_ref()))
        .transpose()?;

    let read_str = |i: usize| -> Option<SmolStr> {
        str_col
            .as_ref()
            .filter(|c| c.is_valid(i))
            .map(|c| SmolStr::new(c.value(i)))
    };
    let read_int = |i: usize| -> Option<SmolStr> {
        int_col
            .as_ref()
            .filter(|c| c.is_valid(i))
            .map(|c| format_smolstr!("{}", c.value(i)))
    };
    let is_set = |m: &BooleanArray, i: usize| m.is_valid(i) && m.value(i);

    for i in 0..attr_batch.num_rows() {
        if !parent_id_col.is_valid(i) {
            continue;
        }
        let val = match &type_masks {
            Some((str_match, int_match)) => {
                if is_set(str_match, i) {
                    read_str(i)
                } else if is_set(int_match, i) {
                    read_int(i)
                } else {
                    None
                }
            }
            // No `type` column: a key-matching row reads whichever value is set.
            None if is_set(&key_mask, i) => read_str(i).or_else(|| read_int(i)),
            None => None,
        };
        if let Some(val) = val {
            out.insert(parent_id_col.value(i), val);
        }
    }
    Ok(out)
}

/// Insert the `parent_id` of every attribute row whose `row_mask` is `true` into
/// `set`. `parent_id` points back at the owning record/resource/scope id.
pub(crate) fn collect_parent_ids(
    attr_batch: &RecordBatch,
    row_mask: &BooleanArray,
    set: &mut FxHashSet<u16>,
) -> Result<()> {
    let Some(parent_id_col) = attr_batch.column_by_name(consts::PARENT_ID) else {
        return Ok(());
    };
    let parent_id_col = parent_id_col
        .as_primitive_opt::<UInt16Type>()
        .ok_or_else(|| anyhow::anyhow!("Failed to convert parent ID column to UInt16Type"))?;
    for (i, m) in row_mask.iter().enumerate() {
        if m == Some(true) && parent_id_col.is_valid(i) {
            set.insert(parent_id_col.value(i));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// AttributeFilter (arrow kernels)
//
// Evaluate an `AttributeFilter` tree directly with arrow compute kernels (no
// DataFusion). Each `Match` leaf is reduced to the set of attribute `parent_id`s
// whose row satisfies `key <op> value`, then mapped to a per-record mask via
// `get_mask_by_id`; `And`/`Or` combine those record-level masks. Shared by the
// listener's `AttributeMatch` item-matching path (`src/listener/annotate.rs`) and
// the `attribute_filter` processor (`src/processor/attribute_filter.rs`).
//
// The returned per-record mask is always non-null (the `Match` arm ends in
// `get_mask_by_id`, which builds from `vec![false; len]`, and `And`/`Or` of
// non-null masks stay non-null), so callers may feed it straight to
// `filter_otap_batch` (true = keep).
// ---------------------------------------------------------------------------

/// Per-record mask (`true` where the record matches `filter`), length
/// `pl.num_rows()`.
pub(crate) fn attribute_filter_mask(
    pl: &Payload,
    filter: &config::processor::AttributeFilter,
) -> Result<BooleanArray> {
    use config::processor::AttributeFilter;
    match filter {
        AttributeFilter::And(children) => {
            let mut acc: Option<BooleanArray> = None;
            for child in children {
                let mask = attribute_filter_mask(pl, child)?;
                acc = Some(match acc {
                    Some(prev) => kernels::boolean::and(&prev, &mask)?,
                    None => mask,
                });
            }
            // empty AND ≡ true
            Ok(acc.unwrap_or_else(|| BooleanArray::from(vec![true; pl.num_rows()])))
        }
        AttributeFilter::Or(children) => {
            let mut acc: Option<BooleanArray> = None;
            for child in children {
                let mask = attribute_filter_mask(pl, child)?;
                acc = Some(match acc {
                    Some(prev) => kernels::boolean::or(&prev, &mask)?,
                    None => mask,
                });
            }
            // empty OR ≡ false
            Ok(acc.unwrap_or_else(|| BooleanArray::from(vec![false; pl.num_rows()])))
        }
        AttributeFilter::Match(expr) => match_expr_mask(pl, expr),
    }
}

/// Per-record mask for a single `key <op> value` leaf against one attribute table.
fn match_expr_mask(
    pl: &Payload,
    expr: &config::processor::AttributeFilterExpr,
) -> Result<BooleanArray> {
    use config::processor::AttributeTable;
    let (attr_batch, loc) = match expr.table {
        AttributeTable::Record => (get_record_attr(pl), AttrLocation::Record),
        AttributeTable::Resource => (
            pl.data.get(ArrowPayloadType::ResourceAttrs),
            AttrLocation::Resource,
        ),
        AttributeTable::Scope => (
            pl.data.get(ArrowPayloadType::ScopeAttrs),
            AttrLocation::Scope,
        ),
    };
    let Some(attr_batch) = attr_batch else {
        return Ok(BooleanArray::from(vec![false; pl.num_rows()]));
    };
    let Some(row_mask) = attr_match_row_mask(attr_batch, expr)? else {
        return Ok(BooleanArray::from(vec![false; pl.num_rows()]));
    };
    let mut set = FxHashSet::default();
    collect_parent_ids(attr_batch, &row_mask, &mut set)?;
    get_mask_by_id(pl, loc, &set)
}

/// Row mask over `attr_batch` (`true` where `key <op> value` holds), or `None`
/// when a required column is missing or the batch is empty.
fn attr_match_row_mask(
    attr_batch: &RecordBatch,
    expr: &config::processor::AttributeFilterExpr,
) -> Result<Option<BooleanArray>> {
    use config::processor::{KeyValueOp, Value as FilterValue};
    if attr_batch.num_rows() == 0 {
        return Ok(None);
    }
    let Some(key_col) = attr_batch.column_by_name(consts::ATTRIBUTE_KEY) else {
        return Ok(None);
    };
    let key_mask = if expr.key_is_regex {
        regex_mask(key_col, &expr.key)?
    } else {
        kernels::cmp::eq(&key_col, &StringArray::new_scalar(expr.key.as_str()))?
    };
    let value_mask = match &expr.value {
        FilterValue::String(v) => {
            let Some(col) = attr_batch.column_by_name(consts::ATTRIBUTE_STR) else {
                return Ok(None);
            };
            match expr.op {
                KeyValueOp::RegexEqual => regex_mask(col, v)?,
                KeyValueOp::RegexNotEqual => kernels::boolean::not(&regex_mask(col, v)?)?,
                _ => cmp_scalar(&expr.op, col, &StringArray::new_scalar(v.as_str()))?,
            }
        }
        FilterValue::Integer(v) => {
            let Some(col) = attr_batch.column_by_name(consts::ATTRIBUTE_INT) else {
                return Ok(None);
            };
            cmp_scalar(&expr.op, col, &Int64Array::new_scalar(*v))?
        }
        FilterValue::Float(v) => {
            let Some(col) = attr_batch.column_by_name(consts::ATTRIBUTE_DOUBLE) else {
                return Ok(None);
            };
            cmp_scalar(&expr.op, col, &Float64Array::new_scalar(*v))?
        }
    };
    Ok(Some(kernels::boolean::and(&key_mask, &value_mask)?))
}

/// Apply an ordered comparison operator between a column and a scalar. Regex
/// operators are rejected — they are only valid for string values and handled by
/// the caller via [`regex_mask`].
fn cmp_scalar<D: Datum>(
    op: &config::processor::KeyValueOp,
    col: &ArrayRef,
    scalar: &D,
) -> Result<BooleanArray> {
    use config::processor::KeyValueOp;
    Ok(match op {
        KeyValueOp::Equal => kernels::cmp::eq(&col, scalar)?,
        KeyValueOp::NotEqual => kernels::cmp::neq(&col, scalar)?,
        KeyValueOp::LessThan => kernels::cmp::lt(&col, scalar)?,
        KeyValueOp::LessThanOrEqual => kernels::cmp::lt_eq(&col, scalar)?,
        KeyValueOp::GreaterThan => kernels::cmp::gt(&col, scalar)?,
        KeyValueOp::GreaterThanOrEqual => kernels::cmp::gt_eq(&col, scalar)?,
        KeyValueOp::RegexEqual | KeyValueOp::RegexNotEqual => {
            return Err(anyhow::anyhow!(
                "regex operator is only valid for string values"
            ));
        }
    })
}

/// Test each row's Utf8 value against `pattern`. Null entries yield null (no match).
///
/// The regexp kernel only accepts plain string arrays — unlike the `cmp` kernels
/// it has no dictionary fast path. Rather than decode the whole column, for a
/// dictionary we match its *distinct values* and expand the result back over the
/// keys with `take`, so the regex runs once per distinct value, not once per row.
fn regex_mask(col: &ArrayRef, pattern: &str) -> Result<BooleanArray> {
    let regex_utf8 = |arr: &dyn Array| -> Result<BooleanArray> {
        Ok(kernels::regexp::regexp_is_match_scalar(
            arr.as_string::<i32>(),
            pattern,
            None,
        )?)
    };
    match col.data_type() {
        DataType::Utf8 => regex_utf8(col.as_ref()),
        DataType::Dictionary(_, value_type) if **value_type == DataType::Utf8 => {
            let dict = col.as_any_dictionary();
            let value_matches = regex_utf8(dict.values().as_ref())?;
            let expanded = kernels::take::take(&value_matches, dict.keys(), None)?;
            Ok(expanded.as_boolean().clone())
        }
        // Other string encodings (LargeUtf8, Utf8View, non-Utf8 dict values):
        // decode to Utf8 and run the kernel. `cast` is a cheap buffer-sharing
        // clone when the column is already Utf8.
        _ => regex_utf8(kernels::cast::cast(col.as_ref(), &DataType::Utf8)?.as_ref()),
    }
}

// ---- typed attribute reads (the `attribute_lookup` processor) ----
//
// `parent_id_values` above renders everything as a string. A lookup probe key
// needs the OTAP value type intact: `cell_matches_key` is strictly
// variant-matched, so an int read back as "42" never matches an integer key.

/// The value columns of one attribute batch, read positionally.
///
/// `str` and `int` may arrive plain or dictionary-encoded (hence [`StrCol`] /
/// [`IntCol`]); `double` and `bool` never are, per the OTAP `attributes_16`
/// schema, so they downcast directly.
pub(crate) struct AttrValueCols<'a> {
    type_col: Option<&'a UInt8Array>,
    str_col: Option<StrCol<'a>>,
    int_col: Option<IntCol<'a>>,
    double_col: Option<&'a Float64Array>,
    bool_col: Option<&'a BooleanArray>,
}

impl<'a> AttrValueCols<'a> {
    pub(crate) fn try_new(attr_batch: &'a RecordBatch) -> Result<Self> {
        Ok(Self {
            type_col: attr_batch
                .column_by_name(consts::ATTRIBUTE_TYPE)
                .and_then(|c| c.as_primitive_opt::<UInt8Type>()),
            str_col: attr_batch
                .column_by_name(consts::ATTRIBUTE_STR)
                .map(|c| StrCol::try_new(c.as_ref()))
                .transpose()?,
            int_col: attr_batch
                .column_by_name(consts::ATTRIBUTE_INT)
                .map(|c| IntCol::try_new(c.as_ref()))
                .transpose()?,
            double_col: attr_batch
                .column_by_name(consts::ATTRIBUTE_DOUBLE)
                .and_then(|c| c.as_primitive_opt::<Float64Type>()),
            bool_col: attr_batch
                .column_by_name(consts::ATTRIBUTE_BOOL)
                .and_then(|c| c.as_boolean_opt()),
        })
    }

    /// Row `i`'s value, or `None` when it is null, its column is absent, or
    /// its type is one this reader does not surface — `Empty`, `Bytes` and the
    /// CBOR `Map`/`Slice` forms are neither probe keys nor scalar values.
    pub(crate) fn value_at(&self, i: usize) -> Option<Value<'a>> {
        let str_at = || {
            self.str_col
                .as_ref()
                .filter(|c| c.is_valid(i))
                .map(|c| Value::String(ZString::from_ref(c.value(i))))
        };
        let int_at = || {
            self.int_col
                .as_ref()
                .filter(|c| c.is_valid(i))
                .map(|c| Value::Int64(c.value(i)))
        };
        let double_at = || {
            self.double_col
                .filter(|c| c.is_valid(i))
                .map(|c| Value::Double(c.value(i)))
        };
        let bool_at = || {
            self.bool_col
                .filter(|c| c.is_valid(i))
                .map(|c| Value::Bool(c.value(i)))
        };
        match self.type_col {
            Some(t) if t.is_valid(i) => match AttributeValueType::try_from(t.value(i)).ok()? {
                AttributeValueType::Str => str_at(),
                AttributeValueType::Int => int_at(),
                AttributeValueType::Double => double_at(),
                AttributeValueType::Bool => bool_at(),
                _ => None,
            },
            // No `type` column (defensive — OTAP always carries it): read
            // whichever value column happens to be populated.
            _ => str_at().or_else(int_at).or_else(double_at).or_else(bool_at),
        }
    }

    /// Row `i`'s value, reading an explicitly null attribute as [`Value::Null`]
    /// rather than as absent — `attribute_collect` stores a null cell for the
    /// first and skips the whole candidate row for the second. Types this
    /// reader cannot surface stay `None`: unusable inputs, not nulls.
    pub(crate) fn value_at_or_null(&self, i: usize) -> Option<Value<'a>> {
        match self.type_col {
            Some(t) if t.is_valid(i) => match AttributeValueType::try_from(t.value(i)).ok()? {
                AttributeValueType::Empty => Some(Value::Null),
                AttributeValueType::Str
                | AttributeValueType::Int
                | AttributeValueType::Double
                | AttributeValueType::Bool => Some(self.value_at(i).unwrap_or(Value::Null)),
                AttributeValueType::Map | AttributeValueType::Slice | AttributeValueType::Bytes => {
                    None
                }
            },
            // Without a `type` column an unreadable row cannot be told apart from
            // an unsupported one; the key is present either way, so it is null.
            _ => Some(self.value_at(i).unwrap_or(Value::Null)),
        }
    }
}

fn parent_ids_u16(attr_batch: &RecordBatch) -> Result<Option<&UInt16Array>> {
    let Some(col) = attr_batch.column_by_name(consts::PARENT_ID) else {
        return Ok(None);
    };
    col.as_primitive_opt::<UInt16Type>()
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("Failed to convert parent ID column to UInt16Type"))
}

/// Picks the null policy the two typed readers share.
fn read<'a>(values: &AttrValueCols<'a>, i: usize, keep_nulls: bool) -> Option<Value<'a>> {
    if keep_nulls {
        values.value_at_or_null(i)
    } else {
        values.value_at(i)
    }
}

/// Map each `parent_id` to the typed value of the attribute named exactly
/// `key`; the last row wins on a repeat, matching [`parent_id_values`]. With
/// `keep_nulls`, a present-but-null attribute maps to [`Value::Null`] — see
/// [`AttrValueCols::value_at_or_null`].
pub(crate) fn parent_id_typed_values<'a>(
    attr_batch: &'a RecordBatch,
    key: &str,
    keep_nulls: bool,
) -> Result<FxHashMap<u16, Value<'a>>> {
    let mut out = FxHashMap::default();
    if attr_batch.num_rows() == 0 {
        return Ok(out);
    }
    let (Some(parent_id_col), Some(key_col)) = (
        parent_ids_u16(attr_batch)?,
        attr_batch.column_by_name(consts::ATTRIBUTE_KEY),
    ) else {
        return Ok(out);
    };
    // The `cmp` kernel has a dictionary fast path, so this never decodes the
    // whole key column.
    let key_mask = kernels::cmp::eq(&key_col, &StringArray::new_scalar(key))?;
    let values = AttrValueCols::try_new(attr_batch)?;
    for i in 0..attr_batch.num_rows() {
        if !parent_id_col.is_valid(i) || !(key_mask.is_valid(i) && key_mask.value(i)) {
            continue;
        }
        if let Some(v) = read(&values, i, keep_nulls) {
            out.insert(parent_id_col.value(i), v);
        }
    }
    Ok(out)
}

/// Map each `(parent_id, element index)` to the typed value of the flattened
/// array-element attribute `<prefix><index><suffix>`.
///
/// OTAP has no array attributes; `Value::append_to` flattens them into dotted
/// keys, so element 0 of `interfaces` is stored as `interfaces.0.<field>`, …
/// With `prefix = "interfaces."` and `suffix = ".ifIndex"` this reads that one
/// field of every element. Gaps survive because the index is read from the
/// key, not assigned by position. `keep_nulls` behaves as in
/// [`parent_id_typed_values`].
pub(crate) fn parent_id_element_values<'a>(
    attr_batch: &'a RecordBatch,
    prefix: &str,
    suffix: &str,
    keep_nulls: bool,
) -> Result<FxHashMap<(u16, u32), Value<'a>>> {
    let mut out = FxHashMap::default();
    if attr_batch.num_rows() == 0 {
        return Ok(out);
    }
    let (Some(parent_id_col), Some(key_col)) = (
        parent_ids_u16(attr_batch)?,
        attr_batch.column_by_name(consts::ATTRIBUTE_KEY),
    ) else {
        return Ok(out);
    };
    let indices = element_indices(key_col, prefix, suffix)?;
    let values = AttrValueCols::try_new(attr_batch)?;
    for (i, index) in indices.iter().enumerate() {
        let (true, Some(index)) = (parent_id_col.is_valid(i), *index) else {
            continue;
        };
        if let Some(v) = read(&values, i, keep_nulls) {
            out.insert((parent_id_col.value(i), index), v);
        }
    }
    Ok(out)
}

/// Per-row element index for keys shaped `<prefix><decimal><suffix>`, `None`
/// where the key does not have that shape. The `key` column is
/// dictionary-encoded in practice, so the parse runs once per distinct value
/// and is spread back over the rows — the trick [`regex_mask`] uses.
fn element_indices(key_col: &ArrayRef, prefix: &str, suffix: &str) -> Result<Vec<Option<u32>>> {
    let parse = |key: &str| -> Option<u32> {
        let middle = key.strip_prefix(prefix)?.strip_suffix(suffix)?;
        // The middle is one nonnegative decimal index. Reject an empty middle
        // and anything non-numeric so `a.` / `.b` does not swallow `a.b.c`;
        // leading zeros are accepted and normalize to the same index.
        if middle.is_empty() || !middle.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        middle.parse::<u32>().ok()
    };
    match key_col.data_type() {
        DataType::Dictionary(_, value_type) if **value_type == DataType::Utf8 => {
            let dict = key_col.as_any_dictionary();
            let dict_values = dict.values().as_string::<i32>();
            let per_value: Vec<Option<u32>> = (0..dict_values.len())
                .map(|i| {
                    dict_values
                        .is_valid(i)
                        .then(|| parse(dict_values.value(i)))
                        .flatten()
                })
                .collect();
            let nulls = key_col.logical_nulls();
            Ok(dict
                .normalized_keys()
                .into_iter()
                .enumerate()
                .map(|(row, k)| {
                    if nulls.as_ref().is_some_and(|n| n.is_null(row)) {
                        return None;
                    }
                    per_value.get(k).copied().flatten()
                })
                .collect())
        }
        _ => {
            let col = StrCol::try_new(key_col.as_ref())?;
            Ok((0..key_col.len())
                .map(|i| col.is_valid(i).then(|| parse(col.value(i))).flatten())
                .collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AttrLocation, IntCol, StrCol, attribute_filter_mask, get_mask_by_id, get_record,
        get_record_attr, parent_id_values, struct_id_column,
    };
    use crate::config::processor::{
        AttributeFilter, AttributeFilterExpr, AttributeTable, KeyValueOp, Value as CfgValue,
    };
    use crate::payload::Payload;
    use arrow::array::{
        ArrayRef, BooleanArray, DictionaryArray, Int64Array, StringArray, UInt8Array, UInt16Array,
    };
    use arrow::datatypes::{UInt8Type, UInt16Type};
    use otel_arrow_dfe_pdata::encode::encode_logs_otap_batch;
    use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::any_value::Value;
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::{AnyValue, KeyValue};
    use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
        LogRecord, LogsData, ResourceLogs, ScopeLogs,
    };
    use otel_arrow_dfe_pdata::proto::opentelemetry::resource::v1::Resource;
    use otel_arrow_dfe_pdata::schema::consts;
    use rustc_hash::FxHashSet;
    use std::sync::Arc;

    fn kv(key: &str, value: Value) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue { value: Some(value) }),
        }
    }

    fn record(attrs: Vec<KeyValue>) -> LogRecord {
        LogRecord {
            time_unix_nano: 1,
            attributes: attrs,
            ..Default::default()
        }
    }

    /// A logs payload with a single resourceless/scopeless ScopeLogs carrying the
    /// given records. Records with attributes get a log `id` assigned (0, 1, …).
    fn logs_payload(records: Vec<LogRecord>) -> Payload {
        let req = LogsData {
            resource_logs: vec![ResourceLogs {
                resource: None,
                scope_logs: vec![ScopeLogs {
                    scope: None,
                    log_records: records,
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        Payload::new(encode_logs_otap_batch(&req).unwrap())
    }

    fn logs_payload_with_resource(res_attrs: Vec<KeyValue>, records: Vec<LogRecord>) -> Payload {
        let req = LogsData {
            resource_logs: vec![ResourceLogs {
                resource: Some(Resource {
                    attributes: res_attrs,
                    ..Default::default()
                }),
                scope_logs: vec![ScopeLogs {
                    scope: None,
                    log_records: records,
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        Payload::new(encode_logs_otap_batch(&req).unwrap())
    }

    #[test]
    fn shared_parent_ids_require_a_present_owner_and_id() {
        use arrow::array::{AsArray, RecordBatch, StructArray};
        use arrow::buffer::NullBuffer;
        for (loc, name) in [
            (AttrLocation::Resource, consts::RESOURCE),
            (AttrLocation::Scope, consts::SCOPE),
        ] {
            let original = logs_payload(vec![record(vec![])]);
            assert_eq!(
                super::location_parent_ids(&original, loc).unwrap(),
                vec![Some(0)]
            );
            let root = get_record(&original).unwrap();
            let index = root.schema().index_of(name).unwrap();
            let owner = root.column(index).as_struct();
            let id_index = owner
                .fields()
                .iter()
                .position(|f| f.name() == consts::ID)
                .unwrap();
            for null_struct in [false, true] {
                let mut children = owner.columns().to_vec();
                children[id_index] = Arc::new(UInt16Array::from(vec![null_struct.then_some(0)]));
                let mut columns = root.columns().to_vec();
                columns[index] = Arc::new(StructArray::new(
                    owner.fields().clone(),
                    children,
                    null_struct.then(|| NullBuffer::from(vec![false])),
                ));
                let mut pl = original.clone();
                pl.data
                    .set(
                        ArrowPayloadType::Logs,
                        RecordBatch::try_new(root.schema(), columns).unwrap(),
                    )
                    .unwrap();
                assert_eq!(super::location_parent_ids(&pl, loc).unwrap(), vec![None]);
            }
            let mut pl = original.clone();
            let indices: Vec<_> = (0..root.num_columns()).filter(|i| *i != index).collect();
            pl.data
                .set(ArrowPayloadType::Logs, root.project(&indices).unwrap())
                .unwrap();
            assert_eq!(super::location_parent_ids(&pl, loc).unwrap(), vec![None]);
        }
    }

    // --- IntCol: must read every physical layout the OTAP encoder may emit for an
    // int column. The Dict8/Dict16 arms are the regression guard for the shard bug
    // where a dictionary-encoded `attribute.int` column read back as all-None. ---

    #[test]
    fn int_col_reads_plain() {
        let arr = Int64Array::from(vec![Some(5), None, Some(-3)]);
        let col = IntCol::try_new(&arr).expect("plain int");
        assert!(matches!(col, IntCol::Plain(_)));
        assert!(col.is_valid(0));
        assert_eq!(col.value(0), 5);
        assert!(!col.is_valid(1));
        assert!(col.is_valid(2));
        assert_eq!(col.value(2), -3);
    }

    #[test]
    fn int_col_reads_dict8() {
        let values = Arc::new(Int64Array::from(vec![7_i64, 9])) as ArrayRef;
        let keys = UInt8Array::from(vec![Some(0u8), None, Some(1u8)]);
        let dict = DictionaryArray::<UInt8Type>::new(keys, values);
        let col = IntCol::try_new(&dict).expect("dict8 int");
        assert!(matches!(col, IntCol::Dict8(_)));
        assert!(col.is_valid(0));
        assert_eq!(col.value(0), 7);
        assert!(!col.is_valid(1));
        assert_eq!(col.value(2), 9);
    }

    #[test]
    fn int_col_reads_dict16() {
        let values = Arc::new(Int64Array::from(vec![100_i64, 200])) as ArrayRef;
        let keys = UInt16Array::from(vec![Some(1u16), Some(0u16)]);
        let dict = DictionaryArray::<UInt16Type>::new(keys, values);
        let col = IntCol::try_new(&dict).expect("dict16 int");
        assert!(matches!(col, IntCol::Dict16(_)));
        assert_eq!(col.value(0), 200);
        assert_eq!(col.value(1), 100);
    }

    #[test]
    fn int_col_rejects_non_int() {
        let arr = StringArray::from(vec!["x"]);
        assert!(IntCol::try_new(&arr).is_err());
    }

    #[test]
    fn str_col_reads_dict8() {
        let values = Arc::new(StringArray::from(vec!["a", "b"])) as ArrayRef;
        let keys = UInt8Array::from(vec![Some(1u8), Some(0u8)]);
        let dict = DictionaryArray::<UInt8Type>::new(keys, values);
        let col = StrCol::try_new(&dict).expect("dict8 str");
        assert!(matches!(col, StrCol::Dict8(_)));
        assert_eq!(col.value(0), "b");
        assert_eq!(col.value(1), "a");
    }

    // ---- parent_id_values ----

    #[test]
    fn parent_id_values_reads_string() {
        let pl = logs_payload(vec![record(vec![kv("k", Value::StringValue("v".into()))])]);
        let attr = get_record_attr(&pl).expect("log attrs present");
        let m = parent_id_values(attr, "k").unwrap();
        assert_eq!(m.get(&0).map(|s| s.as_str()), Some("v"));
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn parent_id_element_values_reads_one_field_of_every_element() {
        let pl = logs_payload(vec![
            record(vec![
                kv("if.0.name", Value::StringValue("eth0".into())),
                kv("if.0.idx", Value::IntValue(1)),
                // gap at 1; leading zeros normalize; non-numeric middles and
                // other fields are not elements of this reader
                kv("if.02.name", Value::StringValue("eth2".into())),
                kv("if.x.name", Value::StringValue("nope".into())),
                kv("if.name", Value::StringValue("nope".into())),
                kv("if.3.name.extra", Value::StringValue("nope".into())),
            ]),
            record(vec![kv("if.7.name", Value::StringValue("eth7".into()))]),
        ]);
        let attr = get_record_attr(&pl).expect("log attrs present");
        let m = super::parent_id_element_values(attr, "if.", ".name", false).unwrap();
        let text = |parent: u16, index: u32| match m.get(&(parent, index)) {
            Some(super::super::Value::String(s)) => Some(s.as_str().to_owned()),
            _ => None,
        };
        assert_eq!(text(0, 0).as_deref(), Some("eth0"));
        assert_eq!(text(0, 2).as_deref(), Some("eth2"));
        assert_eq!(text(1, 7).as_deref(), Some("eth7"));
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn parent_id_values_reads_int_as_decimal() {
        let pl = logs_payload(vec![
            record(vec![kv("shard.id", Value::IntValue(7))]),
            record(vec![kv("shard.id", Value::IntValue(42))]),
        ]);
        let attr = get_record_attr(&pl).expect("log attrs present");
        let m = parent_id_values(attr, "shard.id").unwrap();
        // ints render as their decimal string form, keyed by the record's log id.
        assert_eq!(m.get(&0).map(|s| s.as_str()), Some("7"));
        assert_eq!(m.get(&1).map(|s| s.as_str()), Some("42"));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn parent_id_values_skips_non_str_int_types() {
        // A double-valued attribute is not shardable → absent from the map.
        let pl = logs_payload(vec![record(vec![kv("d", Value::DoubleValue(1.5))])]);
        let attr = get_record_attr(&pl).expect("log attrs present");
        let m = parent_id_values(attr, "d").unwrap();
        assert!(m.is_empty(), "double values must not be read: {m:?}");
    }

    #[test]
    fn parent_id_values_absent_key_is_empty() {
        let pl = logs_payload(vec![record(vec![kv(
            "present",
            Value::StringValue("x".into()),
        )])]);
        let attr = get_record_attr(&pl).expect("log attrs present");
        assert!(parent_id_values(attr, "absent").unwrap().is_empty());
    }

    #[test]
    fn parent_id_values_last_duplicate_key_wins() {
        // One record carrying `k` twice: the later row overwrites the earlier.
        let pl = logs_payload(vec![record(vec![
            kv("k", Value::StringValue("first".into())),
            kv("k", Value::StringValue("second".into())),
        ])]);
        let attr = get_record_attr(&pl).expect("log attrs present");
        let m = parent_id_values(attr, "k").unwrap();
        assert_eq!(m.get(&0).map(|s| s.as_str()), Some("second"));
    }

    #[test]
    fn parent_id_values_reads_resource_attr() {
        let pl = logs_payload_with_resource(
            vec![kv("host.name", Value::StringValue("h1".into()))],
            vec![record(vec![])],
        );
        let attr = pl
            .data
            .get(ArrowPayloadType::ResourceAttrs)
            .expect("resource attrs present");
        let m = parent_id_values(attr, "host.name").unwrap();
        assert_eq!(m.get(&0).map(|s| s.as_str()), Some("h1"));
    }

    // ---- get_mask_by_id / struct_id_column ----

    #[test]
    fn get_mask_by_id_selects_matching_records() {
        let pl = logs_payload(vec![
            record(vec![kv("a", Value::StringValue("x".into()))]), // id 0
            record(vec![kv("a", Value::StringValue("y".into()))]), // id 1
            record(vec![kv("a", Value::StringValue("z".into()))]), // id 2
        ]);
        let id_set: FxHashSet<u16> = [0u16, 2].into_iter().collect();
        let mask = get_mask_by_id(&pl, AttrLocation::Record, &id_set).unwrap();
        assert_eq!(mask, BooleanArray::from(vec![true, false, true]));
    }

    #[test]
    fn struct_id_column_present_for_resource() {
        let pl = logs_payload_with_resource(
            vec![kv("host.name", Value::StringValue("h".into()))],
            vec![record(vec![])],
        );
        let rec = get_record(&pl).expect("root record");
        assert!(
            struct_id_column(rec, consts::RESOURCE).is_some(),
            "resource struct should expose an id column"
        );
    }

    // ---- attribute_filter_mask ----

    fn match_expr(
        table: AttributeTable,
        key: &str,
        op: KeyValueOp,
        value: CfgValue,
    ) -> AttributeFilter {
        AttributeFilter::Match(AttributeFilterExpr {
            table,
            key: key.to_string(),
            key_is_regex: false,
            op,
            value,
        })
    }

    #[test]
    fn filter_mask_record_string_eq() {
        let pl = logs_payload(vec![
            record(vec![kv("env", Value::StringValue("prod".into()))]), // id 0
            record(vec![kv("env", Value::StringValue("dev".into()))]),  // id 1
            record(vec![kv("env", Value::StringValue("prod".into()))]), // id 2
        ]);
        let filter = match_expr(
            AttributeTable::Record,
            "env",
            KeyValueOp::Equal,
            CfgValue::String("prod".into()),
        );
        let mask = attribute_filter_mask(&pl, &filter).unwrap();
        assert_eq!(mask, BooleanArray::from(vec![true, false, true]));
    }

    #[test]
    fn filter_mask_record_string_not_eq() {
        let pl = logs_payload(vec![
            record(vec![kv("env", Value::StringValue("prod".into()))]),
            record(vec![kv("env", Value::StringValue("dev".into()))]),
        ]);
        let filter = match_expr(
            AttributeTable::Record,
            "env",
            KeyValueOp::NotEqual,
            CfgValue::String("prod".into()),
        );
        let mask = attribute_filter_mask(&pl, &filter).unwrap();
        assert_eq!(mask, BooleanArray::from(vec![false, true]));
    }

    #[test]
    fn filter_mask_record_regex_value() {
        let pl = logs_payload(vec![
            record(vec![kv("host", Value::StringValue("web-01".into()))]),
            record(vec![kv("host", Value::StringValue("db-01".into()))]),
        ]);
        let filter = AttributeFilter::Match(AttributeFilterExpr {
            table: AttributeTable::Record,
            key: "host".to_string(),
            key_is_regex: false,
            op: KeyValueOp::RegexEqual,
            value: CfgValue::String("^web-".into()),
        });
        let mask = attribute_filter_mask(&pl, &filter).unwrap();
        assert_eq!(mask, BooleanArray::from(vec![true, false]));
    }

    #[test]
    fn filter_mask_int_greater_than_or_equal() {
        let pl = logs_payload(vec![
            record(vec![kv("code", Value::IntValue(500))]),
            record(vec![kv("code", Value::IntValue(200))]),
            record(vec![kv("code", Value::IntValue(404))]),
        ]);
        let filter = match_expr(
            AttributeTable::Record,
            "code",
            KeyValueOp::GreaterThanOrEqual,
            CfgValue::Integer(400),
        );
        let mask = attribute_filter_mask(&pl, &filter).unwrap();
        assert_eq!(mask, BooleanArray::from(vec![true, false, true]));
    }

    #[test]
    fn filter_mask_resource_table() {
        // Both records hang off the single resource (parent_id 0), so a resource-table
        // match selects every record under it.
        let pl = logs_payload_with_resource(
            vec![kv("host.name", Value::StringValue("h1".into()))],
            vec![record(vec![]), record(vec![])],
        );
        let filter = match_expr(
            AttributeTable::Resource,
            "host.name",
            KeyValueOp::Equal,
            CfgValue::String("h1".into()),
        );
        let mask = attribute_filter_mask(&pl, &filter).unwrap();
        assert_eq!(mask, BooleanArray::from(vec![true, true]));
    }

    #[test]
    fn filter_mask_and_combines() {
        let pl = logs_payload(vec![
            record(vec![
                kv("env", Value::StringValue("prod".into())),
                kv("tier", Value::StringValue("web".into())),
            ]), // id 0
            record(vec![
                kv("env", Value::StringValue("prod".into())),
                kv("tier", Value::StringValue("db".into())),
            ]), // id 1
        ]);
        let filter = AttributeFilter::And(vec![
            match_expr(
                AttributeTable::Record,
                "env",
                KeyValueOp::Equal,
                CfgValue::String("prod".into()),
            ),
            match_expr(
                AttributeTable::Record,
                "tier",
                KeyValueOp::Equal,
                CfgValue::String("web".into()),
            ),
        ]);
        let mask = attribute_filter_mask(&pl, &filter).unwrap();
        assert_eq!(mask, BooleanArray::from(vec![true, false]));
    }

    #[test]
    fn filter_mask_or_combines() {
        let pl = logs_payload(vec![
            record(vec![kv("env", Value::StringValue("prod".into()))]),
            record(vec![kv("env", Value::StringValue("dev".into()))]),
            record(vec![kv("env", Value::StringValue("stage".into()))]),
        ]);
        let filter = AttributeFilter::Or(vec![
            match_expr(
                AttributeTable::Record,
                "env",
                KeyValueOp::Equal,
                CfgValue::String("prod".into()),
            ),
            match_expr(
                AttributeTable::Record,
                "env",
                KeyValueOp::Equal,
                CfgValue::String("stage".into()),
            ),
        ]);
        let mask = attribute_filter_mask(&pl, &filter).unwrap();
        assert_eq!(mask, BooleanArray::from(vec![true, false, true]));
    }

    #[test]
    fn filter_mask_missing_attr_is_all_false() {
        // Records carry no attributes → a record-table match selects nothing.
        let pl = logs_payload(vec![record(vec![]), record(vec![])]);
        let filter = match_expr(
            AttributeTable::Record,
            "env",
            KeyValueOp::Equal,
            CfgValue::String("prod".into()),
        );
        let mask = attribute_filter_mask(&pl, &filter).unwrap();
        assert_eq!(mask, BooleanArray::from(vec![false, false]));
    }

    // ---- metrics: masks are sized by root rows, not items ----

    /// Two gauge metric rows (`k = a` / `k = b`) carrying `dps_per_row` data
    /// points each, so `num_items()` is a multiple of the root row count.
    fn multi_dp_metrics(dps_per_row: usize) -> Payload {
        use otel_arrow_dfe_pdata::encode::encode_metrics_otap_batch;
        use otel_arrow_dfe_pdata::proto::opentelemetry::metrics::v1::{
            Gauge, Metric, MetricsData, NumberDataPoint, ResourceMetrics, ScopeMetrics, metric,
            number_data_point,
        };

        let gauge = |name: &str, tag: &str| Metric {
            name: name.to_string(),
            description: String::new(),
            unit: String::new(),
            metadata: vec![],
            data: Some(metric::Data::Gauge(Gauge {
                data_points: (0..dps_per_row)
                    .map(|i| NumberDataPoint {
                        attributes: vec![kv("k", Value::StringValue(tag.into()))],
                        start_time_unix_nano: 0,
                        time_unix_nano: i as u64 + 1,
                        exemplars: vec![],
                        flags: 0,
                        value: Some(number_data_point::Value::AsInt(i as i64)),
                    })
                    .collect(),
            })),
        };
        let data = MetricsData {
            resource_metrics: vec![ResourceMetrics {
                resource: None,
                scope_metrics: vec![ScopeMetrics {
                    scope: None,
                    metrics: vec![gauge("m_a", "a"), gauge("m_b", "b")],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        Payload::new(encode_metrics_otap_batch(&data).unwrap())
    }

    /// The mask must be as long as the root (UnivariateMetrics) batch. Sizing it
    /// by `num_items()` overruns that batch once a metric row holds more than one
    /// data point, and `filter_otap_batch` rejects (or overruns) the result.
    #[test]
    fn masks_over_metrics_are_root_row_length() {
        let pl = multi_dp_metrics(4);
        assert_eq!(pl.num_rows(), 2, "two metric rows");
        assert_eq!(pl.num_items(), 8, "eight data points");

        let id_set: FxHashSet<u16> = [0u16].into_iter().collect();
        let mask = get_mask_by_id(&pl, AttrLocation::Record, &id_set).unwrap();
        assert_eq!(mask.len(), pl.num_rows());
        assert_eq!(mask, BooleanArray::from(vec![true, false]));

        // Same for the filter path, including the empty-AND / missing-attr arms
        // that synthesise a constant mask.
        let all = attribute_filter_mask(&pl, &AttributeFilter::And(vec![])).unwrap();
        assert_eq!(all, BooleanArray::from(vec![true, true]));
        let none = attribute_filter_mask(&pl, &AttributeFilter::Or(vec![])).unwrap();
        assert_eq!(none, BooleanArray::from(vec![false, false]));
        let missing = attribute_filter_mask(
            &pl,
            &match_expr(
                AttributeTable::Record,
                "absent",
                KeyValueOp::Equal,
                CfgValue::String("x".into()),
            ),
        )
        .unwrap();
        assert_eq!(missing, BooleanArray::from(vec![false, false]));
    }

    /// A metric row with no data points has `num_items() == 0` but is still a row
    /// that has to be masked.
    #[test]
    fn masks_over_metrics_without_data_points() {
        let pl = multi_dp_metrics(0);
        assert_eq!(pl.num_rows(), 2);
        assert_eq!(pl.num_items(), 0);

        let mask = get_mask_by_id(&pl, AttrLocation::Record, &FxHashSet::default()).unwrap();
        assert_eq!(mask, BooleanArray::from(vec![false, false]));
    }
}
