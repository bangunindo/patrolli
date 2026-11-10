// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

// Longest-prefix match over a BGP-sized table: about 1M v4 and 200k v6
// prefixes with a full-table-like length mix (mostly /24 and /48, the rest
// spread over /8–/23 and /29–/47, so most routes nest under wider ones).
//
// Measures:
//   - `build`: inserting every prefix into a fresh `PrefixSet` (what an
//     immutable table load pays for its index)
//   - `lpm/{v4,v6}_{hit,miss}`: one longest-prefix match
//   - `mutable/upsert_remove`: insert then remove one prefix in a full set,
//     the per-update cost a BGP feed would pay
// and prints the trie's `mem_size` once, per 1M prefixes.
//
// Usage:
//   cargo bench --bench bench_lookup_prefix
//   taskset -c 2 cargo bench --bench bench_lookup_prefix -- lpm

use std::hint::black_box;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use ipnet::{IpNet, Ipv4Net, Ipv6Net};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use patrolli::cache::lookup_table::prefix::PrefixSet;

const V4_PREFIXES: usize = 1_000_000;
const V6_PREFIXES: usize = 200_000;
const PROBES: usize = 4096;

fn v4_len(rng: &mut StdRng) -> u8 {
    match rng.random_range(0..100u32) {
        0..60 => 24,
        60..70 => 23,
        70..80 => 22,
        80..95 => rng.random_range(16..=21),
        _ => rng.random_range(8..=15),
    }
}

fn v6_len(rng: &mut StdRng) -> u8 {
    match rng.random_range(0..100u32) {
        0..50 => 48,
        50..65 => 32,
        65..80 => rng.random_range(40..=47),
        80..90 => rng.random_range(29..=39),
        _ => rng.random_range(49..=64),
    }
}

fn v4_net(rng: &mut StdRng) -> IpNet {
    // public unicast-ish space, 1.0.0.0 – 223.255.255.255
    let addr = rng.random_range(0x0100_0000..0xe000_0000u32);
    IpNet::V4(Ipv4Net::new_assert(Ipv4Addr::from(addr), v4_len(rng)).trunc())
}

fn v6_net(rng: &mut StdRng) -> IpNet {
    // 2000::/4, where the global unicast table lives
    let addr = (0x2u128 << 124) | (rng.random::<u128>() >> 4);
    IpNet::V6(Ipv6Net::new_assert(Ipv6Addr::from(addr), v6_len(rng)).trunc())
}

struct Table {
    nets: Vec<IpNet>,
    set: PrefixSet<u32>,
}

fn table() -> Table {
    let mut rng = StdRng::seed_from_u64(42);
    let mut nets: Vec<IpNet> = (0..V4_PREFIXES).map(|_| v4_net(&mut rng)).collect();
    nets.extend((0..V6_PREFIXES).map(|_| v6_net(&mut rng)));
    let mut set = PrefixSet::new();
    for (i, n) in nets.iter().enumerate() {
        set.insert(*n, i as u32);
    }
    Table { nets, set }
}

/// Addresses inside random stored prefixes, so every probe matches something.
fn hits(t: &Table, v4: bool, rng: &mut StdRng) -> Vec<IpAddr> {
    let nets: Vec<IpNet> = t
        .nets
        .iter()
        .filter(|n| matches!(n, IpNet::V4(_)) == v4)
        .copied()
        .collect();
    (0..PROBES)
        .map(|_| match nets[rng.random_range(0..nets.len())] {
            IpNet::V4(n) => IpAddr::V4(Ipv4Addr::from(
                n.network().to_bits() | (rng.random::<u32>() & n.hostmask().to_bits()),
            )),
            IpNet::V6(n) => IpAddr::V6(Ipv6Addr::from(
                n.network().to_bits() | (rng.random::<u128>() & n.hostmask().to_bits()),
            )),
        })
        .collect()
}

/// Addresses no prefix covers: v4 from 240.0.0.0/4, v6 from fc00::/7.
fn misses(v4: bool, rng: &mut StdRng) -> Vec<IpAddr> {
    (0..PROBES)
        .map(|_| {
            if v4 {
                IpAddr::V4(Ipv4Addr::from(0xf000_0000 | (rng.random::<u32>() >> 4)))
            } else {
                IpAddr::V6(Ipv6Addr::from(
                    (0xfcu128 << 120) | (rng.random::<u128>() >> 8),
                ))
            }
        })
        .collect()
}

fn bench(c: &mut Criterion) {
    let t = table();
    let per_million = t.set.mem_size() as f64 / (t.set.len() as f64 / 1e6);
    eprintln!(
        "prefixes: {} stored ({} generated), mem_size: {:.1} MiB, {:.1} MiB per 1M prefixes",
        t.set.len(),
        t.nets.len(),
        t.set.mem_size() as f64 / (1 << 20) as f64,
        per_million / (1 << 20) as f64,
    );

    let mut g = c.benchmark_group("build");
    g.sample_size(10);
    g.throughput(Throughput::Elements(t.nets.len() as u64));
    g.bench_function("full_table", |b| {
        b.iter(|| {
            let mut set = PrefixSet::new();
            for (i, n) in t.nets.iter().enumerate() {
                set.insert(*n, i as u32);
            }
            black_box(set.len())
        })
    });
    g.finish();

    let mut rng = StdRng::seed_from_u64(7);
    let cases = [
        ("v4_hit", hits(&t, true, &mut rng)),
        ("v6_hit", hits(&t, false, &mut rng)),
        ("v4_miss", misses(true, &mut rng)),
        ("v6_miss", misses(false, &mut rng)),
    ];
    let mut g = c.benchmark_group("lpm");
    g.throughput(Throughput::Elements(PROBES as u64));
    for (name, probes) in &cases {
        g.bench_function(*name, |b| {
            b.iter(|| {
                let mut found = 0u32;
                for addr in probes {
                    if t.set.lpm(black_box(*addr)).is_some() {
                        found += 1;
                    }
                }
                black_box(found)
            })
        });
    }
    g.finish();

    // a mutable table stores an `Arc` per prefix
    let mut set: PrefixSet<Arc<u32>> = PrefixSet::new();
    for (i, n) in t.nets.iter().enumerate() {
        set.insert(*n, Arc::new(i as u32));
    }
    let fresh: Vec<IpNet> = {
        let mut rng = StdRng::seed_from_u64(99);
        (0..PROBES)
            .map(|_| v4_net(&mut rng))
            .filter(|n| !set.contains(n))
            .collect()
    };
    let mut g = c.benchmark_group("mutable");
    g.throughput(Throughput::Elements(fresh.len() as u64));
    g.bench_function("upsert_remove", |b| {
        b.iter(|| {
            for n in &fresh {
                set.insert(*n, Arc::new(0));
            }
            for n in &fresh {
                black_box(set.remove(n));
            }
        })
    });
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
