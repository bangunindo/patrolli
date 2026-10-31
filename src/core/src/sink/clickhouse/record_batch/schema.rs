// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Canonical Arrow schemas for the ClickHouse exporter tables.
//!
//! One schema per insertable table in [`super::super::ddl`], column-for-column. These are
//! the contract the builders in `logs`/`traces`/`metrics` must satisfy exactly, because
//! `clickhouse-ext-arrow` derives its `INSERT INTO t(c1, c2, …) FORMAT ArrowStream` column
//! list from the *first* batch's schema and forbids the schema changing between batches of
//! one insert. Handing out a clone of a `LazyLock` static is what makes a whole stream of
//! batches safe to feed to a single `ArrowInsert`.
//!
//! Three rules follow from the DDL and are enforced here:
//!
//! - **Names are byte-identical to the DDL.** Columns bind by name; an unknown name fails
//!   the insert at `end()`. ClickHouse's default `flatten_nested = 1` means a `Nested(…)`
//!   column really is a set of `Array(…)` columns named `Parent.Member`, so that is what we
//!   emit — `Events.Timestamp`, `Exemplars.Value`, `ValueAtQuantiles.Quantile`, …
//! - **The logs table's eight `__patrolli_materialized.*` columns are absent.** They are
//!   `MATERIALIZED`, so ClickHouse computes them and rejects any attempt to insert them.
//! - **`LowCardinality(String)` is written as plain `Utf8`.** ClickHouse accepts an Arrow
//!   dictionary here too, but a plain string column keeps the IPC stream free of dictionary
//!   replacement batches and keeps the schema trivially stable across batches. The
//!   server-side column stays `LowCardinality` either way.
//!
//! Fields are declared nullable — mirroring the upstream exporter, whose end-to-end tests
//! run against a live server — but the builders never actually emit a null: no ClickHouse
//! column here is `Nullable`, so every OTAP-omitted value is materialized to its proto
//! default instead of relying on `input_format_null_as_default`.

use arrow::datatypes::{DataType, Field, Fields, Schema, SchemaRef, TimeUnit};
use std::sync::{Arc, LazyLock};

/// `DateTime64(9)` — the log/span timestamp columns.
pub(super) fn ts_nanos() -> DataType {
    DataType::Timestamp(TimeUnit::Nanosecond, None)
}

/// `DateTime` — every metric timestamp column is second precision, not nanosecond.
pub(super) fn ts_secs() -> DataType {
    DataType::Timestamp(TimeUnit::Second, None)
}

/// `Map(LowCardinality(String), String)`.
///
/// Spelled out rather than derived so it matches what `MapBuilder::new(None, …)` produces
/// (`entries`/`keys`/`values`, non-null entries, non-null keys) — otherwise
/// `RecordBatch::try_new` rejects the column for a field mismatch.
pub(super) fn attr_map() -> DataType {
    DataType::Map(
        Arc::new(Field::new(
            "entries",
            DataType::Struct(Fields::from(vec![
                Field::new("keys", DataType::Utf8, false),
                Field::new("values", DataType::Utf8, true),
            ])),
            false,
        )),
        false,
    )
}

/// `Array(T)`, with arrow's conventional `item` element name.
pub(super) fn list_of(dt: DataType) -> DataType {
    DataType::List(Arc::new(Field::new("item", dt, true)))
}

fn f(name: &str, dt: DataType) -> Field {
    Field::new(name, dt, true)
}

// ---------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------

pub(super) static LOGS: LazyLock<SchemaRef> = LazyLock::new(|| {
    Arc::new(Schema::new(vec![
        f("Timestamp", ts_nanos()),
        f("TraceId", DataType::Utf8),
        f("SpanId", DataType::Utf8),
        f("TraceFlags", DataType::UInt8),
        f("SeverityText", DataType::Utf8),
        f("SeverityNumber", DataType::UInt8),
        f("ServiceName", DataType::Utf8),
        f("Body", DataType::Utf8),
        f("ResourceSchemaUrl", DataType::Utf8),
        f("ResourceAttributes", attr_map()),
        f("ScopeSchemaUrl", DataType::Utf8),
        f("ScopeName", DataType::Utf8),
        f("ScopeVersion", DataType::Utf8),
        f("ScopeAttributes", attr_map()),
        f("LogAttributes", attr_map()),
        f("EventName", DataType::Utf8),
    ]))
});

// ---------------------------------------------------------------------------
// Traces
// ---------------------------------------------------------------------------

