// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The UPDATE message (RFC 4271 §4.3) with the attributes a route row keeps,
//! read the way RFC 7606 asks for.
//!
//! [`Update::parse`] fails only when the message's structure is broken, so
//! its prefixes cannot be known: RFC 7606's "session reset". A malformed
//! attribute whose bounds are intact sets [`Attrs::withdraw`] instead, and the
//! caller withdraws whatever the message announces ("treat-as-withdraw"); one
//! nothing reads is dropped ("attribute discard").

use super::{AFI_IPV4, Afi, Reader, Result, WireError};
use ipnet::{IpNet, Ipv4Net, Ipv6Net};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::ops::Range;

const ATTR_ORIGIN: u8 = 1;
const ATTR_AS_PATH: u8 = 2;
const ATTR_NEXT_HOP: u8 = 3;
const ATTR_MED: u8 = 4;
const ATTR_LOCAL_PREF: u8 = 5;
const ATTR_COMMUNITIES: u8 = 8;
const ATTR_MP_REACH: u8 = 14;
const ATTR_MP_UNREACH: u8 = 15;
const ATTR_AS4_PATH: u8 = 17;
const ATTR_LARGE_COMMUNITIES: u8 = 32;
const FLAG_EXTENDED_LENGTH: u8 = 0x10;

/// An UPDATE's parts, borrowed from the message.
#[derive(Debug, Clone, Copy)]
pub struct Update<'a> {
    /// IPv4 unicast withdrawals, raw (see [`prefixes`])
    pub withdrawn: &'a [u8],
    /// IPv4 unicast announcements, raw
    pub nlri: &'a [u8],
    pub attrs: Attrs<'a>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Attrs<'a> {
    /// raw segments, checked (see [`AsPath::decode`])
    pub as_path: Option<&'a [u8]>,
    /// raw segments, checked as 4-octet; dropped when malformed
    pub as4_path: Option<&'a [u8]>,
    pub next_hop: Option<Ipv4Addr>,
    pub med: Option<u32>,
    pub local_pref: Option<u32>,
    /// a non-zero multiple of 4 bytes
    pub communities: Option<&'a [u8]>,
    /// a non-zero multiple of 12 bytes
    pub large_communities: Option<&'a [u8]>,
    pub mp_reach: Option<MpReach<'a>>,
    pub mp_unreach: Option<MpUnreach<'a>>,
    /// Set when an attribute the route depends on is malformed: everything
    /// the message announces must be withdrawn instead (RFC 7606 §2).
    pub withdraw: Option<&'static str>,
    /// how many attributes the message carried
    pub count: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct MpReach<'a> {
    pub afi: u16,
    pub safi: u8,
    /// set for the unicast families only; other families are not read
    pub next_hop: Option<IpAddr>,
    pub nlri: &'a [u8],
}

#[derive(Debug, Clone, Copy)]
pub struct MpUnreach<'a> {
    pub afi: u16,
    pub safi: u8,
    pub nlri: &'a [u8],
}

impl<'a> Update<'a> {
    /// Reads an UPDATE body (after the BGP header). `asn4`: AS_PATH carries
    /// 4-octet AS numbers. `add_path`: per unicast family, whether prefixes
    /// carry a path id; the NLRI are checked here, so iterating them later
    /// cannot fail.
    pub fn parse(body: &'a [u8], asn4: bool, add_path: [bool; 2]) -> Result<Self> {
        let mut r = Reader::new(body);
        let withdrawn_len = r.u16("update truncated")? as usize;
        let withdrawn = r.take(withdrawn_len, "withdrawn routes overrun the update")?;
        let attrs_len = r.u16("update truncated")? as usize;
        let attrs = r.take(attrs_len, "path attributes overrun the update")?;
        let nlri = r.rest();

        let v4_add_path = add_path[Afi::V4.index()];
        check_prefixes(withdrawn, Afi::V4, v4_add_path)?;
        check_prefixes(nlri, Afi::V4, v4_add_path)?;
        let attrs = Attrs::parse(attrs, asn4, add_path)?;
        Ok(Self {
            withdrawn,
            nlri,
            attrs,
        })
    }

    /// The AFI/SAFI this message marks the End-of-RIB of (RFC 4724 §2): an
    /// empty UPDATE for IPv4 unicast, otherwise an MP_UNREACH_NLRI with no
    /// prefixes and nothing announced or withdrawn beside it.
    pub fn end_of_rib(&self) -> Option<(u16, u8)> {
        if !self.withdrawn.is_empty() || !self.nlri.is_empty() || self.attrs.mp_reach.is_some() {
            return None;
        }
        match self.attrs.mp_unreach {
            None if self.attrs.count == 0 => Some((AFI_IPV4, super::SAFI_UNICAST)),
            Some(u) if u.nlri.is_empty() => Some((u.afi, u.safi)),
            _ => None,
        }
    }
}

