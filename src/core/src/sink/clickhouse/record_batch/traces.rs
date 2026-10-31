// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! `OtapArrowRecords::Traces` → the `otel_traces` record batch.
//!
//! Like logs, the `Spans` batch is already at ClickHouse row grain, so the scalar columns are
//! casts and clones. What is new here is the two child tables: `SpanEvents` and `SpanLinks`
//! hang off a span by `parent_id`, and ClickHouse stores them as `Nested(...)` — which under
//! the default `flatten_nested = 1` is a set of `Array(...)` columns, one per member.
//!
//! Those are built with [`ChildLists`]: one gather index over the child batch in span order,
//! plus the list offsets. Every member column is then a single `take` followed by wrapping
//! the result in a `ListArray` that reuses those offsets — including the child's own
//! attribute maps, which ride the same index.

use anyhow::{Context, Result};
use arrow::array::{ArrayRef, RecordBatch};
use otel_arrow_dfe_pdata::OtapArrowRecords;
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
use otel_arrow_dfe_pdata::schema::consts;
use std::sync::Arc;

use super::ChTable;
use super::attrs;
use super::child::ChildLists;
use super::common::{
    col, duration_nanos, enum_to_string, hex_ids, ids_as_u64, map_value, struct_field, struct_ids,
    ts_nanos, utf8_or_empty,
};

/// `SpanKind` as the Go ClickHouse exporter writes it (`traceutil.SpanKindStr`).
///
/// Deliberately **not** the proto `as_str_name()` form (`SPAN_KIND_SERVER`) that the Rust
/// reference exporter emits. `ddl.rs` transcribes the *Go* exporter's schema precisely so the
/// community Grafana dashboards work against these tables, and those dashboards filter on
/// `SpanKind = 'Server'`. Column names alone are not enough — the values have to match too.
fn span_kind_str(v: i32) -> &'static str {
    match v {
        0 => "Unspecified",
        1 => "Internal",
        2 => "Server",
        3 => "Client",
        4 => "Producer",
        5 => "Consumer",
        _ => "",
    }
}

/// `StatusCode` as the Go exporter writes it (`traceutil.StatusCodeStr`). See
/// [`span_kind_str`] for why this is not the proto name.
fn status_code_str(v: i32) -> &'static str {
    match v {
        0 => "Unset",
        1 => "Ok",
        2 => "Error",
        _ => "",
    }
}

pub(super) fn build(records: &OtapArrowRecords) -> Result<Vec<(ChTable, RecordBatch)>> {
    let Some(root) = records.get(ArrowPayloadType::Spans) else {
        return Ok(Vec::new());
    };
    let len = root.num_rows();
    if len == 0 {
        return Ok(Vec::new());
    }

    let resource_ids = struct_ids(
        struct_field(root, consts::RESOURCE, consts::ID).as_ref(),
        len,
    )?;
    let span_ids = ids_as_u64(col(root, consts::ID), len)?;
    let span_parents: Vec<Option<u64>> = span_ids.iter().copied().map(Some).collect();

    // Note there is no `ScopeAttributes` here, and so no `scope.id` lookup either: unlike logs
    // and metrics, the upstream traces table carries only `ScopeName` / `ScopeVersion`, so
    // `ScopeAttrs` is deliberately not pivoted. Emitting it would fail the insert with
    // `NO_SUCH_COLUMN_IN_TABLE`.
    let resource_attrs = attrs::maps_for(records, ArrowPayloadType::ResourceAttrs, &resource_ids)?;
    let span_attrs = attrs::maps_for(records, ArrowPayloadType::SpanAttrs, &span_parents)?;
    let service_name = map_value(&resource_attrs, "service.name")?;

    let events = ChildLists::new(
        records,
        ArrowPayloadType::SpanEvents,
        ArrowPayloadType::SpanEventAttrs,
        &span_ids,
    )?;
    let links = ChildLists::new(
        records,
        ArrowPayloadType::SpanLinks,
        ArrowPayloadType::SpanLinkAttrs,
        &span_ids,
    )?;

    let columns: Vec<ArrayRef> = vec![
        ts_nanos(col(root, consts::START_TIME_UNIX_NANO), len)?,
        hex_ids(col(root, consts::TRACE_ID), len)?,
        hex_ids(col(root, consts::SPAN_ID), len)?,
        hex_ids(col(root, consts::PARENT_SPAN_ID), len)?,
        utf8_or_empty(col(root, consts::TRACE_STATE), len)?,
        utf8_or_empty(col(root, consts::NAME), len)?,
        enum_to_string(col(root, consts::KIND), len, span_kind_str)?,
        service_name,
        Arc::new(resource_attrs),
        utf8_or_empty(
            struct_field(root, consts::SCOPE, consts::NAME).as_ref(),
            len,
        )?,
        utf8_or_empty(
            struct_field(root, consts::SCOPE, consts::VERSION).as_ref(),
            len,
        )?,
        Arc::new(span_attrs),
        duration_nanos(col(root, consts::DURATION_TIME_UNIX_NANO), len)?,
        enum_to_string(
            struct_field(root, consts::STATUS, consts::STATUS_CODE).as_ref(),
            len,
            status_code_str,
        )?,
        utf8_or_empty(
            struct_field(root, consts::STATUS, consts::STATUS_MESSAGE).as_ref(),
            len,
        )?,
        // Events Nested(...)
        events.list(events.ts_nanos(consts::TIME_UNIX_NANO)?)?,
        events.list(events.utf8(consts::NAME)?)?,
        events.list(events.attributes())?,
        // Links Nested(...)
        links.list(links.hex(consts::TRACE_ID)?)?,
        links.list(links.hex(consts::SPAN_ID)?)?,
        links.list(links.utf8(consts::TRACE_STATE)?)?,
        links.list(links.attributes())?,
    ];

    let batch = RecordBatch::try_new(ChTable::Traces.schema(), columns)
        .context("traces columns do not match the ClickHouse schema")?;
    Ok(vec![(ChTable::Traces, batch)])
}

