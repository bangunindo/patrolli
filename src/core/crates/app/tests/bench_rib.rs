// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Route table harness: a real RIB dump in a `source.type: bgp` lookup table.
//!
//! Reads an MRT RIB dump (a RIPE RIS `bview.*.gz`, or any TABLE_DUMP_V2 file,
//! gzip or plain; decompress a RouteViews `.bz2` first), keeps the first route
//! seen for each prefix so the result looks like one router's Loc-RIB, and
//! loads it into a route table once per simulated router. It reports:
//!
//! * insert rate, and heap bytes per stored route (live jemalloc blocks,
//!   counted by a wrapping allocator; the parsed routes themselves excluded);
//! * longest-prefix-match rate, probing the way `attribute_lookup` does with
//!   a literal `vrf: global`: once at addresses inside known prefixes, once at
//!   uniformly random IPv4 addresses (with the hit ratio);
//! * the resync sweep of one router with half its routes stale;
//! * a peer down (one router's whole partition).
//!
//! Not an assertion, and never downloads: point `BENCH_MRT` at a dump.
//!
//! ```bash
//! BENCH_MRT=~/data/bview.20261101.0000.gz \
//!     cargo test --release --test bench_rib -- --ignored --nocapture
//! ```
//!
//! Knobs (env): `BENCH_MRT` (required), `BENCH_ROUTERS` routers sharing the
//! table, each with the full set (1), `BENCH_LIMIT` prefixes kept, 0 = all
//! (0), `BENCH_LOOKUPS` probes per pass (10000000).

// jemalloc, which the allocation count wraps, is not built for MSVC.
#![cfg(not(target_env = "msvc"))]

use bgpkit_parser::BgpkitParser;
use bgpkit_parser::models::{AsPathSegment, Community, ElemType, MetaCommunity};
use ipnet::IpNet;
use patrolli::cache::lookup_table::LookupTable;
use patrolli::cache::lookup_table::bgp::{PathSegment, RouteRow, RouteText};
use patrolli::cache::lookup_table::cell::{CellOwned, KeyRef, Lookup};
use patrolli::cache::lookup_table::mutable::{Upsert, now_millis};
use patrolli::config::lookup_table as cfg;
use rustc_hash::FxHashSet;
use std::alloc::{GlobalAlloc, Layout};
use std::fs::File;
use std::io::{BufReader, Read};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tikv_jemallocator::{Jemalloc, usable_size};
use uuid::Uuid;

