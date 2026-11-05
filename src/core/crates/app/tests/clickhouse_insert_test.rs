// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! End-to-end insert tests for the ClickHouse sink's `Payload` → Arrow conversion.
//!
//! The unit tests in `sink::clickhouse::record_batch` prove the batches have the right
//! *shape*; only a live server can prove the Arrow → ClickHouse *type* mapping. Two parts of
//! it are not covered by the upstream otel-arrow exporter's own end-to-end tests, so they are
//! unverified until this runs:
//!
//! - **`DateTime`** ← Arrow `Timestamp(Second)`. Every metric timestamp column is second
//!   precision; the upstream reference has no such column, so that mapping is ours.
//! - **Flattened `Nested(...)`** ← `List<...>` columns named `Parent.Member`. If ClickHouse's
//!   default `flatten_nested = 1` did not expand `Nested` that way, the insert would fail
//!   with `NO_SUCH_COLUMN_IN_TABLE`.
//!
//! A successful insert is itself the strongest assertion available: `clickhouse-ext-arrow`
//! names every column explicitly in its `INSERT`, and the server rejects the request if any
//! name is not a column of the target table. So these fail loudly on any drift between
//! `record_batch::schema` and `ddl`.
//!
//! Like the other tests here, these need a running Docker daemon (image:
//! `clickhouse/clickhouse-server`).

mod setup;

use clickhouse::Client;
use clickhouse_ext_arrow::ArrowClientExt;
use patrolli::config::sink::{ClickhouseCreateSchema, ClickhouseTables};
use patrolli::payload::Payload;
use patrolli::sink::clickhouse::ddl::{DEFAULT_DATABASE, Ddl};
use patrolli::sink::clickhouse::record_batch::{ChTable, to_record_batches};

/// Start a server and create the full exporter schema on it.
async fn setup_schema() -> (
    testcontainers::ContainerAsync<testcontainers::GenericImage>,
    Client,
    ClickhouseTables,
) {
    // `false` = bloom-filter indexes rather than `TYPE text(...)`, so the DDL stays valid on
    // servers older than 26.2. The text-index branch is covered separately by
    // `full_text_search_ddl_is_accepted`.
    setup_schema_with(false).await
}

async fn setup_schema_with(
    full_text_search: bool,
) -> (
    testcontainers::ContainerAsync<testcontainers::GenericImage>,
    Client,
    ClickhouseTables,
) {
    let (container, client) = setup::setup_clickhouse().await;
    let tables = ClickhouseTables::default();
    let create_schema = ClickhouseCreateSchema::default();
    for (label, sql) in
        Ddl::new(DEFAULT_DATABASE, &tables, &create_schema, full_text_search).statements()
    {
        client
            .query(&sql)
            .execute()
            .await
            .unwrap_or_else(|e| panic!("{label} failed: {e}\n{sql}"));
    }
    (container, client, tables)
}

/// Convert a payload and insert every batch it produces.
async fn insert(client: &Client, tables: &ClickhouseTables, payload: &mut Payload) {
    for (table, batch) in to_record_batches(payload).expect("conversion failed") {
        let name = table.name(tables);
        let mut insert = client
            .insert_arrow(name)
            .unwrap_or_else(|e| panic!("insert_arrow({name}): {e}"));
        insert
            .write(&batch)
            .await
            .unwrap_or_else(|e| panic!("write to {name}: {e}"));
        // `end()` is where the server reports an unknown column name.
        insert
            .end()
            .await
            .unwrap_or_else(|e| panic!("end insert into {name}: {e}"));
    }
}

