// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! One BMP connection, applied to the route table in stream order.
//!
//! Only Loc-RIB peers (RFC 9069) are read: the router's selected routes, one
//! per prefix, which is what flow enrichment needs. Adj-RIB-In and Adj-RIB-Out
//! messages are counted and skipped without decoding their UPDATE.

use super::codec::{BmpFrames, DEFAULT_MAX_MESSAGE};
use super::events::{Event, EventKind, EventSink};
use super::paths::{Paths, Withdrawn};
use super::routers::{Attachment, Routers};
use super::stats::BmpStats;
use super::wire::bmp::{self, Message, PeerHeader};
use super::wire::open::{Open, sends_path_ids};
use super::wire::update::{AsPath, SegmentKind, Update, prefixes};
use super::wire::{Afi, BGP_UPDATE, Reader, bgp_message};
use crate::cache::lookup_table::LookupTable;
use crate::cache::lookup_table::bgp::{Family, PathSegment, RouteRow, RouteTable, RouteText};
use crate::cache::lookup_table::cell::CellRef;
use crate::cache::lookup_table::mutable::Upsert;
use crate::config::lookup_table as cfg;
use futures::StreamExt;
use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;
use smol_str::SmolStr;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncRead;
use tokio::time::Instant;
use tokio_util::codec::FramedRead;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};
use uuid::Uuid;

/// The route table is compared with the registry after this many messages,
/// or this long, whichever comes first.
pub(crate) const TABLE_CHECK_MESSAGES: u32 = 256;
const TABLE_CHECK_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct BmpConfig {
    /// how long a lost session's routes stay
    pub keep: Duration,
    pub max_message_size: usize,
    /// the VRFs kept; `None` keeps them all
    pub vrfs: Option<FxHashSet<SmolStr>>,
}

impl Default for BmpConfig {
    fn default() -> Self {
        Self {
            keep: Duration::from_secs(300),
            max_message_size: DEFAULT_MAX_MESSAGE,
            vrfs: None,
        }
    }
}

/// Why a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEnd {
    /// the router closed the connection
    Eof,
    /// the router sent Termination
    Terminated,
    /// the listener stopped, or a new session from the router replaced this one
    Cancelled,
    /// the first message was not Initiation
    NoInitiation,
    /// the byte stream cannot be framed any further
    Frame(&'static str),
    /// a message the table depends on could not be read (RFC 7606 "session
    /// reset"): reconnecting makes the router send everything again
    Reset(&'static str),
    /// the route table was created, rebuilt or removed: only a new dump can
    /// fill it
    Resync,
}

impl SessionEnd {
    pub const KINDS: usize = 7;

    pub fn index(&self) -> usize {
        match self {
            Self::Eof => 0,
            Self::Terminated => 1,
            Self::Cancelled => 2,
            Self::NoInitiation => 3,
            Self::Frame(_) => 4,
            Self::Reset(_) => 5,
            Self::Resync => 6,
        }
    }

    /// [`Self::as_str`] of each kind, by [`Self::index`].
    pub const NAMES: [&str; Self::KINDS] = [
        "eof",
        "terminated",
        "cancelled",
        "no_initiation",
        "frame",
        "reset",
        "resync",
    ];

    pub fn as_str(&self) -> &'static str {
        Self::NAMES[self.index()]
    }

    /// The decode error behind a frame or reset end.
    pub fn error(&self) -> Option<&'static str> {
        match self {
            Self::Frame(why) | Self::Reset(why) => Some(why),
            _ => None,
        }
    }
}

/// The BMP side of one listener: its routers, counters and settings, shared
/// by every session.
#[derive(Debug)]
pub struct Collector {
    listener: Uuid,
    tables: Arc<LookupTable>,
    config: BmpConfig,
    stats: Arc<BmpStats>,
    routers: Routers,
    events: Option<EventSink>,
}

