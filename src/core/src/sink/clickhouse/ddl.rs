// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! DDL for the OpenTelemetry Clickhouse exporter's schema.
//!
//! These are transcriptions of the exporter's `internal/sqltemplates/*.sql` — the
//! **`Map`** flavour, where attributes are `Map(LowCardinality(String), String)`, not the
//! newer `JSON`-typed variant. Keeping the column names, types, codecs, skip indexes,
//! partition and sort keys identical is the whole point: tables created here are the ones
//! the community Grafana dashboards and the Clickhouse datasource already know how to
//! query.
//!
//! Only whitespace differs from upstream (their templates carry some gofmt artifacts).
//! Insert-side compatibility follows from the column layout, so if you change anything
//! here, change it to match upstream — don't improve on it.

use crate::config::sink::{ClickhouseCreateSchema, ClickhouseTableEngine, ClickhouseTables};
use std::time::Duration;

/// Always present, so the exporter skips `CREATE DATABASE` for it — and so do we.
pub const DEFAULT_DATABASE: &str = "default";

/// First Clickhouse release with `TYPE text(...)` skip indexes. Below this the logs table
/// falls back to bloom filters, mirroring the exporter's own version probe.
pub const MIN_FULL_TEXT_SEARCH_VERSION: (u32, u32) = (26, 2);

/// Backtick-quote an identifier, doubling any embedded backtick.
fn quote_ident(ident: &str) -> String {
    format!("`{}`", ident.replace('`', "``"))
}

/// A statement paired with a label used for error context.
pub type Statement = (&'static str, String);

/// Renders every `CREATE` the exporter runs, bound to one sink's database, engine, cluster
/// and TTL. Build it once per endpoint and call [`Ddl::statements`].
pub struct Ddl<'a> {
    database: &'a str,
    tables: &'a ClickhouseTables,
    /// `ON CLUSTER `name`` or empty.
    cluster: String,
    /// `Engine(params)`, e.g. `MergeTree()`.
    engine: String,
    ttl: Duration,
    /// Whether the server supports `TYPE text(...)` indexes (logs table only).
    full_text_search: bool,
}

impl<'a> Ddl<'a> {
    pub fn new(
        database: &'a str,
        tables: &'a ClickhouseTables,
        create_schema: &ClickhouseCreateSchema,
        full_text_search: bool,
    ) -> Self {
        Self {
            database,
            tables,
            cluster: cluster_string(create_schema.cluster_name.as_deref()),
            engine: engine_string(&create_schema.table_engine),
            ttl: create_schema.ttl,
            full_text_search,
        }
    }

    /// Every statement, in dependency order: database, then logs, then the traces trio
    /// (table, trace-id lookup table, materialized view feeding it), then one table per
    /// metric datapoint type.
    pub fn statements(&self) -> Vec<Statement> {
        let mut out = Vec::with_capacity(10);
        if let Some(sql) = self.create_database() {
            out.push(("create database", sql));
        }
        out.push(("create logs table", self.logs_table()));
        out.push(("create traces table", self.traces_table()));
        out.push(("create trace id ts table", self.traces_id_ts_table()));
        out.push(("create trace id ts view", self.traces_id_ts_mv()));
        out.push(("create metrics gauge table", self.metrics_gauge_table()));
        out.push(("create metrics sum table", self.metrics_sum_table()));
        out.push(("create metrics summary table", self.metrics_summary_table()));
        out.push((
            "create metrics histogram table",
            self.metrics_histogram_table(),
        ));
        out.push((
            "create metrics exponential histogram table",
            self.metrics_exp_histogram_table(),
        ));
        out
    }

    fn qualified(&self, table: &str) -> String {
        format!("{}.{}", quote_ident(self.database), quote_ident(table))
    }

    fn create_database(&self) -> Option<String> {
        if self.database == DEFAULT_DATABASE {
            return None;
        }
        let name = quote_ident(self.database);
        let cluster = &self.cluster;
        Some(format!("CREATE DATABASE IF NOT EXISTS {name} {cluster}"))
    }

