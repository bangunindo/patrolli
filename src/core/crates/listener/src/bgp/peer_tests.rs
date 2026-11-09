// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Passive BGP sessions over in-memory streams: the test plays the router.

use super::codec::BgpFrames;
use super::events::{Event, EventKind, EventSink};
use super::peer::BgpConfig;
use super::routers::Protocol;
use super::session::{BmpConfig, Collector, CollectorConfig, SessionEnd, TABLE_CHECK_MESSAGES};
use super::stats::Stats;
use super::wire::bgp::{self as msg, BGP_KEEPALIVE, BGP_NOTIFICATION, BGP_ROUTE_REFRESH};
use super::wire::bmp::build as b;
use super::wire::open::{Open, caps, open_body};
use super::wire::update::build as u;
use super::wire::{AFI_IPV4, AFI_IPV6, BGP_OPEN, BGP_UPDATE, SAFI_UNICAST};
use futures::StreamExt;
use patrolli::cache::lookup_table::LookupTable;
use patrolli::cache::lookup_table::bgp::RouteTable;
use patrolli::cache::lookup_table::cell::CellOwned;
use patrolli::config::listener::OnFull;
use patrolli::config::lookup_table as cfg;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};
use tokio::task::JoinHandle;
use tokio_util::codec::FramedRead;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const ROUTER: &str = "198.51.100.1";
const ROUTER_ID: Ipv4Addr = Ipv4Addr::new(198, 51, 100, 1);
/// the connection's local address: patrolli's BGP identifier by default
const LOCAL: &str = "192.0.2.10";
const LIMIT: Duration = Duration::from_secs(5);

struct Bed {
    registry: Arc<LookupTable>,
    listener: Uuid,
    table_id: Uuid,
    collector: Arc<Collector>,
}

fn table_config(listener: Uuid) -> cfg::LookupTable {
    cfg::LookupTable {
        id: Uuid::now_v7(),
        name: "routes".into(),
        source: cfg::Source::Bgp(cfg::Bgp {
            listener,
            max_rows: 1000,
        }),
        key: cfg::Bgp::key(),
    }
}

impl Bed {
    fn new(bgp: BgpConfig) -> Self {
        Self::with(
            CollectorConfig {
                bgp: Some(bgp),
                ..CollectorConfig::default()
            },
            None,
        )
    }

    fn with(config: CollectorConfig, events: Option<EventSink>) -> Self {
        let registry = Arc::new(LookupTable::new());
        let listener = Uuid::now_v7();
        let table = table_config(listener);
        registry.insert(&table).unwrap();
        let mut collector = Collector::new(listener, registry.clone(), config);
        if let Some(events) = events {
            collector = collector.with_events(events);
        }
        Self {
            registry,
            listener,
            table_id: table.id,
            collector: Arc::new(collector),
        }
    }

    fn table(&self) -> RouteTable {
        self.registry
            .route_table_for_listener(&self.listener)
            .unwrap()
    }

    /// A column of the global row stored under exactly `prefix`.
    fn text(&self, prefix: &str, column: &str) -> Option<String> {
        let table = self.table();
        let (cells, _) = table.row_cells(
            ROUTER.parse().unwrap(),
            cfg::bgp::GLOBAL_VRF,
            prefix.parse().unwrap(),
        )?;
        let read = self.registry.get(&table.id()).unwrap().read();
        Some(match &cells[read.column(column).unwrap() as usize] {
            CellOwned::String(s) => s.to_string(),
            CellOwned::Integer(i) => i.to_string(),
            CellOwned::Ip(ip) => ip.to_string(),
            CellOwned::Null => "null".into(),
            other => format!("{other:?}"),
        })
    }

    fn stat(&self, f: impl Fn(&Stats) -> &std::sync::atomic::AtomicU64) -> u64 {
        Stats::get(f(self.collector.stats()))
    }

    /// Starts a session for a router connecting from `ROUTER`.
    fn connect(&self) -> (Router, JoinHandle<SessionEnd>, CancellationToken) {
        let (ours, theirs) = tokio::io::duplex(1 << 16);
        let cancel = CancellationToken::new();
        let collector = self.collector.clone();
        let session = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                collector
                    .bgp_session(
                        theirs,
                        ROUTER.parse().unwrap(),
                        LOCAL.parse().unwrap(),
                        cancel,
                    )
                    .await
            }
        });
        let (rd, wr) = tokio::io::split(ours);
        let mut frames = BgpFrames::default();
        frames.allow_extended();
        (
            Router {
                rd: FramedRead::new(rd, frames),
                wr: Some(wr),
            },
            session,
            cancel,
        )
    }
}

struct Router {
    rd: FramedRead<ReadHalf<DuplexStream>, BgpFrames>,
    wr: Option<WriteHalf<DuplexStream>>,
}

