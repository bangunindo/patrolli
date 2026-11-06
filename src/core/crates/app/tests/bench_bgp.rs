// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! BMP and passive BGP session throughput, per core, no Docker needed.
//!
//! Turns an MRT RIB dump into the streams a router would send, then replays
//! them through [`Collector::session`] (BMP) and [`Collector::bgp_session`]
//! (BGP) on one current-thread runtime, so every number is one core's:
//!
//! * the BMP stream is Initiation, a Loc-RIB Peer Up, one Route Monitoring
//!   per UPDATE and End-of-RIB for both families (RFC 9069);
//! * the BGP stream is the router's OPEN and KEEPALIVE, the same UPDATEs and
//!   End-of-RIB. patrolli's answers go to a sink.
//!
//! The UPDATEs carry the routes of the dump's peer with the most IPv4 routes
//! and of its peer with the most IPv6 routes, standing for one router's
//! Loc-RIB (or of the peers `BENCH_PEER` names), packed the way a router
//! packs its dump:
//! prefixes that share their attributes go in one UPDATE up to 4096 bytes,
//! or at most `BENCH_PACK` of them. Optional churn (`BENCH_UPDATES`) is every
//! UPDATE of the MRT update files, from every peer, replayed as the same
//! router's after its dump.
//!
//! Phases, each per protocol:
//!
//! * `decode`: no route table, so the session frames and parses every message
//!   and stops there;
//! * `dump`: the dump into an empty table;
//! * `redump`: the router reconnects and dumps the same routes again;
//! * `churn`: the update files, after a dump.
//!
//! Reported per phase: messages/s and routes/s (best of `BENCH_ROUNDS`), the
//! session thread's CPU time against the wall clock (1.00 = bound on that one
//! core), and allocations per message. In the default in-memory mode the
//! stream is read from a buffer, so the figures are the session's own cost;
//! `BENCH_TCP=1` sends it over loopback TCP from another thread instead,
//! adding the socket reads.
//!
//! Not an assertion, and never downloads: point `BENCH_MRT` at a RIB dump
//! (a RIPE RIS `bview.*.gz`, or any TABLE_DUMP_V2 file, gzip or plain).
//!
//! ```bash
//! BENCH_MRT=~/data/ris/bview.20261105.0000.gz BENCH_CPU=3 \
//!     cargo test --release --test bench_bgp -- --ignored --nocapture
//! ```
//!
//! Knobs (env): `BENCH_MRT` (required), `BENCH_UPDATES` comma-separated MRT
//! update files (none), `BENCH_PEER` comma-separated peer addresses whose
//! routes make the dump (the top IPv4 and the top IPv6 peer), `BENCH_PACK`
//! most prefixes per UPDATE, 0 = as many as fit (0), `BENCH_ROUNDS` (3), `BENCH_PROTOCOLS` (`bmp,bgp`), `BENCH_PHASES`
//! (`decode,dump,redump,churn`), `BENCH_CPU` the CPU the session runs on
//! (unpinned), `BENCH_TCP` (0), `BENCH_SENDER_CPU` the TCP sender's CPU
//! (unpinned), `BENCH_EVENTS` `routes` to send per-route events to a
//! draining thread, as `events.routes: true` does (off), `BENCH_STREAM_OUT`
//! a directory to also write each protocol's streams to
//! (`<protocol>-{dump,churn}.bin`), for replaying into a real binary, e.g.
//! one whose heap is being profiled (unset), `BENCH_PERF_CTL` a perf control
//! FIFO, so that `perf record -D -1 --control fifo:<it>` samples only the
//! timed sessions (unset).

// sched_setaffinity and CLOCK_THREAD_CPUTIME_ID are Linux's.
#![cfg(all(target_os = "linux", not(target_env = "msvc")))]

use patrolli::cache::lookup_table::LookupTable;
use patrolli::config::listener::OnFull;
use patrolli::config::lookup_table as cfg;
use patrolli::listener::bgp::events::EventSink;
use patrolli::listener::bgp::stats::Stats;
use patrolli::listener::bgp::{BgpConfig, BmpConfig, Collector, CollectorConfig, SessionEnd};
use rustc_hash::FxHashMap;
use std::alloc::{GlobalAlloc, Layout};
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tikv_jemallocator::Jemalloc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// jemalloc, counting the allocations made.
struct Counting;

