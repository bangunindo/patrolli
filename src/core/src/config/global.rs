// SPDX-FileCopyrightText: 2025 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::Channel;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tokio::time::Duration;
use tracing_subscriber::filter::LevelFilter;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Logger {
    #[serde(default)]
    pub level: Level,
    #[serde(default = "default_output")]
    pub output: String,
    #[serde(default)]
    pub format: Format,
    #[serde(default)]
    pub print_trace: bool,
    #[serde(default)]
    pub modules: Vec<LogModule>,
    #[serde(default)]
    pub disable_color: bool,
    #[serde(default)]
    pub disable_throttle: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LogModule {
    pub name: String,
    pub level: Level,
}

impl Logger {
    pub fn validate(&self) -> Result<()> {
        Ok(())
    }
}

impl Default for Logger {
    fn default() -> Self {
        Logger {
            level: Level::default(),
            output: default_output(),
            format: Format::default(),
            print_trace: false,
            disable_color: false,
            disable_throttle: false,
            modules: vec![],
        }
    }
}

fn default_output() -> String {
    "stderr".to_string()
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = LogLevel)]
pub enum Level {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
    Off,
}

impl From<Level> for LevelFilter {
    fn from(level: Level) -> LevelFilter {
        match level {
            Level::Trace => LevelFilter::TRACE,
            Level::Debug => LevelFilter::DEBUG,
            Level::Info => LevelFilter::INFO,
            Level::Warn => LevelFilter::WARN,
            Level::Error => LevelFilter::ERROR,
            Level::Off => LevelFilter::OFF,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    #[default]
    Text,
    Json,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
// no deny_unknown_fields: a flattened internally tagged enum borrows every key
// instead of claiming its own, so the parent would reject all of them. The
// channel variants deny unknown fields instead.
pub struct ProcessorConfig {
    #[serde(flatten, default)]
    pub channel: Channel,
}

impl ProcessorConfig {
    pub fn validate(&mut self) -> Result<()> {
        self.channel.validate()?;
        self.channel.validate_no_write_through()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MibConfig {
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub dirs: Vec<String>,
    #[serde(default)]
    pub disable_system_mib: bool,
}

impl MibConfig {
    pub fn validate(&mut self) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiskChannel {
    #[serde(default = "default_disk_channel_data_dir")]
    pub data_dir: String,
    #[serde(default = "default_disk_max_journal_size")]
    pub max_journal_size: u64,
    #[serde(default = "default_cache_size")]
    pub cache_size: u64,
}

impl Default for DiskChannel {
    fn default() -> Self {
        Self {
            data_dir: default_disk_channel_data_dir(),
            max_journal_size: default_disk_max_journal_size(),
            cache_size: default_cache_size(),
        }
    }
}

fn default_disk_channel_data_dir() -> String {
    "/var/lib/patrolli/channel/".to_string()
}

fn default_disk_max_journal_size() -> u64 {
    512 * 1024 * 1024
}

fn default_cache_size() -> u64 {
    128 * 1024 * 1024
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prometheus {
    #[serde(default = "default_prometheus_address")]
    pub address: SocketAddr,
    #[serde(default)]
    pub bearer_token: Option<String>,
    #[serde(default)]
    pub tls: Option<super::Tls>,
}

impl fmt::Debug for Prometheus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Prometheus")
            .field("address", &self.address)
            .field(
                "bearer_token",
                &self.bearer_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("tls", &self.tls)
            .finish()
    }
}

impl Default for Prometheus {
    fn default() -> Self {
        Self {
            address: default_prometheus_address(),
            bearer_token: None,
            tls: None,
        }
    }
}

impl Prometheus {
    pub fn validate(&self) -> Result<()> {
        if self.address.port() == 0 {
            anyhow::bail!("Prometheus address must use a non-zero port");
        }

        if let Some(token) = &self.bearer_token {
            if token.is_empty() {
                anyhow::bail!("Prometheus bearer_token must not be empty");
            }
            http::HeaderValue::from_str(&format!("Bearer {token}"))
                .context("Prometheus bearer_token contains invalid HTTP header characters")?;
        }

        if let Some(tls) = &self.tls {
            tls.validate()?;
            tls.validate_server()?;
        }

        Ok(())
    }
}

fn default_prometheus_address() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9464)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Global {
    #[serde(default)]
    pub disk_channel: DiskChannel,
    #[serde(
        default = "default_internal_metrics_interval",
        with = "crate::common::serde::duration_millis"
    )]
    pub internal_metrics_interval: Duration,
    /// How long a shutdown may take from the signal. Every teardown step is
    /// cut short to end by then; past it plus a short grace the process
    /// exits regardless. 0 is the default.
    #[serde(
        default = "default_shutdown_timeout",
        with = "crate::common::serde::duration_millis"
    )]
    pub shutdown_timeout: Duration,
    #[serde(default)]
    pub logger: Logger,
    #[serde(default)]
    pub processor: ProcessorConfig,
    #[serde(default)]
    pub mib: MibConfig,
    pub prometheus: Option<Prometheus>,
    pub backend: Option<super::cluster::Backend>,
}

impl Default for Global {
    fn default() -> Self {
        Self {
            disk_channel: DiskChannel::default(),
            internal_metrics_interval: default_internal_metrics_interval(),
            shutdown_timeout: default_shutdown_timeout(),
            logger: Logger::default(),
            processor: ProcessorConfig::default(),
            mib: MibConfig::default(),
            prometheus: None,
            backend: None,
        }
    }
}

impl Global {
    pub fn validate(&mut self) -> Result<()> {
        std::fs::create_dir_all(&self.disk_channel.data_dir)
            .context("Failed to create channel disk data directory")?;
        if self.internal_metrics_interval.is_zero() {
            self.internal_metrics_interval = default_internal_metrics_interval();
        }
        if self.shutdown_timeout.is_zero() {
            self.shutdown_timeout = default_shutdown_timeout();
        }
        self.logger.validate()?;
        self.processor.validate()?;
        self.mib.validate()?;
        if let Some(prometheus) = &self.prometheus {
            prometheus.validate()?;
        }
        if let Some(backend) = &mut self.backend {
            backend.validate()?;
        }
        Ok(())
    }
}

fn default_internal_metrics_interval() -> Duration {
    Duration::from_secs(1)
}

/// Under Kubernetes' default 30s `terminationGracePeriodSeconds`, with room
/// for the watchdog's grace before the SIGKILL.
fn default_shutdown_timeout() -> Duration {
    Duration::from_secs(20)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing acks the processor's input, so its keys would never go.
    #[test]
    fn the_processor_channel_refuses_write_through() {
        let mut config: ProcessorConfig =
            serde_yaml_ng::from_str("type: disk\nwrite_through: true\n").unwrap();
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("write_through"), "{err}");
    }