impl Router {
    async fn send(&mut self, m: &[u8]) {
        self.wr.as_mut().unwrap().write_all(m).await.unwrap();
    }

    /// Closes the router's side; reads still work.
    async fn hang_up(&mut self) {
        if let Some(mut wr) = self.wr.take() {
            wr.shutdown().await.unwrap();
        }
    }

    /// The next message patrolli sends, or none once it closed.
    async fn recv(&mut self) -> Option<(u8, Vec<u8>)> {
        match self.rd.next().await {
            Some(Ok(f)) => Some((f[18], f[19..].to_vec())),
            _ => None,
        }
    }

    /// The next message, skipping KEEPALIVEs.
    async fn recv_other(&mut self) -> Option<(u8, Vec<u8>)> {
        loop {
            match self.recv().await? {
                (BGP_KEEPALIVE, _) => continue,
                m => return Some(m),
            }
        }
    }

    /// OPEN, our OPEN back, KEEPALIVEs both ways, then patrolli's
    /// End-of-RIB for each family the router's OPEN carries; returns
    /// patrolli's OPEN.
    async fn establish(&mut self, open: Vec<u8>) -> Open {
        self.send(&open).await;
        let (kind, body) = self.recv().await.expect("an open");
        assert_eq!(kind, BGP_OPEN);
        let ours = Open::parse(&body).unwrap();
        assert_eq!(self.recv().await.unwrap().0, BGP_KEEPALIVE);
        self.send(&msg::keepalive()).await;
        self.end_of_ribs(&open).await;
        ours
    }

    /// patrolli's End-of-RIB markers once Established (RFC 4724): one per
    /// family of the router's `open`, IPv4 first.
    async fn end_of_ribs(&mut self, open: &[u8]) {
        let theirs = Open::parse(&open[19..]).unwrap();
        for (i, afi) in [AFI_IPV4, AFI_IPV6].into_iter().enumerate() {
            if theirs.families[i] {
                let (kind, body) = self.recv().await.expect("an end-of-rib");
                assert_eq!(
                    msg::message(kind, &body),
                    msg::end_of_rib(afi, SAFI_UNICAST)
                );
            }
        }
    }

    /// A NOTIFICATION's code and subcode, then the stream's end.
    async fn notified(&mut self) -> (u8, u8) {
        let (code, subcode, _) = self.notified_with_data().await;
        (code, subcode)
    }

    /// A NOTIFICATION's code, subcode and data, then the stream's end.
    async fn notified_with_data(&mut self) -> (u8, u8, Vec<u8>) {
        let (kind, body) = self.recv_other().await.expect("a notification");
        assert_eq!(kind, BGP_NOTIFICATION, "{body:?}");
        assert!(self.recv_other().await.is_none(), "closed after it");
        (body[0], body[1], body[2..].to_vec())
    }
}

/// A router's OPEN: 4-octet AS, both families, route refresh.
fn open(asn: u32, hold: u16) -> Vec<u8> {
    open_with(
        asn,
        hold,
        &[
            caps::multiprotocol(AFI_IPV4, SAFI_UNICAST),
            caps::multiprotocol(AFI_IPV6, SAFI_UNICAST),
            caps::four_octet(asn),
            caps::route_refresh(),
        ],
    )
}

fn open_with(asn: u32, hold: u16, list: &[(u8, Vec<u8>)]) -> Vec<u8> {
    msg::message(BGP_OPEN, &open_body(asn, hold, ROUTER_ID, list))
}

fn update(body: Vec<u8>) -> Vec<u8> {
    msg::message(BGP_UPDATE, &body)
}

fn v4_route(nets: &[&str], path: &[u32]) -> Vec<u8> {
    update(u::update(
        &[],
        &[
            u::origin(0),
            u::as_path(&[(2, path)]),
            u::next_hop_attr("192.0.2.1"),
        ],
        &u::nlri(nets, None),
    ))
}

fn v6_route(nets: &[&str], path: &[u32]) -> Vec<u8> {
    let nh = "2001:db8::1"
        .parse::<std::net::Ipv6Addr>()
        .unwrap()
        .octets();
    update(u::update(
        &[],
        &[
            u::origin(0),
            u::as_path(&[(2, path)]),
            u::mp_reach(AFI_IPV6, SAFI_UNICAST, &nh, &u::nlri(nets, None)),
        ],
        &[],
    ))
}

fn v4_withdraw(nets: &[&str]) -> Vec<u8> {
    update(u::update(&u::nlri(nets, None), &[], &[]))
}

fn eor_v4() -> Vec<u8> {
    update(u::update(&[], &[], &[]))
}

fn sink(buffer: usize, routes: bool, on_full: OnFull) -> (EventSink, flume::Receiver<Event>) {
    let (tx, rx) = flume::bounded(buffer);
    (EventSink::new(tx, routes, on_full, Arc::default()), rx)
}

