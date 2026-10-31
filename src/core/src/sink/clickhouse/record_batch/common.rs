// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Column-level helpers shared by the logs, traces and metrics builders.
//!
//! Everything here takes an `Option<&ArrayRef>` because OTAP is column-sparse: a column
//! whose values are all null or default is omitted from the encoded batch entirely. Every
//! helper therefore turns "absent or null" into the proto default for the type, so the
//! arrays handed to `RecordBatch::try_new` are null-free — which is what the non-`Nullable`
//! ClickHouse columns need.
//!
//! The work is deliberately whole-column (`cast`, `take`, `zip`) rather than row-wise;
//! `hex_ids` is the one unavoidable exception, since ClickHouse stores trace and span ids as
//! hex text while OTAP carries raw bytes.

use anyhow::{Result, anyhow};
use arrow::array::{
    Array, ArrayRef, AsArray, BooleanArray, Float64Array, Int32Array, ListArray, MapArray,
    PrimitiveArray, RecordBatch, Scalar, StringArray, StringBuilder, StructArray, UInt8Array,
    UInt32Array, UInt64Array, new_empty_array, new_null_array,
};
use arrow::buffer::OffsetBuffer;
use arrow::compute::kernels::{boolean, cast::cast, concat::concat, take::take, zip::zip};
use arrow::datatypes::{
    ArrowPrimitiveType, DataType, Field, Float64Type, Int32Type, TimeUnit, UInt8Type, UInt32Type,
    UInt64Type,
};
use std::sync::Arc;

/// Fetch a top-level column by name.
pub(super) fn col<'a>(batch: &'a RecordBatch, name: &str) -> Option<&'a ArrayRef> {
    batch.column_by_name(name)
}

/// Fetch `<struct_name>.<field>` from a struct column (e.g. `resource.schema_url`).
///
/// Returns `None` when either the struct or the field was omitted, which OTAP does freely.
pub(super) fn struct_field(
    batch: &RecordBatch,
    struct_name: &str,
    field: &str,
) -> Option<ArrayRef> {
    let s: &StructArray = batch.column_by_name(struct_name)?.as_struct_opt()?;
    s.column_by_name(field).cloned()
}

/// Decode a dictionary column to its value type; pass anything else through untouched.
///
/// `take(values, keys)` is the universal unpack — unlike `cast`, it works for every value
/// type OTAP dictionary-encodes, including `FixedSizeBinary` ids and `Duration`.
pub(super) fn undict(array: &ArrayRef) -> Result<ArrayRef> {
    match array.data_type() {
        DataType::Dictionary(_, _) => {
            let dict = array
                .as_any_dictionary_opt()
                .ok_or_else(|| anyhow!("column claims Dictionary but does not downcast"))?;
            // `take` panics on an empty values array, which a fully-null dictionary can have.
            if dict.values().is_empty() {
                return Ok(new_null_array(dict.values().data_type(), array.len()));
            }
            Ok(take(dict.values().as_ref(), dict.keys(), None)?)
        }
        _ => Ok(array.clone()),
    }
}

/// Replace nulls with `default`. No-op (and no copy) when the column has none.
fn fill_null(array: ArrayRef, default: &dyn arrow::array::Datum) -> Result<ArrayRef> {
    if array.null_count() == 0 {
        return Ok(array);
    }
    let valid = boolean::is_not_null(array.as_ref())?;
    Ok(zip(&valid, &array, default)?)
}

/// A non-null `Utf8` column: dictionaries decoded, nulls and absence rendered as `""`.
pub(super) fn utf8_or_empty(array: Option<&ArrayRef>, len: usize) -> Result<ArrayRef> {
    let Some(array) = array else {
        return Ok(Arc::new(StringArray::from(vec![""; len])));
    };
    let plain = undict(array)?;
    let utf8 = if matches!(plain.data_type(), DataType::Utf8) {
        plain
    } else {
        cast(plain.as_ref(), &DataType::Utf8)?
    };
    fill_null(utf8, &Scalar::new(StringArray::from(vec![""])))
}

