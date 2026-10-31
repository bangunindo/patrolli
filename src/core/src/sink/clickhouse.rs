// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod ddl;
pub mod record_batch;

use super::export::{Encoded, Export, ExportSink, Exporter};
use crate::common::health::{Health, SendGuard};
use crate::common::internal_metrics::SinkEndpoint;
use crate::config::sink::{ClickhouseCompression, ClickhouseCreateSchema, ClickhouseDistribution};
use crate::{chan, config, payload};
use anyhow::{Result, anyhow};
use arrow::array::RecordBatch;
use async_trait::async_trait;
use clickhouse::error::Error as ChError;
use clickhouse::{Client as ChClient, Compression};
use clickhouse_ext_arrow::ArrowClientExt;
use futures::future::join_all;
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use record_batch::{ChTable, to_record_batches};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{OnceCell, Semaphore};
use tokio::time::timeout;
use tracing::{error, info, trace, warn};
use uuid::Uuid;

/// Mirrors clickhouse-rs's own defaults for the connection pool (`src/http_client.rs`),
/// which we lose by supplying a custom hyper client.
const TCP_KEEPALIVE: Duration = Duration::from_secs(60);
/// ClickHouse's server-side keep-alive is 3s by default, so expire idle sockets first.
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(2);

/// clickhouse-rs exposes no TLS settings of its own: its `rustls-tls` feature only wires up
/// a connector trusting the public roots. A custom CA, mTLS or `insecure_skip_verify` has to
/// be injected by building the hyper client ourselves and handing it over via
/// [`ChClient::with_http_client`].
fn build_tls_client(tls: &config::Tls) -> Result<ChClient> {
    let tls_config = crate::common::tls::build_rustls_client_config(tls)?;

    let mut http = HttpConnector::new();
    http.set_keepalive(Some(TCP_KEEPALIVE));
    // HttpConnector rejects non-`http` URIs unless this is off; HttpsConnector enforces
    // the scheme itself.
    http.enforce_http(false);

    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_or_http()
        .enable_http1()
        .wrap_connector(http);

    let hyper_client = HyperClient::builder(TokioExecutor::new())
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .build(https);

    Ok(ChClient::with_http_client(hyper_client))
}

/// Session settings shared by every statement this sink issues — queries *and* inserts.
///
/// These have to sit on the [`ChClient`] rather than on each statement: `clickhouse-ext-arrow`
/// gives `ArrowInsert` no per-insert setting knob, and it builds itself from a clone of this
/// client, so client level is the only place `async_insert` can reach an `INSERT` at all.
/// `ping` and the DDL inherit the same settings for free.
fn apply_settings(mut client: ChClient, conf: &config::sink::Clickhouse) -> ChClient {
    if !conf.write_timeout.is_zero() {
        client = client.with_setting(
            "max_execution_time",
            conf.write_timeout.as_secs().to_string(),
        );
    }
    if conf.async_insert {
        client = client.with_setting("async_insert", "1").with_setting(
            "wait_for_async_insert",
            if conf.wait_for_async_insert { "1" } else { "0" },
        );
    }
    // Applied last so the escape hatch can override anything modelled above.
    for (name, value) in &conf.settings {
        client = client.with_setting(name, value);
    }
    client
}

struct Client {
    sink_config: Arc<config::sink::Sink>,
    client: ChClient,
    health: Health,
    /// `max_concurrent_inserts` for this endpoint; `None` when unlimited.
    inserts: Option<Semaphore>,
}

impl Client {
    fn conf(&self) -> &config::sink::Clickhouse {
        match &self.sink_config.config {
            config::sink::Config::Clickhouse(ch) => ch,
            _ => unreachable!(),
        }
    }

    async fn ping(&self) -> Result<()> {
        let i = self.client.query("SELECT 1").fetch_one::<u8>().await?;
        if i != 1 {
            return Err(anyhow!("Clickhouse server returned unexpected value"));
        }
        Ok(())
    }