static ALLOCS: AtomicU64 = AtomicU64::new(0);

// SAFETY: every call is delegated unchanged to jemalloc.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { Jemalloc.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { Jemalloc.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { Jemalloc.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { Jemalloc.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn env<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn pin(cpu: Option<usize>) {
    let Some(cpu) = cpu else { return };
    // SAFETY: a zeroed cpu_set_t is the empty set; 0 is the calling thread.
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(cpu, &mut set);
        assert_eq!(
            libc::sched_setaffinity(0, size_of::<libc::cpu_set_t>(), &set),
            0,
            "pin to cpu {cpu}: {}",
            std::io::Error::last_os_error()
        );
    }
}

/// CPU time of the calling thread.
fn thread_cpu() -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: ts is a valid timespec to write.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

/// Tells `perf record -D -1 --control fifo:<BENCH_PERF_CTL>` to sample only
/// the timed sessions.
fn perf_ctl(command: &str) {
    if let Ok(fifo) = std::env::var("BENCH_PERF_CTL") {
        let mut f = std::fs::OpenOptions::new().write(true).open(fifo).unwrap();
        writeln!(f, "{command}").unwrap();
    }
}

// --- MRT -------------------------------------------------------------------

fn open_mrt(path: &str) -> Vec<u8> {
    let file = BufReader::new(File::open(path).unwrap_or_else(|e| panic!("{path}: {e}")));
    let mut reader: Box<dyn Read> = if path.ends_with(".gz") {
        Box::new(flate2::read::MultiGzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut out = Vec::new();
    reader.read_to_end(&mut out).unwrap();
    out
}

fn be16(b: &[u8], o: usize) -> usize {
    u16::from_be_bytes([b[o], b[o + 1]]) as usize
}

fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes(b[o..o + 4].try_into().unwrap())
}

/// The records of an MRT file: (type, subtype, body), the ET microseconds
/// dropped.
fn records(mrt: &[u8]) -> impl Iterator<Item = (u16, u16, &[u8])> {
    let mut off = 0;
    std::iter::from_fn(move || {
        if off + 12 > mrt.len() {
            return None;
        }
        let kind = be16(mrt, off + 4) as u16;
        let subtype = be16(mrt, off + 6) as u16;
        let len = be32(mrt, off + 8) as usize;
        let mut body = &mrt[off + 12..off + 12 + len];
        off += 12 + len;
        if matches!(kind, 17 | 33 | 49) {
            body = &body[4..];
        }
        Some((kind, subtype, body))
    })
}

const TABLE_DUMP_V2: u16 = 13;
const BGP4MP: u16 = 16;
const BGP4MP_ET: u16 = 17;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    V4,
    V6,
}

/// One route of the dump: its prefix in NLRI encoding, and its attributes
/// with a full MP_REACH_NLRI's next hop taken out.
struct RibRoute<'a> {
    family: Family,
    nlri: Vec<u8>,
    attrs: Vec<u8>,
    /// IPv6's next hop, from the abbreviated MP_REACH_NLRI (RFC 6396 §4.3.4)
    next_hop: &'a [u8],
}

/// Iterates (flags, type, value) over path attributes.
fn attributes(mut b: &[u8]) -> impl Iterator<Item = (u8, u8, &[u8])> {
    std::iter::from_fn(move || {
        if b.len() < 3 {
            return None;
        }
        let (flags, kind) = (b[0], b[1]);
        let (len, hdr) = if flags & 0x10 != 0 {
            (be16(b, 2), 4)
        } else {
            (b[2] as usize, 3)
        };
        let value = &b[hdr..hdr + len];
        b = &b[hdr + len..];
        Some((flags, kind, value))
    })
}

fn push_attr(out: &mut Vec<u8>, flags: u8, kind: u8, value: &[u8]) {
    if value.len() > 255 {
        out.extend_from_slice(&[flags | 0x10, kind]);
        out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    } else {
        out.extend_from_slice(&[flags & !0x10, kind, value.len() as u8]);
    }
    out.extend_from_slice(value);
}