impl Collector {
    /// A collector for `listener`. Routes a previous collector left in the
    /// table go at `keep` from now unless their router comes back for them.
    pub fn new(listener: Uuid, tables: Arc<LookupTable>, config: BmpConfig) -> Self {
        let c = Self {
            listener,
            tables,
            config,
            stats: Arc::default(),
            routers: Routers::default(),
            events: None,
        };
        if let Some(table) = c.route_table() {
            let partitions = table.partitions();
            if !partitions.is_empty() {
                debug!(partitions = partitions.len(), "bmp routes left to expire");
            }
            for (router, vrf) in partitions {
                c.routers.seed(router, vrf, c.config.keep);
            }
        }
        c
    }

    /// Counts into `stats`, which may outlive this collector.
    pub fn with_stats(mut self, stats: Arc<BmpStats>) -> Self {
        self.stats = stats;
        self
    }

    /// Sends the sessions' events to `sink`.
    pub fn with_events(mut self, sink: EventSink) -> Self {
        self.events = Some(sink);
        self
    }

    pub fn events(&self) -> Option<&EventSink> {
        self.events.as_ref()
    }

    pub fn stats(&self) -> &BmpStats {
        &self.stats
    }

    pub fn routers(&self) -> &Routers {
        &self.routers
    }

    /// The table this listener writes, if one is configured.
    pub fn route_table(&self) -> Option<RouteTable> {
        self.tables.route_table_for_listener(&self.listener)
    }

    /// Reads one BMP connection from `router` until it ends.
    #[tracing::instrument(
        name = "bmp_session",
        skip_all,
        fields(listener_id = %self.listener, router = %router)
    )]
    pub async fn session<S: AsyncRead + Unpin>(
        &self,
        stream: S,
        router: IpAddr,
        cancel: CancellationToken,
    ) -> SessionEnd {
        let (attachment, superseded) = self
            .routers
            .attach(router, cancel.clone(), self.config.keep)
            .await;
        BmpStats::incr(&self.stats.sessions_opened);
        if superseded {
            BmpStats::incr(&self.stats.sessions_superseded);
        }
        debug!(superseded, "bmp session started");

        let mut session = Session::new(self, &attachment);
        session.emit(EventKind::SessionUp { superseded });
        let end = session.run(stream, &cancel).await;
        session.emit(EventKind::SessionDown {
            reason: end.as_str(),
            error: end.error(),
        });
        session.flush_events(&cancel).await;
        drop(session);
        self.routers.detach(attachment, self.config.keep);

        self.stats.closed(&end);
        match end {
            SessionEnd::Frame(why) | SessionEnd::Reset(why) => {
                warn!(reason = end.as_str(), error = why, "bmp session reset");
            }
            SessionEnd::NoInitiation => {
                warn!(reason = end.as_str(), "bmp session reset");
            }
            _ => debug!(reason = end.as_str(), "bmp session ended"),
        }
        end
    }

    /// Sweeps the routes of routers whose `keep` deadline passed, until
    /// `token` is cancelled.
    pub async fn run_expiry(&self, token: CancellationToken) {
        loop {
            let changed = self.routers.changed();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let next = self.routers.next_deadline();
            tokio::select! {
                _ = token.cancelled() => return,
                _ = &mut changed => {}
                _ = sleep_until(next) => {
                    self.expire(Instant::now());
                }
            }
        }
    }

    /// Sweeps what the routers past their deadline at `now` left behind;
    /// returns how many rows went.
    pub fn expire(&self, now: Instant) -> usize {
        let expired = self.routers.take_expired(now);
        if expired.is_empty() {
            return 0;
        }
        let table = self.route_table();
        let mut swept = 0;
        for e in &expired {
            BmpStats::incr(&self.stats.keep_expiries);
            let mut rows = 0;
            if let Some(table) = &table {
                for vrf in &e.vrfs {
                    rows += table.sweep(e.router, vrf, None, e.cutoff);
                }
            }
            swept += rows;
            if let Some(events) = &self.events {
                events.try_send(Event::new(
                    e.router,
                    EventKind::KeepExpired { rows: rows as u64 },
                ));
            }
        }
        BmpStats::add(&self.stats.rows_swept, swept as u64);
        debug!(routers = expired.len(), rows = swept, "bmp keep expired");
        swept
    }

    /// Ends every session for good (the listener is stopping).
    pub fn close_all(&self) {
        self.routers.close_all();
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(d).await,
        None => std::future::pending().await,
    }
}

