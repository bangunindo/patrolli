// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::snmp;
use crate::{common, payload};
use anyhow::{Context, Result, anyhow};
use async_snmp::Oid;
use cron::Schedule as CronSchedule;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use smol_str::SmolStr;
use std::net::IpAddr;
use std::str::FromStr;
use strum_macros::{Display, EnumDiscriminants, EnumString, IntoStaticStr};
use tokio::time::{Duration, Instant};
use tracing::error;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, EnumDiscriminants, ToSchema)]
#[serde(rename_all = "snake_case", tag = "type", content = "config")]
#[schema(as = ItemConfig)]
#[strum_discriminants(
    name(ItemKind),
    derive(
        Hash,
        Serialize,
        Deserialize,
        EnumString,
        Display,
        ToSchema,
        IntoStaticStr
    ),
    vis(pub),
    strum(serialize_all = "snake_case"),
    serde(rename_all = "snake_case")
)]
pub enum Config {
    Listener(Listener),
    Snmp(Snmp),
    SnmpWalk(SnmpWalk),
    Dummy(Dummy),
    PrometheusScrape(PrometheusScrape),
}

impl Config {
    pub fn validate(&mut self) -> Result<()> {
        match self {
            Config::Listener(config) => config.validate()?,
            Config::Snmp(config) => config.validate()?,
            Config::SnmpWalk(config) => config.validate()?,
            Config::Dummy(config) => config.validate()?,
            Config::PrometheusScrape(config) => config.validate()?,
        }
        Ok(())
    }

    pub fn to_json_value(&self) -> Result<serde_json::Value> {
        match self {
            Config::Listener(config) => Ok(serde_json::to_value(config)?),
            Config::Snmp(config) => Ok(serde_json::to_value(config)?),
            Config::SnmpWalk(config) => Ok(serde_json::to_value(config)?),
            Config::Dummy(config) => Ok(serde_json::to_value(config)?),
            Config::PrometheusScrape(config) => Ok(serde_json::to_value(config)?),
        }
    }
}

// TODO: CronSchedule is big, measured at 624 Bytes (many BTreeSet), we should either:
//  - Interned CronSchedule, and use Arc<CronSchedule> instead
//  - Look for alternative data structures to present CronSchedule
#[derive(Debug, Clone)]
pub enum Schedule {
    Cron(Box<CronSchedule>),
    Interval(Duration),
    // A special schedule that is primarily used on tests. Usually this kind of
    // schedule will use the execute now function or listener item
    Never,
}

impl FromStr for Schedule {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "never" {
            return Ok(Schedule::Never);
        }
        // Try parsing as a number (e.g. "300")
        if let Ok(secs) = s.parse::<u64>() {
            return Ok(Schedule::Interval(Duration::from_secs(secs)));
        }
        let cron = CronSchedule::from_str(s).context("failed to parse cron schedule")?;
        Ok(Schedule::Cron(Box::new(cron)))
    }
}

// TODO: Many of configs are template based, consider using interned config to reduce memory usage
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub id: Uuid,
    pub name: SmolStr,
    pub endpoint_id: Option<Uuid>,
    pub host_id: Uuid,
    #[serde(default = "default_schedule", with = "common::serde::schedule")]
    pub schedule: Schedule,
    pub listener_id: Option<Uuid>,
    #[serde(default = "default_timeout", with = "common::serde::duration_millis")]
    pub timeout: Duration,
    // TODO: customize retry behavior and delay. retry: fixed and backoff
    #[serde(default)]
    pub processors: SmallVec<[Uuid; 4]>,
    #[serde(flatten)]
    pub config: Config,
}

impl Item {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: Uuid,
        name: SmolStr,
        endpoint_id: Option<Uuid>,
        host_id: Uuid,
        schedule: Schedule,
        listener_id: Option<Uuid>,
        timeout: Duration,
        processors: Vec<Uuid>,
        config: Config,
    ) -> Self {
        Self {
            id,
            name,
            endpoint_id,
            host_id,
            schedule,
            listener_id,
            timeout,
            processors: processors.into(),
            config,
        }
    }

    pub fn validate(&mut self) -> Result<()> {
        if let Schedule::Interval(duration) = self.schedule
            && duration.is_zero()
        {
            self.schedule = default_schedule();
        }
        if self.timeout.is_zero() {
            self.timeout = default_timeout();
        }
        self.config.validate()?;
        Ok(())
    }
}

fn default_timeout() -> Duration {
    Duration::from_secs(1)
}

