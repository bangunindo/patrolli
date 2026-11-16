// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The `icmp` item: ping the host of the item's endpoint `count` times and
//! report reachability, loss, round-trip time and jitter as gauges.
//!
//! One ICMP socket per address family serves every item of the process
//! ([`client`]). On Linux's unprivileged `SOCK_DGRAM` ICMP sockets the kernel
//! owns the echo identifier, so a reply is matched by `(target, sequence)`
//! alone: every echo request takes its sequence from one process-wide counter,
//! which keeps two in-flight requests to one target from colliding.

use crate::{Collected, Error, MonitorMetric, MonitorRecord};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use otel_arrow_dfe_pdata::otlp::metrics::MetricType;
use patrolli::cache::item::Stats;
use patrolli::config::endpoint::Endpoint;
use patrolli::config::host::Host;
use patrolli::config::item::{Config as ItemConfig, Icmp, IpVersion, Item, ItemKind};
use patrolli::payload::{
    DataPointsOwned, GaugeOwned, MetricRowOwned, NumberDataPointOwned, NumberValue, ZStringOwned,
};
use rustc_hash::FxHashMap;
use smol_str::SmolStr;
use std::hash::BuildHasher;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU16, Ordering};
use surge_ping::{Client, Config, ICMP, IcmpPacket, PingIdentifier, PingSequence, SurgeError};
use tokio::sync::OnceCell;
use tokio::time::{Duration, Instant, sleep_until, timeout_at};
use tracing::info;

static CLIENT_V4: OnceCell<Client> = OnceCell::const_new();
static CLIENT_V6: OnceCell<Client> = OnceCell::const_new();

/// The echo identifier of every request this process sends. The kernel
/// replaces it on Linux `SOCK_DGRAM` sockets; on raw sockets (and macOS) it is
/// what tells our replies from another pinger's.
static IDENTIFIER: LazyLock<PingIdentifier> =
    LazyLock::new(|| PingIdentifier(std::hash::RandomState::new().hash_one(()) as u16));

/// The sequence of the next echo request, shared by every target so that
/// `(target, sequence)` stays unique while requests are in flight.
static SEQUENCE: AtomicU16 = AtomicU16::new(0);

const ICMPV4_ECHO_REPLY: u8 = 0;
const ICMPV4_DEST_UNREACHABLE: u8 = 3;
const ICMPV6_DEST_UNREACHABLE: u8 = 1;
const ICMPV6_ECHO_REPLY: u8 = 129;

/// The process-wide ICMP client for `ip`'s family, opened on first use.
///
/// A failed open is not cached, so the next check tries again: granting the
/// privilege (`net.ipv4.ping_group_range`, `CAP_NET_RAW`) needs no restart.
/// Like the SNMP transport, the socket's receive task lives on the runtime
/// that opened it.
pub(crate) async fn client(ip: IpAddr) -> Result<&'static Client, Error> {
    let (cell, kind, family) = match ip {
        IpAddr::V4(_) => (&CLIENT_V4, ICMP::V4, "v4"),
        IpAddr::V6(_) => (&CLIENT_V6, ICMP::V6, "v6"),
    };
    cell.get_or_try_init(|| async {
        let client = Client::new(&Config::builder().kind(kind).build()).map_err(|e| {
            Error::config(format!(
                "failed to open icmp {family} socket, needs net.ipv4.ping_group_range \
                 or CAP_NET_RAW: {e}"
            ))
        })?;
        let socket_type = if client.get_socket().get_type() == socket2::Type::RAW {
            "raw"
        } else {
            "dgram"
        };
        info!(family, socket_type, "icmp socket opened");
        Ok(client)
    })
    .await
}

