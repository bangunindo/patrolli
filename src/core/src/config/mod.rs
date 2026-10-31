// SPDX-FileCopyrightText: 2025 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::common;
use anyhow::{Context, Result, bail};
use otel_arrow_dfe_pdata::otap::transform::LiteralValue;
use rustc_hash::FxHashSet;
use sensitive::{SecretError, Sensitive, SensitiveRefs};
use serde::{Deserialize, Serialize};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::OnceLock;
use strum_macros::{Display, EnumDiscriminants, EnumString};
use tokio::time::Duration;
use utoipa::ToSchema;
use uuid::Uuid;

pub mod cluster;
pub mod endpoint;
pub mod global;
pub mod host;
pub mod item;
pub mod listener;
pub mod lookup_table;
pub mod processor;
pub mod sensitive;
pub mod sink;
pub mod snmp;

pub static NODE_ID: OnceLock<Uuid> = OnceLock::new();

fn default_worker_count() -> usize {
    std::thread::available_parallelism()
        .unwrap_or(NonZeroUsize::new(1).unwrap())
        .get()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// no deny_unknown_fields: read from the whole config file only to pick the run
// mode, before the rest of it can be parsed
pub struct PartialConfig {
    pub global: global::Global,
}

/// The whole config file in backend mode, where everything but `global` lives
/// in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendFileConfig {
    pub global: global::Global,
}

impl PartialConfig {
    pub fn validate(&mut self) -> Result<()> {
        self.global.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MonitorConfig {
    #[serde(default = "default_monitor_workers")]
    pub worker_count: usize,
    #[serde(flatten)]
    pub batch: Batch,
}

impl MonitorConfig {
    pub fn validate(&mut self) -> Result<()> {
        self.batch.validate()?;
        if self.worker_count == 0 {
            self.worker_count = default_monitor_workers();
        }
        Ok(())
    }
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            worker_count: default_monitor_workers(),
            batch: Batch::default(),
        }
    }
}

fn default_monitor_workers() -> usize {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalFileConfig {
    #[serde(default)]
    pub global: global::Global,
    #[serde(default = "default_monitor_config")]
    pub monitor: MonitorConfig,
    #[serde(default)]
    pub lookup_tables: Vec<lookup_table::LookupTable>,
    #[serde(default)]
    pub endpoints: Vec<endpoint::Endpoint>,
    #[serde(default)]
    pub listeners: Vec<listener::Listener>,
    #[serde(default)]
    pub hosts: Vec<host::Host>,
    #[serde(default)]
    pub processors: Vec<processor::Processor>,
    #[serde(default)]
    pub items: Vec<item::Item>,
    #[serde(default)]
    pub sinks: Vec<sink::Sink>,
}

fn default_monitor_config() -> MonitorConfig {
    MonitorConfig {
        worker_count: default_monitor_workers(),
        batch: Batch::default(),
    }
}

impl LocalFileConfig {
    pub fn validate(&mut self) -> Result<()> {
        self.global.validate()?;
        self.monitor.validate()?;
        for (i, endpoint) in self.endpoints.iter_mut().enumerate() {
            endpoint
                .validate()
                .context(format!("endpoint {} is invalid", i))?;
        }
        for (i, listener) in self.listeners.iter_mut().enumerate() {
            listener
                .validate()
                .context(format!("listener {} is invalid", i))?;
        }
        for (i, host) in self.hosts.iter_mut().enumerate() {
            host.validate().context(format!("host {} is invalid", i))?;
        }
        for (i, item) in self.items.iter_mut().enumerate() {
            item.validate().context(format!("item {} is invalid", i))?;
        }
        for (i, sink) in self.sinks.iter_mut().enumerate() {
            sink.validate().context(format!("sink {} is invalid", i))?;
        }
        // Backend mode validates every processor as it reads it from the database
        // (`cluster::config_watcher::processor_query`); local mode has to do it here,
        // or a processor's `validate` never runs and its precomputed state — the
        // `TableTransform`s of the attribute transforms — stays `None`.
        for (i, processor) in self.processors.iter_mut().enumerate() {
            processor
                .validate()
                .context(format!("processor {} is invalid", i))?;
        }
        let mut lookup_ids = FxHashSet::default();
        for lookup_table in &mut self.lookup_tables {
            let name = lookup_table.name.clone();
            lookup_table
                .validate()
                .with_context(|| format!("lookup table {name}"))?;
            if !lookup_ids.insert(lookup_table.id) {
                bail!("duplicate lookup table id {}", lookup_table.id);
            }
        }
        Ok(())
    }
}

#[derive(Default, Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryChannel {
    #[serde(default)]
    pub non_blocking: bool,
    #[serde(default)]
    pub channel_buffer_size: usize,
}