pub(super) static TRACES: LazyLock<SchemaRef> = LazyLock::new(|| {
    Arc::new(Schema::new(vec![
        f("Timestamp", ts_nanos()),
        f("TraceId", DataType::Utf8),
        f("SpanId", DataType::Utf8),
        f("ParentSpanId", DataType::Utf8),
        f("TraceState", DataType::Utf8),
        f("SpanName", DataType::Utf8),
        f("SpanKind", DataType::Utf8),
        f("ServiceName", DataType::Utf8),
        f("ResourceAttributes", attr_map()),
        f("ScopeName", DataType::Utf8),
        f("ScopeVersion", DataType::Utf8),
        f("SpanAttributes", attr_map()),
        f("Duration", DataType::UInt64),
        f("StatusCode", DataType::Utf8),
        f("StatusMessage", DataType::Utf8),
        // `Events Nested(...)` / `Links Nested(...)`, flattened.
        f("Events.Timestamp", list_of(ts_nanos())),
        f("Events.Name", list_of(DataType::Utf8)),
        f("Events.Attributes", list_of(attr_map())),
        f("Links.TraceId", list_of(DataType::Utf8)),
        f("Links.SpanId", list_of(DataType::Utf8)),
        f("Links.TraceState", list_of(DataType::Utf8)),
        f("Links.Attributes", list_of(attr_map())),
    ]))
});

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

/// `METRICS_IDENTITY_COLUMNS` — the resource/scope/metric envelope every metric table
/// opens with, in DDL order.
fn metrics_identity() -> Vec<Field> {
    vec![
        f("ResourceAttributes", attr_map()),
        f("ResourceSchemaUrl", DataType::Utf8),
        f("ScopeName", DataType::Utf8),
        f("ScopeVersion", DataType::Utf8),
        f("ScopeAttributes", attr_map()),
        f("ScopeDroppedAttrCount", DataType::UInt32),
        f("ScopeSchemaUrl", DataType::Utf8),
        f("ServiceName", DataType::Utf8),
        f("MetricName", DataType::Utf8),
        f("MetricDescription", DataType::Utf8),
        f("MetricUnit", DataType::Utf8),
        f("Attributes", attr_map()),
        f("StartTimeUnix", ts_secs()),
        f("TimeUnix", ts_secs()),
    ]
}

/// `Exemplars Nested(...)`, flattened. Shared by gauge, sum, histogram and exponential
/// histogram; summary is the one datapoint type that carries no exemplars.
fn exemplars() -> Vec<Field> {
    vec![
        f("Exemplars.FilteredAttributes", list_of(attr_map())),
        f("Exemplars.TimeUnix", list_of(ts_secs())),
        f("Exemplars.Value", list_of(DataType::Float64)),
        f("Exemplars.SpanId", list_of(DataType::Utf8)),
        f("Exemplars.TraceId", list_of(DataType::Utf8)),
    ]
}

fn metrics_schema(datapoint: Vec<Field>) -> SchemaRef {
    let mut fields = metrics_identity();
    fields.extend(datapoint);
    Arc::new(Schema::new(fields))
}

pub(super) static METRICS_GAUGE: LazyLock<SchemaRef> = LazyLock::new(|| {
    let mut dp = vec![f("Value", DataType::Float64), f("Flags", DataType::UInt32)];
    dp.extend(exemplars());
    metrics_schema(dp)
});

pub(super) static METRICS_SUM: LazyLock<SchemaRef> = LazyLock::new(|| {
    let mut dp = vec![f("Value", DataType::Float64), f("Flags", DataType::UInt32)];
    dp.extend(exemplars());
    dp.extend([
        f("AggregationTemporality", DataType::Int32),
        f("IsMonotonic", DataType::Boolean),
    ]);
    metrics_schema(dp)
});

pub(super) static METRICS_SUMMARY: LazyLock<SchemaRef> = LazyLock::new(|| {
    metrics_schema(vec![
        f("Count", DataType::UInt64),
        f("Sum", DataType::Float64),
        f("ValueAtQuantiles.Quantile", list_of(DataType::Float64)),
        f("ValueAtQuantiles.Value", list_of(DataType::Float64)),
        f("Flags", DataType::UInt32),
    ])
});

