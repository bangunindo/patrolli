// SPDX-FileCopyrightText: 2025 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::global::{Level, LogModule};
use super::sensitive::{SensitiveFields, SensitiveRefs};
use super::{Batch, snmp::SecurityModel};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use strum_macros::{Display, EnumDiscriminants, EnumString, IntoStaticStr};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, EnumDiscriminants, ToSchema)]
#[serde(rename_all = "snake_case", tag = "type", content = "config")]
#[schema(as = ListenerConfig)]
#[strum_discriminants(
    name(ListenerKind),
    derive(Serialize, Deserialize, EnumString, Display, ToSchema, IntoStaticStr),
    vis(pub),
    strum(serialize_all = "snake_case"),
    serde(rename_all = "snake_case")
)]
pub enum Config {
    Dummy(Dummy),
    Netflow(Netflow),
    SnmpTrap(SnmpTrap),
    Syslog(Box<Syslog>),
    OpenTelemetry(OpenTelemetry),
    InternalLogs(InternalLogs),
    InternalMetrics(InternalMetrics),
}

impl Config {
    pub fn validate(&mut self) -> Result<()> {
        match self {
            Self::Dummy(dummy) => dummy.validate()?,
            Self::Netflow(netflow) => netflow.validate()?,
            Self::SnmpTrap(snmp_trap) => snmp_trap.validate()?,
            Self::Syslog(syslog) => syslog.validate()?,
            Self::OpenTelemetry(open_telemetry) => open_telemetry.validate()?,
            Self::InternalLogs(internal_logs) => internal_logs.validate()?,
            Self::InternalMetrics(internal_metrics) => internal_metrics.validate()?,
        };
        Ok(())
    }

    pub fn to_json_value(&self) -> Result<serde_json::Value> {
        match self {
            Self::Dummy(dummy) => Ok(serde_json::to_value(dummy)?),
            Self::Netflow(netflow) => Ok(serde_json::to_value(netflow)?),
            Self::SnmpTrap(snmp_trap) => Ok(serde_json::to_value(snmp_trap)?),
            Self::Syslog(syslog) => Ok(serde_json::to_value(syslog)?),
            Self::OpenTelemetry(open_telemetry) => Ok(serde_json::to_value(open_telemetry)?),
            Self::InternalLogs(internal_logs) => Ok(serde_json::to_value(internal_logs)?),
            Self::InternalMetrics(internal_metrics) => Ok(serde_json::to_value(internal_metrics)?),
        }
    }
}

