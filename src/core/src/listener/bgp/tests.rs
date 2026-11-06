// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Sessions end to end: captured and crafted BMP streams into a route table.

use super::events::{Event, EventKind, EventSink};
use super::routers::Protocol;
use super::wire::bmp::{self, Message, build as b};
use super::wire::open::{self, Open, build_open, caps};
use super::wire::update::{AsPath, SegmentKind, Update, build as u, prefixes};
use super::wire::{AFI_IPV4, AFI_IPV6, Afi, BGP_UPDATE, Reader, SAFI_UNICAST, bgp_message};
use super::{BmpConfig, Collector, CollectorConfig, SessionEnd, codec::BmpFrames, stats::Stats};
use crate::cache::lookup_table::LookupTable;
use crate::cache::lookup_table::bgp::RouteTable;
use crate::cache::lookup_table::cell::CellOwned;
use crate::config::listener::OnFull;
use crate::config::lookup_table as cfg;
use bytes::BytesMut;
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio_util::codec::Decoder;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// The concatenated TCP payloads of a classic pcap (Ethernet, IPv4 or IPv6),
/// in capture order.
fn tcp_payload(pcap: &[u8]) -> Vec<u8> {
    let le = match pcap.get(..4) {
        Some([0xd4, 0xc3, 0xb2, 0xa1] | [0x4d, 0x3c, 0xb2, 0xa1]) => true,
        Some([0xa1, 0xb2, 0xc3, 0xd4] | [0xa1, 0xb2, 0x3c, 0x4d]) => false,
        _ => panic!("not a classic pcap"),
    };
    let u32_at = |o: usize| {
        let v: [u8; 4] = pcap[o..o + 4].try_into().unwrap();
        if le {
            u32::from_le_bytes(v)
        } else {
            u32::from_be_bytes(v)
        }
    };
    assert_eq!(u32_at(20), 1, "ethernet captures only");
    let be16 = |b: &[u8], o: usize| u16::from_be_bytes([b[o], b[o + 1]]) as usize;
    let mut out = Vec::new();
    let mut off = 24;
    while off + 16 <= pcap.len() {
        let incl = u32_at(off + 8) as usize;
        let pkt = &pcap[off + 16..off + 16 + incl];
        off += 16 + incl;
        let (tcp, end) = match be16(pkt, 12) {
            0x0800 => {
                let ip = 14;
                let ihl = (pkt[ip] & 0x0f) as usize * 4;
                assert_eq!(pkt[ip + 9], 6, "tcp only");
                (ip + ihl, ip + be16(pkt, ip + 2))
            }
            0x86dd => {
                let ip = 14;
                assert_eq!(pkt[ip + 6], 6, "tcp only");
                (ip + 40, ip + 40 + be16(pkt, ip + 4))
            }
            other => panic!("ethertype {other:#x}"),
        };
        let data = tcp + (pkt[tcp + 12] >> 4) as usize * 4;
        out.extend_from_slice(&pkt[data..end]);
    }
    out
}

fn capture(name: &str) -> Vec<u8> {
    let pcap: &[u8] = match name {
        "eor" => include_bytes!("testdata/akvorado/bmp-eor.pcap"),
        "init" => include_bytes!("testdata/akvorado/bmp-init.pcap"),
        "l3vpn" => include_bytes!("testdata/akvorado/bmp-l3vpn.pcap"),
        "locrib" => include_bytes!("testdata/akvorado/bmp-locrib.pcap"),
        "peer-down" => include_bytes!("testdata/akvorado/bmp-peer-down.pcap"),
        "peer-up" => include_bytes!("testdata/akvorado/bmp-peer-up.pcap"),
        "peers-up" => include_bytes!("testdata/akvorado/bmp-peers-up.pcap"),
        "reach-addpath" => include_bytes!("testdata/akvorado/bmp-reach-addpath.pcap"),
        "reach-unknown-family" => include_bytes!("testdata/akvorado/bmp-reach-unknown-family.pcap"),
        "reach-vpls" => include_bytes!("testdata/akvorado/bmp-reach-vpls.pcap"),
        "reach" => include_bytes!("testdata/akvorado/bmp-reach.pcap"),
        "terminate" => include_bytes!("testdata/akvorado/bmp-terminate.pcap"),
        "unreach" => include_bytes!("testdata/akvorado/bmp-unreach.pcap"),
        other => panic!("no capture {other}"),
    };
    tcp_payload(pcap)
}

const ALL_CAPTURES: [&str; 13] = [
    "init",
    "peers-up",
    "peer-up",
    "eor",
    "reach",
    "reach-addpath",
    "reach-unknown-family",
    "reach-vpls",
    "unreach",
    "peer-down",
    "l3vpn",
    "locrib",
    "terminate",
];

fn frames(stream: &[u8]) -> Vec<Vec<u8>> {
    let mut codec = BmpFrames::new(super::codec::DEFAULT_MAX_MESSAGE);
    let mut buf = BytesMut::from(stream);
    let mut out = Vec::new();
    while let Some(f) = codec.decode(&mut buf).unwrap() {
        out.push(f.to_vec());
    }
    assert!(buf.is_empty(), "a capture ends on a message boundary");
    out
}

