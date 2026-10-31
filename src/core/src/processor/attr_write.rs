// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Sparse attribute upserts: one lookup per existing row, one merge per column.

use crate::cache::lookup_table::cell::CellRef;
use crate::payload::attr::StrCol;
use anyhow::{Result, bail};
use arrow::array::{
    Array, ArrayRef, AsArray, BooleanArray, DictionaryArray, Float64Array, Int64Array,
    PrimitiveArray, RecordBatch, StringArray, UInt8Array, UInt16Array, UInt64Array, new_null_array,
};
use arrow::compute::{cast, concat, take};
use arrow::datatypes::{
    ArrowDictionaryKeyType, ArrowNativeType, DataType, Field, Schema, UInt8Type, UInt16Type,
    UInt32Type,
};
use otel_arrow_dfe_pdata::otlp::attributes::AttributeValueType;
use otel_arrow_dfe_pdata::schema::consts;
use rustc_hash::FxHashMap;
use std::sync::Arc;

pub(super) struct Write<'a> {
    pub key: &'a str,
    pub parent: u16,
    pub cell: CellRef<'a>,
}

/// `writes` has unique (key, parent) pairs, ordered for deterministic inserts.
/// Existing duplicate rows are retained, and all are replaced on overwrite.
pub(super) fn merge(
    existing: &RecordBatch,
    writes: &[Write<'_>],
    overwrite: bool,
) -> Result<Option<RecordBatch>> {
    if writes.is_empty() {
        return Ok(None);
    }
    let n = existing.num_rows();
    let keys = StrCol::try_new(
        existing
            .column_by_name(consts::ATTRIBUTE_KEY)
            .ok_or_else(|| anyhow::anyhow!("missing attribute key column"))?
            .as_ref(),
    )?;
    let parents = existing
        .column_by_name(consts::PARENT_ID)
        .and_then(|c| c.as_primitive_opt::<UInt16Type>())
        .ok_or_else(|| anyhow::anyhow!("missing UInt16 attribute parent column"))?;
    let keyed = KeyRuns::of(writes);
    let mut present = vec![false; writes.len()];
    // Rows of `existing` in their original order, empty while that is the
    // identity — only `overwrite` ever replaces a row, and rows are never
    // dropped.
    let mut kept = Vec::new();
    let mut changed = false;
    // Can a write land on a row the batch already carries? Enrichment normally
    // writes keys the batch does not have yet, and the key column answers that
    // for all of its rows at once, which is worth asking: the scan below reads
    // every row and hashes its key, for nothing, whenever the answer is no.
    if keys.may_hold_any(keyed.distinct.iter().copied()) {
        let index: FxHashMap<_, _> = writes
            .iter()
            .enumerate()
            .map(|(i, write)| ((write.key, write.parent), i))
            .collect();
        if overwrite {
            kept.reserve(n);
        }
        for row in 0..n {
            let replacement = if keys.is_valid(row) && parents.is_valid(row) {
                index.get(&(keys.value(row), parents.value(row))).copied()
            } else {
                None
            };
            let mut source = row as u64;
            if let Some(write) = replacement {
                present[write] = true;
                if overwrite {
                    source = (n + write) as u64;
                    changed = true;
                }
            }
            if overwrite {
                kept.push(source);
            }
        }
    }
    // `(parent, write)`, not the bare write index: the ordering below reads the
    // parent once per write instead of chasing `writes` on every comparison.
    let mut appended: Vec<(u16, u32)> = present
        .into_iter()
        .enumerate()
        .filter(|(_, present)| !*present)
        .map(|(i, _)| (writes[i].parent, i as u32))
        .collect();
    changed |= !appended.is_empty();
    if !changed {
        return Ok(None);
    }

    // Where the appended rows go. An attribute batch reaches the sink through
    // `sort_by_parent_then_id`, which returns untouched when the parent column
    // is already sorted — appending at the end forfeits that and costs a full
    // sort plus a `take` of every column downstream. Both sides are sorted
    // here (`existing` by construction, the writes after one sort of the few
    // appended indices), so interleaving them is one pass and keeps the batch
    // on the fast path. An unsorted `existing` has nothing to preserve, so it
    // keeps the cheaper append.
    let source_of = |row: usize| kept.get(row).copied().unwrap_or(row as u64);
    let mut selection = Vec::with_capacity(n + appended.len());
    if parents.null_count() == 0 && parents.values().is_sorted() {
        // Ties keep the write order, which is the order an append would have
        // given them.
        appended.sort_unstable();
        let mut next = 0;
        for row in 0..n {
            let parent = parents.value(row);
            // `<`, not `<=`: a write for a parent that already has rows lands
            // after them, as it does when appended.
            while next < appended.len() && appended[next].0 < parent {
                selection.push((n + appended[next].1 as usize) as u64);
                next += 1;
            }
            selection.push(source_of(row));
        }
        selection.extend(
            appended[next..]
                .iter()
                .map(|&(_, write)| (n + write as usize) as u64),
        );
    } else {
        selection.extend((0..n).map(source_of));
        selection.extend(
            appended
                .iter()
                .map(|&(_, write)| (n + write as usize) as u64),
        );
    }

    let added = write_batch(
        writes,
        &keyed,
        existing
            .column_by_name(consts::ATTRIBUTE_KEY)
            .map(|c| c.data_type()),
    )?;
    let selection = UInt64Array::from(selection);
    let old_schema = existing.schema();
    let added_schema = added.schema();
    let mut fields = old_schema.fields().to_vec();
    for field in added_schema.fields() {
        if existing.column_by_name(field.name()).is_none() {
            fields.push(field.clone());
        }
    }
    let mut columns = Vec::with_capacity(fields.len());
    for field in &mut fields {
        let old = existing
            .column_by_name(field.name())
            .cloned()
            .unwrap_or_else(|| new_null_array(field.data_type(), n));
        let new = added
            .column_by_name(field.name())
            .cloned()
            .unwrap_or_else(|| new_null_array(field.data_type(), writes.len()));
        let combined = concat_column(&old, &new)?;
        let column = take(combined.as_ref(), &selection, None)?;
        *field = Arc::new(
            field
                .as_ref()
                .clone()
                .with_data_type(column.data_type().clone())
                .with_nullable(field.is_nullable() || column.logical_null_count() > 0),
        );
        columns.push(column);
    }
    let schema = Schema::new_with_metadata(fields, old_schema.metadata().clone());
    Ok(Some(RecordBatch::try_new(Arc::new(schema), columns)?))
}

/// Merge dictionaries in a wide temporary, then retain or widen the original
/// key width according to the merged dictionary size.
///
/// `concat` interns the value arrays only when the inputs carry at least as
/// many values as rows. An attribute batch is the opposite shape, so the arrays
/// are appended verbatim and re-writing an existing key still grows the count —
/// which, taken at face value, widens the column for nothing and creeps upward
/// across chained enrichment nodes until it drops to plain values.
///
/// So the count is trusted only while it still fits the current width. Past
/// that, and only then, the values are really interned and the true distinct
/// count decides, leaving the common path at the cost it had before.
fn concat_column(old: &ArrayRef, new: &ArrayRef) -> Result<ArrayRef> {
    if let DataType::Dictionary(key, value) = old.data_type() {
        // Fast path: append at the column's own key width. The widen below
        // exists to keep the intermediate in range, and the appended values
        // are in range exactly while they still fit that width — the same
        // budget the wide path goes on to check. Worth the check on its own:
        // it drops two casts of the whole column, and `new` is encoded once
        // against the width it will be stored at rather than twice.
        let values_len = |a: &ArrayRef| a.as_any_dictionary().values().len();
        if let Ok(new_same) = cast(new, old.data_type())
            && values_len(old) + values_len(&new_same) <= key_capacity(key)
        {
            return Ok(concat(&[old.as_ref(), new_same.as_ref()])?);
        }
        let wide = DataType::Dictionary(Box::new(DataType::UInt32), value.clone());
        let old_wide = cast(old, &wide)?;
        let new_wide = cast(new, &wide)?;
        let combined = concat(&[old_wide.as_ref(), new_wide.as_ref()])?;
        let values_of = |a: &ArrayRef| a.as_dictionary::<UInt32Type>().values().len();
        if values_of(&combined) <= key_capacity(key) {
            return Ok(cast(&combined, old.data_type())?);
        }
        let interned = cast(&cast(&combined, value)?, &wide)?;
        let count = values_of(&interned);
        let target = if count <= key_capacity(key) {
            old.data_type().clone()
        } else if count <= 65_536 {
            DataType::Dictionary(Box::new(DataType::UInt16), value.clone())
        } else {
            value.as_ref().clone()
        };
        return Ok(cast(&interned, &target)?);
    }
    if old.data_type() != new.data_type() {
        bail!(
            "incompatible attribute column types: {:?} and {:?}",
            old.data_type(),
            new.data_type()
        );
    }
    Ok(concat(&[old.as_ref(), new.as_ref()])?)
}

/// How many distinct values a dictionary key of this width can address.
fn key_capacity(key: &DataType) -> usize {
    match key {
        DataType::Int8 | DataType::UInt8 => 256,
        DataType::Int16 | DataType::UInt16 => 65_536,
        // wider than any count `merge` can produce in one batch
        _ => usize::MAX,
    }
}

/// The keys of a write set, read once: the distinct ones and, per write, which
/// of them it carries.
///
/// `writes` is grouped by key, so this is a run scan rather than a hash of
/// every row, and both users want the same thing out of it — whether the batch
/// being merged into could hold any of these keys, and the dictionary of the
/// column that is about to be built from them.
struct KeyRuns<'a> {
    distinct: Vec<&'a str>,
    of_write: Vec<usize>,
}

