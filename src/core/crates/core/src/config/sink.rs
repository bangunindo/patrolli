// SPDX-FileCopyrightText: 2025 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::sensitive::{Sensitive, SensitiveFields, SensitiveRefs};
use super::{Batch, Channel, Format, Retry};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;
use strum_macros::{Display, EnumDiscriminants, EnumString};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, EnumDiscriminants, ToSchema)]
#[serde(rename_all = "snake_case", tag = "type", content = "config")]
#[schema(as = SinkConfig)]
#[strum_discriminants(
    name(SinkKind),
    derive(Serialize, Deserialize, EnumString, Display, ToSchema),
    vis(pub),
    strum(serialize_all = "snake_case"),
    serde(rename_all = "snake_case")
)]
pub enum Config {
    File(File),
    Kafka(Kafka),
    OpenTelemetry(OpenTelemetry),
    Clickhouse(Box<Clickhouse>),
}

impl Config {
    pub fn validate(&mut self) -> Result<()> {
        match self {
            Self::File(file) => file.validate()?,
            Self::Kafka(kafka) => kafka.validate()?,
            Self::OpenTelemetry(open_telemetry) => open_telemetry.validate()?,
            Self::Clickhouse(clickhouse) => clickhouse.validate()?,
        };
        Ok(())
    }

    pub fn to_json_value(&self) -> Result<serde_json::Value> {
        match self {
            Self::File(config) => Ok(serde_json::to_value(config)?),
            Self::Kafka(config) => Ok(serde_json::to_value(config)?),
            Self::OpenTelemetry(open_telemetry) => Ok(serde_json::to_value(open_telemetry)?),
            Self::Clickhouse(clickhouse) => Ok(serde_json::to_value(clickhouse)?),
        }
    }
}

impl SensitiveFields for Config {
    fn sensitive_mut(&mut self) -> SensitiveRefs<'_> {
        match self {
            Self::File(_) => Vec::new(),
            Self::Kafka(kafka) => kafka.sensitive_mut(),
            Self::OpenTelemetry(open_telemetry) => open_telemetry
                .tls
                .as_mut()
                .map_or_else(Vec::new, |tls| tls.sensitive_mut("tls.key_password")),
            Self::Clickhouse(clickhouse) => clickhouse.sensitive_mut(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sink {
    pub id: Uuid,
    pub name: String,
    #[serde(default = "super::default_worker_count")]
    pub worker_count: usize,
    #[serde(default)]
    pub inputs: Vec<Uuid>,
    #[serde(flatten, default)]
    pub batch: Batch,
    #[serde(default)]
    pub channel: Channel,
    #[serde(default)]
    pub retry: Retry,
    #[serde(flatten)]
    pub config: Config,
}

impl Sink {
    pub fn validate(&mut self) -> Result<()> {
        self.batch.normalize()?;
        self.channel.validate()?;
        self.retry.validate()?;
        self.config.validate()?;
        if self.channel.write_through()
            && let Config::Kafka(kafka) = &self.config
            && kafka.required_acks != KafkaRequiredAcks::All
        {
            // Anything less acks a batch the broker can still lose.
            anyhow::bail!("write_through on a kafka sink requires required_acks: all");
        }
        if self.worker_count == 0 {
            self.worker_count = super::default_worker_count();
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub path: String,
    #[serde(default)]
    pub format: Format,
    #[serde(default)]
    pub throttle: usize,
}

impl File {
    pub fn validate(&mut self) -> Result<()> {
        Ok(())
    }
}

#[derive(Default, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum KafkaKeyStrategy {
    #[default]
    ItemId,
    HostId,
    ListenerId,
}

#[derive(Default, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum KafkaBalancerStrategy {
    Fnv1a,
    #[default]
    Murmur2,
    Random,
    Consistent,
    Fnv1aRandom,
    Murmur2Random,
    ConsistentRandom,
}

impl KafkaBalancerStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            KafkaBalancerStrategy::Fnv1a => "fnv1a",
            KafkaBalancerStrategy::Murmur2 => "murmur2",
            KafkaBalancerStrategy::Random => "random",
            KafkaBalancerStrategy::Consistent => "consistent",
            KafkaBalancerStrategy::Fnv1aRandom => "fnv1a_random",
            KafkaBalancerStrategy::Murmur2Random => "murmur2_random",
            KafkaBalancerStrategy::ConsistentRandom => "consistent_random",
        }
    }
}

