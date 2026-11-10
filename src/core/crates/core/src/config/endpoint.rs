// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::sensitive::{Sensitive, SensitiveFields, SensitiveRefs};
use super::snmp;
use crate::common;
use anyhow::{Result, anyhow};
use http::{Uri, uri::Authority};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::net::IpAddr;
use std::time::Duration;
use strum_macros::{Display, EnumDiscriminants, EnumString, IntoStaticStr};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, EnumDiscriminants, ToSchema)]
#[serde(rename_all = "snake_case", tag = "type", content = "config")]
#[schema(as = EndpointConfig)]
#[strum_discriminants(
    name(EndpointKind),
    derive(Serialize, Deserialize, EnumString, Display, ToSchema, IntoStaticStr),
    vis(pub),
    strum(serialize_all = "snake_case"),
    serde(rename_all = "snake_case")
)]
pub enum Config {
    SnmpV1(SnmpV1),
    #[serde(rename = "snmp_v2c")]
    #[strum_discriminants(serde(rename = "snmp_v2c"))]
    SnmpV2C(SnmpV2C),
    SnmpV3(SnmpV3),
    Http(Http),
}

impl Config {
    pub fn validate(&mut self) -> Result<()> {
        match self {
            Self::SnmpV1(snmp) => snmp.validate()?,
            Self::SnmpV2C(snmp) => snmp.validate()?,
            Self::SnmpV3(snmp) => snmp.validate()?,
            Self::Http(http) => http.validate()?,
        };
        Ok(())
    }

    pub fn to_json_value(&self) -> Result<serde_json::Value> {
        match self {
            Self::SnmpV1(snmp) => Ok(serde_json::to_value(snmp)?),
            Self::SnmpV2C(snmp) => Ok(serde_json::to_value(snmp)?),
            Self::SnmpV3(snmp) => Ok(serde_json::to_value(snmp)?),
            Self::Http(http) => Ok(serde_json::to_value(http)?),
        }
    }

    pub fn ip(&self) -> Option<IpAddr> {
        match self {
            Self::SnmpV1(snmp) => host_ip(snmp.address.host()),
            Self::SnmpV2C(snmp) => host_ip(snmp.address.host()),
            Self::SnmpV3(snmp) => host_ip(snmp.address.host()),
            Self::Http(http) => http.url.host().and_then(host_ip),
        }
    }
}

impl SensitiveFields for Config {
    fn sensitive_mut(&mut self) -> SensitiveRefs<'_> {
        match self {
            Self::SnmpV1(_) | Self::SnmpV2C(_) => Vec::new(),
            Self::SnmpV3(snmp) => snmp.security_model.sensitive_mut(),
            Self::Http(http) => http.sensitive_mut(),
        }
    }
}

/// `http` hands an IPv6 host back in its URI brackets (`[2001:db8::1]`), which
/// `IpAddr` does not parse.
fn host_ip(host: &str) -> Option<IpAddr> {
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    host.parse().ok()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub id: Uuid,
    pub name: SmolStr,
    pub host_id: Uuid,
    #[serde(flatten)]
    pub config: Config,
}

impl Endpoint {
    pub fn new(id: Uuid, name: SmolStr, host_id: Uuid, config: Config) -> Self {
        Self {
            id,
            name,
            host_id,
            config,
        }
    }

