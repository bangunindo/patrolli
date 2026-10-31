// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! OTAP attribute tables → `Map(LowCardinality(String), String)` columns.
//!
//! OTAP keeps attributes in side tables — one row per key, joined to the record that owns
//! them by `parent_id`, with the value in whichever of the `str`/`int`/`double`/`bool`/
//! `bytes`/`ser` columns matches the row's `type`. ClickHouse wants one string→string map
//! per record. So this module does two things:
//!
//! 1. [`to_maps`] groups an attribute batch by `parent_id` and renders each value as a
//!    string, producing a *compact* map array with one row per parent that actually has
//!    attributes.
//! 2. [`remap`] expands that back to one row per parent record, in record order.
//!
//! The value rendering follows the upstream ClickHouse exporter exactly (its
//! `group_attributes_to_map_str`), because those are the strings the community Grafana
//! dashboards and the Go exporter's own output assume.
//!
//! This is the one genuinely row-wise part of the conversion: turning a variant column into
//! text has no kernel form. [`remap`], by contrast, is a single `take`.

use anyhow::{Result, anyhow};
use arrow::array::{
    Array, ArrayRef, AsArray, BinaryArray, BooleanArray, Float64Array, Int64Array, MapArray,
    MapBuilder, StringArray, StringBuilder, UInt32Array,
};
use arrow::compute::kernels::{concat::concat, take::take};
use arrow::datatypes::{DataType, Float64Type, Int64Type};
use arrow::record_batch::RecordBatch;
use otel_arrow_dfe_pdata::OtapArrowRecords;
use otel_arrow_dfe_pdata::otlp::attributes::AttributeValueType;
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
use otel_arrow_dfe_pdata::schema::consts;
use rustc_hash::FxHashMap;

use super::common::{col, u8_values, undict};

/// A `Map<Utf8, Utf8>` array of `len` empty maps — the shape a record with no attributes,
/// or a whole signal with the attribute table omitted, must still produce. ClickHouse's map
/// columns are not `Nullable`, so this is empty maps rather than nulls.
pub(super) fn empty_maps(len: usize) -> MapArray {
    let mut b = MapBuilder::new(None, StringBuilder::new(), StringBuilder::new());
    for _ in 0..len {
        // Infallible: appending an entry with no keys buffered cannot fail.
        let _ = b.append(true);
    }
    b.finish()
}

/// Materialize one attribute table against its parents, in parent row order.
///
/// The one-call form of [`to_maps`] + [`remap`], and what the three signal builders
/// actually use. An absent table yields empty maps rather than failing, because OTAP omits
/// an attribute table entirely when nothing in the batch carries attributes.
pub(super) fn maps_for(
    records: &OtapArrowRecords,
    pt: ArrowPayloadType,
    parent_ids: &[Option<u64>],
) -> Result<MapArray> {
    let compact = records.get(pt).map(to_maps).transpose()?.flatten();
    remap(compact.as_ref(), parent_ids)
}

/// Group an attribute batch by `parent_id`, rendering every value as a string.
///
/// Returns the parent ids alongside the compact map array (one row each, same order), or
/// `None` when the batch carries no `parent_id` column at all.
pub(super) fn to_maps(batch: &RecordBatch) -> Result<Option<(Vec<u64>, MapArray)>> {
    let rows = batch.num_rows();
    let parent_ids = super::common::parent_ids_as_u64(col(batch, consts::PARENT_ID), rows)?;
    if col(batch, consts::PARENT_ID).is_none() {
        return Ok(None);
    }

    let groups = group_by_parent(&parent_ids);
    let keys = col(batch, consts::ATTRIBUTE_KEY)
        .map(undict)
        .transpose()?
        .map(|k| cast_utf8(&k))
        .transpose()?;
    let vals = AnyValueCols::new(|n| col(batch, n).cloned(), rows)?;

    let mut ids = Vec::with_capacity(groups.len());
    let mut map = MapBuilder::with_capacity(
        None,
        StringBuilder::new(),
        StringBuilder::new(),
        groups.len(),
    );

    for (id, group_rows) in groups {
        for row in group_rows {
            // A row with no key has nowhere to go in a map — skip it, as upstream does.
            let Some(key) = keys
                .as_ref()
                .and_then(|k| k.is_valid(row).then(|| k.value(row)))
            else {
                continue;
            };
            map.keys().append_value(key);
            vals.append_value(row, map.values());
        }
        map.append(true)?;
        ids.push(id);
    }

    Ok(Some((ids, map.finish())))
}