#[derive(Default, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum KafkaCompression {
    #[default]
    None,
    Gzip,
    Snappy,
    Lz4,
    Zstd,
}

impl KafkaCompression {
    pub fn as_str(&self) -> &'static str {
        match self {
            KafkaCompression::None => "none",
            KafkaCompression::Gzip => "gzip",
            KafkaCompression::Snappy => "snappy",
            KafkaCompression::Lz4 => "lz4",
            KafkaCompression::Zstd => "zstd",
        }
    }
}

#[derive(Default, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum KafkaRequiredAcks {
    /// `acks=0`: delivered once written to the leader's socket. The broker
    /// answers nothing, so its errors look like success.
    None,
    /// `acks=1`: delivered once the partition leader has written it.
    #[default]
    One,
    /// Delivered once every in-sync replica has it; the only durable setting.
    All,
}

impl KafkaRequiredAcks {
    pub fn as_str(&self) -> &'static str {
        match self {
            KafkaRequiredAcks::None => "0",
            KafkaRequiredAcks::One => "1",
            KafkaRequiredAcks::All => "all",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum KafkaSaslMechanism {
    Plain,
    #[serde(rename = "sha256")]
    SHA256,
    #[serde(rename = "sha512")]
    SHA512,
}

impl KafkaSaslMechanism {
    pub fn as_str(&self) -> &'static str {
        match self {
            KafkaSaslMechanism::Plain => "PLAIN",
            KafkaSaslMechanism::SHA256 => "SCRAM-SHA-256",
            KafkaSaslMechanism::SHA512 => "SCRAM-SHA-512",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub enum KafkaSecurityProtocol {
    #[serde(rename = "plaintext")]
    PlainText,
    #[serde(rename = "ssl")]
    Ssl,
    #[serde(rename = "sasl_plaintext")]
    SaslPlainText,
    #[serde(rename = "sasl_ssl")]
    SaslSSL,
}

impl KafkaSecurityProtocol {
    pub fn as_str(&self) -> &'static str {
        match self {
            KafkaSecurityProtocol::PlainText => "plaintext",
            KafkaSecurityProtocol::Ssl => "ssl",
            KafkaSecurityProtocol::SaslPlainText => "sasl_plaintext",
            KafkaSecurityProtocol::SaslSSL => "sasl_ssl",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct KafkaSasl {
    pub mechanism: KafkaSaslMechanism,
    pub username: String,
    pub password: Sensitive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct KafkaSecurity {
    pub protocol: KafkaSecurityProtocol,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Kafka {
    pub hosts: Vec<String>,
    pub topic: String,
    #[serde(default)]
    pub key_strategy: KafkaKeyStrategy,
    #[serde(default)]
    pub allow_empty_key: bool,
    pub format: Format,
    /// How long one attempt at a batch may take (`message.timeout.ms`); a
    /// batch not delivered by then is retried per the sink's `retry:`.
    /// 0 takes the default of 30.
    #[serde(default = "default_write_timeout")]
    pub write_timeout_seconds: u64,

    #[serde(default)]
    pub balancer_strategy: KafkaBalancerStrategy,
    #[serde(default)]
    pub required_acks: KafkaRequiredAcks,
    #[serde(default)]
    pub compression: KafkaCompression,
    pub security: KafkaSecurity,
    pub ssl: Option<super::Tls>,
    pub sasl: Option<KafkaSasl>,
    /// How many librdkafka producers the sink's workers share, taken
    /// round-robin per batch. Each worker has one batch in flight, so
    /// `worker_count` is the write concurrency; this only sets how many clients
    /// (each with its own queue, broker threads and connections) carry it.
    #[serde(default = "default_kafka_producer_count")]
    pub producer_count: usize,
    #[serde(default = "default_partition_batch_size")]
    pub partition_batch_size: usize,
    #[serde(default = "default_partition_batch_count")]
    pub partition_batch_count: usize,
    #[serde(default = "default_global_queue_size")]
    pub global_queue_size: usize,
    /// Largest message the producer sends, in bytes (librdkafka's
    /// `message.max.bytes`), checked before compression; a bigger one is
    /// rejected without being sent. The broker checks its own
    /// `max.message.bytes` against the compressed batch, so raising this lets a
    /// compressible message past librdkafka for the broker to judge. 0 takes
    /// librdkafka's default of 1000000.
    #[serde(default = "default_max_message_size")]
    pub max_message_size: usize,
    #[serde(default)]
    pub enable_producer_stats: bool,
}

fn default_partition_batch_size() -> usize {
    1_000_000
}

fn default_partition_batch_count() -> usize {
    10_000
}

fn default_global_queue_size() -> usize {
    1_048_576_000
}

fn default_max_message_size() -> usize {
    1_000_000
}

/// librdkafka's accepted range for `message.max.bytes`.
const MAX_MESSAGE_SIZE_RANGE: std::ops::RangeInclusive<usize> = 1_000..=1_000_000_000;

fn default_write_timeout() -> u64 {
    30
}

fn default_kafka_producer_count() -> usize {
    1
}

impl Kafka {
    pub fn validate(&mut self) -> anyhow::Result<()> {
        if self.hosts.is_empty() {
            anyhow::bail!("kafka hosts cannot be empty");
        }

        if self.topic.is_empty() {
            anyhow::bail!("kafka topic cannot be empty");
        }

        if self.producer_count == 0 {
            self.producer_count = default_kafka_producer_count();
        }
        if self.write_timeout_seconds == 0 {
            self.write_timeout_seconds = default_write_timeout();
        }
        if self.partition_batch_size == 0 {
            self.partition_batch_size = default_partition_batch_size();
        }
        if self.partition_batch_count == 0 {
            self.partition_batch_count = default_partition_batch_count();
        }
        if self.global_queue_size == 0 {
            self.global_queue_size = default_global_queue_size();
        }
        if self.max_message_size == 0 {
            self.max_message_size = default_max_message_size();
        }
        if !MAX_MESSAGE_SIZE_RANGE.contains(&self.max_message_size) {
            anyhow::bail!(
                "kafka max_message_size must be between {} and {} bytes",
                MAX_MESSAGE_SIZE_RANGE.start(),
                MAX_MESSAGE_SIZE_RANGE.end()
            );
        }

        match self.security.protocol {
            KafkaSecurityProtocol::PlainText => {
                if self.ssl.is_some() {
                    anyhow::bail!(
                        "Protocol is 'plaintext' but an 'ssl' block was provided. Remove the 'ssl' block or change the protocol to 'ssl' or 'sasl_ssl'."
                    );
                }
                if self.sasl.is_some() {
                    anyhow::bail!(
                        "Protocol is 'plaintext' but a 'sasl' block was provided. Remove the 'sasl' block or change the protocol to 'sasl_plaintext' or 'sasl_ssl'."
                    );
                }
            }

            KafkaSecurityProtocol::Ssl => {
                if self.sasl.is_some() {
                    anyhow::bail!(
                        "Protocol is 'ssl' (TLS only) but a 'sasl' block was provided. Use 'sasl_ssl' if you need both TLS and SASL."
                    );
                }
                let ssl = self.ssl.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("Missing Config: Protocol 'ssl' requires an 'ssl' block for encryption settings.")
                })?;
                ssl.validate()?;
                ssl.validate_client()?;
            }

            KafkaSecurityProtocol::SaslPlainText => {
                if self.ssl.is_some() {
                    anyhow::bail!(
                        "Protocol is 'sasl_plaintext' but an 'ssl' block was provided. Use 'sasl_ssl' if you need both TLS and SASL."
                    );
                }
                let sasl = self.sasl.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("Missing Config: Protocol 'sasl_plaintext' requires a 'sasl' block for authentication.")
                })?;
                self.validate_sasl(sasl)?;
            }

            KafkaSecurityProtocol::SaslSSL => {
                let ssl = self.ssl.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("Missing Config: Protocol 'sasl_ssl' requires an 'ssl' block for TLS encryption.")
                })?;
                let sasl = self.sasl.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("Missing Config: Protocol 'sasl_ssl' requires a 'sasl' block for credentials.")
                })?;

                ssl.validate()?;
                ssl.validate_client()?;
                self.validate_sasl(sasl)?;
            }
        }

        Ok(())
    }

