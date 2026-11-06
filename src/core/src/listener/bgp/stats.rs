// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Counters of one bgp listener, shared by its BMP and BGP sessions.

use super::routers::Protocol;
use super::session::SessionEnd;
use super::wire::bgp::{CODE_NAMES, code_index};
use crate::common::internal_metrics::BgpListener;
use std::sync::atomic::{AtomicU64, Ordering};

/// BMP message types 0–6, then everything else.
pub const MESSAGE_KINDS: usize = 8;

/// The metric label of each BMP message kind.
pub const MESSAGE_NAMES: [&str; MESSAGE_KINDS] = [
    "route_monitoring",
    "statistics_report",
    "peer_down",
    "peer_up",
    "initiation",
    "termination",
    "route_mirroring",
    "other",
];

/// BGP message types 1–5, then everything else.
pub const BGP_MESSAGE_KINDS: usize = 6;

/// The metric label of each BGP message kind, by [`bgp_message_index`].
pub const BGP_MESSAGE_NAMES: [&str; BGP_MESSAGE_KINDS] = [
    "other",
    "open",
    "update",
    "notification",
    "keepalive",
    "route_refresh",
];

pub fn bgp_message_index(kind: u8) -> usize {
    match kind {
        1..=5 => kind as usize,
        _ => 0,
    }
}

type PerProtocol = [AtomicU64; 2];

/// The (AFI, SAFI) pairs not kept but counted on their own (IANA registries):
/// the families a router plausibly sends. Anything else counts as `other`, so
/// a sender cannot grow the label set past these.
pub const IGNORED_FAMILIES: [(u16, u8); 22] = [
    (1, 2),      // IPv4 multicast
    (1, 4),      // IPv4 labeled unicast
    (1, 5),      // IPv4 MCAST-VPN
    (1, 66),     // IPv4 MDT
    (1, 73),     // IPv4 SR TE policy
    (1, 128),    // VPNv4
    (1, 129),    // VPNv4 multicast
    (1, 132),    // route target constraint
    (1, 133),    // IPv4 flowspec
    (1, 134),    // VPNv4 flowspec
    (2, 2),      // IPv6 multicast
    (2, 4),      // IPv6 labeled unicast
    (2, 5),      // IPv6 MCAST-VPN
    (2, 73),     // IPv6 SR TE policy
    (2, 128),    // VPNv6
    (2, 129),    // VPNv6 multicast
    (2, 133),    // IPv6 flowspec
    (2, 134),    // VPNv6 flowspec
    (25, 65),    // VPLS
    (25, 70),    // EVPN
    (16388, 71), // BGP-LS
    (16388, 72), // BGP-LS VPN
];

/// [`IGNORED_FAMILIES`], then `other`.
const IGNORED_SLOTS: usize = IGNORED_FAMILIES.len() + 1;

fn ignored_index(afi: u16, safi: u8) -> usize {
    IGNORED_FAMILIES
        .iter()
        .position(|&f| f == (afi, safi))
        .unwrap_or(IGNORED_FAMILIES.len())
}

#[derive(Debug, Default)]
pub struct Stats {
    pub sessions_opened: PerProtocol,
    /// by protocol, then [`SessionEnd::index`]
    pub sessions_closed: [[AtomicU64; SessionEnd::KINDS]; 2],
    /// a router reconnected while its previous session was still live
    pub sessions_superseded: PerProtocol,
    /// a router connected over one protocol while live over the other
    pub sessions_refused: PerProtocol,
    /// by BMP message type
    pub messages: [AtomicU64; MESSAGE_KINDS],
    /// by [`bgp_message_index`]
    pub bgp_messages: [AtomicU64; BGP_MESSAGE_KINDS],
    /// NOTIFICATIONs sent and received, by [`code_index`]
    pub notifications_sent: [AtomicU64; CODE_NAMES.len()],
    pub notifications_received: [AtomicU64; CODE_NAMES.len()],
    /// a message that failed to parse but carried nothing the table needs
    pub messages_skipped: AtomicU64,
    /// Adj-RIB-In / Adj-RIB-Out messages, which are not read
    pub adj_rib_messages: AtomicU64,
    /// Loc-RIB route monitoring before its Peer Up, dropped
    pub without_peer_up: AtomicU64,
    /// messages of VRFs the listener does not keep
    pub vrf_filtered: AtomicU64,
    /// bytes of the messages read
    pub bytes: AtomicU64,
    pub routes_announced: AtomicU64,
    pub routes_withdrawn: AtomicU64,
    /// writes refused at the table's `max_rows`
    pub routes_rejected: AtomicU64,
    /// withdrawals of a prefix or path the table does not hold
    pub unknown_withdraws: AtomicU64,
    /// messages whose malformed attributes withdrew their routes (RFC 7606)
    pub treat_as_withdraw: AtomicU64,
    pub end_of_rib: AtomicU64,
    /// rows removed by End-of-RIB and `keep` sweeps
    pub rows_swept: AtomicU64,
    /// routers whose `keep` deadline passed
    pub keep_expiries: AtomicU64,
    /// MP_REACH/MP_UNREACH attributes of families that are not kept, by
    /// [`IGNORED_FAMILIES`] index, the last slot for any other family
    ignored_families: [AtomicU64; IGNORED_SLOTS],
}

