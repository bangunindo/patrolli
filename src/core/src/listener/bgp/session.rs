// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The listener-wide [`Collector`] and one BMP connection, applied to the
//! route table in stream order.
//!
//! Only Loc-RIB peers (RFC 9069) are read: the router's selected routes, one
//! per prefix, which is what flow enrichment needs. Adj-RIB-In and Adj-RIB-Out
//! messages are counted and skipped without decoding their UPDATE.

use super::codec::{BmpFrames, DEFAULT_MAX_MESSAGE};
use super::events::{Event, EventKind, EventSink};
use super::peer::BgpConfig;
use super::routers::{Protocol, Routers};
use super::routes::{RouteWriter, TableCheck};
use super::stats::Stats;
use super::wire::bmp::{self, Message, PeerHeader};
use super::wire::open::{Open, sends_path_ids};
use super::wire::update::Update;
use super::wire::{Afi, BGP_UPDATE, Reader, bgp_message};
use crate::cache::lookup_table::LookupTable;
use crate::cache::lookup_table::bgp::RouteTable;
use crate::config::lookup_table as cfg;
use futures::StreamExt;
use rustc_hash::{FxHashMap, FxHashSet};
use smol_str::SmolStr;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncRead;
use tokio::time::Instant;
use tokio_util::codec::FramedRead;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use uuid::Uuid;

/// The route table is compared with the registry after this many messages,
/// or this long, whichever comes first.
pub(crate) const TABLE_CHECK_MESSAGES: u32 = 256;
pub(crate) const TABLE_CHECK_INTERVAL: Duration = Duration::from_millis(100);

/// The BMP section of a listener.
#[derive(Debug, Clone)]
pub struct BmpConfig {
    /// how long a lost session's routes stay
    pub keep: Duration,
    pub max_message_size: usize,
}

impl Default for BmpConfig {
    fn default() -> Self {
        Self {
            keep: Duration::from_secs(300),
            max_message_size: DEFAULT_MAX_MESSAGE,
        }
    }
}

/// What one listener collects, and how.
#[derive(Debug, Clone, Default)]
pub struct CollectorConfig {
    pub bmp: Option<BmpConfig>,
    pub bgp: Option<BgpConfig>,
    /// the VRFs kept; `None` keeps them all
    pub vrfs: Option<FxHashSet<SmolStr>>,
}

impl From<BmpConfig> for CollectorConfig {
    fn from(bmp: BmpConfig) -> Self {
        Self::bmp(bmp)
    }
}

impl CollectorConfig {
    /// A BMP-only listener.
    pub fn bmp(bmp: BmpConfig) -> Self {
        Self {
            bmp: Some(bmp),
            ..Self::default()
        }
    }

    pub fn keeps_vrf(&self, vrf: &str) -> bool {
        self.vrfs.as_ref().is_none_or(|kept| kept.contains(vrf))
    }

    /// The protocols configured.
    pub fn protocols(&self) -> Vec<Protocol> {
        let mut out = Vec::with_capacity(2);
        if self.bmp.is_some() {
            out.push(Protocol::Bmp);
        }
        if self.bgp.is_some() {
            out.push(Protocol::Bgp);
        }
        out
    }

    pub fn keep(&self, protocol: Protocol) -> Duration {
        let keep = match protocol {
            Protocol::Bmp => self.bmp.as_ref().map(|b| b.keep),
            Protocol::Bgp => self.bgp.as_ref().map(|b| b.keep),
        };
        keep.unwrap_or(BmpConfig::default().keep)
    }

    /// The longest `keep` configured: routes a previous collector left carry
    /// no protocol.
    fn longest_keep(&self) -> Duration {
        Protocol::ALL
            .iter()
            .filter(|p| self.protocols().contains(p))
            .map(|p| self.keep(*p))
            .max()
            .unwrap_or(BmpConfig::default().keep)
    }
}

