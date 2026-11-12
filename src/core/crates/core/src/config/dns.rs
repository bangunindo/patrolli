// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! `global.dns`: the in-process resolver behind the `dns_lookup` processor.
//!
//! Read from the YAML file in both run modes. The resolver always starts and
//! stays idle until a `dns_lookup` processor queues a question, so a node needs
//! no edit here to use the processor. `cache::dns` is sized from it and
//! `patrolli_dns` resolves with it.

use anyhow::{Context, Result, bail};
use ipnet::IpNet;
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;
use std::time::Duration;

/// IPv4 addresses of the 13 root servers, a to m.
pub const ROOT_SERVERS: [Ipv4Addr; 13] = [
    Ipv4Addr::new(198, 41, 0, 4),
    Ipv4Addr::new(170, 247, 170, 2),
    Ipv4Addr::new(192, 33, 4, 12),
    Ipv4Addr::new(199, 7, 91, 13),
    Ipv4Addr::new(192, 203, 230, 10),
    Ipv4Addr::new(192, 5, 5, 241),
    Ipv4Addr::new(192, 112, 36, 4),
    Ipv4Addr::new(198, 97, 190, 53),
    Ipv4Addr::new(192, 36, 148, 17),
    Ipv4Addr::new(192, 58, 128, 30),
    Ipv4Addr::new(193, 0, 14, 129),
    Ipv4Addr::new(199, 7, 83, 42),
    Ipv4Addr::new(202, 12, 27, 33),
];

/// How a question outside every zone is answered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnsMode {
    /// Recursively from the root servers; `forwarders` are asked only when
    /// that fails, never when it answers "no such name".
    #[default]
    Recursive,
    /// Every question goes to `forwarders`, for networks without open
    /// outbound DNS.
    Forward,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dns {
    #[serde(default)]
    pub mode: DnsMode,
    /// Where recursion starts. Recursive mode only.
    #[serde(default = "default_root_servers")]
    pub root_servers: Vec<IpAddr>,
    /// Applied to both of the recursor's limits. Its default of 24 fails every
    /// IPv6 reverse lookup: an `ip6.arpa` name has 34 labels.
    #[serde(default = "default_recursion_limit")]
    pub recursion_limit: u8,
    /// Zones whose name servers the recursor remembers.
    #[serde(default = "default_recursor_ns_cache_size")]
    pub recursor_ns_cache_size: usize,
    #[serde(default = "default_recursor_response_cache_size")]
    pub recursor_response_cache_size: u64,
    /// Questions resolved concurrently. Each UDP query opens its own socket.
    #[serde(default = "default_max_inflight")]
    pub max_inflight: usize,
    /// Questions waiting for a resolver; past it new ones are dropped and
    /// asked again on their next lookup.
    #[serde(default = "default_queue_capacity")]
    pub queue_capacity: usize,
    /// Per recursive lookup, in milliseconds.
    #[serde(
        default = "default_lookup_timeout",
        with = "crate::common::serde::duration_millis"
    )]
    pub lookup_timeout: Duration,
    /// Answers kept in memory, positive, negative and failed together.
    #[serde(default = "default_max_entries")]
    pub max_entries: u64,
    /// Clamp, in seconds, for the TTL of a found name.
    #[serde(default, with = "crate::common::serde::duration_seconds")]
    pub positive_ttl_min: Duration,
    #[serde(
        default = "default_positive_ttl_max",
        with = "crate::common::serde::duration_seconds"
    )]
    pub positive_ttl_max: Duration,
    /// Clamp, in seconds, for the TTL of "no such name" (from the SOA).
    #[serde(default, with = "crate::common::serde::duration_seconds")]
    pub negative_ttl_min: Duration,
    #[serde(
        default = "default_negative_ttl_max",
        with = "crate::common::serde::duration_seconds"
    )]
    pub negative_ttl_max: Duration,
    /// For a negative answer that carries no SOA record, in seconds.
    #[serde(
        default = "default_negative_ttl_default",
        with = "crate::common::serde::duration_seconds"
    )]
    pub negative_ttl_default: Duration,
    /// How long a failed lookup (timeout, refused, server failure) is
    /// remembered before it is asked again, in seconds.
    #[serde(
        default = "default_failure_ttl",
        with = "crate::common::serde::duration_seconds"
    )]
    pub failure_ttl: Duration,
    /// How long past its TTL an answer is still served while it is
    /// refreshed, in seconds. A refresh that fails keeps the last answer
    /// until then.
    #[serde(
        default = "default_serve_stale",
        with = "crate::common::serde::duration_seconds"
    )]
    pub serve_stale: Duration,
    /// `udp://ip[:port]`, `tcp://ip[:port]` or `https://host[:port][/path]`
    /// (DNS over HTTPS). Required in forward mode; a fallback for failures in
    /// recursive mode.
    #[serde(default)]
    pub forwarders: Vec<Forwarder>,
    /// Concurrent queries per forwarder.
    #[serde(default = "default_forwarder_max_inflight")]
    pub forwarder_max_inflight: usize,
    /// Per forwarder query, in milliseconds.
    #[serde(
        default = "default_forwarder_timeout",
        with = "crate::common::serde::duration_millis"
    )]
    pub forwarder_timeout: Duration,
    /// Consecutive errors or timeouts after which a forwarder is skipped for
    /// `forwarder_cooldown`. Negative answers count as successes.
    #[serde(default = "default_forwarder_failure_threshold")]
    pub forwarder_failure_threshold: u32,
    /// In milliseconds.
    #[serde(
        default = "default_forwarder_cooldown",
        with = "crate::common::serde::duration_millis"
    )]
    pub forwarder_cooldown: Duration,
    /// Address ranges answered only by their own forwarders (internal DNS),
    /// never by the recursor or `forwarders`; the longest prefix wins.
    #[serde(default)]
    pub zones: Vec<Zone>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zone {
    pub cidrs: Vec<Cidr>,
    pub forwarders: Vec<Forwarder>,
}