pub async fn process_icmp(
    item: &Item,
    endpoint: &Endpoint,
    host: &Host,
    stats: &Arc<Stats>,
) -> Result<Collected, Error> {
    let ItemConfig::Icmp(conf) = &item.config else {
        return Err(Error::config(format!(
            "not an icmp config: {:?}",
            std::mem::discriminant(&item.config),
        )));
    };
    // Everything below finishes by `deadline`, so the result reaches the
    // worker before its own `item.timeout` drops the whole check.
    let deadline = Instant::now() + item.timeout.saturating_sub(Icmp::DEADLINE_MARGIN);

    let target = endpoint
        .config
        .host()
        .ok_or_else(|| Error::config("endpoint has no host"))?;
    let ip = resolve(target, conf.ip_version, deadline).await?;
    let client = client(ip).await?;

    let replies = ping_all(client, ip, conf, deadline).await?;
    let summary = summarize(&replies.rtts);
    let rows = build_rows(&summary, replies.ttl, ip, Utc::now());

    let resource = crate::create_resource(host, Some(endpoint), item);
    let kind = ItemKind::from(&item.config);
    let records: Vec<MonitorMetric> = rows
        .into_iter()
        .map(|row| MonitorMetric {
            row,
            resource: resource.clone(),
            kind,
            processors: item.processors.clone(),
            stats: stats.clone(),
        })
        .collect();
    let count = records.len() as u64;
    let record = MonitorRecord::BatchMetric(records);

    if summary.received > 0 {
        return Ok(Collected::new(record, count));
    }
    // Down: the rows saying so are still delivered, and the item fails too.
    let reason = replies
        .reason
        .unwrap_or_else(|| "no echo reply".to_string());
    let mut failed = FxHashMap::default();
    failed.insert(
        item.id,
        Error::unreachable(format!("{reason}, {} packets sent to {ip}", summary.sent)),
    );
    Ok(Collected::partial(record, count, failed))
}

/// The address to ping: `target` itself when it is an IP literal, otherwise
/// the first address it resolves to in the wanted family.
async fn resolve(target: &str, version: IpVersion, deadline: Instant) -> Result<IpAddr, Error> {
    if let Ok(ip) = target.parse::<IpAddr>() {
        return if version.matches(&ip) {
            Ok(ip)
        } else {
            Err(Error::config(format!(
                "{ip} does not match ip_version {version:?}"
            )))
        };
    }
    let addrs = timeout_at(deadline, tokio::net::lookup_host((target, 0)))
        .await
        .map_err(|_| Error::timeout(format!("resolving {target} timed out")))?
        .map_err(|e| Error::unreachable(format!("failed to resolve {target}: {e}")))?;
    let mut any = false;
    for addr in addrs {
        any = true;
        if version.matches(&addr.ip()) {
            return Ok(addr.ip());
        }
    }
    if any {
        Err(Error::config(format!(
            "{target} has no address matching ip_version {version:?}"
        )))
    } else {
        Err(Error::unreachable(format!("failed to resolve {target}")))
    }
}

/// What came back from one check's echo requests.
struct Replies {
    /// One entry per request, in send order: its round-trip time, or `None`
    /// when it went unanswered.
    rtts: Vec<Option<Duration>>,
    /// The TTL of the last reply, when the socket reports one (IPv4 on a raw
    /// socket, or on macOS).
    ttl: Option<u8>,
    /// Why requests were lost, when something said so.
    reason: Option<String>,
}

/// Send `conf.count` echo requests `conf.interval` apart, each waiting for its
/// reply until `deadline`. Each request is its own future, so a slow reply
/// never holds back the next send.
async fn ping_all(
    client: &Client,
    ip: IpAddr,
    conf: &Icmp,
    deadline: Instant,
) -> Result<Replies, Error> {
    let payload = vec![0u8; usize::from(conf.payload_size)];
    let start = Instant::now();
    let mut pending: FuturesUnordered<_> = (0..conf.count)
        .map(|i| {
            let payload = &payload;
            async move {
                sleep_until(start + conf.interval * u32::from(i)).await;
                let wait = deadline.saturating_duration_since(Instant::now());
                if wait.is_zero() {
                    return (i, None);
                }
                let mut pinger = client.pinger(ip, *IDENTIFIER).await;
                pinger.timeout(wait);
                let seq = PingSequence(SEQUENCE.fetch_add(1, Ordering::Relaxed));
                (i, Some(pinger.ping(seq, payload).await))
            }
        })
        .collect();

    let mut rtts = vec![None; usize::from(conf.count)];
    let mut ttls: Vec<Option<u8>> = vec![None; usize::from(conf.count)];
    let mut reason = None;
    while let Some((i, result)) = pending.next().await {
        let i = usize::from(i);
        match result {
            // Never sent: the deadline came first.
            None => {}
            Some(Ok((IcmpPacket::V4(packet), rtt))) => match packet.get_icmp_type().0 {
                ICMPV4_ECHO_REPLY => {
                    rtts[i] = Some(rtt);
                    ttls[i] = packet.get_ttl();
                }
                ICMPV4_DEST_UNREACHABLE => {
                    reason = Some("destination unreachable".to_string());
                }
                other => reason = Some(format!("icmp type {other} received")),
            },
            Some(Ok((IcmpPacket::V6(packet), rtt))) => match packet.get_icmpv6_type().0 {
                // The IPv6 hop limit the crate reports is not the reply's, so
                // no TTL is kept for IPv6.
                ICMPV6_ECHO_REPLY => rtts[i] = Some(rtt),
                ICMPV6_DEST_UNREACHABLE => {
                    reason = Some("destination unreachable".to_string());
                }
                other => reason = Some(format!("icmpv6 type {other} received")),
            },
            Some(Err(SurgeError::ClientDestroyed)) => {
                return Err(Error::internal("icmp client destroyed"));
            }
            Some(Err(SurgeError::Timeout { .. })) => {}
            // A send error (no route, network down) says more than silence.
            Some(Err(e)) => reason = Some(e.to_string()),
        }
    }
    let ttl = ttls.into_iter().rev().flatten().next();
    Ok(Replies { rtts, ttl, reason })
}

