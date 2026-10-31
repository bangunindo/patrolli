// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Convert a [`Payload`] into the ClickHouse-shaped Arrow record batches the exporter
//! schema in [`super::ddl`] expects.
//!
//! # Why Arrow and not rows
//!
//! OTAP is normalized: a root batch (Logs / UnivariateMetrics / Spans) plus side tables
//! joined by `parent_id`. The ClickHouse tables are denormalized — one row per log record,
//! span, or metric *data point*, with resource/scope/record attributes inlined as
//! `Map(LowCardinality(String), String)`. Going through Arrow rather than a row struct means
//! most of that work is whole-column:
//!
//! - columns that already line up (log `Timestamp`, data-point `Flags`, histogram bucket
//!   lists) are moved by cloning an `ArrayRef` — no per-row copy at all;
//! - the transformations that remain are arrow kernels: `take` broadcasts metric-level
//!   columns down to data-point grain, `filter` splits gauge from sum, `cast`/`zip`
//!   normalize types and fill defaults.
//!
//! Only the attribute pivot is genuinely row-wise, because it turns a variant `AnyValue`
//! column into strings.
//!
//! # Insert contract
//!
//! `clickhouse-ext-arrow` builds `INSERT INTO t(c1, c2, …) FORMAT ArrowStream` from the
//! *first* batch's schema and forbids the schema changing between batches of one insert. So
//! every batch this module returns for a given table carries exactly [`ChTable::schema`] —
//! all columns, same order, same types, no nulls. See [`schema`] for the details.
//!
//! Wiring this into the sink's insert loop is a separate concern; this module only builds
//! the batches.

mod attrs;
mod child;
mod common;
mod logs;
mod metrics;
mod schema;
#[cfg(test)]
mod tests_support;
mod traces;

use crate::config::sink::ClickhouseTables;
use crate::payload::Payload;
use anyhow::{Context, Result};
use arrow::array::RecordBatch;
use arrow::datatypes::SchemaRef;
use otel_arrow_dfe_pdata::OtapArrowRecords;

/// One insertable ClickHouse destination.
///
/// `otel_traces_trace_id_ts` is deliberately absent: it is populated by a materialized view
/// over the traces table, never inserted into directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChTable {
    Logs,
    Traces,
    MetricsGauge,
    MetricsSum,
    MetricsSummary,
    MetricsHistogram,
    MetricsExpHistogram,
}

impl ChTable {
    /// Every destination, in the order [`super::ddl::Ddl::statements`] creates them.
    ///
    /// The sink routes by whatever [`to_record_batches`] returns rather than walking this, so
    /// outside tests it goes unread — it exists so the schema and table-name tests can assert
    /// they cover every variant.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const ALL: [ChTable; 7] = [
        ChTable::Logs,
        ChTable::Traces,
        ChTable::MetricsGauge,
        ChTable::MetricsSum,
        ChTable::MetricsSummary,
        ChTable::MetricsHistogram,
        ChTable::MetricsExpHistogram,
    ];

    /// The configured table name.
    ///
    /// Reads from the same [`ClickhouseTables`] the DDL renders its names from, so a
    /// renamed table stays consistent between `CREATE` and `INSERT`.
    pub fn name<'a>(&self, tables: &'a ClickhouseTables) -> &'a str {
        match self {
            ChTable::Logs => &tables.logs,
            ChTable::Traces => &tables.traces,
            ChTable::MetricsGauge => &tables.metrics_gauge,
            ChTable::MetricsSum => &tables.metrics_sum,
            ChTable::MetricsSummary => &tables.metrics_summary,
            ChTable::MetricsHistogram => &tables.metrics_histogram,
            ChTable::MetricsExpHistogram => &tables.metrics_exp_histogram,
        }
    }

    /// The canonical schema for this table.
    ///
    /// Cheap: a clone of a process-wide `LazyLock`, so all batches for a table share one
    /// `SchemaRef` and can stream into a single `ArrowInsert`.
    pub fn schema(&self) -> SchemaRef {
        match self {
            ChTable::Logs => schema::LOGS.clone(),
            ChTable::Traces => schema::TRACES.clone(),
            ChTable::MetricsGauge => schema::METRICS_GAUGE.clone(),
            ChTable::MetricsSum => schema::METRICS_SUM.clone(),
            ChTable::MetricsSummary => schema::METRICS_SUMMARY.clone(),
            ChTable::MetricsHistogram => schema::METRICS_HISTOGRAM.clone(),
            ChTable::MetricsExpHistogram => schema::METRICS_EXP_HISTOGRAM.clone(),
        }
    }
}