    #[test]
    fn prometheus_block_defaults_to_loopback() {
        let config: Prometheus = serde_yaml_ng::from_str("{}").unwrap();
        assert_eq!(config.address, "127.0.0.1:9464".parse().unwrap());
        assert!(config.bearer_token.is_none());
        assert!(config.tls.is_none());
        config.validate().unwrap();
    }

    #[test]
    fn prometheus_deserializes_under_global() {
        let config: crate::config::PartialConfig =
            serde_yaml_ng::from_str("global:\n  prometheus: {}\n").unwrap();
        assert_eq!(
            config.global.prometheus.unwrap().address,
            "127.0.0.1:9464".parse().unwrap()
        );
    }

    #[test]
    fn prometheus_secrets_are_redacted_from_debug_output() {
        let config = Prometheus {
            bearer_token: Some("bearer-secret".to_string()),
            tls: Some(super::super::Tls {
                cert_path: Some("/cert.pem".to_string()),
                key_path: Some("/key.pem".to_string()),
                key_password: Some(super::super::sensitive::Sensitive::Plain(
                    "key-secret".to_string(),
                )),
                ..Default::default()
            }),
            ..Default::default()
        };

        let debug = format!("{config:?}");
        assert!(!debug.contains("bearer-secret"));
        assert!(!debug.contains("key-secret"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn prometheus_rejects_empty_or_invalid_bearer_tokens() {
        let mut config = Prometheus {
            bearer_token: Some(String::new()),
            ..Default::default()
        };
        assert!(config.validate().is_err());

        config.bearer_token = Some("line\nbreak".to_string());
        assert!(config.validate().is_err());
    }
}