/// jemalloc, counting the live bytes it hands out. Its own `stats.allocated`
/// is not available: the patrolli build leaves jemalloc's stats off.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call is delegated unchanged to jemalloc; the counter only
// reads the usable size of blocks jemalloc owns.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { Jemalloc.alloc(layout) };
        if !ptr.is_null() {
            LIVE.fetch_add(unsafe { usable_size(ptr) }, Ordering::Relaxed);
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { Jemalloc.alloc_zeroed(layout) };
        if !ptr.is_null() {
            LIVE.fetch_add(unsafe { usable_size(ptr) }, Ordering::Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(unsafe { usable_size(ptr) }, Ordering::Relaxed);
        unsafe { Jemalloc.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let old = unsafe { usable_size(ptr) };
        let new_ptr = unsafe { Jemalloc.realloc(ptr, layout, size) };
        if !new_ptr.is_null() {
            LIVE.fetch_sub(old, Ordering::Relaxed);
            LIVE.fetch_add(unsafe { usable_size(new_ptr) }, Ordering::Relaxed);
        }
        new_ptr
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

/// A route parsed out of the dump, owning what a [`RouteRow`] borrows.
struct Route {
    prefix: IpNet,
    next_hop: Option<IpAddr>,
    /// (kind, ASNs), kind as in [`segment`]
    path: Vec<(u8, Vec<u32>)>,
    communities: Vec<u32>,
    large: Vec<[u32; 3]>,
    local_pref: Option<u32>,
    med: Option<u32>,
}

impl Route {
    fn row<'a>(
        &'a self,
        router: IpAddr,
        vrf: &'a str,
        path: &'a mut Vec<PathSegment<'a>>,
    ) -> RouteRow<'a> {
        path.clear();
        path.extend(self.path.iter().map(|(kind, asns)| segment(*kind, asns)));
        RouteRow {
            router,
            vrf,
            prefix: self.prefix,
            next_hop: self.next_hop,
            as_path: path,
            communities: &self.communities,
            large_communities: &self.large,
            local_pref: self.local_pref,
            med: self.med,
            path_id: None,
        }
    }
}

fn segment(kind: u8, asns: &[u32]) -> PathSegment<'_> {
    match kind {
        0 => PathSegment::Sequence(asns),
        1 => PathSegment::Set(asns),
        2 => PathSegment::ConfedSequence(asns),
        _ => PathSegment::ConfedSet(asns),
    }
}

/// The well-known communities are their RFC 1997 values.
fn community(c: &Community) -> u32 {
    match c {
        Community::NoExport => 0xffff_ff01,
        Community::NoAdvertise => 0xffff_ff02,
        Community::NoExportSubConfed => 0xffff_ff03,
        Community::Custom(asn, value) => (asn.to_u32() << 16) | u32::from(*value),
    }
}

/// The first announcement of each prefix, in dump order.
fn read_rib(path: &str, limit: usize) -> Vec<Route> {
    let file = BufReader::new(File::open(path).unwrap_or_else(|e| panic!("{path}: {e}")));
    let reader: Box<dyn Read> = if path.ends_with(".gz") {
        Box::new(flate2::read::MultiGzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut seen = FxHashSet::default();
    let mut routes = Vec::new();
    for elem in BgpkitParser::from_reader(reader).into_elem_iter() {
        if elem.elem_type != ElemType::ANNOUNCE || !seen.insert(elem.prefix.prefix) {
            continue;
        }
        let path = elem
            .as_path
            .map(|p| {
                p.segments
                    .iter()
                    .map(|s| {
                        let (kind, asns) = match s {
                            AsPathSegment::AsSequence(a) => (0, a),
                            AsPathSegment::AsSet(a) => (1, a),
                            AsPathSegment::ConfedSequence(a) => (2, a),
                            AsPathSegment::ConfedSet(a) => (3, a),
                        };
                        (kind, asns.iter().map(|a| a.to_u32()).collect())
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut communities = Vec::new();
        let mut large = Vec::new();
        for c in elem.communities.iter().flatten() {
            match c {
                MetaCommunity::Plain(c) => communities.push(community(c)),
                MetaCommunity::Large(l) => {
                    large.push([l.global_admin, l.local_data[0], l.local_data[1]])
                }
                _ => {}
            }
        }
        routes.push(Route {
            prefix: elem.prefix.prefix,
            next_hop: elem.next_hop,
            path,
            communities,
            large,
            local_pref: elem.local_pref,
            med: elem.med,
        });
        if limit > 0 && routes.len() >= limit {
            break;
        }
    }
    routes
}

/// Heap bytes live right now, in jemalloc's size classes.
fn allocated() -> usize {
    LIVE.load(Ordering::Relaxed)
}

fn router(i: usize) -> IpAddr {
    IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + i as u32 + 1))
}

/// xorshift64*: deterministic, and no dependency on a `rand` API.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

/// An address inside `net`, past its network address where there is room.
fn inside(net: &IpNet, salt: u64) -> IpAddr {
    match net {
        IpNet::V4(n) => {
            let host_bits = 32 - u32::from(n.prefix_len());
            let offset = if host_bits == 0 {
                0
            } else {
                (salt as u32) & ((1u64 << host_bits) - 1) as u32
            };
            IpAddr::V4(Ipv4Addr::from(u32::from(n.network()) | offset))
        }
        IpNet::V6(n) => {
            let host_bits = 128 - u32::from(n.prefix_len());
            let offset = if host_bits == 0 {
                0
            } else {
                u128::from(salt) & (u128::MAX >> (128 - host_bits.min(64)))
            };
            IpAddr::V6((u128::from(n.network()) | offset).into())
        }
    }
}

fn rate(n: usize, elapsed: Duration) -> String {
    format!("{:.2}M/s", n as f64 / elapsed.as_secs_f64() / 1e6)
}

#[test]
#[ignore = "harness: needs BENCH_MRT, run with --release --ignored --nocapture"]
fn bench_rib() {
    let Ok(path) = std::env::var("BENCH_MRT") else {
        eprintln!("bench_rib: set BENCH_MRT to an MRT RIB dump; skipping");
        return;
    };
    let routers: usize = env("BENCH_ROUTERS", 1).max(1);
    let limit: usize = env("BENCH_LIMIT", 0);
    let lookups: usize = env("BENCH_LOOKUPS", 10_000_000);

    let started = Instant::now();
    let routes = read_rib(&path, limit);
    let v4 = routes.iter().filter(|r| r.prefix.addr().is_ipv4()).count();
    println!(
        "parsed {} routes ({v4} v4, {} v6) in {:.1?}",
        routes.len(),
        routes.len() - v4,
        started.elapsed()
    );
    assert!(!routes.is_empty(), "no announcements in {path}");

    let registry = LookupTable::new();
    let config = cfg::LookupTable {
        id: Uuid::now_v7(),
        name: "routes".into(),
        source: cfg::Source::Bgp(cfg::Bgp {
            listener: Uuid::now_v7(),
            max_rows: routes.len() * routers,
        }),
        key: cfg::Bgp::key(),
    };
    let before = allocated();
    registry.insert(&config).unwrap();
    let table = registry.route_table(&config.id).unwrap();

    let mut text = RouteText::default();
    let started = Instant::now();
    for r in 0..routers {
        let router = router(r);
        for route in &routes {
            let mut path = Vec::new();
            let row = route.row(router, cfg::bgp::GLOBAL_VRF, &mut path);
            assert_eq!(table.upsert(&row, &mut text).unwrap(), Upsert::Inserted);
        }
    }
    let elapsed = started.elapsed();
    let stored = table.len();
    let bytes = allocated().saturating_sub(before);
    println!(
        "inserted {stored} routes ({routers} router(s)) in {elapsed:.1?}: {}; \
         {:.1} MiB, {} B/route",
        rate(stored, elapsed),
        bytes as f64 / (1 << 20) as f64,
        bytes / stored.max(1)
    );

    let read = registry.get(&config.id).unwrap().read();
    let origin = read.column(cfg::bgp::ORIGIN_AS).unwrap();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let probe = |addr: IpAddr, router: IpAddr| {
        let partition = [KeyRef::Ip(router), KeyRef::String(cfg::bgp::GLOBAL_VRF)];
        read.lookup(Lookup::Prefix {
            partition: &partition,
            addr,
        })
        .map(|m| m.row.cell(origin).is_null())
    };

    let known: Vec<IpAddr> = (0..lookups.min(1 << 20))
        .map(|_| {
            let salt = rng.next();
            inside(&routes[salt as usize % routes.len()].prefix, salt >> 8)
        })
        .collect();
    let started = Instant::now();
    let mut hits = 0usize;
    for i in 0..lookups {
        hits += probe(known[i % known.len()], router(i % routers)).is_some() as usize;
    }
    let elapsed = started.elapsed();
    println!(
        "lpm, known prefixes: {lookups} in {elapsed:.1?}: {}, {:.1}% hit",
        rate(lookups, elapsed),
        100.0 * hits as f64 / lookups as f64
    );

    let random: Vec<IpAddr> = (0..lookups.min(1 << 20))
        .map(|_| IpAddr::V4(Ipv4Addr::from(rng.next() as u32)))
        .collect();
    let started = Instant::now();
    let mut hits = 0usize;
    for i in 0..lookups {
        hits += probe(random[i % random.len()], router(i % routers)).is_some() as usize;
    }
    let elapsed = started.elapsed();
    println!(
        "lpm, random v4: {lookups} in {elapsed:.1?}: {}, {:.1}% hit",
        rate(lookups, elapsed),
        100.0 * hits as f64 / lookups as f64
    );
    drop(read);

    // A resync of router 0 re-sends every other route; the sweep takes the rest.
    std::thread::sleep(Duration::from_millis(2));
    let cutoff = now_millis();
    let first = router(0);
    for route in routes.iter().step_by(2) {
        let mut path = Vec::new();
        let row = route.row(first, cfg::bgp::GLOBAL_VRF, &mut path);
        let _ = table.upsert(&row, &mut text).unwrap();
    }
    let started = Instant::now();
    let swept = table.sweep(first, cfg::bgp::GLOBAL_VRF, None, cutoff);
    println!(
        "sweep: {swept} stale of {} in {:.1?}",
        routes.len(),
        started.elapsed()
    );
    assert_eq!(swept, routes.len() / 2);

    let started = Instant::now();
    let removed = table.peer_down(first, cfg::bgp::GLOBAL_VRF);
    println!("peer down: {removed} routes in {:.1?}", started.elapsed());
    assert_eq!(removed, routes.len() - swept);
}

/// The path the listener will take, small enough to run every time: routes go
/// in as [`RouteRow`]s and come back out of a lookup with a literal VRF.
#[test]
fn routes_round_trip_through_a_bgp_table() {
    let registry = LookupTable::new();
    let config: cfg::LookupTable = serde_yaml_ng::from_str(
        "id: 0192f8b0-0000-7000-8000-000000000010\n\
         name: routes\n\
         source: {type: bgp, listener: 0192f8b0-0000-7000-8000-0000000000aa}\n",
    )
    .unwrap();
    registry.insert(&config).unwrap();
    let table = registry.route_table(&config.id).unwrap();
    let router: IpAddr = "192.0.2.1".parse().unwrap();

    let routes = [
        ("10.0.0.0/8", vec![(0u8, vec![64500, 64501])]),
        (
            "10.1.0.0/16",
            vec![(0, vec![64500]), (1, vec![64510, 64511])],
        ),
        ("2001:db8::/32", vec![(0, vec![64502])]),
    ];
    let mut text = RouteText::default();
    for (prefix, path) in &routes {
        let route = Route {
            prefix: prefix.parse().unwrap(),
            next_hop: Some("192.0.2.254".parse().unwrap()),
            path: path.clone(),
            communities: vec![(64500 << 16) | 1],
            large: vec![],
            local_pref: Some(100),
            med: None,
        };
        let mut segments = Vec::new();
        let row = route.row(router, cfg::bgp::GLOBAL_VRF, &mut segments);
        assert_eq!(table.upsert(&row, &mut text).unwrap(), Upsert::Inserted);
    }

    let read = registry.get(&config.id).unwrap().read();
    let col = |name| read.column(name).unwrap();
    let at = |vrf: &str, addr: &str| {
        let partition = [KeyRef::Ip(router), KeyRef::String(vrf)];
        read.lookup(Lookup::Prefix {
            partition: &partition,
            addr: addr.parse().unwrap(),
        })
        .map(|m| {
            (
                m.prefix.unwrap().to_string(),
                m.row.cell(col(cfg::bgp::ORIGIN_AS)).to_owned_cell(),
                m.row.cell(col(cfg::bgp::AS_PATH)).to_owned_cell(),
            )
        })
    };
    assert_eq!(
        at("global", "10.2.3.4"),
        Some((
            "10.0.0.0/8".into(),
            CellOwned::Integer(64501),
            CellOwned::String("64500 64501".into())
        ))
    );
    assert_eq!(
        at("global", "10.1.3.4"),
        Some((
            "10.1.0.0/16".into(),
            CellOwned::Null,
            CellOwned::String("64500 {64510,64511}".into())
        )),
        "an aggregate has no one origin"
    );
    assert_eq!(at("global", "2001:db8::1").unwrap().0, "2001:db8::/32");
    assert_eq!(
        at("64500:1", "10.2.3.4"),
        None,
        "another VRF is another table"
    );
}
