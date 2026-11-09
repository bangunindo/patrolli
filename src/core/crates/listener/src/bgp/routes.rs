// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! What a session writes: one router's UPDATEs, End-of-RIB sweeps and Peer
//! Downs, applied to the route table in stream order, and the events they
//! make. BMP and BGP sessions share it; only how they get their UPDATEs
//! differs.

use super::events::{Event, EventKind, EventSink};
use super::paths::{Paths, Withdrawn};
use super::routers::{Attachment, Protocol};
use super::session::Collector;
use super::stats::Stats;
use super::wire::Afi;
use super::wire::update::{AsPath, SegmentKind, Update, prefixes};
use patrolli::cache::lookup_table::bgp::{Family, PathSegment, RouteRow, RouteTable, RouteText};
use patrolli::cache::lookup_table::cell::CellRef;
use patrolli::cache::lookup_table::mutable::{SharedRow, Upsert};
use smallvec::SmallVec;
use smol_str::SmolStr;
use std::net::IpAddr;
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};

/// Buffers reused across messages, so a warm session does not allocate.
#[derive(Debug, Default)]
struct Scratch {
    path: AsPath,
    as4_path: AsPath,
    communities: Vec<u32>,
    large: Vec<[u32; 3]>,
    text: RouteText,
}

/// An UPDATE's prefixes in one family.
struct Nlri<'a> {
    family: Afi,
    bytes: &'a [u8],
    next_hop: Option<IpAddr>,
}

/// How the table registry stands against the table a session writes.
#[derive(Debug, PartialEq, Eq)]
pub enum TableCheck {
    /// still the same table, or still none
    Current,
    /// the table was removed
    Removed,
    /// a table was created, or the one written was rebuilt: it is empty and
    /// only a new dump fills it
    Replaced,
}

/// One session's writes to the route table.
pub struct RouteWriter<'c> {
    c: &'c Collector,
    pub router: IpAddr,
    pub protocol: Protocol,
    /// the session's start: rows older than this were not written by it
    pub cutoff: u64,
    table: Option<RouteTable>,
    paths: Paths,
    scratch: Scratch,
    warned_full: bool,
    warned_write: bool,
    /// events of the message being applied, sent once it is
    pending: Vec<Event>,
}

impl<'c> RouteWriter<'c> {
    pub fn new(c: &'c Collector, attachment: &Attachment) -> Self {
        Self {
            c,
            router: attachment.router,
            protocol: attachment.protocol,
            cutoff: attachment.cutoff,
            table: c.route_table(),
            paths: Paths::new(attachment.router, attachment.cutoff),
            scratch: Scratch::default(),
            warned_full: false,
            warned_write: false,
            pending: Vec::new(),
        }
    }