impl SensitiveFields for Config {
    fn sensitive_mut(&mut self) -> SensitiveRefs<'_> {
        match self {
            Self::SnmpTrap(snmp_trap) => snmp_trap
                .v3
                .as_mut()
                .map_or_else(Vec::new, V3::sensitive_mut),
            Self::Syslog(syslog) => syslog
                .listener
                .tls
                .as_mut()
                .map_or_else(Vec::new, |tls| tls.tls.sensitive_mut("tls.key_password")),
            Self::OpenTelemetry(open_telemetry) => open_telemetry
                .tls
                .as_mut()
                .map_or_else(Vec::new, |tls| tls.sensitive_mut("tls.key_password")),
            Self::Dummy(_)
            | Self::Netflow(_)
            | Self::InternalLogs(_)
            | Self::InternalMetrics(_) => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Listener {
    pub id: Uuid,
    pub name: SmolStr,
    #[serde(default = "super::default_worker_count")]
    pub worker_count: usize,
    /// Ignored by the listener types that do not batch (`dummy`,
    /// `internal_metrics`).
    #[serde(flatten, default)]
    pub batch: Batch,
    #[serde(flatten)]
    pub config: Config,
    #[serde(default)]
    // if set to true, still honors processor defined for the listener
    // even if the instance does not monitor the item's host
    pub disable_cluster_forwarding: bool,
}

impl Listener {
    pub fn new(
        id: Uuid,
        name: SmolStr,
        worker_count: usize,
        batch: Batch,
        config: Config,
        disable_cluster_forwarding: bool,
    ) -> Self {
        Self {
            id,
            name,
            worker_count,
            batch,
            config,
            disable_cluster_forwarding,
        }
    }

    pub fn validate(&mut self) -> Result<()> {
        if self.worker_count == 0 {
            self.worker_count = super::default_worker_count();
        }
        self.batch.validate()?;
        self.config.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(default, deny_unknown_fields)]
#[schema(as = DummyListener)]
pub struct Dummy {
    #[serde(default = "default_speed")]
    pub speed: u64,
    #[serde(default = "default_messages_per_batch")]
    pub messages_per_batch: usize,
}

impl Dummy {
    pub fn validate(&mut self) -> Result<()> {
        if self.speed == 0 {
            self.speed = default_speed();
        }
        if self.messages_per_batch == 0 {
            self.messages_per_batch = default_messages_per_batch();
        }
        Ok(())
    }
}

fn default_speed() -> u64 {
    5
}

fn default_messages_per_batch() -> usize {
    100
}

impl Default for Dummy {
    fn default() -> Self {
        Self {
            speed: default_speed(),
            messages_per_batch: default_messages_per_batch(),
        }
    }
}

fn default_max_packet_size() -> usize {
    4096
}

fn default_buffer_cap() -> usize {
    100
}

fn default_recv_buffer_size() -> usize {
    32 << 20
}

fn default_netflow_address() -> SocketAddr {
    "0.0.0.0:2055".parse().unwrap()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UdpListener {
    #[schema(value_type = String)]
    pub address: SocketAddr,
    #[serde(default = "default_max_packet_size")]
    pub max_packet_size: usize,
    #[serde(default = "default_buffer_cap")]
    pub buffer_cap: usize,
    /// Kernel receive buffer (`SO_RCVBUF`) requested per socket, in bytes.
    /// Granted in full with `CAP_NET_ADMIN`, otherwise capped at
    /// `net.core.rmem_max`. The kernel doubles it for bookkeeping. 0 uses the
    /// default (32 MiB).
    #[serde(default = "default_recv_buffer_size")]
    pub recv_buffer_size: usize,
    /// Microseconds the reader keeps polling an empty socket before it
    /// sleeps. Keeps the core out of deep idle states between back-to-back
    /// datagrams, which only matters with a small `recv_buffer_size` and one
    /// socket near 1M datagrams/s; every polled gap costs CPU, and a busy
    /// socket pins a core. 0 (the default) sleeps as soon as the socket is
    /// empty. At most 1000.
    #[serde(default)]
    pub spin_before_block_us: u64,
    #[serde(default = "default_one")]
    pub worker_count: usize,
    #[serde(default)]
    #[schema(value_type = Vec<String>)]
    pub ip_whitelist: Vec<IpAddr>,
    #[serde(default)]
    #[schema(value_type = Vec<String>)]
    pub ip_blacklist: Vec<IpAddr>,
}

fn default_one() -> usize {
    1
}

impl UdpListener {
    pub fn validate(&mut self) -> Result<()> {
        anyhow::ensure!(self.address.port() > 0, "invalid address' port");
        anyhow::ensure!(
            self.ip_whitelist.is_empty() || self.ip_blacklist.is_empty(),
            "ip whitelist and blacklist cannot be used together"
        );
        self.ip_whitelist.sort_unstable();
        self.ip_blacklist.sort_unstable();
        if self.max_packet_size == 0 {
            self.max_packet_size = default_max_packet_size();
        }
        if self.buffer_cap == 0 {
            self.buffer_cap = default_buffer_cap();
        }
        if self.recv_buffer_size == 0 {
            self.recv_buffer_size = default_recv_buffer_size();
        }
        anyhow::ensure!(
            self.recv_buffer_size <= i32::MAX as usize,
            "recv_buffer_size must be at most {} bytes",
            i32::MAX
        );
        anyhow::ensure!(
            self.spin_before_block_us <= 1000,
            "spin_before_block_us must be at most 1000"
        );
        if self.worker_count == 0 {
            self.worker_count = default_one();
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TcpListener {
    #[schema(value_type = String)]
    pub address: SocketAddr,
    #[serde(default)]
    #[schema(value_type = Vec<String>)]
    pub ip_whitelist: Vec<IpAddr>,
    #[serde(default)]
    #[schema(value_type = Vec<String>)]
    pub ip_blacklist: Vec<IpAddr>,
}

impl TcpListener {
    pub fn validate(&mut self) -> Result<()> {
        anyhow::ensure!(self.address.port() > 0, "invalid address' port");
        anyhow::ensure!(
            self.ip_whitelist.is_empty() || self.ip_blacklist.is_empty(),
            "ip whitelist and blacklist cannot be used together"
        );
        self.ip_whitelist.sort_unstable();
        self.ip_blacklist.sort_unstable();
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TlsListener {
    #[schema(value_type = String)]
    pub address: SocketAddr,
    pub tls: super::Tls,
    #[serde(default)]
    #[schema(value_type = Vec<String>)]
    pub ip_whitelist: Vec<IpAddr>,
    #[serde(default)]
    #[schema(value_type = Vec<String>)]
    pub ip_blacklist: Vec<IpAddr>,
}

impl TlsListener {
    pub fn validate(&mut self) -> Result<()> {
        anyhow::ensure!(self.address.port() > 0, "invalid address' port");
        anyhow::ensure!(
            self.ip_whitelist.is_empty() || self.ip_blacklist.is_empty(),
            "ip whitelist and blacklist cannot be used together"
        );
        self.ip_whitelist.sort_unstable();
        self.ip_blacklist.sort_unstable();
        self.tls.validate()?;
        self.tls.validate_server()?;
        if !(self.tls.cert_path.is_some() && self.tls.key_path.is_some()) {
            return Err(anyhow::anyhow!("cert_path and key_path must be present"));
        }
        Ok(())
    }
}

fn default_netflow_udp_listener() -> UdpListener {
    UdpListener {
        address: default_netflow_address(),
        max_packet_size: default_max_packet_size(),
        buffer_cap: default_buffer_cap(),
        recv_buffer_size: default_recv_buffer_size(),
        spin_before_block_us: 0,
        worker_count: default_one(),
        ip_whitelist: Vec::new(),
        ip_blacklist: Vec::new(),
    }
}

fn default_netflow_max_sources() -> usize {
    netflow_parser::DEFAULT_MAX_SOURCES
}

fn default_netflow_prune_interval() -> Duration {
    Duration::from_secs(5 * 60)
}

fn default_netflow_prune_idle_after() -> Duration {
    Duration::from_secs(60 * 60)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Netflow {
    #[serde(default = "default_netflow_udp_listener")]
    pub listener: UdpListener,
    /// Exporter sources tracked for template state, per parser shard. The
    /// parser is sharded by source IP, so the process-wide ceiling is this
    /// times the shard count.
    #[serde(default = "default_netflow_max_sources")]
    pub max_sources: usize,
    /// How often idle sources are swept out of the parser caches.
    #[serde(
        with = "crate::common::serde::duration_seconds",
        default = "default_netflow_prune_interval"
    )]
    #[schema(value_type = u64)]
    pub prune_interval: Duration,
    /// A source unseen for this long is dropped by the sweep.
    #[serde(
        with = "crate::common::serde::duration_seconds",
        default = "default_netflow_prune_idle_after"
    )]
    #[schema(value_type = u64)]
    pub prune_idle_after: Duration,
}

impl Netflow {
    pub fn validate(&mut self) -> Result<()> {
        self.listener.validate()?;
        if self.max_sources == 0 {
            self.max_sources = default_netflow_max_sources();
        }
        if self.prune_interval.is_zero() {
            self.prune_interval = default_netflow_prune_interval();
        }
        if self.prune_idle_after.is_zero() {
            self.prune_idle_after = default_netflow_prune_idle_after();
        }
        Ok(())
    }
}

fn default_snmp_trap_address() -> SocketAddr {
    "0.0.0.0:162".parse().unwrap()
}

fn default_snmp_trap_udp_listener() -> UdpListener {
    UdpListener {
        address: default_snmp_trap_address(),
        max_packet_size: default_max_packet_size(),
        buffer_cap: default_buffer_cap(),
        recv_buffer_size: default_recv_buffer_size(),
        spin_before_block_us: 0,
        worker_count: default_one(),
        ip_whitelist: Vec::new(),
        ip_blacklist: Vec::new(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct V1V2C {
    pub communities: Vec<String>,
}

impl V1V2C {
    pub fn validate(&mut self) -> Result<()> {
        anyhow::ensure!(
            !self.communities.is_empty(),
            "at least one community must be configured"
        );
        for community in &self.communities {
            anyhow::ensure!(
                !community.is_empty() && community.len() <= 64,
                "community must be between 1 and 64 bytes long"
            );
        }
        self.communities.sort_unstable();
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct V3 {
    pub security_models: Vec<SecurityModel>,
}

impl V3 {
    pub fn validate(&mut self) -> Result<()> {
        anyhow::ensure!(
            !self.security_models.is_empty(),
            "at least one security model must be configured"
        );
        for security_model in &self.security_models {
            security_model.validate()?;
        }
        self.security_models.sort_unstable_by(|a, b| {
            let a_key = match a {
                SecurityModel::Usm(usm) => &usm.username,
            };
            let b_key = match b {
                SecurityModel::Usm(usm) => &usm.username,
            };
            a_key.cmp(b_key)
        });
        // the trap receiver keeps one user per username, so a second user with
        // the same name silently replaces the first:
        // https://github.com/lukeod/async-snmp/blob/170eacabe83015f09f6e18e4e5cd26bdf2296d0b/src/notification/mod.rs#L881
        // the sort above puts equal names next to each other.
        for pair in self.security_models.windows(2) {
            let (SecurityModel::Usm(a), SecurityModel::Usm(b)) = (&pair[0], &pair[1]);
            anyhow::ensure!(
                a.username != b.username,
                "security model username {:?} is configured more than once",
                a.username
            );
        }
        Ok(())
    }

    pub fn sensitive_mut(&mut self) -> SensitiveRefs<'_> {
        self.security_models
            .iter_mut()
            .flat_map(SecurityModel::sensitive_mut)
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SnmpTrap {
    #[serde(default = "default_snmp_trap_udp_listener")]
    pub listener: UdpListener,
    #[serde(default)]
    pub use_mib: bool,
    pub v1: Option<V1V2C>,
    pub v2c: Option<V1V2C>,
    pub v3: Option<V3>,
}

impl SnmpTrap {
    pub fn validate(&mut self) -> Result<()> {
        self.listener.validate()?;
        anyhow::ensure!(
            !(self.v1.is_none() && self.v2c.is_none() && self.v3.is_none()),
            "either v1, v2c or v3 must be set"
        );
        if let Some(v1) = &mut self.v1 {
            v1.validate()?;
        }
        if let Some(v2c) = &mut self.v2c {
            v2c.validate()?;
        }
        if let Some(v3) = &mut self.v3 {
            v3.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SyslogListener {
    pub udp: Option<UdpListener>,
    pub tcp: Option<TcpListener>,
    pub tls: Option<TlsListener>,
    #[serde(
        with = "crate::common::serde::duration_seconds",
        default = "default_syslog_idle_timeout"
    )]
    #[schema(value_type = u64)]
    pub idle_timeout: Duration,
}

fn default_syslog_idle_timeout() -> Duration {
    Duration::from_mins(10)
}

impl SyslogListener {
    pub fn validate(&mut self) -> Result<()> {
        if self.udp.is_none() && self.tcp.is_none() && self.tls.is_none() {
            self.udp = Some(default_syslog_udp_listener());
        }
        if let Some(tcp) = &self.tcp
            && let Some(tls) = &self.tls
            && tcp.address == tls.address
        {
            return Err(anyhow::anyhow!(
                "tcp and tls listeners cannot share the same address"
            ));
        }
        if let Some(udp) = &mut self.udp {
            udp.validate()?;
        }
        if let Some(tcp) = &mut self.tcp {
            tcp.validate()?;
        }
        if let Some(tls) = &mut self.tls {
            tls.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Syslog {
    #[serde(default = "default_syslog_listener")]
    pub listener: SyslogListener,
    #[serde(default)]
    pub cise: Cise,
    #[serde(default)]
    pub cef: Cef,
    #[serde(default)]
    pub cisco: Cisco,
}

/// CEF (Common Event Format), inside syslog or raw.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Cef {
    /// Off, CEF records are plain syslog: no `cef` attribute, and their `rt`
    /// and severity are not read. Raw CEF lines are still framed over TCP.
    pub enabled: bool,
}

impl Default for Cef {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Cisco IOS, IOS XR, NX-OS, ASA and FTD `%FACILITY-SEVERITY-MNEMONIC:` messages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Cisco {
    /// Off, they are plain syslog: no `cisco` attribute, and the device's own
    /// hostname, time and severity are not read.
    pub enabled: bool,
}

impl Default for Cisco {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Cisco ISE / ACS parsing, and the reassembly of the events ISE splits into
/// segments.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Cise {
    /// Off, ISE/ACS messages are plain syslog: one record per segment, no
    /// `cise` attribute.
    pub enabled: bool,
    /// How long, in milliseconds, an event's first segment waits for the
    /// rest before what arrived is sent as an incomplete event.
    #[serde(with = "crate::common::serde::duration_millis")]
    #[schema(value_type = u64, default = 10000)]
    pub reassembly_timeout: Duration,
    /// Events held at once; past it the oldest is sent incomplete.
    pub max_pending: usize,
    /// Bytes held at once; past it the oldest event is sent incomplete.
    pub max_pending_bytes: usize,
    /// Events split into more segments are not reassembled: each segment is
    /// its own record. `1` turns reassembly off.
    pub max_segments: u32,
}

impl Default for Cise {
    fn default() -> Self {
        Self {
            enabled: true,
            reassembly_timeout: Duration::from_millis(10_000),
            max_pending: 16 * 1024,
            max_pending_bytes: 64 * 1024 * 1024,
            max_segments: 256,
        }
    }
}

impl Cise {
    pub fn validate(&mut self) {
        let default = Self::default();
        if self.reassembly_timeout.is_zero() {
            self.reassembly_timeout = default.reassembly_timeout;
        }
        if self.max_pending == 0 {
            self.max_pending = default.max_pending;
        }
        if self.max_pending_bytes == 0 {
            self.max_pending_bytes = default.max_pending_bytes;
        }
        if self.max_segments == 0 {
            self.max_segments = default.max_segments;
        }
    }
}

fn default_syslog_udp_listener() -> UdpListener {
    UdpListener {
        address: default_syslog_address(),
        max_packet_size: 65536,
        buffer_cap: default_buffer_cap(),
        recv_buffer_size: default_recv_buffer_size(),
        spin_before_block_us: 0,
        worker_count: default_one(),
        ip_whitelist: Vec::new(),
        ip_blacklist: Vec::new(),
    }
}

fn default_syslog_listener() -> SyslogListener {
    SyslogListener {
        udp: Some(default_syslog_udp_listener()),
        tcp: None,
        tls: None,
        idle_timeout: default_syslog_idle_timeout(),
    }
}

fn default_syslog_address() -> SocketAddr {
    "0.0.0.0:514".parse().unwrap()
}

impl Syslog {
    pub fn validate(&mut self) -> Result<()> {
        self.listener.validate()?;
        self.cise.validate();
        Ok(())
    }
}

fn default_otap_listener() -> TcpListener {
    TcpListener {
        address: "0.0.0.0:4317".parse().unwrap(),
        ip_whitelist: vec![],
        ip_blacklist: vec![],
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ListenerOtelMode {
    #[default]
    Otap,
    Otlp,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(default, deny_unknown_fields)]
#[schema(as = OpenTelemetryListener)]
pub struct OpenTelemetry {
    pub mode: ListenerOtelMode,
    #[serde(default = "default_otap_listener")]
    pub listener: TcpListener,
    pub tls: Option<super::Tls>,
}

impl Default for OpenTelemetry {
    fn default() -> Self {
        Self {
            mode: ListenerOtelMode::default(),
            tls: None,
            listener: default_otap_listener(),
        }
    }
}

impl OpenTelemetry {
    pub fn validate(&mut self) -> Result<()> {
        self.listener.validate()?;
        if let Some(tls) = &self.tls {
            tls.validate()?;
            tls.validate_server()?;
        }
        Ok(())
    }
}

/// Exports patrolli's own logs as log records, one per event, with the event's
/// fields and its enclosing spans' fields as record attributes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(default, deny_unknown_fields)]
#[schema(as = InternalLogsListener)]
pub struct InternalLogs {
    /// Level for `patrolli` when `include` is empty.
    pub level: Level,
    /// Modules to export, each at its own level. Empty exports `patrolli` at `level`.
    pub include: Vec<LogModule>,
    /// Modules never exported. The longest matching prefix wins, so an entry
    /// can carve a submodule out of an include.
    pub exclude: Vec<String>,
    /// Events held between capture and this listener; beyond it events are
    /// dropped and counted, never waited for.
    pub buffer_size: usize,
}

fn default_internal_logs_buffer_size() -> usize {
    10_000
}

impl Default for InternalLogs {
    fn default() -> Self {
        Self {
            level: Level::default(),
            include: Vec::new(),
            exclude: Vec::new(),
            buffer_size: default_internal_logs_buffer_size(),
        }
    }
}

impl InternalLogs {
    pub fn validate(&mut self) -> Result<()> {
        if self.buffer_size == 0 {
            self.buffer_size = default_internal_logs_buffer_size();
        }
        if self.include.iter().any(|m| m.name.is_empty()) {
            return Err(anyhow::anyhow!("include module name cannot be empty"));
        }
        if self.exclude.iter().any(String::is_empty) {
            return Err(anyhow::anyhow!("exclude module name cannot be empty"));
        }
        Ok(())
    }
}

/// Which OTel signal the `internal_metrics` listener emits.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InternalMetricsSignal {
    /// One log record per tick whose attributes are a nested document of every
    /// subsystem's counters. Supports [`InternalMetrics::report_throughput`].
    #[default]
    Log,
    /// One OTLP metric per family, cumulative, with the identifying dimensions
    /// as data-point attributes. Shares its catalogue with the Prometheus
    /// exporter, so the two outputs cannot drift.
    Metrics,
}

/// Reports this node's own internal metrics every `interval`. Every main node
/// runs its listeners, so each node reports itself under its own
/// `patrolli.node.id` scope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(default, deny_unknown_fields)]
#[schema(as = InternalMetricsListener)]
pub struct InternalMetrics {
    pub signal: InternalMetricsSignal,
    /// Log signal only: report per-second rates diffed against the previous
    /// snapshot instead of raw cumulative values.
    pub report_throughput: bool,
    /// Time between two reports. Snapshots refresh every
    /// `global.internal_metrics_interval`, so a shorter interval repeats them.
    #[serde(with = "crate::common::serde::duration_millis")]
    #[schema(value_type = u64, default = 60000)]
    pub interval: Duration,
}

fn default_internal_metrics_interval() -> Duration {
    Duration::from_secs(60)
}

impl Default for InternalMetrics {
    fn default() -> Self {
        Self {
            signal: InternalMetricsSignal::default(),
            report_throughput: false,
            interval: default_internal_metrics_interval(),
        }
    }
}

impl InternalMetrics {
    pub fn validate(&mut self) -> Result<()> {
        if self.interval.is_zero() {
            self.interval = default_internal_metrics_interval();
        }
        // Metric sums are cumulative by construction, so a rate is something the
        // storage backend derives. Pre-dividing here would produce a value that is
        // neither a Cumulative nor a Delta sum, so reject the combination rather
        // than silently ignoring the flag.
        if self.signal == InternalMetricsSignal::Metrics && self.report_throughput {
            anyhow::bail!(
                "report_throughput is not supported with 'signal: metrics'; \
                 metric sums are cumulative and rates belong to the storage backend"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn netflow(yaml: &str) -> Netflow {
        let mut conf: Netflow = serde_yaml_ng::from_str(yaml).expect("valid yaml");
        conf.validate().expect("valid config");
        conf
    }

    #[test]
    fn netflow_parser_options_default_when_absent() {
        let conf = netflow("{}");
        assert_eq!(conf.max_sources, netflow_parser::DEFAULT_MAX_SOURCES);
        assert_eq!(conf.prune_interval, Duration::from_secs(300));
        assert_eq!(conf.prune_idle_after, Duration::from_secs(3600));
    }

    #[test]
    fn netflow_parser_options_are_read_from_yaml() {
        let conf = netflow("max_sources: 50\nprune_interval: 30\nprune_idle_after: 90\n");
        assert_eq!(conf.max_sources, 50);
        assert_eq!(conf.prune_interval, Duration::from_secs(30));
        assert_eq!(conf.prune_idle_after, Duration::from_secs(90));
    }

    #[test]
    fn netflow_zero_parser_options_fall_back_to_defaults() {
        // zero means "use the default" everywhere else in this file, and a zero
        // prune_interval would panic `tokio::time::interval`
        let conf = netflow("max_sources: 0\nprune_interval: 0\nprune_idle_after: 0\n");
        assert_eq!(conf.max_sources, netflow_parser::DEFAULT_MAX_SOURCES);
        assert_eq!(conf.prune_interval, Duration::from_secs(300));
        assert_eq!(conf.prune_idle_after, Duration::from_secs(3600));
    }

    fn syslog(yaml: &str) -> Syslog {
        let mut conf: Syslog = serde_yaml_ng::from_str(yaml).expect("valid yaml");
        conf.validate().expect("valid config");
        conf
    }

    #[test]
    fn syslog_cise_defaults_when_absent() {
        let conf = syslog("{}");
        assert_eq!(conf.cise, Cise::default());
        assert!(conf.cise.enabled);
        assert_eq!(conf.cise.reassembly_timeout, Duration::from_secs(10));
        assert_eq!(conf.cise.max_pending, 16384);
        assert_eq!(conf.cise.max_pending_bytes, 64 * 1024 * 1024);
        assert_eq!(conf.cise.max_segments, 256);
    }

    #[test]
    fn syslog_cise_options_are_read_from_yaml() {
        let conf = syslog(
            "cise:\n  enabled: false\n  reassembly_timeout: 2500\n  max_pending: 10\n  max_pending_bytes: 4096\n  max_segments: 8\n",
        );
        assert!(!conf.cise.enabled);
        assert_eq!(conf.cise.reassembly_timeout, Duration::from_millis(2500));
        assert_eq!(conf.cise.max_pending, 10);
        assert_eq!(conf.cise.max_pending_bytes, 4096);
        assert_eq!(conf.cise.max_segments, 8);
        // Unset fields keep their defaults.
        assert_eq!(
            syslog("cise:\n  max_segments: 1\n").cise,
            Cise {
                max_segments: 1,
                ..Cise::default()
            }
        );
    }

    #[test]
    fn syslog_cise_zero_options_fall_back_to_defaults() {
        let conf = syslog(
            "cise:\n  reassembly_timeout: 0\n  max_pending: 0\n  max_pending_bytes: 0\n  max_segments: 0\n",
        );
        assert_eq!(conf.cise, Cise::default());
    }

    #[test]
    fn syslog_cef_and_cisco_default_on_and_read_from_yaml() {
        let conf = syslog("{}");
        assert!(conf.cef.enabled && conf.cisco.enabled);
        let conf = syslog("cef:\n  enabled: false\ncisco:\n  enabled: false\n");
        assert!(!conf.cef.enabled && !conf.cisco.enabled);
        assert!(serde_yaml_ng::from_str::<Syslog>("cef:\n  enable: false\n").is_err());
        assert!(serde_yaml_ng::from_str::<Syslog>("cisco:\n  enable: false\n").is_err());
    }

    #[test]
    fn syslog_cise_rejects_unknown_keys() {
        assert!(serde_yaml_ng::from_str::<Syslog>("cise:\n  timeout: 5\n").is_err());
    }

    fn listener(yaml: &str) -> Listener {
        let mut conf: Listener = serde_yaml_ng::from_str(yaml).expect("valid yaml");
        conf.validate().expect("valid config");
        conf
    }

    #[test]
    fn internal_logs_defaults_from_empty_config() {
        let conf = listener(
            "id: 019b77b3-df79-70a2-ab01-f43b2cf723b0\nname: logs\ntype: internal_logs\nconfig: {}\n",
        );
        let Config::InternalLogs(logs) = &conf.config else {
            panic!("not internal_logs: {:?}", conf.config);
        };
        assert_eq!(logs, &InternalLogs::default());
        assert_eq!(logs.buffer_size, 10_000);
        assert_eq!(
            ListenerKind::from(&conf.config).to_string(),
            "internal_logs"
        );
    }

    #[test]
    fn internal_logs_reads_every_option() {
        let conf = listener(
            r#"
id: 019b77b3-df79-70a2-ab01-f43b2cf723b0
name: logs
batch_size: 200
batch_timeout: 250
type: internal_logs
config:
  level: warn
  include:
    - { name: patrolli::cluster, level: debug }
  exclude: [patrolli::sink::kafka]
  buffer_size: 500
"#,
        );
        assert_eq!(conf.batch.batch_size, 200);
        assert_eq!(conf.batch.batch_timeout, Duration::from_millis(250));
        let Config::InternalLogs(logs) = conf.config else {
            panic!("not internal_logs");
        };
        assert_eq!(logs.level, Level::Warn);
        assert_eq!(
            logs.include,
            [LogModule {
                name: "patrolli::cluster".into(),
                level: Level::Debug
            }]
        );
        assert_eq!(logs.exclude, ["patrolli::sink::kafka"]);
        assert_eq!(logs.buffer_size, 500);
    }

    #[test]
    fn batch_defaults_on_the_listener_when_omitted() {
        let conf = listener(
            r#"
id: 019b77b3-df79-70a2-ab01-f43b2cf723b0
name: nf
type: netflow
config: {}
"#,
        );
        assert_eq!(conf.batch.batch_size, 1000);
        assert_eq!(conf.batch.batch_timeout, Duration::from_secs(1));
    }

    #[test]
    fn batch_is_accepted_and_ignored_by_listeners_that_do_not_batch() {
        let conf = listener(
            r#"
id: 019b77b3-df79-70a2-ab01-f43b2cf723b1
name: metrics
batch_size: 10
type: internal_metrics
config: {}
"#,
        );
        assert_eq!(conf.batch.batch_size, 10);
    }

    /// The kafka sink lost its `batch_size` twice to this: a key under the
    /// wrong block was dropped without a word.
    #[test]
    fn batch_under_config_is_rejected() {
        let err = serde_yaml_ng::from_str::<Listener>(
            r#"
id: 019b77b3-df79-70a2-ab01-f43b2cf723b0
name: nf
type: netflow
config:
  batch_size: 200
"#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("unknown field `batch_size`"), "{err}");
    }

    #[test]
    fn unknown_listener_keys_are_rejected() {
        for yaml in [
            // on the parent
            r#"
id: 019b77b3-df79-70a2-ab01-f43b2cf723b0
name: nf
workers: 2
type: netflow
config: {}
"#,
            // in a nested block
            r#"
id: 019b77b3-df79-70a2-ab01-f43b2cf723b0
name: nf
type: netflow
config:
  listener:
    address: 0.0.0.0:2055
    workers: 2
"#,
        ] {
            let err = serde_yaml_ng::from_str::<Listener>(yaml)
                .unwrap_err()
                .to_string();
            assert!(err.contains("unknown field `workers`"), "{err}");
        }
    }

    #[test]
    fn internal_logs_round_trips_through_the_db_shape() {
        // the DB stores the tag and the payload in separate columns and the
        // watcher rebuilds `{type, config}` from them
        let config = Config::InternalLogs(InternalLogs {
            exclude: vec!["patrolli::api".into()],
            ..Default::default()
        });
        let rebuilt: Config = serde_json::from_value(serde_json::json!({
            "type": ListenerKind::from(&config).to_string(),
            "config": config.to_json_value().unwrap(),
        }))
        .unwrap();
        assert_eq!(rebuilt, config);
    }

    #[test]
    fn internal_logs_zero_buffer_falls_back_and_empty_names_are_rejected() {
        let mut logs = InternalLogs {
            buffer_size: 0,
            ..Default::default()
        };
        logs.validate().unwrap();
        assert_eq!(logs.buffer_size, 10_000);

        let mut logs = InternalLogs {
            exclude: vec![String::new()],
            ..Default::default()
        };
        assert!(logs.validate().is_err());

        let mut logs = InternalLogs {
            include: vec![LogModule {
                name: String::new(),
                level: Level::Info,
            }],
            ..Default::default()
        };
        assert!(logs.validate().is_err());
    }

    #[test]
    fn internal_metrics_defaults_from_empty_config() {
        let conf = listener(
            "id: 019b77b3-df79-70a2-ab01-f43b2cf723b1\nname: metrics\ntype: internal_metrics\nconfig: {}\n",
        );
        let Config::InternalMetrics(metrics) = &conf.config else {
            panic!("not internal_metrics: {:?}", conf.config);
        };
        assert_eq!(metrics, &InternalMetrics::default());
        assert_eq!(metrics.signal, InternalMetricsSignal::Log);
        assert_eq!(metrics.interval, Duration::from_secs(60));
        assert_eq!(
            ListenerKind::from(&conf.config).to_string(),
            "internal_metrics"
        );
    }

    #[test]
    fn internal_metrics_reads_every_option() {
        let conf = listener(
            r#"
id: 019b77b3-df79-70a2-ab01-f43b2cf723b1
name: metrics
type: internal_metrics
config:
  signal: log
  report_throughput: true
  interval: 1500
"#,
        );
        let Config::InternalMetrics(metrics) = conf.config else {
            panic!("not internal_metrics");
        };
        assert_eq!(metrics.signal, InternalMetricsSignal::Log);
        assert!(metrics.report_throughput);
        assert_eq!(metrics.interval, Duration::from_millis(1500));
    }

    #[test]
    fn internal_metrics_round_trips_through_the_db_shape() {
        let config = Config::InternalMetrics(InternalMetrics {
            signal: InternalMetricsSignal::Metrics,
            interval: Duration::from_secs(5),
            ..Default::default()
        });
        let rebuilt: Config = serde_json::from_value(serde_json::json!({
            "type": ListenerKind::from(&config).to_string(),
            "config": config.to_json_value().unwrap(),
        }))
        .unwrap();
        assert_eq!(rebuilt, config);
    }

    #[test]
    fn internal_metrics_zero_interval_falls_back_and_metric_rates_are_rejected() {
        let mut metrics = InternalMetrics {
            interval: Duration::ZERO,
            ..Default::default()
        };
        metrics.validate().unwrap();
        assert_eq!(metrics.interval, Duration::from_secs(60));

        let mut metrics = InternalMetrics {
            signal: InternalMetricsSignal::Metrics,
            report_throughput: true,
            ..Default::default()
        };
        assert!(metrics.validate().is_err());
    }
}
