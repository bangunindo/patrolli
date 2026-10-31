// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

// Every test binary compiles this module but uses only part of it.
#![allow(dead_code)]

pub mod api;
pub mod kafka;
pub mod otlp;
pub mod patrolli;
mod upstream;

use ::patrolli::{common, common::mib, config, db};
use anyhow::Context;
use deadpool_postgres::{Config as PgConfig, Pool, PoolConfig, Runtime, Transaction};
use std::sync::{Arc, Once};
use std::time::Duration;
use testcontainers::core::WaitFor;
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};
use tokio_postgres::NoTls;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use uuid::Uuid;

struct LocalTimer;

impl FormatTime for LocalTimer {
    fn format_time(&self, w: &mut Writer) -> std::fmt::Result {
        let now = common::now();
        write!(w, "{}", now.to_rfc3339())
    }
}

/// Install the test binary's subscriber, once.
///
/// Events go through libtest's output capture, so a passing test prints nothing
/// and a failing one shows its own logs (`--nocapture` shows them all). Capture
/// follows threads spawned from the test, runtime workers included. `RUST_LOG`
/// sets the filter in [`Targets`] syntax, e.g. `RUST_LOG=info,patrolli=trace`;
/// the default is `info`.
pub fn setup_logging() {
    static LOGGING: Once = Once::new();
    LOGGING.call_once(|| {
        let targets = match std::env::var("RUST_LOG") {
            Ok(filter) if !filter.is_empty() => filter
                .parse::<Targets>()
                .unwrap_or_else(|e| panic!("RUST_LOG={filter:?} is not a target filter: {e}")),
            _ => Targets::default().with_default(config::global::Level::Info),
        };
        let layer = tracing_subscriber::fmt::layer()
            .with_test_writer()
            .with_timer(LocalTimer)
            .with_filter(targets);
        tracing_subscriber::registry().with(layer).init();
    });
}

/// Loads only the MIBs vendored in `tests/mibs` (IF-MIB and what it imports;
/// mib-rs embeds the SMI base), never the system ones, so a runner without
/// `snmp-mibs-downloader` resolves the same names as a workstation with it.
pub async fn setup_mib() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/mibs");
    _ = mib::init(&[dir], true).await;
}

pub const SECRET_KEYS: &str = "test:cGF0cm9sbGktdGVzdC1zZWNyZXQta2V5LTMyYnl0ZXM=";
pub const SECRET_ACTIVE_KEY: &str = "test";

pub fn secret_keys() -> Arc<config::sensitive::SecretKeys> {
    Arc::new(
        config::sensitive::SecretKeys::parse(Some(SECRET_KEYS), Some(SECRET_ACTIVE_KEY))
            .expect("invalid test secret keys"),
    )
}

pub const CLUSTER_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(2);
pub const CLUSTER_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);

/// How long [`eventually`] keeps retrying. Generous on purpose: a check returns
/// as soon as it holds, so only a failing test waits this long, and a loaded CI
/// runner gets the slack.
pub const EVENTUALLY: Duration = Duration::from_secs(20);