    fn logs_table(&self) -> String {
        let name = self.qualified(&self.tables.logs);
        let cluster = &self.cluster;
        let engine = &self.engine;
        let ttl = ttl_expr(self.ttl, "toDateTime(Timestamp)");
        let indexes = if self.full_text_search {
            LOGS_TEXT_INDEXES
        } else {
            LOGS_BLOOM_INDEXES
        };
        format!(
            r#"CREATE TABLE IF NOT EXISTS {name} {cluster} (
    `Timestamp` DateTime64(9) COMMENT 'Event timestamp with nanosecond precision' CODEC(Delta(8), ZSTD(1)),
    `TraceId` String COMMENT 'W3C trace identifier' CODEC(ZSTD(1)),
    `SpanId` String COMMENT 'W3C span identifier' CODEC(ZSTD(1)),
    `TraceFlags` UInt8 COMMENT 'W3C trace flags',
    `SeverityText` LowCardinality(String) COMMENT 'Log severity as text' CODEC(ZSTD(1)),
    `SeverityNumber` UInt8 COMMENT 'Log severity as number (1-24)',
    `ServiceName` LowCardinality(String) COMMENT 'Service that emitted the log' CODEC(ZSTD(1)),
    `Body` String COMMENT 'Log message body' CODEC(ZSTD(1)),
    `ResourceSchemaUrl` LowCardinality(String) COMMENT 'Schema URL for the resource' CODEC(ZSTD(1)),
    `ResourceAttributes` Map(LowCardinality(String), String) COMMENT 'Resource attributes as key-value pairs' CODEC(ZSTD(1)),
    `ScopeSchemaUrl` LowCardinality(String) COMMENT 'Schema URL for the instrumentation scope' CODEC(ZSTD(1)),
    `ScopeName` String COMMENT 'Instrumentation scope name' CODEC(ZSTD(1)),
    `ScopeVersion` LowCardinality(String) COMMENT 'Instrumentation scope version' CODEC(ZSTD(1)),
    `ScopeAttributes` Map(LowCardinality(String), String) COMMENT 'Instrumentation scope attributes' CODEC(ZSTD(1)),
    `LogAttributes` Map(LowCardinality(String), String) COMMENT 'Log record attributes' CODEC(ZSTD(1)),
    `EventName` String COMMENT 'Event name for log records representing events' CODEC(ZSTD(1)),
    `__patrolli_materialized.node.id` LowCardinality(String) MATERIALIZED ScopeAttributes['patrolli.node.id'] CODEC(ZSTD(1)),
    `__patrolli_materialized.listener.id` LowCardinality(String) MATERIALIZED ScopeAttributes['patrolli.listener.id'] CODEC(ZSTD(1)),
    `__patrolli_materialized.listener.name` LowCardinality(String) MATERIALIZED ScopeAttributes['patrolli.listener.name'] CODEC(ZSTD(1)),
    `__patrolli_materialized.item.id` LowCardinality(String) MATERIALIZED ResourceAttributes['patrolli.item.id'] CODEC(ZSTD(1)),
    `__patrolli_materialized.item.name` LowCardinality(String) MATERIALIZED ResourceAttributes['patrolli.item.name'] CODEC(ZSTD(1)),
    `__patrolli_materialized.host.id` LowCardinality(String) MATERIALIZED ResourceAttributes['host.id'] CODEC(ZSTD(1)),
    `__patrolli_materialized.host.name` LowCardinality(String) MATERIALIZED ResourceAttributes['host.name'] CODEC(ZSTD(1)),
    `__patrolli_materialized.source.address` LowCardinality(String) MATERIALIZED LogAttributes['source.address'] CODEC(ZSTD(1)),
{indexes}
) ENGINE = {engine}
PARTITION BY toDate(Timestamp)
ORDER BY (toStartOfFiveMinutes(Timestamp), ServiceName, Timestamp)
{ttl}
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1"#
        )
    }

    fn traces_table(&self) -> String {
        let name = self.qualified(&self.tables.traces);
        let cluster = &self.cluster;
        let engine = &self.engine;
        let ttl = ttl_expr(self.ttl, "toDateTime(Timestamp)");
        format!(
            r#"CREATE TABLE IF NOT EXISTS {name} {cluster} (
    Timestamp DateTime64(9) CODEC(Delta, ZSTD(1)),
    TraceId String CODEC(ZSTD(1)),
    SpanId String CODEC(ZSTD(1)),
    ParentSpanId String CODEC(ZSTD(1)),
    TraceState String CODEC(ZSTD(1)),
    SpanName LowCardinality(String) CODEC(ZSTD(1)),
    SpanKind LowCardinality(String) CODEC(ZSTD(1)),
    ServiceName LowCardinality(String) CODEC(ZSTD(1)),
    ResourceAttributes Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    ScopeName String CODEC(ZSTD(1)),
    ScopeVersion String CODEC(ZSTD(1)),
    SpanAttributes Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    Duration UInt64 CODEC(ZSTD(1)),
    StatusCode LowCardinality(String) CODEC(ZSTD(1)),
    StatusMessage String CODEC(ZSTD(1)),
    Events Nested (
        Timestamp DateTime64(9),
        Name LowCardinality(String),
        Attributes Map(LowCardinality(String), String)
    ) CODEC(ZSTD(1)),
    Links Nested (
        TraceId String,
        SpanId String,
        TraceState String,
        Attributes Map(LowCardinality(String), String)
    ) CODEC(ZSTD(1)),
    INDEX idx_trace_id TraceId TYPE bloom_filter(0.001) GRANULARITY 1,
    INDEX idx_res_attr_key mapKeys(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_res_attr_value mapValues(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_span_attr_key mapKeys(SpanAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_span_attr_value mapValues(SpanAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_duration Duration TYPE minmax GRANULARITY 1
) ENGINE = {engine}
PARTITION BY toDate(Timestamp)
ORDER BY (ServiceName, SpanName, toDateTime(Timestamp))
{ttl}
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1"#
        )
    }

    /// Lookup table that answers "which time range does this trace id span", so a
    /// trace-by-id query can prune partitions instead of scanning the traces table.
    fn traces_id_ts_table(&self) -> String {
        let name = self.qualified(&format!("{}_trace_id_ts", self.tables.traces));
        let cluster = &self.cluster;
        let engine = &self.engine;
        let ttl = ttl_expr(self.ttl, "toDateTime(Start)");
        format!(
            r#"CREATE TABLE IF NOT EXISTS {name} {cluster} (
    TraceId String CODEC(ZSTD(1)),
    Start DateTime CODEC(Delta, ZSTD(1)),
    End DateTime CODEC(Delta, ZSTD(1)),
    INDEX idx_trace_id TraceId TYPE bloom_filter(0.01) GRANULARITY 1
) ENGINE = {engine}
PARTITION BY toDate(Start)
ORDER BY (TraceId, Start)
{ttl}
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1"#
        )
    }

    fn traces_id_ts_mv(&self) -> String {
        let name = self.qualified(&format!("{}_trace_id_ts_mv", self.tables.traces));
        let target = self.qualified(&format!("{}_trace_id_ts", self.tables.traces));
        let source = self.qualified(&self.tables.traces);
        let cluster = &self.cluster;
        format!(
            r#"CREATE MATERIALIZED VIEW IF NOT EXISTS {name} {cluster}
TO {target}
AS SELECT
    TraceId,
    min(Timestamp) as Start,
    max(Timestamp) as End
FROM {source}
WHERE TraceId != ''
GROUP BY TraceId"#
        )
    }

    fn metrics_gauge_table(&self) -> String {
        self.metrics_table(&self.tables.metrics_gauge, GAUGE_COLUMNS)
    }

    fn metrics_sum_table(&self) -> String {
        self.metrics_table(&self.tables.metrics_sum, SUM_COLUMNS)
    }

    fn metrics_summary_table(&self) -> String {
        self.metrics_table(&self.tables.metrics_summary, SUMMARY_COLUMNS)
    }

    fn metrics_histogram_table(&self) -> String {
        self.metrics_table(&self.tables.metrics_histogram, HISTOGRAM_COLUMNS)
    }

    fn metrics_exp_histogram_table(&self) -> String {
        self.metrics_table(&self.tables.metrics_exp_histogram, EXP_HISTOGRAM_COLUMNS)
    }

    /// All five metric tables share an identical envelope — resource/scope/metric
    /// identity up front, the same skip indexes and sort key at the back — and differ only
    /// in the datapoint columns wedged between them.
    fn metrics_table(&self, table: &str, columns: &str) -> String {
        let name = self.qualified(table);
        let cluster = &self.cluster;
        let engine = &self.engine;
        let ttl = ttl_expr(self.ttl, "toDateTime(TimeUnix)");
        format!(
            r#"CREATE TABLE IF NOT EXISTS {name} {cluster} (
{METRICS_IDENTITY_COLUMNS}
{columns}
{METRICS_INDEXES}
) ENGINE = {engine}
{ttl}
PARTITION BY toDate(TimeUnix)
ORDER BY (ServiceName, MetricName, toStartOfHour(TimeUnix), cityHash64(Attributes), TimeUnix)
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1"#
        )
    }
}