/// A Loc-RIB instance, keyed by its distinguisher and BGP id: RFC 9069 §6.1.1
/// lets one instance appear as several emulated peers, one per family, that
/// share both.
type PeerKey = ([u8; 8], Ipv4Addr);

#[derive(Debug)]
struct Peer {
    vrf: SmolStr,
    add_path: [bool; 2],
    /// false for a VRF the listener does not keep: its messages are only
    /// counted
    kept: bool,
}

/// Buffers reused across messages, so a warm session does not allocate.
#[derive(Debug, Default)]
struct Scratch {
    path: AsPath,
    as4_path: AsPath,
    communities: Vec<u32>,
    large: Vec<[u32; 3]>,
    text: RouteText,
}

struct Session<'c> {
    c: &'c Collector,
    router: IpAddr,
    cutoff: u64,
    table: Option<RouteTable>,
    peers: FxHashMap<PeerKey, Peer>,
    paths: Paths,
    scratch: Scratch,
    warned_full: bool,
    warned_write: bool,
    /// events of the message being applied, sent once it is
    pending: Vec<Event>,
}

/// An UPDATE's prefixes in one family.
struct Nlri<'a> {
    family: Afi,
    bytes: &'a [u8],
    next_hop: Option<IpAddr>,
}

impl<'c> Session<'c> {
    fn new(c: &'c Collector, attachment: &Attachment) -> Self {
        Self {
            c,
            router: attachment.router,
            cutoff: attachment.cutoff,
            table: c.route_table(),
            peers: FxHashMap::default(),
            paths: Paths::new(attachment.router, attachment.cutoff),
            scratch: Scratch::default(),
            warned_full: false,
            warned_write: false,
            pending: Vec::new(),
        }
    }

    /// Queues an event, if the listener sends any.
    fn emit(&mut self, kind: EventKind) {
        if self.c.events.is_some() {
            self.pending.push(Event::new(self.router, kind));
        }
    }

    async fn flush_events(&mut self, cancel: &CancellationToken) {
        if let Some(events) = &self.c.events
            && !self.pending.is_empty()
        {
            events.send_all(&mut self.pending, cancel).await;
        }
    }

    async fn run<S: AsyncRead + Unpin>(
        &mut self,
        stream: S,
        cancel: &CancellationToken,
    ) -> SessionEnd {
        let mut frames = FramedRead::new(stream, BmpFrames::new(self.c.config.max_message_size));
        let mut initiated = false;
        let mut since_check = 0;
        let mut last_check = Instant::now();
        loop {
            let frame = tokio::select! {
                biased;
                _ = cancel.cancelled() => return SessionEnd::Cancelled,
                frame = frames.next() => frame,
            };
            let frame = match frame {
                None => return SessionEnd::Eof,
                Some(Err(e)) => return SessionEnd::Frame(e.kind()),
                Some(Ok(frame)) => frame,
            };
            BmpStats::add(&self.c.stats.bytes, frame.len() as u64);

            since_check += 1;
            if since_check >= TABLE_CHECK_MESSAGES || last_check.elapsed() >= TABLE_CHECK_INTERVAL {
                since_check = 0;
                last_check = Instant::now();
                if !self.table_is_current() {
                    return SessionEnd::Resync;
                }
            }

            let applied = self.apply(&frame, &mut initiated);
            self.flush_events(cancel).await;
            if let Err(end) = applied {
                return end;
            }
        }
    }

    /// Whether the registry still serves the table this session started
    /// with, or still has none.
    fn table_is_current(&self) -> bool {
        match (&self.table, self.c.route_table()) {
            (None, None) => true,
            (Some(ours), Some(now)) => ours.id() == now.id() && ours.is_current(&self.c.tables),
            _ => false,
        }
    }