/// Why a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEnd {
    /// the router closed the connection
    Eof,
    /// the router sent Termination (BMP)
    Terminated,
    /// the listener stopped, or a new session from the router replaced this one
    Cancelled,
    /// the first message was not Initiation (BMP)
    NoInitiation,
    /// the byte stream cannot be framed any further
    Frame(&'static str),
    /// a message the table depends on could not be read (RFC 7606 "session
    /// reset"): reconnecting makes the router send everything again
    Reset(&'static str),
    /// the route table was created, rebuilt or removed: only a new dump can
    /// fill it
    Resync,
    /// the router sent a NOTIFICATION (BGP)
    Notification,
    /// nothing came from the router for the hold time (BGP)
    HoldTimerExpired,
    /// the router's OPEN was refused (BGP)
    OpenRejected(&'static str),
    /// the router was live over the other protocol
    Refused,
}

impl SessionEnd {
    pub const KINDS: usize = 11;

    pub fn index(&self) -> usize {
        match self {
            Self::Eof => 0,
            Self::Terminated => 1,
            Self::Cancelled => 2,
            Self::NoInitiation => 3,
            Self::Frame(_) => 4,
            Self::Reset(_) => 5,
            Self::Resync => 6,
            Self::Notification => 7,
            Self::HoldTimerExpired => 8,
            Self::OpenRejected(_) => 9,
            Self::Refused => 10,
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
        "notification",
        "hold_timer_expired",
        "open_rejected",
        "refused",
    ];

    /// The [`Self::index`]es a session of `protocol` can end with.
    pub fn reasons(protocol: Protocol) -> &'static [usize] {
        match protocol {
            Protocol::Bmp => &[0, 1, 2, 3, 4, 5, 6, 10],
            Protocol::Bgp => &[0, 2, 4, 5, 6, 7, 8, 9, 10],
        }
    }

    pub fn as_str(&self) -> &'static str {
        Self::NAMES[self.index()]
    }

    /// The decode error behind a frame, reset or rejected-OPEN end.
    pub fn error(&self) -> Option<&'static str> {
        match self {
            Self::Frame(why) | Self::Reset(why) | Self::OpenRejected(why) => Some(why),
            _ => None,
        }
    }

    /// Ended by input the router should not have sent.
    pub fn is_fault(&self) -> bool {
        matches!(
            self,
            Self::Frame(_)
                | Self::Reset(_)
                | Self::NoInitiation
                | Self::OpenRejected(_)
                | Self::HoldTimerExpired
        )
    }
}

/// One listener's routers, counters and settings, shared by every BMP and
/// BGP session.
#[derive(Debug)]
pub struct Collector {
    listener: Uuid,
    tables: Arc<LookupTable>,
    config: CollectorConfig,
    stats: Arc<Stats>,
    routers: Routers,
    events: Option<EventSink>,
}

impl Collector {
    /// A collector for `listener`. Routes a previous collector left in the
    /// table go at `keep` from now unless their router comes back for them.
    pub fn new(listener: Uuid, tables: Arc<LookupTable>, config: CollectorConfig) -> Self {
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
                debug!(partitions = partitions.len(), "routes left to expire");
            }
            let keep = c.config.longest_keep();
            for (router, vrf) in partitions {
                c.routers.seed(router, vrf, keep);
            }
        }
        c
    }

    /// Counts into `stats`, which may outlive this collector.
    pub fn with_stats(mut self, stats: Arc<Stats>) -> Self {
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

    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    pub fn routers(&self) -> &Routers {
        &self.routers
    }

    pub fn config(&self) -> &CollectorConfig {
        &self.config
    }

    pub fn listener(&self) -> Uuid {
        self.listener
    }

    pub fn tables(&self) -> &Arc<LookupTable> {
        &self.tables
    }

    /// The table this listener writes, if one is configured.
    pub fn route_table(&self) -> Option<RouteTable> {
        self.tables.route_table_for_listener(&self.listener)
    }

    /// Counts a session its router refused: live over `live`.
    pub(super) fn refused(&self, protocol: Protocol, live: Protocol) {
        Stats::incr(&self.stats.sessions_refused[protocol.index()]);
        self.stats.closed(protocol, &SessionEnd::Refused);
        warn!(
            protocol = protocol.as_str(),
            live = live.as_str(),
            "session refused, router live over another protocol"
        );
    }

    /// Counts and logs how a session ended.
    pub(super) fn ended(&self, protocol: Protocol, end: &SessionEnd) {
        self.stats.closed(protocol, end);
        if end.is_fault() {
            match end.error() {
                Some(why) => warn!(
                    protocol = protocol.as_str(),
                    reason = end.as_str(),
                    error = why,
                    "session reset"
                ),
                None => warn!(
                    protocol = protocol.as_str(),
                    reason = end.as_str(),
                    "session reset"
                ),
            }
        } else {
            debug!(
                protocol = protocol.as_str(),
                reason = end.as_str(),
                "session ended"
            );
        }
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
        let bmp = self.config.bmp.clone().unwrap_or_default();
        let attached = self
            .routers
            .attach(router, Protocol::Bmp, cancel.clone(), bmp.keep)
            .await;
        let (attachment, superseded) = match attached {
            Ok(attached) => attached,
            Err(refused) => {
                self.refused(Protocol::Bmp, refused.live);
                return SessionEnd::Refused;
            }
        };
        Stats::incr(&self.stats.sessions_opened[Protocol::Bmp.index()]);
        if superseded {
            Stats::incr(&self.stats.sessions_superseded[Protocol::Bmp.index()]);
        }
        debug!(superseded, "bmp session started");

        let mut session = Session::new(self, RouteWriter::new(self, &attachment));
        session.w.emit(EventKind::SessionUp { superseded });
        let end = session.run(stream, &bmp, &cancel).await;
        session.w.emit(EventKind::SessionDown {
            reason: end.as_str(),
            error: end.error(),
            notification: None,
            message: None,
        });
        session.w.flush_events(&cancel).await;
        drop(session);
        self.routers.detach(attachment, bmp.keep);
        self.ended(Protocol::Bmp, &end);
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
            Stats::incr(&self.stats.keep_expiries);
            let mut rows = 0;
            if let Some(table) = &table {
                for vrf in &e.vrfs {
                    rows += table.sweep(e.router, vrf, None, e.cutoff);
                }
            }
            swept += rows;
            if let Some(events) = &self.events {
                let mut event = Event::new(e.router, EventKind::KeepExpired { rows: rows as u64 });
                event.protocol = e.protocol;
                events.try_send(event);
            }
        }
        Stats::add(&self.stats.rows_swept, swept as u64);
        debug!(routers = expired.len(), rows = swept, "keep expired");
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

struct Session<'c> {
    c: &'c Collector,
    w: RouteWriter<'c>,
    peers: FxHashMap<PeerKey, Peer>,
}