async fn eventually(check: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + LIMIT;
    while !check() {
        assert!(
            std::time::Instant::now() < deadline,
            "condition not met in time"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn ended(session: JoinHandle<SessionEnd>) -> SessionEnd {
    tokio::time::timeout(LIMIT, session)
        .await
        .expect("session ended")
        .unwrap()
}

#[tokio::test]
async fn our_open_mirrors_the_routers_as_and_offers_no_add_path() {
    let (events, rx) = sink(100, false, OnFull::Drop);
    let bed = Bed::with(
        CollectorConfig {
            bgp: Some(BgpConfig::default()),
            ..CollectorConfig::default()
        },
        Some(events),
    );
    let (mut router, session, cancel) = bed.connect();
    let ours = router.establish(open(64500, 180)).await;
    assert_eq!(ours.asn, 64500, "the router's own AS: iBGP");
    assert_eq!(ours.hold_time, 90);
    assert_eq!(ours.bgp_id, LOCAL.parse::<Ipv4Addr>().unwrap());
    assert_eq!(ours.families, [true, true]);
    assert_eq!(ours.add_path, [0, 0]);
    assert!(ours.four_octet_as && ours.route_refresh && ours.extended_message);
    assert_eq!(
        ours.graceful_restart,
        Some(0),
        "receiver only, for End-of-RIB"
    );

    router.send(&v4_route(&["10.0.0.0/8"], &[64501])).await;
    eventually(|| bed.table().len() == 1).await;
    cancel.cancel();
    assert_eq!((router.notified().await), (6, 2), "administrative shutdown");
    assert_eq!(ended(session).await, SessionEnd::Cancelled);
    let events: Vec<_> = rx.drain().collect();
    let names: Vec<_> = events.iter().map(|e| e.kind.name()).collect();
    assert_eq!(names, ["session_up", "peer_up", "session_down"]);
    assert!(events.iter().all(|e| e.protocol == Some(Protocol::Bgp)));
    assert_eq!(
        events[1].kind,
        EventKind::PeerUp {
            vrf: "global".into(),
            asn: 64500,
            bgp_id: ROUTER_ID,
        }
    );
    assert!(matches!(
        events[2].kind,
        EventKind::SessionDown {
            reason: "cancelled",
            notification: Some((6, 2)),
            ..
        }
    ));
    assert_eq!(bed.stat(|s| &s.notifications_sent[6]), 1);
}

#[tokio::test]
async fn a_configured_as_wins_and_a_peer_override_over_it() {
    let mut config = BgpConfig {
        asn: Some(4_200_000_000),
        router_id: Some(Ipv4Addr::new(10, 0, 0, 1)),
        ..BgpConfig::default()
    };
    let bed = Bed::new(config.clone());
    let (mut router, _session, _cancel) = bed.connect();
    let ours = router.establish(open(64500, 180)).await;
    assert_eq!(
        ours.asn, 4_200_000_000,
        "AS_TRANS on the wire, the rest in the capability"
    );
    assert_eq!(ours.bgp_id, Ipv4Addr::new(10, 0, 0, 1));

    config.peer_asn.insert(ROUTER.parse().unwrap(), 65010);
    let bed = Bed::new(config);
    let (mut router, _session, _cancel) = bed.connect();
    assert_eq!(router.establish(open(64500, 180)).await.asn, 65010);
}

#[tokio::test(start_paused = true)]
async fn a_peer_can_have_its_own_hold_time() {
    // RFC 4271 §10: configurable per peer
    let mut config = BgpConfig::default();
    config.peer_hold_time.insert(ROUTER.parse().unwrap(), 30);
    let bed = Bed::new(config);
    let (mut router, _session, _cancel) = bed.connect();
    let ours = router.establish(open(64500, 90)).await;
    assert_eq!(ours.hold_time, 30);
    let start = tokio::time::Instant::now();
    assert_eq!(router.recv().await.unwrap().0, BGP_KEEPALIVE);
    assert_eq!(start.elapsed(), Duration::from_secs(10));
}

#[tokio::test]
async fn end_of_rib_is_the_only_update_patrolli_sends() {
    let bed = Bed::new(BgpConfig::default());
    for list in [
        // no multiprotocol capability: IPv4 unicast only
        Vec::new(),
        vec![caps::multiprotocol(AFI_IPV6, SAFI_UNICAST)],
        caps::v4_v6().to_vec(),
    ] {
        let (mut router, session, cancel) = bed.connect();
        // checks one End-of-RIB per family of the router's OPEN (RFC 4724)
        router.establish(open_with(64500, 90, &list)).await;
        router.send(&v4_route(&["10.0.0.0/8"], &[64501])).await;
        router.send(&v6_route(&["2001:db8::/32"], &[64501])).await;
        router
            .send(&msg::route_refresh(AFI_IPV4, SAFI_UNICAST))
            .await;
        router.send(&eor_v4()).await;
        cancel.cancel();
        // after them, until it closes: never an UPDATE
        while let Some((kind, _)) = router.recv().await {
            assert_ne!(kind, BGP_UPDATE, "{list:?}");
        }
        assert_eq!(ended(session).await, SessionEnd::Cancelled);
    }
}

#[tokio::test]
async fn an_external_peer_is_read_as_one() {
    let bed = Bed::new(BgpConfig {
        asn: Some(65000),
        ..BgpConfig::default()
    });
    let (mut router, _session, _cancel) = bed.connect();
    router.establish(open(64500, 90)).await;
    let route = |path: Vec<u8>, extra: Vec<Vec<u8>>, net: &str| {
        let mut attrs = vec![u::origin(0), path, u::next_hop_attr("192.0.2.1")];
        attrs.extend(extra);
        update(u::update(&[], &attrs, &u::nlri(&[net], None)))
    };
    // RFC 4271 §5.1.5: LOCAL_PREF from an external peer is ignored;
    // RFC 8092 §3: a repeated large community is removed
    router
        .send(&route(
            u::as_path(&[(2, &[64500])]),
            vec![
                u::local_pref(200),
                u::large_communities(&[[64500, 2, 3], [64500, 1, 1], [64500, 2, 3]]),
            ],
            "10.0.0.0/8",
        ))
        .await;
    // RFC 5065 §5: a confederation segment from it is a malformed AS_PATH
    router
        .send(&route(
            u::as_path(&[(3, &[65001]), (2, &[64500])]),
            vec![],
            "10.1.0.0/16",
        ))
        .await;
    eventually(|| bed.stat(|s| &s.treat_as_withdraw) == 1).await;
    assert_eq!(bed.text("10.0.0.0/8", "local_pref").unwrap(), "null");
    assert_eq!(
        bed.text("10.0.0.0/8", "large_communities").unwrap(),
        "64500:1:1 64500:2:3"
    );
    assert_eq!(bed.text("10.1.0.0/16", "origin_as"), None);
    assert_eq!(bed.table().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn keepalives_go_out_at_a_third_of_the_hold_time() {
    let bed = Bed::new(BgpConfig::default());
    let (mut router, _session, _cancel) = bed.connect();
    let ours = router.establish(open(64500, 9)).await;
    assert_eq!(
        ours.hold_time, 90,
        "what we offer; the router's lower one wins"
    );
    for _ in 0..3 {
        let start = tokio::time::Instant::now();
        // the router keeps its side alive too
        router.send(&msg::keepalive()).await;
        assert_eq!(router.recv().await.unwrap().0, BGP_KEEPALIVE);
        assert_eq!(start.elapsed(), Duration::from_secs(3));
    }

    // no hold time: no keepalives, no hold timer
    let bed = Bed::new(BgpConfig::default());
    let (mut router, session, _cancel) = bed.connect();
    router.establish(open(64500, 0)).await;
    let quiet = tokio::time::timeout(Duration::from_secs(3600), router.recv()).await;
    assert!(quiet.is_err(), "{quiet:?}");
    assert!(!session.is_finished());
}

#[tokio::test(start_paused = true)]
async fn a_silent_router_runs_out_its_hold_time() {
    let (events, rx) = sink(100, false, OnFull::Drop);
    let bed = Bed::with(
        CollectorConfig {
            bgp: Some(BgpConfig::default()),
            ..CollectorConfig::default()
        },
        Some(events),
    );
    let (mut router, session, _cancel) = bed.connect();
    router.establish(open(64500, 9)).await;
    let start = tokio::time::Instant::now();
    assert_eq!(router.notified().await, (4, 0));
    assert_eq!(start.elapsed(), Duration::from_secs(9));
    assert_eq!(session.await.unwrap(), SessionEnd::HoldTimerExpired);
    let down = rx.drain().last().unwrap();
    assert_eq!(down.kind.name(), "session_down");
    assert!(matches!(
        down.kind,
        EventKind::SessionDown {
            reason: "hold_timer_expired",
            notification: Some((4, 0)),
            ..
        }
    ));
}

#[tokio::test]
async fn a_bad_open_is_refused_with_its_reason() {
    let bed = Bed::new(BgpConfig::default());
    let ours: Ipv4Addr = LOCAL.parse().unwrap();
    let mut v3 = open(64500, 90);
    v3[19] = 3;
    let same_id = msg::message(BGP_OPEN, &open_body(64500, 90, ours, &[]));
    let zero_id = msg::message(BGP_OPEN, &open_body(64500, 90, Ipv4Addr::UNSPECIFIED, &[]));
    // a parameter of type 1 (the deprecated authentication), length 0
    let mut auth = open_body(64500, 90, ROUTER_ID, &[]);
    auth[9] += 2;
    auth.extend_from_slice(&[1, 0]);
    let auth = msg::message(BGP_OPEN, &auth);
    for (first, want, end) in [
        (open(0, 90), (2, 2), SessionEnd::OpenRejected("peer as 0")),
        (
            auth,
            (2, 4),
            SessionEnd::OpenRejected("unsupported optional parameter"),
        ),
        (v3, (2, 1), SessionEnd::OpenRejected("unsupported version")),
        (
            open(64500, 2),
            (2, 6),
            SessionEnd::OpenRejected("unacceptable hold time"),
        ),
        (
            same_id,
            (2, 3),
            SessionEnd::OpenRejected("bad bgp identifier"),
        ),
        (
            zero_id,
            (2, 3),
            SessionEnd::OpenRejected("bad bgp identifier"),
        ),
        (
            msg::keepalive(),
            (5, 1),
            SessionEnd::Reset("expected an open"),
        ),
    ] {
        let (mut router, session, _cancel) = bed.connect();
        router.send(&first).await;
        assert_eq!(router.notified().await, want, "{end:?}");
        assert_eq!(ended(session).await, end);
    }
    assert_eq!(bed.stat(|s| &s.notifications_sent[2]), 6);
    assert_eq!(bed.stat(|s| &s.sessions_opened[Protocol::Bgp.index()]), 7);
    assert_eq!(
        bed.stat(|s| &s.sessions_closed[Protocol::Bgp.index()][9]),
        6,
        "open_rejected"
    );
}

#[tokio::test]
async fn updates_fill_the_table_and_end_of_rib_sweeps_what_a_reconnect_left() {
    let bed = Bed::new(BgpConfig::default());
    let (mut router, session, _cancel) = bed.connect();
    router.establish(open(64500, 90)).await;
    router
        .send(&v4_route(&["10.0.0.0/8", "10.1.0.0/16"], &[64501, 64502]))
        .await;
    router.send(&v6_route(&["2001:db8::/32"], &[64501])).await;
    router.send(&v4_withdraw(&["10.1.0.0/16"])).await;
    eventually(|| bed.stat(|s| &s.routes_withdrawn) == 1).await;
    assert_eq!(bed.table().len(), 2);
    assert_eq!(bed.text("10.0.0.0/8", "as_path").unwrap(), "64501 64502");
    assert_eq!(bed.text("10.0.0.0/8", "origin_as").unwrap(), "64502");
    assert_eq!(bed.text("10.0.0.0/8", "path_id").unwrap(), "null");
    assert_eq!(
        bed.text("2001:db8::/32", "next_hop").unwrap(),
        "2001:db8::1"
    );

    // the router goes away: its routes stay for keep
    router.hang_up().await;
    assert_eq!(ended(session).await, SessionEnd::Eof);
    assert_eq!(bed.table().len(), 2);

    // it comes back with 10.2.0.0/16 only; End-of-RIB for v4 sweeps the old
    // v4 route, and v6 waits for its own
    tokio::time::sleep(Duration::from_millis(2)).await;
    let (mut router, _session, _cancel) = bed.connect();
    router.establish(open(64500, 90)).await;
    router.send(&v4_route(&["10.2.0.0/16"], &[64501])).await;
    router.send(&eor_v4()).await;
    eventually(|| bed.stat(|s| &s.end_of_rib) == 1).await;
    assert_eq!(bed.text("10.0.0.0/8", "origin_as"), None, "swept");
    assert!(bed.text("10.2.0.0/16", "origin_as").is_some());
    assert!(bed.text("2001:db8::/32", "origin_as").is_some());
}

#[tokio::test]
async fn a_two_octet_router_has_its_as4_path_merged() {
    let bed = Bed::new(BgpConfig::default());
    let (mut router, _session, _cancel) = bed.connect();
    let ours = router
        .establish(open_with(
            64500,
            90,
            &[caps::multiprotocol(AFI_IPV4, SAFI_UNICAST)],
        ))
        .await;
    assert_eq!(ours.asn, 64500);
    router
        .send(&update(u::update(
            &[],
            &[
                u::origin(0),
                u::as_path2(&[(2, &[64501, 23456])]),
                u::next_hop_attr("192.0.2.1"),
                u::as4_path(&[(2, &[4_200_000_001])]),
            ],
            &u::nlri(&["10.0.0.0/8"], None),
        )))
        .await;
    eventually(|| bed.table().len() == 1).await;
    assert_eq!(
        bed.text("10.0.0.0/8", "as_path").unwrap(),
        "64501 4200000001"
    );
}

#[tokio::test]
async fn errors_are_notified_with_their_subcode_and_data() {
    let bed = Bed::new(BgpConfig::default());
    let nh6 = "2001:db8::1"
        .parse::<std::net::Ipv6Addr>()
        .unwrap()
        .octets();
    let bad_reach = u::mp_reach(
        AFI_IPV6,
        SAFI_UNICAST,
        &nh6[..4],
        &u::nlri(&["2001:db8::/32"], None),
    );
    let bad_med = u::attr(u::OPTIONAL, 4, &[1]);
    // (when, message, NOTIFICATION code, subcode and data)
    #[derive(Clone, Copy, PartialEq)]
    enum At {
        OpenSent,
        OpenConfirm,
        Established,
    }
    type Notification = (u8, u8, Vec<u8>);
    let cases: Vec<(At, Vec<u8>, Notification)> = vec![
        // RFC 4271 §6.1: an unknown type in any state, a KEEPALIVE with a
        // body
        (At::OpenSent, msg::message(9, &[]), (1, 3, vec![9])),
        (At::Established, msg::message(9, &[]), (1, 3, vec![9])),
        (
            At::Established,
            msg::message(BGP_KEEPALIVE, &[0]),
            (1, 2, vec![0, 20]),
        ),
        // RFC 6608 §4: the unexpected type as data
        (At::OpenSent, msg::keepalive(), (5, 1, vec![BGP_KEEPALIVE])),
        (
            At::OpenConfirm,
            v4_route(&["10.0.0.0/8"], &[64501]),
            (5, 2, vec![BGP_UPDATE]),
        ),
        (At::Established, open(64500, 90), (5, 3, vec![BGP_OPEN])),
        // RFC 4271 §6.3: a broken NLRI field
        (
            At::Established,
            update(u::update(&[], &[], &[33, 10, 0, 0, 0, 0])),
            (3, 10, vec![]),
        ),
        // RFC 4760 §7, RFC 7606 §7.11: an IPv6 next hop of 4 bytes
        (
            At::Established,
            update(u::update(
                &[],
                &[u::origin(0), u::as_path(&[]), bad_reach.clone()],
                &[],
            )),
            (3, 9, bad_reach),
        ),
        // RFC 7606 §5.2: a bad attribute and nothing reachable
        (
            At::Established,
            update(u::update(
                &u::nlri(&["10.0.0.0/8"], None),
                std::slice::from_ref(&bad_med),
                &[],
            )),
            (3, 5, bad_med),
        ),
    ];
    for (at, message, want) in cases {
        let (mut router, session, _cancel) = bed.connect();
        match at {
            At::OpenSent => {}
            At::OpenConfirm => {
                router.send(&open(64500, 90)).await;
                assert_eq!(router.recv().await.unwrap().0, BGP_OPEN);
                assert_eq!(router.recv().await.unwrap().0, BGP_KEEPALIVE);
            }
            At::Established => {
                router.establish(open(64500, 90)).await;
            }
        }
        router.send(&message).await;
        assert_eq!(router.notified_with_data().await, want);
        assert!(matches!(
            ended(session).await,
            SessionEnd::Reset(_) | SessionEnd::Frame(_)
        ));
    }
}

#[tokio::test]
async fn a_malformed_update_resets_the_session() {
    let bed = Bed::new(BgpConfig::default());
    let (mut router, session, _cancel) = bed.connect();
    router.establish(open(64500, 90)).await;
    // the attributes claiming more bytes than the message holds
    let mut body = u::update(&[], &[u::origin(0)], &[]);
    let attrs_len_at = 2;
    body[attrs_len_at..attrs_len_at + 2].copy_from_slice(&40u16.to_be_bytes());
    router.send(&update(body)).await;
    assert_eq!(router.notified().await, (3, 1));
    assert!(matches!(ended(session).await, SessionEnd::Reset(_)));
}

#[tokio::test]
async fn messages_past_4096_bytes_need_extended_messages() {
    // 900 host routes, 5 bytes each: past 4096, within the table's max_rows
    let nets: Vec<String> = (0..900)
        .map(|i| format!("10.0.{}.{}/32", i / 256, i % 256))
        .collect();
    let nets: Vec<&str> = nets.iter().map(String::as_str).collect();
    let big = v4_route(&nets, &[64501]);
    assert!(big.len() > 4096);

    let bed = Bed::new(BgpConfig::default());
    let (mut router, session, _cancel) = bed.connect();
    router.establish(open(64500, 90)).await;
    router.send(&big).await;
    assert_eq!(router.notified().await, (1, 2));
    assert_eq!(ended(session).await, SessionEnd::Frame("length"));

    let mut list = vec![
        caps::multiprotocol(AFI_IPV4, SAFI_UNICAST),
        caps::four_octet(64500),
    ];
    list.push(caps::extended_message());
    let (mut router, _session, _cancel) = bed.connect();
    router.establish(open_with(64500, 90, &list)).await;
    router.send(&big).await;
    eventually(|| bed.table().len() == 900).await;
}

#[tokio::test]
async fn a_routers_notification_ends_the_session_with_its_text() {
    let (events, rx) = sink(100, false, OnFull::Drop);
    let bed = Bed::with(
        CollectorConfig {
            bgp: Some(BgpConfig::default()),
            ..CollectorConfig::default()
        },
        Some(events),
    );
    let (mut router, session, _cancel) = bed.connect();
    router.establish(open(64500, 90)).await;
    router
        .send(&msg::notification(
            6,
            2,
            &msg::shutdown_communication("maintenance"),
        ))
        .await;
    assert_eq!(ended(session).await, SessionEnd::Notification);
    let down = rx.drain().last().unwrap();
    assert_eq!(
        down.kind,
        EventKind::SessionDown {
            reason: "notification",
            error: None,
            notification: Some((6, 2)),
            message: Some("maintenance".into()),
        }
    );
    assert_eq!(bed.stat(|s| &s.notifications_received[6]), 1);
}

#[tokio::test]
async fn a_rebuilt_table_is_refilled_by_route_refresh_or_a_reset() {
    let bed = Bed::new(BgpConfig::default());
    let (mut router, _session, _cancel) = bed.connect();
    router.establish(open(64500, 90)).await;
    router.send(&v4_route(&["10.0.0.0/8"], &[64501])).await;
    eventually(|| bed.table().len() == 1).await;

    // the table is rebuilt: a new, empty one for the same listener
    bed.registry.remove(&bed.table_id);
    bed.registry.insert(&table_config(bed.listener)).unwrap();
    for _ in 0..TABLE_CHECK_MESSAGES {
        router.send(&msg::keepalive()).await;
    }
    let mut refreshed = Vec::new();
    while refreshed.len() < 2 {
        let (kind, body) = router.recv_other().await.unwrap();
        assert_eq!(kind, BGP_ROUTE_REFRESH);
        refreshed.push(u16::from_be_bytes([body[0], body[1]]));
    }
    assert_eq!(refreshed, [AFI_IPV4, AFI_IPV6]);
    // what the router sends again lands in the new table
    router.send(&v4_route(&["10.0.0.0/8"], &[64501])).await;
    eventually(|| bed.table().len() == 1).await;

    // without route refresh, only a new session refills it
    let (mut router, session, _cancel) = bed.connect();
    router
        .establish(open_with(
            64501,
            90,
            &[caps::multiprotocol(AFI_IPV4, SAFI_UNICAST)],
        ))
        .await;
    let table = bed.table();
    bed.registry.remove(&table.id());
    bed.registry.insert(&table_config(bed.listener)).unwrap();
    for _ in 0..TABLE_CHECK_MESSAGES {
        router.send(&msg::keepalive()).await;
    }
    assert_eq!(router.notified().await, (6, 6));
    assert_eq!(ended(session).await, SessionEnd::Resync);
}

#[tokio::test(start_paused = true)]
async fn keepalives_go_on_while_a_full_channel_holds_the_reader() {
    let (events, rx) = sink(1, true, OnFull::Wait);
    let bed = Bed::with(
        CollectorConfig {
            bgp: Some(BgpConfig::default()),
            ..CollectorConfig::default()
        },
        Some(events),
    );
    let (mut router, session, _cancel) = bed.connect();
    router.send(&open(64500, 9)).await;
    assert_eq!(router.recv().await.unwrap().0, BGP_OPEN);
    assert_eq!(router.recv().await.unwrap().0, BGP_KEEPALIVE);
    router.send(&msg::keepalive()).await;
    router.end_of_ribs(&open(64500, 9)).await;
    router.send(&v4_route(&["10.0.0.0/8"], &[64501])).await;

    // session_up fills the channel: peer_up, and the reader with it, wait.
    // Over a minute the router sees keepalives every 3 s and patrolli never
    // runs out its own hold time.
    let start = tokio::time::Instant::now();
    let mut keepalives = 0;
    while start.elapsed() < Duration::from_secs(60) {
        assert_eq!(router.recv().await.unwrap().0, BGP_KEEPALIVE);
        keepalives += 1;
    }
    assert_eq!(keepalives, 20);
    assert!(!session.is_finished());
    assert!(bed.table().is_empty(), "the routes wait behind the events");

    // the pipeline drains: the route goes in
    let mut names = Vec::new();
    while names.len() < 3 {
        names.push(rx.recv_async().await.unwrap().kind.name());
    }
    assert_eq!(names, ["session_up", "peer_up", "announce"]);
    eventually(|| bed.table().len() == 1).await;
}

#[tokio::test]
async fn a_router_live_over_bmp_is_refused_over_bgp_and_back() {
    let bed = Bed::with(
        CollectorConfig {
            bmp: Some(BmpConfig::default()),
            bgp: Some(BgpConfig::default()),
            vrfs: None,
        },
        None,
    );
    // a live BMP session from the router
    let (mut bmp, bmp_stream) = tokio::io::duplex(1 << 16);
    let collector = bed.collector.clone();
    let bmp_cancel = CancellationToken::new();
    let bmp_session = tokio::spawn({
        let cancel = bmp_cancel.clone();
        async move {
            collector
                .session(bmp_stream, ROUTER.parse().unwrap(), cancel)
                .await
        }
    });
    bmp.write_all(&b::initiation("r1")).await.unwrap();
    eventually(|| bed.collector.routers().is_live(ROUTER.parse().unwrap())).await;

    let (mut router, session, _cancel) = bed.connect();
    assert_eq!(router.notified().await, (6, 5), "connection rejected");
    assert_eq!(ended(session).await, SessionEnd::Refused);
    assert_eq!(bed.stat(|s| &s.sessions_refused[Protocol::Bgp.index()]), 1);
    assert!(!bmp_session.is_finished(), "the live session goes on");

    // the other way round
    drop(bmp);
    assert_eq!(ended(bmp_session).await, SessionEnd::Eof);
    let (mut router, _session, _cancel) = bed.connect();
    router.establish(open(64500, 90)).await;
    let collector = bed.collector.clone();
    let end = collector
        .session(
            &b::initiation("r1")[..],
            ROUTER.parse().unwrap(),
            bmp_cancel,
        )
        .await;
    assert_eq!(end, SessionEnd::Refused);
    assert_eq!(bed.stat(|s| &s.sessions_refused[Protocol::Bmp.index()]), 1);
}

#[tokio::test]
async fn a_reconnect_replaces_the_live_session_with_a_collision_cease() {
    let bed = Bed::new(BgpConfig::default());
    let (mut old, old_session, _cancel) = bed.connect();
    old.establish(open(64500, 90)).await;
    let (mut new, _session, _cancel) = bed.connect();
    assert_eq!(old.notified().await, (6, 7));
    assert_eq!(ended(old_session).await, SessionEnd::Cancelled);
    new.establish(open(64500, 90)).await;
    assert_eq!(
        bed.stat(|s| &s.sessions_superseded[Protocol::Bgp.index()]),
        1
    );
}

#[tokio::test]
async fn a_listener_that_keeps_no_global_vrf_only_counts_updates() {
    let bed = Bed::with(
        CollectorConfig {
            bgp: Some(BgpConfig::default()),
            vrfs: Some(["65000:1".into()].into_iter().collect()),
            ..CollectorConfig::default()
        },
        None,
    );
    let (mut router, _session, _cancel) = bed.connect();
    router.establish(open(64500, 90)).await;
    router.send(&v4_route(&["10.0.0.0/8"], &[64501])).await;
    eventually(|| bed.stat(|s| &s.vrf_filtered) == 1).await;
    assert!(bed.table().is_empty());
}

#[tokio::test]
async fn netgauze_reads_what_we_send() {
    use netgauze_bgp_pkt::BgpMessage;
    use netgauze_bgp_pkt::wire::deserializer::BgpParsingContext;
    use netgauze_parse_utils::reader::SliceReader;
    use netgauze_parse_utils::traits::ParseFromWithOneInput;
    let bed = Bed::new(BgpConfig {
        asn: Some(4_200_000_000),
        ..BgpConfig::default()
    });
    let (mut router, _session, cancel) = bed.connect();
    router.send(&open(64500, 90)).await;
    let mut sent = Vec::new();
    for _ in 0..2 {
        let (kind, body) = router.recv().await.unwrap();
        sent.push(msg::message(kind, &body));
    }
    router.send(&msg::keepalive()).await;
    cancel.cancel();
    while let Some((kind, body)) = router.recv().await {
        if kind != BGP_KEEPALIVE {
            sent.push(msg::message(kind, &body));
        }
    }
    sent.push(msg::route_refresh(AFI_IPV6, SAFI_UNICAST));
    for m in &sent {
        let mut cur = SliceReader::new(m);
        let parsed = BgpMessage::parse(&mut cur, &mut BgpParsingContext::default())
            .unwrap_or_else(|e| panic!("{m:?}: {e:?}"));
        assert!(cur.is_empty());
        if let BgpMessage::Open(open) = parsed {
            assert_eq!(open.my_asn4(), 4_200_000_000);
        }
    }
    assert_eq!(
        sent.len(),
        4,
        "open, keepalive, notification, route refresh"
    );
}
