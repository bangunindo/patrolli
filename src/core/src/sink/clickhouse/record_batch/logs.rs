// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! `OtapArrowRecords::Logs` → the `otel_logs` record batch.
//!
//! The easy signal: OTAP's `Logs` batch is already one row per log record, which is exactly
//! the grain ClickHouse stores. So there is no join to do at row level — every scalar column
//! is a cast or a clone — and the only real work is pivoting the three attribute tables
//! (resource, scope, record) into `Map` columns and collapsing the `body` AnyValue struct
//! into a string.

use anyhow::{Context, Result};
use arrow::array::{Array, ArrayRef, AsArray, RecordBatch, StringBuilder, StructArray};
use otel_arrow_dfe_pdata::OtapArrowRecords;
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
use otel_arrow_dfe_pdata::schema::consts;
use std::sync::Arc;

use super::ChTable;
use super::attrs::{self, AnyValueCols};
use super::common::{
    col, hex_ids, ids_as_u64, struct_field, struct_ids, ts_nanos, u8_or_zero, utf8_or_empty,
};

/// OTAP names the log record's W3C trace flags `flags`; the upstream ClickHouse exporter
/// reads `span_flags` instead. Neither is wrong for every producer, so try both.
const SPAN_FLAGS_ALT: &str = "span_flags";

pub(super) fn build(records: &OtapArrowRecords) -> Result<Vec<(ChTable, RecordBatch)>> {
    let Some(root) = records.get(ArrowPayloadType::Logs) else {
        return Ok(Vec::new());
    };
    let len = root.num_rows();
    if len == 0 {
        return Ok(Vec::new());
    }

    // The three attribute joins. Resource and scope hang off the ids embedded in their
    // struct columns; log attributes hang off the record's own `id`.
    let resource_ids = struct_ids(
        struct_field(root, consts::RESOURCE, consts::ID).as_ref(),
        len,
    )?;
    let scope_ids = struct_ids(struct_field(root, consts::SCOPE, consts::ID).as_ref(), len)?;
    let log_ids: Vec<Option<u64>> = ids_as_u64(col(root, consts::ID), len)?
        .into_iter()
        .map(Some)
        .collect();

    let resource_attrs = attrs::maps_for(records, ArrowPayloadType::ResourceAttrs, &resource_ids)?;
    let scope_attrs = attrs::maps_for(records, ArrowPayloadType::ScopeAttrs, &scope_ids)?;
    let log_attrs = attrs::maps_for(records, ArrowPayloadType::LogAttrs, &log_ids)?;

    let service_name = super::common::map_value(&resource_attrs, "service.name")?;

    let flags = col(root, consts::FLAGS).or_else(|| col(root, SPAN_FLAGS_ALT));

    let columns: Vec<ArrayRef> = vec![
        ts_nanos(col(root, consts::TIME_UNIX_NANO), len)?,
        hex_ids(col(root, consts::TRACE_ID), len)?,
        hex_ids(col(root, consts::SPAN_ID), len)?,
        u8_or_zero(flags, len)?,
        utf8_or_empty(col(root, consts::SEVERITY_TEXT), len)?,
        u8_or_zero(col(root, consts::SEVERITY_NUMBER), len)?,
        service_name,
        body_strings(root, len)?,
        utf8_or_empty(
            struct_field(root, consts::RESOURCE, consts::SCHEMA_URL).as_ref(),
            len,
        )?,
        Arc::new(resource_attrs),
        // The root `schema_url` column is the *scope* schema url; the resource's own lives
        // inside the resource struct. Same split the JSON serializer makes.
        utf8_or_empty(col(root, consts::SCHEMA_URL), len)?,
        utf8_or_empty(
            struct_field(root, consts::SCOPE, consts::NAME).as_ref(),
            len,
        )?,
        utf8_or_empty(
            struct_field(root, consts::SCOPE, consts::VERSION).as_ref(),
            len,
        )?,
        Arc::new(scope_attrs),
        Arc::new(log_attrs),
        utf8_or_empty(col(root, consts::EVENT_NAME), len)?,
    ];

    let batch = RecordBatch::try_new(ChTable::Logs.schema(), columns)
        .context("logs columns do not match the ClickHouse schema")?;
    Ok(vec![(ChTable::Logs, batch)])
}