impl<'a> Attrs<'a> {
    fn parse(bytes: &'a [u8], asn4: bool, add_path: [bool; 2]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let mut attrs = Attrs::default();
        let mut seen = [false; 256];
        while !r.is_empty() {
            let flags = r.u8("attribute header truncated")?;
            let kind = r.u8("attribute header truncated")?;
            let len = if flags & FLAG_EXTENDED_LENGTH != 0 {
                r.u16("attribute header truncated")? as usize
            } else {
                r.u8("attribute header truncated")? as usize
            };
            // An attribute running past the field leaves no way to find the
            // attributes after it, MP_REACH_NLRI among them.
            let value = r.take(len, "attribute overruns the attributes")?;
            attrs.count += 1;
            let first = !std::mem::replace(&mut seen[kind as usize], true);
            match kind {
                ATTR_MP_REACH | ATTR_MP_UNREACH if !first => {
                    return Err(WireError("mp_reach or mp_unreach repeated"));
                }
                // RFC 7606 §3 (g): later copies are discarded
                _ if !first => {}
                ATTR_ORIGIN => {
                    if !matches!(value, [0..=2]) {
                        attrs.malformed("malformed origin");
                    }
                }
                ATTR_AS_PATH => match AsPath::check(value, asn4) {
                    Ok(()) => attrs.as_path = Some(value),
                    Err(_) => attrs.malformed("malformed as_path"),
                },
                ATTR_AS4_PATH => {
                    // RFC 6793 §6: a malformed AS4_PATH is discarded
                    if AsPath::check(value, true).is_ok() {
                        attrs.as4_path = Some(value);
                    }
                }
                ATTR_NEXT_HOP => match <[u8; 4]>::try_from(value) {
                    Ok(ip) => attrs.next_hop = Some(Ipv4Addr::from(ip)),
                    Err(_) => attrs.malformed("malformed next_hop"),
                },
                ATTR_MED => match <[u8; 4]>::try_from(value) {
                    Ok(v) => attrs.med = Some(u32::from_be_bytes(v)),
                    Err(_) => attrs.malformed("malformed med"),
                },
                ATTR_LOCAL_PREF => match <[u8; 4]>::try_from(value) {
                    Ok(v) => attrs.local_pref = Some(u32::from_be_bytes(v)),
                    Err(_) => attrs.malformed("malformed local_pref"),
                },
                ATTR_COMMUNITIES => {
                    if !value.is_empty() && value.len() % 4 == 0 {
                        attrs.communities = Some(value);
                    } else {
                        attrs.malformed("malformed communities");
                    }
                }
                ATTR_LARGE_COMMUNITIES => {
                    if !value.is_empty() && value.len() % 12 == 0 {
                        attrs.large_communities = Some(value);
                    } else {
                        attrs.malformed("malformed large_communities");
                    }
                }
                ATTR_MP_REACH => attrs.mp_reach = Some(MpReach::parse(value, add_path)?),
                ATTR_MP_UNREACH => attrs.mp_unreach = Some(MpUnreach::parse(value, add_path)?),
                // ATOMIC_AGGREGATE, AGGREGATOR, extended communities and
                // everything unknown: nothing the row keeps
                _ => {}
            }
        }
        Ok(attrs)
    }

    fn malformed(&mut self, why: &'static str) {
        self.withdraw.get_or_insert(why);
    }

    /// The communities, each as its 32-bit value.
    pub fn communities(&self) -> impl Iterator<Item = u32> + 'a {
        let (chunks, _) = self.communities.unwrap_or_default().as_chunks::<4>();
        chunks.iter().map(|c| u32::from_be_bytes(*c))
    }

    /// The large communities: global administrator, local data 1 and 2.
    pub fn large_communities(&self) -> impl Iterator<Item = [u32; 3]> + 'a {
        let (chunks, _) = self.large_communities.unwrap_or_default().as_chunks::<12>();
        chunks.iter().map(|c| {
            let word = |i: usize| u32::from_be_bytes([c[i], c[i + 1], c[i + 2], c[i + 3]]);
            [word(0), word(4), word(8)]
        })
    }
}