/// The chosen peers, and every RIB entry of theirs.
fn read_rib<'a>(
    mrt: &'a [u8],
    peer: Option<Vec<IpAddr>>,
) -> (Vec<(IpAddr, u32)>, Vec<RibRoute<'a>>) {
    let mut peers: Vec<(IpAddr, u32)> = Vec::new();
    let mut entries: Vec<(u16, Family, &[u8], &[u8])> = Vec::new();
    for (kind, subtype, body) in records(mrt) {
        if kind != TABLE_DUMP_V2 {
            continue;
        }
        match subtype {
            1 => {
                let name_len = be16(body, 4);
                let mut o = 6 + name_len;
                let count = be16(body, o);
                o += 2;
                for _ in 0..count {
                    let ty = body[o];
                    o += 5;
                    let addr = if ty & 1 != 0 {
                        let a: [u8; 16] = body[o..o + 16].try_into().unwrap();
                        o += 16;
                        IpAddr::V6(Ipv6Addr::from(a))
                    } else {
                        let a: [u8; 4] = body[o..o + 4].try_into().unwrap();
                        o += 4;
                        IpAddr::V4(Ipv4Addr::from(a))
                    };
                    let asn = if ty & 2 != 0 {
                        o += 4;
                        be32(body, o - 4)
                    } else {
                        o += 2;
                        be16(body, o - 2) as u32
                    };
                    peers.push((addr, asn));
                }
            }
            2 | 4 => {
                let family = if subtype == 2 { Family::V4 } else { Family::V6 };
                let bits = body[4] as usize;
                let nlri = &body[4..5 + bits.div_ceil(8)];
                let mut o = 5 + bits.div_ceil(8);
                let count = be16(body, o);
                o += 2;
                for _ in 0..count {
                    let index = be16(body, o) as u16;
                    let attr_len = be16(body, o + 6);
                    entries.push((index, family, nlri, &body[o + 8..o + 8 + attr_len]));
                    o += 8 + attr_len;
                }
            }
            _ => {}
        }
    }
    // a router's Loc-RIB holds both families: by default the peer with the
    // most IPv4 routes and the one with the most IPv6 routes
    let chosen: Vec<u16> = match peer {
        Some(addrs) => addrs
            .iter()
            .map(|addr| {
                peers
                    .iter()
                    .position(|(a, _)| a == addr)
                    .unwrap_or_else(|| panic!("peer {addr} not in the dump")) as u16
            })
            .collect(),
        None => [Family::V4, Family::V6]
            .into_iter()
            .filter_map(|family| {
                let mut counts = vec![0usize; peers.len()];
                for e in entries.iter().filter(|e| e.1 == family) {
                    counts[e.0 as usize] += 1;
                }
                (0..counts.len())
                    .filter(|&i| counts[i] > 0)
                    .max_by_key(|&i| counts[i])
                    .map(|i| i as u16)
            })
            .collect(),
    };
    let routes = entries
        .into_iter()
        .filter(|e| chosen.contains(&e.0))
        .filter_map(|(_, family, nlri, raw)| {
            let mut attrs = Vec::with_capacity(raw.len());
            let mut next_hop: &[u8] = &[];
            for (flags, kind, value) in attributes(raw) {
                if kind == 14 {
                    // abbreviated: next hop length, next hop
                    let len = *value.first()? as usize;
                    next_hop = value.get(1..1 + len)?;
                } else {
                    push_attr(&mut attrs, flags, kind, value);
                }
            }
            if family == Family::V6 && next_hop.is_empty() {
                return None;
            }
            Some(RibRoute {
                family,
                nlri: nlri.to_vec(),
                attrs,
                next_hop,
            })
        })
        .collect();
    let chosen = chosen.iter().map(|&i| peers[i as usize]).collect();
    (chosen, routes)
}

/// The UPDATE bodies of every BGP4MP_MESSAGE_AS4 in `mrt`.
fn read_updates(mrt: &[u8], out: &mut Vec<Vec<u8>>) {
    for (kind, subtype, body) in records(mrt) {
        if !matches!(kind, BGP4MP | BGP4MP_ET) || subtype != 4 {
            continue;
        }
        let afi = be16(body, 10);
        let msg = &body[12 + if afi == 2 { 32 } else { 8 }..];
        if msg.len() > 19 && msg[18] == 2 && msg.len() <= 4096 {
            out.push(msg[19..].to_vec());
        }
    }
}

// --- streams ---------------------------------------------------------------