impl<'a> KeyRuns<'a> {
    fn of(writes: &[Write<'a>]) -> Self {
        let mut distinct: Vec<&str> = Vec::new();
        let mut of_write = Vec::with_capacity(writes.len());
        for write in writes {
            // from the back: the run this write belongs to is the last one seen
            let idx = match distinct.iter().rposition(|key| *key == write.key) {
                Some(idx) => idx,
                None => {
                    distinct.push(write.key);
                    distinct.len() - 1
                }
            };
            of_write.push(idx);
        }
        Self { distinct, of_write }
    }

    /// The key column of the added rows, dictionary-encoded to `want` when that
    /// is the width the batch it is merging into already uses: the key strings
    /// are written once each rather than once per row. `None` when the target
    /// is not a `Utf8` dictionary this can build, or when the keys would not
    /// fit it — [`concat_column`] handles both.
    fn dictionary(&self, want: Option<&DataType>) -> Option<ArrayRef> {
        let DataType::Dictionary(key, value) = want? else {
            return None;
        };
        if value.as_ref() != &DataType::Utf8 || self.distinct.len() > key_capacity(key) {
            return None;
        }
        let values: ArrayRef = Arc::new(StringArray::from(self.distinct.clone()));
        fn build<K: ArrowDictionaryKeyType>(
            indices: &[usize],
            values: ArrayRef,
        ) -> Option<ArrayRef> {
            let keys: Vec<K::Native> = indices
                .iter()
                .map(|&idx| K::Native::from_usize(idx))
                .collect::<Option<_>>()?;
            let keys = PrimitiveArray::<K>::from_iter_values(keys);
            Some(Arc::new(DictionaryArray::<K>::try_new(keys, values).ok()?))
        }
        match key.as_ref() {
            DataType::UInt8 => build::<UInt8Type>(&self.of_write, values),
            DataType::UInt16 => build::<UInt16Type>(&self.of_write, values),
            DataType::UInt32 => build::<UInt32Type>(&self.of_write, values),
            _ => None,
        }
    }
}

fn write_batch(
    writes: &[Write<'_>],
    keyed: &KeyRuns<'_>,
    key_type: Option<&DataType>,
) -> Result<RecordBatch> {
    let key_column = keyed
        .dictionary(key_type)
        .unwrap_or_else(|| Arc::new(StringArray::from_iter_values(writes.iter().map(|w| w.key))));
    let mut fields = vec![
        Field::new(consts::PARENT_ID, DataType::UInt16, false),
        Field::new(consts::ATTRIBUTE_KEY, key_column.data_type().clone(), false),
        Field::new(consts::ATTRIBUTE_TYPE, DataType::UInt8, false),
    ];
    let types: Vec<_> = writes
        .iter()
        .map(|write| match write.cell {
            CellRef::String(_) => AttributeValueType::Str,
            CellRef::Integer(_) => AttributeValueType::Int,
            CellRef::Double(_) => AttributeValueType::Double,
            CellRef::Bool(_) => AttributeValueType::Bool,
            _ => AttributeValueType::Empty,
        })
        .collect();
    let mut columns: Vec<ArrayRef> = vec![
        Arc::new(UInt16Array::from_iter_values(
            writes.iter().map(|w| w.parent),
        )),
        key_column,
        Arc::new(UInt8Array::from_iter_values(types.iter().map(|t| *t as u8))),
    ];
    for (ty, name) in [
        (AttributeValueType::Str, consts::ATTRIBUTE_STR),
        (AttributeValueType::Int, consts::ATTRIBUTE_INT),
        (AttributeValueType::Double, consts::ATTRIBUTE_DOUBLE),
        (AttributeValueType::Bool, consts::ATTRIBUTE_BOOL),
    ] {
        if !types.contains(&ty) {
            continue;
        }
        let column: ArrayRef = match ty {
            AttributeValueType::Str => Arc::new(
                writes
                    .iter()
                    .map(|w| match w.cell {
                        CellRef::String(v) => Some(v),
                        _ => None,
                    })
                    .collect::<StringArray>(),
            ),
            AttributeValueType::Int => Arc::new(
                writes
                    .iter()
                    .map(|w| match w.cell {
                        CellRef::Integer(v) => Some(v),
                        _ => None,
                    })
                    .collect::<Int64Array>(),
            ),
            AttributeValueType::Double => Arc::new(
                writes
                    .iter()
                    .map(|w| match w.cell {
                        CellRef::Double(v) => Some(v),
                        _ => None,
                    })
                    .collect::<Float64Array>(),
            ),
            AttributeValueType::Bool => Arc::new(
                writes
                    .iter()
                    .map(|w| match w.cell {
                        CellRef::Bool(v) => Some(v),
                        _ => None,
                    })
                    .collect::<BooleanArray>(),
            ),
            _ => unreachable!(),
        };
        fields.push(Field::new(name, column.data_type().clone(), true));
        columns.push(column);
    }
    Ok(RecordBatch::try_new(
        Arc::new(Schema::new(fields)),
        columns,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Value, attr::AttrValueCols};
    use arrow::array::BinaryArray;

    /// A batch of exactly these writes, with the plain key column a caller that
    /// has no existing batch to match gets.
    fn plain_batch(writes: &[Write<'_>]) -> RecordBatch {
        write_batch(writes, &KeyRuns::of(writes), None).unwrap()
    }

    #[test]
    fn sparse_merge_preserves_duplicates_order_and_insert_only_nulls() {
        let existing = plain_batch(&[
            Write {
                key: "x",
                parent: 0,
                cell: CellRef::String("old"),
            },
            Write {
                key: "null",
                parent: 0,
                cell: CellRef::Null,
            },
            Write {
                key: "x",
                parent: 0,
                cell: CellRef::String("duplicate"),
            },
            Write {
                key: "untouched",
                parent: 1,
                cell: CellRef::Integer(7),
            },
        ]);
        let writes = [
            Write {
                key: "null",
                parent: 0,
                cell: CellRef::String("filled"),
            },
            Write {
                key: "x",
                parent: 0,
                cell: CellRef::Bool(true),
            },
            Write {
                key: "z",
                parent: 2,
                cell: CellRef::Double(1.5),
            },
        ];
        for overwrite in [false, true] {
            let out = merge(&existing, &writes, overwrite).unwrap().unwrap();
            assert_eq!(out.num_rows(), 5);
            let keys = StrCol::try_new(out.column_by_name(consts::ATTRIBUTE_KEY).unwrap().as_ref())
                .unwrap();
            assert_eq!(
                (0..5).map(|i| keys.value(i)).collect::<Vec<_>>(),
                ["x", "null", "x", "untouched", "z"]
            );
            let values = AttrValueCols::try_new(&out).unwrap();
            assert!(matches!(values.value_at(3), Some(Value::Int64(7))));
            assert!(matches!(values.value_at(4), Some(Value::Double(1.5))));
            if overwrite {
                for row in [0, 2] {
                    assert!(matches!(values.value_at(row), Some(Value::Bool(true))));
                    assert!(
                        out.column_by_name(consts::ATTRIBUTE_STR)
                            .unwrap()
                            .is_null(row)
                    );
                }
                assert!(matches!(values.value_at(1), Some(Value::String(_))));
            } else {
                assert_eq!(
                    out.slice(0, 4).column_by_name(consts::ATTRIBUTE_TYPE),
                    existing.column_by_name(consts::ATTRIBUTE_TYPE)
                );
                assert!(merge(&existing, &writes[..2], false).unwrap().is_none());
            }
        }
    }

    /// Appended rows go where they belong, not at the end.
    ///
    /// The batch is read downstream by `sort_by_parent_then_id`, which is free
    /// only while the parent column is sorted, so a merge that appends costs a
    /// full sort and a `take` of every column somewhere else. The order is a
    /// property of the merge, not a side effect of this input: a write for a
    /// parent that sorts before an existing row has to land before it.
    #[test]
    fn appended_rows_keep_the_parent_column_sorted() {
        let existing = plain_batch(&[
            Write {
                key: "a",
                parent: 0,
                cell: CellRef::String("x"),
            },
            Write {
                key: "a",
                parent: 2,
                cell: CellRef::String("y"),
            },
        ]);
        // deliberately not in parent order, and neither one appends at the end
        let writes = [
            Write {
                key: "b",
                parent: 1,
                cell: CellRef::Integer(1),
            },
            Write {
                key: "b",
                parent: 0,
                cell: CellRef::Integer(0),
            },
        ];
        let out = merge(&existing, &writes, false).unwrap().unwrap();
        let parents = out
            .column_by_name(consts::PARENT_ID)
            .unwrap()
            .as_primitive::<UInt16Type>();
        assert_eq!(parents.values().to_vec(), [0, 0, 1, 2]);
        let keys =
            StrCol::try_new(out.column_by_name(consts::ATTRIBUTE_KEY).unwrap().as_ref()).unwrap();
        // a write for a parent that already has rows still lands after them
        assert_eq!(
            (0..4).map(|i| keys.value(i)).collect::<Vec<_>>(),
            ["a", "b", "b", "a"]
        );

        // An unsorted batch has no order to preserve, so it keeps the append —
        // every row still survives, with its own parent.
        let unsorted = plain_batch(&[
            Write {
                key: "a",
                parent: 2,
                cell: CellRef::String("x"),
            },
            Write {
                key: "a",
                parent: 0,
                cell: CellRef::String("y"),
            },
        ]);
        let out = merge(&unsorted, &writes[..1], false).unwrap().unwrap();
        let parents = out
            .column_by_name(consts::PARENT_ID)
            .unwrap()
            .as_primitive::<UInt16Type>();
        assert_eq!(parents.values().to_vec(), [2, 0, 1]);
    }

    /// The per-row scan a dictionary key column lets `merge` skip is the one
    /// that finds collisions, so a key the dictionary does carry must never be
    /// skipped over: that would append a second row for a `(key, parent)` the
    /// batch already has instead of overwriting the one it has.
    #[test]
    fn a_written_key_already_in_the_dictionary_is_still_found() {
        let old = plain_batch(&[
            Write {
                key: "a",
                parent: 0,
                cell: CellRef::String("old"),
            },
            Write {
                key: "c",
                parent: 1,
                cell: CellRef::String("other"),
            },
        ]);
        let dict = DataType::Dictionary(Box::new(DataType::UInt8), Box::new(DataType::Utf8));
        let mut fields = old.schema().fields().to_vec();
        let mut columns = old.columns().to_vec();
        columns[1] = cast(&columns[1], &dict).unwrap();
        fields[1] = Arc::new(fields[1].as_ref().clone().with_data_type(dict));
        let old = RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).unwrap();

        // "b" is not in the dictionary, "a" is
        let writes = [
            Write {
                key: "b",
                parent: 0,
                cell: CellRef::String("new"),
            },
            Write {
                key: "a",
                parent: 0,
                cell: CellRef::String("new"),
            },
        ];
        let out = merge(&old, &writes, true).unwrap().unwrap();
        assert_eq!(out.num_rows(), 3, "only the key it did not have is added");
        let keys =
            StrCol::try_new(out.column_by_name(consts::ATTRIBUTE_KEY).unwrap().as_ref()).unwrap();
        assert_eq!(
            (0..3).map(|i| keys.value(i)).collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        let values =
            StrCol::try_new(out.column_by_name(consts::ATTRIBUTE_STR).unwrap().as_ref()).unwrap();
        assert_eq!(values.value(0), "new", "the existing row was overwritten");
    }

    #[test]
    fn unrelated_binary_values_and_metadata_survive() {
        let old = plain_batch(&[
            Write {
                key: "bytes",
                parent: 0,
                cell: CellRef::Null,
            },
            Write {
                key: "change",
                parent: 0,
                cell: CellRef::String("old"),
            },
        ]);
        let mut fields = old.schema().fields().to_vec();
        fields.push(Arc::new(Field::new(
            consts::ATTRIBUTE_BYTES,
            DataType::Binary,
            true,
        )));
        let mut columns = old.columns().to_vec();
        columns[2] = Arc::new(UInt8Array::from(vec![
            AttributeValueType::Bytes as u8,
            AttributeValueType::Str as u8,
        ]));
        columns.push(Arc::new(BinaryArray::from(vec![
            Some(b"opaque".as_slice()),
            None,
        ])));
        let old = RecordBatch::try_new(
            Arc::new(Schema::new_with_metadata(
                fields,
                [("audit".into(), "keep".into())].into(),
            )),
            columns,
        )
        .unwrap();
        let out = merge(
            &old,
            &[Write {
                key: "change",
                parent: 0,
                cell: CellRef::Integer(9),
            }],
            true,
        )
        .unwrap()
        .unwrap();
        assert_eq!(out.schema().metadata(), old.schema().metadata());
        assert_eq!(
            out.column_by_name(consts::ATTRIBUTE_BYTES),
            old.column_by_name(consts::ATTRIBUTE_BYTES)
        );
    }

    /// `concat` only interns the two value arrays when the inputs carry at
    /// least as many values as rows. An attribute batch is the opposite shape —
    /// many parents over a handful of keys — so overwriting keys that already
    /// exist still grows the naive count, and trusting it widened the column
    /// for nothing.
    #[test]
    fn rewriting_existing_keys_does_not_widen_the_dictionary() {
        // 200 distinct keys across 5 parents: 1000 rows, comfortably inside a
        // UInt8 dictionary, and far more rows than values
        let strings: Vec<_> = (0..200).map(|i| format!("key-{i}")).collect();
        let mut old_writes = Vec::new();
        for parent in 0..5u16 {
            for key in &strings {
                old_writes.push(Write {
                    key,
                    parent,
                    cell: CellRef::String("old"),
                });
            }
        }
        let old = plain_batch(&old_writes);
        let narrow = DataType::Dictionary(Box::new(DataType::UInt8), Box::new(DataType::Utf8));
        let mut fields = old.schema().fields().to_vec();
        let mut columns = old.columns().to_vec();
        columns[1] = cast(&columns[1], &narrow).unwrap();
        fields[1] = Arc::new(fields[1].as_ref().clone().with_data_type(narrow.clone()));
        let old = RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).unwrap();

        // rewrite 60 keys that already exist: 260 values appended, 200 distinct
        let new_writes: Vec<_> = strings[..60]
            .iter()
            .map(|key| Write {
                key,
                parent: 0,
                cell: CellRef::String("new"),
            })
            .collect();
        let out = merge(&old, &new_writes, true).unwrap().unwrap();
        let key = out.column_by_name(consts::ATTRIBUTE_KEY).unwrap();
        assert_eq!(
            key.data_type(),
            &narrow,
            "no key was added, so the column must keep its width"
        );
        assert_eq!(
            out.num_rows(),
            old_writes.len(),
            "overwrites replace rather than append"
        );
    }

    #[test]
    fn dictionaries_widen_at_the_boundary_and_fall_back_to_plain_when_full() {
        for n in [254, 256, 65_536] {
            let strings: Vec<_> = (0..n).map(|i| format!("key-{i}")).collect();
            let old_writes: Vec<_> = strings
                .iter()
                .map(|key| Write {
                    key,
                    parent: 0,
                    cell: CellRef::String("old"),
                })
                .collect();
            let old = plain_batch(&old_writes);
            let mut fields = old.schema().fields().to_vec();
            let mut columns = old.columns().to_vec();
            let dictionary = DataType::Dictionary(
                Box::new(if n <= 256 {
                    DataType::UInt8
                } else {
                    DataType::UInt16
                }),
                Box::new(DataType::Utf8),
            );
            columns[1] = cast(&columns[1], &dictionary).unwrap();
            fields[1] = Arc::new(fields[1].as_ref().clone().with_data_type(dictionary));
            let old = RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).unwrap();
            let out = merge(
                &old,
                &[Write {
                    key: "new",
                    parent: 0,
                    cell: CellRef::String("new"),
                }],
                false,
            )
            .unwrap()
            .unwrap();
            let key = out.column_by_name(consts::ATTRIBUTE_KEY).unwrap();
            let expected = if n == 254 {
                DataType::Dictionary(Box::new(DataType::UInt8), Box::new(DataType::Utf8))
            } else if n == 256 {
                DataType::Dictionary(Box::new(DataType::UInt16), Box::new(DataType::Utf8))
            } else {
                DataType::Utf8
            };
            assert_eq!(key.data_type(), &expected);
            assert_eq!(StrCol::try_new(key.as_ref()).unwrap().value(n), "new");
        }
    }
}