/// Bounds applied to TTLs before an answer is cached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TtlPolicy {
    pub positive_min: Duration,
    pub positive_max: Duration,
    pub negative_min: Duration,
    pub negative_max: Duration,
    pub negative_default: Duration,
    pub failure: Duration,
    pub stale: Duration,
}

impl Default for Dns {
    fn default() -> Self {
        Self {
            mode: DnsMode::default(),
            root_servers: default_root_servers(),
            recursion_limit: default_recursion_limit(),
            recursor_ns_cache_size: default_recursor_ns_cache_size(),
            recursor_response_cache_size: default_recursor_response_cache_size(),
            max_inflight: default_max_inflight(),
            queue_capacity: default_queue_capacity(),
            lookup_timeout: default_lookup_timeout(),
            max_entries: default_max_entries(),
            positive_ttl_min: Duration::ZERO,
            positive_ttl_max: default_positive_ttl_max(),
            negative_ttl_min: Duration::ZERO,
            negative_ttl_max: default_negative_ttl_max(),
            negative_ttl_default: default_negative_ttl_default(),
            failure_ttl: default_failure_ttl(),
            serve_stale: default_serve_stale(),
            forwarders: Vec::new(),
            forwarder_max_inflight: default_forwarder_max_inflight(),
            forwarder_timeout: default_forwarder_timeout(),
            forwarder_failure_threshold: default_forwarder_failure_threshold(),
            forwarder_cooldown: default_forwarder_cooldown(),
            zones: Vec::new(),
        }
    }
}