/// A non-null primitive column of `T`, with `default` for nulls and absence.
pub(super) fn prim_or_default<T: ArrowPrimitiveType>(
    array: Option<&ArrayRef>,
    len: usize,
    default: T::Native,
) -> Result<ArrayRef>
where
    PrimitiveArray<T>: From<Vec<T::Native>>,
{
    let scalar = Scalar::new(PrimitiveArray::<T>::from(vec![default]));
    let Some(array) = array else {
        return Ok(Arc::new(PrimitiveArray::<T>::from(vec![default; len])));
    };
    let plain = undict(array)?;
    let target = T::DATA_TYPE;
    let converted = if plain.data_type() == &target {
        plain
    } else {
        cast(plain.as_ref(), &target)?
    };
    fill_null(converted, &scalar)
}

/// A non-null `Boolean` column, `false` for nulls and absence.
pub(super) fn bool_or_false(array: Option<&ArrayRef>, len: usize) -> Result<ArrayRef> {
    let Some(array) = array else {
        return Ok(Arc::new(BooleanArray::from(vec![false; len])));
    };
    let plain = undict(array)?;
    let b = if matches!(plain.data_type(), DataType::Boolean) {
        plain
    } else {
        cast(plain.as_ref(), &DataType::Boolean)?
    };
    fill_null(b, &Scalar::new(BooleanArray::from(vec![false])))
}

/// A non-null `Timestamp(Nanosecond)` column — the `DateTime64(9)` columns.
pub(super) fn ts_nanos(array: Option<&ArrayRef>, len: usize) -> Result<ArrayRef> {
    ts(array, len, TimeUnit::Nanosecond)
}

/// A non-null `Timestamp(Second)` column — every metric `DateTime` column.
///
/// OTAP timestamps are nanoseconds, so this narrows; sub-second precision is lost, which is
/// what the upstream exporter's schema chose.
pub(super) fn ts_secs(array: Option<&ArrayRef>, len: usize) -> Result<ArrayRef> {
    ts(array, len, TimeUnit::Second)
}

fn ts(array: Option<&ArrayRef>, len: usize, unit: TimeUnit) -> Result<ArrayRef> {
    let target = DataType::Timestamp(unit, None);
    let zero = cast(
        &PrimitiveArray::<arrow::datatypes::Int64Type>::from(vec![0i64]),
        &target,
    )?;
    let Some(array) = array else {
        let nulls = new_null_array(&target, len);
        let all_false = BooleanArray::from(vec![false; len]);
        return Ok(zip(&all_false, &nulls, &Scalar::new(zero))?);
    };
    let plain = undict(array)?;
    let converted = if plain.data_type() == &target {
        plain
    } else {
        cast(plain.as_ref(), &target)?
    };
    fill_null(converted, &Scalar::new(zero))
}

/// `Duration(Nanosecond)` (possibly dictionary-encoded) → `UInt64` nanoseconds.
pub(super) fn duration_nanos(array: Option<&ArrayRef>, len: usize) -> Result<ArrayRef> {
    let Some(array) = array else {
        return Ok(Arc::new(UInt64Array::from(vec![0u64; len])));
    };
    let plain = undict(array)?;
    // Durations are physically i64; reinterpret rather than cast, which arrow refuses to do
    // directly from Duration to UInt64.
    let as_i64 = if matches!(plain.data_type(), DataType::Duration(_)) {
        cast(plain.as_ref(), &DataType::Int64)?
    } else {
        plain
    };
    let u64s = cast(as_i64.as_ref(), &DataType::UInt64)?;
    fill_null(u64s, &Scalar::new(UInt64Array::from(vec![0u64])))
}