const MAX_BGP: usize = 4096;
const BGP_HEADER: usize = 19;

fn update_body(withdrawn: &[u8], attrs: &[u8], nlri: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + withdrawn.len() + attrs.len() + nlri.len());
    out.extend_from_slice(&(withdrawn.len() as u16).to_be_bytes());
    out.extend_from_slice(withdrawn);
    out.extend_from_slice(&(attrs.len() as u16).to_be_bytes());
    out.extend_from_slice(attrs);
    out.extend_from_slice(nlri);
    out
}

fn mp_reach_v6(next_hop: &[u8], nlri: &[u8]) -> Vec<u8> {
    let mut v = vec![0, 2, 1, next_hop.len() as u8];
    v.extend_from_slice(next_hop);
    v.push(0);
    v.extend_from_slice(nlri);
    v
}

/// The dump as UPDATE bodies: prefixes sharing attributes and next hop go in
/// one UPDATE, up to `pack` of them (0 = as many as fit).
fn pack_dump(routes: &[RibRoute<'_>], pack: usize) -> (Vec<Vec<u8>>, usize) {
    struct Group<'r> {
        family: Family,
        attrs: &'r [u8],
        next_hop: &'r [u8],
        nlri: Vec<u8>,
        prefixes: usize,
    }
    fn emit(g: &mut Group<'_>, out: &mut Vec<Vec<u8>>) {
        if g.prefixes == 0 {
            return;
        }
        out.push(match g.family {
            Family::V4 => update_body(&[], g.attrs, &g.nlri),
            Family::V6 => {
                let mut attrs = Vec::new();
                push_attr(&mut attrs, 0x80, 14, &mp_reach_v6(g.next_hop, &g.nlri));
                attrs.extend_from_slice(g.attrs);
                update_body(&[], &attrs, &[])
            }
        });
        g.nlri.clear();
        g.prefixes = 0;
    }
    let mut groups: Vec<Group<'_>> = Vec::new();
    let mut by_key: FxHashMap<(bool, &[u8], &[u8]), usize> = FxHashMap::default();
    let mut out = Vec::new();
    for r in routes {
        let key = (r.family == Family::V6, r.attrs.as_slice(), r.next_hop);
        let i = *by_key.entry(key).or_insert_with(|| {
            groups.push(Group {
                family: r.family,
                attrs: &r.attrs,
                next_hop: r.next_hop,
                nlri: Vec::new(),
                prefixes: 0,
            });
            groups.len() - 1
        });
        let g = &mut groups[i];
        // header, two lengths, attributes, an extended MP_REACH header
        let fixed = BGP_HEADER + 4 + g.attrs.len() + 4 + 5 + g.next_hop.len();
        if (pack > 0 && g.prefixes >= pack) || fixed + g.nlri.len() + r.nlri.len() > MAX_BGP {
            emit(g, &mut out);
        }
        g.nlri.extend_from_slice(&r.nlri);
        g.prefixes += 1;
    }
    for g in &mut groups {
        emit(g, &mut out);
    }
    (out, routes.len())
}

fn end_of_rib() -> [Vec<u8>; 2] {
    let mut v6 = Vec::new();
    push_attr(&mut v6, 0x80, 15, &[0, 2, 1]);
    [update_body(&[], &[], &[]), update_body(&[], &v6, &[])]
}

fn bgp_message(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![0xff; 16];
    out.extend_from_slice(&((BGP_HEADER + body.len()) as u16).to_be_bytes());
    out.push(kind);
    out.extend_from_slice(body);
    out
}

fn open_body(asn: u32, hold: u16, id: Ipv4Addr) -> Vec<u8> {
    let caps: &[&[u8]] = &[
        &[1, 4, 0, 1, 0, 1],
        &[1, 4, 0, 2, 0, 1],
        &[2, 0],
        &[
            65,
            4,
            (asn >> 24) as u8,
            (asn >> 16) as u8,
            (asn >> 8) as u8,
            asn as u8,
        ],
    ];
    let caps: Vec<u8> = caps.concat();
    let mut out = vec![4];
    out.extend_from_slice(&(u16::try_from(asn).unwrap_or(23456)).to_be_bytes());
    out.extend_from_slice(&hold.to_be_bytes());
    out.extend_from_slice(&id.octets());
    out.push(caps.len() as u8 + 2);
    out.push(2);
    out.push(caps.len() as u8);
    out.extend_from_slice(&caps);
    out
}

