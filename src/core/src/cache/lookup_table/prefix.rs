// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! IP prefixes (CIDR) and the longest-prefix-match set both table kinds index
//! them with.

use super::cell::parse_ip;
use ipnet::{IpNet, Ipv4Net, Ipv6Net};
use prefix_trie::PrefixMap;
use std::net::{IpAddr, Ipv4Addr};

/// Why a prefix did not parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixError {
    /// neither a CIDR nor a bare address
    Syntax,
    /// the address has bits set past the prefix length (`10.0.0.1/8`)
    HostBits,
}

impl std::fmt::Display for PrefixError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PrefixError::Syntax => "not an IP prefix",
            PrefixError::HostBits => "the prefix has host bits set",
        })
    }
}

impl std::error::Error for PrefixError {}

/// Parses a CIDR (`10.1.0.0/16`, `2001:db8::/32`) or a bare address, which is
/// a host route (/32 or /128).
///
/// A prefix with host bits set is rejected rather than truncated: it is
/// almost always a typo, and silently widening it would shadow narrower
/// routes. A v4-mapped v6 prefix (`::ffff:10.0.0.0/104`) folds onto v4, the
/// same way [`parse_ip`] folds a probe address, so build and probe agree.
pub fn parse_prefix(raw: &str) -> Result<IpNet, PrefixError> {
    let raw = raw.trim();
    let net = if raw.contains('/') {
        raw.parse::<IpNet>().map_err(|_| PrefixError::Syntax)?
    } else {
        // `parse_ip` already folds a v4-mapped address onto v4
        IpNet::from(parse_ip(raw).map_err(|_| PrefixError::Syntax)?)
    };
    if net.trunc() != net {
        return Err(PrefixError::HostBits);
    }
    Ok(canonical(net))
}

/// Folds a v4-mapped v6 prefix (`::ffff:0:0/96` or longer) onto v4.
pub fn canonical(net: IpNet) -> IpNet {
    if let IpNet::V6(v6) = net
        && v6.prefix_len() >= 96
        && let Some(v4) = v6.network().to_ipv4_mapped()
    {
        // `to_ipv4_mapped` checks the `::ffff:` top 96 bits, which the
        // prefix length guarantees are all network bits
        return IpNet::V4(Ipv4Net::new_assert(v4, v6.prefix_len() - 96));
    }
    net
}

/// The host route probing `addr`, built on the stack.
#[inline]
fn host_v4(addr: Ipv4Addr) -> Ipv4Net {
    Ipv4Net::new_assert(addr, 32)
}

/// A longest-prefix-match map, one trie per address family so `0.0.0.0/0` and
/// `::/0` stay separate defaults.
#[derive(Debug, Clone)]
pub struct PrefixSet<V> {
    v4: PrefixMap<Ipv4Net, V>,
    v6: PrefixMap<Ipv6Net, V>,
}

impl<V> Default for PrefixSet<V> {
    fn default() -> Self {
        Self {
            v4: PrefixMap::new(),
            v6: PrefixMap::new(),
        }
    }
}

impl<V> PrefixSet<V> {
    pub fn new() -> Self {
        Self::default()
    }

    /// The narrowest stored prefix containing `addr`, and its value. A
    /// v4-mapped v6 address is looked up as v4.
    #[inline]
    pub fn lpm(&self, addr: IpAddr) -> Option<(IpNet, &V)> {
        match addr.to_canonical() {
            IpAddr::V4(a) => self.v4.get_lpm(&host_v4(a)).map(|(p, v)| (IpNet::V4(p), v)),
            IpAddr::V6(a) => self
                .v6
                .get_lpm(&Ipv6Net::new_assert(a, 128))
                .map(|(p, v)| (IpNet::V6(p), v)),
        }
    }

    /// The value stored under exactly `net`.
    pub fn get(&self, net: &IpNet) -> Option<&V> {
        match net {
            IpNet::V4(n) => self.v4.get(n),
            IpNet::V6(n) => self.v6.get(n),
        }
    }

    pub fn contains(&self, net: &IpNet) -> bool {
        self.get(net).is_some()
    }

    /// Stores `value` under exactly `net`, returning what it replaced.
    pub fn insert(&mut self, net: IpNet, value: V) -> Option<V> {
        match net {
            IpNet::V4(n) => self.v4.insert(n, value),
            IpNet::V6(n) => self.v6.insert(n, value),
        }
    }

    /// Removes exactly `net`; a wider or narrower prefix is untouched.
    pub fn remove(&mut self, net: &IpNet) -> Option<V> {
        match net {
            IpNet::V4(n) => self.v4.remove(n),
            IpNet::V6(n) => self.v6.remove(n),
        }
    }

    /// Keeps only the entries `keep` answers `true` for.
    pub fn retain(&mut self, mut keep: impl FnMut(IpNet, &V) -> bool) {
        self.v4.retain(|p, v| keep(IpNet::V4(*p), v));
        self.v6.retain(|p, v| keep(IpNet::V6(*p), v));
    }