    /// Applies one framed message. `Err` ends the session.
    fn apply(&mut self, frame: &[u8], initiated: &mut bool) -> Result<(), SessionEnd> {
        let kind = frame.get(5).copied().unwrap_or(u8::MAX);
        self.c.stats.message(kind);
        let message = match Message::parse(frame) {
            Ok(m) => m,
            Err(e) => {
                // a Loc-RIB message we cannot read may hide a withdrawal
                let loc_rib = frame.get(bmp::COMMON_HEADER_LEN) == Some(&bmp::PEER_LOC_RIB);
                let routes = matches!(kind, bmp::ROUTE_MONITORING | bmp::PEER_UP | bmp::PEER_DOWN);
                if loc_rib && routes || !*initiated {
                    return Err(SessionEnd::Reset(e.0));
                }
                BmpStats::incr(&self.c.stats.messages_skipped);
                return Ok(());
            }
        };
        if !*initiated {
            if !matches!(message, Message::Initiation { .. }) {
                return Err(SessionEnd::NoInitiation);
            }
            *initiated = true;
            return Ok(());
        }
        match message {
            Message::Initiation { .. } => Ok(()),
            Message::Termination { .. } => Err(SessionEnd::Terminated),
            Message::PeerUp {
                peer,
                received_open,
                ..
            } if peer.is_loc_rib() => self.peer_up(&peer, received_open),
            Message::PeerDown { peer, .. } if peer.is_loc_rib() => {
                self.peer_down(&peer);
                Ok(())
            }
            Message::RouteMonitoring { peer, bgp } if peer.is_loc_rib() => {
                self.route_monitoring(&peer, bgp)
            }
            Message::StatisticsReport { peer, stats } if peer.is_loc_rib() => {
                // read for the self-check against the table's own counts
                // (phase 3); for now only a malformed one is noted
                if bmp::loc_rib_counts(stats).is_err() {
                    BmpStats::incr(&self.c.stats.messages_skipped);
                }
                Ok(())
            }
            Message::PeerUp { .. }
            | Message::PeerDown { .. }
            | Message::RouteMonitoring { .. }
            | Message::StatisticsReport { .. }
            | Message::RouteMirroring { .. } => {
                BmpStats::incr(&self.c.stats.adj_rib_messages);
                Ok(())
            }
            Message::Other { .. } => Ok(()),
        }
    }

    fn peer_up(&mut self, header: &PeerHeader, received_open: &[u8]) -> Result<(), SessionEnd> {
        let open = Open::parse(received_open).map_err(|e| SessionEnd::Reset(e.0))?;
        let vrfs = &self.c.config.vrfs;
        let peer = self.peers.entry(key(header)).or_insert_with(|| {
            let vrf = bmp::distinguisher_text(&header.distinguisher)
                .map_or(SmolStr::new_static(cfg::bgp::GLOBAL_VRF), SmolStr::from);
            Peer {
                kept: vrfs.as_ref().is_none_or(|kept| kept.contains(&vrf)),
                vrf,
                add_path: [false; 2],
            }
        });
        if !peer.kept {
            BmpStats::incr(&self.c.stats.vrf_filtered);
            return Ok(());
        }
        // one emulated peer per family adds to what the others announced
        for family in Afi::ALL {
            peer.add_path[family.index()] |= sends_path_ids(&open, family);
        }
        let vrf = peer.vrf.clone();
        self.c.routers.note_vrf(self.router, &vrf);
        self.emit(EventKind::PeerUp {
            vrf,
            asn: header.asn,
            bgp_id: header.bgp_id,
        });
        Ok(())
    }

    fn peer_down(&mut self, header: &PeerHeader) {
        // the emulated peers of an instance share a key, so the instance goes
        // whole
        let Some(peer) = self.peers.remove(&key(header)) else {
            return;
        };
        if !peer.kept {
            BmpStats::incr(&self.c.stats.vrf_filtered);
            return;
        }
        let mut removed = 0;
        if let Some(table) = &self.table {
            removed = table.peer_down(self.router, &peer.vrf);
            BmpStats::add(&self.c.stats.routes_withdrawn, removed as u64);
        }
        self.paths.drop_vrf(&peer.vrf);
        self.emit(EventKind::PeerDown {
            vrf: peer.vrf,
            asn: header.asn,
            bgp_id: header.bgp_id,
            rows: removed as u64,
        });
    }