/// Raw id bytes → lowercase hex text, the form ClickHouse's `TraceId`/`SpanId` columns use.
///
/// Handles `FixedSizeBinary` and `Binary`, dictionary-encoded or not. Nulls and absence
/// render as `""`.
pub(super) fn hex_ids(array: Option<&ArrayRef>, len: usize) -> Result<ArrayRef> {
    let Some(array) = array else {
        return Ok(Arc::new(StringArray::from(vec![""; len])));
    };
    let plain = undict(array)?;
    // Ids are 8 or 16 bytes, so hex always fits this scratch buffer.
    let mut scratch = [0u8; 64];
    let mut out = StringBuilder::with_capacity(len, len * 32);

    let mut push = |bytes: Option<&[u8]>, out: &mut StringBuilder| {
        match bytes {
            Some(b) => {
                let n = b.len() * 2;
                if n <= scratch.len() && const_hex::encode_to_slice(b, &mut scratch[..n]).is_ok() {
                    // encode_to_slice writes lowercase-hex ASCII, so this is always valid UTF-8.
                    out.append_value(core::str::from_utf8(&scratch[..n]).unwrap_or(""));
                } else {
                    out.append_value(const_hex::encode(b));
                }
            }
            None => out.append_value(""),
        }
    };

    match plain.data_type() {
        DataType::FixedSizeBinary(_) => {
            let a = plain.as_fixed_size_binary();
            for i in 0..a.len() {
                push(a.is_valid(i).then(|| a.value(i)), &mut out);
            }
        }
        DataType::Binary => {
            let a = plain.as_binary::<i32>();
            for i in 0..a.len() {
                push(a.is_valid(i).then(|| a.value(i)), &mut out);
            }
        }
        DataType::LargeBinary => {
            let a = plain.as_binary::<i64>();
            for i in 0..a.len() {
                push(a.is_valid(i).then(|| a.value(i)), &mut out);
            }
        }
        // Already text (or something unexpected): fall back to the string path rather than
        // dropping the column.
        _ => return utf8_or_empty(Some(&plain), len),
    }
    Ok(Arc::new(out.finish()))
}

/// Map a `Dictionary(_, Int32)` enum column to text.
///
/// Used for `SpanKind` and `StatusCode`, which ClickHouse stores as strings.
///
/// A null or omitted column means the proto default — enum value 0 — not "no value": a span
/// with no `Status` message still has code `Unset`, and that is what the Go exporter writes.
/// So nulls render as `name_of(0)`. Only a value outside the enum falls back to `""`, which
/// is `name_of`'s own decision.
pub(super) fn enum_to_string(
    array: Option<&ArrayRef>,
    len: usize,
    name_of: fn(i32) -> &'static str,
) -> Result<ArrayRef> {
    let Some(array) = array else {
        return Ok(Arc::new(StringArray::from(vec![name_of(0); len])));
    };
    let plain = undict(array)?;
    let ints = cast(plain.as_ref(), &DataType::Int32)?;
    let ints: &Int32Array = ints.as_primitive::<Int32Type>();
    let mut out = StringBuilder::with_capacity(len, len * 8);
    for i in 0..ints.len() {
        let v = if ints.is_valid(i) { ints.value(i) } else { 0 };
        out.append_value(name_of(v));
    }
    Ok(Arc::new(out.finish()))
}

/// Pull one key's value out of every row of a `Map<Utf8, Utf8>` column.
///
/// This is how `ServiceName` is derived from `ResourceAttributes['service.name']`, mirroring
/// the upstream exporter. Rows without the key yield `""`.
pub(super) fn map_value(map: &MapArray, key: &str) -> Result<ArrayRef> {
    let keys: &StringArray = map
        .keys()
        .as_string_opt::<i32>()
        .ok_or_else(|| anyhow!("attribute map keys are not Utf8"))?;
    let values: &StringArray = map
        .values()
        .as_string_opt::<i32>()
        .ok_or_else(|| anyhow!("attribute map values are not Utf8"))?;
    let offsets = map.offsets();

    let mut out = StringBuilder::with_capacity(map.len(), map.len() * 16);
    for row in 0..map.len() {
        let (start, end) = (offsets[row] as usize, offsets[row + 1] as usize);
        let found = (start..end).find(|&i| keys.is_valid(i) && keys.value(i) == key);
        match found {
            Some(i) if values.is_valid(i) => out.append_value(values.value(i)),
            _ => out.append_value(""),
        }
    }
    Ok(Arc::new(out.finish()))
}

/// Merge OTAP's `int_value` / `double_value` pair into the single `Float64` ClickHouse
/// stores, preferring the double when both are set. Absent or null on both sides → `0.0`.
pub(super) fn number_value(
    int_value: Option<&ArrayRef>,
    double_value: Option<&ArrayRef>,
    len: usize,
) -> Result<ArrayRef> {
    let from_int = match int_value {
        Some(a) => cast(undict(a)?.as_ref(), &DataType::Float64)?,
        None => new_null_array(&DataType::Float64, len),
    };
    let Some(double_value) = double_value else {
        return fill_null(from_int, &Scalar::new(Float64Array::from(vec![0.0])));
    };
    let from_double = {
        let plain = undict(double_value)?;
        if matches!(plain.data_type(), DataType::Float64) {
            plain
        } else {
            cast(plain.as_ref(), &DataType::Float64)?
        }
    };
    let has_double = boolean::is_not_null(from_double.as_ref())?;
    let merged = zip(&has_double, &from_double, &from_int)?;
    fill_null(merged, &Scalar::new(Float64Array::from(vec![0.0])))
}