    pub fn iter(&self) -> impl Iterator<Item = (IpNet, &V)> {
        self.v4
            .iter()
            .map(|(p, v)| (IpNet::V4(p), v))
            .chain(self.v6.iter().map(|(p, v)| (IpNet::V6(p), v)))
    }

    pub fn len(&self) -> usize {
        self.v4.len() + self.v6.len()
    }

    pub fn is_empty(&self) -> bool {
        self.v4.is_empty() && self.v6.is_empty()
    }

    /// Bytes held by the tries, not counting heap owned by the values.
    pub fn mem_size(&self) -> usize {
        self.v4.mem_size() + self.v6.mem_size()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::{RngExt, SeedableRng};
    use std::collections::BTreeMap;
    use std::net::Ipv6Addr;

    fn net(s: &str) -> IpNet {
        parse_prefix(s).unwrap()
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn parses_cidrs_and_bare_addresses() {
        assert_eq!(
            parse_prefix("10.1.0.0/16"),
            Ok("10.1.0.0/16".parse().unwrap())
        );
        assert_eq!(
            parse_prefix(" 2001:db8::/32 "),
            Ok("2001:db8::/32".parse().unwrap())
        );
        assert_eq!(parse_prefix("10.0.0.1"), Ok("10.0.0.1/32".parse().unwrap()));
        assert_eq!(
            parse_prefix("2001:db8::1"),
            Ok("2001:db8::1/128".parse().unwrap())
        );
        assert_eq!(parse_prefix("0.0.0.0/0"), Ok("0.0.0.0/0".parse().unwrap()));
        assert_eq!(parse_prefix("::/0"), Ok("::/0".parse().unwrap()));
    }

    #[test]
    fn rejects_syntax_errors() {
        for bad in [
            "",
            "nope",
            "10.0.0.0/33",
            "10.0.0/8",
            "2001:db8::/129",
            "/8",
            "10.0.0.0/",
        ] {
            assert_eq!(parse_prefix(bad), Err(PrefixError::Syntax), "{bad:?}");
        }
    }

    #[test]
    fn rejects_host_bits() {
        assert_eq!(parse_prefix("10.0.0.1/8"), Err(PrefixError::HostBits));
        assert_eq!(parse_prefix("10.1.2.0/16"), Err(PrefixError::HostBits));
        assert_eq!(parse_prefix("2001:db8::1/32"), Err(PrefixError::HostBits));
    }

    #[test]
    fn folds_v4_mapped_prefixes() {
        assert_eq!(net("::ffff:10.0.0.0/104"), net("10.0.0.0/8"));
        assert_eq!(net("::ffff:0:0/96"), net("0.0.0.0/0"));
        assert_eq!(net("::ffff:10.1.2.3"), net("10.1.2.3/32"));
        // shorter than /96 covers more than the mapped range: left as v6
        assert_eq!(net("::/80"), "::/80".parse().unwrap());
        // the mapped range's /96 neighbours are not mapped
        assert!(matches!(net("::fffe:0:0/96"), IpNet::V6(_)));
    }

    #[test]
    fn longest_prefix_wins() {
        let mut set = PrefixSet::new();
        set.insert(net("10.0.0.0/8"), 8);
        set.insert(net("10.1.0.0/16"), 16);
        set.insert(net("10.1.2.0/24"), 24);
        assert_eq!(set.lpm(ip("10.1.2.3")), Some((net("10.1.2.0/24"), &24)));
        assert_eq!(set.lpm(ip("10.1.3.3")), Some((net("10.1.0.0/16"), &16)));
        assert_eq!(set.lpm(ip("10.2.0.0")), Some((net("10.0.0.0/8"), &8)));
        assert_eq!(set.lpm(ip("11.0.0.0")), None);
        // a mapped probe reaches the v4 trie
        assert_eq!(
            set.lpm(ip("::ffff:10.1.2.3")),
            Some((net("10.1.2.0/24"), &24))
        );
    }

    #[test]
    fn removing_a_narrow_prefix_exposes_the_wider_one() {
        let mut set = PrefixSet::new();
        set.insert(net("10.0.0.0/8"), 8);
        set.insert(net("10.1.0.0/16"), 16);
        assert_eq!(set.remove(&net("10.1.0.0/16")), Some(16));
        assert_eq!(set.lpm(ip("10.1.0.1")), Some((net("10.0.0.0/8"), &8)));
        // exact only: neither a wider nor a narrower prefix is removed
        assert_eq!(set.remove(&net("10.0.0.0/7")), None);
        assert_eq!(set.remove(&net("10.0.0.0/9")), None);
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn default_routes_stay_per_family() {
        let mut set = PrefixSet::new();
        set.insert(net("0.0.0.0/0"), 4);
        assert_eq!(set.lpm(ip("2001:db8::1")), None);
        set.insert(net("::/0"), 6);
        assert_eq!(set.lpm(ip("192.0.2.1")), Some((net("0.0.0.0/0"), &4)));
        assert_eq!(set.lpm(ip("2001:db8::1")), Some((net("::/0"), &6)));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn host_routes_match_only_their_address() {
        let mut set = PrefixSet::new();
        set.insert(net("192.0.2.7"), 1);
        set.insert(net("2001:db8::7"), 2);
        assert_eq!(set.lpm(ip("192.0.2.7")), Some((net("192.0.2.7/32"), &1)));
        assert_eq!(set.lpm(ip("192.0.2.8")), None);
        assert_eq!(
            set.lpm(ip("2001:db8::7")),
            Some((net("2001:db8::7/128"), &2))
        );
        assert_eq!(set.lpm(ip("2001:db8::6")), None);
    }

    #[test]
    fn retain_drops_entries_and_keeps_the_count() {
        let mut set = PrefixSet::new();
        set.insert(net("10.0.0.0/8"), 1);
        set.insert(net("10.1.0.0/16"), 2);
        set.insert(net("2001:db8::/32"), 3);
        set.retain(|_, v| *v != 2);
        assert_eq!(set.len(), 2);
        assert_eq!(set.lpm(ip("10.1.0.1")), Some((net("10.0.0.0/8"), &1)));
        assert!(set.mem_size() > 0);
    }

    /// A reference model: every stored prefix, the longest match found by
    /// scanning them all.
    #[derive(Default)]
    struct Oracle(BTreeMap<IpNet, u32>);

    impl Oracle {
        fn lpm(&self, addr: IpAddr) -> Option<(IpNet, u32)> {
            self.0
                .iter()
                .filter(|(n, _)| n.contains(&addr))
                .max_by_key(|(n, _)| n.prefix_len())
                .map(|(n, v)| (*n, *v))
        }
    }

    fn random_net(rng: &mut StdRng) -> IpNet {
        // a narrow address space so prefixes nest and collide often
        if rng.random_bool(0.6) {
            let addr = Ipv4Addr::from(0x0a00_0000 | (rng.random::<u32>() & 0x00ff_ffff));
            let len = rng.random_range(0..=32u8);
            IpNet::V4(Ipv4Net::new_assert(addr, len).trunc())
        } else {
            let bits = 0x2001_0db8u128 << 96 | (rng.random::<u128>() & ((1 << 96) - 1));
            let len = rng.random_range(0..=128u8);
            IpNet::V6(Ipv6Net::new_assert(Ipv6Addr::from(bits), len).trunc())
        }
    }

    fn random_addr(rng: &mut StdRng) -> IpAddr {
        match random_net(rng) {
            IpNet::V4(n) => IpAddr::V4(Ipv4Addr::from(
                n.network().to_bits() | (rng.random::<u32>() & n.hostmask().to_bits()),
            )),
            IpNet::V6(n) => IpAddr::V6(Ipv6Addr::from(
                n.network().to_bits() | (rng.random::<u128>() & n.hostmask().to_bits()),
            )),
        }
    }

    /// The first and last address of `n` and each just outside it.
    fn boundaries(n: IpNet) -> Vec<IpAddr> {
        match n {
            IpNet::V4(n) => {
                let (lo, hi) = (n.network().to_bits(), n.broadcast().to_bits());
                [Some(lo), Some(hi), lo.checked_sub(1), hi.checked_add(1)]
                    .into_iter()
                    .flatten()
                    .map(|v| IpAddr::V4(Ipv4Addr::from(v)))
                    .collect()
            }
            IpNet::V6(n) => {
                let (lo, hi) = (n.network().to_bits(), n.broadcast().to_bits());
                [Some(lo), Some(hi), lo.checked_sub(1), hi.checked_add(1)]
                    .into_iter()
                    .flatten()
                    .map(|v| IpAddr::V6(Ipv6Addr::from(v)))
                    .collect()
            }
        }
    }

    #[test]
    fn matches_a_brute_force_oracle() {
        for seed in 0..20u64 {
            let mut rng = StdRng::seed_from_u64(seed);
            let mut set = PrefixSet::new();
            let mut oracle = Oracle::default();
            for step in 0..400u32 {
                let n = random_net(&mut rng);
                if rng.random_bool(0.3) {
                    assert_eq!(
                        set.remove(&n),
                        oracle.0.remove(&n),
                        "seed {seed} remove {n}"
                    );
                } else {
                    assert_eq!(
                        set.insert(n, step),
                        oracle.0.insert(n, step),
                        "seed {seed} insert {n}"
                    );
                }
                assert_eq!(set.len(), oracle.0.len(), "seed {seed}");
                if step % 50 == 0 {
                    check(&set, &oracle, &mut rng, seed);
                }
            }
            check(&set, &oracle, &mut rng, seed);
        }
    }

    fn check(set: &PrefixSet<u32>, oracle: &Oracle, rng: &mut StdRng, seed: u64) {
        let mut probes: Vec<IpAddr> = (0..200).map(|_| random_addr(rng)).collect();
        for n in oracle.0.keys() {
            probes.extend(boundaries(*n));
        }
        for addr in probes {
            assert_eq!(
                set.lpm(addr).map(|(n, v)| (n, *v)),
                oracle.lpm(addr),
                "seed {seed} probe {addr}"
            );
        }
        for (n, v) in &oracle.0 {
            assert_eq!(set.get(n), Some(v));
        }
    }
}