impl MemoryChannel {
    pub fn validate(&mut self) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DiskChannel {
    #[serde(default)]
    pub non_blocking: bool,
    #[serde(default = "default_channel_max_memtable_size")]
    pub max_memtable_size: u64,
    /// Bytes of payloads (as stored, after compression) the topic holds
    /// before a send waits or, with `non_blocking`, fails; 0 is unbounded.
    /// Not the size of its files, which keep delivered payloads until a
    /// compaction drops them.
    #[serde(default = "default_channel_disk_max_size")]
    pub max_disk_size: u64,
    #[serde(default = "default_worker_count")]
    pub worker_count: usize,
    #[serde(default)]
    pub compression: OtapCompression,
    /// Stores payloads in the OTAP transport encoding (batches sorted, ids
    /// delta-encoded) before `compression`: smaller on disk, mostly for
    /// metrics, for a costlier encode and decode. Payloads come back with
    /// their attributes reordered. A topic still reads what it stored with
    /// the other setting.
    #[serde(default)]
    pub transport_optimized: bool,
    /// Slots between the disk bridge and the consumer. A payload that finds a
    /// free slot skips the disk; 0 spills whenever the consumer is busy.
    #[serde(default = "default_disk_channel_buffer_size")]
    pub channel_buffer_size: usize,
    /// Every payload is written to disk before it is handed on, and its key is
    /// deleted only once the sink acks it, so a crash or kill loses nothing
    /// that was accepted (at-least-once: expect duplicates after one). Only
    /// for a sink's channel; `channel_buffer_size` then bounds what is read
    /// ahead of the workers, not what skips the disk.
    #[serde(default)]
    pub write_through: bool,
}

impl Default for DiskChannel {
    fn default() -> Self {
        Self {
            non_blocking: false,
            max_memtable_size: default_channel_max_memtable_size(),
            max_disk_size: default_channel_disk_max_size(),
            worker_count: default_worker_count(),
            compression: OtapCompression::default(),
            transport_optimized: false,
            channel_buffer_size: default_disk_channel_buffer_size(),
            write_through: false,
        }
    }
}

impl DiskChannel {
    pub fn validate(&mut self) -> Result<()> {
        if self.worker_count == 0 {
            self.worker_count = default_worker_count();
        }
        Ok(())
    }
}

fn default_channel_disk_max_size() -> u64 {
    1024 * 1024 * 1024
}

fn default_disk_channel_buffer_size() -> usize {
    10
}

fn default_channel_max_memtable_size() -> u64 {
    64 * 1024 * 1024
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Channel {
    Memory(MemoryChannel),
    Disk(DiskChannel),
}

impl Default for Channel {
    fn default() -> Self {
        Self::Memory(MemoryChannel::default())
    }
}

impl Channel {
    pub fn validate(&mut self) -> Result<()> {
        match self {
            Channel::Memory(channel) => channel.validate(),
            Channel::Disk(channel) => channel.validate(),
        }
    }

    pub fn write_through(&self) -> bool {
        matches!(
            self,
            Channel::Disk(DiskChannel {
                write_through: true,
                ..
            })
        )
    }

    /// For a channel nobody acks (processor, forwarder): its keys would never
    /// be deleted.
    pub fn validate_no_write_through(&self) -> Result<()> {
        if self.write_through() {
            bail!("write_through is only supported on a sink's channel");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Batch {
    /// Omitted or 0 takes the default of 1000. At most 65535 on a listener or
    /// monitor, which numbers a payload's rows with a `u16`; a sink only counts
    /// whole payloads against it, so there it goes up to 2147483647.
    #[serde(default)]
    pub batch_size: usize,
    /// Milliseconds. Omitted or 0 takes the default of 1000.
    #[serde(default, with = "common::serde::duration_millis")]
    #[schema(value_type = u64, default = 0)]
    pub batch_timeout: Duration,
}

fn default_batch_size() -> usize {
    1000
}

fn default_batch_timeout() -> Duration {
    Duration::from_secs(1)
}

impl Batch {
    /// Fills in the defaults. Enough for a sink: it collects whole payloads
    /// until their items reach `batch_size`, and wherever it merges them,
    /// `PayloadBatch` caps each merged payload at `u16::MAX` items on its own.
    /// The bound is the database's `INTEGER` column.
    pub fn normalize(&mut self) -> Result<()> {
        if self.batch_size == 0 {
            self.batch_size = default_batch_size();
        };
        if self.batch_timeout.is_zero() {
            self.batch_timeout = default_batch_timeout();
        };
        if self.batch_size > i32::MAX as usize {
            return Err(anyhow::anyhow!("batch size cannot exceed 2147483647"));
        }
        Ok(())
    }

    /// For the stages that build a payload row by row (listeners, monitors,
    /// the forwarder): its row ids are `u16`, and they must flush before the
    /// builder runs out of them.
    pub fn validate(&mut self) -> Result<()> {
        self.normalize()?;
        if self.batch_size > u16::MAX as usize {
            return Err(anyhow::anyhow!("batch size cannot exceed 65535"));
        }
        Ok(())
    }
}

impl Default for Batch {
    fn default() -> Self {
        Self {
            batch_size: default_batch_size(),
            batch_timeout: default_batch_timeout(),
        }
    }
}

/// How a sink paces the retries of a batch its destination could not take:
/// `initial_interval`, growing by `multiplier` up to `max_interval`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Retry {
    /// Milliseconds before the first retry. Omitted or 0 takes the default of 1000.
    #[serde(with = "common::serde::duration_millis")]
    #[schema(value_type = u64, default = 1000)]
    pub initial_interval: Duration,
    /// Milliseconds; the longest wait between two retries. Omitted or 0 takes
    /// the default of 60000.
    #[serde(with = "common::serde::duration_millis")]
    #[schema(value_type = u64, default = 60000)]
    pub max_interval: Duration,
    /// Growth of the wait from one retry to the next, at least 1. Omitted or 0
    /// takes the default of 2.
    #[schema(default = 2.0)]
    pub multiplier: f64,
    /// Milliseconds a batch is retried for before it is dropped. 0 (the
    /// default) retries until it lands.
    #[serde(with = "common::serde::duration_millis")]
    #[schema(value_type = u64, default = 0)]
    pub max_elapsed: Duration,
}

fn default_retry_initial_interval() -> Duration {
    Duration::from_secs(1)
}

fn default_retry_max_interval() -> Duration {
    Duration::from_secs(60)
}

fn default_retry_multiplier() -> f64 {
    2.0
}

impl Retry {
    pub fn validate(&mut self) -> Result<()> {
        if self.initial_interval.is_zero() {
            self.initial_interval = default_retry_initial_interval();
        }
        if self.max_interval.is_zero() {
            self.max_interval = default_retry_max_interval();
        }
        if self.multiplier == 0.0 {
            self.multiplier = default_retry_multiplier();
        }
        if !(self.multiplier.is_finite() && self.multiplier >= 1.0) {
            bail!(
                "retry multiplier must be at least 1, got {}",
                self.multiplier
            );
        }
        if self.max_interval < self.initial_interval {
            bail!(
                "retry max_interval ({}ms) is shorter than initial_interval ({}ms)",
                self.max_interval.as_millis(),
                self.initial_interval.as_millis()
            );
        }
        Ok(())
    }
}

impl Default for Retry {
    fn default() -> Self {
        Self {
            initial_interval: default_retry_initial_interval(),
            max_interval: default_retry_max_interval(),
            multiplier: default_retry_multiplier(),
            max_elapsed: Duration::ZERO,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    #[default]
    JsonOtlpBatch,
    JsonNestedBatch,
    JsonOtlpDenormalized,
    JsonNestedDenormalized,
    ProtoOtlp,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Tls {
    #[serde(default)]
    pub insecure_skip_verify: bool,
    pub ca_cert_path: Option<String>,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
    pub key_password: Option<Sensitive>,
}

impl Tls {
    pub fn key_password(&self) -> Result<Option<&str>, SecretError> {
        self.key_password
            .as_ref()
            .map(Sensitive::expose)
            .transpose()
    }

    pub fn sensitive_mut(&mut self, kind: &'static str) -> SensitiveRefs<'_> {
        self.key_password
            .iter_mut()
            .map(|value| (kind, value))
            .collect()
    }

    fn validate_client(&self) -> Result<()> {
        if self.ca_cert_path.is_none() && !self.insecure_skip_verify {
            anyhow::bail!(
                "SSL Error: 'ca_cert_path' is missing. You must provide a CA certificate or set 'insecure_skip_verify: true'"
            );
        }
        Ok(())
    }

    fn validate_server(&self) -> Result<()> {
        if self.cert_path.is_none() {
            anyhow::bail!(
                "SSL Error: 'cert_path' is missing. You must provide a client certificate."
            );
        }
        if self.key_path.is_none() {
            anyhow::bail!("SSL Error: 'key_path' is missing. You must provide a private key.");
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        match (&self.cert_path, &self.key_path) {
            (Some(_), Some(_)) => {}
            (None, None) => {}
            (Some(_), None) => {
                anyhow::bail!(
                    "TLS Error: 'cert_path' provided but 'key_path' is missing. Both are required for SSL authentication."
                );
            }
            (None, Some(_)) => {
                anyhow::bail!(
                    "TLS Error: 'key_path' provided but 'cert_path' is missing. Both are required for SSL authentication."
                );
            }
        }

        if let Some(ca_cert_path) = &self.ca_cert_path
            && !Path::new(ca_cert_path).exists()
        {
            anyhow::bail!(
                "SSL Error: CA certificate file not found at: {}",
                ca_cert_path
            );
        }
        if let Some(cert_path) = &self.cert_path
            && !Path::new(cert_path).exists()
        {
            anyhow::bail!(
                "SSL Error: Client certificate file not found at: {}",
                cert_path
            );
        }
        if let Some(key_path) = &self.key_path
            && !Path::new(key_path).exists()
        {
            anyhow::bail!("SSL Error: Private key file not found at: {}", key_path);
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum OtapCompression {
    None = 0,
    Lz4 = 1,
    #[default]
    Zstd = 2,
}

impl TryFrom<u8> for OtapCompression {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(OtapCompression::None),
            1 => Ok(OtapCompression::Lz4),
            2 => Ok(OtapCompression::Zstd),
            _ => Err(anyhow::anyhow!("Invalid compression type: {}", value)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, EnumDiscriminants)]
#[serde(
    rename_all = "snake_case",
    tag = "type",
    content = "value",
    deny_unknown_fields
)]
#[strum_discriminants(
    name(OtelValueType),
    derive(Serialize, Deserialize, EnumString, Display, ToSchema),
    vis(pub),
    strum(serialize_all = "snake_case"),
    serde(rename_all = "snake_case")
)]
pub enum OtelValue {
    Bool(bool),
    Double(f64),
    Int(i64),
    Str(String),
}

impl From<OtelValue> for LiteralValue {
    fn from(value: OtelValue) -> Self {
        match value {
            OtelValue::Bool(b) => Self::Bool(b),
            OtelValue::Int(i) => Self::Int(i),
            OtelValue::Double(d) => Self::Double(d),
            OtelValue::Str(s) => Self::Str(s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stages that build payloads keep the `u16` row ceiling; a sink's
    /// `normalize` only has the database column's.
    #[test]
    fn batch_size_is_capped_at_u16_only_where_payloads_are_built() {
        let batch = |batch_size| Batch {
            batch_size,
            batch_timeout: Duration::ZERO,
        };
        assert!(batch(65_535).validate().is_ok());
        assert!(batch(65_536).validate().is_err());
        assert!(batch(65_536).normalize().is_ok());
        assert!(batch(i32::MAX as usize + 1).normalize().is_err());

        let mut zero = batch(0);
        zero.normalize().unwrap();
        assert_eq!(zero, Batch::default());
    }

    /// `Global::validate` creates the disk-channel directory, so point it at a
    /// temp dir the test can actually write to.
    fn config_with_lookup_names(entries: &[(&str, &str)]) -> (tempfile::TempDir, LocalFileConfig) {
        let dir = tempfile::tempdir().expect("temp dir");
        let tables: String = entries
            .iter()
            .map(|(id, name)| {
                format!(
                    "  - id: {id}\n    name: {name}\n    source:\n      type: processor\n      \
                     columns:\n        - name: host\n        - name: site\n    key:\n      \
                     type: hash\n      key_columns: [host]\n"
                )
            })
            .collect();
        let yaml = format!(
            "global:\n  disk_channel:\n    data_dir: {}\nlookup_tables:\n{tables}",
            dir.path().display()
        );
        let config = serde_yaml_ng::from_str(&yaml).expect("valid yaml");
        (dir, config)
    }

    const ID_A: &str = "0192f8b0-0000-7000-8000-00000000000a";
    const ID_B: &str = "0192f8b0-0000-7000-8000-00000000000b";

    #[test]
    fn distinct_lookup_tables_validate() {
        let (_dir, mut config) = config_with_lookup_names(&[(ID_A, "alpha"), (ID_B, "beta")]);
        config.validate().unwrap();
        assert_eq!(config.lookup_tables.len(), 2);
    }

    #[test]
    fn duplicate_lookup_table_names_are_allowed() {
        // the id is the identity; the name is only a label, as for every other
        // entity in this config
        let (_dir, mut config) = config_with_lookup_names(&[(ID_A, "alpha"), (ID_B, "alpha")]);
        config.validate().unwrap();
        assert_eq!(config.lookup_tables.len(), 2);
    }

    #[test]
    fn duplicate_lookup_table_ids_are_rejected() {
        let (_dir, mut config) = config_with_lookup_names(&[(ID_A, "alpha"), (ID_A, "beta")]);
        let err = format!("{:#}", config.validate().unwrap_err());
        assert!(err.contains("duplicate lookup table id"), "{err}");
    }

    /// Symbolic OIDs resolve against the MIB, which unit tests do not load,
    /// so swap every `oid:` for a numeric one. The keys are what is checked.
    fn numeric_oids(value: &mut serde_yaml_ng::Value) {
        match value {
            serde_yaml_ng::Value::Mapping(map) => {
                for (key, value) in map.iter_mut() {
                    if key.as_str() == Some("oid") && value.is_string() {
                        *value = "1.3.6.1.2.1.1.3.0".into();
                    } else {
                        numeric_oids(value);
                    }
                }
            }
            serde_yaml_ng::Value::Sequence(seq) => seq.iter_mut().for_each(numeric_oids),
            _ => {}
        }
    }

    /// Every uncommented key in the shipped configs must be one the structs
    /// know, now that unknown keys are errors. Deserialize only: `validate`
    /// would check TLS files and create directories.
    #[test]
    fn the_shipped_configs_deserialize() {
        let name = "config-full.yaml";
        let yaml = include_str!("../../config-full.yaml");
        let mut value: serde_yaml_ng::Value = serde_yaml_ng::from_str(yaml).unwrap();
        numeric_oids(&mut value);
        if let Err(e) = serde_yaml_ng::from_value::<LocalFileConfig>(value) {
            panic!("{name} is stale: {e}");
        }
        if let Err(e) =
            serde_yaml_ng::from_str::<BackendFileConfig>(include_str!("../../config-backend.yaml"))
        {
            panic!("config-backend.yaml is stale: {e}");
        }
    }

    #[test]
    fn backend_file_config_rejects_anything_but_global() {
        let err = serde_yaml_ng::from_str::<BackendFileConfig>("global: {}\nlisteners: []\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown field `listeners`"), "{err}");
        // the mode is picked before this check, so it must not care
        serde_yaml_ng::from_str::<PartialConfig>("global: {}\nlisteners: []\n").unwrap();
    }

    #[test]
    fn unknown_keys_are_rejected_in_flattened_and_tagged_blocks() {
        for (yaml, field) in [
            // global.processor flattens an internally tagged enum, so the
            // channel variant is what rejects it
            (
                "global:\n  processor:\n    type: memory\n    buffer: 1\n",
                "buffer",
            ),
            (
                "global:\n  processor:\n    type: disk\n    buffer: 1\n",
                "buffer",
            ),
            ("monitor:\n  workers: 10\n", "workers"),
            ("global:\n  monitor: {}\n", "monitor"),
        ] {
            let err = serde_yaml_ng::from_str::<LocalFileConfig>(yaml)
                .unwrap_err()
                .to_string();
            assert!(
                err.contains(&format!("unknown field `{field}`")),
                "{yaml}: {err}"
            );
        }
    }

    #[test]
    fn an_invalid_lookup_table_is_reported_by_name() {
        let (_dir, mut config) = config_with_lookup_names(&[(ID_A, "alpha")]);
        // a key column that is not one of the declared columns
        if let lookup_table::Key::Hash(hash) = &mut config.lookup_tables[0].key {
            hash.key_columns = vec!["ghost".into()];
        }
        let err = format!("{:#}", config.validate().unwrap_err());
        assert!(err.contains("lookup table alpha"), "{err}");
        assert!(err.contains("not a column"), "{err}");
    }
}