impl Dns {
    pub fn validate(&mut self) -> Result<()> {
        match self.mode {
            DnsMode::Forward if self.forwarders.is_empty() => {
                bail!("dns: forward mode requires forwarders")
            }
            DnsMode::Recursive if self.root_servers.is_empty() => {
                bail!("dns: root_servers must not be empty in recursive mode")
            }
            _ => {}
        }
        for (name, value) in [
            ("max_inflight", self.max_inflight as u64),
            ("queue_capacity", self.queue_capacity as u64),
            ("max_entries", self.max_entries),
            ("forwarder_max_inflight", self.forwarder_max_inflight as u64),
            (
                "forwarder_failure_threshold",
                u64::from(self.forwarder_failure_threshold),
            ),
            ("recursion_limit", u64::from(self.recursion_limit)),
        ] {
            if value == 0 {
                bail!("dns: {name} must be greater than 0");
            }
        }
        if self.lookup_timeout.is_zero() || self.forwarder_timeout.is_zero() {
            bail!("dns: lookup_timeout and forwarder_timeout must be greater than 0");
        }
        if self.positive_ttl_min > self.positive_ttl_max {
            bail!("dns: positive_ttl_min must not exceed positive_ttl_max");
        }
        if self.negative_ttl_min > self.negative_ttl_max {
            bail!("dns: negative_ttl_min must not exceed negative_ttl_max");
        }
        unique_forwarders(&self.forwarders).context("dns.forwarders")?;
        let mut cidrs = FxHashSet::default();
        for (i, zone) in self.zones.iter_mut().enumerate() {
            if zone.cidrs.is_empty() {
                bail!("dns.zones[{i}]: cidrs must not be empty");
            }
            if zone.forwarders.is_empty() {
                bail!("dns.zones[{i}]: forwarders must not be empty");
            }
            unique_forwarders(&zone.forwarders)
                .with_context(|| format!("dns.zones[{i}].forwarders"))?;
            for cidr in &mut zone.cidrs {
                // host bits are noise: 10.1.2.3/8 is 10.0.0.0/8
                cidr.0 = cidr.0.trunc();
                if !cidrs.insert(cidr.0) {
                    bail!("dns.zones[{i}]: {} is already listed by a zone", cidr.0);
                }
            }
        }
        Ok(())
    }

    pub fn ttl_policy(&self) -> TtlPolicy {
        TtlPolicy {
            positive_min: self.positive_ttl_min,
            positive_max: self.positive_ttl_max,
            negative_min: self.negative_ttl_min,
            negative_max: self.negative_ttl_max,
            negative_default: self.negative_ttl_default,
            failure: self.failure_ttl,
            stale: self.serve_stale,
        }
    }

    /// The CIDRs of every zone, for the cache to tell an internal address
    /// with a resolver of its own from one nothing can answer.
    pub fn zone_cidrs(&self) -> Vec<IpNet> {
        self.zones
            .iter()
            .flat_map(|zone| zone.cidrs.iter().map(|cidr| cidr.0))
            .collect()
    }
}

fn unique_forwarders(forwarders: &[Forwarder]) -> Result<()> {
    let mut seen = FxHashSet::default();
    for forwarder in forwarders {
        if !seen.insert(forwarder) {
            bail!("{forwarder} is listed twice");
        }
    }
    Ok(())
}

/// An address range, written `10.0.0.0/8` or `2001:db8::/32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Cidr(pub IpNet);

impl TryFrom<String> for Cidr {
    type Error = String;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        value
            .parse::<IpNet>()
            .map(Cidr)
            .map_err(|_| format!("{value:?} is not a CIDR (10.0.0.0/8, 2001:db8::/32)"))
    }
}

impl From<Cidr> for String {
    fn from(cidr: Cidr) -> Self {
        cidr.0.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Transport {
    Udp,
    Tcp,
    /// DNS over HTTPS (RFC 8484), on HTTP/2.
    Https,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Transport::Udp => "udp",
            Transport::Tcp => "tcp",
            Transport::Https => "https",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ForwarderHost {
    Ip(IpAddr),
    /// Resolved with the system resolver when the forwarder is built, never
    /// through this resolver. `https://` only.
    Name(String),
}

impl fmt::Display for ForwarderHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ForwarderHost::Ip(IpAddr::V6(ip)) => write!(f, "[{ip}]"),
            ForwarderHost::Ip(ip) => write!(f, "{ip}"),
            ForwarderHost::Name(name) => f.write_str(name),
        }
    }
}

/// An upstream resolver: `udp://ip[:port]`, `tcp://ip[:port]` (port 53 by
/// default, IP only) or `https://host[:port][/path]` (DNS over HTTPS, port
/// 443 and `/dns-query` by default; `host` is the TLS server name).
///
/// Displayed with every default spelled out, which is also how it is named in
/// metrics and logs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Forwarder {
    pub transport: Transport,
    pub host: ForwarderHost,
    pub port: u16,
    /// `Some` exactly for `https`.
    pub path: Option<String>,
}