async fn scalar<T>(client: &Client, sql: &str) -> T
where
    T: clickhouse::RowOwned + clickhouse::RowRead,
{
    client
        .query(sql)
        .fetch_one::<T>()
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

/// The logs table's `TYPE text(...)` skip indexes, which the sink uses on servers at or above
/// `MIN_FULL_TEXT_SEARCH_VERSION`. Nothing else exercises that branch, yet it is the one a
/// modern server actually takes — and a malformed index would fail `create_schema` at
/// startup. Skipped on older servers, where the syntax legitimately does not exist.
#[tokio::test]
async fn full_text_search_ddl_is_accepted() {
    setup::setup_logging();
    let (_container, client) = setup::setup_clickhouse().await;
    let version = client
        .query("SELECT version()")
        .fetch_one::<String>()
        .await
        .expect("version");
    let (major, minor) = {
        let mut parts = version.split('.');
        let major: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        let minor: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        (major, minor)
    };
    if (major, minor) < patrolli::sink::clickhouse::ddl::MIN_FULL_TEXT_SEARCH_VERSION {
        eprintln!("server {version} predates text indexes; skipping");
        return;
    }

    let tables = ClickhouseTables::default();
    let create_schema = ClickhouseCreateSchema::default();
    for (label, sql) in Ddl::new(DEFAULT_DATABASE, &tables, &create_schema, true).statements() {
        client
            .query(&sql)
            .execute()
            .await
            .unwrap_or_else(|e| panic!("{label} failed on {version}: {e}\n{sql}"));
    }

    // And the converter's output still inserts against that variant of the table.
    insert(&client, &tables, &mut setup::otlp::logs_payload()).await;
    let logs = ChTable::Logs.name(&tables);
    assert_eq!(
        scalar::<u64>(&client, &format!("SELECT count() FROM {logs}")).await,
        2
    );
}

#[tokio::test]
async fn logs_insert_round_trip() {
    setup::setup_logging();
    let (_container, client, tables) = setup_schema().await;
    insert(&client, &tables, &mut setup::otlp::logs_payload()).await;

    let logs = ChTable::Logs.name(&tables);
    assert_eq!(
        scalar::<u64>(&client, &format!("SELECT count() FROM {logs}")).await,
        2
    );
    assert_eq!(
        scalar::<String>(
            &client,
            &format!("SELECT Body FROM {logs} ORDER BY Timestamp LIMIT 1")
        )
        .await,
        "hello"
    );
    assert_eq!(
        scalar::<String>(
            &client,
            &format!(
                "SELECT LogAttributes['source.address'] FROM {logs} ORDER BY Timestamp LIMIT 1"
            )
        )
        .await,
        "10.0.0.1"
    );
    assert_eq!(
        scalar::<String>(&client, &format!("SELECT ServiceName FROM {logs} LIMIT 1")).await,
        "patrolli"
    );
    assert_eq!(
        scalar::<String>(&client, &format!("SELECT ScopeName FROM {logs} LIMIT 1")).await,
        "patrolli.listener"
    );
    // DateTime64(9) keeps nanosecond precision.
    assert_eq!(
        scalar::<i64>(
            &client,
            &format!(
                "SELECT toUnixTimestamp64Nano(Timestamp) FROM {logs} ORDER BY Timestamp LIMIT 1"
            )
        )
        .await,
        1_700_000_000_000_000_001
    );
}

/// The eight `__patrolli_materialized.*` columns are `MATERIALIZED`, so the converter must not
/// insert them — and the server must be able to compute them from what it does insert.
#[tokio::test]
async fn logs_materialized_columns_are_computed_server_side() {
    setup::setup_logging();
    let (_container, client, tables) = setup_schema().await;
    insert(&client, &tables, &mut setup::otlp::logs_payload()).await;

    let logs = ChTable::Logs.name(&tables);
    assert_eq!(
        scalar::<String>(
            &client,
            &format!("SELECT `__patrolli_materialized.host.name` FROM {logs} LIMIT 1")
        )
        .await,
        "node-a"
    );
    assert_eq!(
        scalar::<String>(
            &client,
            &format!("SELECT `__patrolli_materialized.node.id` FROM {logs} LIMIT 1")
        )
        .await,
        "n1"
    );
    assert_eq!(
        scalar::<String>(
            &client,
            &format!(
                "SELECT `__patrolli_materialized.source.address` FROM {logs} ORDER BY Timestamp LIMIT 1"
            )
        )
        .await,
        "10.0.0.1"
    );
}

#[tokio::test]
async fn traces_insert_with_flattened_nested_columns() {
    setup::setup_logging();
    let (_container, client, tables) = setup_schema().await;
    insert(&client, &tables, &mut setup::otlp::traces_payload()).await;

    let traces = ChTable::Traces.name(&tables);
    assert_eq!(
        scalar::<u64>(&client, &format!("SELECT count() FROM {traces}")).await,
        2
    );
    // The Go exporter's spellings, which the Grafana dashboards filter on.
    assert_eq!(
        scalar::<String>(
            &client,
            &format!("SELECT SpanKind FROM {traces} WHERE SpanName = 'GET /x'")
        )
        .await,
        "Server"
    );
    assert_eq!(
        scalar::<String>(
            &client,
            &format!("SELECT StatusCode FROM {traces} WHERE SpanName = 'GET /x'")
        )
        .await,
        "Error"
    );
    assert_eq!(
        scalar::<u64>(
            &client,
            &format!("SELECT Duration FROM {traces} WHERE SpanName = 'GET /x'")
        )
        .await,
        500
    );

    // Events / Links Nested(...): readable as arrays means the flattened `Parent.Member`
    // naming matched what `flatten_nested = 1` created.
    assert_eq!(
        scalar::<u64>(
            &client,
            &format!("SELECT length(Events.Name) FROM {traces} WHERE SpanName = 'GET /x'")
        )
        .await,
        1
    );
    assert_eq!(
        scalar::<String>(
            &client,
            &format!("SELECT Events.Name[1] FROM {traces} WHERE SpanName = 'GET /x'")
        )
        .await,
        "exception"
    );
    assert_eq!(
        scalar::<String>(
            &client,
            &format!(
                "SELECT Events.Attributes[1]['exception.type'] FROM {traces} WHERE SpanName = 'GET /x'"
            )
        )
        .await,
        "IOError"
    );
    assert_eq!(
        scalar::<String>(
            &client,
            &format!("SELECT Links.TraceState[1] FROM {traces} WHERE SpanName = 'GET /x'")
        )
        .await,
        "c=d"
    );
    // The span with no children stored empty arrays, not nulls.
    assert_eq!(
        scalar::<u64>(
            &client,
            &format!("SELECT length(Events.Name) FROM {traces} WHERE SpanName = 'internal'")
        )
        .await,
        0
    );
}

#[tokio::test]
async fn metrics_insert_into_all_five_tables() {
    setup::setup_logging();
    let (_container, client, tables) = setup_schema().await;
    insert(&client, &tables, &mut setup::otlp::metrics_payload()).await;

    for (table, want) in [
        (ChTable::MetricsGauge, 1u64),
        (ChTable::MetricsSum, 2),
        (ChTable::MetricsSummary, 1),
        (ChTable::MetricsHistogram, 1),
        (ChTable::MetricsExpHistogram, 1),
    ] {
        let name = table.name(&tables);
        assert_eq!(
            scalar::<u64>(&client, &format!("SELECT count() FROM {name}")).await,
            want,
            "{name} row count"
        );
    }

    let gauge = ChTable::MetricsGauge.name(&tables);
    // The `DateTime` mapping: Arrow Timestamp(Second) must land as the right instant.
    assert_eq!(
        scalar::<u32>(
            &client,
            &format!("SELECT toUnixTimestamp(TimeUnix) FROM {gauge}")
        )
        .await,
        1_700_000_060
    );
    assert_eq!(
        scalar::<u32>(
            &client,
            &format!("SELECT toUnixTimestamp(StartTimeUnix) FROM {gauge}")
        )
        .await,
        1_700_000_000
    );
    assert_eq!(
        scalar::<f64>(&client, &format!("SELECT Value FROM {gauge}")).await,
        0.75
    );
    // Exemplars Nested(...), flattened.
    assert_eq!(
        scalar::<f64>(&client, &format!("SELECT Exemplars.Value[1] FROM {gauge}")).await,
        0.75
    );
    assert_eq!(
        scalar::<String>(
            &client,
            &format!("SELECT Exemplars.FilteredAttributes[1]['ex.key'] FROM {gauge}")
        )
        .await,
        "ex.val"
    );
    assert_eq!(
        scalar::<String>(&client, &format!("SELECT MetricUnit FROM {gauge}")).await,
        "1"
    );
}

/// Gauge and sum come out of one `NumberDataPoints` batch, split only by the parent metric's
/// type — and the metric-level columns have to reach every data point.
#[tokio::test]
async fn metrics_gauge_sum_split_and_broadcast() {
    setup::setup_logging();
    let (_container, client, tables) = setup_schema().await;
    insert(&client, &tables, &mut setup::otlp::metrics_payload()).await;

    let gauge = ChTable::MetricsGauge.name(&tables);
    let sum = ChTable::MetricsSum.name(&tables);

    assert_eq!(
        scalar::<String>(&client, &format!("SELECT MetricName FROM {gauge}")).await,
        "cpu.util"
    );
    assert_eq!(
        scalar::<u64>(
            &client,
            &format!("SELECT count() FROM {sum} WHERE MetricName = 'net.bytes' AND IsMonotonic")
        )
        .await,
        2
    );
    // Cumulative = 2, broadcast from the metric row onto both data points.
    assert_eq!(
        scalar::<u64>(
            &client,
            &format!("SELECT count() FROM {sum} WHERE AggregationTemporality = 2")
        )
        .await,
        2
    );
    // Int-valued data points land in the Float64 column.
    assert_eq!(
        scalar::<f64>(
            &client,
            &format!("SELECT Value FROM {sum} WHERE Attributes['iface'] = 'eth0'")
        )
        .await,
        4096.0
    );
    assert_eq!(
        scalar::<f64>(
            &client,
            &format!("SELECT Value FROM {sum} WHERE Attributes['iface'] = 'eth1'")
        )
        .await,
        8192.0
    );
    // Resource and scope attributes reach every data point too.
    assert_eq!(
        scalar::<u64>(
            &client,
            &format!("SELECT count() FROM {sum} WHERE ScopeAttributes['patrolli.node.id'] = 'n1'")
        )
        .await,
        2
    );
}

#[tokio::test]
async fn metrics_array_columns_round_trip() {
    setup::setup_logging();
    let (_container, client, tables) = setup_schema().await;
    insert(&client, &tables, &mut setup::otlp::metrics_payload()).await;

    let histogram = ChTable::MetricsHistogram.name(&tables);
    assert_eq!(
        scalar::<u64>(&client, &format!("SELECT BucketCounts[2] FROM {histogram}")).await,
        2
    );
    assert_eq!(
        scalar::<f64>(
            &client,
            &format!("SELECT ExplicitBounds[1] FROM {histogram}")
        )
        .await,
        10.0
    );
    assert_eq!(
        scalar::<f64>(&client, &format!("SELECT Min FROM {histogram}")).await,
        5.0
    );

    let exp = ChTable::MetricsExpHistogram.name(&tables);
    assert_eq!(
        scalar::<i32>(&client, &format!("SELECT NegativeOffset FROM {exp}")).await,
        -1
    );
    assert_eq!(
        scalar::<i32>(&client, &format!("SELECT PositiveOffset FROM {exp}")).await,
        3
    );
    assert_eq!(
        scalar::<u64>(
            &client,
            &format!("SELECT PositiveBucketCounts[1] FROM {exp}")
        )
        .await,
        2
    );
    assert_eq!(
        scalar::<u64>(&client, &format!("SELECT ZeroCount FROM {exp}")).await,
        1
    );

    // ValueAtQuantiles Nested(...), flattened into two parallel arrays.
    let summary = ChTable::MetricsSummary.name(&tables);
    assert_eq!(
        scalar::<f64>(
            &client,
            &format!("SELECT ValueAtQuantiles.Quantile[2] FROM {summary}")
        )
        .await,
        0.99
    );
    assert_eq!(
        scalar::<f64>(
            &client,
            &format!("SELECT ValueAtQuantiles.Value[2] FROM {summary}")
        )
        .await,
        190.0
    );
    assert_eq!(
        scalar::<u64>(&client, &format!("SELECT Count FROM {summary}")).await,
        4
    );
}

// ---------------------------------------------------------------------------------------------
// Sink-level tests
//
// Everything above drives `to_record_batches` + `insert_arrow` directly, which proves the
// conversion and the column contract but never executes the sink. These drive `ClickhouseSink`
// itself, so they cover the parts only `process_records` does: schema creation through the sink,
// endpoint selection, fanning one flush window out into one `INSERT` per table, the item
// accounting, and the flush-on-shutdown that keeps a stopping worker from dropping its batch.
// ---------------------------------------------------------------------------------------------

/// Build a sink whose batch window nothing can close on its own — `batch_size` is the `u16::MAX`
/// ceiling and `batch_timeout` an hour — so the only thing that can flush it is the shutdown
/// path. `extra` is appended under `config:`, already indented two spaces.
fn sink_config(endpoint: &str, extra: &str) -> patrolli::config::sink::Sink {
    let yaml = format!(
        r#"
id: 00000000-0000-0000-0000-0000000000ff
name: clickhouse-e2e
worker_count: 1
batch_size: 65535
batch_timeout: 3600000
type: clickhouse
config:
  endpoints:
    - {endpoint}
  database: default
  username: {user}
  password: {password}
{extra}
"#,
        user = setup::CLICKHOUSE_USER,
        password = setup::CLICKHOUSE_PASSWORD,
    );
    let mut config: patrolli::config::sink::Sink =
        serde_yaml_ng::from_str(&yaml).unwrap_or_else(|e| panic!("sink config yaml: {e}\n{yaml}"));
    config.validate().expect("sink config validate");
    config
}

/// Feed `payloads` through a real sink and return its internal metrics.
///
/// The sender is dropped before `run`, so the sink drains the channel, sees the disconnect, and
/// exits the loop with a full batch still buffered — which is exactly the shutdown path under
/// test. No sleeps, no cancellation races.
async fn run_sink(
    config: patrolli::config::sink::Sink,
    payloads: Vec<Payload>,
) -> patrolli::common::internal_metrics::Sink {
    use patrolli::sink::Sink as _;

    let (tx, rx) = flume::bounded(payloads.len().max(1));
    for pl in payloads {
        tx.send_async(pl).await.expect("send payload");
    }
    let rx = patrolli::chan::Consumer::from_flume(&tx, rx);
    drop(tx);

    let sink = patrolli::sink::clickhouse::ClickhouseSink::new(config, rx)
        .await
        .expect("build sink");
    sink.run(tokio_util::sync::CancellationToken::new()).await;
    sink.metrics()
}

/// One flush window carrying all three signals fans out into every destination table at once —
/// the concurrent-insert path — and the item accounting adds up against what the server actually
/// stored.
///
/// `create_schema: {}` also makes the sink run its own DDL here, including the server-version
/// probe that picks between text and bloom-filter skip indexes, and the ping that decides whether
/// the endpoint is usable at all.
#[tokio::test]
async fn sink_inserts_every_signal_and_accounts_for_every_item() {
    setup::setup_logging();
    let (container, client) = setup::setup_clickhouse().await;
    let endpoint = setup::clickhouse_url(&container).await;
    let tables = ClickhouseTables::default();

    let metrics = run_sink(
        sink_config(&endpoint, "  create_schema: {}"),
        vec![
            setup::otlp::logs_payload(),
            setup::otlp::traces_payload(),
            setup::otlp::metrics_payload(),
        ],
    )
    .await;

    // Rows really landed, in every table the payloads map to.
    let mut stored = 0u64;
    for table in ChTable::ALL {
        let name = table.name(&tables);
        let n = scalar::<u64>(&client, &format!("SELECT count() FROM {name}")).await;
        assert!(n > 0, "{name} got no rows");
        stored += n;
    }

    // The counter an operator reads must equal what the server holds. This is the assertion that
    // would catch counting batches, or payloads, instead of rows.
    assert_eq!(
        metrics.total_records_sent, stored,
        "total_records_sent should equal the rows in ClickHouse"
    );
    assert_eq!(
        metrics.total_records_errored, 0,
        "nothing should have failed"
    );

    // Every ingested item ends up in exactly one bucket.
    assert_eq!(
        metrics.total_records,
        metrics.total_records_sent + metrics.total_records_errored + metrics.total_records_filtered,
        "total_records must equal sent + errored + filtered"
    );

    // Conversion and network time are attributed separately.
    assert!(
        !metrics.engine_time_spent.is_zero(),
        "conversion should be charged to engine_time_spent"
    );
    assert!(
        !metrics.send_wait_time.is_zero(),
        "the insert should be charged to send_wait_time"
    );
}

/// One unusable table must not take the others down with it.
///
/// The logs table is pointed at a name that was never created, so its `INSERT` fails server-side
/// while traces succeed in the same flush. That is the case the per-table error accounting and the
/// "only transport faults trip the breaker" rule exist for: a missing table fails identically on
/// every endpoint, so treating it as ill health would walk them all into backoff.
#[tokio::test]
async fn sink_survives_a_rejected_table_without_losing_the_others() {
    setup::setup_logging();
    let (container, client, tables) = setup_schema().await;
    let endpoint = setup::clickhouse_url(&container).await;

    // No `create_schema`, so the renamed logs table stays absent.
    let config = sink_config(&endpoint, "  tables:\n    logs: otel_logs_absent");
    let metrics = run_sink(
        config,
        vec![setup::otlp::logs_payload(), setup::otlp::traces_payload()],
    )
    .await;

    let traces = ChTable::Traces.name(&tables);
    assert!(
        scalar::<u64>(&client, &format!("SELECT count() FROM {traces}")).await > 0,
        "traces should still insert when the logs table is unusable"
    );
    assert!(
        metrics.total_records_errored > 0,
        "the rejected rows should be counted as errored"
    );
    assert!(
        metrics.total_records_sent > 0,
        "the accepted rows should still be counted as sent"
    );
    assert_eq!(
        metrics.total_records,
        metrics.total_records_sent + metrics.total_records_errored + metrics.total_records_filtered,
        "total_records must equal sent + errored + filtered even when a table fails"
    );
}