    fn validate_sasl(&self, sasl: &KafkaSasl) -> anyhow::Result<()> {
        if sasl.username.is_empty() {
            anyhow::bail!("SASL Error: Username cannot be empty.");
        }
        if let Ok(password) = sasl.password.expose()
            && password.is_empty()
        {
            anyhow::bail!("SASL Error: Password cannot be empty.");
        }
        Ok(())
    }

    pub fn sensitive_mut(&mut self) -> SensitiveRefs<'_> {
        let mut refs = Vec::new();
        if let Some(ssl) = &mut self.ssl {
            refs.extend(ssl.sensitive_mut("ssl.key_password"));
        }
        if let Some(sasl) = &mut self.sasl {
            refs.push(("sasl.password", &mut sasl.password));
        }
        refs
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OtelShardKey {
    HostId,
    ItemId,
    EndpointId,
    ListenerId,
    SourceIp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, ToSchema)]
#[serde(rename_all = "snake_case", tag = "strategy", deny_unknown_fields)]
pub enum OtelDistribution {
    #[default]
    Broadcast,
    RoundRobin,
    Failover,
    Sharded {
        key: OtelShardKey,
    },
}

fn default_otap_connection_count() -> usize {
    1
}

#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OtelSinkMode {
    #[default]
    Otap,
    Otlp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenTelemetry {
    pub destinations: Vec<String>,
    #[serde(default)]
    pub distribution: OtelDistribution,
    #[serde(default)]
    pub mode: OtelSinkMode,
    #[serde(default = "default_otap_connection_count")]
    pub connection_count: usize,
    pub tls: Option<super::Tls>,
    #[serde(default)]
    pub compression: super::OtapCompression,
    #[serde(with = "crate::common::serde::duration_millis", default)]
    #[schema(value_type = u64)]
    pub write_timeout: Duration,
}