/// `ENGINE = <name>(<params>)`. Params are joined as written; an empty list still yields
/// the trailing `()`, which is what the exporter emits for the default `MergeTree`.
fn engine_string(engine: &ClickhouseTableEngine) -> String {
    format!("{}({})", engine.name, engine.params.join(", "))
}

fn cluster_string(cluster_name: Option<&str>) -> String {
    match cluster_name {
        Some(name) => format!("ON CLUSTER {}", quote_ident(name)),
        None => String::new(),
    }
}

/// `TTL <time_field> + toIntervalX(n)`, picking the coarsest unit that divides `ttl`
/// exactly. Empty when retention is disabled.
fn ttl_expr(ttl: Duration, time_field: &str) -> String {
    const DAY: u64 = 24 * 60 * 60;
    const HOUR: u64 = 60 * 60;
    const MINUTE: u64 = 60;

    let secs = ttl.as_secs();
    if secs == 0 {
        return String::new();
    }
    if secs.is_multiple_of(DAY) {
        format!("TTL {time_field} + toIntervalDay({})", secs / DAY)
    } else if secs.is_multiple_of(HOUR) {
        format!("TTL {time_field} + toIntervalHour({})", secs / HOUR)
    } else if secs.is_multiple_of(MINUTE) {
        format!("TTL {time_field} + toIntervalMinute({})", secs / MINUTE)
    } else {
        format!("TTL {time_field} + toIntervalSecond({secs})")
    }
}