    fn stats(&self) -> &'c Stats {
        self.c.stats()
    }

    /// Queues an event, if the listener sends any.
    pub fn emit(&mut self, kind: EventKind) {
        if self.c.events().is_some() {
            self.pending
                .push(Event::new(self.router, kind).with_protocol(self.protocol));
        }
    }

    /// Sends the queued events; how long this may wait is the listener's
    /// `events.on_full`.
    pub async fn flush_events(&mut self, cancel: &CancellationToken) {
        if let Some(events) = self.c.events()
            && !self.pending.is_empty()
        {
            events.send_all(&mut self.pending, cancel).await;
        }
    }

    /// Compares the table written with the registry's.
    pub fn check_table(&self) -> TableCheck {
        match (&self.table, self.c.route_table()) {
            (None, None) => TableCheck::Current,
            (Some(ours), Some(now))
                if ours.id() == now.id() && ours.is_current(self.c.tables()) =>
            {
                TableCheck::Current
            }
            (_, None) => TableCheck::Removed,
            (_, Some(_)) => TableCheck::Replaced,
        }
    }

    /// Writes to whatever table the registry has now, forgetting what the
    /// session knew of the old one.
    pub fn reset_table(&mut self) {
        self.table = self.c.route_table();
        self.paths = Paths::new(self.router, self.cutoff);
    }

    /// Removes the routes of one VRF (a Peer Down); returns how many rows
    /// went.
    pub fn peer_down(&mut self, vrf: &SmolStr) -> u64 {
        let mut removed = 0;
        if let Some(table) = &self.table {
            removed = table.peer_down(self.router, vrf) as u64;
            Stats::add(&self.stats().routes_withdrawn, removed);
        }
        self.paths.drop_vrf(vrf);
        removed
    }

    /// Applies one UPDATE to `vrf`. Never fails: what cannot be written is
    /// counted.
    pub fn apply_update(
        &mut self,
        vrf: &SmolStr,
        asn4: bool,
        add_path: [bool; 2],
        update: &Update<'_>,
    ) {
        let RouteWriter {
            c,
            router,
            protocol,
            cutoff,
            table,
            paths,
            scratch: s,
            warned_full,
            warned_write,
            pending,
        } = self;
        let stats = c.stats();
        let events = c.events().is_some();
        let route_events = c.events().is_some_and(EventSink::routes);
        let event = |kind| Event::new(*router, kind).with_protocol(*protocol);
        if let Some((afi, safi)) = update.end_of_rib() {
            Stats::incr(&stats.end_of_rib);
            if let Some(family) = Afi::unicast(afi, safi) {
                let family = cache_family(family);
                let mut swept = 0;
                if let Some(table) = table {
                    swept = table.sweep(*router, vrf, Some(family), *cutoff);
                    Stats::add(&stats.rows_swept, swept as u64);
                }
                paths.sweep(vrf, Some(family), *cutoff);
                if events {
                    pending.push(event(EventKind::EndOfRib {
                        vrf: vrf.clone(),
                        family,
                        rows: swept as u64,
                    }));
                }
            }
            return;
        }

        let attrs = &update.attrs;
        let mut withdrawn: SmallVec<[Nlri<'_>; 2]> = SmallVec::new();
        let mut announced: SmallVec<[Nlri<'_>; 2]> = SmallVec::new();
        withdrawn.push(Nlri {
            family: Afi::V4,
            bytes: update.withdrawn,
            next_hop: None,
        });
        if let Some(u) = attrs.mp_unreach {
            match Afi::unicast(u.afi, u.safi) {
                Some(family) => withdrawn.push(Nlri {
                    family,
                    bytes: u.nlri,
                    next_hop: None,
                }),
                None => stats.ignored_family(u.afi, u.safi),
            }
        }
        if let Some(r) = attrs.mp_reach {
            match Afi::unicast(r.afi, r.safi) {
                Some(family) => announced.push(Nlri {
                    family,
                    bytes: r.nlri,
                    next_hop: r.next_hop,
                }),
                None => stats.ignored_family(r.afi, r.safi),
            }
        }
        announced.push(Nlri {
            family: Afi::V4,
            bytes: update.nlri,
            next_hop: attrs.next_hop.map(IpAddr::V4),
        });

        // RFC 7606: a malformed route attribute withdraws what it came with
        if attrs.withdraw.is_some() && announced.iter().any(|n| !n.bytes.is_empty()) {
            Stats::incr(&stats.treat_as_withdraw);
            withdrawn.extend(announced.drain(..));
        }

        // without a table the routes are only counted, or sent as events
        if table.is_none() && !route_events {
            return;
        }
        let mut write_failed = |e: anyhow::Error| {
            if !std::mem::replace(warned_write, true) {
                error!(error = %format!("{e:#}"), "route write failed");
            }
        };
        for nlri in &withdrawn {
            let add_path = add_path[nlri.family.index()];
            for (path_id, prefix) in prefixes(nlri.bytes, nlri.family, add_path).flatten() {
                if route_events {
                    pending.push(event(EventKind::Withdraw {
                        vrf: vrf.clone(),
                        prefix,
                        path_id,
                    }));
                }
                let Some(table) = table.as_ref() else {
                    continue;
                };
                match paths.withdraw(table, vrf, prefix, path_id) {
                    Ok(Withdrawn::Unknown) => Stats::incr(&stats.unknown_withdraws),
                    Ok(_) => Stats::incr(&stats.routes_withdrawn),
                    Err(e) => write_failed(e),
                }
            }
        }
        if announced.iter().all(|n| n.bytes.is_empty()) {
            return;
        }

        s.path.clear();
        if let Some(bytes) = attrs.as_path {
            // checked by the parser; a failure here would leave the path empty
            let _ = s.path.decode(bytes, asn4);
        }
        // the parser leaves an AS4_PATH only where RFC 6793 merges it
        if let Some(bytes) = attrs.as4_path {
            let _ = s.as4_path.decode(bytes, true);
            s.path.merge_as4(&s.as4_path);
        }
        s.communities.clear();
        s.communities.extend(attrs.communities());
        s.large.clear();
        s.large.extend(attrs.large_communities());
        // RFC 8092 §3: redundant values are removed; the attribute is a set
        if s.large.len() > 1 {
            s.large.sort_unstable();
            s.large.dedup();
        }
        let segments: SmallVec<[PathSegment<'_>; 4]> = s
            .path
            .segments()
            .map(|(kind, asns)| match kind {
                SegmentKind::Sequence => PathSegment::Sequence(asns),
                SegmentKind::Set => PathSegment::Set(asns),
                SegmentKind::ConfedSequence => PathSegment::ConfedSequence(asns),
                SegmentKind::ConfedSet => PathSegment::ConfedSet(asns),
            })
            .collect();

        for nlri in &announced {
            let add_path = add_path[nlri.family.index()];
            // without path ids the prefixes share every cell: one row, built
            // on the first and stored under all of them (`Err` once it failed)
            let mut shared: Option<Result<SharedRow, ()>> = None;
            for (path_id, prefix) in prefixes(nlri.bytes, nlri.family, add_path).flatten() {
                let route = RouteRow {
                    router: *router,
                    vrf,
                    prefix,
                    next_hop: nlri.next_hop,
                    as_path: &segments,
                    communities: &s.communities,
                    large_communities: &s.large,
                    local_pref: attrs.local_pref,
                    med: attrs.med,
                    path_id,
                };
                if route_events {
                    let cells = route.cells(&mut s.text).map(CellRef::to_owned_cell);
                    pending.push(event(EventKind::Announce {
                        prefix,
                        cells: Box::new(cells),
                    }));
                }
                let Some(table) = table.as_ref() else {
                    continue;
                };
                let written = if path_id.is_none() {
                    let row = shared.get_or_insert_with(|| {
                        table.row(&route, &mut s.text).map_err(&mut write_failed)
                    });
                    match row {
                        Ok(row) => Ok(table.upsert_row(row, prefix)),
                        Err(()) => {
                            Stats::incr(&stats.routes_rejected);
                            continue;
                        }
                    }
                } else {
                    paths.announce(table, vrf, &route, &mut s.text)
                };
                match written {
                    Ok(Upsert::Rejected) => {
                        Stats::incr(&stats.routes_rejected);
                        if !std::mem::replace(warned_full, true) {
                            warn!(lookup_table_id = %table.id(), "route table full");
                        }
                    }
                    Ok(_) => Stats::incr(&stats.routes_announced),
                    Err(e) => {
                        Stats::incr(&stats.routes_rejected);
                        write_failed(e);
                    }
                }
            }
        }
    }
}

pub fn cache_family(family: Afi) -> Family {
    match family {
        Afi::V4 => Family::V4,
        Afi::V6 => Family::V6,
    }
}