    /// Runs the OpenTelemetry Clickhouse exporter's DDL — database, one table per signal,
    /// plus the trace-id/timestamp lookup table and the materialized view that fills it.
    /// See [`ddl`] for the schema itself.
    ///
    /// Every statement is `IF NOT EXISTS`, so this is safe to run from each endpoint (and
    /// from every node when `cluster_name` is set). Existing tables are left untouched —
    /// a table whose columns have drifted from this schema will fail later, at insert
    /// time, not here.
    async fn create_schema(&self, create_schema: &ClickhouseCreateSchema) -> Result<()> {
        let conf = self.conf();
        let client = self
            .client
            .clone()
            .with_database(ddl::DEFAULT_DATABASE)
            .with_validation(false);

        let ddl = ddl::Ddl::new(
            &conf.database,
            &conf.tables,
            create_schema,
            self.has_full_text_search(&client).await,
        );
        for (label, sql) in ddl.statements() {
            trace!(%sql, "running schema ddl");
            client
                .query(&sql)
                .execute()
                .await
                .map_err(|e| anyhow!("{label}: {e}"))?;
        }
        Ok(())
    }

    /// Whether the server is new enough for `TYPE text(...)` skip indexes on the logs
    /// table. Anything we can't parse falls back to bloom filters, which every supported
    /// Clickhouse understands.
    async fn has_full_text_search(&self, client: &ChClient) -> bool {
        let version = match client.query("SELECT version()").fetch_one::<String>().await {
            Ok(version) => version,
            Err(e) => {
                warn!(error = %e, "failed to read clickhouse server version, using bloom filter indexes");
                return false;
            }
        };
        let mut parts = version.split('.').map(str::parse::<u32>);
        match (parts.next(), parts.next()) {
            (Some(Ok(major)), Some(Ok(minor))) => {
                (major, minor) >= ddl::MIN_FULL_TEXT_SEARCH_VERSION
            }
            _ => {
                warn!(
                    version,
                    "unrecognized clickhouse server version, using bloom filter indexes"
                );
                false
            }
        }
    }
}

struct ClientCollection {
    clients: Vec<Client>,
    rr_cursor: AtomicUsize,
}

impl ClientCollection {
    /// Walk the ring from `start` and take the first client that hands out a send slot.
    /// `None` when every endpoint is backing off.
    fn pick_from(&self, start: usize) -> Option<(&Client, SendGuard<'_>)> {
        let n = self.clients.len();
        (0..n).find_map(|offset| {
            let client = &self.clients[(start + offset) % n];
            client.health.try_send().map(|guard| (client, guard))
        })
    }

    fn pick_random(&self) -> Option<(&Client, SendGuard<'_>)> {
        self.pick_from(rand::random_range(0..self.clients.len()))
    }

    fn pick_round_robin(&self) -> Option<(&Client, SendGuard<'_>)> {
        let rr_cursor = self.rr_cursor.fetch_add(1, Ordering::Relaxed);
        let start = rr_cursor % self.clients.len();
        self.pick_from(start)
    }

    fn pick_failover(&self) -> Option<(&Client, SendGuard<'_>)> {
        self.pick_from(0)
    }

    /// How long until some endpoint takes a send again.
    fn retry_in(&self) -> Duration {
        self.clients
            .iter()
            .map(|client| client.health.retry_in())
            .min()
            .unwrap_or_default()
    }
}

/// Stream every batch destined for one table into a single `INSERT … FORMAT ArrowStream`.
///
/// All batches for a table carry the same [`ChTable::schema`] — a clone of a shared
/// `LazyLock` — which is what lets them share one insert: `clickhouse-ext-arrow` builds the
/// statement from the *first* batch's schema and forbids the shape changing afterwards.
///
/// The table name is passed bare and resolved by the client's database. It must not be
/// qualified: the escaping backtick-wraps the whole string, so `db.table` would become one
/// nonexistent identifier.
async fn insert_table(
    client: &ChClient,
    table: &str,
    batches: &[RecordBatch],
) -> Result<(), ChError> {
    let mut insert = client.insert_arrow(table)?;
    for batch in batches {
        insert.write(batch).await?;
    }
    // `end()` is where the server reports a rejected column or type.
    insert.end().await
}

