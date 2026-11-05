// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Counters of one BMP listener, shared by its sessions.

use super::session::SessionEnd;
use crate::common::internal_metrics::BgpListener;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// BMP message types 0–6, then everything else.
pub const MESSAGE_KINDS: usize = 8;

/// The metric label of each message kind.
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

#[derive(Debug, Default)]
pub struct BmpStats {
    pub sessions_opened: AtomicU64,
    /// by [`SessionEnd::index`]
    pub sessions_closed: [AtomicU64; SessionEnd::KINDS],
    /// a router reconnected while its previous session was still live
    pub sessions_superseded: AtomicU64,
    /// by BMP message type
    pub messages: [AtomicU64; MESSAGE_KINDS],
    /// a message that failed to parse but carried nothing the table needs
    pub messages_skipped: AtomicU64,
    /// Adj-RIB-In / Adj-RIB-Out messages, which are not read
    pub adj_rib_messages: AtomicU64,
    /// Loc-RIB route monitoring before its Peer Up, dropped
    pub without_peer_up: AtomicU64,
    /// Loc-RIB messages of VRFs the listener does not keep
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
    /// (AFI, SAFI)
    ignored_families: Mutex<FxHashMap<(u16, u8), u64>>,
}

impl BmpStats {
    pub fn add(counter: &AtomicU64, n: u64) {
        counter.fetch_add(n, Ordering::Relaxed);
    }

    pub fn incr(counter: &AtomicU64) {
        Self::add(counter, 1);
    }

    pub fn message(&self, kind: u8) {
        Self::incr(&self.messages[(kind as usize).min(MESSAGE_KINDS - 1)]);
    }

    pub fn closed(&self, end: &SessionEnd) {
        Self::incr(&self.sessions_closed[end.index()]);
    }

    pub fn ignored_family(&self, afi: u16, safi: u8) {
        *self.ignored_families.lock().entry((afi, safi)).or_default() += 1;
    }

    pub fn ignored_families(&self) -> FxHashMap<(u16, u8), u64> {
        self.ignored_families.lock().clone()
    }

    pub fn get(counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }

    /// Every BMP message read, whatever its type.
    pub fn total_messages(&self) -> u64 {
        self.messages.iter().map(Self::get).sum()
    }

    /// The counters as a metrics snapshot; the gauges and the event drops
    /// live elsewhere and are filled in by the caller.
    pub fn snapshot(&self) -> BgpListener {
        let get = Self::get;
        let mut ignored_families: Vec<_> = self
            .ignored_families()
            .into_iter()
            .map(|((afi, safi), n)| (afi, safi, n))
            .collect();
        ignored_families.sort_unstable();
        BgpListener {
            sessions_opened: get(&self.sessions_opened),
            sessions_superseded: get(&self.sessions_superseded),
            sessions_closed: SessionEnd::NAMES
                .iter()
                .zip(&self.sessions_closed)
                .map(|(name, n)| (*name, get(n)))
                .collect(),
            messages: MESSAGE_NAMES
                .iter()
                .zip(&self.messages)
                .map(|(name, n)| (*name, get(n)))
                .collect(),
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
            ignored_families,
            ..BgpListener::default()
        }
    }
}
