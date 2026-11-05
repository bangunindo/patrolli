// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! One passive BGP session (RFC 4271): a router connects, the two exchange
//! OPENs, and what the router announces goes into the route table in stream
//! order, as for a BMP Loc-RIB.
//!
//! patrolli waits for the router's OPEN before sending its own, so that it
//! can answer with the router's own AS (an iBGP session) when no AS is
//! configured, as pmacct does. It offers IPv4 and IPv6 unicast, 4-octet AS,
//! route refresh, extended messages, extended next hop (RFC 8950) and
//! Graceful Restart with no families (RFC 4724 §3: it only asks for
//! End-of-RIB). It never offers ADD-PATH: the table shows one path per prefix.
//!
//! The socket is split. A writer task sends the KEEPALIVEs on its own timer,
//! so they keep going while the reader waits on a full event channel
//! (`on_full: wait`): the router's UPDATEs then back up in TCP, and its hold
//! timer stays satisfied. The reader's hold timer counts only the time spent
//! waiting for the router.

use super::codec::BgpFrames;
use super::events::EventKind;
use super::routers::Protocol;
use super::routes::{RouteWriter, TableCheck};
use super::session::{Collector, SessionEnd, TABLE_CHECK_INTERVAL, TABLE_CHECK_MESSAGES};
use super::stats::Stats;
use super::wire::bgp::{
    self as msg, BGP_KEEPALIVE, BGP_NOTIFICATION, BGP_ROUTE_REFRESH, Notification, code, subcode,
};
use super::wire::open::{Open, caps, open_body};
use super::wire::update::Update;
use super::wire::{AFI_IPV4, AFI_IPV6, Afi, BGP_HEADER_LEN, BGP_OPEN, BGP_UPDATE, SAFI_UNICAST};
use crate::config::listener::BgpPeering;
use crate::config::lookup_table as cfg;
use bytes::Bytes;
use futures::StreamExt;
use rustc_hash::FxHashMap;
use smol_str::SmolStr;
use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::{Instant, Interval, MissedTickBehavior, interval_at};
use tokio_util::codec::FramedRead;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, debug};

/// How long a router has for its OPEN after connecting, and for its first
/// KEEPALIVE when no hold time was agreed (RFC 4271 §8.2.2 suggests 4
/// minutes).
pub const OPEN_WAIT: Duration = Duration::from_secs(240);
/// How long the writer gets to send the last NOTIFICATION once the session
/// is over.
const WRITER_CLOSE: Duration = Duration::from_secs(1);
/// Messages queued for the writer. A session sends a handful in its life.
const OUTBOX: usize = 16;

/// The BGP section of a listener.
#[derive(Debug, Clone)]
pub struct BgpConfig {
    /// how long a lost session's routes stay
    pub keep: Duration,
    /// seconds offered
    pub hold_time: u16,
    /// patrolli's AS; none answers each router with its own
    pub asn: Option<u32>,
    /// patrolli's BGP identifier; none uses the session's local IPv4
    pub router_id: Option<Ipv4Addr>,
    /// patrolli's AS toward single routers
    pub peer_asn: FxHashMap<IpAddr, u32>,
}

impl Default for BgpConfig {
    fn default() -> Self {
        Self {
            keep: Duration::from_secs(300),
            hold_time: 90,
            asn: None,
            router_id: None,
            peer_asn: FxHashMap::default(),
        }
    }
}

impl From<&BgpPeering> for BgpConfig {
    fn from(conf: &BgpPeering) -> Self {
        Self {
            keep: conf.keep,
            hold_time: conf.hold_time,
            asn: conf.asn,
            router_id: conf.router_id,
            peer_asn: conf
                .peers
                .iter()
                .filter_map(|p| Some((p.address.to_canonical(), p.asn?)))
                .collect(),
        }
    }
}

enum Out {
    Send(Vec<u8>),
    /// start (or stop) sending KEEPALIVEs at this period
    Keepalives(Option<Duration>),
}

/// The reader's side of the writer task.
struct Outbox<'c> {
    tx: mpsc::Sender<Out>,
    stats: &'c Stats,
}

impl Outbox<'_> {
    /// Queues a message. A writer that cannot keep up with a handful of
    /// messages is stuck on a dead socket, which the reader finds out too.
    fn send(&self, bytes: Vec<u8>) {
        let _ = self.tx.try_send(Out::Send(bytes));
    }

    fn keepalives(&self, every: Option<Duration>) {
        let _ = self.tx.try_send(Out::Keepalives(every));
    }

    fn notify(&self, code: u8, subcode: u8, data: &[u8]) {
        self.stats.notification_sent(code);
        self.send(msg::notification(code, subcode, data));
    }
}