/// Collapse the `body` AnyValue struct into the single `String` column ClickHouse stores.
///
/// A null body row renders as `""` — the proto default, and the only option for a
/// non-`Nullable` column.
fn body_strings(root: &RecordBatch, len: usize) -> Result<ArrayRef> {
    let body: Option<&StructArray> = col(root, consts::BODY).and_then(|c| c.as_struct_opt());
    let Some(body) = body else {
        return utf8_or_empty(None, len);
    };
    let cols = AnyValueCols::new(|n| body.column_by_name(n).cloned(), len)?;
    let mut out = StringBuilder::with_capacity(len, len * 32);
    for row in 0..len {
        if body.is_valid(row) {
            cols.append_value(row, &mut out);
        } else {
            out.append_value("");
        }
    }
    Ok(Arc::new(out.finish()))
}

#[cfg(test)]
mod tests {
    use super::super::tests_support as support;
    use super::*;

    #[test]
    fn builds_a_schema_conformant_null_free_batch() {
        let mut pl = support::logs_payload();
        let out = super::super::to_record_batches(&mut pl).unwrap();
        assert_eq!(out.len(), 1);
        let (table, batch) = &out[0];
        assert_eq!(*table, ChTable::Logs);
        assert_eq!(batch.schema(), ChTable::Logs.schema());
        support::assert_no_nulls(batch);
        assert_eq!(batch.num_rows(), 2);
    }

    #[test]
    fn scalar_columns_carry_the_record_values() {
        let mut pl = support::logs_payload();
        let out = super::super::to_record_batches(&mut pl).unwrap();
        let batch = &out[0].1;

        assert_eq!(support::str_col(batch, "Body"), vec!["hello", "world"]);
        assert_eq!(
            support::str_col(batch, "SeverityText"),
            vec!["INFO", "ERROR"]
        );
        assert_eq!(support::str_col(batch, "EventName"), vec!["ev", ""]);
        // 16-byte trace ids render as 32 hex chars; the absent one is empty.
        let trace_ids = support::str_col(batch, "TraceId");
        assert_eq!(trace_ids[0].len(), 32);
        assert!(trace_ids[0].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn attributes_land_on_the_right_rows() {
        let mut pl = support::logs_payload();
        let out = super::super::to_record_batches(&mut pl).unwrap();
        let batch = &out[0].1;

        // Record attributes differ per row.
        assert_eq!(
            support::map_col(batch, "LogAttributes", 0),
            vec![("source.address".to_string(), "10.0.0.1".to_string())]
        );
        assert_eq!(
            support::map_col(batch, "LogAttributes", 1),
            vec![("source.address".to_string(), "10.0.0.2".to_string())]
        );
        // Resource attributes are shared by both rows, and drive ServiceName.
        for row in 0..2 {
            assert!(
                support::map_col(batch, "ResourceAttributes", row)
                    .contains(&("service.name".to_string(), "patrolli".to_string()))
            );
        }
        assert_eq!(
            support::str_col(batch, "ServiceName"),
            vec!["patrolli", "patrolli"]
        );
        assert_eq!(
            support::map_col(batch, "ScopeAttributes", 0),
            vec![("patrolli.node.id".to_string(), "n1".to_string())]
        );
    }

    /// A payload that has round-tripped a disk channel or the cluster forwarder arrives with
    /// delta-encoded parent ids. If those were read literally the attribute joins would
    /// silently misalign — every record would get some other record's attributes — so this is
    /// the regression test for that decode.
    #[test]
    fn survives_a_transport_encoded_round_trip() {
        let direct = super::super::to_record_batches(&mut support::logs_payload()).unwrap();
        let mut round_tripped = support::round_trip(&support::logs_payload());
        let after = super::super::to_record_batches(&mut round_tripped).unwrap();

        assert_eq!(after.len(), direct.len());
        support::assert_equivalent(&after[0].1, &direct[0].1);
    }

    #[test]
    fn empty_payload_yields_nothing() {
        let mut pl = support::empty_logs_payload();
        assert!(super::super::to_record_batches(&mut pl).unwrap().is_empty());
    }
}