impl Stats {
    pub fn add(counter: &AtomicU64, n: u64) {
        counter.fetch_add(n, Ordering::Relaxed);
    }

    pub fn incr(counter: &AtomicU64) {
        Self::add(counter, 1);
    }

    pub fn message(&self, kind: u8) {
        Self::incr(&self.messages[(kind as usize).min(MESSAGE_KINDS - 1)]);
    }

    pub fn bgp_message(&self, kind: u8) {
        Self::incr(&self.bgp_messages[bgp_message_index(kind)]);
    }

    pub fn notification_sent(&self, code: u8) {
        Self::incr(&self.notifications_sent[code_index(code)]);
    }

    pub fn notification_received(&self, code: u8) {
        Self::incr(&self.notifications_received[code_index(code)]);
    }

    pub fn closed(&self, protocol: Protocol, end: &SessionEnd) {
        Self::incr(&self.sessions_closed[protocol.index()][end.index()]);
    }

    pub fn ignored_family(&self, afi: u16, safi: u8) {
        Self::incr(&self.ignored_families[ignored_index(afi, safi)]);
    }

    /// The families seen so far, `None` for the `other` bucket.
    pub fn ignored_families(&self) -> Vec<(Option<(u16, u8)>, u64)> {
        let families = IGNORED_FAMILIES.iter().copied().map(Some).chain([None]);
        families
            .zip(&self.ignored_families)
            .map(|(family, n)| (family, Self::get(n)))
            .filter(|&(_, n)| n > 0)
            .collect()
    }

    pub fn get(counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }

    /// Every BMP and BGP message read, whatever its type.
    pub fn total_messages(&self) -> u64 {
        self.messages
            .iter()
            .chain(&self.bgp_messages)
            .map(Self::get)
            .sum()
    }

    /// The counters as a metrics snapshot, with the per-protocol ones for
    /// `protocols` only; the gauges and the event drops live elsewhere and
    /// are filled in by the caller.
    pub fn snapshot(&self, protocols: &[Protocol]) -> BgpListener {
        let get = Self::get;
        let per = |counters: &PerProtocol| -> Vec<(&'static str, u64)> {
            protocols
                .iter()
                .map(|p| (p.as_str(), get(&counters[p.index()])))
                .collect()
        };
        let mut sessions_closed = Vec::new();
        let mut messages = Vec::new();
        for p in protocols {
            for end in SessionEnd::reasons(*p) {
                sessions_closed.push((
                    p.as_str(),
                    SessionEnd::NAMES[*end],
                    get(&self.sessions_closed[p.index()][*end]),
                ));
            }
            match p {
                Protocol::Bmp => messages.extend(
                    MESSAGE_NAMES
                        .iter()
                        .zip(&self.messages)
                        .map(|(name, n)| (p.as_str(), *name, get(n))),
                ),
                Protocol::Bgp => messages.extend(
                    BGP_MESSAGE_NAMES
                        .iter()
                        .zip(&self.bgp_messages)
                        .map(|(name, n)| (p.as_str(), *name, get(n))),
                ),
            }
        }
        let by_code = |counters: &[AtomicU64; CODE_NAMES.len()]| -> Vec<(&'static str, u64)> {
            if !protocols.contains(&Protocol::Bgp) {
                return Vec::new();
            }
            CODE_NAMES
                .iter()
                .zip(counters)
                .map(|(name, n)| (*name, get(n)))
                .collect()
        };
        BgpListener {
            sessions_opened: per(&self.sessions_opened),
            sessions_superseded: per(&self.sessions_superseded),
            sessions_refused: per(&self.sessions_refused),
            sessions_closed,
            messages,
            notifications_sent: by_code(&self.notifications_sent),
            notifications_received: by_code(&self.notifications_received),
            messages_skipped: get(&self.messages_skipped),
            adj_rib_messages: get(&self.adj_rib_messages),
            without_peer_up: get(&self.without_peer_up),
            vrf_filtered: get(&self.vrf_filtered),
            routes_announced: get(&self.routes_announced),
            routes_withdrawn: get(&self.routes_withdrawn),
            routes_rejected: get(&self.routes_rejected),
            unknown_withdraws: get(&self.unknown_withdraws),
            treat_as_withdraw: get(&self.treat_as_withdraw),
            end_of_rib: get(&self.end_of_rib),
            rows_swept: get(&self.rows_swept),
            keep_expiries: get(&self.keep_expiries),
            ignored_families: self.ignored_families(),
            ..BgpListener::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unnamed_families_share_one_bucket() {
        let stats = Stats::default();
        stats.ignored_family(1, 128);
        stats.ignored_family(1, 128);
        stats.ignored_family(25, 70);
        for afi in 3..1000 {
            stats.ignored_family(afi, 255);
        }
        assert_eq!(
            stats.ignored_families(),
            [(Some((1, 128)), 2), (Some((25, 70)), 1), (None, 997)]
        );
    }
}