async fn tick(every: &mut Option<Interval>) {
    match every {
        Some(every) => {
            every.tick().await;
        }
        None => std::future::pending().await,
    }
}

/// Writes what the reader queues, and KEEPALIVEs on their own timer, until
/// the reader is done; then closes the write side.
async fn write_loop<W: AsyncWrite + Unpin>(mut w: W, mut rx: mpsc::Receiver<Out>) {
    let keepalive = msg::keepalive();
    let mut every: Option<Interval> = None;
    loop {
        let bytes = tokio::select! {
            biased;
            out = rx.recv() => match out {
                Some(Out::Send(bytes)) => bytes,
                Some(Out::Keepalives(period)) => {
                    every = period.map(|p| {
                        let mut i = interval_at(Instant::now() + p, p);
                        i.set_missed_tick_behavior(MissedTickBehavior::Delay);
                        i
                    });
                    continue;
                }
                None => break,
            },
            _ = tick(&mut every) => keepalive.clone(),
        };
        if w.write_all(&bytes).await.is_err() {
            return;
        }
    }
    let _ = w.shutdown().await;
}

impl Collector {
    /// Runs one passive BGP connection from `router` until it ends. `local`
    /// is the connection's own address, patrolli's BGP identifier when none
    /// is configured.
    #[tracing::instrument(
        name = "bgp_session",
        skip_all,
        fields(listener_id = %self.listener(), router = %router)
    )]
    pub async fn bgp_session<S>(
        &self,
        stream: S,
        router: IpAddr,
        local: IpAddr,
        cancel: CancellationToken,
    ) -> SessionEnd
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let config = self.config().bgp.clone().unwrap_or_default();
        let (rd, wr) = tokio::io::split(stream);
        let (tx, rx) = mpsc::channel(OUTBOX);
        let mut writer = tokio::spawn(write_loop(wr, rx).in_current_span());
        let out = Outbox {
            tx,
            stats: self.stats(),
        };

        let attached = self
            .routers()
            .attach(router, Protocol::Bgp, cancel.clone(), config.keep)
            .await;
        let end = match attached {
            Err(refused) => {
                out.notify(code::CEASE, subcode::CONNECTION_REJECTED, &[]);
                self.refused(Protocol::Bgp, refused.live);
                SessionEnd::Refused
            }
            Ok((attachment, superseded)) => {
                Stats::incr(&self.stats().sessions_opened[Protocol::Bgp.index()]);
                if superseded {
                    Stats::incr(&self.stats().sessions_superseded[Protocol::Bgp.index()]);
                }
                debug!(superseded, "bgp session started");
                let mut session = PeerSession {
                    c: self,
                    config: &config,
                    w: RouteWriter::new(self, &attachment),
                    out: &out,
                    bgp_id: config.router_id.unwrap_or_else(|| local_id(local)),
                    notification: None,
                    message: None,
                };
                session.w.emit(EventKind::SessionUp { superseded });
                let end = session.run(rd, &cancel).await;
                if end == SessionEnd::Cancelled {
                    let sub = if self.routers().is_replaced(&attachment) {
                        subcode::CONNECTION_COLLISION
                    } else {
                        subcode::ADMINISTRATIVE_SHUTDOWN
                    };
                    session.notify(code::CEASE, sub, &[]);
                }
                session.w.emit(EventKind::SessionDown {
                    reason: end.as_str(),
                    error: end.error(),
                    notification: session.notification,
                    message: session.message.take(),
                });
                session.w.flush_events(&cancel).await;
                drop(session);
                self.routers().detach(attachment, config.keep);
                self.ended(Protocol::Bgp, &end);
                end
            }
        };
        // the writer sends what is queued, then closes
        drop(out);
        if tokio::time::timeout(WRITER_CLOSE, &mut writer)
            .await
            .is_err()
        {
            writer.abort();
        }
        end
    }
}

/// patrolli's BGP identifier from the connection's local address; an IPv6
/// listener has a configured one.
fn local_id(local: IpAddr) -> Ipv4Addr {
    match local.to_canonical() {
        IpAddr::V4(v4) if !v4.is_unspecified() => v4,
        IpAddr::V4(_) => Ipv4Addr::new(0, 0, 0, 1),
        IpAddr::V6(v6) => {
            let o = v6.octets();
            Ipv4Addr::new(o[12], o[13], o[14], o[15].max(1))
        }
    }
}

