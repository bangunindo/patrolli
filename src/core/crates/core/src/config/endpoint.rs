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
    Icmp(Icmp),
}

impl Config {
    pub fn validate(&mut self) -> Result<()> {
        match self {
            Self::SnmpV1(snmp) => snmp.validate()?,
            Self::SnmpV2C(snmp) => snmp.validate()?,
            Self::SnmpV3(snmp) => snmp.validate()?,
            Self::Http(http) => http.validate()?,
            Self::Icmp(icmp) => icmp.validate()?,
        };
        Ok(())
    }

    pub fn to_json_value(&self) -> Result<serde_json::Value> {
        match self {
            Self::SnmpV1(snmp) => Ok(serde_json::to_value(snmp)?),
            Self::SnmpV2C(snmp) => Ok(serde_json::to_value(snmp)?),
            Self::SnmpV3(snmp) => Ok(serde_json::to_value(snmp)?),
            Self::Http(http) => Ok(serde_json::to_value(http)?),
            Self::Icmp(icmp) => Ok(serde_json::to_value(icmp)?),
        }
    }

    pub fn ip(&self) -> Option<IpAddr> {
        match self {
            Self::SnmpV1(snmp) => host_ip(snmp.address.host()),
            Self::SnmpV2C(snmp) => host_ip(snmp.address.host()),
            Self::SnmpV3(snmp) => host_ip(snmp.address.host()),
            Self::Http(http) => http.url.host().and_then(host_ip),
            Self::Icmp(icmp) => host_ip(&icmp.address),
        }
    }

    /// The endpoint's host, an IP literal (IPv6 without its brackets) or a
    /// hostname, whatever the endpoint type: what an icmp item pings.
    pub fn host(&self) -> Option<&str> {
        let host = match self {
            Self::SnmpV1(snmp) => snmp.address.host(),
            Self::SnmpV2C(snmp) => snmp.address.host(),
            Self::SnmpV3(snmp) => snmp.address.host(),
            Self::Http(http) => http.url.host()?,
            Self::Icmp(icmp) => icmp.address.as_str(),
        };
        Some(strip_brackets(host))
    }
}

impl SensitiveFields for Config {
    fn sensitive_mut(&mut self) -> SensitiveRefs<'_> {
        match self {
            Self::SnmpV1(_) | Self::SnmpV2C(_) => Vec::new(),
            Self::SnmpV3(snmp) => snmp.security_model.sensitive_mut(),
            Self::Http(http) => http.sensitive_mut(),
            Self::Icmp(_) => Vec::new(),
        }
    }
}

/// `http` hands an IPv6 host back in its URI brackets (`[2001:db8::1]`), which
/// `IpAddr` does not parse.
fn host_ip(host: &str) -> Option<IpAddr> {
    strip_brackets(host).parse().ok()
}

fn strip_brackets(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
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

/// A host to ping: an IP address or a hostname, without a port.
// The field is `address`, like the SNMP endpoints', because the database
// trigger that refreshes listener endpoint IPs reads `config -> 'address'`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = IcmpEndpoint)]
pub struct Icmp {
    pub address: String,
}

impl Icmp {
    pub fn validate(&mut self) -> Result<()> {
        let address = self.address.trim();
        anyhow::ensure!(!address.is_empty(), "address must not be empty");
        let host = strip_brackets(address);
        let valid = host.parse::<IpAddr>().is_ok()
            || (host == address
                && host.len() <= 253
                && host
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_')));
        anyhow::ensure!(
            valid,
            "address must be an IP address or a hostname, without a port: {address}"
        );
        self.address = host.to_string();
        Ok(())
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

    fn icmp(address: &str) -> Result<Config> {
        let mut config: Config = serde_json::from_value(serde_json::json!({
            "type": "icmp",
            "config": { "address": address },
        }))?;
        config.validate()?;
        Ok(config)
    }

    #[test]
    fn icmp_address_accepts_ips_and_hostnames() {
        for (given, kept) in [
            ("192.0.2.1", "192.0.2.1"),
            (" 192.0.2.1 ", "192.0.2.1"),
            ("2001:db8::1", "2001:db8::1"),
            ("[2001:db8::1]", "2001:db8::1"),
            ("switch-01.example.com", "switch-01.example.com"),
            ("my_host", "my_host"),
        ] {
            let Config::Icmp(conf) = icmp(given).unwrap() else {
                unreachable!()
            };
            assert_eq!(conf.address, kept, "{given}");
        }
    }

    #[test]
    fn icmp_address_rejects_ports_and_junk() {
        for address in [
            "",
            "  ",
            "192.0.2.1:7",
            "[2001:db8::1]:7",
            "example.com:80",
            "http://example.com",
            "a b",
            &"a".repeat(254),
        ] {
            assert!(icmp(address).is_err(), "{address:?} must be rejected");
        }
    }

    #[test]
    fn icmp_endpoint_has_no_other_fields() {
        let config = serde_json::from_value::<Config>(serde_json::json!({
            "type": "icmp",
            "config": { "address": "192.0.2.1", "port": 7 },
        }));
        assert!(config.is_err());
    }

    #[test]
    fn host_is_the_bare_host_of_every_endpoint_type() {
        assert_eq!(snmp_v2c("192.0.2.1:1161").host(), Some("192.0.2.1"));
        assert_eq!(snmp_v2c("[2001:db8::1]:1161").host(), Some("2001:db8::1"));
        assert_eq!(
            snmp_v2c("switch.example.com").host(),
            Some("switch.example.com")
        );
        assert_eq!(
            http("http://[2001:db8::1]:8080/metrics").host(),
            Some("2001:db8::1")
        );
        assert_eq!(
            http("https://example.com/metrics").host(),
            Some("example.com")
        );
        assert_eq!(icmp("[2001:db8::1]").unwrap().host(), Some("2001:db8::1"));
        assert_eq!(icmp("example.com").unwrap().host(), Some("example.com"));
    }

    #[test]
    fn icmp_endpoint_yields_its_ip() {
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        assert_eq!(icmp("192.0.2.1").unwrap().ip(), Some(ip));
        assert_eq!(icmp("example.com").unwrap().ip(), None);
    }

    #[test]
    fn icmp_endpoint_round_trips_through_yaml() {
        let yaml = "id: 0194f2a0-0000-7000-8000-000000000001\n\
                    name: gw\n\
                    host_id: 0194f2a0-0000-7000-8000-000000000002\n\
                    type: icmp\n\
                    config:\n  address: 192.0.2.1\n";
        let mut endpoint: Endpoint = serde_yaml_ng::from_str(yaml).unwrap();
        endpoint.validate().unwrap();
        let Config::Icmp(conf) = &endpoint.config else {
            panic!("an icmp endpoint");
        };
        assert_eq!(conf.address, "192.0.2.1");
        let back: Endpoint =
            serde_yaml_ng::from_str(&serde_yaml_ng::to_string(&endpoint).unwrap()).unwrap();
        assert!(matches!(back.config, Config::Icmp(Icmp { address }) if address == "192.0.2.1"));
    }
}