const LOGS_TEXT_INDEXES: &str = r#"    INDEX idx_trace_id TraceId TYPE text(tokenizer = 'array'),
    INDEX idx_res_attr_key mapKeys(ResourceAttributes) TYPE text(tokenizer = 'array'),
    INDEX idx_res_attr_value mapValues(ResourceAttributes) TYPE text(tokenizer = 'array'),
    INDEX idx_scope_attr_key mapKeys(ScopeAttributes) TYPE text(tokenizer = 'array'),
    INDEX idx_scope_attr_value mapValues(ScopeAttributes) TYPE text(tokenizer = 'array'),
    INDEX idx_log_attr_key mapKeys(LogAttributes) TYPE text(tokenizer = 'array'),
    INDEX idx_log_attr_value mapValues(LogAttributes) TYPE text(tokenizer = 'array'),
    INDEX idx_lower_body lower(Body) TYPE text(tokenizer = 'splitByNonAlpha')"#;

const LOGS_BLOOM_INDEXES: &str = r#"    INDEX idx_trace_id TraceId TYPE bloom_filter(0.001) GRANULARITY 1,
    INDEX idx_res_attr_key mapKeys(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_res_attr_value mapValues(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_scope_attr_key mapKeys(ScopeAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_scope_attr_value mapValues(ScopeAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_log_attr_key mapKeys(LogAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_log_attr_value mapValues(LogAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_lower_body lower(Body) TYPE tokenbf_v1(32768, 3, 0) GRANULARITY 8"#;

const METRICS_IDENTITY_COLUMNS: &str = r#"    ResourceAttributes Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    ResourceSchemaUrl String CODEC(ZSTD(1)),
    ScopeName String CODEC(ZSTD(1)),
    ScopeVersion String CODEC(ZSTD(1)),
    ScopeAttributes Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    ScopeDroppedAttrCount UInt32 CODEC(ZSTD(1)),
    ScopeSchemaUrl String CODEC(ZSTD(1)),
    ServiceName LowCardinality(String) CODEC(ZSTD(1)),
    MetricName LowCardinality(String) CODEC(ZSTD(1)),
    MetricDescription String CODEC(ZSTD(1)),
    MetricUnit String CODEC(ZSTD(1)),
    Attributes Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    StartTimeUnix DateTime CODEC(Delta, ZSTD(1)),
    TimeUnix DateTime CODEC(Delta, ZSTD(1)),"#;

const METRICS_INDEXES: &str = r#"    INDEX idx_res_attr_key mapKeys(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_res_attr_value mapValues(ResourceAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_scope_attr_key mapKeys(ScopeAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_scope_attr_value mapValues(ScopeAttributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_attr_key mapKeys(Attributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_attr_value mapValues(Attributes) TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_time_minmax TimeUnix TYPE minmax GRANULARITY 1"#;

// The per-datapoint-type column blocks, each spliced between METRICS_IDENTITY_COLUMNS and
// METRICS_INDEXES. Column order matters — the exporter's INSERTs are positional — so these
// are kept verbatim (Exemplars repeated and all) to stay diffable against upstream's
// `metrics_*_table.sql`.

const GAUGE_COLUMNS: &str = r#"    Value Float64 CODEC(ZSTD(1)),
    Flags UInt32 CODEC(ZSTD(1)),
    Exemplars Nested (
        FilteredAttributes Map(LowCardinality(String), String),
        TimeUnix DateTime,
        Value Float64,
        SpanId String,
        TraceId String
    ) CODEC(ZSTD(1)),"#;

const SUM_COLUMNS: &str = r#"    Value Float64 CODEC(ZSTD(1)),
    Flags UInt32 CODEC(ZSTD(1)),
    Exemplars Nested (
        FilteredAttributes Map(LowCardinality(String), String),
        TimeUnix DateTime,
        Value Float64,
        SpanId String,
        TraceId String
    ) CODEC(ZSTD(1)),
    AggregationTemporality Int32 CODEC(ZSTD(1)),
    IsMonotonic Boolean CODEC(Delta, ZSTD(1)),"#;

const SUMMARY_COLUMNS: &str = r#"    Count UInt64 CODEC(Delta, ZSTD(1)),
    Sum Float64 CODEC(ZSTD(1)),
    ValueAtQuantiles Nested(
        Quantile Float64,
        Value Float64
    ) CODEC(ZSTD(1)),
    Flags UInt32 CODEC(ZSTD(1)),"#;

const HISTOGRAM_COLUMNS: &str = r#"    Count UInt64 CODEC(Delta, ZSTD(1)),
    Sum Float64 CODEC(ZSTD(1)),
    BucketCounts Array(UInt64) CODEC(ZSTD(1)),
    ExplicitBounds Array(Float64) CODEC(ZSTD(1)),
    Exemplars Nested (
        FilteredAttributes Map(LowCardinality(String), String),
        TimeUnix DateTime,
        Value Float64,
        SpanId String,
        TraceId String
    ) CODEC(ZSTD(1)),
    Flags UInt32 CODEC(ZSTD(1)),
    Min Float64 CODEC(ZSTD(1)),
    Max Float64 CODEC(ZSTD(1)),
    AggregationTemporality Int32 CODEC(ZSTD(1)),"#;

const EXP_HISTOGRAM_COLUMNS: &str = r#"    Count UInt64 CODEC(Delta, ZSTD(1)),
    Sum Float64 CODEC(ZSTD(1)),
    Scale Int32 CODEC(ZSTD(1)),
    ZeroCount UInt64 CODEC(ZSTD(1)),
    PositiveOffset Int32 CODEC(ZSTD(1)),
    PositiveBucketCounts Array(UInt64) CODEC(ZSTD(1)),
    NegativeOffset Int32 CODEC(ZSTD(1)),
    NegativeBucketCounts Array(UInt64) CODEC(ZSTD(1)),
    Exemplars Nested (
        FilteredAttributes Map(LowCardinality(String), String),
        TimeUnix DateTime,
        Value Float64,
        SpanId String,
        TraceId String
    ) CODEC(ZSTD(1)),
    Flags UInt32 CODEC(ZSTD(1)),
    Min Float64 CODEC(ZSTD(1)),
    Max Float64 CODEC(ZSTD(1)),
    AggregationTemporality Int32 CODEC(ZSTD(1)),"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn ddl(create_schema: &ClickhouseCreateSchema, full_text_search: bool) -> Ddl<'_> {
        static TABLES: std::sync::LazyLock<ClickhouseTables> =
            std::sync::LazyLock::new(ClickhouseTables::default);
        Ddl::new("otel", &TABLES, create_schema, full_text_search)
    }

    #[test]
    fn ttl_picks_the_coarsest_exact_unit() {
        let f = "toDateTime(TimeUnix)";
        assert_eq!(ttl_expr(Duration::ZERO, f), "");
        assert_eq!(
            ttl_expr(Duration::from_secs(30 * 24 * 3600), f),
            "TTL toDateTime(TimeUnix) + toIntervalDay(30)"
        );
        assert_eq!(
            ttl_expr(Duration::from_secs(48 * 3600 + 3600), f),
            "TTL toDateTime(TimeUnix) + toIntervalHour(49)"
        );
        assert_eq!(
            ttl_expr(Duration::from_secs(90 * 60), f),
            "TTL toDateTime(TimeUnix) + toIntervalMinute(90)"
        );
        assert_eq!(
            ttl_expr(Duration::from_secs(45), f),
            "TTL toDateTime(TimeUnix) + toIntervalSecond(45)"
        );
    }

    #[test]
    fn engine_and_cluster_strings() {
        assert_eq!(
            engine_string(&ClickhouseTableEngine::default()),
            "MergeTree()"
        );
        assert_eq!(
            engine_string(&ClickhouseTableEngine {
                name: "ReplicatedMergeTree".to_string(),
                params: vec![
                    "'/clickhouse/tables/{shard}/otel'".to_string(),
                    "'{replica}'".to_string()
                ],
            }),
            "ReplicatedMergeTree('/clickhouse/tables/{shard}/otel', '{replica}')"
        );
        assert_eq!(cluster_string(None), "");
        assert_eq!(cluster_string(Some("prod")), "ON CLUSTER `prod`");
        assert_eq!(cluster_string(Some("we`ird")), "ON CLUSTER `we``ird`");
    }

    #[test]
    fn create_database_is_skipped_for_default() {
        let cs = ClickhouseCreateSchema::default();
        assert!(
            Ddl::new(DEFAULT_DATABASE, &ClickhouseTables::default(), &cs, false)
                .create_database()
                .is_none()
        );
        assert_eq!(
            ddl(&cs, false).create_database().unwrap(),
            "CREATE DATABASE IF NOT EXISTS `otel` "
        );
    }

    #[test]
    fn statement_set_matches_the_exporter() {
        let cs = ClickhouseCreateSchema::default();
        let statements = ddl(&cs, false).statements();
        // database + logs + 3 traces + 5 metrics
        assert_eq!(statements.len(), 10);
        for (label, sql) in &statements {
            assert!(
                sql.starts_with("CREATE DATABASE IF NOT EXISTS")
                    || sql.starts_with("CREATE TABLE IF NOT EXISTS")
                    || sql.starts_with("CREATE MATERIALIZED VIEW IF NOT EXISTS"),
                "{label} is not an idempotent CREATE: {sql}"
            );
            // clickhouse-rs treats `?` as a bind placeholder and refuses to send the
            // statement if it is left unbound.
            assert!(!sql.contains('?'), "{label} contains a bind placeholder");
        }
    }

    #[test]
    fn metric_tables_carry_their_datapoint_columns() {
        let cs = ClickhouseCreateSchema::default();
        let d = ddl(&cs, false);

        let gauge = d.metrics_gauge_table();
        assert!(gauge.contains("CREATE TABLE IF NOT EXISTS `otel`.`otel_metrics_gauge` "));
        assert!(gauge.contains("    Value Float64 CODEC(ZSTD(1)),"));
        assert!(!gauge.contains("AggregationTemporality"));

        let sum = d.metrics_sum_table();
        assert!(sum.contains("    IsMonotonic Boolean CODEC(Delta, ZSTD(1)),"));
        assert!(sum.contains("    AggregationTemporality Int32 CODEC(ZSTD(1)),"));

        let summary = d.metrics_summary_table();
        assert!(summary.contains("ValueAtQuantiles Nested("));
        assert!(!summary.contains("Exemplars"));

        let histogram = d.metrics_histogram_table();
        assert!(histogram.contains("    BucketCounts Array(UInt64) CODEC(ZSTD(1)),"));
        assert!(histogram.contains("    ExplicitBounds Array(Float64) CODEC(ZSTD(1)),"));

        let exp = d.metrics_exp_histogram_table();
        assert!(exp.contains("    PositiveBucketCounts Array(UInt64) CODEC(ZSTD(1)),"));
        assert!(exp.contains("    NegativeBucketCounts Array(UInt64) CODEC(ZSTD(1)),"));

        // Shared envelope, every table.
        for sql in [&gauge, &sum, &summary, &histogram, &exp] {
            assert!(sql.contains(METRICS_IDENTITY_COLUMNS));
            assert!(sql.contains(METRICS_INDEXES));
            assert!(sql.contains(
                "ORDER BY (ServiceName, MetricName, toStartOfHour(TimeUnix), cityHash64(Attributes), TimeUnix)"
            ));
        }
    }

    #[test]
    fn logs_indexes_follow_server_support() {
        let cs = ClickhouseCreateSchema::default();
        assert!(
            ddl(&cs, false)
                .logs_table()
                .contains("TYPE bloom_filter(0.001) GRANULARITY 1")
        );
        assert!(
            ddl(&cs, true)
                .logs_table()
                .contains("TYPE text(tokenizer = 'array')")
        );
    }

    #[test]
    fn traces_trio_is_wired_to_the_traces_table_name() {
        let cs = ClickhouseCreateSchema::default();
        let d = ddl(&cs, false);
        assert!(
            d.traces_id_ts_table()
                .contains("CREATE TABLE IF NOT EXISTS `otel`.`otel_traces_trace_id_ts` ")
        );
        let mv = d.traces_id_ts_mv();
        assert!(mv.contains("`otel`.`otel_traces_trace_id_ts_mv`"));
        assert!(mv.contains("TO `otel`.`otel_traces_trace_id_ts`"));
        assert!(mv.contains("FROM `otel`.`otel_traces`"));
    }

    #[test]
    fn cluster_and_ttl_reach_every_statement() {
        let cs = ClickhouseCreateSchema {
            cluster_name: Some("prod".to_string()),
            table_engine: ClickhouseTableEngine::default(),
            ttl: Duration::from_secs(7 * 24 * 3600),
        };
        for (label, sql) in ddl(&cs, false).statements() {
            assert!(sql.contains("ON CLUSTER `prod`"), "{label} lost ON CLUSTER");
            // Retention is a property of the tables; the materialized view inherits it
            // from the table it writes into, and a database has none.
            if !sql.starts_with("CREATE TABLE") {
                assert!(!sql.contains("TTL "), "{label} should not carry a TTL");
                continue;
            }
            assert!(sql.contains("+ toIntervalDay(7)"), "{label} lost the TTL");
        }
    }
}