impl OpenTelemetry {
    pub fn validate(&mut self) -> Result<()> {
        if self.destinations.is_empty() {
            anyhow::bail!("open_telemetry sink destinations cannot be empty");
        }
        for dst in &self.destinations {
            if dst.is_empty() {
                anyhow::bail!("open_telemetry sink destination cannot be empty");
            }
        }

        if self.connection_count == 0 {
            self.connection_count = default_otap_connection_count();
        }

        if let Some(tls) = &self.tls {
            tls.validate()?;
            tls.validate_client()?;
        }

        Ok(())
    }
}

/// How batches are routed across `Clickhouse::endpoints`.
///
/// There is deliberately no `broadcast` (unlike [`OtelDistribution`]): sending
/// the same rows to every node would duplicate them, not replicate them.
/// Replication is Clickhouse's job — see `table_engine`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, ToSchema)]
#[serde(rename_all = "snake_case", tag = "strategy", deny_unknown_fields)]
pub enum ClickhouseDistribution {
    /// Rotate batches across endpoints.
    #[default]
    RoundRobin,
    /// Pick an endpoint at random per batch.
    Random,
    /// Stick to the first reachable endpoint, in the order listed.
    Failover,
}

/// Wire compression for the HTTP interface. Mirrors what the Clickhouse Rust
/// client supports — gzip/deflate/br are not offered.
#[derive(Default, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClickhouseCompression {
    None,
    #[default]
    Lz4,
    Zstd,
}

/// `ENGINE = <name>(<params>)` used when `create_schema` runs the DDL.
/// Combine with `cluster_name` and `ReplicatedMergeTree` for replication.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ClickhouseTableEngine {
    pub name: String,
    pub params: Vec<String>,
}

impl Default for ClickhouseTableEngine {
    fn default() -> Self {
        Self {
            name: "MergeTree".to_string(),
            params: Vec::new(),
        }
    }
}