impl<'c> Session<'c> {
    fn new(c: &'c Collector, w: RouteWriter<'c>) -> Self {
        Self {
            c,
            w,
            peers: FxHashMap::default(),
        }
    }

    async fn run<S: AsyncRead + Unpin>(
        &mut self,
        stream: S,
        bmp: &BmpConfig,
        cancel: &CancellationToken,
    ) -> SessionEnd {
        let mut frames = FramedRead::new(stream, BmpFrames::new(bmp.max_message_size));
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
            Stats::add(&self.c.stats.bytes, frame.len() as u64);

            since_check += 1;
            if since_check >= TABLE_CHECK_MESSAGES || last_check.elapsed() >= TABLE_CHECK_INTERVAL {
                since_check = 0;
                last_check = Instant::now();
                // only a reconnect makes a BMP router dump again
                if self.w.check_table() != TableCheck::Current {
                    return SessionEnd::Resync;
                }
            }

            let applied = self.apply(&frame, &mut initiated);
            self.w.flush_events(cancel).await;
            if let Err(end) = applied {
                return end;
            }
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
                Stats::incr(&self.c.stats.messages_skipped);
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
                    Stats::incr(&self.c.stats.messages_skipped);
                }
                Ok(())
            }
            Message::PeerUp { .. }
            | Message::PeerDown { .. }
            | Message::RouteMonitoring { .. }
            | Message::StatisticsReport { .. }
            | Message::RouteMirroring { .. } => {
                Stats::incr(&self.c.stats.adj_rib_messages);
                Ok(())
            }
            Message::Other { .. } => Ok(()),
        }
    }

    fn peer_up(&mut self, header: &PeerHeader, received_open: &[u8]) -> Result<(), SessionEnd> {
        let open = Open::parse(received_open).map_err(|e| SessionEnd::Reset(e.0))?;
        let config = &self.c.config;
        let peer = self.peers.entry(key(header)).or_insert_with(|| {
            let vrf = bmp::distinguisher_text(&header.distinguisher)
                .map_or(SmolStr::new_static(cfg::bgp::GLOBAL_VRF), SmolStr::from);
            Peer {
                kept: config.keeps_vrf(&vrf),
                vrf,
                add_path: [false; 2],
            }
        });
        if !peer.kept {
            Stats::incr(&self.c.stats.vrf_filtered);
            return Ok(());
        }
        // one emulated peer per family adds to what the others announced
        for family in Afi::ALL {
            peer.add_path[family.index()] |= sends_path_ids(&open, family);
        }
        let vrf = peer.vrf.clone();
        self.c.routers.note_vrf(self.w.router, &vrf);
        self.w.emit(EventKind::PeerUp {
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
            Stats::incr(&self.c.stats.vrf_filtered);
            return;
        }
        let removed = self.w.peer_down(&peer.vrf);
        self.w.emit(EventKind::PeerDown {
            vrf: peer.vrf,
            asn: header.asn,
            bgp_id: header.bgp_id,
            rows: removed,
        });
    }

    fn route_monitoring(&mut self, header: &PeerHeader, bgp: &[u8]) -> Result<(), SessionEnd> {
        let Some(peer) = self.peers.get(&key(header)) else {
            // without its Peer Up the ADD-PATH state is unknown, and guessing
            // it wrong misreads every prefix
            Stats::incr(&self.c.stats.without_peer_up);
            return Ok(());
        };
        if !peer.kept {
            Stats::incr(&self.c.stats.vrf_filtered);
            return Ok(());
        }
        let (vrf, add_path) = (peer.vrf.clone(), peer.add_path);
        let mut r = Reader::new(bgp);
        let (kind, body) = bgp_message(&mut r).map_err(|e| SessionEnd::Reset(e.0))?;
        if kind != BGP_UPDATE {
            return Err(SessionEnd::Reset("route monitoring without an update"));
        }
        let update = Update::parse(body, header.asn4(), add_path)
            .map_err(|e| SessionEnd::Reset(e.reason))?;
        self.w.apply_update(&vrf, header.asn4(), add_path, &update);
        Ok(())
    }
}

fn key(header: &PeerHeader) -> PeerKey {
    (header.distinguisher, header.bgp_id)
}