/// A non-null `List<item>` column: nulls and absence become empty lists.
///
/// ClickHouse's `Array(...)` columns are not `Nullable`, and the field name has to be arrow's
/// conventional `item` to match the declared schema — OTAP's own list fields do not always
/// agree — so this rebuilds the list rather than passing it through.
pub(super) fn list_or_empty(
    array: Option<&ArrayRef>,
    len: usize,
    item: &DataType,
) -> Result<ArrayRef> {
    let field = Arc::new(Field::new("item", item.clone(), true));
    let empty = |rows: usize| -> ArrayRef {
        Arc::new(ListArray::new(
            field.clone(),
            OffsetBuffer::new(vec![0i32; rows + 1].into()),
            new_empty_array(item),
            None,
        ))
    };

    let Some(array) = array else {
        return Ok(empty(len));
    };
    let list = array
        .as_list_opt::<i32>()
        .ok_or_else(|| anyhow!("expected a List column, found {:?}", array.data_type()))?;

    let values = if list.values().data_type() == item {
        list.values().clone()
    } else {
        cast(list.values().as_ref(), item)?
    };
    let rebuilt = ListArray::new(
        field.clone(),
        list.offsets().clone(),
        values,
        list.nulls().cloned(),
    );
    if rebuilt.null_count() == 0 {
        return Ok(Arc::new(rebuilt));
    }

    // Null rows must become *empty* lists, not null ones. Same trick the attribute remap
    // uses: append a one-row empty list and point every null at it, so one `take` does it.
    let sentinel = rebuilt.len() as u32;
    let indices: UInt32Array = (0..rebuilt.len())
        .map(|i| {
            if rebuilt.is_valid(i) {
                i as u32
            } else {
                sentinel
            }
        })
        .collect::<Vec<_>>()
        .into();
    let with_sentinel = concat(&[&rebuilt as &dyn Array, empty(1).as_ref()])?;
    Ok(take(with_sentinel.as_ref(), &indices, None)?)
}

/// Split OTAP's `quantile: List<Struct{quantile, value}>` into the two parallel
/// `Array(Float64)` columns ClickHouse's `ValueAtQuantiles Nested(...)` expands to.
///
/// Both outputs reuse the source's offset buffer and one struct child each, so no element
/// data is copied — only the list envelope is rebuilt.
pub(super) fn split_quantiles(
    array: Option<&ArrayRef>,
    len: usize,
    quantile_field: &str,
    value_field: &str,
) -> Result<(ArrayRef, ArrayRef)> {
    let f64_list = |a: Option<&ArrayRef>| list_or_empty(a, len, &DataType::Float64);

    let Some(array) = array else {
        return Ok((f64_list(None)?, f64_list(None)?));
    };
    let list = array
        .as_list_opt::<i32>()
        .ok_or_else(|| anyhow!("quantile column is not a List"))?;
    let Some(entries) = list.values().as_struct_opt() else {
        return Ok((f64_list(None)?, f64_list(None)?));
    };

    let child = |name: &str| -> Result<ArrayRef> {
        let values = match entries.column_by_name(name) {
            Some(c) => prim_or_default::<Float64Type>(Some(c), entries.len(), 0.0)?,
            None => Arc::new(Float64Array::from(vec![0.0; entries.len()])) as ArrayRef,
        };
        let rebuilt: ArrayRef = Arc::new(ListArray::new(
            Arc::new(Field::new("item", DataType::Float64, true)),
            list.offsets().clone(),
            values,
            list.nulls().cloned(),
        ));
        f64_list(Some(&rebuilt))
    };

    Ok((child(quantile_field)?, child(value_field)?))
}