/// One table's share of a batch.
#[derive(Clone)]
pub struct TableBatch {
    table: ChTable,
    batches: Vec<RecordBatch>,
    rows: u64,
    /// `insert_deduplication_token`, the same on every attempt: an insert that timed out may
    /// still have landed, and a table that deduplicates inserts then drops the retry.
    token: String,
}

/// The rows of a batch, by destination table.
pub struct Tables(Vec<TableBatch>);

pub type ClickhouseSink = ExportSink<Clickhouse>;

impl ClickhouseSink {
    // `Worker::new` builds from a `Prepared` exporter; this is for tests/clickhouse_insert_test.rs.
    #[allow(dead_code)]
    pub async fn new(
        sink_config: config::sink::Sink,
        channel: chan::Consumer<payload::Payload>,
    ) -> Result<Self> {
        let exporter = Clickhouse::new(&sink_config);
        Ok(ExportSink::with_exporter(sink_config, channel, exporter))
    }
}

pub struct Clickhouse {
    sink_config: config::sink::Sink,
    clients: OnceCell<ClientCollection>,
}

impl Clickhouse {
    pub(super) fn new(sink_config: &config::sink::Sink) -> Self {
        debug_assert!(matches!(
            &sink_config.config,
            config::sink::Config::Clickhouse(_)
        ));
        Self {
            sink_config: sink_config.clone(),
            clients: OnceCell::new(),
        }
    }

    fn conf(&self) -> &config::sink::Clickhouse {
        match &self.sink_config.config {
            config::sink::Config::Clickhouse(ch) => ch,
            _ => unreachable!(),
        }
    }

    /// Initialize ClickHouse clients and create db schema if needed. Returns error if there's
    /// a schema mismatch with existing tables.
    async fn initialize_clients(&self) -> Result<ClientCollection> {
        info!("initializing clients");
        let mut clients = Vec::with_capacity(self.conf().endpoints.len());
        let conf = Arc::new(self.sink_config.clone());
        for endpoint in self.conf().endpoints.iter() {
            let base = match &self.conf().tls {
                Some(tls) => build_tls_client(tls)?,
                None => ChClient::default(),
            };
            let mut ch_client = base
                .with_url(endpoint)
                .with_database(&self.conf().database)
                .with_validation(self.conf().enable_schema_validation);
            if let (Some(user), Some(pass)) = (&self.conf().username, &self.conf().password) {
                ch_client = ch_client.with_user(user).with_password(pass.expose()?);
            }
            match self.conf().compression {
                ClickhouseCompression::None => {
                    ch_client = ch_client.with_compression(Compression::None);
                }
                ClickhouseCompression::Lz4 => {
                    // `compress=1` responses use the server's `network_compression_method`,
                    // which defaults to ZSTD since 26.9, while the client only decodes LZ4
                    // frames there. Pin it, or every read fails with "incorrect magic number".
                    ch_client = ch_client
                        .with_compression(Compression::Lz4)
                        .with_setting("network_compression_method", "lz4");
                }
                ClickhouseCompression::Zstd => {
                    ch_client = ch_client.with_compression(Compression::zstd());
                }
            }
            let ch_client = apply_settings(ch_client, self.conf());
            let client = Client {
                sink_config: conf.clone(),
                client: ch_client,
                health: Health::new(endpoint.as_str()),
                inserts: insert_limit(self.conf()),
            };
            if let Some(create_schema) = &self.conf().create_schema
                && let Err(e) = client.create_schema(create_schema).await
            {
                error!(error = %format!("{e:#}"), endpoint, "failed to create clickhouse schema");
            }
            match client.ping().await {
                Ok(_) => {
                    client.health.set_healthy();
                }
                Err(e) => {
                    warn!(error = %format!("{e:#}"), endpoint, "failed to ping clickhouse server");
                    client.health.set_unhealthy();
                }
            }
            clients.push(client);
        }
        Ok(ClientCollection {
            clients,
            rr_cursor: AtomicUsize::new(0),
        })
    }
}