/// Convert one payload into the ClickHouse batches it maps to.
///
/// Logs and traces yield at most one entry; metrics yield up to five, one per data-point
/// type present (with `NumberDataPoints` fanning out to both gauge and sum). Tables with no
/// rows are omitted, so an empty payload yields an empty `Vec`.
///
/// Takes `&mut` because the first thing it does is materialize transport-optimized ids —
/// see [`decode_ids`].
pub fn to_record_batches(payload: &mut Payload) -> Result<Vec<(ChTable, RecordBatch)>> {
    if payload.is_empty() {
        return Ok(Vec::new());
    }
    decode_ids(payload)?;

    match &payload.data {
        OtapArrowRecords::Logs(_) => logs::build(&payload.data),
        OtapArrowRecords::Traces(_) => traces::build(&payload.data),
        OtapArrowRecords::Metrics(_) => metrics::build(&payload.data),
    }
}

/// Materialize transport-optimized (delta / quasi-delta encoded) `parent_id` columns.
///
/// Every join in this module reads `parent_id` as a literal id, so this has to have
/// happened first. `src/listener/otel.rs` does it on ingest, but a payload that round-trips
/// a disk channel or the inter-node forwarder is re-encoded by `Producer::produce_bar` on
/// the way out and arrives here encoded again — `Payload::from_bytes` decodes on the way
/// back in, and this call covers anything that reaches the sink by another route.
///
/// Idempotent: each decoder checks the `column_encoding` field metadata and clones through
/// when the column is already plain, so the common case costs a schema check.
fn decode_ids(payload: &mut Payload) -> Result<()> {
    payload
        .data
        .decode_transport_optimized_ids()
        .context("failed to decode transport-optimized ids")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::sink::ClickhouseTables;

    #[test]
    fn table_names_track_the_config() {
        let tables = ClickhouseTables::default();
        assert_eq!(ChTable::Logs.name(&tables), "otel_logs");
        assert_eq!(ChTable::Traces.name(&tables), "otel_traces");
        assert_eq!(ChTable::MetricsGauge.name(&tables), "otel_metrics_gauge");
        assert_eq!(
            ChTable::MetricsExpHistogram.name(&tables),
            "otel_metrics_exp_histogram"
        );

        let renamed = ClickhouseTables {
            logs: "my_logs".to_string(),
            ..ClickhouseTables::default()
        };
        assert_eq!(ChTable::Logs.name(&renamed), "my_logs");
    }

    /// Schemas are shared statics, not rebuilt per call — an `ArrowInsert` relies on the
    /// schema being identical across every batch it is fed.
    #[test]
    fn schemas_are_shared_statics() {
        for table in ChTable::ALL {
            assert!(std::sync::Arc::ptr_eq(&table.schema(), &table.schema()));
        }
    }

    #[test]
    fn all_covers_every_variant() {
        // Compile-time exhaustiveness: adding a variant without adding it to ALL fails here.
        for table in ChTable::ALL {
            match table {
                ChTable::Logs
                | ChTable::Traces
                | ChTable::MetricsGauge
                | ChTable::MetricsSum
                | ChTable::MetricsSummary
                | ChTable::MetricsHistogram
                | ChTable::MetricsExpHistogram => {}
            }
        }
        assert_eq!(ChTable::ALL.len(), 7);
    }
}