/// Expand a compact per-parent map array to one row per parent record.
///
/// Parents with no attributes — and rows whose `parent_id` is null — get an empty map, not
/// a null one. That is arranged by appending a single empty-map sentinel to the compact
/// array and pointing every unmatched row at it, so the whole expansion is one `take`
/// instead of a row-by-row rebuild.
pub(super) fn remap(
    compact: Option<&(Vec<u64>, MapArray)>,
    parent_ids: &[Option<u64>],
) -> Result<MapArray> {
    let Some((ids, maps)) = compact else {
        return Ok(empty_maps(parent_ids.len()));
    };

    let mut id_to_row: FxHashMap<u64, u32> = FxHashMap::default();
    id_to_row.reserve(ids.len());
    for (row, id) in ids.iter().enumerate() {
        let _ = id_to_row.insert(*id, row as u32);
    }

    let sentinel = maps.len() as u32;
    let indices: UInt32Array = parent_ids
        .iter()
        .map(|pid| {
            pid.and_then(|p| id_to_row.get(&p).copied())
                .unwrap_or(sentinel)
        })
        .collect::<Vec<_>>()
        .into();

    let with_sentinel = concat(&[maps as &dyn Array, &empty_maps(1) as &dyn Array])?;
    let taken = take(with_sentinel.as_ref(), &indices, None)?;
    taken
        .as_any()
        .downcast_ref::<MapArray>()
        .cloned()
        .ok_or_else(|| anyhow!("take over a MapArray did not produce a MapArray"))
}

/// Group row indices by `parent_id`, preserving first-appearance order.
///
/// OTAP emits attribute rows sorted by `parent_id`, so the common case is a single forward
/// scan over contiguous runs with no hashing. The hashed path is only taken if that
/// invariant is broken.
fn group_by_parent(parent_ids: &[Option<u64>]) -> Vec<(u64, Vec<usize>)> {
    let sorted = parent_ids
        .iter()
        .flatten()
        .try_fold(None::<u64>, |prev, &id| match prev {
            Some(p) if id < p => Err(()),
            _ => Ok(Some(id)),
        })
        .is_ok();

    if sorted {
        let mut out: Vec<(u64, Vec<usize>)> = Vec::new();
        for (row, pid) in parent_ids.iter().enumerate() {
            let Some(id) = pid else { continue };
            match out.last_mut() {
                Some((last, rows)) if last == id => rows.push(row),
                _ => out.push((*id, vec![row])),
            }
        }
        return out;
    }

    let mut order: Vec<u64> = Vec::new();
    let mut by_id: FxHashMap<u64, Vec<usize>> = FxHashMap::default();
    for (row, pid) in parent_ids.iter().enumerate() {
        let Some(id) = pid else { continue };
        by_id
            .entry(*id)
            .or_insert_with(|| {
                order.push(*id);
                Vec::new()
            })
            .push(row);
    }
    order
        .into_iter()
        .map(|id| {
            let rows = by_id.remove(&id).unwrap_or_default();
            (id, rows)
        })
        .collect()
}

fn cast_utf8(array: &ArrayRef) -> Result<StringArray> {
    let a = if matches!(array.data_type(), DataType::Utf8) {
        array.clone()
    } else {
        arrow::compute::kernels::cast::cast(array.as_ref(), &DataType::Utf8)?
    };
    a.as_string_opt::<i32>()
        .cloned()
        .ok_or_else(|| anyhow!("attribute key column is not Utf8"))
}

/// The OTAP `AnyValue` column family: a `type` discriminator plus one column per value
/// type, any of which OTAP may omit.
///
/// Shared by the attribute tables and by the log `body` struct, which uses the same column
/// names — so the same renderer serves both.
pub(super) struct AnyValueCols {
    types: Vec<u8>,
    str_: Option<StringArray>,
    int: Option<Int64Array>,
    double: Option<Float64Array>,
    bool_: Option<BooleanArray>,
    bytes: Option<BinaryArray>,
    ser: Option<BinaryArray>,
}

impl AnyValueCols {
    /// `lookup` resolves a column name against either a `RecordBatch` or a `StructArray`.
    pub(super) fn new(lookup: impl Fn(&str) -> Option<ArrayRef>, len: usize) -> Result<Self> {
        let plain = |name: &str| -> Result<Option<ArrayRef>> {
            lookup(name).map(|a| undict(&a)).transpose()
        };
        let to = |a: Option<ArrayRef>, dt: DataType| -> Result<Option<ArrayRef>> {
            a.map(|a| {
                if a.data_type() == &dt {
                    Ok(a)
                } else {
                    Ok(arrow::compute::kernels::cast::cast(a.as_ref(), &dt)?)
                }
            })
            .transpose()
        };

        Ok(Self {
            types: u8_values(lookup(consts::ATTRIBUTE_TYPE).as_ref(), len)?,
            str_: to(plain(consts::ATTRIBUTE_STR)?, DataType::Utf8)?
                .and_then(|a| a.as_string_opt::<i32>().cloned()),
            int: to(plain(consts::ATTRIBUTE_INT)?, DataType::Int64)?
                .and_then(|a| a.as_primitive_opt::<Int64Type>().cloned()),
            double: to(plain(consts::ATTRIBUTE_DOUBLE)?, DataType::Float64)?
                .and_then(|a| a.as_primitive_opt::<Float64Type>().cloned()),
            bool_: to(plain(consts::ATTRIBUTE_BOOL)?, DataType::Boolean)?
                .and_then(|a| a.as_boolean_opt().cloned()),
            bytes: to(plain(consts::ATTRIBUTE_BYTES)?, DataType::Binary)?
                .and_then(|a| a.as_binary_opt::<i32>().cloned()),
            ser: to(plain(consts::ATTRIBUTE_SER)?, DataType::Binary)?
                .and_then(|a| a.as_binary_opt::<i32>().cloned()),
        })
    }