/// One endpoint's share of `max_concurrent_inserts`, or `None` for no limit.
fn insert_limit(conf: &config::sink::Clickhouse) -> Option<Semaphore> {
    (conf.max_concurrent_inserts > 0).then(|| Semaphore::new(conf.max_concurrent_inserts))
}

/// Whether a failed insert may land if sent again: the endpoint could not be reached, did not
/// answer in time, or answered with an error it recovers from on its own (overload, a
/// replica that is read-only for now). Anything else (a rejected column, a missing table)
/// fails identically on every attempt and every endpoint.
fn is_retryable(e: &ChError) -> bool {
    match e {
        ChError::Network(_) | ChError::TimedOut => true,
        ChError::BadResponse(reason) => is_transient_response(reason),
        // `clickhouse-ext-arrow` writes through an `io::Write`, so a failed write comes back
        // as an Arrow I/O error over the client's own error.
        ChError::Other(source) => is_transient_source(source.as_ref()),
        _ => false,
    }
}

/// Server error codes that clear up without anyone changing the insert.
const TRANSIENT_CODES: &[u32] = &[
    159, // TIMEOUT_EXCEEDED: `max_execution_time`, which `write_timeout` sets
    164, // READONLY: a replica that lost Keeper
    202, // TOO_MANY_SIMULTANEOUS_QUERIES
    203, // NO_FREE_CONNECTION
    209, // SOCKET_TIMEOUT
    210, // NETWORK_ERROR: between the server and a shard or replica
    241, // MEMORY_LIMIT_EXCEEDED
    242, // TABLE_IS_READ_ONLY
    252, // TOO_MANY_PARTS: merges are behind
    285, // TOO_FEW_LIVE_REPLICAS
    319, // UNKNOWN_STATUS_OF_INSERT: the deduplication token makes the retry safe
    999, // KEEPER_EXCEPTION
];

/// HTTP statuses a proxy in front of the server answers while it is overloaded or away.
const TRANSIENT_STATUSES: &[&str] = &["429", "502", "503", "504"];

/// A server exception reads `Code: <n>. DB::Exception: …`, or just `Code: <n>` when only the
/// header carried it; with no code, the answer came from something in front of the server,
/// and the client reports it as `<status> <reason>` when the body was empty.
fn is_transient_response(reason: &str) -> bool {
    if let Some(rest) = reason.split("Code: ").nth(1) {
        let digits = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        return rest[..digits]
            .parse()
            .is_ok_and(|code| TRANSIENT_CODES.contains(&code));
    }
    reason
        .split(' ')
        .next()
        .is_some_and(|status| TRANSIENT_STATUSES.contains(&status))
}