impl<'a> MpReach<'a> {
    fn parse(value: &'a [u8], add_path: [bool; 2]) -> Result<Self> {
        let mut r = Reader::new(value);
        let afi = r.u16("mp_reach truncated")?;
        let safi = r.u8("mp_reach truncated")?;
        let nh_len = r.u8("mp_reach truncated")? as usize;
        let nh = r.take(nh_len, "mp_reach next hop truncated")?;
        let _reserved = r.u8("mp_reach truncated")?;
        let nlri = r.rest();
        let next_hop = match Afi::unicast(afi, safi) {
            Some(family) => {
                check_prefixes(nlri, family, add_path[family.index()])?;
                Some(next_hop(nh)?)
            }
            None => None,
        };
        Ok(Self {
            afi,
            safi,
            next_hop,
            nlri,
        })
    }
}

impl<'a> MpUnreach<'a> {
    fn parse(value: &'a [u8], add_path: [bool; 2]) -> Result<Self> {
        let mut r = Reader::new(value);
        let afi = r.u16("mp_unreach truncated")?;
        let safi = r.u8("mp_unreach truncated")?;
        let nlri = r.rest();
        if let Some(family) = Afi::unicast(afi, safi) {
            check_prefixes(nlri, family, add_path[family.index()])?;
        }
        Ok(Self { afi, safi, nlri })
    }
}

/// An MP_REACH_NLRI next hop: 4 bytes (IPv4), 16 (IPv6 global, or IPv6 for
/// IPv4 routes per RFC 8950) or 32 (global plus link-local; the link-local
/// one only when the global one is unspecified).
fn next_hop(nh: &[u8]) -> Result<IpAddr> {
    match nh.len() {
        4 => Ok(IpAddr::V4(Ipv4Addr::from(
            <[u8; 4]>::try_from(nh).unwrap_or_default(),
        ))),
        16 | 32 => {
            let v6 = |b: &[u8]| Ipv6Addr::from(<[u8; 16]>::try_from(b).unwrap_or_default());
            let global = v6(&nh[..16]);
            if global.is_unspecified() && nh.len() == 32 {
                Ok(IpAddr::V6(v6(&nh[16..])))
            } else {
                Ok(IpAddr::V6(global))
            }
        }
        _ => Err(WireError("unsupported next hop length")),
    }
}

/// Checks that `bytes` holds whole prefixes of `family` and returns how many.
pub fn check_prefixes(bytes: &[u8], family: Afi, add_path: bool) -> Result<usize> {
    let mut n = 0;
    for p in prefixes(bytes, family, add_path) {
        p?;
        n += 1;
    }
    Ok(n)
}

/// The prefixes in a raw NLRI field: an optional path id (RFC 7911), then a
/// length in bits and as many bytes as it takes. Bits past the length are
/// cleared.
pub fn prefixes(bytes: &[u8], family: Afi, add_path: bool) -> Prefixes<'_> {
    Prefixes {
        r: Reader::new(bytes),
        family,
        add_path,
    }
}

#[derive(Debug, Clone)]
pub struct Prefixes<'a> {
    r: Reader<'a>,
    family: Afi,
    add_path: bool,
}

impl Iterator for Prefixes<'_> {
    type Item = Result<(Option<u32>, IpNet)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.r.is_empty() {
            return None;
        }
        let item = self.read();
        if item.is_err() {
            // stop for good: what follows a bad prefix cannot be framed
            self.r = Reader::new(&[]);
        }
        Some(item)
    }
}

impl Prefixes<'_> {
    fn read(&mut self) -> Result<(Option<u32>, IpNet)> {
        let path_id = if self.add_path {
            Some(self.r.u32("prefix truncated")?)
        } else {
            None
        };
        let bits = self.r.u8("prefix truncated")?;
        let max = match self.family {
            Afi::V4 => 32,
            Afi::V6 => 128,
        };
        if bits > max {
            return Err(WireError("prefix length out of range"));
        }
        let bytes = self.r.take(bits.div_ceil(8) as usize, "prefix truncated")?;
        let net = match self.family {
            Afi::V4 => {
                let mut a = [0; 4];
                a[..bytes.len()].copy_from_slice(bytes);
                IpNet::V4(Ipv4Net::new_assert(Ipv4Addr::from(a), bits).trunc())
            }
            Afi::V6 => {
                let mut a = [0; 16];
                a[..bytes.len()].copy_from_slice(bytes);
                IpNet::V6(Ipv6Net::new_assert(Ipv6Addr::from(a), bits).trunc())
            }
        };
        Ok((path_id, net))
    }
}

/// AS path segment types (RFC 4271 §4.3, RFC 5065 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    Set,
    Sequence,
    ConfedSequence,
    ConfedSet,
}

impl SegmentKind {
    fn from_wire(kind: u8) -> Option<Self> {
        match kind {
            1 => Some(Self::Set),
            2 => Some(Self::Sequence),
            3 => Some(Self::ConfedSequence),
            4 => Some(Self::ConfedSet),
            _ => None,
        }
    }

    fn is_confed(self) -> bool {
        matches!(self, Self::ConfedSequence | Self::ConfedSet)
    }
}