/// Convenience wrappers for the primitive widths the ClickHouse schema uses.
pub(super) fn u8_or_zero(array: Option<&ArrayRef>, len: usize) -> Result<ArrayRef> {
    prim_or_default::<UInt8Type>(array, len, 0)
}
pub(super) fn u32_or_zero(array: Option<&ArrayRef>, len: usize) -> Result<ArrayRef> {
    prim_or_default::<UInt32Type>(array, len, 0)
}
pub(super) fn u64_or_zero(array: Option<&ArrayRef>, len: usize) -> Result<ArrayRef> {
    prim_or_default::<UInt64Type>(array, len, 0)
}
pub(super) fn i32_or_zero(array: Option<&ArrayRef>, len: usize) -> Result<ArrayRef> {
    prim_or_default::<Int32Type>(array, len, 0)
}
pub(super) fn f64_or_zero(array: Option<&ArrayRef>, len: usize) -> Result<ArrayRef> {
    prim_or_default::<Float64Type>(array, len, 0.0)
}

/// Read a `UInt8` column as a plain vector, defaulting absent/null entries to 0.
pub(super) fn u8_values(array: Option<&ArrayRef>, len: usize) -> Result<Vec<u8>> {
    let arr = u8_or_zero(array, len)?;
    let arr: &UInt8Array = arr.as_primitive::<UInt8Type>();
    Ok(arr.values().to_vec())
}

/// Read an id column of any integer width as `u64`, defaulting absent/null entries to the
/// row index — the same fallback the JSON serializer uses when OTAP omits `id`.
pub(super) fn ids_as_u64(array: Option<&ArrayRef>, len: usize) -> Result<Vec<u64>> {
    let Some(array) = array else {
        return Ok((0..len as u64).collect());
    };
    let plain = undict(array)?;
    let as_u64 = cast(plain.as_ref(), &DataType::UInt64)?;
    let a: &UInt64Array = as_u64.as_primitive::<UInt64Type>();
    Ok((0..len)
        .map(|i| if a.is_valid(i) { a.value(i) } else { i as u64 })
        .collect())
}

/// Read a `resource.id` / `scope.id` column as `u64`, defaulting absent/null entries to 0.
///
/// Zero is OTAP's convention for "the single, unnumbered resource/scope" — a batch whose
/// rows all share one resource omits the id column entirely, and its `ResourceAttrs` rows
/// carry `parent_id = 0`. The JSON serializer's `ResourceCols::id_at` defaults the same way,
/// so both output paths agree on which attributes belong to which record.
pub(super) fn struct_ids(array: Option<&ArrayRef>, len: usize) -> Result<Vec<Option<u64>>> {
    let Some(array) = array else {
        return Ok(vec![Some(0); len]);
    };
    let plain = undict(array)?;
    let as_u64 = cast(plain.as_ref(), &DataType::UInt64)?;
    let a: &UInt64Array = as_u64.as_primitive::<UInt64Type>();
    Ok((0..len)
        .map(|i| Some(if a.is_valid(i) { a.value(i) } else { 0 }))
        .collect())
}

/// Read a `parent_id` column of any integer width as `u64`. Unlike [`ids_as_u64`] a null
/// here means "no parent", so it is reported as `None` rather than defaulted.
pub(super) fn parent_ids_as_u64(array: Option<&ArrayRef>, len: usize) -> Result<Vec<Option<u64>>> {
    let Some(array) = array else {
        return Ok(vec![None; len]);
    };
    let plain = undict(array)?;
    let as_u64 = cast(plain.as_ref(), &DataType::UInt64)?;
    let a: &UInt64Array = as_u64.as_primitive::<UInt64Type>();
    Ok((0..len)
        .map(|i| a.is_valid(i).then(|| a.value(i)))
        .collect())
}

/// Build a `take` index that broadcasts parent-grain columns down to child grain.
///
/// Returns the gather index alongside a keep-mask that is `false` for child rows whose
/// parent id does not resolve. Unresolvable rows point at index 0 so `take` stays in bounds;
/// the caller filters them out afterwards.
pub(super) fn gather_index(
    parent_ids: &[Option<u64>],
    id_to_row: &rustc_hash::FxHashMap<u64, u32>,
) -> (UInt32Array, BooleanArray) {
    let mut idx = Vec::with_capacity(parent_ids.len());
    let mut keep = Vec::with_capacity(parent_ids.len());
    for pid in parent_ids {
        match pid.and_then(|p| id_to_row.get(&p)) {
            Some(&row) => {
                idx.push(row);
                keep.push(true);
            }
            None => {
                idx.push(0);
                keep.push(false);
            }
        }
    }
    (UInt32Array::from(idx), BooleanArray::from(keep))
}