#[cfg(test)]
mod tests {
    use super::super::tests_support as support;
    use super::*;

    fn batch() -> RecordBatch {
        let mut pl = support::traces_payload();
        let out = super::super::to_record_batches(&mut pl).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, ChTable::Traces);
        out.into_iter().next().unwrap().1
    }

    #[test]
    fn builds_a_schema_conformant_null_free_batch() {
        let b = batch();
        assert_eq!(b.schema(), ChTable::Traces.schema());
        support::assert_no_nulls(&b);
        assert_eq!(b.num_rows(), 2);
    }

    #[test]
    fn scalar_columns_use_the_go_exporter_spellings() {
        let b = batch();
        let row = support::row_of(&b, "SpanName", "GET /x");
        assert_eq!(support::str_col(&b, "SpanKind")[row], "Server");
        assert_eq!(support::str_col(&b, "StatusCode")[row], "Error");
        assert_eq!(support::str_col(&b, "StatusMessage")[row], "boom");
        assert_eq!(support::str_col(&b, "TraceState")[row], "a=b");
        assert_eq!(support::str_col(&b, "ServiceName")[row], "patrolli");

        let other = 1 - row;
        assert_eq!(support::str_col(&b, "SpanKind")[other], "Internal");
        // No status on the second span: the proto default renders as "Unset", not "".
        assert_eq!(support::str_col(&b, "StatusCode")[other], "Unset");
    }

    #[test]
    fn duration_is_end_minus_start_in_nanos() {
        let b = batch();
        let row = support::row_of(&b, "SpanName", "GET /x");
        assert_eq!(support::u64_col(&b, "Duration")[row], 500);
        assert_eq!(support::u64_col(&b, "Duration")[1 - row], 250);
    }

    #[test]
    fn ids_are_lowercase_hex_of_the_right_width() {
        let b = batch();
        let row = support::row_of(&b, "SpanName", "GET /x");
        assert_eq!(support::str_col(&b, "TraceId")[row], "11".repeat(16));
        assert_eq!(support::str_col(&b, "SpanId")[row], "22".repeat(8));
        assert_eq!(support::str_col(&b, "ParentSpanId")[row], "33".repeat(8));
        // The second span has no parent, so the column is empty rather than null.
        assert_eq!(support::str_col(&b, "ParentSpanId")[1 - row], "");
    }

    #[test]
    fn events_and_links_become_per_span_lists() {
        let b = batch();
        let row = support::row_of(&b, "SpanName", "GET /x");
        let other = 1 - row;

        assert_eq!(support::list_str(&b, "Events.Name", row), vec!["exception"]);
        assert_eq!(
            support::list_maps(&b, "Events.Attributes", row),
            vec![vec![("exception.type".to_string(), "IOError".to_string())]]
        );

        assert_eq!(
            support::list_str(&b, "Links.TraceId", row),
            vec!["44".repeat(16)]
        );
        assert_eq!(
            support::list_str(&b, "Links.SpanId", row),
            vec!["55".repeat(8)]
        );
        assert_eq!(support::list_str(&b, "Links.TraceState", row), vec!["c=d"]);
        assert_eq!(
            support::list_maps(&b, "Links.Attributes", row),
            vec![vec![("link.kind".to_string(), "follows".to_string())]]
        );

        // The span with no children gets empty lists, not nulls.
        assert!(support::list_str(&b, "Events.Name", other).is_empty());
        assert!(support::list_str(&b, "Links.TraceId", other).is_empty());
    }

    #[test]
    fn span_attributes_land_on_the_right_row() {
        let b = batch();
        let row = support::row_of(&b, "SpanName", "GET /x");
        assert_eq!(
            support::map_col(&b, "SpanAttributes", row),
            vec![("http.method".to_string(), "GET".to_string())]
        );
        assert!(support::map_col(&b, "SpanAttributes", 1 - row).is_empty());
    }

    #[test]
    fn survives_a_transport_encoded_round_trip() {
        let direct = batch();
        let mut round_tripped = support::round_trip(&support::traces_payload());
        let after = super::super::to_record_batches(&mut round_tripped).unwrap();
        support::assert_equivalent(&after[0].1, &direct);
    }

    #[test]
    fn empty_payload_yields_nothing() {
        let mut pl = support::empty_traces_payload();
        assert!(super::super::to_record_batches(&mut pl).unwrap().is_empty());
    }
}