fn bmp_frame(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![3];
    out.extend_from_slice(&((6 + body.len()) as u32).to_be_bytes());
    out.push(kind);
    out.extend_from_slice(body);
    out
}

fn loc_rib_header(asn: u32, id: Ipv4Addr) -> Vec<u8> {
    let mut out = vec![3, 0];
    out.extend_from_slice(&[0; 8 + 16]);
    out.extend_from_slice(&asn.to_be_bytes());
    out.extend_from_slice(&id.octets());
    out.extend_from_slice(&[0; 8]);
    out
}

/// A router's stream of `updates`, and how many messages it holds.
fn bmp_stream(asn: u32, id: Ipv4Addr, updates: &[&[u8]]) -> (Vec<u8>, usize) {
    let peer = loc_rib_header(asn, id);
    let mut out = bmp_frame(4, &[0, 2, 0, 5, b'b', b'e', b'n', b'c', b'h']);
    let open = bgp_message(1, &open_body(asn, 90, id));
    let mut up = peer.clone();
    up.extend_from_slice(&[0; 20]);
    up.extend_from_slice(&open);
    up.extend_from_slice(&open);
    out.extend_from_slice(&bmp_frame(3, &up));
    let mut body = Vec::new();
    for u in updates {
        body.clear();
        body.extend_from_slice(&peer);
        body.extend_from_slice(&bgp_message(2, u));
        out.extend_from_slice(&bmp_frame(0, &body));
    }
    (out, updates.len() + 2)
}

fn bgp_stream(asn: u32, id: Ipv4Addr, updates: &[&[u8]]) -> (Vec<u8>, usize) {
    let mut out = bgp_message(1, &open_body(asn, 90, id));
    out.extend_from_slice(&bgp_message(4, &[]));
    for u in updates {
        out.extend_from_slice(&bgp_message(2, u));
    }
    (out, updates.len() + 2)
}

// --- running ---------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Protocol {
    Bmp,
    Bgp,
}

struct Bench {
    protocol: Protocol,
    tcp: bool,
    sender_cpu: Option<usize>,
    asn: u32,
    router: IpAddr,
    rt: tokio::runtime::Runtime,
}

struct Run {
    wall: Duration,
    cpu: Duration,
    allocs: u64,
    end: SessionEnd,
}

impl Bench {
    /// Runs one session over `stream` to its end.
    fn session(&self, c: &Collector, stream: Arc<[u8]>) -> Run {
        let cancel = CancellationToken::new();
        let local = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        let router = self.router;
        let protocol = self.protocol;
        perf_ctl("enable");
        let (allocs, cpu, started) = (ALLOCS.load(Ordering::Relaxed), thread_cpu(), Instant::now());
        let end = if self.tcp {
            let sender_cpu = self.sender_cpu;
            self.rt.block_on(async {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                let sender = std::thread::spawn(move || {
                    pin(sender_cpu);
                    let mut s = TcpStream::connect(addr).unwrap();
                    s.write_all(&stream).unwrap();
                    s.shutdown(std::net::Shutdown::Write).unwrap();
                    // hold the socket until patrolli is done answering
                    let _ = std::io::copy(&mut s, &mut std::io::sink());
                });
                let (sock, _) = listener.accept().await.unwrap();
                let end = match protocol {
                    Protocol::Bmp => c.session(sock, router, cancel).await,
                    Protocol::Bgp => c.bgp_session(sock, router, local, cancel).await,
                };
                sender.join().unwrap();
                end
            })
        } else {
            self.rt.block_on(async {
                let rd = Cursor::new(stream);
                match protocol {
                    Protocol::Bmp => c.session(rd, router, cancel).await,
                    Protocol::Bgp => {
                        let io = tokio::io::join(rd, tokio::io::sink());
                        c.bgp_session(io, router, local, cancel).await
                    }
                }
            })
        };
        let run = Run {
            wall: started.elapsed(),
            cpu: thread_cpu() - cpu,
            allocs: ALLOCS.load(Ordering::Relaxed) - allocs,
            end,
        };
        perf_ctl("disable");
        run
    }

