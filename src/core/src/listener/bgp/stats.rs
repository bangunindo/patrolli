// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Counters of one BMP listener, shared by its sessions.

use super::session::SessionEnd;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// BMP message types 0–6, then everything else.
pub const MESSAGE_KINDS: usize = 8;

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
}
