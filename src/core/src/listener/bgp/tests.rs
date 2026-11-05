// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Sessions end to end: captured and crafted BMP streams into a route table.

use super::wire::bmp::{self, Message, build as b};
use super::wire::open::{self, Open, build_open, caps};
use super::wire::update::{AsPath, SegmentKind, Update, build as u, prefixes};
use super::wire::{AFI_IPV4, AFI_IPV6, Afi, BGP_UPDATE, Reader, SAFI_UNICAST, bgp_message};
use super::{BmpConfig, Collector, SessionEnd, codec::BmpFrames, stats::BmpStats};
use crate::cache::lookup_table::LookupTable;
use crate::cache::lookup_table::bgp::RouteTable;
use crate::cache::lookup_table::cell::CellOwned;
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
                    modes[f.index()] = open::sends_path_ids(&received, f)
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
        let collector = Arc::new(Collector::new(
            listener,
            registry.clone(),
            BmpConfig {
                keep,
                ..BmpConfig::default()
            },
        ));
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

    fn stat(&self, f: impl Fn(&BmpStats) -> &std::sync::atomic::AtomicU64) -> u64 {
        BmpStats::get(f(self.collector.stats()))
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
        h.stat(|s| &s.sessions_closed[SessionEnd::Terminated.index()]),
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
        h.collector.stats().ignored_families().get(&(AFI_IPV4, 128)),
        Some(&1)
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
    assert_eq!(h.stat(|s| &s.sessions_superseded), 1);
    assert_eq!(h.table().len(), 1);
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