    fn collector(&self, with_table: bool) -> Collector {
        let listener = Uuid::now_v7();
        let tables = Arc::new(LookupTable::new());
        if with_table {
            tables
                .insert(&cfg::LookupTable {
                    id: Uuid::now_v7(),
                    name: "routes".into(),
                    source: cfg::Source::Bgp(cfg::Bgp {
                        listener,
                        max_rows: 4_000_000,
                    }),
                    key: cfg::Bgp::key(),
                })
                .unwrap();
        }
        let config = CollectorConfig {
            bmp: Some(BmpConfig::default()),
            bgp: Some(BgpConfig {
                // RIPE RIS: the router is an external peer
                asn: Some(if self.asn == 12654 { 64512 } else { 12654 }),
                ..BgpConfig::default()
            }),
            vrfs: None,
        };
        let c = Collector::new(listener, tables, config);
        if std::env::var("BENCH_EVENTS").as_deref() == Ok("routes") {
            let (tx, rx) = flume::bounded(65536);
            std::thread::spawn(move || while rx.recv().is_ok() {});
            c.with_events(EventSink::new(
                tx,
                true,
                OnFull::Wait,
                Arc::new(AtomicU64::new(0)),
            ))
        } else {
            c
        }
    }

    fn stream(&self, updates: &[&[u8]]) -> (Arc<[u8]>, usize) {
        let id = Ipv4Addr::new(198, 51, 100, 1);
        let (bytes, n) = match self.protocol {
            Protocol::Bmp => bmp_stream(self.asn, id, updates),
            Protocol::Bgp => bgp_stream(self.asn, id, updates),
        };
        (bytes.into(), n)
    }
}

fn counters(s: &Stats) -> [u64; 3] {
    [
        s.routes_announced.load(Ordering::Relaxed),
        s.routes_withdrawn.load(Ordering::Relaxed),
        s.unknown_withdraws.load(Ordering::Relaxed),
    ]
}

struct Row {
    messages: usize,
    bytes: usize,
    runs: Vec<(Run, [u64; 3])>,
}

fn report(protocol: Protocol, phase: &str, row: Row) {
    let best = row
        .runs
        .iter()
        .min_by_key(|(r, _)| r.wall)
        .expect("one round at least");
    let (run, [announced, withdrawn, unknown]) = best;
    let secs = run.wall.as_secs_f64();
    let routes = announced + withdrawn + unknown;
    println!(
        "{:<4} {phase:<7} {:>9} msg {:>8.1} MiB  {:>7.0}k msg/s {:>7.0}k route/s {:>5.0} ns/msg {:>6.1} MiB/s  cpu/wall {:.2}  {:>5.1} alloc/msg  ann {announced} wd {withdrawn} unknown-wd {unknown}  end {:?}",
        format!("{protocol:?}").to_lowercase(),
        row.messages,
        row.bytes as f64 / (1 << 20) as f64,
        row.messages as f64 / secs / 1e3,
        routes as f64 / secs / 1e3,
        secs * 1e9 / row.messages as f64,
        row.bytes as f64 / (1 << 20) as f64 / secs,
        run.cpu.as_secs_f64() / secs,
        run.allocs as f64 / row.messages as f64,
        run.end,
    );
}