/// Every unicast route of every peer in `stream`, Adj-RIB included, in
/// akvorado's dump format minus the origin AS (which akvorado derives
/// differently): `[family] prefix via next-hop 0:0/path-id [path] [communities]
/// [large communities]`.
fn dump_all_peers(stream: &[u8]) -> BTreeMap<String, Vec<String>> {
    let mut add_path: BTreeMap<(IpAddr, [u8; 8]), [bool; 2]> = BTreeMap::new();
    let mut rib: BTreeMap<(String, String, String, u32), String> = BTreeMap::new();
    for frame in frames(stream) {
        match Message::parse(&frame).unwrap() {
            Message::PeerUp {
                peer,
                sent_open,
                received_open,
                ..
            } => {
                let sent = Open::parse(sent_open).unwrap();
                let received = Open::parse(received_open).unwrap();
                let mut modes = [false; 2];
                for f in Afi::ALL {
                    // Adj-RIB-In: the peer sends and the router receives
                    modes[f.index()] = received.add_path[f.index()] & open::ADD_PATH_SEND != 0
                        && sent.add_path[f.index()] & open::ADD_PATH_RECEIVE != 0;
                }
                add_path.insert((peer.address, peer.distinguisher), modes);
            }
            Message::RouteMonitoring { peer, bgp } => {
                let modes = add_path
                    .get(&(peer.address, peer.distinguisher))
                    .copied()
                    .unwrap_or_default();
                let (kind, body) = bgp_message(&mut Reader::new(bgp)).unwrap();
                assert_eq!(kind, BGP_UPDATE);
                let update = Update::parse(body, peer.asn4(), modes).unwrap();
                let a = update.attrs;
                let peer_key = peer.address.to_string();
                let mut groups = vec![(Afi::V4, update.withdrawn, None, false)];
                if let Some(un) = a.mp_unreach
                    && let Some(f) = Afi::unicast(un.afi, un.safi)
                {
                    groups.push((f, un.nlri, None, false));
                }
                if let Some(r) = a.mp_reach
                    && let Some(f) = Afi::unicast(r.afi, r.safi)
                {
                    groups.push((f, r.nlri, r.next_hop, true));
                }
                groups.push((Afi::V4, update.nlri, a.next_hop.map(IpAddr::V4), true));
                let mut path = AsPath::default();
                if let Some(bytes) = a.as_path {
                    path.decode(bytes, peer.asn4()).unwrap();
                }
                let asns: Vec<String> = path
                    .segments()
                    .flat_map(|(kind, asns)| match kind {
                        SegmentKind::Set => &asns[..1],
                        _ => asns,
                    })
                    .map(u32::to_string)
                    .collect();
                let comms: Vec<String> = a.communities().map(|c| c.to_string()).collect();
                let large: Vec<String> = a
                    .large_communities()
                    .map(|[g, d1, d2]| format!("{{{g} {d1} {d2}}}"))
                    .collect();
                for (family, bytes, nh, announce) in groups {
                    let fam = match family {
                        Afi::V4 => "ipv4-unicast",
                        Afi::V6 => "ipv6-unicast",
                    };
                    for p in prefixes(bytes, family, modes[family.index()]) {
                        let (id, net) = p.unwrap();
                        let key = (
                            peer_key.clone(),
                            fam.to_string(),
                            net.to_string(),
                            id.unwrap_or(0),
                        );
                        if announce {
                            let line = format!(
                                "[{fam}] {net} via {} 0:0/{} [{}] [{}] [{}]",
                                nh.unwrap(),
                                id.unwrap_or(0),
                                asns.join(" "),
                                comms.join(" "),
                                large.join(" "),
                            );
                            rib.insert(key, line);
                        } else {
                            rib.remove(&key);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for ((peer, ..), line) in rib {
        out.entry(peer).or_default().push(line);
    }
    out
}

/// akvorado's expected lines, unicast only, without the origin AS token.
fn akvorado(lines: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = lines
        .iter()
        .filter(|l| l.starts_with("[ipv4-unicast]") || l.starts_with("[ipv6-unicast]"))
        .map(|l| {
            let mut tokens: Vec<&str> = l.split(' ').collect();
            tokens.remove(5);
            tokens.join(" ")
        })
        .collect();
    out.sort();
    out
}

#[test]
fn juniper_captures_decode_to_akvoradoes_rib() {
    let stream: Vec<u8> = ["init", "peers-up", "eor", "reach", "reach-addpath"]
        .iter()
        .flat_map(|n| capture(n))
        .collect();
    let got = dump_all_peers(&stream);
    // root_test.go "init, peers up, eor, reach NLRI, reach ADD-PATH"
    let want: BTreeMap<String, Vec<String>> = [
        (
            "2001:db8::3",
            akvorado(&[
                "[ipv6-unicast] 2001:db8:1::/64 via 2001:db8::3 0:0/0 174 [65013 65013 174 174 174] [4260691978 4260691988] []",
                "[ipv6-unicast] 2001:db8:2::/64 via 2001:db8::3 0:0/0 12322 [65013 65013 1299 1299 1299 12322] [4260691998] []",
                "[ipv6-unicast] 2001:db8::2/127 via 2001:db8::3 0:0/0 65013 [65013] [] []",
            ]),
        ),
        (
            "2001:db8::7",
            akvorado(&[
                "[ipv4-unicast] 192.0.2.6/31 via 192.0.2.7 0:0/0 65017 [65017] [] []",
                "[ipv6-unicast] 2001:db8:1::/64 via 2001:db8::7 0:0/0 174 [65017 65013 174 174 174] [4260954122 4260954132] [{65017 300 4}]",
                "[ipv6-unicast] 2001:db8:2::/64 via 2001:db8::7 0:0/0 12322 [65017 65017 1299 1299 1299 12322] [4260954142] [{65017 400 2}]",
                "[ipv6-unicast] 2001:db8::6/127 via 2001:db8::7 0:0/0 65017 [65017] [] []",
            ]),
        ),
        (
            "192.0.2.1",
            akvorado(&[
                "[ipv4-unicast] 192.0.2.0/31 via 192.0.2.1 0:0/0 65011 [65011] [] []",
                "[ipv4-unicast] 198.51.100.0/25 via 192.0.2.1 0:0/0 64476 [65011 65011 174 1299 64476] [4260560906 4260560916] []",
                "[ipv4-unicast] 198.51.100.128/25 via 192.0.2.1 0:0/0 396919 [65011 65011 174 29447 396919] [4260560908] []",
            ]),
        ),
        (
            "192.0.2.5",
            akvorado(&["[ipv4-unicast] 192.0.2.4/31 via 192.0.2.5 0:0/1 65500 [] [] []"]),
        ),
    ]
    .into_iter()
    .map(|(peer, mut lines)| {
        lines.sort();
        (peer.to_string(), lines)
    })
    .collect();
    let got: BTreeMap<String, Vec<String>> = got
        .into_iter()
        .map(|(peer, mut lines)| {
            lines.sort();
            (peer, lines)
        })
        .collect();
    assert_eq!(got, want);
}

#[test]
fn every_capture_decodes_without_error() {
    for name in ALL_CAPTURES {
        for frame in frames(&capture(name)) {
            let message = Message::parse(&frame).unwrap_or_else(|e| panic!("{name}: {e}"));
            if let Message::RouteMonitoring { peer, bgp } = message {
                let (_, body) = bgp_message(&mut Reader::new(bgp)).unwrap();
                // add-path decoding needs the peer's open; without it, only
                // the captures without path ids frame
                let _ = Update::parse(body, peer.asn4(), [false; 2]);
            }
        }
    }
}

// ── sessions ────────────────────────────────────────────────────────────

/// Waits (up to 5 s) for `check` to hold.
async fn eventually(check: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !check() {
        assert!(
            std::time::Instant::now() < deadline,
            "condition not met in time"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

const ROUTER: &str = "198.51.100.254";

struct Harness {
    registry: Arc<LookupTable>,
    config: cfg::LookupTable,
    collector: Arc<Collector>,
}

impl Harness {
    fn new() -> Self {
        Self::with_keep(Duration::from_secs(300))
    }

    fn with_keep(keep: Duration) -> Self {
        Self::with(
            BmpConfig {
                keep,
                ..BmpConfig::default()
            },
            None,
        )
    }

    fn with(collect: impl Into<CollectorConfig>, events: Option<EventSink>) -> Self {
        let registry = Arc::new(LookupTable::new());
        let listener = Uuid::now_v7();
        let config = cfg::LookupTable {
            id: Uuid::now_v7(),
            name: "routes".into(),
            source: cfg::Source::Bgp(cfg::Bgp {
                listener,
                max_rows: 1000,
            }),
            key: cfg::Bgp::key(),
        };
        registry.insert(&config).unwrap();
        let mut collector = Collector::new(listener, registry.clone(), collect.into());
        if let Some(events) = events {
            collector = collector.with_events(events);
        }
        let collector = Arc::new(collector);
        Self {
            registry,
            config,
            collector,
        }
    }

    fn table(&self) -> RouteTable {
        self.registry.route_table(&self.config.id).unwrap()
    }

    fn router(&self) -> IpAddr {
        ROUTER.parse().unwrap()
    }

    async fn run(&self, stream: &[u8]) -> SessionEnd {
        self.collector
            .session(stream, self.router(), CancellationToken::new())
            .await
    }

    /// A column of the row stored under exactly `prefix`.
    fn cell(&self, vrf: &str, prefix: &str, column: &str) -> Option<CellOwned> {
        let (cells, _) = self
            .table()
            .row_cells(self.router(), vrf, prefix.parse().unwrap())?;
        let read = self.registry.get(&self.config.id).unwrap().read();
        Some(cells[read.column(column).unwrap() as usize].clone())
    }

    fn text(&self, vrf: &str, prefix: &str, column: &str) -> Option<String> {
        match self.cell(vrf, prefix, column)? {
            CellOwned::String(s) => Some(s.to_string()),
            CellOwned::Integer(i) => Some(i.to_string()),
            CellOwned::Ip(ip) => Some(ip.to_string()),
            CellOwned::Null => Some("null".into()),
            other => Some(format!("{other:?}")),
        }
    }

    fn stat(&self, f: impl Fn(&Stats) -> &std::sync::atomic::AtomicU64) -> u64 {
        Stats::get(f(self.collector.stats()))
    }
}

fn global_peer() -> Vec<u8> {
    b::loc_rib([0; 8])
}

/// Initiation, then a Loc-RIB Peer Up for `peer` with v4 and v6 and the
/// given ADD-PATH families.
fn start(peer: &[u8], add_path: &[(u16, u8, u8)]) -> Vec<u8> {
    let mut list = caps::v4_v6().to_vec();
    list.push(caps::four_octet(64500));
    if !add_path.is_empty() {
        list.push(caps::add_path(add_path));
    }
    let open = build_open(64500, Ipv4Addr::new(192, 0, 2, 254), &list);
    let mut s = b::initiation("r1");
    s.extend_from_slice(&b::peer_up(peer, &open));
    s
}

fn v4_route(peer: &[u8], nets: &[&str], path: &[u32], nh: &str) -> Vec<u8> {
    b::route_monitoring(
        peer,
        &u::update(
            &[],
            &[
                u::origin(0),
                u::as_path(&[(2, path)]),
                u::next_hop_attr(nh),
                u::local_pref(100),
            ],
            &u::nlri(nets, None),
        ),
    )
}

fn v6_route(peer: &[u8], nets: &[&str], path: &[u32], path_id: Option<u32>) -> Vec<u8> {
    let nh = "2001:db8::1"
        .parse::<std::net::Ipv6Addr>()
        .unwrap()
        .octets();
    b::route_monitoring(
        peer,
        &u::update(
            &[],
            &[
                u::origin(0),
                u::as_path(&[(2, path)]),
                u::mp_reach(AFI_IPV6, SAFI_UNICAST, &nh, &u::nlri(nets, path_id)),
            ],
            &[],
        ),
    )
}

fn v4_withdraw(peer: &[u8], nets: &[&str]) -> Vec<u8> {
    b::route_monitoring(peer, &u::update(&u::nlri(nets, None), &[], &[]))
}

fn eor_v4(peer: &[u8]) -> Vec<u8> {
    b::route_monitoring(peer, &u::update(&[], &[], &[]))
}

fn eor_v6(peer: &[u8]) -> Vec<u8> {
    b::route_monitoring(
        peer,
        &u::update(&[], &[u::mp_unreach(AFI_IPV6, SAFI_UNICAST, &[])], &[]),
    )
}

#[tokio::test]
async fn gobgps_loc_rib_capture_fills_the_table() {
    let h = Harness::new();
    let end = h.run(&capture("locrib")).await;
    assert_eq!(end, SessionEnd::Eof);
    let g = cfg::bgp::GLOBAL_VRF;
    // two paths for 198.51.100.0/25: the lowest id is shown
    assert_eq!(h.table().len(), 4);
    assert_eq!(h.text(g, "198.51.100.0/25", "path_id").unwrap(), "1");
    assert_eq!(
        h.text(g, "198.51.100.0/25", "next_hop").unwrap(),
        "192.0.2.1"
    );
    assert_eq!(
        h.text(g, "198.51.100.0/25", "as_path").unwrap(),
        "65011 174 64476"
    );
    assert_eq!(h.text(g, "198.51.100.0/25", "origin_as").unwrap(), "64476");
    assert_eq!(h.text(g, "198.51.100.0/25", "peer_as").unwrap(), "65011");
    assert_eq!(
        h.text(g, "198.51.100.0/25", "communities").unwrap(),
        "64779:1 64779:2"
    );
    assert_eq!(
        h.text(g, "198.51.100.0/25", "large_communities").unwrap(),
        "65000:300:4"
    );
    assert_eq!(
        h.text(g, "198.51.100.128/25", "origin_as").unwrap(),
        "396919"
    );
    assert_eq!(
        h.text(g, "2001:db8:1::/64", "next_hop").unwrap(),
        "2001:db8::3"
    );
    assert_eq!(
        h.text(g, "2001:db8:1::/64", "as_path").unwrap(),
        "65013 174 174 174"
    );
    assert_eq!(h.text(g, "2001:db8:2::/64", "origin_as").unwrap(), "12322");
    assert_eq!(h.stat(|s| &s.routes_announced), 5);
    assert_eq!(h.stat(|s| &s.messages[bmp::ROUTE_MONITORING as usize]), 5);
}

#[tokio::test]
async fn juniper_adj_rib_in_is_counted_and_skipped() {
    let h = Harness::new();
    let stream: Vec<u8> = ["init", "peers-up", "eor", "reach", "terminate"]
        .iter()
        .flat_map(|n| capture(n))
        .collect();
    assert_eq!(h.run(&stream).await, SessionEnd::Terminated);
    assert!(h.table().is_empty());
    assert!(h.stat(|s| &s.adj_rib_messages) > 0);
    assert_eq!(h.stat(|s| &s.routes_announced), 0);
    assert_eq!(
        h.stat(|s| &s.sessions_closed[Protocol::Bmp.index()][SessionEnd::Terminated.index()]),
        1
    );
}

#[tokio::test]
async fn a_session_writes_withdraws_and_sweeps_at_end_of_rib() {
    let h = Harness::new();
    let g = cfg::bgp::GLOBAL_VRF;
    let p = global_peer();
    // a previous session left routes behind
    let mut s = start(&p, &[]);
    s.extend(v4_route(
        &p,
        &["10.0.0.0/8", "10.1.0.0/16"],
        &[64501],
        "192.0.2.1",
    ));
    s.extend(v6_route(&p, &["2001:db8::/32"], &[64501], None));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    assert_eq!(h.table().len(), 3);

    // the new dump re-sends 10.0.0.0/8 only, then End-of-RIB for v4
    tokio::time::sleep(Duration::from_millis(2)).await;
    let mut s = start(&p, &[]);
    s.extend(v4_route(&p, &["10.0.0.0/8"], &[64502, 64503], "192.0.2.9"));
    s.extend(v4_withdraw(&p, &["192.0.2.0/24"]));
    s.extend(eor_v4(&p));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    assert_eq!(h.text(g, "10.0.0.0/8", "as_path").unwrap(), "64502 64503");
    assert_eq!(h.text(g, "10.0.0.0/8", "next_hop").unwrap(), "192.0.2.9");
    assert_eq!(h.text(g, "10.0.0.0/8", "local_pref").unwrap(), "100");
    assert_eq!(h.text(g, "10.0.0.0/8", "path_id").unwrap(), "null");
    assert_eq!(h.cell(g, "10.1.0.0/16", "prefix"), None, "swept");
    assert!(
        h.cell(g, "2001:db8::/32", "as_path").is_some(),
        "v6 waits for its own EoR"
    );
    assert_eq!(h.stat(|s| &s.unknown_withdraws), 1);
    assert_eq!(h.stat(|s| &s.rows_swept), 1);

    // v6 End-of-RIB with nothing re-sent
    tokio::time::sleep(Duration::from_millis(2)).await;
    let mut s = start(&p, &[]);
    s.extend(v4_route(&p, &["10.0.0.0/8"], &[64502], "192.0.2.9"));
    s.extend(eor_v6(&p));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    assert_eq!(h.table().len(), 1);
}

#[tokio::test]
async fn vrfs_are_partitions_and_peer_down_takes_one_whole() {
    let h = Harness::new();
    let global = global_peer();
    let red = b::loc_rib(b::rd0(64500, 1));
    let mut s = start(&global, &[]);
    s.extend(start(&red, &[])[b::initiation("r1").len()..].to_vec());
    s.extend(v4_route(&global, &["10.0.0.0/8"], &[64501], "192.0.2.1"));
    s.extend(v4_route(&red, &["10.0.0.0/8"], &[64511], "192.0.2.1"));
    s.extend(v6_route(&red, &["2001:db8::/32"], &[64511], None));
    s.extend(b::peer_down(&red, 6));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    assert_eq!(h.table().len(), 1);
    assert_eq!(
        h.text("global", "10.0.0.0/8", "origin_as").unwrap(),
        "64501"
    );
    assert_eq!(h.cell("64500:1", "10.0.0.0/8", "origin_as"), None);
    assert_eq!(h.stat(|s| &s.routes_withdrawn), 2);
}

#[tokio::test]
async fn add_path_promotes_the_next_path_on_withdraw() {
    let h = Harness::new();
    let g = cfg::bgp::GLOBAL_VRF;
    let p = global_peer();
    let mut s = start(&p, &[(AFI_IPV6, SAFI_UNICAST, open::ADD_PATH_SEND)]);
    s.extend(v6_route(&p, &["2001:db8::/32"], &[64502], Some(2)));
    s.extend(v6_route(&p, &["2001:db8::/32"], &[64501], Some(1)));
    s.extend(b::route_monitoring(
        &p,
        &u::update(
            &[],
            &[u::mp_unreach(
                AFI_IPV6,
                SAFI_UNICAST,
                &u::nlri(&["2001:db8::/32"], Some(1)),
            )],
            &[],
        ),
    ));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    assert_eq!(h.text(g, "2001:db8::/32", "path_id").unwrap(), "2");
    assert_eq!(h.text(g, "2001:db8::/32", "origin_as").unwrap(), "64502");
}

/// RFC 9069 §5.2: a Loc-RIB OPEN's ADD-PATH mode is ignored, so a router
/// advertising receive only still sends path ids. Read as prefix bytes, a path
/// id of 0 would announce `::/0`.
#[tokio::test]
async fn add_path_receive_mode_still_carries_path_ids() {
    let h = Harness::new();
    let g = cfg::bgp::GLOBAL_VRF;
    let p = global_peer();
    let mut s = start(&p, &[(AFI_IPV6, SAFI_UNICAST, open::ADD_PATH_RECEIVE)]);
    s.extend(v6_route(&p, &["2001:db8::/32"], &[64501], Some(0)));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    assert_eq!(h.table().len(), 1);
    assert_eq!(h.text(g, "2001:db8::/32", "path_id").unwrap(), "0");
    assert_eq!(h.text(g, "::/0", "origin_as"), None);
}

#[tokio::test]
async fn a_malformed_attribute_withdraws_what_it_announces() {
    let h = Harness::new();
    let p = global_peer();
    let mut s = start(&p, &[]);
    s.extend(v4_route(&p, &["10.0.0.0/8"], &[64501], "192.0.2.1"));
    s.extend(b::route_monitoring(
        &p,
        &u::update(
            &[],
            &[u::origin(0), u::attr(u::WELL_KNOWN, 3, &[1, 2])],
            &u::nlri(&["10.0.0.0/8"], None),
        ),
    ));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    assert!(h.table().is_empty());
    assert_eq!(h.stat(|s| &s.treat_as_withdraw), 1);
}

#[tokio::test]
async fn confederation_paths_are_kept() {
    let h = Harness::new();
    let p = global_peer();
    let mut s = start(&p, &[]);
    s.extend(b::route_monitoring(
        &p,
        &u::update(
            &[],
            &[
                u::origin(0),
                u::as_path(&[(3, &[65001, 65002]), (2, &[64500, 64501])]),
                u::next_hop_attr("192.0.2.1"),
            ],
            &u::nlri(&["10.0.0.0/8"], None),
        ),
    ));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    assert_eq!(
        h.text("global", "10.0.0.0/8", "as_path").unwrap(),
        "(65001 65002) 64500 64501"
    );
    assert_eq!(h.text("global", "10.0.0.0/8", "peer_as").unwrap(), "64500");
}

#[tokio::test]
async fn faults_end_the_session() {
    let h = Harness::new();
    let p = global_peer();

    // Initiation must come first
    assert_eq!(h.run(&b::peer_up(&p, &[])).await, SessionEnd::NoInitiation);

    // an oversized length cannot be framed
    let mut s = b::initiation("r1");
    let mut big = b::termination(0);
    big[1..5].copy_from_slice(&(2u32 << 20).to_be_bytes());
    s.extend(big);
    assert_eq!(h.run(&s).await, SessionEnd::Frame("length"));

    // an UPDATE whose structure is broken hides its prefixes
    let mut s = start(&p, &[]);
    let mut body = u::update(&[], &[u::origin(0)], &[]);
    body[0..2].copy_from_slice(&500u16.to_be_bytes());
    s.extend(b::route_monitoring(&p, &body));
    assert!(matches!(h.run(&s).await, SessionEnd::Reset(_)));

    // a truncated Loc-RIB Peer Up
    let mut s = b::initiation("r1");
    s.extend(b::frame(bmp::PEER_UP, &p));
    assert!(matches!(h.run(&s).await, SessionEnd::Reset(_)));
}

#[tokio::test]
async fn routes_before_peer_up_and_other_families_are_skipped() {
    let h = Harness::new();
    let p = global_peer();
    let mut s = b::initiation("r1");
    s.extend(v4_route(&p, &["10.0.0.0/8"], &[64501], "192.0.2.1"));
    s.extend(start(&p, &[])[b::initiation("r1").len()..].to_vec());
    s.extend(b::route_monitoring(
        &p,
        &u::update(
            &[],
            &[
                u::origin(0),
                u::mp_reach(AFI_IPV4, 128, &[0; 12], &[1, 2, 3]),
            ],
            &[],
        ),
    ));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    assert!(h.table().is_empty());
    assert_eq!(h.stat(|s| &s.without_peer_up), 1);
    assert_eq!(
        h.collector.stats().ignored_families(),
        [(Some((AFI_IPV4, 128)), 1)]
    );
}

#[tokio::test]
async fn without_a_table_routes_are_counted_only() {
    let h = Harness::new();
    h.registry.remove(&h.config.id);
    let p = global_peer();
    let mut s = start(&p, &[]);
    s.extend(v4_route(&p, &["10.0.0.0/8"], &[64501], "192.0.2.1"));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    assert_eq!(h.stat(|s| &s.messages[bmp::ROUTE_MONITORING as usize]), 1);
    assert_eq!(h.stat(|s| &s.routes_announced), 0);
}

#[tokio::test]
async fn a_table_change_mid_stream_forces_a_resync() {
    let h = Harness::new();
    let p = global_peer();
    let (mut tx, rx) = tokio::io::duplex(1 << 16);
    let collector = h.collector.clone();
    let router = h.router();
    let session = tokio::spawn(async move {
        collector
            .session(rx, router, CancellationToken::new())
            .await
    });
    let mut s = start(&p, &[]);
    s.extend(v4_route(&p, &["10.0.0.0/8"], &[64501], "192.0.2.1"));
    tx.write_all(&s).await.unwrap();
    eventually(|| h.table().len() == 1).await;

    // an edit that cannot keep the rows (another listener) rebuilds it, and
    // this listener no longer has a table
    h.registry.remove(&h.config.id);
    for _ in 0..super::session::TABLE_CHECK_MESSAGES {
        // the session hangs up as soon as it notices
        if tx.write_all(&b::initiation("r1")).await.is_err() {
            break;
        }
    }
    assert_eq!(session.await.unwrap(), SessionEnd::Resync);
}

#[tokio::test(start_paused = true)]
async fn keep_expiry_sweeps_a_router_that_stays_away() {
    let h = Harness::with_keep(Duration::from_secs(60));
    let p = global_peer();
    let mut s = start(&p, &[]);
    s.extend(v4_route(&p, &["10.0.0.0/8"], &[64501], "192.0.2.1"));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    assert_eq!(h.table().len(), 1);

    let token = CancellationToken::new();
    let collector = h.collector.clone();
    let expiry = tokio::spawn({
        let token = token.clone();
        async move { collector.run_expiry(token).await }
    });
    tokio::time::sleep(Duration::from_secs(59)).await;
    assert_eq!(h.table().len(), 1, "kept while the router may come back");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(h.table().is_empty());
    assert_eq!(h.stat(|s| &s.keep_expiries), 1);
    token.cancel();
    expiry.await.unwrap();
}

/// A Loc-RIB Peer Up for an emulated peer carrying `afi` unicast only, as a
/// router with one emulated peer per family sends (RFC 9069 §6.1.1).
fn family_peer_up(peer: &[u8], afi: u16) -> Vec<u8> {
    let list = [
        caps::multiprotocol(afi, SAFI_UNICAST),
        caps::four_octet(64500),
    ];
    b::peer_up(
        peer,
        &build_open(64500, Ipv4Addr::new(192, 0, 2, 254), &list),
    )
}

/// One emulated peer of two goes down: the Peer Down does not say which
/// family, so the routes stay, the next message ends the session for a new
/// dump, and `keep` then sweeps the family the router no longer sends.
#[tokio::test(start_paused = true)]
async fn a_family_peer_down_resyncs_and_keep_sweeps_the_gone_family() {
    let h = Harness::with_keep(Duration::from_secs(60));
    let p = global_peer();
    let mut s = b::initiation("r1");
    s.extend(family_peer_up(&p, AFI_IPV4));
    s.extend(family_peer_up(&p, AFI_IPV6));
    s.extend(v4_route(&p, &["10.0.0.0/8"], &[64501], "192.0.2.1"));
    s.extend(v6_route(&p, &["2001:db8::/32"], &[64501], None));
    s.extend(b::peer_down(&p, 6));
    s.extend(v4_route(&p, &["10.1.0.0/16"], &[64501], "192.0.2.1"));
    assert_eq!(h.run(&s).await, SessionEnd::Resync);
    assert_eq!(h.table().len(), 2, "both families kept until the new dump");
    assert_eq!(h.stat(|s| &s.routes_withdrawn), 0);
    assert_eq!(h.stat(|s| &s.without_peer_up), 0);

    // the router comes back with its IPv4 peer only
    tokio::time::sleep(Duration::from_secs(10)).await;
    let mut s = b::initiation("r1");
    s.extend(family_peer_up(&p, AFI_IPV4));
    s.extend(v4_route(&p, &["10.0.0.0/8"], &[64501], "192.0.2.1"));
    s.extend(v4_route(&p, &["10.1.0.0/16"], &[64501], "192.0.2.1"));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    assert_eq!(h.table().len(), 3);
    let now = tokio::time::Instant::now();
    assert_eq!(h.collector.expire(now + Duration::from_secs(61)), 1);
    assert_eq!(h.cell("global", "2001:db8::/32", "origin_as"), None);
    assert_eq!(h.table().len(), 2);
}

/// Every emulated peer of an instance going down in one burst removes it at
/// the last Peer Down, without a resync.
#[tokio::test]
async fn the_last_family_peer_down_takes_the_instance_whole() {
    let h = Harness::new();
    let p = global_peer();
    let mut s = b::initiation("r1");
    s.extend(family_peer_up(&p, AFI_IPV4));
    s.extend(family_peer_up(&p, AFI_IPV6));
    s.extend(v4_route(&p, &["10.0.0.0/8"], &[64501], "192.0.2.1"));
    s.extend(v6_route(&p, &["2001:db8::/32"], &[64501], None));
    s.extend(b::peer_down(&p, 6));
    s.extend(b::peer_down(&p, 6));
    s.extend(v4_route(&p, &["10.1.0.0/16"], &[64501], "192.0.2.1"));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    assert!(h.table().is_empty());
    assert_eq!(h.stat(|s| &s.routes_withdrawn), 2);
    assert_eq!(h.stat(|s| &s.without_peer_up), 1);
}

#[tokio::test(start_paused = true)]
async fn keep_expiry_sweeps_what_a_reconnect_did_not_resend() {
    let h = Harness::with_keep(Duration::from_secs(60));
    let p = global_peer();
    let red = b::loc_rib(b::rd0(64500, 1));
    let mut s = start(&p, &[]);
    s.extend(start(&red, &[])[b::initiation("r1").len()..].to_vec());
    s.extend(v4_route(
        &p,
        &["10.0.0.0/8", "10.1.0.0/16"],
        &[64501],
        "192.0.2.1",
    ));
    s.extend(v4_route(&red, &["10.0.0.0/8"], &[64501], "192.0.2.1"));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    assert_eq!(h.table().len(), 3);

    // the router comes back without the red VRF and without End-of-RIB
    tokio::time::sleep(Duration::from_secs(10)).await;
    let (mut tx, rx) = tokio::io::duplex(1 << 16);
    let collector = h.collector.clone();
    let router = h.router();
    let session = tokio::spawn(async move {
        collector
            .session(rx, router, CancellationToken::new())
            .await
    });
    let mut s = start(&p, &[]);
    s.extend(v4_route(&p, &["10.0.0.0/8"], &[64501], "192.0.2.1"));
    tx.write_all(&s).await.unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;

    // the deadline was pushed to a full keep after the reconnect
    assert_eq!(
        h.collector
            .expire(tokio::time::Instant::now() + Duration::from_secs(55)),
        0
    );
    let swept = h
        .collector
        .expire(tokio::time::Instant::now() + Duration::from_secs(61));
    assert_eq!(swept, 2, "the stale /16 and the red VRF");
    assert_eq!(h.table().len(), 1);
    drop(tx);
    assert_eq!(session.await.unwrap(), SessionEnd::Eof);
}

#[tokio::test]
async fn a_reconnect_replaces_the_live_session() {
    let h = Harness::new();
    let p = global_peer();
    let (mut tx, rx) = tokio::io::duplex(1 << 16);
    let collector = h.collector.clone();
    let router = h.router();
    let old = tokio::spawn(async move {
        collector
            .session(rx, router, CancellationToken::new())
            .await
    });
    tx.write_all(&start(&p, &[])).await.unwrap();
    eventually(|| h.collector.routers().is_live(router)).await;

    let mut s = start(&p, &[]);
    s.extend(v4_route(&p, &["10.0.0.0/8"], &[64501], "192.0.2.1"));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    assert_eq!(old.await.unwrap(), SessionEnd::Cancelled);
    assert_eq!(h.stat(|s| &s.sessions_superseded[Protocol::Bmp.index()]), 1);
    assert_eq!(h.table().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn keep_expiry_sweeps_what_a_replacing_session_did_not_resend() {
    let h = Harness::with_keep(Duration::from_secs(60));
    let p = global_peer();
    let router = h.router();
    let open = |rx| {
        let collector = h.collector.clone();
        tokio::spawn(async move {
            collector
                .session(rx, router, CancellationToken::new())
                .await
        })
    };

    // the first session stays up (a half-open TCP connection, say)
    let (mut old_tx, rx) = tokio::io::duplex(1 << 16);
    let old = open(rx);
    let mut s = start(&p, &[]);
    s.extend(v4_route(
        &p,
        &["10.0.0.0/8", "10.1.0.0/16"],
        &[64501],
        "192.0.2.1",
    ));
    old_tx.write_all(&s).await.unwrap();
    eventually(|| h.table().len() == 2).await;

    // the router reconnects while it lasts, re-sends one route, and never
    // sends End-of-RIB (GoBGP over BMP sends none)
    tokio::time::sleep(Duration::from_secs(10)).await;
    let (mut tx, rx) = tokio::io::duplex(1 << 16);
    let new = open(rx);
    let mut s = start(&p, &[]);
    s.extend(v4_route(&p, &["10.0.0.0/8"], &[64501], "192.0.2.1"));
    tx.write_all(&s).await.unwrap();
    assert_eq!(old.await.unwrap(), SessionEnd::Cancelled);
    tokio::time::sleep(Duration::from_millis(10)).await;

    // the replacing session got a whole keep, then the stale /16 goes
    let now = tokio::time::Instant::now();
    assert_eq!(h.collector.expire(now + Duration::from_secs(55)), 0);
    assert_eq!(h.collector.expire(now + Duration::from_secs(61)), 1);
    assert_eq!(h.table().len(), 1);
    assert_eq!(
        h.text("global", "10.0.0.0/8", "origin_as").unwrap(),
        "64501"
    );
    assert_eq!(h.text("global", "10.1.0.0/16", "origin_as"), None);
    drop(tx);
    assert_eq!(new.await.unwrap(), SessionEnd::Eof);
}

// ── events, VRF filter, a collector's successor ───────────────────────────

/// A sink of `buffer` events, and its receiving end.
fn sink(buffer: usize, routes: bool, on_full: OnFull) -> (EventSink, flume::Receiver<Event>) {
    let (tx, rx) = flume::bounded(buffer);
    (EventSink::new(tx, routes, on_full, Arc::default()), rx)
}

fn names(rx: &flume::Receiver<Event>) -> Vec<&'static str> {
    rx.drain().map(|e| e.kind.name()).collect()
}

#[tokio::test]
async fn events_follow_the_stream() {
    let (events, rx) = sink(100, true, OnFull::Drop);
    let h = Harness::with(BmpConfig::default(), Some(events));
    let p = global_peer();
    let mut s = start(&p, &[]);
    s.extend(v4_route(
        &p,
        &["10.0.0.0/8", "10.1.0.0/16"],
        &[64501, 64502],
        "192.0.2.1",
    ));
    s.extend(v4_withdraw(&p, &["10.1.0.0/16"]));
    s.extend(eor_v4(&p));
    s.extend(b::peer_down(&p, 2));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    let events: Vec<_> = rx.drain().collect();
    let kinds: Vec<_> = events.iter().map(|e| e.kind.name()).collect();
    assert_eq!(
        kinds,
        [
            "session_up",
            "peer_up",
            "announce",
            "announce",
            "withdraw",
            "end_of_rib",
            "peer_down",
            "session_down"
        ]
    );
    assert!(events.iter().all(|e| e.router == h.router()));
    let EventKind::Announce { prefix, cells } = &events[2].kind else {
        unreachable!()
    };
    assert_eq!(prefix.to_string(), "10.0.0.0/8");
    // the cells are the row's: router, vrf, next hop, origin AS, ...
    assert_eq!(cells[1], CellOwned::String("global".into()));
    assert_eq!(cells[3], CellOwned::Integer(64502));
    assert!(matches!(
        &events[5].kind,
        EventKind::EndOfRib { rows: 0, .. }
    ));
    assert!(matches!(
        &events[6].kind,
        EventKind::PeerDown { rows: 1, .. }
    ));
    assert_eq!(
        events[7].kind,
        EventKind::SessionDown {
            reason: "eof",
            error: None,
            notification: None,
            message: None,
        }
    );
}

#[tokio::test]
async fn route_events_are_opt_in_and_sent_without_a_table() {
    let (events, rx) = sink(100, false, OnFull::Drop);
    let h = Harness::with(BmpConfig::default(), Some(events));
    let p = global_peer();
    let mut s = start(&p, &[]);
    s.extend(v4_route(&p, &["10.0.0.0/8"], &[64501], "192.0.2.1"));
    s.extend(eor_v4(&p));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    assert_eq!(
        names(&rx),
        ["session_up", "peer_up", "end_of_rib", "session_down"]
    );

    // no table: the routes still go out as events
    let (events, rx) = sink(100, true, OnFull::Drop);
    let collector = Collector::new(
        Uuid::now_v7(),
        Arc::new(LookupTable::new()),
        BmpConfig::default().into(),
    )
    .with_events(events);
    let mut s = start(&p, &[]);
    s.extend(v4_route(&p, &["10.0.0.0/8"], &[64501], "192.0.2.1"));
    s.extend(v4_withdraw(&p, &["10.0.0.0/8"]));
    let end = collector
        .session(&s[..], h.router(), CancellationToken::new())
        .await;
    assert_eq!(end, SessionEnd::Eof);
    assert_eq!(
        names(&rx),
        [
            "session_up",
            "peer_up",
            "announce",
            "withdraw",
            "session_down"
        ]
    );
}

#[tokio::test]
async fn a_full_channel_drops_events_without_holding_the_routes() {
    let (events, rx) = sink(2, true, OnFull::Drop);
    let h = Harness::with(BmpConfig::default(), Some(events));
    let p = global_peer();
    let mut s = start(&p, &[]);
    s.extend(v4_route(
        &p,
        &["10.0.0.0/8", "10.1.0.0/16", "10.2.0.0/16"],
        &[64501],
        "192.0.2.1",
    ));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    assert_eq!(h.table().len(), 3);
    assert_eq!(names(&rx), ["session_up", "peer_up"]);
    // three announcements and the session's end
    assert_eq!(h.collector.events().unwrap().dropped(), 4);
}

#[tokio::test]
async fn wait_holds_the_session_until_its_events_are_taken() {
    let (events, rx) = sink(1, true, OnFull::Wait);
    let h = Arc::new(Harness::with(BmpConfig::default(), Some(events)));
    let p = global_peer();
    let mut s = start(&p, &[]);
    s.extend(v4_route(&p, &["10.0.0.0/8"], &[64501], "192.0.2.1"));
    s.extend(v4_route(&p, &["10.1.0.0/16"], &[64501], "192.0.2.1"));

    let (mut tx, stream) = tokio::io::duplex(1 << 16);
    tx.write_all(&s).await.unwrap();
    drop(tx);
    let collector = h.collector.clone();
    let router = h.router();
    let session = tokio::spawn(async move {
        collector
            .session(stream, router, CancellationToken::new())
            .await
    });
    // session_up fills the channel; peer_up waits, and with it the routes
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!session.is_finished());
    assert!(h.table().is_empty(), "the routes wait behind the events");

    let mut seen = Vec::new();
    while let Ok(e) = rx.recv_async().await {
        seen.push(e.kind.name());
        if seen.len() == 5 {
            break;
        }
    }
    assert_eq!(session.await.unwrap(), SessionEnd::Eof);
    assert_eq!(
        seen,
        [
            "session_up",
            "peer_up",
            "announce",
            "announce",
            "session_down"
        ]
    );
    assert_eq!(h.table().len(), 2);

    // a stuck pipeline cannot hold a cancel
    let cancel = CancellationToken::new();
    let collector = h.collector.clone();
    let session = tokio::spawn({
        let cancel = cancel.clone();
        let s = s.clone();
        async move { collector.session(&s[..], router, cancel).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!session.is_finished());
    cancel.cancel();
    assert_eq!(session.await.unwrap(), SessionEnd::Cancelled);
}

#[tokio::test]
async fn vrfs_not_kept_are_counted_only() {
    let (events, rx) = sink(100, false, OnFull::Drop);
    let h = Harness::with(
        CollectorConfig {
            vrfs: Some(["global".into()].into_iter().collect()),
            ..CollectorConfig::bmp(BmpConfig::default())
        },
        Some(events),
    );
    let global = global_peer();
    let red = b::loc_rib(b::rd0(64500, 1));
    let mut s = start(&global, &[]);
    s.extend(start(&red, &[])[b::initiation("r1").len()..].to_vec());
    s.extend(v4_route(&global, &["10.0.0.0/8"], &[64501], "192.0.2.1"));
    s.extend(v4_route(&red, &["10.0.0.0/8"], &[64511], "192.0.2.1"));
    s.extend(b::peer_down(&red, 6));
    assert_eq!(h.run(&s).await, SessionEnd::Eof);
    assert_eq!(h.table().len(), 1);
    assert_eq!(h.cell("64500:1", "10.0.0.0/8", "origin_as"), None);
    assert_eq!(h.stat(|s| &s.vrf_filtered), 3, "peer up, route, peer down");
    assert_eq!(names(&rx), ["session_up", "peer_up", "session_down"]);
}

#[tokio::test(start_paused = true)]
async fn a_new_collector_expires_what_the_old_one_left() {
    let keep = Duration::from_secs(60);
    let h = Harness::with_keep(keep);
    let p = global_peer();
    let (gone, back): (IpAddr, IpAddr) = (
        "198.51.100.1".parse().unwrap(),
        "198.51.100.2".parse().unwrap(),
    );
    for router in [gone, back] {
        let mut s = start(&p, &[]);
        s.extend(v4_route(
            &p,
            &["10.0.0.0/8", "10.1.0.0/16"],
            &[64501],
            "192.0.2.1",
        ));
        let end = h
            .collector
            .session(&s[..], router, CancellationToken::new())
            .await;
        assert_eq!(end, SessionEnd::Eof);
    }
    assert_eq!(h.table().len(), 4);

    // the transport restarts: the old collector and its deadlines are gone
    tokio::time::sleep(Duration::from_millis(5)).await;
    let listener = match &h.config.source {
        cfg::Source::Bgp(b) => b.listener,
        _ => unreachable!(),
    };
    let next = Arc::new(Collector::new(
        listener,
        h.registry.clone(),
        BmpConfig {
            keep,
            ..BmpConfig::default()
        }
        .into(),
    ));
    assert_eq!(
        next.routers().next_deadline(),
        Some(tokio::time::Instant::now() + keep)
    );

    // one router comes back and re-sends half its routes
    let (mut tx, rx) = tokio::io::duplex(1 << 16);
    let session = tokio::spawn({
        let next = next.clone();
        async move { next.session(rx, back, CancellationToken::new()).await }
    });
    let mut s = start(&p, &[]);
    s.extend(v4_route(&p, &["10.0.0.0/8"], &[64501], "192.0.2.1"));
    tx.write_all(&s).await.unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;

    let swept = next.expire(tokio::time::Instant::now() + keep);
    assert_eq!(
        swept, 3,
        "all of the router that stayed away, the stale half of the other"
    );
    let table = h.table();
    assert_eq!(table.len(), 1);
    assert!(
        table
            .row_cells(back, "global", "10.0.0.0/8".parse().unwrap())
            .is_some()
    );
    drop(tx);
    assert_eq!(session.await.unwrap(), SessionEnd::Eof);
}

/// Bit flips, truncations and length tweaks of real and crafted messages,
/// through a whole session: nothing a router sends may panic.
#[tokio::test]
async fn mutated_messages_never_panic() {
    let h = Harness::new();
    let p = global_peer();
    let mut seeds: Vec<Vec<u8>> = ALL_CAPTURES
        .iter()
        .flat_map(|n| frames(&capture(n)))
        .collect();
    seeds.push(v4_route(
        &p,
        &["10.0.0.0/8", "10.1.0.0/16"],
        &[64501, 64502],
        "192.0.2.1",
    ));
    seeds.push(v6_route(&p, &["2001:db8::/32"], &[64501], Some(3)));
    seeds.push(b::route_monitoring(
        &p,
        &u::update(
            &u::nlri(&["10.2.0.0/16"], None),
            &[
                u::origin(0),
                u::as_path(&[(3, &[65001]), (1, &[64500, 64501])]),
                u::as4_path(&[(2, &[4_200_000_000])]),
                u::communities(&[1, 2]),
                u::large_communities(&[[1, 2, 3]]),
                u::med(1),
            ],
            &u::nlri(&["10.3.0.0/16"], None),
        ),
    ));
    let preamble = start(&p, &[(AFI_IPV6, SAFI_UNICAST, 3)]);

    let mut rng = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    for _ in 0..20_000 {
        let seed = &seeds[next() as usize % seeds.len()];
        let mut m = seed.clone();
        match next() % 4 {
            0 => {
                let at = next() as usize % m.len();
                m[at] ^= 1 << (next() % 8);
            }
            1 => m.truncate(next() as usize % m.len()),
            2 => {
                // keep the frame length right so the body is what breaks
                let at = 6 + next() as usize % (m.len().saturating_sub(6)).max(1);
                if at < m.len() {
                    m[at] = next() as u8;
                }
            }
            _ => {
                let at = next() as usize % m.len();
                m[at] = 0xff;
            }
        }
        let mut s = preamble.clone();
        s.extend(m);
        let _ = h.run(&s).await;
    }
}

// ── differential: NetGauze encodes, we decode ───────────────────────────

mod differential {
    use super::super::wire::Afi;
    use super::super::wire::update::{AsPath, SegmentKind, Update, prefixes};
    use ipnet::{IpNet, Ipv4Net, Ipv6Net};
    use netgauze_bgp_pkt::community::{Community, LargeCommunity};
    use netgauze_bgp_pkt::nlri::{
        Ipv4Unicast, Ipv4UnicastAddress, Ipv6Unicast, Ipv6UnicastAddress,
    };
    use netgauze_bgp_pkt::path_attribute as pa;
    use netgauze_bgp_pkt::update::BgpUpdateMessage;
    use netgauze_parse_utils::WritablePdu;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }

        fn chance(&mut self) -> bool {
            self.next() & 1 == 0
        }
    }

    /// What a generated UPDATE carries, and what decoding it must give back.
    #[derive(Debug, Default, PartialEq)]
    struct Routes {
        withdrawn: Vec<(Option<u32>, IpNet)>,
        nlri: Vec<(Option<u32>, IpNet)>,
        mp_nlri: Vec<(Option<u32>, IpNet)>,
        mp_next_hop: Option<IpAddr>,
        next_hop: Option<Ipv4Addr>,
        path: Vec<(SegmentKind, Vec<u32>)>,
        communities: Vec<u32>,
        large: Vec<[u32; 3]>,
        med: Option<u32>,
        local_pref: Option<u32>,
    }

    fn v4(rng: &mut Rng) -> Option<Ipv4Net> {
        // unicast space only: NetGauze refuses multicast and broadcast
        let addr = Ipv4Addr::from((rng.next() as u32) & 0xdfff_ffff);
        let net = Ipv4Net::new(addr, rng.below(33) as u8).ok()?.trunc();
        Ipv4Unicast::from_net(net).ok().map(|_| net)
    }

    fn v6(rng: &mut Rng) -> Option<Ipv6Net> {
        let addr =
            Ipv6Addr::from(((rng.next() as u128) << 64 | rng.next() as u128) & !(0xff << 120));
        let net = Ipv6Net::new(addr, rng.below(129) as u8).ok()?.trunc();
        Ipv6Unicast::from_net(net).ok().map(|_| net)
    }

    fn attr(optional: bool, transitive: bool, value: pa::PathAttributeValue) -> pa::PathAttribute {
        pa::PathAttribute::from(optional, transitive, false, true, value).unwrap()
    }

    fn v4_list(
        rng: &mut Rng,
        n: u64,
        add_path: bool,
        out: &mut Vec<(Option<u32>, IpNet)>,
    ) -> Vec<Ipv4UnicastAddress> {
        let mut list = Vec::new();
        for _ in 0..n {
            if let Some(net) = v4(rng) {
                let pid = add_path.then(|| rng.below(8) as u32);
                out.push((pid, IpNet::V4(net)));
                list.push(Ipv4UnicastAddress::new(
                    pid,
                    Ipv4Unicast::from_net(net).unwrap(),
                ));
            }
        }
        list
    }

    fn generate(rng: &mut Rng, add_path: [bool; 2]) -> (Vec<u8>, Routes) {
        let mut want = Routes::default();
        let n = rng.below(3);
        let withdrawn = v4_list(rng, n, add_path[0], &mut want.withdrawn);
        let mut attrs = vec![attr(
            false,
            true,
            pa::PathAttributeValue::Origin(pa::Origin::IGP),
        )];
        let mut segments = Vec::new();
        for _ in 0..rng.below(4) {
            let (kind, ng) = if rng.chance() {
                (SegmentKind::Sequence, pa::AsPathSegmentType::AsSequence)
            } else {
                (SegmentKind::Set, pa::AsPathSegmentType::AsSet)
            };
            let len = 1 + rng.below(5);
            let asns: Vec<u32> = (0..len).map(|_| rng.next() as u32).collect();
            want.path.push((kind, asns.clone()));
            segments.push(pa::As4PathSegment::new(ng, asns));
        }
        attrs.push(attr(
            false,
            true,
            pa::PathAttributeValue::AsPath(pa::AsPath::As4PathSegments(segments.into())),
        ));
        let n = rng.below(4);
        let nlri = v4_list(rng, n, add_path[0], &mut want.nlri);
        if !nlri.is_empty() || rng.chance() {
            let nh = Ipv4Addr::from(rng.next() as u32);
            want.next_hop = Some(nh);
            attrs.push(attr(
                false,
                true,
                pa::PathAttributeValue::NextHop(pa::NextHop::new(nh)),
            ));
        }
        if rng.chance() {
            let v = rng.next() as u32;
            want.med = Some(v);
            attrs.push(attr(
                true,
                false,
                pa::PathAttributeValue::MultiExitDiscriminator(pa::MultiExitDiscriminator::new(v)),
            ));
        }
        if rng.chance() {
            let v = rng.next() as u32;
            want.local_pref = Some(v);
            attrs.push(attr(
                false,
                true,
                pa::PathAttributeValue::LocalPreference(pa::LocalPreference::new(v)),
            ));
        }
        if rng.chance() {
            let len = 1 + rng.below(4);
            want.communities = (0..len).map(|_| rng.next() as u32).collect();
            let list: Vec<Community> = want
                .communities
                .iter()
                .map(|c| Community::new(*c))
                .collect();
            attrs.push(attr(
                true,
                true,
                pa::PathAttributeValue::Communities(pa::Communities::new(list)),
            ));
        }
        if rng.chance() {
            let len = 1 + rng.below(3);
            want.large = (0..len)
                .map(|_| [rng.next() as u32, rng.next() as u32, rng.next() as u32])
                .collect();
            let list: Vec<LargeCommunity> = want
                .large
                .iter()
                .map(|[a, b, c]| LargeCommunity::new(*a, *b, *c))
                .collect();
            attrs.push(attr(
                true,
                true,
                pa::PathAttributeValue::LargeCommunities(pa::LargeCommunities::new(list)),
            ));
        }
        if rng.chance() {
            let global = Ipv6Addr::from((rng.next() as u128) << 64 | 1);
            want.mp_next_hop = Some(IpAddr::V6(global));
            let mut list = Vec::new();
            for _ in 0..rng.below(4) {
                if let Some(net) = v6(rng) {
                    let pid = add_path[1].then(|| rng.below(8) as u32);
                    want.mp_nlri.push((pid, IpNet::V6(net)));
                    list.push(Ipv6UnicastAddress::new(
                        pid,
                        Ipv6Unicast::from_net(net).unwrap(),
                    ));
                }
            }
            attrs.push(attr(
                true,
                false,
                pa::PathAttributeValue::MpReach(pa::MpReach::Ipv6Unicast {
                    next_hop_global: global,
                    next_hop_local: None,
                    nlri: list.into(),
                }),
            ));
        }
        let message = BgpUpdateMessage::new(withdrawn, attrs, nlri);
        let mut body = Vec::new();
        message.write(&mut body).unwrap();
        (body, want)
    }

    fn decode(body: &[u8], add_path: [bool; 2]) -> Routes {
        let u = Update::parse(body, true, add_path).unwrap();
        let a = u.attrs;
        assert_eq!(a.withdraw, None);
        let list = |bytes, family: Afi| -> Vec<(Option<u32>, IpNet)> {
            prefixes(bytes, family, add_path[family.index()])
                .map(Result::unwrap)
                .collect()
        };
        let mut path = AsPath::default();
        path.decode(a.as_path.unwrap(), true).unwrap();
        Routes {
            withdrawn: list(u.withdrawn, Afi::V4),
            nlri: list(u.nlri, Afi::V4),
            mp_nlri: a
                .mp_reach
                .map(|r| list(r.nlri, Afi::V6))
                .unwrap_or_default(),
            mp_next_hop: a.mp_reach.and_then(|r| r.next_hop),
            next_hop: a.next_hop,
            path: path
                .segments()
                .map(|(k, asns)| (k, asns.to_vec()))
                .collect(),
            communities: a.communities().collect(),
            large: a.large_communities().collect(),
            med: a.med,
            local_pref: a.local_pref,
        }
    }

    #[test]
    fn netgauze_encoded_updates_decode_to_what_was_encoded() {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        for case in 0..5_000 {
            let add_path = [rng.chance(), rng.chance()];
            let (body, want) = generate(&mut rng, add_path);
            assert_eq!(decode(&body, add_path), want, "case {case}");
        }
    }
}

/// The listener on a real socket, run as `WorkerGroup` runs it.
mod transport {
    use super::*;
    use crate::config::listener::{Bgp, BgpEvents, BgpPeer, BgpPeering, Bmp, TcpListener};
    use crate::listener::bgp::BgpListener;
    use crate::listener::bgp::codec::BgpFrames;
    use crate::listener::bgp::wire::BGP_OPEN;
    use crate::listener::bgp::wire::bgp::{self as msg, BGP_KEEPALIVE, BGP_NOTIFICATION};
    use crate::listener::test_util::{self, CLOSE_LIMIT, Port, item_for};
    use crate::listener::{DRAIN_TIMEOUT, Listener as _};
    use crate::{cache, config};
    use futures::StreamExt;
    use std::net::SocketAddr;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpStream;
    use tokio_util::codec::FramedRead;

    type Server = test_util::Harness<BgpListener>;

    /// A listener with a route table, its records belonging to one item so
    /// they reach the pipeline.
    /// A cache whose item takes the records of listener `id`, with a route
    /// table filled by it.
    fn routes_cache(id: Uuid) -> cache::Cache {
        let cache = item_for(id);
        let table = cfg::LookupTable {
            id: Uuid::now_v7(),
            name: "routes".into(),
            source: cfg::Source::Bgp(cfg::Bgp {
                listener: id,
                max_rows: 1000,
            }),
            key: cfg::Bgp::key(),
        };
        cache.lookup_table.insert(&table).unwrap();
        cache
    }

    /// A BMP listener `id` on `addr`, with per-route events when `routes`.
    fn bmp_conf(
        id: Uuid,
        addr: SocketAddr,
        whitelist: Vec<IpAddr>,
        routes: bool,
    ) -> config::listener::Listener {
        let mut bgp = Bgp {
            bgp: None,
            bmp: Some(Bmp {
                listener: TcpListener {
                    address: addr,
                    ip_whitelist: whitelist,
                    ip_blacklist: Vec::new(),
                },
                ..Bmp::default()
            }),
            vrfs: Vec::new(),
            events: BgpEvents {
                routes,
                ..BgpEvents::default()
            },
        };
        bgp.validate().unwrap();
        config::listener::Listener::new(
            id,
            "routes".into(),
            test_util::WORKERS,
            config::Batch {
                batch_size: 100,
                batch_timeout: Duration::from_millis(50),
            },
            config::listener::Config::Bgp(Box::new(bgp)),
            false,
        )
    }

    async fn server(whitelist: &[&str]) -> (Server, cache::Cache, Uuid) {
        let id = Uuid::now_v7();
        let cache = routes_cache(id);
        let whitelist: Vec<IpAddr> = whitelist.iter().map(|ip| ip.parse().unwrap()).collect();
        let build_cache = cache.clone();
        let server = Server::start(Port::Tcp, DRAIN_TIMEOUT, move |addr: SocketAddr, sender| {
            let conf = bmp_conf(id, addr, whitelist.clone(), true);
            let listener = BgpListener::new(conf.clone(), sender, None, build_cache.clone(), None);
            (conf, listener)
        })
        .await;
        (server, cache, id)
    }

    async fn wait_for(what: &str, check: impl AsyncFn() -> bool) {
        let deadline = std::time::Instant::now() + CLOSE_LIMIT;
        while !check().await {
            assert!(std::time::Instant::now() < deadline, "{what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_bmp_connection_fills_the_table_and_feeds_the_pipeline() {
        let (server, cache, id) = server(&[]).await;
        let (rows, consumer) = server.consume();
        let feed = capture("locrib");
        let mut router = TcpStream::connect(server.addr).await.unwrap();
        router.write_all(&feed).await.unwrap();

        let table = || cache.lookup_table.route_table_for_listener(&id).unwrap();
        wait_for("routes written", async || {
            server
                .listener
                .metrics()
                .await
                .bgp
                .unwrap()
                .routes_announced
                == 5
        })
        .await;
        assert_eq!(table().len(), 4, "two paths of one prefix show as one row");
        let m = server.listener.metrics().await;
        assert_eq!(m.total_bytes, feed.len() as u64);
        assert_eq!(m.total_items, frames(&feed).len() as u64);
        let bgp = m.bgp.unwrap();
        assert_eq!(bgp.routers_live, 1);
        assert!(bgp.route_table);
        assert_eq!(bgp.sessions_opened, [("bmp", 1)]);

        // session_up, peer_up, one record per route change
        let want = 2 + bgp.routes_announced as usize + bgp.routes_withdrawn as usize;
        test_util::Harness::<BgpListener>::wait_for_rows(&rows, want).await;

        // a closing listener ends the session but keeps its routes: nothing
        // is armed to sweep them
        let (elapsed, _) = server.close(false).await;
        assert!(elapsed < CLOSE_LIMIT);
        assert_eq!(table().len(), 4);
        let mut buf = [0; 16];
        let read = tokio::time::timeout(CLOSE_LIMIT, router.read(&mut buf)).await;
        assert!(matches!(read, Ok(Ok(0)) | Ok(Err(_))), "{read:?}");
        consumer.abort();
    }

    /// Removed or disabled (the watcher closes it either way), a bgp listener
    /// leaves no one to withdraw or expire its routes, so its table empties.
    /// An edit restarts it, and the new transport keeps them under `keep`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn removing_the_listener_empties_its_table_and_an_edit_does_not() {
        let id = Uuid::now_v7();
        let cache = routes_cache(id);
        let (_admin, sender, _rx) = test_util::processor_topic(1024).await;
        let group = crate::listener::WorkerGroup::new(sender, None, cache.clone(), None);
        let addr = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        group
            .add(bmp_conf(id, addr, Vec::new(), false))
            .await
            .unwrap();

        let table = || cache.lookup_table.route_table_for_listener(&id).unwrap();
        // the transport binds once a worker starts it
        let deadline = std::time::Instant::now() + CLOSE_LIMIT;
        let mut router = loop {
            match TcpStream::connect(addr).await {
                Ok(router) => break router,
                Err(e) => assert!(std::time::Instant::now() < deadline, "{e}"),
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        router.write_all(&capture("locrib")).await.unwrap();
        wait_for("routes written", async || table().len() == 4).await;

        group
            .add(bmp_conf(id, addr, Vec::new(), true))
            .await
            .unwrap();
        assert_eq!(table().len(), 4, "an edit keeps the routes");

        group.close_listener(id).await.unwrap();
        assert!(table().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_ip_filter_turns_routers_away() {
        let (server, _cache, _) = server(&["192.0.2.1"]).await;
        let mut router = TcpStream::connect(server.addr).await.unwrap();
        let _ = router.write_all(&capture("locrib")).await;
        let mut buf = [0; 16];
        let read = tokio::time::timeout(CLOSE_LIMIT, router.read(&mut buf)).await;
        assert!(matches!(read, Ok(Ok(0)) | Ok(Err(_))), "{read:?}");
        let bgp = server.listener.metrics().await.bgp.unwrap();
        assert_eq!(bgp.sessions_opened, [("bmp", 0)]);
        assert_eq!(bgp.routers_live, 0);
        server.close(true).await;
    }

    /// A listener with only a `bgp` section, and a route table.
    async fn bgp_server(peers: Vec<BgpPeer>) -> (Server, cache::Cache, Uuid) {
        let id = Uuid::now_v7();
        let cache = item_for(id);
        let table = cfg::LookupTable {
            id: Uuid::now_v7(),
            name: "routes".into(),
            source: cfg::Source::Bgp(cfg::Bgp {
                listener: id,
                max_rows: 1000,
            }),
            key: cfg::Bgp::key(),
        };
        cache.lookup_table.insert(&table).unwrap();
        let build_cache = cache.clone();
        let server = Server::start(Port::Tcp, DRAIN_TIMEOUT, move |addr: SocketAddr, sender| {
            let mut bgp = Bgp {
                bmp: None,
                bgp: Some(BgpPeering {
                    listener: TcpListener {
                        address: addr,
                        ip_whitelist: Vec::new(),
                        ip_blacklist: Vec::new(),
                    },
                    peers: peers.clone(),
                    ..BgpPeering::default()
                }),
                vrfs: Vec::new(),
                events: BgpEvents::default(),
            };
            bgp.validate().unwrap();
            let conf = config::listener::Listener::new(
                id,
                "routes".into(),
                test_util::WORKERS,
                config::Batch {
                    batch_size: 100,
                    batch_timeout: Duration::from_millis(50),
                },
                config::listener::Config::Bgp(Box::new(bgp)),
                false,
            );
            let listener = BgpListener::new(conf.clone(), sender, None, build_cache.clone(), None);
            (conf, listener)
        })
        .await;
        (server, cache, id)
    }

    /// Plays a router over `stream`: OPEN, then the KEEPALIVE once patrolli
    /// answered with its own.
    async fn establish(stream: TcpStream) -> FramedRead<TcpStream, BgpFrames> {
        let mut frames = FramedRead::new(stream, BgpFrames::default());
        let open = msg::message(
            BGP_OPEN,
            &open::open_body(
                64500,
                90,
                Ipv4Addr::new(198, 51, 100, 1),
                &[
                    caps::multiprotocol(AFI_IPV4, SAFI_UNICAST),
                    caps::four_octet(64500),
                ],
            ),
        );
        frames.get_mut().write_all(&open).await.unwrap();
        let kind = |f: &bytes::Bytes| f[18];
        let ours = frames.next().await.unwrap().unwrap();
        assert_eq!(kind(&ours), BGP_OPEN);
        assert_eq!(kind(&frames.next().await.unwrap().unwrap()), BGP_KEEPALIVE);
        frames.get_mut().write_all(&msg::keepalive()).await.unwrap();
        frames
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_bgp_peer_fills_the_table_and_is_told_when_the_listener_stops() {
        let (server, cache, id) = bgp_server(Vec::new()).await;
        let mut router = establish(TcpStream::connect(server.addr).await.unwrap()).await;
        let update = u::message(&u::update(
            &[],
            &[
                u::origin(0),
                u::as_path(&[(2, &[64501, 64502])]),
                u::next_hop_attr("192.0.2.1"),
            ],
            &u::nlri(&["10.0.0.0/8", "10.1.0.0/16"], None),
        ));
        router.get_mut().write_all(&update).await.unwrap();

        let table = || cache.lookup_table.route_table_for_listener(&id).unwrap();
        wait_for("routes written", async || table().len() == 2).await;
        let m = server.listener.metrics().await;
        let bgp = m.bgp.unwrap();
        assert_eq!(bgp.sessions_opened, [("bgp", 1)]);
        assert_eq!(bgp.routers_live, 1);
        assert_eq!(m.total_items, 3, "open, keepalive, update");
        assert!(bgp.messages.contains(&("bgp", "update", 1)));

        // stopping tells the router why, and keeps the routes
        server.close(false).await;
        let mut notified = None;
        while let Some(Ok(f)) = router.next().await {
            if f[18] == BGP_NOTIFICATION {
                notified = Some((f[19], f[20]));
            }
        }
        assert_eq!(notified, Some((6, 2)));
        assert_eq!(table().len(), 2);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_peer_with_a_password_must_sign_its_segments() {
        use crate::config::sensitive::Sensitive;
        use socket2::{Domain, Socket, Type};
        let (server, _cache, _) = bgp_server(vec![BgpPeer {
            address: "127.0.0.1".parse().unwrap(),
            password: Some(Sensitive::Plain("s3cret".into())),
            asn: None,
            hold_time: None,
        }])
        .await;
        // the listener starts with its first worker
        wait_for("bgp server up", async || {
            server.listener.metrics().await.total_workers > 0
        })
        .await;
        let addr = server.addr;
        let connect = move |key: Option<&'static [u8]>| {
            tokio::task::spawn_blocking(move || {
                let s = Socket::new(Domain::IPV4, Type::STREAM, None)?;
                if let Some(key) = key {
                    s.bind(&SocketAddr::new(addr.ip(), 0).into())?;
                    crate::listener::bgp::md5::set_key(&s, addr.ip(), key)?;
                }
                s.connect_timeout(&addr.into(), Duration::from_millis(500))?;
                s.set_nonblocking(true)?;
                Ok::<_, std::io::Error>(std::net::TcpStream::from(s))
            })
        };
        assert!(
            connect(None).await.unwrap().is_err(),
            "an unsigned syn is dropped"
        );
        let signed = connect(Some(b"s3cret")).await.unwrap().expect("signed");
        establish(TcpStream::from_std(signed).unwrap()).await;
        server.close(true).await;
    }
}