    /// Render row `row` into `out`.
    ///
    /// Every arm falls back to `""` when the value column the `type` points at was omitted
    /// or is null, so a value is always appended and the map's key/value runs stay aligned.
    pub(super) fn append_value(&self, row: usize, out: &mut StringBuilder) {
        let t = self.types.get(row).copied().unwrap_or(0);

        // `f64::to_string` already yields the shortest round-tripping form, which is what
        // Go's `strconv.FormatFloat(v, 'f', -1, 64)` produces in the reference exporter —
        // so `1.0` renders as "1", not "1.0".
        let rendered = if t == AttributeValueType::Str as u8 {
            self.str_
                .as_ref()
                .filter(|c| c.is_valid(row))
                .map(|c| c.value(row).to_string())
        } else if t == AttributeValueType::Int as u8 {
            self.int
                .as_ref()
                .filter(|c| c.is_valid(row))
                .map(|c| c.value(row).to_string())
        } else if t == AttributeValueType::Double as u8 {
            self.double
                .as_ref()
                .filter(|c| c.is_valid(row))
                .map(|c| c.value(row).to_string())
        } else if t == AttributeValueType::Bool as u8 {
            self.bool_
                .as_ref()
                .filter(|c| c.is_valid(row))
                .map(|c| if c.value(row) { "true" } else { "false" }.to_string())
        } else if t == AttributeValueType::Bytes as u8 {
            self.bytes.as_ref().filter(|c| c.is_valid(row)).map(|c| {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(c.value(row))
            })
        } else if t == AttributeValueType::Map as u8 || t == AttributeValueType::Slice as u8 {
            self.ser
                .as_ref()
                .filter(|c| c.is_valid(row))
                .and_then(|c| cbor_to_json(c.value(row)))
        } else {
            // Empty, or a type this build does not know.
            None
        };

        out.append_value(rendered.as_deref().unwrap_or(""));
    }
}