impl fmt::Display for Forwarder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}://{}:{}{}",
            self.transport.as_str(),
            self.host,
            self.port,
            self.path.as_deref().unwrap_or("")
        )
    }
}

impl From<Forwarder> for String {
    fn from(forwarder: Forwarder) -> Self {
        forwarder.to_string()
    }
}

impl TryFrom<String> for Forwarder {
    type Error = String;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        value.parse()
    }
}

impl FromStr for Forwarder {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let usage = "expected udp://ip[:port], tcp://ip[:port] or https://host[:port][/path]";
        let (scheme, rest) = s
            .split_once("://")
            .ok_or_else(|| format!("{s:?}: {usage}"))?;
        let transport = match scheme {
            "udp" => Transport::Udp,
            "tcp" => Transport::Tcp,
            "https" => Transport::Https,
            _ => return Err(format!("{s:?}: unsupported scheme {scheme:?}; {usage}")),
        };
        let (authority, path) = match rest.find('/') {
            Some(at) => (&rest[..at], Some(&rest[at..])),
            None => (rest, None),
        };
        let default_port = match transport {
            Transport::Https => 443,
            _ => 53,
        };
        let (host, port) = split_host_port(authority)
            .map_err(|e| format!("{s:?}: {e}"))?
            .ok_or_else(|| format!("{s:?}: missing host"))?;
        let port = port.unwrap_or(default_port);
        if port == 0 {
            return Err(format!("{s:?}: port must not be 0"));
        }
        let path = match transport {
            Transport::Https => {
                let path = path.filter(|p| *p != "/").unwrap_or("/dns-query");
                Some(path.to_string())
            }
            _ if path.is_some_and(|p| p != "/") => {
                return Err(format!("{s:?}: only https:// forwarders take a path"));
            }
            _ => None,
        };
        let host = match (host.parse::<IpAddr>(), transport) {
            (Ok(ip), _) => ForwarderHost::Ip(ip),
            (Err(_), Transport::Https) if is_hostname(host) => {
                ForwarderHost::Name(host.to_ascii_lowercase())
            }
            (Err(_), Transport::Https) => return Err(format!("{s:?}: invalid host {host:?}")),
            (Err(_), _) => {
                return Err(format!(
                    "{s:?}: a {scheme}:// forwarder needs an IP address, not a host name"
                ));
            }
        };
        Ok(Forwarder {
            transport,
            host,
            port,
            path,
        })
    }
}

/// `host`, `host:port`, `[v6]`, `[v6]:port` or a bare v6 address.
fn split_host_port(authority: &str) -> std::result::Result<Option<(&str, Option<u16>)>, String> {
    if authority.is_empty() {
        return Ok(None);
    }
    let parse_port = |port: &str| {
        port.parse::<u16>()
            .map_err(|_| format!("invalid port {port:?}"))
    };
    if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, after) = bracketed
            .split_once(']')
            .ok_or_else(|| "unclosed '['".to_string())?;
        let port = match after {
            "" => None,
            _ => Some(parse_port(after.strip_prefix(':').ok_or_else(|| {
                format!("unexpected {after:?} after the address")
            })?)?),
        };
        return Ok(Some((host, port)));
    }
    if authority.parse::<IpAddr>().is_ok() {
        return Ok(Some((authority, None)));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => Ok(Some((host, Some(parse_port(port)?)))),
        None => Ok(Some((authority, None))),
    }
}