    fn route_monitoring(&mut self, header: &PeerHeader, bgp: &[u8]) -> Result<(), SessionEnd> {
        let Some(peer) = self.peers.get(&key(header)) else {
            // without its Peer Up the ADD-PATH state is unknown, and guessing
            // it wrong misreads every prefix
            BmpStats::incr(&self.c.stats.without_peer_up);
            return Ok(());
        };
        if !peer.kept {
            BmpStats::incr(&self.c.stats.vrf_filtered);
            return Ok(());
        }
        let (vrf, add_path) = (peer.vrf.clone(), peer.add_path);
        let mut r = Reader::new(bgp);
        let (kind, body) = bgp_message(&mut r).map_err(|e| SessionEnd::Reset(e.0))?;
        if kind != BGP_UPDATE {
            return Err(SessionEnd::Reset("route monitoring without an update"));
        }
        let update =
            Update::parse(body, header.asn4(), add_path).map_err(|e| SessionEnd::Reset(e.0))?;
        self.apply_update(&vrf, header.asn4(), add_path, &update);
        Ok(())
    }

    fn apply_update(
        &mut self,
        vrf: &SmolStr,
        asn4: bool,
        add_path: [bool; 2],
        update: &Update<'_>,
    ) {
        let Session {
            c,
            router,
            cutoff,
            table,
            paths,
            scratch: s,
            warned_full,
            warned_write,
            pending,
            ..
        } = self;
        let stats = &c.stats;
        let events = c.events.is_some();
        let route_events = c.events.as_ref().is_some_and(EventSink::routes);
        if let Some((afi, safi)) = update.end_of_rib() {
            BmpStats::incr(&stats.end_of_rib);
            if let Some(family) = Afi::unicast(afi, safi) {
                let family = cache_family(family);
                let mut swept = 0;
                if let Some(table) = table {
                    swept = table.sweep(*router, vrf, Some(family), *cutoff);
                    BmpStats::add(&stats.rows_swept, swept as u64);
                }
                paths.sweep(vrf, Some(family), *cutoff);
                if events {
                    pending.push(Event::new(
                        *router,
                        EventKind::EndOfRib {
                            vrf: vrf.clone(),
                            family,
                            rows: swept as u64,
                        },
                    ));
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
            BmpStats::incr(&stats.treat_as_withdraw);
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
                    pending.push(Event::new(
                        *router,
                        EventKind::Withdraw {
                            vrf: vrf.clone(),
                            prefix,
                            path_id,
                        },
                    ));
                }
                let Some(table) = table.as_ref() else {
                    continue;
                };
                match paths.withdraw(table, vrf, prefix, path_id) {
                    Ok(Withdrawn::Unknown) => BmpStats::incr(&stats.unknown_withdraws),
                    Ok(_) => BmpStats::incr(&stats.routes_withdrawn),
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
        if !asn4 && let Some(bytes) = attrs.as4_path {
            let _ = s.as4_path.decode(bytes, true);
            s.path.merge_as4(&s.as4_path);
        }
        s.communities.clear();
        s.communities.extend(attrs.communities());
        s.large.clear();
        s.large.extend(attrs.large_communities());
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
                    pending.push(Event::new(
                        *router,
                        EventKind::Announce {
                            prefix,
                            cells: Box::new(cells),
                        },
                    ));
                }
                let Some(table) = table.as_ref() else {
                    continue;
                };
                match paths.announce(table, vrf, &route, &mut s.text) {
                    Ok(Upsert::Rejected) => {
                        BmpStats::incr(&stats.routes_rejected);
                        if !std::mem::replace(warned_full, true) {
                            warn!(lookup_table_id = %table.id(), "route table full");
                        }
                    }
                    Ok(_) => BmpStats::incr(&stats.routes_announced),
                    Err(e) => {
                        BmpStats::incr(&stats.routes_rejected);
                        write_failed(e);
                    }
                }
            }
        }
    }
}

fn key(header: &PeerHeader) -> PeerKey {
    (header.distinguisher, header.bgp_id)
}

fn cache_family(family: Afi) -> Family {
    match family {
        Afi::V4 => Family::V4,
        Afi::V6 => Family::V6,
    }
}