#[derive(Debug, PartialEq)]
pub(crate) struct Summary {
    pub sent: usize,
    pub received: usize,
    /// Lost requests over sent ones, 0 to 1.
    pub loss: f64,
    pub min: Option<Duration>,
    pub avg: Option<Duration>,
    pub max: Option<Duration>,
    /// Mean absolute difference between consecutive replies' round-trip times,
    /// skipping lost requests. Needs two replies.
    pub jitter: Option<Duration>,
}

pub(crate) fn summarize(rtts: &[Option<Duration>]) -> Summary {
    let received: Vec<Duration> = rtts.iter().flatten().copied().collect();
    let sent = rtts.len();
    let loss = if sent == 0 {
        0.0
    } else {
        (sent - received.len()) as f64 / sent as f64
    };
    let avg =
        (!received.is_empty()).then(|| received.iter().sum::<Duration>() / received.len() as u32);
    let jitter = (received.len() >= 2).then(|| {
        let diffs: Duration = received.windows(2).map(|w| w[1].abs_diff(w[0])).sum();
        diffs / (received.len() - 1) as u32
    });
    Summary {
        sent,
        received: received.len(),
        loss,
        min: received.iter().min().copied(),
        avg,
        max: received.iter().max().copied(),
        jitter,
    }
}

/// One gauge row per metric. RTT, jitter and TTL rows are left out, never
/// zeroed, when there is nothing to measure them on.
pub(crate) fn build_rows(
    summary: &Summary,
    ttl: Option<u8>,
    ip: IpAddr,
    time: DateTime<Utc>,
) -> Vec<MetricRowOwned> {
    let row = |name, description, unit, value| gauge(name, description, unit, value, ip, time);
    let seconds = |d: Duration| NumberValue::Float(d.as_secs_f64());
    let mut rows = vec![
        row(
            "icmp.reachable",
            "1 when at least one echo reply came back, else 0",
            "1",
            NumberValue::Int(i64::from(summary.received > 0)),
        ),
        row(
            "icmp.packet_loss",
            "Unanswered echo requests over sent ones",
            "1",
            NumberValue::Float(summary.loss),
        ),
    ];
    for (name, description, value) in [
        ("icmp.rtt.min", "Shortest round-trip time", summary.min),
        ("icmp.rtt.avg", "Mean round-trip time", summary.avg),
        ("icmp.rtt.max", "Longest round-trip time", summary.max),
        (
            "icmp.rtt.jitter",
            "Mean difference between consecutive round-trip times",
            summary.jitter,
        ),
    ] {
        if let Some(d) = value {
            rows.push(row(name, description, "s", seconds(d)));
        }
    }
    if let Some(ttl) = ttl {
        rows.push(row(
            "icmp.ttl",
            "IP TTL of the last echo reply",
            "{hop}",
            NumberValue::Int(i64::from(ttl)),
        ));
    }
    rows
}