#[test]
#[ignore = "harness: needs BENCH_MRT, run with --release --ignored --nocapture"]
fn bench_bgp() {
    let Ok(path) = std::env::var("BENCH_MRT") else {
        eprintln!("bench_bgp: set BENCH_MRT to an MRT RIB dump; skipping");
        return;
    };
    let rounds: usize = env("BENCH_ROUNDS", 3).max(1);
    let pack: usize = env("BENCH_PACK", 0);
    let cpu: Option<usize> = std::env::var("BENCH_CPU").ok().map(|v| v.parse().unwrap());
    let sender_cpu: Option<usize> = std::env::var("BENCH_SENDER_CPU")
        .ok()
        .map(|v| v.parse().unwrap());
    let tcp = env("BENCH_TCP", 0) != 0;
    let protocols: String = env("BENCH_PROTOCOLS", "bmp,bgp".to_string());
    let phases: String = env("BENCH_PHASES", "decode,dump,redump,churn".to_string());
    let peer: Option<Vec<IpAddr>> = std::env::var("BENCH_PEER")
        .ok()
        .map(|v| v.split(',').map(|a| a.parse().unwrap()).collect());

    let started = Instant::now();
    let mrt = open_mrt(&path);
    let (peers, routes) = read_rib(&mrt, peer);
    // the first peer's AS stands for the router's
    let asn = peers[0].1;
    let v4 = routes.iter().filter(|r| r.family == Family::V4).count();
    let (dump, prefixes) = pack_dump(&routes, pack);
    println!(
        "peers {peers:?}: {prefixes} routes ({v4} v4, {} v6) in {} UPDATEs ({:.1} prefixes each), read in {:.1?}",
        prefixes - v4,
        dump.len(),
        prefixes as f64 / dump.len() as f64,
        started.elapsed()
    );
    let [eor4, eor6] = end_of_rib();
    let mut dump_updates: Vec<&[u8]> = dump.iter().map(Vec::as_slice).collect();
    dump_updates.extend([eor4.as_slice(), eor6.as_slice()]);

    let mut churn: Vec<Vec<u8>> = Vec::new();
    for f in std::env::var("BENCH_UPDATES")
        .unwrap_or_default()
        .split(',')
        .filter(|f| !f.is_empty())
    {
        read_updates(&open_mrt(f), &mut churn);
    }
    if !churn.is_empty() {
        println!("churn: {} UPDATEs", churn.len());
    }
    let churn_updates: Vec<&[u8]> = churn.iter().map(Vec::as_slice).collect();
    println!(
        "session cpu {cpu:?}, {}, {rounds} round(s), best shown",
        if tcp { "loopback tcp" } else { "in memory" }
    );

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::scope(|s| {
        s.spawn(|| {
            pin(cpu);
            for protocol in protocols.split(',') {
                let protocol = match protocol {
                    "bmp" => Protocol::Bmp,
                    "bgp" => Protocol::Bgp,
                    other => panic!("protocol {other}"),
                };
                let bench = Bench {
                    protocol,
                    tcp,
                    sender_cpu,
                    asn,
                    router: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
                    rt: tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap(),
                };
                let (dump_stream, dump_messages) = bench.stream(&dump_updates);
                let (churn_stream, churn_messages) = bench.stream(&churn_updates);
                if let Ok(dir) = std::env::var("BENCH_STREAM_OUT") {
                    let name = format!("{protocol:?}").to_lowercase();
                    std::fs::write(format!("{dir}/{name}-dump.bin"), &dump_stream).unwrap();
                    std::fs::write(format!("{dir}/{name}-churn.bin"), &churn_stream).unwrap();
                }
                for phase in phases.split(',') {
                    let mut row = Row {
                        messages: dump_messages,
                        bytes: dump_stream.len(),
                        runs: Vec::new(),
                    };
                    for _ in 0..rounds {
                        let (c, run) = match phase {
                            "decode" => {
                                let c = bench.collector(false);
                                let run = bench.session(&c, dump_stream.clone());
                                (c, run)
                            }
                            "dump" => {
                                let c = bench.collector(true);
                                let run = bench.session(&c, dump_stream.clone());
                                (c, run)
                            }
                            "redump" => {
                                let c = bench.collector(true);
                                bench.session(&c, dump_stream.clone());
                                let before = counters(c.stats());
                                let run = bench.session(&c, dump_stream.clone());
                                let after = counters(c.stats());
                                row.runs
                                    .push((run, std::array::from_fn(|i| after[i] - before[i])));
                                continue;
                            }
                            "churn" => {
                                if churn.is_empty() {
                                    break;
                                }
                                row.messages = churn_messages;
                                row.bytes = churn_stream.len();
                                let c = bench.collector(true);
                                bench.session(&c, dump_stream.clone());
                                let before = counters(c.stats());
                                let run = bench.session(&c, churn_stream.clone());
                                let after = counters(c.stats());
                                row.runs
                                    .push((run, std::array::from_fn(|i| after[i] - before[i])));
                                continue;
                            }
                            other => panic!("phase {other}"),
                        };
                        let counts = counters(c.stats());
                        row.runs.push((run, counts));
                    }
                    if !row.runs.is_empty() {
                        tx.send((protocol, phase.to_string(), row)).unwrap();
                    }
                }
            }
            drop(tx);
        });
        for (protocol, phase, row) in rx {
            report(protocol, &phase, row);
        }
    });
}