fn is_hostname(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

fn default_root_servers() -> Vec<IpAddr> {
    ROOT_SERVERS.iter().copied().map(IpAddr::V4).collect()
}

fn default_recursion_limit() -> u8 {
    64
}

fn default_recursor_ns_cache_size() -> usize {
    200_000
}

fn default_recursor_response_cache_size() -> u64 {
    1_048_576
}

fn default_max_inflight() -> usize {
    512
}

fn default_queue_capacity() -> usize {
    50_000
}

fn default_lookup_timeout() -> Duration {
    Duration::from_millis(5000)
}

fn default_max_entries() -> u64 {
    1_000_000
}

fn default_positive_ttl_max() -> Duration {
    Duration::from_secs(86_400)
}

fn default_negative_ttl_max() -> Duration {
    Duration::from_secs(3600)
}

fn default_negative_ttl_default() -> Duration {
    Duration::from_secs(300)
}

fn default_failure_ttl() -> Duration {
    Duration::from_secs(60)
}

fn default_serve_stale() -> Duration {
    Duration::from_secs(3600)
}

fn default_forwarder_max_inflight() -> usize {
    256
}

fn default_forwarder_timeout() -> Duration {
    Duration::from_millis(2000)
}

fn default_forwarder_failure_threshold() -> u32 {
    3
}

fn default_forwarder_cooldown() -> Duration {
    Duration::from_secs(30)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> std::result::Result<Forwarder, String> {
        s.parse()
    }

    fn dns(yaml: &str) -> Result<Dns> {
        let mut dns: Dns = serde_yaml_ng::from_str(yaml)?;
        dns.validate()?;
        Ok(dns)
    }

    #[test]
    fn an_empty_block_takes_the_documented_defaults() {
        let d = dns("{}").unwrap();
        assert_eq!(d.mode, DnsMode::Recursive);
        assert_eq!(d.root_servers.len(), 13);
        assert_eq!(d.recursion_limit, 64);
        assert_eq!(d.recursor_ns_cache_size, 200_000);
        assert_eq!(d.max_inflight, 512);
        assert_eq!(d.queue_capacity, 50_000);
        assert_eq!(d.lookup_timeout, Duration::from_secs(5));
        assert_eq!(d.max_entries, 1_000_000);
        assert_eq!(d.positive_ttl_max, Duration::from_secs(86_400));
        assert_eq!(d.failure_ttl, Duration::from_secs(60));
        assert_eq!(d.serve_stale, Duration::from_secs(3600));
        assert_eq!(d.forwarder_failure_threshold, 3);
        assert_eq!(d.forwarder_cooldown, Duration::from_secs(30));
        assert!(d.forwarders.is_empty() && d.zones.is_empty());
        // and the Rust default is the same thing
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            serde_json::to_value(Dns::default()).unwrap()
        );
    }

    #[test]
    fn udp_and_tcp_forwarders_default_to_port_53() {
        let f = parse("udp://8.8.8.8").unwrap();
        assert_eq!(
            (f.transport, &f.host, f.port, &f.path),
            (
                Transport::Udp,
                &ForwarderHost::Ip("8.8.8.8".parse().unwrap()),
                53,
                &None
            )
        );
        assert_eq!(parse("tcp://10.0.0.53:5353").unwrap().port, 5353);
        let v6 = parse("udp://[2001:db8::1]").unwrap();
        assert_eq!(v6.host, ForwarderHost::Ip("2001:db8::1".parse().unwrap()));
        assert_eq!(v6.to_string(), "udp://[2001:db8::1]:53");
        assert_eq!(parse("udp://[2001:db8::1]:54").unwrap().port, 54);
        assert_eq!(parse("udp://2001:db8::1").unwrap().port, 53);
    }

    #[test]
    fn https_forwarders_default_to_443_and_dns_query() {
        let ip = parse("https://1.1.1.1").unwrap();
        assert_eq!(ip.host, ForwarderHost::Ip("1.1.1.1".parse().unwrap()));
        assert_eq!((ip.port, ip.path.as_deref()), (443, Some("/dns-query")));
        assert_eq!(ip.to_string(), "https://1.1.1.1:443/dns-query");

        let named = parse("https://Dns.Google:8443/resolve").unwrap();
        assert_eq!(named.host, ForwarderHost::Name("dns.google".into()));
        assert_eq!(
            (named.port, named.path.as_deref()),
            (8443, Some("/resolve"))
        );
        assert_eq!(
            parse("https://dns.quad9.net/").unwrap().path.as_deref(),
            Some("/dns-query")
        );
        // the canonical form parses back to itself
        assert_eq!(parse(&named.to_string()).unwrap(), named);
    }

    #[test]
    fn bad_forwarders_are_rejected() {
        for (input, error) in [
            ("8.8.8.8", "expected"),
            ("tls://1.1.1.1", "unsupported scheme"),
            ("dns://1.1.1.1", "unsupported scheme"),
            ("udp://dns.google", "needs an IP address"),
            ("tcp://resolver.local:53", "needs an IP address"),
            ("udp://8.8.8.8/dns-query", "only https://"),
            ("udp://8.8.8.8:0", "port must not be 0"),
            ("udp://8.8.8.8:99999", "invalid port"),
            ("https://", "missing host"),
            ("https://bad_host!", "invalid host"),
            ("https://[2001:db8::1", "unclosed"),
        ] {
            let err = parse(input).unwrap_err();
            assert!(err.contains(error), "{input}: {err}");
        }
    }

    #[test]
    fn forward_mode_needs_forwarders() {
        assert!(dns("mode: forward").is_err());
        dns("mode: forward\nforwarders: [udp://10.0.0.53]").unwrap();
    }

    #[test]
    fn limits_and_ttl_bounds_are_checked() {
        for yaml in [
            "max_inflight: 0",
            "queue_capacity: 0",
            "forwarder_failure_threshold: 0",
            "positive_ttl_min: 10\npositive_ttl_max: 5",
            "negative_ttl_min: 10\nnegative_ttl_max: 5",
            "root_servers: []",
            "lookup_timeout: 0",
            "forwarders: [udp://1.1.1.1, 'udp://1.1.1.1:53']",
            "unknown_knob: 1",
        ] {
            assert!(dns(yaml).is_err(), "{yaml}");
        }
    }

    /// The block is commented out in the reference (it shows the defaults),
    /// so nothing else would notice it drifting from the struct.
    #[test]
    fn the_documented_block_deserializes_and_validates() {
        let reference = include_str!("../../../../config-full.yaml");
        let block = reference
            .split_once("  # dns:\n")
            .unwrap()
            .1
            .split_once("\n\n")
            .unwrap()
            .0;
        let yaml = block
            .lines()
            .map(|line| line.strip_prefix("  # ").unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let mut documented: Dns = serde_yaml_ng::from_str(&yaml).unwrap();
        documented.validate().unwrap();
        let defaults = Dns::default();
        assert_eq!(documented.root_servers, defaults.root_servers);
        assert_eq!(documented.serve_stale, defaults.serve_stale);
        assert_eq!(documented.forwarder_cooldown, defaults.forwarder_cooldown);
        assert_eq!(documented.forwarders.len(), 2);
        assert_eq!(documented.zones.len(), 1);
    }

    #[test]
    fn zones_need_forwarders_and_distinct_cidrs() {
        let ok = dns("zones:\n\
             - cidrs: [10.1.2.3/8, 192.168.0.0/16]\n  forwarders: [udp://10.0.0.53]\n\
             - cidrs: [10.20.0.0/16]\n  forwarders: [https://dns.corp.example]\n")
        .unwrap();
        // host bits are dropped, and a nested range is allowed
        assert_eq!(ok.zones[0].cidrs[0].0, "10.0.0.0/8".parse().unwrap());
        assert_eq!(ok.zone_cidrs().len(), 3);

        for (yaml, error) in [
            (
                "zones: [{cidrs: [10.0.0.0/8], forwarders: []}]",
                "forwarders must not be empty",
            ),
            (
                "zones: [{cidrs: [], forwarders: [udp://10.0.0.53]}]",
                "cidrs must not be empty",
            ),
            (
                "zones:\n- {cidrs: [10.0.0.0/8], forwarders: [udp://10.0.0.53]}\n\
                 - {cidrs: [10.9.9.9/8], forwarders: [udp://10.0.0.54]}",
                "already listed",
            ),
        ] {
            let err = format!("{:#}", dns(yaml).unwrap_err());
            assert!(err.contains(error), "{yaml}: {err}");
        }
        assert!(dns("zones: [{cidrs: [not-a-cidr], forwarders: [udp://10.0.0.53]}]").is_err());
    }
}