/// A decoded AS path: the AS numbers of every segment in one buffer. Meant to
/// be reused across messages, so decoding allocates nothing once it has grown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AsPath {
    asns: Vec<u32>,
    segments: Vec<(SegmentKind, Range<usize>)>,
}

impl AsPath {
    /// Checks the segments of an AS_PATH or AS4_PATH value (RFC 7606 §7.2):
    /// known types, no empty segment, nothing left over.
    pub fn check(bytes: &[u8], asn4: bool) -> Result<()> {
        let width = if asn4 { 4 } else { 2 };
        let mut r = Reader::new(bytes);
        while !r.is_empty() {
            SegmentKind::from_wire(r.u8("as path segment truncated")?)
                .ok_or(WireError("unknown as path segment type"))?;
            let count = r.u8("as path segment truncated")? as usize;
            if count == 0 {
                return Err(WireError("empty as path segment"));
            }
            r.take(count * width, "as path segment truncated")?;
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.asns.clear();
        self.segments.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Replaces the path with a value [`Self::check`] accepted.
    pub fn decode(&mut self, bytes: &[u8], asn4: bool) -> Result<()> {
        self.clear();
        let mut r = Reader::new(bytes);
        while !r.is_empty() {
            let kind = SegmentKind::from_wire(r.u8("as path segment truncated")?)
                .ok_or(WireError("unknown as path segment type"))?;
            let count = r.u8("as path segment truncated")? as usize;
            let start = self.asns.len();
            for _ in 0..count {
                let asn = if asn4 {
                    r.u32("as path segment truncated")?
                } else {
                    r.u16("as path segment truncated")?.into()
                };
                self.asns.push(asn);
            }
            self.segments.push((kind, start..self.asns.len()));
        }
        Ok(())
    }

    pub fn segments(&self) -> impl Iterator<Item = (SegmentKind, &[u32])> {
        self.segments
            .iter()
            .map(|(kind, range)| (*kind, self.asns.get(range.clone()).unwrap_or_default()))
    }

    /// The path length RFC 6793 §4.2.3 compares: each AS of a sequence, one
    /// per set, none for confederation segments.
    fn counted_len(&self) -> usize {
        self.segments()
            .map(|(kind, asns)| match kind {
                SegmentKind::Sequence => asns.len(),
                SegmentKind::Set => 1,
                _ => 0,
            })
            .sum()
    }

    /// Rebuilds a 2-octet path with the 4-octet AS numbers of `as4`
    /// (RFC 6793 §4.2.3): the leading ASes of `self` that `as4` does not
    /// cover, then `as4` without confederation segments. `as4` is ignored when
    /// it is the longer of the two.
    pub fn merge_as4(&mut self, as4: &AsPath) {
        let (n, m) = (self.counted_len(), as4.counted_len());
        if m > n {
            return;
        }
        let mut keep = n - m;
        let mut merged = AsPath::default();
        for (kind, asns) in self.segments() {
            let take = match kind {
                _ if kind.is_confed() => asns.len(),
                SegmentKind::Sequence => asns.len().min(keep),
                _ if keep > 0 => asns.len(),
                _ => 0,
            };
            if take == 0 {
                continue;
            }
            match kind {
                SegmentKind::Sequence => keep -= take,
                SegmentKind::Set => keep -= 1,
                _ => {}
            }
            merged.push(kind, &asns[..take]);
        }
        for (kind, asns) in as4.segments() {
            if !kind.is_confed() {
                merged.push(kind, asns);
            }
        }
        *self = merged;
    }

    fn push(&mut self, kind: SegmentKind, asns: &[u32]) {
        let start = self.asns.len();
        self.asns.extend_from_slice(asns);
        self.segments.push((kind, start..self.asns.len()));
    }
}

/// Test helpers that build UPDATE bodies byte by byte, for the cases a
/// library encoder will not produce (confederations, malformed attributes).
#[cfg(test)]
pub mod build {
    use super::*;

    pub const WELL_KNOWN: u8 = 0x40;
    pub const OPTIONAL: u8 = 0x80;
    pub const TRANSITIVE: u8 = 0x40;

    pub fn prefix(net: &str, path_id: Option<u32>) -> Vec<u8> {
        let net: IpNet = net.parse().unwrap();
        let mut out = Vec::new();
        if let Some(id) = path_id {
            out.extend_from_slice(&id.to_be_bytes());
        }
        let bits = net.prefix_len();
        out.push(bits);
        let octets = match net.network() {
            IpAddr::V4(a) => a.octets().to_vec(),
            IpAddr::V6(a) => a.octets().to_vec(),
        };
        out.extend_from_slice(&octets[..bits.div_ceil(8) as usize]);
        out
    }

    pub fn nlri(nets: &[&str], path_id: Option<u32>) -> Vec<u8> {
        nets.iter().flat_map(|n| prefix(n, path_id)).collect()
    }

    pub fn attr(flags: u8, kind: u8, value: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        if value.len() > 255 {
            out.extend_from_slice(&[flags | FLAG_EXTENDED_LENGTH, kind]);
            out.extend_from_slice(&(value.len() as u16).to_be_bytes());
        } else {
            out.extend_from_slice(&[flags, kind, value.len() as u8]);
        }
        out.extend_from_slice(value);
        out
    }

    pub fn origin(v: u8) -> Vec<u8> {
        attr(WELL_KNOWN, ATTR_ORIGIN, &[v])
    }

    /// AS_PATH with 4-octet ASNs; each segment `(type, asns)`.
    pub fn as_path(segments: &[(u8, &[u32])]) -> Vec<u8> {
        attr(WELL_KNOWN, ATTR_AS_PATH, &segments_bytes(segments, true))
    }

    pub fn as_path2(segments: &[(u8, &[u32])]) -> Vec<u8> {
        attr(WELL_KNOWN, ATTR_AS_PATH, &segments_bytes(segments, false))
    }

    pub fn as4_path(segments: &[(u8, &[u32])]) -> Vec<u8> {
        attr(
            OPTIONAL | TRANSITIVE,
            ATTR_AS4_PATH,
            &segments_bytes(segments, true),
        )
    }

    pub fn segments_bytes(segments: &[(u8, &[u32])], asn4: bool) -> Vec<u8> {
        let mut v = Vec::new();
        for (kind, asns) in segments {
            v.extend_from_slice(&[*kind, asns.len() as u8]);
            for a in *asns {
                if asn4 {
                    v.extend_from_slice(&a.to_be_bytes());
                } else {
                    v.extend_from_slice(&(*a as u16).to_be_bytes());
                }
            }
        }
        v
    }

    pub fn next_hop_attr(ip: &str) -> Vec<u8> {
        let ip: Ipv4Addr = ip.parse().unwrap();
        attr(WELL_KNOWN, ATTR_NEXT_HOP, &ip.octets())
    }

    pub fn med(v: u32) -> Vec<u8> {
        attr(OPTIONAL, ATTR_MED, &v.to_be_bytes())
    }

    pub fn local_pref(v: u32) -> Vec<u8> {
        attr(WELL_KNOWN, ATTR_LOCAL_PREF, &v.to_be_bytes())
    }

    pub fn communities(values: &[u32]) -> Vec<u8> {
        let v: Vec<u8> = values.iter().flat_map(|c| c.to_be_bytes()).collect();
        attr(OPTIONAL | TRANSITIVE, ATTR_COMMUNITIES, &v)
    }

    pub fn large_communities(values: &[[u32; 3]]) -> Vec<u8> {
        let v: Vec<u8> = values
            .iter()
            .flatten()
            .flat_map(|c| c.to_be_bytes())
            .collect();
        attr(OPTIONAL | TRANSITIVE, ATTR_LARGE_COMMUNITIES, &v)
    }

    pub fn mp_reach(afi: u16, safi: u8, next_hop: &[u8], nlri: &[u8]) -> Vec<u8> {
        let mut v = afi.to_be_bytes().to_vec();
        v.push(safi);
        v.push(next_hop.len() as u8);
        v.extend_from_slice(next_hop);
        v.push(0);
        v.extend_from_slice(nlri);
        attr(OPTIONAL, ATTR_MP_REACH, &v)
    }

    pub fn mp_unreach(afi: u16, safi: u8, nlri: &[u8]) -> Vec<u8> {
        let mut v = afi.to_be_bytes().to_vec();
        v.push(safi);
        v.extend_from_slice(nlri);
        attr(OPTIONAL, ATTR_MP_UNREACH, &v)
    }

    /// An UPDATE body.
    pub fn update(withdrawn: &[u8], attrs: &[Vec<u8>], nlri: &[u8]) -> Vec<u8> {
        let attrs: Vec<u8> = attrs.concat();
        let mut out = (withdrawn.len() as u16).to_be_bytes().to_vec();
        out.extend_from_slice(withdrawn);
        out.extend_from_slice(&(attrs.len() as u16).to_be_bytes());
        out.extend_from_slice(&attrs);
        out.extend_from_slice(nlri);
        out
    }

    /// A whole BGP UPDATE message: header and body.
    pub fn message(body: &[u8]) -> Vec<u8> {
        let mut out = vec![0xff; 16];
        out.extend_from_slice(&((super::super::BGP_HEADER_LEN + body.len()) as u16).to_be_bytes());
        out.push(super::super::BGP_UPDATE);
        out.extend_from_slice(body);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;
    use crate::listener::bgp::wire::{AFI_IPV6, SAFI_UNICAST};

    fn nets(bytes: &[u8], family: Afi, add_path: bool) -> Vec<(Option<u32>, String)> {
        prefixes(bytes, family, add_path)
            .map(|p| p.map(|(id, n)| (id, n.to_string())).unwrap())
            .collect()
    }

    fn path_text(p: &AsPath) -> String {
        p.segments()
            .map(|(k, a)| format!("{k:?}{a:?}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn prefixes_decode_with_and_without_path_ids() {
        let raw = nlri(&["10.0.0.0/8", "192.0.2.128/25", "0.0.0.0/0"], None);
        assert_eq!(
            nets(&raw, Afi::V4, false),
            [
                (None, "10.0.0.0/8".into()),
                (None, "192.0.2.128/25".into()),
                (None, "0.0.0.0/0".into())
            ]
        );
        let raw = nlri(&["2001:db8::/32", "::/0"], Some(7));
        assert_eq!(
            nets(&raw, Afi::V6, true),
            [(Some(7), "2001:db8::/32".into()), (Some(7), "::/0".into())]
        );
    }

    #[test]
    fn host_bits_past_the_length_are_cleared() {
        // a /20 sent with its fourth nibble set
        assert_eq!(
            nets(&[20, 10, 1, 0xff], Afi::V4, false),
            [(None, "10.1.240.0/20".into())]
        );
    }

    #[test]
    fn a_bad_prefix_is_an_error() {
        assert!(check_prefixes(&[33, 1, 2, 3, 4, 5], Afi::V4, false).is_err());
        assert!(check_prefixes(&[129], Afi::V6, false).is_err());
        assert!(check_prefixes(&[24, 10, 1], Afi::V4, false).is_err());
        assert!(check_prefixes(&[0, 0, 0], Afi::V4, true).is_err());
        let mut it = super::prefixes(&[24, 10, 1], Afi::V4, false);
        assert!(it.next().unwrap().is_err());
        assert!(it.next().is_none(), "an error ends the iteration");
    }

    #[test]
    fn an_update_reads_the_route_attributes() {
        let body = update(
            &nlri(&["10.9.0.0/16"], None),
            &[
                origin(0),
                as_path(&[(2, &[64500, 4_200_000_000]), (1, &[64510, 64511])]),
                next_hop_attr("192.0.2.1"),
                med(10),
                local_pref(200),
                communities(&[(64500 << 16) | 1]),
                large_communities(&[[4_200_000_000, 1, 2]]),
            ],
            &nlri(&["198.51.100.0/24"], None),
        );
        let u = Update::parse(&body, true, [false; 2]).unwrap();
        assert_eq!(nets(u.withdrawn, Afi::V4, false)[0].1, "10.9.0.0/16");
        assert_eq!(nets(u.nlri, Afi::V4, false)[0].1, "198.51.100.0/24");
        let a = u.attrs;
        assert_eq!(a.next_hop, Some(Ipv4Addr::new(192, 0, 2, 1)));
        assert_eq!((a.med, a.local_pref), (Some(10), Some(200)));
        assert_eq!(a.communities().collect::<Vec<_>>(), [(64500 << 16) | 1]);
        assert_eq!(
            a.large_communities().collect::<Vec<_>>(),
            [[4_200_000_000, 1, 2]]
        );
        assert_eq!(a.withdraw, None);
        assert_eq!(a.count, 7);
        let mut path = AsPath::default();
        path.decode(a.as_path.unwrap(), true).unwrap();
        assert_eq!(
            path_text(&path),
            "Sequence[64500, 4200000000] Set[64510, 64511]"
        );
        assert_eq!(u.end_of_rib(), None);
    }

    #[test]
    fn mp_reach_and_unreach_carry_v6_routes() {
        let mut nh = "2001:db8::1".parse::<Ipv6Addr>().unwrap().octets().to_vec();
        nh.extend_from_slice(&"fe80::1".parse::<Ipv6Addr>().unwrap().octets());
        let body = update(
            &[],
            &[
                origin(0),
                as_path(&[]),
                mp_reach(
                    AFI_IPV6,
                    SAFI_UNICAST,
                    &nh,
                    &nlri(&["2001:db8:1::/48"], Some(2)),
                ),
                mp_unreach(AFI_IPV6, SAFI_UNICAST, &nlri(&["2001:db8:2::/48"], Some(3))),
            ],
            &[],
        );
        let u = Update::parse(&body, true, [false, true]).unwrap();
        let reach = u.attrs.mp_reach.unwrap();
        assert_eq!(reach.next_hop, Some("2001:db8::1".parse().unwrap()));
        assert_eq!(
            nets(reach.nlri, Afi::V6, true),
            [(Some(2), "2001:db8:1::/48".into())]
        );
        let unreach = u.attrs.mp_unreach.unwrap();
        assert_eq!(
            nets(unreach.nlri, Afi::V6, true),
            [(Some(3), "2001:db8:2::/48".into())]
        );

        // without ADD-PATH for v6 the same bytes do not frame
        assert!(Update::parse(&body, true, [true, false]).is_err());
    }

    #[test]
    fn a_link_local_next_hop_is_used_only_without_a_global_one() {
        let mut nh = [0u8; 32].to_vec();
        nh[16..].copy_from_slice(&"fe80::1".parse::<Ipv6Addr>().unwrap().octets());
        let body = update(&[], &[mp_reach(AFI_IPV6, SAFI_UNICAST, &nh, &[])], &[]);
        let u = Update::parse(&body, true, [false; 2]).unwrap();
        assert_eq!(
            u.attrs.mp_reach.unwrap().next_hop,
            Some("fe80::1".parse().unwrap())
        );
        // an IPv4 route over an IPv6 next hop (RFC 8950)
        let nh = "2001:db8::9".parse::<Ipv6Addr>().unwrap().octets();
        let body = update(
            &[],
            &[mp_reach(
                AFI_IPV4,
                SAFI_UNICAST,
                &nh,
                &nlri(&["10.0.0.0/8"], None),
            )],
            &[],
        );
        let u = Update::parse(&body, true, [false; 2]).unwrap();
        assert_eq!(
            u.attrs.mp_reach.unwrap().next_hop,
            Some("2001:db8::9".parse().unwrap())
        );
        // a unicast next hop of an odd length breaks the attribute
        let body = update(
            &[],
            &[mp_reach(AFI_IPV4, SAFI_UNICAST, &[1, 2, 3], &[])],
            &[],
        );
        assert!(Update::parse(&body, true, [false; 2]).is_err());
    }

    #[test]
    fn other_families_are_kept_raw_and_not_checked() {
        // VPNv4 (SAFI 128): an RD in the next hop, labels in the NLRI
        let body = update(
            &[],
            &[mp_reach(AFI_IPV4, 128, &[0; 12], &[0xff, 1, 2])],
            &[],
        );
        let u = Update::parse(&body, true, [false; 2]).unwrap();
        let reach = u.attrs.mp_reach.unwrap();
        assert_eq!(
            (reach.afi, reach.safi, reach.next_hop),
            (AFI_IPV4, 128, None)
        );
    }

    #[test]
    fn end_of_rib_is_recognised_per_family() {
        let empty = update(&[], &[], &[]);
        let u = Update::parse(&empty, true, [false; 2]).unwrap();
        assert_eq!(u.end_of_rib(), Some((AFI_IPV4, SAFI_UNICAST)));
        let body = update(&[], &[mp_unreach(AFI_IPV6, SAFI_UNICAST, &[])], &[]);
        let u = Update::parse(&body, true, [false; 2]).unwrap();
        assert_eq!(u.end_of_rib(), Some((AFI_IPV6, SAFI_UNICAST)));
        // an MP_UNREACH that withdraws something is not one
        let body = update(
            &[],
            &[mp_unreach(
                AFI_IPV6,
                SAFI_UNICAST,
                &nlri(&["2001:db8::/32"], None),
            )],
            &[],
        );
        let u = Update::parse(&body, true, [false; 2]).unwrap();
        assert_eq!(u.end_of_rib(), None);
        // nor is an update with attributes and no NLRI
        let body = update(&[], &[origin(0)], &[]);
        let u = Update::parse(&body, true, [false; 2]).unwrap();
        assert_eq!(u.end_of_rib(), None);
    }

    #[test]
    fn a_malformed_route_attribute_means_withdraw() {
        for bad in [
            attr(WELL_KNOWN, ATTR_ORIGIN, &[3]),
            attr(WELL_KNOWN, ATTR_AS_PATH, &[2, 0]),
            attr(WELL_KNOWN, ATTR_AS_PATH, &[9, 1, 0, 0, 0, 1]),
            attr(WELL_KNOWN, ATTR_AS_PATH, &[2, 2, 0, 0, 0, 1]),
            attr(WELL_KNOWN, ATTR_NEXT_HOP, &[1, 2, 3]),
            attr(OPTIONAL, ATTR_MED, &[1]),
            attr(WELL_KNOWN, ATTR_LOCAL_PREF, &[]),
            attr(OPTIONAL, ATTR_COMMUNITIES, &[1, 2, 3]),
            attr(OPTIONAL, ATTR_COMMUNITIES, &[]),
            attr(OPTIONAL, ATTR_LARGE_COMMUNITIES, &[0; 8]),
        ] {
            let body = update(
                &[],
                std::slice::from_ref(&bad),
                &nlri(&["10.0.0.0/8"], None),
            );
            let u = Update::parse(&body, true, [false; 2]).unwrap();
            assert!(u.attrs.withdraw.is_some(), "{bad:?}");
            assert_eq!(nets(u.nlri, Afi::V4, false).len(), 1, "the NLRI still read");
        }
    }

    #[test]
    fn a_malformed_as4_path_is_discarded() {
        let body = update(
            &[],
            &[attr(OPTIONAL | TRANSITIVE, ATTR_AS4_PATH, &[2, 1, 0, 1])],
            &[],
        );
        let u = Update::parse(&body, false, [false; 2]).unwrap();
        assert_eq!(u.attrs.as4_path, None);
        assert_eq!(u.attrs.withdraw, None);
    }

    #[test]
    fn duplicates_keep_the_first_copy_but_mp_attributes_may_not_repeat() {
        let body = update(&[], &[med(1), med(2)], &[]);
        assert_eq!(
            Update::parse(&body, true, [false; 2]).unwrap().attrs.med,
            Some(1)
        );
        let body = update(
            &[],
            &[
                mp_unreach(AFI_IPV6, SAFI_UNICAST, &[]),
                mp_unreach(AFI_IPV6, SAFI_UNICAST, &[]),
            ],
            &[],
        );
        assert!(Update::parse(&body, true, [false; 2]).is_err());
    }

    #[test]
    fn a_broken_structure_is_a_session_reset() {
        let good = update(
            &nlri(&["10.0.0.0/8"], None),
            &[
                origin(0),
                as_path(&[(2, &[64500])]),
                next_hop_attr("192.0.2.1"),
            ],
            &nlri(&["10.1.0.0/16"], None),
        );
        Update::parse(&good, true, [false; 2]).unwrap();
        for cut in 0..good.len() {
            // a cut inside the NLRI can still frame whole prefixes
            let _ = Update::parse(&good[..cut], true, [false; 2]);
        }
        // the withdrawn length runs past the message
        let mut bad = good.clone();
        bad[0..2].copy_from_slice(&1000u16.to_be_bytes());
        assert!(Update::parse(&bad, true, [false; 2]).is_err());
        // an attribute runs past the attributes
        let mut bad = good.clone();
        let first_attr_len = 2 + 2 + 2 + 2;
        bad[first_attr_len] = 200;
        assert!(Update::parse(&bad, true, [false; 2]).is_err());
    }

    #[test]
    fn two_byte_paths_decode_and_merge_with_as4_path() {
        let mut path = AsPath::default();
        // what a 2-octet speaker sends: AS_TRANS (23456) for the big ASN
        path.decode(
            &segments_bytes(&[(3, &[65001]), (2, &[64500, 23456, 64502])], false),
            false,
        )
        .unwrap();
        let mut as4 = AsPath::default();
        as4.decode(&segments_bytes(&[(2, &[4_200_000_000, 64502])], true), true)
            .unwrap();
        path.merge_as4(&as4);
        assert_eq!(
            path_text(&path),
            "ConfedSequence[65001] Sequence[64500] Sequence[4200000000, 64502]"
        );

        // an AS4_PATH longer than the AS_PATH is ignored
        let mut short = AsPath::default();
        short
            .decode(&segments_bytes(&[(2, &[23456])], false), false)
            .unwrap();
        let before = short.clone();
        short.merge_as4(&as4);
        assert_eq!(short, before);

        // a set counts as one AS
        let mut set = AsPath::default();
        set.decode(
            &segments_bytes(&[(2, &[64500]), (1, &[23456, 64511])], false),
            false,
        )
        .unwrap();
        let mut as4_set = AsPath::default();
        as4_set
            .decode(&segments_bytes(&[(1, &[4_200_000_000, 64511])], true), true)
            .unwrap();
        set.merge_as4(&as4_set);
        assert_eq!(path_text(&set), "Sequence[64500] Set[4200000000, 64511]");
    }

    #[test]
    fn confederation_segments_decode() {
        let bytes = segments_bytes(&[(3, &[65001, 65002]), (4, &[65003]), (2, &[64500])], true);
        AsPath::check(&bytes, true).unwrap();
        let mut path = AsPath::default();
        path.decode(&bytes, true).unwrap();
        assert_eq!(
            path_text(&path),
            "ConfedSequence[65001, 65002] ConfedSet[65003] Sequence[64500]"
        );
    }
}