/// `true` when every row is kept — lets callers skip a needless `filter` pass.
pub(super) fn all_kept(mask: &BooleanArray) -> bool {
    mask.null_count() == 0 && mask.true_count() == mask.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{
        DictionaryArray, FixedSizeBinaryArray, Int64Array, TimestampNanosecondArray,
    };
    use arrow::datatypes::UInt8Type as DictKey;

    fn arr(a: impl Array + 'static) -> ArrayRef {
        Arc::new(a)
    }

    #[test]
    fn undict_unpacks_and_passes_through() {
        let dict: DictionaryArray<DictKey> = vec!["a", "b", "a"].into_iter().collect();
        let out = undict(&arr(dict)).unwrap();
        assert_eq!(out.as_string::<i32>().value(2), "a");

        let plain = arr(StringArray::from(vec!["x"]));
        assert_eq!(undict(&plain).unwrap().as_string::<i32>().value(0), "x");
    }

    #[test]
    fn absent_columns_become_defaults_not_nulls() {
        assert_eq!(utf8_or_empty(None, 3).unwrap().null_count(), 0);
        assert_eq!(u32_or_zero(None, 3).unwrap().null_count(), 0);
        assert_eq!(ts_secs(None, 3).unwrap().null_count(), 0);
        assert_eq!(hex_ids(None, 3).unwrap().null_count(), 0);
        assert_eq!(bool_or_false(None, 3).unwrap().null_count(), 0);

        let s = utf8_or_empty(None, 2).unwrap();
        assert_eq!(s.as_string::<i32>().value(0), "");
    }

    #[test]
    fn nulls_are_filled_with_the_type_default() {
        let a = arr(StringArray::from(vec![Some("x"), None]));
        let out = utf8_or_empty(Some(&a), 2).unwrap();
        assert_eq!(out.null_count(), 0);
        assert_eq!(out.as_string::<i32>().value(1), "");

        let n = arr(Int64Array::from(vec![Some(7), None]));
        let out = u64_or_zero(Some(&n), 2).unwrap();
        assert_eq!(out.as_primitive::<UInt64Type>().values(), &[7, 0]);
    }

    #[test]
    fn hex_ids_render_lowercase_and_default_empty() {
        let ids = FixedSizeBinaryArray::try_from_sparse_iter_with_size(
            vec![Some(vec![0xde, 0xad, 0xbe, 0xef]), None].into_iter(),
            4,
        )
        .unwrap();
        let out = hex_ids(Some(&arr(ids)), 2).unwrap();
        let out = out.as_string::<i32>();
        assert_eq!(out.value(0), "deadbeef");
        assert_eq!(out.value(1), "");
    }

    #[test]
    fn timestamps_narrow_to_seconds() {
        let ns = arr(TimestampNanosecondArray::from(vec![
            Some(1_500_000_000_123_456_789),
            None,
        ]));
        let out = ts_secs(Some(&ns), 2).unwrap();
        assert_eq!(
            out.data_type(),
            &DataType::Timestamp(TimeUnit::Second, None)
        );
        assert_eq!(out.null_count(), 0);
        let v = out.as_primitive::<arrow::datatypes::TimestampSecondType>();
        assert_eq!(v.value(0), 1_500_000_000);
        assert_eq!(v.value(1), 0);
    }

    #[test]
    fn number_value_prefers_double_then_int_then_zero() {
        let ints = arr(Int64Array::from(vec![Some(1), Some(2), None]));
        let doubles = arr(Float64Array::from(vec![Some(9.5), None, None]));
        let out = number_value(Some(&ints), Some(&doubles), 3).unwrap();
        assert_eq!(out.as_primitive::<Float64Type>().values(), &[9.5, 2.0, 0.0]);
    }

    #[test]
    fn gather_index_flags_unresolvable_parents() {
        let mut map = rustc_hash::FxHashMap::default();
        let _ = map.insert(10u64, 0u32);
        let _ = map.insert(20u64, 1u32);
        let (idx, keep) = gather_index(&[Some(20), Some(99), None, Some(10)], &map);
        assert_eq!(idx.values(), &[1, 0, 0, 0]);
        assert_eq!(
            keep.iter().collect::<Vec<_>>(),
            vec![Some(true), Some(false), Some(false), Some(true)]
        );
        assert!(!all_kept(&keep));
        assert!(all_kept(&BooleanArray::from(vec![true, true])));
    }
}