fn is_transient_source(e: &(dyn std::error::Error + 'static)) -> bool {
    let mut next = Some(e);
    while let Some(e) = next {
        if let Some(ch) = e.downcast_ref::<ChError>() {
            return is_retryable(ch);
        }
        // `io::Error::source` skips the error it wraps, which is the one that says.
        if let Some(io) = e.downcast_ref::<std::io::Error>()
            && let Some(ch) = io.get_ref().and_then(|e| e.downcast_ref::<ChError>())
        {
            return is_retryable(ch);
        }
        next = e.source();
    }
    false
}

#[async_trait]
impl Exporter for Clickhouse {
    type Encoded = Tables;

    async fn start(&self) -> Result<()> {
        self.clients
            .get_or_try_init(|| self.initialize_clients())
            .await?;
        Ok(())
    }

    /// Rebatch, then OTAP -> Clickhouse-shaped Arrow. Pure CPU, no awaits.
    fn encode(&self, mut batch: payload::PayloadBatch) -> Encoded<Tables> {
        let mut by_table: HashMap<ChTable, Vec<RecordBatch>> = HashMap::new();
        let mut converted = 0u64;
        let mut errored = 0u64;
        // `None` means "up to `u16::MAX` items per payload", i.e. merge as much as allowed:
        // one insert per table beats one per source payload for a columnar destination.
        for mut pl in batch.batch(None) {
            let items = pl.num_items() as u64;
            match to_record_batches(&mut pl) {
                Ok(batches) => {
                    converted += items;
                    for (table, rb) in batches {
                        by_table.entry(table).or_default().push(rb);
                    }
                }
                Err(e) => {
                    // Conversion is all-or-nothing per payload — a failure partway through
                    // metrics discards the tables already built — so the whole payload is lost.
                    errored += items;
                    error!(error = %format!("{e:#}"), "failed to convert payload for clickhouse");
                }
            }
        }
        let tables: Vec<TableBatch> = by_table
            .into_iter()
            .map(|(table, batches)| TableBatch {
                table,
                rows: batches.iter().map(|rb| rb.num_rows() as u64).sum(),
                batches,
                token: Uuid::now_v7().to_string(),
            })
            .collect();
        // Every destination row is one item, and the gauge/sum split sends each number data
        // point to exactly one table, so `rows <= converted` always holds; the difference is
        // what the converter filtered out.
        let rows: u64 = tables.iter().map(|t| t.rows).sum();
        Encoded {
            data: (!tables.is_empty()).then_some(Tables(tables)),
            filtered: converted.saturating_sub(rows),
            errored,
        }
    }

    fn items(&self, tables: &Tables) -> u64 {
        tables.0.iter().map(|t| t.rows).sum()
    }

    /// One endpoint for the whole batch, and one insert per table, all in flight together
    /// as far as the endpoint's `max_concurrent_inserts` allows.
    async fn export(&self, tables: &Tables) -> Export<Tables> {
        let clients = self
            .clients
            .get()
            .expect("clients are initialized when the worker starts");
        let picked = match self.conf().distribution {
            ClickhouseDistribution::RoundRobin => clients.pick_round_robin(),
            ClickhouseDistribution::Random => clients.pick_random(),
            ClickhouseDistribution::Failover => clients.pick_failover(),
        };
        let Some((client, mut guard)) = picked else {
            return Export::Retry {
                error: anyhow!("every clickhouse endpoint is backing off"),
                after: Some(clients.retry_in()),
            };
        };
        let names = &self.conf().tables;
        let write_timeout = self.conf().write_timeout;
        let results = join_all(tables.0.iter().map(|t| async move {
            // Waiting for a turn is not the endpoint being slow, so it comes before the
            // timeout starts. The semaphore is never closed.
            let _permit = match &client.inserts {
                Some(inserts) => Some(inserts.acquire().await.expect("semaphore is never closed")),
                None => None,
            };
            // A per-insert setting has to go on the client: `ArrowInsert` takes its settings
            // from the client it is built from.
            let ch = client
                .client
                .clone()
                .with_setting("insert_deduplication_token", t.token.as_str());
            let insert = insert_table(&ch, t.table.name(names), &t.batches);
            let res = if write_timeout.is_zero() {
                insert.await.map_err(anyhow::Error::from)
            } else {
                match timeout(write_timeout, insert).await {
                    Ok(res) => res.map_err(anyhow::Error::from),
                    Err(_) => Err(anyhow!("timed out after {write_timeout:?}")),
                }
            };
            (t, res)
        }))
        .await;

        let mut retry = Vec::new();
        let mut retry_error = None;
        let (mut rejected, mut rejected_tables, mut reject_error) = (0u64, 0usize, None);
        for (t, res) in results {
            let Err(e) = res else { continue };
            // Our own `write_timeout` firing counts as retryable, like the client's.
            let retryable = e.downcast_ref::<ChError>().is_none_or(is_retryable);
            let e = e.context(format!("insert into {}", t.table.name(names)));
            if retryable {
                retry.push(t.clone());
                retry_error.get_or_insert(e);
            } else {
                rejected += t.rows;
                rejected_tables += 1;
                reject_error.get_or_insert(e);
            }
        }
        // Only what is retried (the endpoint unreachable or overloaded) may trip the breaker,
        // which sends the next batches to the other endpoints. A rejected column fails identically on
        // every endpoint, so counting that as ill health would walk all of them into backoff
        // and stall the sink instead of just dropping the batch that cannot land.
        if retry.is_empty() {
            guard.succeeded();
        } else {
            guard.failed();
        }
        drop(guard);

        let delivered = tables.0.len() - retry.len() - rejected_tables;
        match (retry_error, reject_error) {
            (None, None) => Export::Delivered,
            (Some(error), None) if delivered == 0 => Export::Retry { error, after: None },
            (None, Some(error)) if delivered == 0 => Export::Reject { error },
            (retry_error, reject_error) => Export::Partial {
                retry: retry_error.map(|error| (Tables(retry), error)),
                rejected: reject_error.map(|error| (rejected, error)),
            },
        }
    }

    fn endpoints(&self) -> Vec<SinkEndpoint> {
        self.clients.get().map_or_else(Vec::new, |clients| {
            clients
                .clients
                .iter()
                .map(|client| SinkEndpoint {
                    endpoint: client.health.endpoint().to_string(),
                    up: client.health.is_healthy(),
                })
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::Sink;
    use super::*;
    use axum::body::Bytes;
    use axum::extract::State;
    use otel_arrow_dfe_pdata::encode::encode_logs_otap_batch;
    use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
        LogRecord, LogsData, ResourceLogs, ScopeLogs,
    };
    use tokio_util::sync::CancellationToken;

    fn bad(reason: &str) -> ChError {
        ChError::BadResponse(reason.to_string())
    }

    #[test]
    fn overload_and_proxy_errors_are_retried_and_bad_inserts_are_not() {
        for reason in [
            "Code: 252. DB::Exception: Too many parts (300). (TOO_MANY_PARTS) (version 25.8.1.1)",
            "Code: 241. DB::Exception: Memory limit (total) exceeded. (MEMORY_LIMIT_EXCEEDED)",
            "Code: 159",
            "503 Service Unavailable",
            "429 Too Many Requests",
        ] {
            assert!(is_retryable(&bad(reason)), "{reason}");
        }
        for reason in [
            "Code: 60. DB::Exception: Table default.otel_logs does not exist. (UNKNOWN_TABLE)",
            "Code: 16. DB::Exception: No such column foo. (NO_SUCH_COLUMN_IN_TABLE)",
            "Code: 2520",
            "404 Not Found",
            "<html><body>Bad Gateway</body></html>",
        ] {
            assert!(!is_retryable(&bad(reason)), "{reason}");
        }
    }

    /// The shape a failed write takes through `clickhouse-ext-arrow`.
    #[test]
    fn a_network_error_under_an_arrow_write_is_retried() {
        let wrapped = |inner: ChError| {
            let io = std::io::Error::other(inner);
            ChError::Other(arrow::error::ArrowError::IoError("write".into(), io).into())
        };
        assert!(is_retryable(&wrapped(ChError::Network("reset".into()))));
        assert!(!is_retryable(&wrapped(bad("Code: 60. DB::Exception: …"))));
        assert!(!is_retryable(&ChError::Other(
            arrow::error::ArrowError::SchemaError("no fields".into()).into()
        )));
    }

    /// Inserts an endpoint has in flight: now, and the most at once.
    #[derive(Default)]
    struct InFlight {
        now: AtomicUsize,
        max: AtomicUsize,
        total: AtomicUsize,
    }

    /// An HTTP endpoint that takes every insert after holding it for `hold`.
    async fn fake_endpoint(hold: Duration) -> (String, Arc<InFlight>) {
        async fn insert(State((seen, hold)): State<(Arc<InFlight>, Duration)>, _body: Bytes) {
            let now = seen.now.fetch_add(1, Ordering::SeqCst) + 1;
            seen.max.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(hold).await;
            seen.now.fetch_sub(1, Ordering::SeqCst);
            seen.total.fetch_add(1, Ordering::SeqCst);
        }
        let seen = Arc::new(InFlight::default());
        let app = axum::Router::new()
            .fallback(insert)
            .with_state((seen.clone(), hold));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, seen)
    }

    fn logs(records: usize) -> payload::Payload {
        let logs = LogsData {
            resource_logs: vec![ResourceLogs {
                resource: None,
                scope_logs: vec![ScopeLogs {
                    scope: None,
                    log_records: vec![LogRecord::default(); records],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        payload::Payload::new(encode_logs_otap_batch(&logs).unwrap())
    }

    /// `workers` workers, each closing a batch per payload of 5 logs (one insert, into the
    /// logs table), sending `payloads` payloads to one endpoint that holds each insert 100ms.
    /// Every record must be sent on the first attempt.
    async fn run(
        workers: usize,
        max_concurrent_inserts: usize,
        write_timeout_ms: u64,
        payloads: usize,
    ) -> Arc<InFlight> {
        let (url, seen) = fake_endpoint(Duration::from_millis(100)).await;
        let mut conf: config::sink::Sink = serde_yaml_ng::from_str(&format!(
            r#"
id: 00000000-0000-0000-0000-000000000005
name: clickhouse
worker_count: {workers}
batch_size: 5
batch_timeout: 3600000
type: clickhouse
config:
  endpoints: [{url}]
  max_concurrent_inserts: {max_concurrent_inserts}
  write_timeout: {write_timeout_ms}
"#
        ))
        .unwrap();
        conf.validate().unwrap();
        let config::sink::Config::Clickhouse(ch) = &conf.config else {
            unreachable!()
        };
        // Skips `initialize_clients`, which pings and creates a schema the fake can't serve.
        let health = Health::new(url.as_str());
        health.set_healthy();
        let client = Client {
            sink_config: Arc::new(conf.clone()),
            client: ChClient::default().with_url(&url),
            health,
            inserts: insert_limit(ch),
        };
        let exporter = Clickhouse {
            sink_config: conf.clone(),
            clients: OnceCell::new_with(Some(ClientCollection {
                clients: vec![client],
                rr_cursor: AtomicUsize::new(0),
            })),
        };

        let (tx, rx) = flume::bounded(payloads);
        let consumer = chan::Consumer::from_flume(&tx, rx);
        for _ in 0..payloads {
            tx.send(logs(5)).unwrap();
        }
        drop(tx);
        let sink = Arc::new(ExportSink::with_exporter(conf, consumer, exporter));
        let token = CancellationToken::new();
        join_all((0..workers).map(|_| sink.run(token.clone()))).await;
        let m = sink.metrics();
        assert_eq!(m.total_records_sent, payloads as u64 * 5);
        assert_eq!(m.total_records_errored, 0);
        assert_eq!(m.total_records_retried, 0);
        seen
    }

    #[tokio::test]
    async fn max_concurrent_inserts_caps_what_an_endpoint_has_in_flight() {
        let seen = run(6, 2, 30_000, 12).await;
        assert_eq!(seen.total.load(Ordering::SeqCst), 12);
        assert_eq!(
            seen.max.load(Ordering::SeqCst),
            2,
            "never more than the cap"
        );
    }

    #[tokio::test]
    async fn without_a_cap_every_worker_has_an_insert_in_flight() {
        let seen = run(6, 0, 30_000, 12).await;
        assert_eq!(seen.total.load(Ordering::SeqCst), 12);
        assert!(
            seen.max.load(Ordering::SeqCst) > 2,
            "workers insert concurrently: {}",
            seen.max.load(Ordering::SeqCst)
        );
    }

    /// One insert at a time, each held 100ms, with a 150ms `write_timeout`: the last of six
    /// workers waits ~500ms for its turn, which must not count as the insert timing out.
    #[tokio::test]
    async fn waiting_for_a_turn_does_not_count_toward_the_write_timeout() {
        let seen = run(6, 1, 150, 6).await;
        assert_eq!(seen.total.load(Ordering::SeqCst), 6);
        assert_eq!(seen.max.load(Ordering::SeqCst), 1);
    }
}