/// Destination table per signal. Metrics fan out by datapoint type because the
/// column layouts differ; this matches the OpenTelemetry collector's schema so
/// existing Grafana dashboards keep working.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ClickhouseTables {
    pub logs: String,
    pub traces: String,
    pub metrics_gauge: String,
    pub metrics_sum: String,
    pub metrics_summary: String,
    pub metrics_histogram: String,
    pub metrics_exp_histogram: String,
}

impl Default for ClickhouseTables {
    fn default() -> Self {
        Self {
            logs: "otel_logs".to_string(),
            traces: "otel_traces".to_string(),
            metrics_gauge: "otel_metrics_gauge".to_string(),
            metrics_sum: "otel_metrics_sum".to_string(),
            metrics_summary: "otel_metrics_summary".to_string(),
            metrics_histogram: "otel_metrics_histogram".to_string(),
            metrics_exp_histogram: "otel_metrics_exp_histogram".to_string(),
        }
    }
}

impl ClickhouseTables {
    /// `(config field name, table name)` for every signal, in declaration order.
    pub fn all(&self) -> [(&'static str, &str); 7] {
        [
            ("logs", self.logs.as_str()),
            ("traces", self.traces.as_str()),
            ("metrics_gauge", self.metrics_gauge.as_str()),
            ("metrics_sum", self.metrics_sum.as_str()),
            ("metrics_summary", self.metrics_summary.as_str()),
            ("metrics_histogram", self.metrics_histogram.as_str()),
            ("metrics_exp_histogram", self.metrics_exp_histogram.as_str()),
        ]
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ClickhouseCreateSchema {
    /// Emits `ON CLUSTER <name>` in the generated DDL. Note this only spreads
    /// the *DDL* across the cluster — it does not shard writes.
    pub cluster_name: Option<String>,
    #[serde(default)]
    pub table_engine: ClickhouseTableEngine,
    /// Retention in seconds, applied as a `TTL` clause in the generated DDL.
    /// 0 disables it.
    #[serde(with = "crate::common::serde::duration_seconds", default)]
    #[schema(value_type = u64, default = 0)]
    pub ttl: Duration,
}

fn default_clickhouse_database() -> String {
    "default".to_string()
}

fn default_clickhouse_write_timeout() -> Duration {
    Duration::from_secs(30)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Clickhouse {
    /// HTTP interface URLs, e.g. `http://ch-1:8123` or `https://ch-1:8443`.
    /// Listing more than one turns on client-side routing (see `distribution`).
    /// With chproxy/HAProxy or a `Distributed` table in front, list the single
    /// front door and leave `distribution` at its default.
    pub endpoints: Vec<String>,
    #[serde(default)]
    pub distribution: ClickhouseDistribution,
    #[serde(default = "default_clickhouse_database")]
    pub database: String,
    pub username: Option<String>,
    pub password: Option<Sensitive>,
    pub tls: Option<super::Tls>,
    #[serde(default)]
    pub compression: ClickhouseCompression,
    /// Server-side [async inserts]. Lets Clickhouse coalesce small INSERTs into
    /// larger parts; keep it on unless `batch_size` is already >= a few thousand
    /// rows per flush.
    ///
    /// [async inserts]: https://clickhouse.com/docs/en/optimize/asynchronous-inserts
    #[serde(default)]
    pub async_insert: bool,
    /// Block until an async insert is flushed to disk. Costs throughput, buys
    /// durability. Ignored when `async_insert` is false.
    #[serde(default)]
    pub wait_for_async_insert: bool,
    /// Enables schema validation at insert time at the cost of throughput
    #[serde(default)]
    pub enable_schema_validation: bool,
    /// Extra Clickhouse session settings sent with every statement, e.g.
    /// `max_execution_time: "60"`. Escape hatch for anything not modelled above.
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    /// Inserts in flight to one endpoint at once, across the sink's workers.
    /// A batch sends one insert per table, so this counts inserts, not
    /// batches. Past it, a worker waits with its batch ready (the wait does
    /// not count toward `write_timeout`). 0 (default) means no limit.
    #[serde(default)]
    pub max_concurrent_inserts: usize,
    /// Use 0 to disable timeout
    #[serde(
        with = "crate::common::serde::duration_millis",
        default = "default_clickhouse_write_timeout"
    )]
    #[schema(value_type = u64, default = 30000)]
    pub write_timeout: Duration,
    /// Run `CREATE DATABASE` / `CREATE TABLE IF NOT EXISTS` on startup. Handy
    /// for a first run; turn it off in production so workers don't race to
    /// create tables and so schema changes stay under your control.
    ///
    /// Omit the block to leave schema management entirely to you. The DDL knobs
    /// inside it (`cluster_name`, `table_engine`, `ttl`) only apply at creation
    /// time — changing them later does not alter existing tables.
    #[serde(default)]
    pub create_schema: Option<ClickhouseCreateSchema>,
    #[serde(default)]
    pub tables: ClickhouseTables,
}

impl Clickhouse {
    pub fn validate(&mut self) -> Result<()> {
        if self.endpoints.is_empty() {
            anyhow::bail!("clickhouse sink endpoints cannot be empty");
        }
        for endpoint in &self.endpoints {
            if endpoint.is_empty() {
                anyhow::bail!("clickhouse sink endpoint cannot be empty");
            }
            if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
                anyhow::bail!(
                    "clickhouse sink endpoint '{endpoint}' must start with http:// or https:// (the HTTP interface, usually port 8123/8443)"
                );
            }
        }

        if self.database.trim().is_empty() {
            anyhow::bail!("clickhouse sink database cannot be empty");
        }

        for (field, table) in self.tables.all() {
            if table.trim().is_empty() {
                anyhow::bail!("clickhouse sink tables.{field} cannot be empty");
            }
        }

        if let Some(create_schema) = &self.create_schema {
            if let Some(cluster_name) = &create_schema.cluster_name
                && cluster_name.trim().is_empty()
            {
                anyhow::bail!(
                    "clickhouse sink cluster_name cannot be empty; omit the field instead"
                );
            }

            if create_schema.table_engine.name.trim().is_empty() {
                anyhow::bail!("clickhouse sink table_engine.name cannot be empty");
            }
        }

        if self.username.is_none() && self.password.is_some() {
            anyhow::bail!("clickhouse sink password requires a username");
        }
        if let Some(tls) = &self.tls {
            tls.validate()?;
            tls.validate_client()?;
        }

        Ok(())
    }

    pub fn sensitive_mut(&mut self) -> SensitiveRefs<'_> {
        let mut refs = Vec::new();
        if let Some(password) = &mut self.password {
            refs.push(("password", password));
        }
        if let Some(tls) = &mut self.tls {
            refs.extend(tls.sensitive_mut("tls.key_password"));
        }
        refs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sink(kind: &str, batch_size: usize, extra: &str) -> Sink {
        let yaml = format!(
            "id: 0192f8b0-0000-7000-8000-00000000000a\nname: sink-1\n\
             batch_size: {batch_size}\ntype: {kind}\nconfig:\n{extra}"
        );
        serde_yaml_ng::from_str(&yaml).expect("valid yaml")
    }

    fn kafka(batch_size: usize) -> Sink {
        sink(
            "kafka",
            batch_size,
            "  hosts: [localhost:9092]\n  topic: patrolli\n  format: json_otlp_batch\n  \
             security:\n    protocol: plaintext\n",
        )
    }

    /// A sink never builds a payload past `u16::MAX` rows, so only the
    /// database column bounds its batch.
    #[test]
    fn sink_batch_size_is_not_capped_at_u16() {
        for mut sink in [kafka(200_000), sink("file", 200_000, "  path: /dev/null\n")] {
            sink.validate().expect("a sink batch past u16");
            assert_eq!(sink.batch.batch_size, 200_000);
        }
        assert!(kafka(i32::MAX as usize).validate().is_ok());
        assert!(kafka(i32::MAX as usize + 1).validate().is_err());
    }

    #[test]
    fn producer_count_defaults_to_one_and_zero_is_normalized() {
        let mut default = kafka(1000);
        default.validate().expect("valid");
        let Config::Kafka(k) = &default.config else {
            unreachable!()
        };
        assert_eq!(k.producer_count, 1);

        let mut zeroed = kafka(1000);
        if let Config::Kafka(k) = &mut zeroed.config {
            k.producer_count = 0;
        }
        zeroed.validate().expect("valid");
        let Config::Kafka(k) = &zeroed.config else {
            unreachable!()
        };
        assert_eq!(k.producer_count, 1);
    }

    fn kafka_with(batch_timeout_ms: u64, write_timeout_seconds: u64) -> Sink {
        let mut sink = kafka(1000);
        sink.batch.batch_timeout = Duration::from_millis(batch_timeout_ms);
        if let Config::Kafka(k) = &mut sink.config {
            k.write_timeout_seconds = write_timeout_seconds;
        }
        sink
    }

    /// `batch_timeout` no longer reaches librdkafka, so it is not held to its
    /// linger rules; `write_timeout_seconds` is one attempt and 0 means the
    /// default, not "no timeout".
    #[test]
    fn kafka_write_timeout_is_one_attempt_and_independent_of_the_batch_timeout() {
        let mut long = kafka_with(60_000, 30);
        long.validate()
            .expect("a batch timeout past the write timeout is fine");

        let mut zeroed = kafka_with(1000, 0);
        zeroed.validate().expect("valid");
        let Config::Kafka(k) = &zeroed.config else {
            unreachable!()
        };
        assert_eq!(k.write_timeout_seconds, 30);
        assert_eq!(k.required_acks, KafkaRequiredAcks::One);
    }

    const KAFKA_CONFIG: &str = "  hosts: [localhost:9092]\n  topic: patrolli\n  \
                                format: json_otlp_batch\n  security:\n    protocol: plaintext\n";

    fn parse(yaml: &str) -> Result<Sink, serde_yaml_ng::Error> {
        serde_yaml_ng::from_str(&format!(
            "id: 0192f8b0-0000-7000-8000-00000000000a\nname: sink-1\n{yaml}"
        ))
    }

    /// Omitted or 0 is librdkafka's default; outside its range it fails here,
    /// not when the producer is built.
    #[test]
    fn kafka_max_message_size_defaults_to_librdkafkas_and_is_range_checked() {
        let max_message_size = |extra: &str| {
            let mut sink = parse(&format!("type: kafka\nconfig:\n{KAFKA_CONFIG}{extra}")).unwrap();
            sink.validate().map(|()| match sink.config {
                Config::Kafka(k) => k.max_message_size,
                _ => unreachable!(),
            })
        };
        assert_eq!(max_message_size("").unwrap(), 1_000_000);
        assert_eq!(
            max_message_size("  max_message_size: 0\n").unwrap(),
            1_000_000
        );
        assert_eq!(
            max_message_size("  max_message_size: 1000000000\n").unwrap(),
            1_000_000_000
        );
        assert!(max_message_size("  max_message_size: 999\n").is_err());
        assert!(max_message_size("  max_message_size: 1000000001\n").is_err());
    }

    /// An omitted batch takes the same default on every sink.
    #[test]
    fn omitted_batch_size_takes_the_default() {
        let mut kafka = parse(&format!("type: kafka\nconfig:\n{KAFKA_CONFIG}")).unwrap();
        kafka.validate().unwrap();
        assert_eq!(kafka.batch.batch_size, 1000);
        assert_eq!(kafka.batch.batch_timeout, Duration::from_secs(1));

        let mut file = parse("type: file\nconfig:\n  path: /dev/null\n").unwrap();
        file.validate().unwrap();
        assert_eq!(file.batch.batch_size, 1000);
        assert_eq!(file.batch.batch_timeout, Duration::from_secs(1));
    }

    /// The bug that happened twice: `batch_size` under `config:` was dropped
    /// without a word and the sink ran on the default.
    #[test]
    fn batch_size_under_config_is_rejected() {
        let err = parse(&format!(
            "type: kafka\nconfig:\n{KAFKA_CONFIG}  batch_size: 50000\n"
        ))
        .unwrap_err()
        .to_string();
        assert!(err.contains("unknown field `batch_size`"), "{err}");
    }

    #[test]
    fn unknown_keys_are_rejected_on_the_sink_and_in_its_channel() {
        for (yaml, field) in [
            (
                "workers: 2\ntype: file\nconfig:\n  path: /dev/null\n",
                "workers",
            ),
            (
                "channel:\n  type: memory\n  buffer_size: 10\ntype: file\nconfig:\n  path: /dev/null\n",
                "buffer_size",
            ),
            (
                "type: open_telemetry\nconfig:\n  destinations: [localhost:4317]\n  \
                 distribution:\n    strategy: sharded\n    key: host_id\n    seed: 1\n",
                "seed",
            ),
            (
                "retry:\n  max_retries: 3\ntype: file\nconfig:\n  path: /dev/null\n",
                "max_retries",
            ),
        ] {
            let err = parse(yaml).unwrap_err().to_string();
            assert!(
                err.contains(&format!("unknown field `{field}`")),
                "{yaml}: {err}"
            );
        }
    }

    /// Kafka only holds what it acked with `acks=all`, so anything less
    /// would delete a write-through key the broker can still lose.
    #[test]
    fn write_through_on_kafka_requires_all_acks() {
        const WRITE_THROUGH: &str = "channel:\n  type: disk\n  write_through: true\n";
        let mut one = parse(&format!(
            "{WRITE_THROUGH}type: kafka\nconfig:\n{KAFKA_CONFIG}"
        ))
        .unwrap();
        let err = one.validate().unwrap_err().to_string();
        assert!(err.contains("required_acks: all"), "{err}");

        let mut all = parse(&format!(
            "{WRITE_THROUGH}type: kafka\nconfig:\n{KAFKA_CONFIG}  required_acks: all\n"
        ))
        .unwrap();
        all.validate().unwrap();
        assert!(all.channel.write_through());

        let mut file = parse(&format!(
            "{WRITE_THROUGH}type: file\nconfig:\n  path: /dev/null\n"
        ))
        .unwrap();
        file.validate().unwrap();
    }

    /// An omitted `retry:`, or a zero in it, takes the defaults; 0 for
    /// `max_elapsed` is the default itself, retrying forever.
    #[test]
    fn retry_defaults_fill_omitted_and_zero_fields() {
        let mut omitted = parse("type: file\nconfig:\n  path: /dev/null\n").unwrap();
        omitted.validate().unwrap();
        assert_eq!(omitted.retry, crate::config::Retry::default());

        let mut zeroed = parse(
            "retry:\n  initial_interval: 0\n  max_interval: 0\n  multiplier: 0\n\
             type: file\nconfig:\n  path: /dev/null\n",
        )
        .unwrap();
        zeroed.validate().unwrap();
        assert_eq!(zeroed.retry, crate::config::Retry::default());
        assert!(zeroed.retry.max_elapsed.is_zero());

        let mut set = parse(
            "retry:\n  initial_interval: 500\n  max_elapsed: 600000\n\
             type: file\nconfig:\n  path: /dev/null\n",
        )
        .unwrap();
        set.validate().unwrap();
        assert_eq!(set.retry.initial_interval, Duration::from_millis(500));
        assert_eq!(set.retry.max_interval, Duration::from_secs(60));
        assert_eq!(set.retry.max_elapsed, Duration::from_secs(600));
    }

    #[test]
    fn retry_rejects_a_shrinking_or_inverted_backoff() {
        for (retry, expected) in [
            ("multiplier: 0.5", "multiplier must be at least 1"),
            ("multiplier: .nan", "multiplier must be at least 1"),
            (
                "initial_interval: 5000\n  max_interval: 1000",
                "shorter than initial_interval",
            ),
        ] {
            let mut sink = parse(&format!(
                "retry:\n  {retry}\ntype: file\nconfig:\n  path: /dev/null\n"
            ))
            .unwrap();
            let err = sink.validate().unwrap_err().to_string();
            assert!(err.contains(expected), "{retry}: {err}");
        }
    }
}
