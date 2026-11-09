// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Reshaping an OTAP child table into the per-parent `Array(...)` columns ClickHouse's
//! `Nested(...)` columns expand to.
//!
//! Three places need this and they are structurally identical: span events, span links, and
//! data-point exemplars. Each is a side table whose rows point at a parent by `parent_id`,
//! and each becomes a group of `Array(...)` columns on the parent's row.
//!
//! [`ChildLists`] computes the join once — a gather index listing every child row grouped by
//! its parent, in parent order, plus the matching list offsets. After that each member column
//! costs one `take` and a `ListArray` wrap that reuses those offsets, so nothing is copied
//! per row.

use anyhow::Result;
use arrow::array::{ArrayRef, ListArray, MapArray, RecordBatch, UInt32Array};
use arrow::buffer::OffsetBuffer;
use arrow::compute::kernels::take::take;
use arrow::datatypes::Field;
use otel_arrow_dfe_pdata::OtapArrowRecords;
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
use otel_arrow_dfe_pdata::schema::consts;
use rustc_hash::FxHashMap;
use std::sync::Arc;

use super::attrs;
use super::common::{
    col, hex_ids, ids_as_u64, number_value, parent_ids_as_u64, ts_nanos, ts_secs, utf8_or_empty,
};

pub(super) struct ChildLists<'a> {
    batch: Option<&'a RecordBatch>,
    /// Rows of `batch`, ordered so each parent's children are contiguous.
    indices: UInt32Array,
    /// `num_parents + 1` boundaries into `indices`.
    offsets: OffsetBuffer<i32>,
    /// The children's own attributes, at child-row grain, ready to be gathered.
    attrs: MapArray,
}

impl<'a> ChildLists<'a> {
    /// Join `child_pt` to `parent_ids` (the parents' `id` values, in parent row order).
    ///
    /// `attr_pt` is the child's attribute table, which hangs off the child's own `id` — a
    /// u32-wide parent, which is the width the reference exporter's attribute grouper cannot
    /// handle at all.
    pub(super) fn new(
        records: &'a OtapArrowRecords,
        child_pt: ArrowPayloadType,
        attr_pt: ArrowPayloadType,
        parent_ids: &[u64],
    ) -> Result<Self> {
        let Some(batch) = records.get(child_pt) else {
            return Ok(Self {
                batch: None,
                indices: UInt32Array::from(Vec::<u32>::new()),
                offsets: OffsetBuffer::new(vec![0i32; parent_ids.len() + 1].into()),
                attrs: attrs::empty_maps(0),
            });
        };

        let rows = batch.num_rows();
        let parents = parent_ids_as_u64(col(batch, consts::PARENT_ID), rows)?;

        let mut by_parent: FxHashMap<u64, Vec<u32>> = FxHashMap::default();
        for (row, parent) in parents.iter().enumerate() {
            if let Some(p) = parent {
                by_parent.entry(*p).or_default().push(row as u32);
            }
        }

        let mut indices: Vec<u32> = Vec::with_capacity(rows);
        let mut offsets: Vec<i32> = Vec::with_capacity(parent_ids.len() + 1);
        offsets.push(0);
        for parent_id in parent_ids {
            if let Some(child_rows) = by_parent.get(parent_id) {
                indices.extend_from_slice(child_rows);
            }
            offsets.push(indices.len() as i32);
        }

        let child_ids: Vec<Option<u64>> = ids_as_u64(col(batch, consts::ID), rows)?
            .into_iter()
            .map(Some)
            .collect();
        let attrs = attrs::maps_for(records, attr_pt, &child_ids)?;

        Ok(Self {
            batch: Some(batch),
            indices: UInt32Array::from(indices),
            offsets: OffsetBuffer::new(offsets.into()),
            attrs,
        })
    }

    fn rows(&self) -> usize {
        self.batch.map_or(0, |b| b.num_rows())
    }

    fn column(&self, name: &str) -> Option<&ArrayRef> {
        self.batch.and_then(|b| col(b, name))
    }

    // Each of these builds one child-grain column with defaults filled, ready for `list`.

    pub(super) fn ts_nanos(&self, name: &str) -> Result<ArrayRef> {
        ts_nanos(self.column(name), self.rows())
    }

    pub(super) fn ts_secs(&self, name: &str) -> Result<ArrayRef> {
        ts_secs(self.column(name), self.rows())
    }

    pub(super) fn utf8(&self, name: &str) -> Result<ArrayRef> {
        utf8_or_empty(self.column(name), self.rows())
    }

    pub(super) fn hex(&self, name: &str) -> Result<ArrayRef> {
        hex_ids(self.column(name), self.rows())
    }

    /// The child's `int_value`/`double_value` pair merged into one `Float64` — exemplars use
    /// the same either-or representation as number data points.
    pub(super) fn number_value(&self) -> Result<ArrayRef> {
        number_value(
            self.column(consts::INT_VALUE),
            self.column(consts::DOUBLE_VALUE),
            self.rows(),
        )
    }

    pub(super) fn attributes(&self) -> ArrayRef {
        Arc::new(self.attrs.clone())
    }

    /// Gather a child-grain column into one list per parent.
    pub(super) fn list(&self, values: ArrayRef) -> Result<ArrayRef> {
        let gathered = take(values.as_ref(), &self.indices, None)?;
        let field = Arc::new(Field::new("item", gathered.data_type().clone(), true));
        Ok(Arc::new(ListArray::new(
            field,
            self.offsets.clone(),
            gathered,
            None,
        )))
    }
}