    pub fn validate(&mut self) -> Result<()> {
        self.config.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SnmpV1 {
    #[schema(value_type = String)]
    #[serde(with = "http_serde::authority")]
    pub address: Authority,
    pub community: String,
}

impl SnmpV1 {
    pub fn validate(&mut self) -> Result<()> {
        if self.address.port().is_none_or(|p| p.as_u16() == 0) {
            self.address = format!("{}:161", self.address.host()).parse()?;
        }
        anyhow::ensure!(
            !self.community.is_empty() && self.community.len() <= 64,
            "community must be between 1 and 64 bytes"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SnmpV2C {
    #[schema(value_type = String)]
    #[serde(with = "http_serde::authority")]
    pub address: Authority,
    pub community: String,
    #[serde(default = "default_max_bulk_repetitions")]
    pub max_bulk_repetitions: u32,
    #[serde(default)]
    pub disable_bulk: Option<bool>,
}

fn default_max_bulk_repetitions() -> u32 {
    25
}

impl SnmpV2C {
    pub fn validate(&mut self) -> Result<()> {
        if self.address.port().is_none_or(|p| p.as_u16() == 0) {
            self.address = format!("{}:161", self.address.host()).parse()?;
        }
        if self.max_bulk_repetitions == 0 {
            self.max_bulk_repetitions = default_max_bulk_repetitions();
        }
        anyhow::ensure!(
            !self.community.is_empty() && self.community.len() <= 64,
            "community must be between 1 and 64 bytes"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SnmpV3 {
    #[schema(value_type = String)]
    #[serde(with = "http_serde::authority")]
    pub address: Authority,
    pub security_model: snmp::SecurityModel,
    #[serde(default = "default_max_bulk_repetitions")]
    pub max_bulk_repetitions: u32,
    #[serde(default)]
    pub disable_bulk: Option<bool>,
}

impl SnmpV3 {
    pub fn validate(&mut self) -> Result<()> {
        if self.address.port().is_none_or(|p| p.as_u16() == 0) {
            self.address = format!("{}:161", self.address.host()).parse()?;
        }
        if self.max_bulk_repetitions == 0 {
            self.max_bulk_repetitions = default_max_bulk_repetitions();
        }
        self.security_model.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub enum HttpAuth {
    Basic {
        username: String,
        password: Sensitive,
    },
    Bearer(Sensitive),
}

fn default_http_connect_timeout() -> Duration {
    Duration::from_secs(1)
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Http {
    #[schema(value_type = String)]
    #[serde(with = "http_serde::uri")]
    pub url: Uri,
    pub tls: Option<super::Tls>,
    pub auth: Option<HttpAuth>,
    #[serde(
        default = "default_http_connect_timeout",
        with = "common::serde::duration_millis"
    )]
    #[schema(value_type = u64, default = 1000)]
    pub connect_timeout: Duration,
}

impl Http {
    pub fn validate(&mut self) -> Result<()> {
        let Some(scheme) = self.url.scheme_str() else {
            return Err(anyhow!("invalid url scheme"));
        };
        if scheme != "http" && scheme != "https" {
            return Err(anyhow!("invalid url scheme"));
        };
        if self.url.host().is_none() {
            return Err(anyhow!("invalid url host"));
        };
        if let Some(tls) = &self.tls {
            tls.validate()?;
            tls.validate_client()?;
        }
        Ok(())
    }

    pub fn sensitive_mut(&mut self) -> SensitiveRefs<'_> {
        let mut refs = Vec::new();
        if let Some(tls) = &mut self.tls {
            refs.extend(tls.sensitive_mut("tls.key_password"));
        }
        match &mut self.auth {
            Some(HttpAuth::Basic { password, .. }) => refs.push(("auth.basic.password", password)),
            Some(HttpAuth::Bearer(token)) => refs.push(("auth.bearer", token)),
            None => {}
        }
        refs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(value: serde_json::Value) -> Config {
        let mut config: Config = serde_json::from_value(value).unwrap();
        config.validate().unwrap();
        config
    }

    fn snmp_v2c(address: &str) -> Config {
        endpoint(serde_json::json!({
            "type": "snmp_v2c",
            "config": { "address": address, "community": "public" },
        }))
    }

    fn http(url: &str) -> Config {
        endpoint(serde_json::json!({ "type": "http", "config": { "url": url } }))
    }

    #[test]
    fn ipv6_endpoint_address_yields_its_ip() {
        let ip: IpAddr = "2001:db8::1".parse().unwrap();
        assert_eq!(snmp_v2c("[2001:db8::1]:1161").ip(), Some(ip));
        assert_eq!(snmp_v2c("[2001:db8::1]").ip(), Some(ip));
        assert_eq!(http("http://[2001:db8::1]:8080/metrics").ip(), Some(ip));
    }

    #[test]
    fn ipv4_endpoint_address_yields_its_ip() {
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        assert_eq!(snmp_v2c("192.0.2.1:1161").ip(), Some(ip));
        assert_eq!(http("https://192.0.2.1/metrics").ip(), Some(ip));
    }

    #[test]
    fn a_hostname_endpoint_has_no_ip() {
        assert_eq!(snmp_v2c("switch.example.com").ip(), None);
        assert_eq!(http("http://example.com/metrics").ip(), None);
    }

    #[test]
    fn snmp_address_without_a_port_defaults_to_161() {
        let Config::SnmpV2C(v4) = snmp_v2c("192.0.2.1") else {
            unreachable!()
        };
        assert_eq!(v4.address.as_str(), "192.0.2.1:161");
        let Config::SnmpV2C(v6) = snmp_v2c("[2001:db8::1]") else {
            unreachable!()
        };
        assert_eq!(v6.address.as_str(), "[2001:db8::1]:161");
        let Config::SnmpV2C(kept) = snmp_v2c("[2001:db8::1]:1161") else {
            unreachable!()
        };
        assert_eq!(kept.address.as_str(), "[2001:db8::1]:1161");
    }
}