pub(super) static METRICS_HISTOGRAM: LazyLock<SchemaRef> = LazyLock::new(|| {
    let mut dp = vec![
        f("Count", DataType::UInt64),
        f("Sum", DataType::Float64),
        f("BucketCounts", list_of(DataType::UInt64)),
        f("ExplicitBounds", list_of(DataType::Float64)),
    ];
    dp.extend(exemplars());
    dp.extend([
        f("Flags", DataType::UInt32),
        f("Min", DataType::Float64),
        f("Max", DataType::Float64),
        f("AggregationTemporality", DataType::Int32),
    ]);
    metrics_schema(dp)
});

pub(super) static METRICS_EXP_HISTOGRAM: LazyLock<SchemaRef> = LazyLock::new(|| {
    let mut dp = vec![
        f("Count", DataType::UInt64),
        f("Sum", DataType::Float64),
        f("Scale", DataType::Int32),
        f("ZeroCount", DataType::UInt64),
        f("PositiveOffset", DataType::Int32),
        f("PositiveBucketCounts", list_of(DataType::UInt64)),
        f("NegativeOffset", DataType::Int32),
        f("NegativeBucketCounts", list_of(DataType::UInt64)),
    ];
    dp.extend(exemplars());
    dp.extend([
        f("Flags", DataType::UInt32),
        f("Min", DataType::Float64),
        f("Max", DataType::Float64),
        f("AggregationTemporality", DataType::Int32),
    ]);
    metrics_schema(dp)
});

#[cfg(test)]
mod tests {
    use super::super::ChTable;
    use crate::config::sink::ClickhouseTables;

    /// Every column the converter emits must exist in the `CREATE TABLE` the sink runs,
    /// otherwise `ArrowInsert::end()` fails with `NO_SUCH_COLUMN_IN_TABLE`. The DDL renders
    /// `Nested(...)` unflattened, so a nested member `Parent.Member` is checked as a bare
    /// `Member` inside the block — which is how ClickHouse expands it under the default
    /// `flatten_nested = 1`.
    #[test]
    fn every_emitted_column_exists_in_the_ddl() {
        use crate::config::sink::ClickhouseCreateSchema;
        use crate::sink::clickhouse::ddl::Ddl;

        let tables = ClickhouseTables::default();
        let cs = ClickhouseCreateSchema::default();
        let statements = Ddl::new("otel", &tables, &cs, false).statements();

        for table in ChTable::ALL {
            let name = table.name(&tables);
            let sql = statements
                .iter()
                .find(|(_, sql)| sql.contains(&format!("`otel`.`{name}` ")))
                .unwrap_or_else(|| panic!("no CREATE TABLE for {name}"));

            for field in table.schema().fields() {
                let col = field.name();
                let leaf = col.rsplit_once('.').map_or(col.as_str(), |(_, m)| m);
                assert!(
                    sql.1.contains(&format!("`{col}`")) || sql.1.contains(&format!(" {leaf} ")),
                    "{name} has no column {col}"
                );
            }
        }
    }

    /// The materialized columns are computed server-side; inserting them is an error.
    #[test]
    fn logs_omits_materialized_columns() {
        for field in ChTable::Logs.schema().fields() {
            assert!(
                !field.name().starts_with("__patrolli_materialized"),
                "{} must not be inserted",
                field.name()
            );
        }
        assert_eq!(ChTable::Logs.schema().fields().len(), 16);
    }

    /// All five metric tables open with the same 14-column envelope.
    #[test]
    fn metric_tables_share_the_identity_envelope() {
        let identity: Vec<_> = super::metrics_identity()
            .iter()
            .map(|f| f.name().clone())
            .collect();
        for table in [
            ChTable::MetricsGauge,
            ChTable::MetricsSum,
            ChTable::MetricsSummary,
            ChTable::MetricsHistogram,
            ChTable::MetricsExpHistogram,
        ] {
            let names: Vec<_> = table
                .schema()
                .fields()
                .iter()
                .take(identity.len())
                .map(|f| f.name().clone())
                .collect();
            assert_eq!(names, identity, "{table:?} lost the identity envelope");
        }
    }

    /// Summary is the only datapoint type without exemplars.
    #[test]
    fn only_summary_lacks_exemplars() {
        let has = |t: ChTable| {
            t.schema()
                .fields()
                .iter()
                .any(|f| f.name().starts_with("Exemplars."))
        };
        assert!(has(ChTable::MetricsGauge));
        assert!(has(ChTable::MetricsSum));
        assert!(has(ChTable::MetricsHistogram));
        assert!(has(ChTable::MetricsExpHistogram));
        assert!(!has(ChTable::MetricsSummary));
    }
}