fn gauge(
    name: &'static str,
    description: &'static str,
    unit: &'static str,
    value: NumberValue,
    ip: IpAddr,
    time: DateTime<Utc>,
) -> MetricRowOwned {
    MetricRowOwned {
        name: ZStringOwned::SmolStr(SmolStr::new_static(name)),
        r#type: MetricType::Gauge,
        description: Some(ZStringOwned::SmolStr(SmolStr::new_static(description))),
        unit: Some(ZStringOwned::SmolStr(SmolStr::new_static(unit))),
        source_ip: Some(ip),
        source_port: None,
        data_points: DataPointsOwned::Gauge(GaugeOwned {
            data_points: vec![NumberDataPointOwned {
                start_time: None,
                time,
                value,
                flags: 0,
                attrs: Vec::new(),
                exemplars: Vec::new(),
            }],
        }),
        attrs: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Option<Duration> {
        Some(Duration::from_millis(n))
    }

    #[test]
    fn every_reply_received() {
        let s = summarize(&[ms(10), ms(14), ms(12)]);
        assert_eq!(s.sent, 3);
        assert_eq!(s.received, 3);
        assert_eq!(s.loss, 0.0);
        assert_eq!(s.min, ms(10));
        assert_eq!(s.avg, ms(12));
        assert_eq!(s.max, ms(14));
        // |14 - 10| and |12 - 14|
        assert_eq!(s.jitter, ms(3));
    }

    #[test]
    fn jitter_skips_lost_requests() {
        let s = summarize(&[ms(10), None, ms(20), None]);
        assert_eq!(s.received, 2);
        assert_eq!(s.loss, 0.5);
        assert_eq!(s.avg, ms(15));
        assert_eq!(s.jitter, ms(10), "10 and 20 are consecutive replies");
    }

    #[test]
    fn one_reply_has_no_jitter() {
        let s = summarize(&[None, ms(7), None]);
        assert_eq!(s.received, 1);
        assert!((s.loss - 2.0 / 3.0).abs() < f64::EPSILON);
        assert_eq!((s.min, s.avg, s.max), (ms(7), ms(7), ms(7)));
        assert_eq!(s.jitter, None);
    }

    #[test]
    fn no_reply_measures_nothing() {
        let s = summarize(&[None, None]);
        assert_eq!(s.received, 0);
        assert_eq!(s.loss, 1.0);
        assert_eq!((s.min, s.avg, s.max, s.jitter), (None, None, None, None));
    }

    fn names(rows: &[MetricRowOwned]) -> Vec<&str> {
        rows.iter().map(|r| r.name.as_ref()).collect()
    }

    fn value(rows: &[MetricRowOwned], name: &str) -> NumberValue {
        let row = rows.iter().find(|r| r.name.as_ref() == name).unwrap();
        let DataPointsOwned::Gauge(g) = &row.data_points else {
            panic!("{name} is a gauge");
        };
        g.data_points[0].value
    }

    #[test]
    fn rows_for_a_reachable_host() {
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        let rows = build_rows(&summarize(&[ms(10), ms(20)]), Some(57), ip, Utc::now());
        assert_eq!(
            names(&rows),
            [
                "icmp.reachable",
                "icmp.packet_loss",
                "icmp.rtt.min",
                "icmp.rtt.avg",
                "icmp.rtt.max",
                "icmp.rtt.jitter",
                "icmp.ttl",
            ]
        );
        for row in &rows {
            assert_eq!(row.r#type, MetricType::Gauge, "{}", row.name.as_ref());
            assert!(matches!(row.data_points, DataPointsOwned::Gauge(_)));
            assert_eq!(row.source_ip, Some(ip));
            assert!(row.attrs.is_empty());
        }
        assert!(matches!(
            value(&rows, "icmp.reachable"),
            NumberValue::Int(1)
        ));
        assert!(matches!(value(&rows, "icmp.packet_loss"), NumberValue::Float(l) if l == 0.0));
        assert!(matches!(value(&rows, "icmp.rtt.avg"), NumberValue::Float(s) if s == 0.015));
        assert!(matches!(value(&rows, "icmp.ttl"), NumberValue::Int(57)));
    }

    #[test]
    fn rows_for_an_unreachable_host_leave_out_what_was_not_measured() {
        let ip: IpAddr = "2001:db8::1".parse().unwrap();
        let rows = build_rows(&summarize(&[None, None, None]), None, ip, Utc::now());
        assert_eq!(names(&rows), ["icmp.reachable", "icmp.packet_loss"]);
        assert!(matches!(
            value(&rows, "icmp.reachable"),
            NumberValue::Int(0)
        ));
        assert!(matches!(value(&rows, "icmp.packet_loss"), NumberValue::Float(l) if l == 1.0));
    }

    #[test]
    fn a_single_reply_has_no_jitter_row() {
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        let rows = build_rows(&summarize(&[ms(5)]), None, ip, Utc::now());
        assert!(!names(&rows).contains(&"icmp.rtt.jitter"));
        assert!(!names(&rows).contains(&"icmp.ttl"));
        assert!(names(&rows).contains(&"icmp.rtt.max"));
    }
}