fn default_schedule() -> Schedule {
    Schedule::Interval(Duration::from_secs(300))
}

impl Item {
    pub fn supports_batch_check(&self) -> bool {
        match self.config {
            Config::Listener(_) => false,
            Config::Snmp(_) => true,
            Config::SnmpWalk(_) => false,
            Config::Dummy(_) => false,
            Config::PrometheusScrape(_) => false,
        }
    }

    pub fn schedulable(&self) -> bool {
        match self.config {
            Config::Listener(_) => false,
            Config::Snmp(_) => true,
            Config::SnmpWalk(_) => true,
            Config::Dummy(_) => true,
            Config::PrometheusScrape(_) => true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ItemMonitor {
    pub next_check: Instant,
    pub id: Uuid,
    pub retries: u32,
}

impl ItemMonitor {
    pub fn new(id: Uuid, schedule: &Schedule) -> Self {
        let mut item = Self {
            next_check: Instant::now(),
            id,
            retries: 0,
        };
        item.update_next_check(schedule);
        item
    }

    fn uuid_mod_interval(&self, interval: Duration) -> Duration {
        let uint_repr = u64::from_be_bytes(self.id.as_bytes()[8..].try_into().unwrap());
        // the smallest duration is in second, we don't need nanosecond
        let dur_mod = uint_repr % (interval.as_nanos() as u64 / 100_000_000);
        Duration::from_nanos(dur_mod * 100_000_000)
    }

    pub fn update_next_check(&mut self, schedule: &Schedule) {
        let time_ref = Instant::now();
        match schedule {
            Schedule::Never => {
                self.next_check = Instant::now() + Duration::from_hours(24 * 365);
            }
            Schedule::Interval(interval) => {
                self.next_check = common::truncate_instant(time_ref, *interval);
                self.next_check += self.uuid_mod_interval(*interval);
                while self.next_check < time_ref {
                    self.next_check += *interval;
                }
            }
            Schedule::Cron(schedule) => {
                let now = common::now();
                if let Some(delta) = schedule
                    .upcoming(*common::DEFAULT_TZ)
                    .find(|dt| dt > &now)
                    .and_then(|dt| dt.signed_duration_since(now).to_std().ok())
                {
                    self.next_check = time_ref + delta;
                } else {
                    error!(
                        item_id = %self.id,
                        schedule = ?schedule,
                        "failed to calculate next check time for cron schedule"
                    );
                }
            }
        }
    }

    pub fn is_due(&self) -> bool {
        self.next_check <= Instant::now()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Snmp {
    #[serde(with = "common::serde::oid")]
    #[schema(value_type = String)]
    pub oid: Oid,
    #[serde(default = "default_true")]
    pub use_mib: bool,
}

impl Snmp {
    pub fn validate(&mut self) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SnmpWalk {
    pub oids: Vec<snmp::OidChain>,
    #[serde(default)]
    pub table_format: bool,
    /// How many of the configured OIDs may produce no value for a row before that
    /// row is discarded instead of shipped incomplete.
    ///
    /// Counts every OID in the config forest, chained ones included, so `0` demands
    /// that every one of them resolve. Omitted (`null`) disables the check and every
    /// row is emitted whatever it holds. Only meaningful with `table_format`, which
    /// is the only mode that has rows.
    #[serde(default)]
    pub max_unknown_oids_per_row: Option<usize>,
    #[serde(default = "default_true")]
    pub use_mib: bool,
    #[serde(default)]
    pub disable_bulk: Option<bool>,
    #[serde(default)]
    pub poll_worker_count: usize,
    #[serde(default = "default_walk_max_results")]
    pub max_results: usize,
    #[serde(default = "default_walk_chunk_size")]
    pub chunk_size: usize,
    #[serde(
        default = "default_walk_request_timeout",
        with = "crate::common::serde::duration_millis"
    )]
    #[schema(value_type = u64, default = 5000)]
    pub request_timeout: Duration,
    #[serde(default = "default_walk_request_retries")]
    pub request_retries: u32,
}

fn default_true() -> bool {
    true
}

fn default_walk_max_results() -> usize {
    100_000
}

fn default_walk_chunk_size() -> usize {
    100
}

fn default_walk_request_timeout() -> Duration {
    Duration::from_secs(5)
}

fn default_walk_request_retries() -> u32 {
    2
}

impl SnmpWalk {
    pub fn validate(&mut self) -> Result<()> {
        if self.oids.is_empty() {
            return Err(anyhow!("no oids specified"));
        }
        if self.poll_worker_count == 0 {
            self.poll_worker_count = 1;
        }
        if self.request_retries > async_snmp::MAX_RETRIES {
            return Err(anyhow!(
                "request_retries must not exceed {}",
                async_snmp::MAX_RETRIES
            ));
        }
        if !self.table_format {
            for oid in &self.oids {
                if !oid.chain.is_empty() {
                    return Err(anyhow!(
                        "oid chain is not supported without table_format enabled: {}",
                        oid.oid
                    ));
                }
            }
            if self.max_unknown_oids_per_row.is_some() {
                return Err(anyhow!(
                    "max_unknown_oids_per_row is not supported without table_format enabled"
                ));
            }
        }
        for oid in &self.oids {
            oid.validate()?;
        }
        Ok(())
    }

    /// Every OID this item can ask a device for, chains included: the number of
    /// values a fully-answered row carries, which is what
    /// `max_unknown_oids_per_row` is measured against.
    pub fn oid_count(&self) -> usize {
        let mut oid_count = 0usize;
        for chain in &self.oids {
            oid_count += chain.oid_count();
        }
        oid_count
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DummyPayload {
    pub row: payload::LogRowOwned,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Dummy {
    pub payload: DummyPayload,
    #[serde(default = "default_dummy_count")]
    pub count: usize,
}

fn default_dummy_count() -> usize {
    1
}

impl Dummy {
    fn validate(&mut self) -> Result<()> {
        if self.count == 0 {
            self.count = default_dummy_count();
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, EnumDiscriminants, ToSchema)]
#[serde(
    rename_all = "snake_case",
    tag = "method",
    content = "config",
    deny_unknown_fields
)]
#[strum_discriminants(
    name(ListenerItemMatchStrategyMethod),
    derive(Serialize, Deserialize, EnumString, Display, ToSchema, PartialOrd, Ord,),
    vis(pub),
    strum(serialize_all = "snake_case"),
    serde(rename_all = "snake_case")
)]
// be careful, the order of this enum matters in the match strategy.
// user can use 'None' as a fallback option
pub enum ListenerItemMatchStrategy {
    /// Match by endpoint source ip, only supported on endpoint defined with IP.
    /// Hostname is not supported.
    EndpointSourceIp,
    #[schema(value_type = Vec<String>)]
    SourceIps(Vec<IpAddr>),
    AttributeMatch(super::processor::AttributeFilter),
    #[default]
    None,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Listener {
    #[serde(default)]
    pub match_strategy: ListenerItemMatchStrategy,
}

impl Listener {
    fn validate(&mut self) -> Result<()> {
        if let ListenerItemMatchStrategy::SourceIps(ips) = &mut self.match_strategy {
            ips.sort_unstable();
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PrometheusScrape {
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub accept_protobuf: bool,
}

impl PrometheusScrape {
    pub fn validate(&mut self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn walk(value: serde_json::Value) -> SnmpWalk {
        serde_json::from_value(value).expect("valid snmp walk config")
    }

    #[test]
    fn oid_count_spans_every_root_and_every_chain() {
        let conf = walk(json!({
            "table_format": true,
            "oids": [
                {"oid": "1.3.6.1.2.1.2.2.1.1", "chain": [
                    {"oid": "1.3.6.1.2.1.31.1.1.1.18"},
                ]},
                {"oid": "1.3.6.1.2.1.2.2.1.2"},
            ],
        }));
        assert_eq!(conf.oid_count(), 3);
    }

    #[test]
    fn max_unknown_oids_per_row_defaults_to_off() {
        let conf = walk(json!({"oids": [{"oid": "1.3.6.1.2.1.2.2.1.1"}]}));
        assert_eq!(conf.max_unknown_oids_per_row, None);
    }

    #[test]
    fn max_unknown_oids_per_row_needs_table_format() {
        // there are no rows to judge without it, so the setting would silently do nothing
        let mut conf = walk(json!({
            "oids": [{"oid": "1.3.6.1.2.1.2.2.1.1"}],
            "max_unknown_oids_per_row": 1,
        }));
        assert!(conf.validate().is_err());

        let mut conf = walk(json!({
            "table_format": true,
            "oids": [{"oid": "1.3.6.1.2.1.2.2.1.1"}],
            "max_unknown_oids_per_row": 1,
        }));
        assert!(conf.validate().is_ok());
    }
}