struct PeerSession<'c> {
    c: &'c Collector,
    config: &'c BgpConfig,
    w: RouteWriter<'c>,
    out: &'c Outbox<'c>,
    bgp_id: Ipv4Addr,
    /// the NOTIFICATION that ended the session, sent or received
    notification: Option<(u8, u8)>,
    /// the router's shutdown communication
    message: Option<SmolStr>,
}

/// A message's type and body.
fn split(frame: &Bytes) -> (u8, &[u8]) {
    (frame[BGP_HEADER_LEN - 1], &frame[BGP_HEADER_LEN..])
}

impl PeerSession<'_> {
    fn notify(&mut self, code: u8, subcode: u8, data: &[u8]) {
        self.out.notify(code, subcode, data);
        self.notification = Some((code, subcode));
    }

    fn received_notification(&mut self, body: &[u8]) -> SessionEnd {
        if let Ok(n) = Notification::parse(body) {
            self.c.stats().notification_received(n.code);
            self.notification = Some((n.code, n.subcode));
            self.message = n.shutdown_communication().map(SmolStr::from);
        }
        SessionEnd::Notification
    }

    /// The next message, waiting at most `hold` for it.
    async fn next<R: AsyncRead + Unpin>(
        &mut self,
        frames: &mut FramedRead<R, BgpFrames>,
        cancel: &CancellationToken,
        hold: Option<Duration>,
    ) -> Result<Bytes, SessionEnd> {
        let expiry = async {
            match hold {
                Some(hold) => tokio::time::sleep(hold).await,
                None => std::future::pending().await,
            }
        };
        let frame = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(SessionEnd::Cancelled),
            frame = frames.next() => frame,
            _ = expiry => {
                self.notify(code::HOLD_TIMER_EXPIRED, 0, &[]);
                return Err(SessionEnd::HoldTimerExpired);
            }
        };
        match frame {
            None => Err(SessionEnd::Eof),
            Some(Err(e)) => {
                if let Some((code, subcode, data)) = e.notification() {
                    self.notify(code, subcode, &data);
                }
                Err(SessionEnd::Frame(e.kind()))
            }
            Some(Ok(frame)) => {
                let stats = self.c.stats();
                Stats::add(&stats.bytes, frame.len() as u64);
                stats.bgp_message(split(&frame).0);
                Ok(frame)
            }
        }
    }

    /// Checks the router's OPEN; a refusal is the OPEN Message Error
    /// subcode, its data and why.
    fn check_open(&self, body: &[u8]) -> Result<Open, (u8, Vec<u8>, &'static str)> {
        if body.first() != Some(&4) {
            // the data is the highest version supported
            return Err((
                subcode::UNSUPPORTED_VERSION,
                vec![0, 4],
                "unsupported version",
            ));
        }
        let open = Open::parse(body).map_err(|e| (0, Vec::new(), e.0))?;
        if matches!(open.hold_time, 1 | 2) {
            return Err((
                subcode::UNACCEPTABLE_HOLD_TIME,
                Vec::new(),
                "unacceptable hold time",
            ));
        }
        if open.bgp_id.is_unspecified() || open.bgp_id == self.bgp_id {
            return Err((
                subcode::BAD_BGP_IDENTIFIER,
                Vec::new(),
                "bad bgp identifier",
            ));
        }
        Ok(open)
    }

    async fn run<R: AsyncRead + Unpin>(&mut self, rd: R, cancel: &CancellationToken) -> SessionEnd {
        let mut frames = FramedRead::new(rd, BgpFrames::default());

        // the router's OPEN first: ours may answer with its AS
        let frame = match self.next(&mut frames, cancel, Some(OPEN_WAIT)).await {
            Ok(frame) => frame,
            Err(end) => return end,
        };
        let open = match split(&frame) {
            (BGP_OPEN, body) => match self.check_open(body) {
                Ok(open) => open,
                Err((sub, data, why)) => {
                    self.notify(code::OPEN_MESSAGE, sub, &data);
                    return SessionEnd::OpenRejected(why);
                }
            },
            (BGP_NOTIFICATION, body) => return self.received_notification(body),
            _ => {
                // RFC 6608: unexpected message in OpenSent
                self.notify(code::FSM, 1, &[]);
                return SessionEnd::Reset("expected an open");
            }
        };
        let router = self.w.router;
        let my_as = (self.config.peer_asn.get(&router).copied())
            .or(self.config.asn)
            .unwrap_or(open.asn);
        let hold = match (self.config.hold_time, open.hold_time) {
            (0, _) | (_, 0) => 0,
            (ours, theirs) => ours.min(theirs),
        };
        self.out.send(msg::message(
            BGP_OPEN,
            &open_body(
                my_as,
                self.config.hold_time,
                self.bgp_id,
                &[
                    caps::multiprotocol(AFI_IPV4, SAFI_UNICAST),
                    caps::multiprotocol(AFI_IPV6, SAFI_UNICAST),
                    caps::four_octet(my_as),
                    caps::route_refresh(),
                    caps::extended_message(),
                    caps::extended_next_hop_v4_over_v6(),
                    caps::graceful_restart_receiver(),
                ],
            ),
        ));
        self.out.send(msg::keepalive());
        if open.extended_message {
            frames.decoder_mut().allow_extended();
        }
        let hold = (hold > 0).then(|| Duration::from_secs(hold.into()));
        self.out
            .keepalives(hold.map(|h| (h / 3).max(Duration::from_secs(1))));

        // OpenConfirm: the router's KEEPALIVE
        let frame = match self
            .next(&mut frames, cancel, hold.or(Some(OPEN_WAIT)))
            .await
        {
            Ok(frame) => frame,
            Err(end) => return end,
        };
        match split(&frame) {
            (BGP_KEEPALIVE, _) => {}
            (BGP_NOTIFICATION, body) => return self.received_notification(body),
            _ => {
                // RFC 6608: unexpected message in OpenConfirm
                self.notify(code::FSM, 2, &[]);
                return SessionEnd::Reset("expected a keepalive");
            }
        }

        // Established
        debug!(
            peer_as = open.asn,
            bgp_id = %open.bgp_id,
            hold_time = hold.map_or(0, |h| h.as_secs()),
            "bgp session established"
        );
        let vrf = SmolStr::new_static(cfg::bgp::GLOBAL_VRF);
        let kept = self.c.config().keeps_vrf(&vrf);
        if kept {
            self.c.routers().note_vrf(router, &vrf);
        }
        self.w.emit(EventKind::PeerUp {
            vrf: vrf.clone(),
            asn: open.asn,
            bgp_id: open.bgp_id,
        });
        self.w.flush_events(cancel).await;
        let refresh: Vec<Afi> = if open.route_refresh {
            Afi::ALL
                .into_iter()
                .filter(|f| open.families[f.index()])
                .collect()
        } else {
            Vec::new()
        };
        let asn4 = open.four_octet_as;
        let mut since_check = 0;
        let mut last_check = Instant::now();
        loop {
            let frame = match self.next(&mut frames, cancel, hold).await {
                Ok(frame) => frame,
                Err(end) => return end,
            };

            since_check += 1;
            if since_check >= TABLE_CHECK_MESSAGES || last_check.elapsed() >= TABLE_CHECK_INTERVAL {
                since_check = 0;
                last_check = Instant::now();
                match self.w.check_table() {
                    TableCheck::Current => {}
                    TableCheck::Removed => self.w.reset_table(),
                    TableCheck::Replaced if refresh.is_empty() => {
                        self.notify(code::CEASE, subcode::OTHER_CONFIGURATION_CHANGE, &[]);
                        return SessionEnd::Resync;
                    }
                    TableCheck::Replaced => {
                        // the new table is empty: what the router sends
                        // again is all it holds, so nothing is stale
                        self.w.reset_table();
                        for family in &refresh {
                            let afi = match family {
                                Afi::V4 => AFI_IPV4,
                                Afi::V6 => AFI_IPV6,
                            };
                            self.out.send(msg::route_refresh(afi, SAFI_UNICAST));
                        }
                        debug!("route table replaced, route refresh sent");
                    }
                }
            }

            match split(&frame) {
                (BGP_UPDATE, body) if kept => match Update::parse(body, asn4, [false; 2]) {
                    Ok(update) => self.w.apply_update(&vrf, asn4, [false; 2], &update),
                    Err(e) => {
                        self.notify(code::UPDATE_MESSAGE, subcode::MALFORMED_ATTRIBUTE_LIST, &[]);
                        return SessionEnd::Reset(e.0);
                    }
                },
                (BGP_UPDATE, _) => Stats::incr(&self.c.stats().vrf_filtered),
                (BGP_KEEPALIVE | BGP_ROUTE_REFRESH, _) => {}
                (BGP_NOTIFICATION, body) => return self.received_notification(body),
                (BGP_OPEN, _) => {
                    // RFC 6608: unexpected message in Established
                    self.notify(code::FSM, 3, &[]);
                    return SessionEnd::Reset("open on an established session");
                }
                (other, _) => {
                    self.notify(code::MESSAGE_HEADER, subcode::BAD_MESSAGE_TYPE, &[other]);
                    return SessionEnd::Frame("bad message type");
                }
            }
            self.w.flush_events(cancel).await;
        }
    }
}