/// Run `check` every 100ms until it returns `Ok`, and hand back its value.
/// Panics with the last error once [`EVENTUALLY`] has passed, so the message
/// says what was still wrong.
///
/// This replaces sleeping for a guessed duration before asserting. A "this never
/// happens" check has no condition to wait for: wait for whatever would have
/// set it off instead, then assert.
pub async fn eventually<T>(mut check: impl AsyncFnMut() -> anyhow::Result<T>) -> T {
    let deadline = tokio::time::Instant::now() + EVENTUALLY;
    loop {
        match check().await {
            Ok(value) => return value,
            Err(e) if tokio::time::Instant::now() >= deadline => {
                panic!("still failing after {EVENTUALLY:?}: {e:#}")
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

/// Credentials the ClickHouse fixture is started with. Public because a test that drives the
/// sink itself has to put them in a `config::sink::Clickhouse`, not just in a bare client.
pub const CLICKHOUSE_USER: &str = "patrolli";
pub const CLICKHOUSE_PASSWORD: &str = "password";

/// The HTTP-interface URL of a running ClickHouse fixture, for `Clickhouse::endpoints`.
pub async fn clickhouse_url(container: &testcontainers::ContainerAsync<GenericImage>) -> String {
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(8123).await.unwrap();
    format!("http://{host}:{port}")
}

/// Start a ClickHouse server and return a client bound to the `default` database.
///
/// `default` on purpose: `Ddl` skips `CREATE DATABASE` for it, and `insert_arrow` emits an
/// unqualified `INSERT INTO <table>`, so the client's database has to be the one the tables
/// were created in.
///
/// Two things the image makes awkward, both handled here:
///
/// - Credentials are **required**. Without `CLICKHOUSE_USER`/`CLICKHOUSE_PASSWORD` the
///   entrypoint disables network access for `default`, and every query fails with
///   `REQUIRED_PASSWORD`. `DEFAULT_ACCESS_MANAGEMENT` lets that user run DDL.
/// - There is no usable log line to wait on: "Ready for connections" goes to the server's
///   trace log file, not stdout or stderr, and `WaitFor::Http` needs a testcontainers feature
///   this crate does not enable. So readiness is polled with the same `SELECT 1` the sink's
///   own health check uses.
pub async fn setup_clickhouse() -> (
    testcontainers::ContainerAsync<GenericImage>,
    clickhouse::Client,
) {
    const READY_TIMEOUT: Duration = Duration::from_secs(60);
    // `PATROLLI_TEST_UPSTREAM=latest` runs against `latest` instead, see `upstream_tag`
    const PINNED: &str = "26.7.3.19";

    let container = GenericImage::new(
        "clickhouse/clickhouse-server",
        upstream::upstream_tag(PINNED),
    )
    .with_wait_for(WaitFor::Nothing)
    .with_env_var("CLICKHOUSE_USER", CLICKHOUSE_USER)
    .with_env_var("CLICKHOUSE_PASSWORD", CLICKHOUSE_PASSWORD)
    .with_env_var("CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT", "1")
    .start()
    .await
    .expect("failed to start clickhouse container");

    // The default client compression is LZ4, which (like the sink's) needs the server's
    // response codec pinned: it defaults to ZSTD since 26.9.
    let client = clickhouse::Client::default()
        .with_setting("network_compression_method", "lz4")
        .with_url(clickhouse_url(&container).await)
        .with_user(CLICKHOUSE_USER)
        .with_password(CLICKHOUSE_PASSWORD)
        .with_database("default");

    let deadline = std::time::Instant::now() + READY_TIMEOUT;
    loop {
        match client.query("SELECT 1").fetch_one::<u8>().await {
            Ok(_) => break,
            Err(e) if std::time::Instant::now() >= deadline => {
                panic!("clickhouse did not become ready within {READY_TIMEOUT:?}: {e}")
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
        }
    }
    (container, client)
}

/// The database behind one [`setup_db`] call. Keep it alive for the whole test:
/// dropping it stops the container, or drops the database it created.
pub struct TestDb {
    /// Where the test database is, for a `patrolli` process to connect to.
    pub url: String,
    /// The Postgres container, when this call started one.
    _container: Option<testcontainers::ContainerAsync<GenericImage>>,
    /// `(server url, database name)` of a database created on the
    /// `PATROLLI_TEST_DATABASE_URL` server, dropped with this guard.
    external: Option<(String, String)>,
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let Some((url, name)) = self.external.take() else {
            return;
        };
        // `drop` cannot await, and the test's runtime may be the current-thread
        // one this runs on, so the DROP gets a thread and a runtime of its own.
        // FORCE ends the sessions the test's pools and nodes still hold.
        let result = std::thread::spawn(move || -> anyhow::Result<()> {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(async {
                    let (client, conn) = tokio_postgres::connect(&url, NoTls).await?;
                    tokio::spawn(conn);
                    client
                        .batch_execute(&format!(r#"DROP DATABASE IF EXISTS "{name}" WITH (FORCE)"#))
                        .await?;
                    Ok(())
                })
        })
        .join();
        if let Ok(Err(e)) = result {
            tracing::warn!(error = %format!("{e:#}"), "failed to drop test database");
        }
    }
}

/// A migrated, empty Patrolli database for one test.
///
/// With `PATROLLI_TEST_DATABASE_URL` set to a server the user can `CREATE DATABASE`
/// on (e.g. `postgres://postgres:postgres@localhost:5432/postgres`), every call
/// creates its own `patrolli_test_<uuid>` database there, so tests stay isolated
/// without a container each. Otherwise it starts a `postgres:18-alpine`
/// container.
pub async fn setup_db() -> (TestDb, Pool) {
    let server_url = std::env::var("PATROLLI_TEST_DATABASE_URL")
        .ok()
        .filter(|url| !url.is_empty());

    let test_db = match server_url {
        Some(url) => {
            let name = format!("patrolli_test_{}", Uuid::now_v7().simple());
            let (client, conn) = tokio_postgres::connect(&url, NoTls)
                .await
                .expect("failed to connect to PATROLLI_TEST_DATABASE_URL");
            tokio::spawn(conn);
            client
                .batch_execute(&format!(r#"CREATE DATABASE "{name}""#))
                .await
                .expect("failed to create the test database");
            TestDb {
                url: with_dbname(&url, &name),
                _container: None,
                external: Some((url, name)),
            }
        }
        None => {
            let container = GenericImage::new("postgres", "18-alpine")
                .with_wait_for(WaitFor::message_on_stderr(
                    "database system is ready to accept connections",
                ))
                .with_env_var("POSTGRES_PASSWORD", "password")
                .with_env_var("POSTGRES_USER", "patrolli")
                .with_env_var("POSTGRES_DB", "patrolli")
                .start()
                .await
                .expect("failed to start postgres container");

            let host = container.get_host().await.unwrap();
            let port = container.get_host_port_ipv4(5432).await.unwrap();
            TestDb {
                url: format!("postgres://patrolli:password@{host}:{port}/patrolli"),
                _container: Some(container),
                external: None,
            }
        }
    };

    let mut cfg = PgConfig::new();
    cfg.url = Some(test_db.url.clone());
    cfg.pool = Some(PoolConfig {
        max_size: 10,
        ..Default::default()
    });
    let pool = cfg
        .create_pool(Some(Runtime::Tokio1), NoTls)
        .expect("failed to create pool");

    db::run_migrations(&pool).await.expect("migrations failed");
    (test_db, pool)
}

/// `url` with its database swapped for `name`, keeping any `?query`. Only the
/// URL form: a key/value connection string has no path to swap.
fn with_dbname(url: &str, name: &str) -> String {
    let rest = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))
        .unwrap_or_else(|| {
            panic!(
                "PATROLLI_TEST_DATABASE_URL={url:?}: expected a postgres:// or postgresql:// url"
            )
        });
    let scheme = &url[..url.len() - rest.len()];
    let (rest, query) = rest
        .split_once('?')
        .map_or((rest, None), |(r, q)| (r, Some(q)));
    let authority = rest.split_once('/').map_or(rest, |(a, _)| a);
    match query {
        Some(query) => format!("{scheme}{authority}/{name}?{query}"),
        None => format!("{scheme}{authority}/{name}"),
    }
}

pub async fn create_host(tx: &Transaction<'_>, name: &str) -> anyhow::Result<Uuid> {
    let host_id = Uuid::now_v7();
    tx.execute(
        "INSERT INTO hosts (host_id, name, adm_status, last_updated) VALUES ($1, $2, 1, now())",
        &[&host_id, &name],
    )
    .await
    .context("failed to insert host")?;
    Ok(host_id)
}

pub async fn create_processor(tx: &Transaction<'_>, name: &str) -> anyhow::Result<Uuid> {
    let processor_id = Uuid::now_v7();
    tx.execute(
        "INSERT INTO processors (processor_id, name, type, adm_status, last_updated, config) \
        VALUES ($1, $2, 'null', 1, now(), 'null'::jsonb)",
        &[&processor_id, &name],
    )
    .await
    .context("failed to insert processor")?;
    Ok(processor_id)
}

/// Insert a lookup table, splitting `source` and `key` into the tag column plus
/// the body blob the way the reader expects to find them.
///
/// `adm_status` is a parameter because "disabled" is a state the config watcher
/// has to honour by *not* loading the table, which is worth testing.
pub async fn create_lookup_table(
    tx: &Transaction<'_>,
    name: &str,
    source: &config::lookup_table::Source,
    key: &config::lookup_table::Key,
    adm_status: i16,
) -> anyhow::Result<Uuid> {
    use config::lookup_table::{KeyKind, SourceKind};

    let lookup_table_id = Uuid::now_v7();
    tx.execute(
        "INSERT INTO lookup_tables \
        (lookup_table_id, name, type, adm_status, last_updated, source, key_type, key) \
        VALUES ($1, $2, $3, $4, now(), $5, $6, $7)",
        &[
            &lookup_table_id,
            &name,
            &SourceKind::from(source).to_string(),
            &adm_status,
            &source.to_json_value()?,
            &KeyKind::from(key).to_string(),
            &key.to_json_value()?,
        ],
    )
    .await
    .context("failed to insert lookup table")?;
    Ok(lookup_table_id)
}

pub async fn create_data(
    tx: &Transaction<'_>,
    host_name: &str,
    endpoint_name: &str,
    item_name: &str,
    processor_id: Option<Uuid>,
    monitored_by: Option<Uuid>,
) -> anyhow::Result<()> {
    let host_id = Uuid::now_v7();
    let endpoint_id = Uuid::now_v7();
    let host_template_id = Uuid::now_v7();
    let item_id = Uuid::now_v7();
    tx.execute(
        "INSERT INTO hosts (host_id, name, adm_status, last_updated, monitored_by) VALUES ($1, $2, 1, now(), $3)",
        &[&host_id, &host_name, &monitored_by],
    )
    .await
    .context("failed to insert host")?;
    let value: serde_json::Value =
        serde_json::json!({ "address": "192.168.88.1:161", "community": "bangunindO" });
    tx.execute(
        "INSERT INTO host_endpoints (endpoint_id, host_id, name, type, adm_status, last_updated, config) \
        VALUES ($1, $2, $3, $4, 1, now(), $5)",
        &[&endpoint_id, &host_id, &endpoint_name, &"snmp_v2c", &value],
    )
        .await
        .context("failed to insert endpoint")?;
    tx.execute(
        "INSERT INTO host_templates (host_template_id, host_id) VALUES ($1, $2)",
        &[&host_template_id, &host_id],
    )
    .await
    .context("failed to insert host template")?;
    let value: serde_json::Value = serde_json::json!({ "oids": [{"oid": "ifIndex"}, {"oid": "ifDescr"}, {"oid": "ifName"}, {"oid": "ifAlias"}] });
    tx.execute(
        "INSERT INTO items (item_id, host_template_id, endpoint_id, name, type, adm_status, last_updated, config, schedule, timeout_ms) \
        VALUES ($1, $2, $3, $4, $5, 1, now(), $6, $7, $8)",
        &[&item_id, &host_template_id, &endpoint_id, &item_name, &"snmp_walk", &value, &"*/5 * * * * * *", &10000],
    )
        .await
        .context("failed to insert item")?;
    if let Some(processor_id) = processor_id {
        tx.execute(
            "INSERT INTO item_processors (item_id, processor_id) VALUES ($1, $2)",
            &[&item_id, &processor_id],
        )
        .await
        .context("failed to insert item processor")?;
    }
    Ok(())
}