/// Render a CBOR-encoded `ser` value as JSON.
///
/// Reuses the JSON serializer's own CBOR renderer so a map or slice attribute reads the
/// same here as it does in the file sink's nested-JSON output.
fn cbor_to_json(bytes: &[u8]) -> Option<String> {
    let value: ciborium::value::Value = ciborium::from_reader(bytes).ok()?;
    serde_json::to_string(&crate::payload::json_ser::CborNativeSer(&value)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{StringBuilder as SB, UInt8Array, UInt16Array};
    use arrow::datatypes::{Field, Schema};
    use std::sync::Arc;

    /// `[(parent_id, key, type, str_value)]` → an attribute batch.
    fn attr_batch(rows: &[(Option<u16>, &str, u8)], strs: &[Option<&str>]) -> RecordBatch {
        let schema = Schema::new(vec![
            Field::new(consts::PARENT_ID, DataType::UInt16, true),
            Field::new(consts::ATTRIBUTE_KEY, DataType::Utf8, true),
            Field::new(consts::ATTRIBUTE_TYPE, DataType::UInt8, true),
            Field::new(consts::ATTRIBUTE_STR, DataType::Utf8, true),
        ]);
        RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(UInt16Array::from(
                    rows.iter().map(|r| r.0).collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    rows.iter().map(|r| r.1).collect::<Vec<_>>(),
                )),
                Arc::new(UInt8Array::from(
                    rows.iter().map(|r| r.2).collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(strs.to_vec())),
            ],
        )
        .unwrap()
    }

    fn as_pairs(map: &MapArray, row: usize) -> Vec<(String, String)> {
        let keys = map.keys().as_string::<i32>();
        let vals = map.values().as_string::<i32>();
        let o = map.offsets();
        (o[row] as usize..o[row + 1] as usize)
            .map(|i| (keys.value(i).to_string(), vals.value(i).to_string()))
            .collect()
    }

    #[test]
    fn groups_by_parent_and_renders_strings() {
        let s = AttributeValueType::Str as u8;
        let b = attr_batch(
            &[(Some(1), "a", s), (Some(1), "b", s), (Some(3), "c", s)],
            &[Some("1a"), Some("1b"), Some("3c")],
        );
        let (ids, maps) = to_maps(&b).unwrap().unwrap();
        assert_eq!(ids, vec![1, 3]);
        assert_eq!(
            as_pairs(&maps, 0),
            vec![
                ("a".to_string(), "1a".to_string()),
                ("b".to_string(), "1b".to_string())
            ]
        );
        assert_eq!(as_pairs(&maps, 1), vec![("c".into(), "3c".into())]);
    }

    #[test]
    fn remap_aligns_to_record_order_and_fills_empties() {
        let s = AttributeValueType::Str as u8;
        let b = attr_batch(
            &[(Some(1), "a", s), (Some(3), "c", s)],
            &[Some("1a"), Some("3c")],
        );
        let compact = to_maps(&b).unwrap().unwrap();

        // Records with ids 3, 0, 1 — note the reordering, the miss, and the null parent.
        let out = remap(Some(&compact), &[Some(3), Some(0), Some(1), None]).unwrap();
        assert_eq!(out.len(), 4);
        assert_eq!(out.null_count(), 0);
        assert_eq!(as_pairs(&out, 0), vec![("c".into(), "3c".into())]);
        assert!(as_pairs(&out, 1).is_empty());
        assert_eq!(as_pairs(&out, 2), vec![("a".into(), "1a".into())]);
        assert!(as_pairs(&out, 3).is_empty());
    }

    #[test]
    fn absent_attribute_table_yields_empty_maps() {
        let out = remap(None, &[Some(1), Some(2)]).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out.null_count(), 0);
        assert!(as_pairs(&out, 0).is_empty());
    }

    #[test]
    fn unsorted_parent_ids_still_group() {
        let s = AttributeValueType::Str as u8;
        let b = attr_batch(
            &[(Some(5), "a", s), (Some(1), "b", s), (Some(5), "c", s)],
            &[Some("5a"), Some("1b"), Some("5c")],
        );
        let (ids, maps) = to_maps(&b).unwrap().unwrap();
        assert_eq!(ids, vec![5, 1]);
        assert_eq!(
            as_pairs(&maps, 0),
            vec![("a".into(), "5a".into()), ("c".into(), "5c".into())]
        );
    }

    #[test]
    fn value_types_render_like_the_reference_exporter() {
        let mut out = SB::new();
        let lookup = |name: &str| -> Option<ArrayRef> {
            match name {
                consts::ATTRIBUTE_TYPE => Some(Arc::new(UInt8Array::from(vec![
                    AttributeValueType::Int as u8,
                    AttributeValueType::Double as u8,
                    AttributeValueType::Bool as u8,
                    AttributeValueType::Bytes as u8,
                    AttributeValueType::Empty as u8,
                ])) as ArrayRef),
                consts::ATTRIBUTE_INT => {
                    Some(
                        Arc::new(Int64Array::from(vec![Some(-7), None, None, None, None]))
                            as ArrayRef,
                    )
                }
                consts::ATTRIBUTE_DOUBLE => {
                    Some(
                        Arc::new(Float64Array::from(vec![None, Some(1.0), None, None, None]))
                            as ArrayRef,
                    )
                }
                consts::ATTRIBUTE_BOOL => {
                    Some(
                        Arc::new(BooleanArray::from(vec![None, None, Some(true), None, None]))
                            as ArrayRef,
                    )
                }
                consts::ATTRIBUTE_BYTES => Some(Arc::new(BinaryArray::from_opt_vec(vec![
                    None,
                    None,
                    None,
                    Some(&[0x01, 0x02]),
                    None,
                ])) as ArrayRef),
                _ => None,
            }
        };
        let cols = AnyValueCols::new(lookup, 5).unwrap();
        for row in 0..5 {
            cols.append_value(row, &mut out);
        }
        let got = out.finish();
        assert_eq!(got.value(0), "-7");
        // Shortest round-trip, matching Go's FormatFloat(_, 'f', -1, 64).
        assert_eq!(got.value(1), "1");
        assert_eq!(got.value(2), "true");
        assert_eq!(got.value(3), "AQI=");
        assert_eq!(got.value(4), "");
    }

    #[test]
    fn map_values_render_as_json() {
        let mut buf = Vec::new();
        ciborium::into_writer(
            &ciborium::value::Value::Map(vec![(
                ciborium::value::Value::Text("k".into()),
                ciborium::value::Value::Integer(3.into()),
            )]),
            &mut buf,
        )
        .unwrap();
        assert_eq!(cbor_to_json(&buf).as_deref(), Some(r#"{"k":3}"#));
    }
}
